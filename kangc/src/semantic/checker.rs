// 类型检查器 — 遍历 AST 执行全部 46 条语义规则
// 两遍扫描: 先收集声明, 再检查函数体
// 错误尽可能收集后一并返回, 不因单条错误中断

use super::flow;
use super::scope::{FuncSignature, ScopeHint, StructInfo, SymbolKind, SymbolTable};
use super::types::*;
use crate::ast::{self, BinOp, UnaryOp};
use crate::ast::{span_of_expr, span_of_lvalue, span_of_stmt};
use crate::error::SemanticError;
use std::collections::HashMap;
use std::ops::Range;

// ── Checker ────────────────────────────────────────────────────────────────────

pub struct Checker {
    symbols: SymbolTable,
    structs: HashMap<String, StructInfo>,
    errors: Vec<SemanticError>,
    passes: usize,
    failures: usize,
    current_func_return: KangType,
    current_func_name: String,
    in_loop: bool,
}

impl Checker {
    pub fn new() -> Self {
        Checker {
            symbols: SymbolTable::new(),
            structs: HashMap::new(),
            errors: Vec::new(),
            passes: 0,
            failures: 0,
            current_func_return: KangType::Void,
            current_func_name: String::new(),
            in_loop: false,
        }
    }

    pub fn passes(&self) -> usize { self.passes }
    pub fn failures(&self) -> usize { self.failures }
    pub fn symbol_count(&self) -> usize { self.symbols.symbol_count }

    // ── 入口: 两遍扫描 ─────────────────────────────────────────────────────

    pub fn check_program(
        &mut self,
        program: &ast::Program,
    ) -> Result<TypedProgram, Vec<SemanticError>> {
        // 第一遍: 收集结构体和函数声明
        self.collect_declarations(program);

        // 第二遍: 检查函数体
        let mut items = Vec::new();
        for top in &program.items {
            match top {
                ast::TopLevel::Struct(s) => {
                    items.push(TypedTopLevel::Struct(s.clone()));
                }
                ast::TopLevel::Func(func) => {
                    let typed_func = self.check_func_def(func);
                    items.push(TypedTopLevel::Func(typed_func));
                }
            }
        }

        if self.errors.is_empty() {
            Ok(TypedProgram { items })
        } else {
            Err(self.errors.clone())
        }
    }

    // ── 第一遍: 声明收集 ───────────────────────────────────────────────────

    fn collect_declarations(&mut self, program: &ast::Program) {
        for top in &program.items {
            match top {
                ast::TopLevel::Struct(s) => {
                    self.check_struct_decl(s);
                }
                ast::TopLevel::Func(f) => {
                    self.register_func_decl(f);
                }
            }
        }
    }

    // ST1: 字段不能是 void
    // ST2: 禁止直接自引用（允许 [Self] 间接引用）
    // ST5: 结构体类型必须在使用前定义（由两遍扫描自然保证，struct 在 pass1 注册）
    fn check_struct_decl(&mut self, s: &ast::StructDef) {
        let mut fields = Vec::new();
        for (name, ty) in &s.fields {
            let kt = KangType::from_ast_type(ty);
            // ST1: 字段不能是 void
            if kt == KangType::Void {
                self.error("结构体字段类型不能是 void (ST1)", &name, 0..0);
                continue;
            }
            // ST2: 禁止直接自引用
            if let KangType::Struct(type_name) = &kt {
                if type_name == &s.name {
                    self.error(
                        &format!(
                            "结构体 \"{}\" 不能直接包含自身类型的字段 (ST2)，请使用 [{}]",
                            s.name, s.name
                        ),
                        &name,
                    0..0);
                    continue;
                }
            }
            fields.push((name.clone(), kt));
        }

        let info = StructInfo { fields };
        self.structs.insert(s.name.clone(), info.clone());

        // 注册为类型符号
        let _ = self.symbols.insert(
            &s.name,
            SymbolKind::Struct(info),
            ScopeHint::Normal,
        );
    }

    // F3: 禁止用户函数重载（同名函数只能定义一个）
    fn register_func_decl(&mut self, f: &ast::FuncDef) {
        let ret = KangType::from_ast_return_type(&f.return_type);
        let params: Vec<(String, KangType)> = f
            .params
            .iter()
            .map(|(n, t)| (n.clone(), KangType::from_ast_type(t)))
            .collect();

        // F3: 检查是否与已有用户函数同名
        if let Some(existing) = self.symbols.lookup(&f.name) {
            if let SymbolKind::Function(existing_sig) = &existing.kind {
                if !existing_sig.is_builtin {
                    self.error(
                        &format!(
                            "函数 \"{}\" 已被定义，用户函数不支持重载 (F3)",
                            f.name
                        ),
                        &f.name,
                    0..0);
                    return;
                }
            }
        }

        let sig = FuncSignature {
            params,
            return_type: ret.clone(),
            is_builtin: false,
            overloads: vec![],
        };
        let _ = self.symbols.insert(
            &f.name,
            SymbolKind::Function(sig),
            ScopeHint::Normal,
        );
    }

    // ── 第二遍: 函数体检查 ─────────────────────────────────────────────────

    fn check_func_def(&mut self, func: &ast::FuncDef) -> TypedFuncDef {
        self.current_func_name = func.name.clone();
        self.current_func_return = KangType::from_ast_return_type(&func.return_type);

        self.symbols.push_scope();

        // 注册参数
        let params: Vec<(String, KangType)> = func
            .params
            .iter()
            .map(|(n, t)| {
                let kt = KangType::from_ast_type(t);
                let _ = self.symbols.insert(n, SymbolKind::Variable(kt.clone()), ScopeHint::Normal);
                (n.clone(), kt)
            })
            .collect();

        // 检查函数体（直接在当前作用域内检查，不额外推作用域）
        let body: Vec<TypedStmt> = func.body.iter().map(|s| self.check_stmt(s)).collect();

        // F1: 非 void 函数所有路径必须 return
        if !self.current_func_return.is_void() && !flow::all_paths_return(&func.body) {
            self.error(
                &format!(
                    "非 void 函数 \"{}\" 必须所有代码路径都有 return 语句 (F1)",
                    func.name
                ),
                &func.name,
            0..0);
        }

        self.symbols.pop_scope();

        TypedFuncDef {
            name: func.name.clone(),
            params,
            return_type: self.current_func_return.clone(),
            body,
        }
    }

    // ── 语句检查 ───────────────────────────────────────────────────────────

    fn check_block(&mut self, stmts: &[ast::Stmt]) -> Vec<TypedStmt> {
        self.symbols.push_scope();
        let typed_stmts: Vec<TypedStmt> = stmts.iter().map(|s| self.check_stmt(s)).collect();
        self.symbols.pop_scope();
        typed_stmts
    }

    fn check_stmt(&mut self, s: &ast::Stmt) -> TypedStmt {
        match s {
            ast::Stmt::VarDecl { bindings, init, .. } => self.check_var_decl(bindings, init),
            ast::Stmt::Assign { lvalue, value, .. } => self.check_assign(lvalue, value),
            ast::Stmt::Return { values, .. } => self.check_return(values, span_of_stmt(s)),
            ast::Stmt::If { condition, then_branch, else_branch, .. } => {
                self.check_if(condition, then_branch, else_branch.as_deref(), span_of_stmt(s))
            }
            ast::Stmt::For { var_name, var_type, start, end, step_lvalue, step_expr, body, .. } => {
                self.check_for(var_name, var_type, start, end, step_lvalue, step_expr, body, span_of_stmt(s))
            }
            ast::Stmt::Expr(e, ..) => {
                let te = self.check_expr(e, None);
                TypedStmt { kind: TypedStmtKind::Expr(Box::new(te)) }
            }
            ast::Stmt::Block(stmts, ..) => {
                let typed = self.check_block(stmts);
                TypedStmt { kind: TypedStmtKind::Block(typed) }
            }
        }
    }

    // ── VarDecl ─────────────────────────────────────────────────────────────

    fn check_var_decl(&mut self, bindings: &[ast::VarBinding], init: &ast::Expr) -> TypedStmt {
        // 先检查初始化表达式
        let expected_count = bindings.len();
        let init_typed = if expected_count == 2 {
            // 多接收: 可能需要二值返回。先不传 expected_type
            self.check_expr(init, None)
        } else {
            // 单接收: 传期望类型（来自 binding 声明的类型）
            let expected = bindings.first().and_then(|b| match b {
                ast::VarBinding::Named { ty, .. } => Some(KangType::from_ast_type(ty)),
                ast::VarBinding::Discard => None,
            });
            self.check_expr(init, expected)
        };

        let init_ty = &init_typed.ty;
        let init_arity = if matches!(init_ty, KangType::Pair(_, _)) { 2 } else { 1 };

        // M4: 接收数量匹配
        if expected_count == 2 && init_arity == 1 && !init_ty.is_void() {
            self.error(
                &format!("函数返回 1 个值，但 var 试图接收 2 个值 (M4)"),
                "",
            span_of_expr(init));
        }

        // M6: 不能从 void 函数接收
        if init_ty.is_void() && !bindings.is_empty() && !(bindings.len() == 1 && matches!(bindings[0], ast::VarBinding::Discard)) {
            self.error("不能从 void 函数接收返回值 (M6)", "", span_of_expr(init));
        }

        let mut typed_bindings = Vec::new();

        for (i, b) in bindings.iter().enumerate() {
            match b {
                ast::VarBinding::Named { name, ty } => {
                    let kt = KangType::from_ast_type(ty);
                    // 检查接收类型是否与返回值类型匹配
                    if i == 0 {
                        let first_ty = if let KangType::Pair(t1, _) = init_ty {
                            t1.as_ref()
                        } else {
                            init_ty
                        };
                        if !init_ty.is_void() && *first_ty != kt {
                            self.error(
                                &format!(
                                    "var 接收类型 {} 与返回值类型 {} 不匹配",
                                    kt, first_ty
                                ),
                                name,
                            span_of_expr(init));
                        } else {
                            self.passes += 1;
                        }
                    } else if i == 1 {
                        if let KangType::Pair(_, t2) = init_ty {
                            if *t2.as_ref() != kt {
                                self.error(
                                    &format!(
                                        "var 接收类型 {} 与返回值类型 {} 不匹配",
                                        kt, t2
                                    ),
                                    name,
                                span_of_expr(init));
                            } else {
                                self.passes += 1;
                            }
                        }
                    }
                    // S1: 参数不可重声明
                    if self.symbols.lookup_current(name).is_some() {
                        self.error(
                            &format!("参数/变量 \"{}\" 不能在同一作用域重新声明 (S1)", name),
                            name,
                        span_of_expr(init));
                    // S4: 不能声明与函数同名的变量
                    } else if let Some(entry) = self.symbols.lookup(name) {
                        if matches!(entry.kind, SymbolKind::Function(_)) {
                            self.error(
                                &format!("变量 \"{}\" 与函数同名，命名空间冲突 (S4)", name),
                                name,
                            span_of_expr(init));
                        }
                        let _ = self.symbols.insert(
                            name,
                            SymbolKind::Variable(kt.clone()),
                            ScopeHint::Normal,
                        );
                    } else {
                        let _ = self.symbols.insert(
                            name,
                            SymbolKind::Variable(kt.clone()),
                            ScopeHint::Normal,
                        );
                    }
                    typed_bindings.push((name.clone(), kt, false));
                }
                ast::VarBinding::Discard => {
                    typed_bindings.push(("_".to_string(), KangType::Void, true));
                }
            }
        }

        TypedStmt {
            kind: TypedStmtKind::VarDecl {
                bindings: typed_bindings,
                init: Box::new(init_typed),
            },
        }
    }

    // ── Assign ──────────────────────────────────────────────────────────────

    fn check_assign(&mut self, lvalue: &ast::LValue, value: &ast::Expr) -> TypedStmt {
        // 先确定左值类型
        let lvalue_ty = self.resolve_lvalue_type(lvalue);

        let val_typed = self.check_expr(value, lvalue_ty.clone());

        if let Some(lt) = lvalue_ty {
            // AS2/AS5/AS6: 赋值类型匹配
            if lt != val_typed.ty {
                self.error(
                    &format!("赋值类型不匹配: 左侧 {}，右侧 {}", lt, val_typed.ty),
                    "",
                span_of_expr(value));
            } else {
                self.passes += 1;
            }
        }

        TypedStmt {
            kind: TypedStmtKind::Assign {
                lvalue: lvalue.clone(),
                value: Box::new(val_typed),
            },
        }
    }

    /// 解析左值的类型（用于赋值类型检查）
    fn resolve_lvalue_type(&mut self, lv: &ast::LValue) -> Option<KangType> {
        match lv {
            ast::LValue::Ident(name, ..) => {
                match self.symbols.lookup(name) {
                    Some(entry) => match &entry.kind {
                        SymbolKind::Variable(kt) => Some(kt.clone()),
                        _ => {
                            // S4: 函数名不可作变量
                            self.error(&format!("\"{}\" 是函数名，不可赋值 (AS4)", name), name, span_of_lvalue(lv));
                            None
                        }
                    },
                    None => {
                        // S2: 未声明变量
                        self.error(&format!("未声明的变量 \"{}\" (S2)", name), name, span_of_lvalue(lv));
                        None
                    }
                }
            }
            ast::LValue::Index { array, .. } => {
                // AS1: str 不可变，索引不可作左值
                let arr_typed = self.check_expr(array, None);
                if matches!(arr_typed.ty, KangType::Str) {
                    self.error("字符串不可变，s[i] 不能作为赋值左值 (AS1)", "", span_of_lvalue(lv));
                    return None;
                }
                // 数组索引: 返回元素类型
                if let KangType::Array(elem) = &arr_typed.ty {
                    Some(*elem.clone())
                } else {
                    None
                }
            }
            ast::LValue::FieldAccess { obj, field, .. } => {
                // ST6/ST7: 字段访问须是结构体且字段存在
                let obj_typed = self.check_expr(obj, None);
                if let KangType::Struct(name) = &obj_typed.ty {
                    if let Some(info) = self.structs.get(name) {
                        for (fname, fty) in &info.fields {
                            if fname == field {
                                return Some(fty.clone());
                            }
                        }
                        self.error(
                            &format!("结构体 \"{}\" 没有字段 \"{}\" (ST7)", name, field),
                            field,
                        span_of_lvalue(lv));
                    } else {
                        self.error(&format!("未定义的结构体类型 \"{}\" (ST5)", name), name, span_of_lvalue(lv));
                    }
                } else {
                    self.error(
                        &format!("非结构体类型不能使用 .field 访问 (ST6)"),
                        field,
                    span_of_lvalue(lv));
                }
                None
            }
        }
    }

    // ── Return ──────────────────────────────────────────────────────────────

    fn check_return(&mut self, values: &[ast::Expr], span: Range<usize>) -> TypedStmt {
        // F2: void 函数 return 不能带值
        if self.current_func_return.is_void() && !values.is_empty() {
            self.error(
                &format!(
                    "void 函数 \"{}\" 的 return 不能带返回值 (F2)",
                    self.current_func_name
                ),
                "",
            span.clone());
        }

        // M1/M2: 返回数量匹配
        let declared_count = if matches!(self.current_func_return, KangType::Pair(_, _)) {
            2
        } else if self.current_func_return.is_void() {
            0
        } else {
            1
        };

        if !self.current_func_return.is_void() && values.len() != declared_count {
            self.error(
                &format!(
                    "return 表达式数量不匹配: 声明返回 {} 个值，实际 {} 个 (M1/M2)",
                    declared_count,
                    values.len()
                ),
                "",
            span);
        }

        // M3: 返回类型匹配
        let typed_values: Vec<TypedExpr> = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let expected = if i == 0 {
                    if let KangType::Pair(ref t1, _) = self.current_func_return {
                        Some(*t1.clone())
                    } else if !self.current_func_return.is_void() {
                        Some(self.current_func_return.clone())
                    } else {
                        None
                    }
                } else if i == 1 {
                    if let KangType::Pair(_, ref t2) = self.current_func_return {
                        Some(*t2.clone())
                    } else {
                        None
                    }
                } else {
                    None
                };
                let tv = self.check_expr(v, expected.clone());
                if let Some(et) = expected {
                    if tv.ty != et {
                        self.error(
                            &format!("返回类型不匹配: 期望 {}，得到 {} (M3)", et, tv.ty),
                            "",
                        span_of_expr(v));
                    } else {
                        self.passes += 1;
                    }
                }
                tv
            })
            .collect();

        TypedStmt {
            kind: TypedStmtKind::Return { values: typed_values },
        }
    }

    // ── If ──────────────────────────────────────────────────────────────────

    fn check_if(
        &mut self,
        condition: &ast::Expr,
        then_branch: &ast::Stmt,
        else_branch: Option<&ast::Stmt>,
        span: Range<usize>,
    ) -> TypedStmt {
        let cond_typed = self.check_expr(condition, Some(KangType::Bool));
        // T3: if 条件必须是 bool
        if cond_typed.ty != KangType::Bool {
            self.error(
                &format!("if 条件必须是 bool 类型，得到 {} (T3)", cond_typed.ty),
                "",
            span);
        } else {
            self.passes += 1;
        }

        let then_typed = self.check_stmt(then_branch);
        let else_typed = else_branch.map(|s| Box::new(self.check_stmt(s)));

        TypedStmt {
            kind: TypedStmtKind::If {
                condition: Box::new(cond_typed),
                then_branch: Box::new(then_typed),
                else_branch: else_typed,
            },
        }
    }

    // ── For ─────────────────────────────────────────────────────────────────

    fn check_for(
        &mut self,
        var_name: &str,
        var_type: &ast::Type,
        start: &ast::Expr,
        end: &ast::Expr,
        step_lvalue: &ast::LValue,
        step_expr: &ast::Expr,
        body: &ast::Stmt,
        span: Range<usize>,
    ) -> TypedStmt {
        self.symbols.push_scope();

        let var_kt = KangType::from_ast_type(var_type);

        // 注册循环变量（标记为 LoopVar）
        let _ = self.symbols.insert(
            var_name,
            SymbolKind::Variable(var_kt.clone()),
            ScopeHint::LoopVar,
        );

        let start_typed = self.check_expr(start, Some(var_kt.clone()));

        let old_in_loop = self.in_loop;
        self.in_loop = true;

        let end_typed = self.check_expr(end, Some(KangType::Bool));

        // T4: for 条件必须是 bool
        if end_typed.ty != KangType::Bool {
            self.error(
                &format!("for 循环条件必须是 bool 类型，得到 {} (T4)", end_typed.ty),
                "",
            span);
        } else {
            self.passes += 1;
        }

        let step_typed = self.check_expr(step_expr, Some(var_kt.clone()));
        let body_typed = self.check_stmt(body);
        self.in_loop = old_in_loop;

        // S3: 循环变量作用域 — 弹出时清除 LoopVar
        self.symbols.pop_scope_keep_non_loop();

        TypedStmt {
            kind: TypedStmtKind::For {
                var_name: var_name.to_string(),
                var_type: var_kt,
                start: Box::new(start_typed),
                end: Box::new(end_typed),
                step_lvalue: step_lvalue.clone(),
                step_expr: Box::new(step_typed),
                body: Box::new(body_typed),
            },
        }
    }

    // ── 表达式类型推导 ─────────────────────────────────────────────────────

    /// 检查表达式，可选的 expected_type 用于上下文类型推断（如空数组 []）
    fn check_expr(&mut self, expr: &ast::Expr, expected_type: Option<KangType>) -> TypedExpr {
        match expr {
            ast::Expr::IntLit(v, ..) => self.typed_lit(TypedExprKind::IntLit(v.clone()), KangType::I32),
            ast::Expr::FloatLit(v, ..) => {
                self.typed_lit(TypedExprKind::FloatLit(v.clone()), KangType::F64)
            }
            ast::Expr::StrLit(v, ..) => self.typed_lit(TypedExprKind::StrLit(v.clone()), KangType::Str),
            ast::Expr::BoolLit(v, ..) => {
                self.typed_lit(TypedExprKind::BoolLit(*v), KangType::Bool)
            }
            ast::Expr::Ident(name, ..) => self.check_ident(name, span_of_expr(expr)),
            ast::Expr::Binary { left, op, right, .. } => self.check_binary(left, op, right, span_of_expr(expr)),
            ast::Expr::Unary { op, expr: inner, .. } => self.check_unary(op, inner, span_of_expr(expr)),
            ast::Expr::Call { func, args, .. } => self.check_call(func, args, span_of_expr(expr)),
            ast::Expr::Index { array, index, .. } => self.check_index(array, index, span_of_expr(expr)),
            ast::Expr::FieldAccess { obj, field, .. } => self.check_field_access(obj, field, span_of_expr(expr)),
            ast::Expr::ArrayLit(elems, ..) => self.check_array_lit(elems, expected_type, span_of_expr(expr)),
            ast::Expr::StructLit { name, fields, .. } => self.check_struct_lit(name, fields, span_of_expr(expr)),
        }
    }

    fn typed_lit(&mut self, kind: TypedExprKind, ty: KangType) -> TypedExpr {
        self.passes += 1;
        TypedExpr { kind, ty }
    }

    // 标识符: 查符号表
    fn check_ident(&mut self, name: &str, span: Range<usize>) -> TypedExpr {
        // 先提取数据，避免 borrow checker 冲突
        let lookup_result = self.symbols.lookup(name).map(|e| {
            (match &e.kind {
                SymbolKind::Variable(kt) => ("var", kt.clone(), e.hint.clone()),
                SymbolKind::Function(_) => ("func", KangType::Void, ScopeHint::Normal),
                _ => ("other", KangType::I32, ScopeHint::Normal),
            }, e.hint.clone())
        });

        match lookup_result {
            Some((("var", kt, hint), _entry_hint)) => {
                // M5: _ 不可作变量使用
                if name == "_" {
                    self.error("_ 是占位符，不能在表达式中使用 (M5)", name, span);
                    return TypedExpr {
                        kind: TypedExprKind::Ident(name.to_string()),
                        ty: KangType::I32,
                    };
                }
                // S3: 检查是否是已失效的循环变量
                if hint == ScopeHint::LoopVar && !self.in_loop {
                    self.error(
                        &format!("循环变量 \"{}\" 在循环结束后不可访问 (S3)", name),
                        name,
                    span);
                }
                self.passes += 1;
                TypedExpr {
                    kind: TypedExprKind::Ident(name.to_string()),
                    ty: kt,
                }
            }
            Some((("func", _, _), _)) => {
                // Kang 不支持一等函数，函数名不可作值使用
                self.error(&format!("函数 \"{}\" 不能作为值引用，Kang 不支持一等函数", name), name, span);
                TypedExpr {
                    kind: TypedExprKind::Ident(name.to_string()),
                    ty: KangType::I32,
                }
            }
            _ => {
                self.error(&format!("未声明的变量 \"{}\" (S2)", name), name, span);
                TypedExpr {
                    kind: TypedExprKind::Ident(name.to_string()),
                    ty: KangType::I32,
                }
            }
        }
    }

    // 二元表达式
    fn check_binary(
        &mut self,
        left: &ast::Expr,
        op: &ast::BinOp,
        right: &ast::Expr,
        span: Range<usize>,
    ) -> TypedExpr {
        let left_typed = self.check_expr(left, None);
        let lt = &left_typed.ty;

        // + 特殊处理: 任一操作数是 str 时自动转字符串拼接
        let is_str_concat = matches!(op, BinOp::Add)
            && (matches!(lt, KangType::Str) || matches!(self.peek_expr_type(right), KangType::Str));

        if is_str_concat {
            // 字符串拼接: 另一个操作数自动转 str
            let right_typed = self.check_expr(right, None);
            self.passes += 1;
            return TypedExpr {
                kind: TypedExprKind::Binary {
                    left: Box::new(left_typed),
                    op: op.clone(),
                    right: Box::new(right_typed),
                },
                ty: KangType::Str,
            };
        }

        let right_typed = self.check_expr(right, None);
        let rt = &right_typed.ty;

        match op {
            BinOp::Or | BinOp::And => {
                // T5: && || 操作数必须是 bool
                if *lt != KangType::Bool {
                    self.error(&format!("\"{}\" 操作数必须是 bool，得到 {} (T5)", op, lt), "", span_of_expr(left));
                }
                if *rt != KangType::Bool {
                    self.error(&format!("\"{}\" 操作数必须是 bool，得到 {} (T5)", op, rt), "", span_of_expr(right));
                }
                if *lt == KangType::Bool && *rt == KangType::Bool {
                    self.passes += 1;
                }
                self.typed_lit(
                    TypedExprKind::Binary {
                        left: Box::new(left_typed),
                        op: op.clone(),
                        right: Box::new(right_typed),
                    },
                    KangType::Bool,
                )
            }
            BinOp::Eq | BinOp::Neq => {
                // T7/T8: == != 要求同类型
                if lt != rt {
                    self.error(
                        &format!("==/!= 要求左右类型相同，得到 {} 和 {} (T7/T8)", lt, rt),
                        "",
                    span);
                } else {
                    self.passes += 1;
                }
                self.typed_lit(
                    TypedExprKind::Binary {
                        left: Box::new(left_typed),
                        op: op.clone(),
                        right: Box::new(right_typed),
                    },
                    KangType::Bool,
                )
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                // T2: 比较运算要求左右同为 i32 或同为 f64
                let valid = matches!(
                    (lt, rt),
                    (KangType::I32, KangType::I32) | (KangType::F64, KangType::F64)
                );
                if !valid {
                    self.error(
                        &format!(
                            "比较运算要求左右同为 i32 或同为 f64，得到 {} 和 {} (T2)",
                            lt, rt
                        ),
                        "",
                    span);
                } else {
                    self.passes += 1;
                }
                self.typed_lit(
                    TypedExprKind::Binary {
                        left: Box::new(left_typed),
                        op: op.clone(),
                        right: Box::new(right_typed),
                    },
                    KangType::Bool,
                )
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                // T1: 算术运算要求左右同为 i32 或同为 f64
                let valid = matches!(
                    (lt, rt),
                    (KangType::I32, KangType::I32) | (KangType::F64, KangType::F64)
                );
                if !valid {
                    self.error(
                        &format!(
                            "算术运算 \"{}\" 要求左右同为 i32 或同为 f64，得到 {} 和 {} (T1)",
                            op, lt, rt
                        ),
                        "",
                    span);
                } else {
                    self.passes += 1;
                }
                let result_ty = if *lt == KangType::I32 { KangType::I32 } else { KangType::F64 };
                TypedExpr {
                    kind: TypedExprKind::Binary {
                        left: Box::new(left_typed),
                        op: op.clone(),
                        right: Box::new(right_typed),
                    },
                    ty: result_ty,
                }
            }
        }
    }

    /// 窥探表达式的类型（用于 str 拼接判断，避免借用冲突）
    fn peek_expr_type(&self, expr: &ast::Expr) -> KangType {
        match expr {
            ast::Expr::IntLit(..) => KangType::I32,
            ast::Expr::FloatLit(..) => KangType::F64,
            ast::Expr::StrLit(..) => KangType::Str,
            ast::Expr::BoolLit(..) => KangType::Bool,
            ast::Expr::Ident(name, ..) => self
                .symbols
                .lookup(name)
                .and_then(|e| match &e.kind {
                    SymbolKind::Variable(kt) => Some(kt.clone()),
                    _ => None,
                })
                .unwrap_or(KangType::I32),
            // 深度嵌套: 保守假设非 str
            _ => KangType::I32,
        }
    }

    // 一元表达式
    fn check_unary(&mut self, op: &ast::UnaryOp, expr: &ast::Expr, span: Range<usize>) -> TypedExpr {
        let inner = self.check_expr(expr, None);
        match op {
            UnaryOp::Neg => {
                // 取负: 必须是 i32 或 f64
                if inner.ty != KangType::I32 && inner.ty != KangType::F64 {
                    self.error(
                        &format!("取负 \"-\" 操作数必须是 i32 或 f64，得到 {} (T1)", inner.ty),
                        "",
                    span);
                } else {
                    self.passes += 1;
                }
                TypedExpr {
                    kind: TypedExprKind::Unary { op: op.clone(), expr: Box::new(inner.clone()) },
                    ty: inner.ty.clone(),
                }
            }
            UnaryOp::Not => {
                // T6: ! 操作数必须是 bool
                if inner.ty != KangType::Bool {
                    self.error(
                        &format!("! 操作数必须是 bool，得到 {} (T6)", inner.ty),
                        "",
                    span);
                } else {
                    self.passes += 1;
                }
                TypedExpr {
                    kind: TypedExprKind::Unary { op: op.clone(), expr: Box::new(inner.clone()) },
                    ty: KangType::Bool,
                }
            }
        }
    }

    // 函数调用
    fn check_call(&mut self, func: &ast::Expr, args: &[ast::Expr], span: Range<usize>) -> TypedExpr {
        // 提取函数名
        let func_name = match func {
            ast::Expr::Ident(name, ..) => name.clone(),
            _ => {
                self.error("调用目标必须是函数名", "", span);
                return TypedExpr {
                    kind: TypedExprKind::Call { func_name: "<unknown>".into(), args: vec![] },
                    ty: KangType::I32,
                };
            }
        };

        // 检查所有参数
        let typed_args: Vec<TypedExpr> = args.iter().map(|a| self.check_expr(a, None)).collect();
        let arg_types: Vec<KangType> = typed_args.iter().map(|a| a.ty.clone()).collect();

        // 特殊处理: 内置泛型函数
        if func_name == "len" {
            return self.check_builtin_len(&func_name, &typed_args, &arg_types, span);
        }
        if func_name == "push" {
            return self.check_builtin_push(&func_name, &typed_args, &arg_types, span);
        }

        // 查找函数签名（提取数据避免 borrow checker 冲突）
        let sig_data = self.symbols.lookup_function(&func_name, &arg_types)
            .map(|sig| (sig.params.clone(), sig.return_type.clone(), sig.params.len()));

        match sig_data {
            Some((params, return_type, param_count)) => {
                // F4/F5: 参数数量
                if param_count != arg_types.len() {
                    self.error(
                        &format!(
                            "函数 \"{}\" 参数数量不匹配: 期望 {} 个，传入 {} 个 (F4/F5)",
                            func_name,
                            param_count,
                            arg_types.len()
                        ),
                        "",
                    span.clone());
                }

                // F6: 参数类型 — Pair 自动解包取第一值（多返回值作单值参数）
                for (i, ((_, pt), at)) in params.iter().zip(&arg_types).enumerate() {
                    let effective_at = match (at, pt) {
                        (KangType::Pair(first, _), _) if !matches!(pt, KangType::Pair(_, _)) => first.as_ref(),
                        _ => at,
                    };
                    if pt != effective_at {
                        self.error(
                            &format!(
                                "函数 \"{}\" 第 {} 个参数类型不匹配: 期望 {}，传入 {} (F6)",
                                func_name,
                                i + 1,
                                pt,
                                effective_at
                            ),
                            "",
                        span.clone());
                    } else {
                        self.passes += 1;
                    }
                }

                TypedExpr {
                    kind: TypedExprKind::Call {
                        func_name: func_name.clone(),
                        args: typed_args,
                    },
                    ty: return_type,
                }
            }
            None => {
                self.error(&format!("未定义的函数 \"{}\"", func_name), &func_name, span);
                TypedExpr {
                    kind: TypedExprKind::Call {
                        func_name: func_name.clone(),
                        args: typed_args,
                    },
                    ty: KangType::I32,
                }
            }
        }
    }

    // len 内置: len(str) -> i32, len([T]) -> i32
    fn check_builtin_len(
        &mut self,
        func_name: &str,
        typed_args: &[TypedExpr],
        arg_types: &[KangType],
        span: Range<usize>,
    ) -> TypedExpr {
        // A5: len 参数必须是 str 或数组
        if arg_types.len() != 1 {
            self.error("len() 接受 1 个参数 (str 或数组)", "", span);
            return TypedExpr {
                kind: TypedExprKind::Call {
                    func_name: func_name.to_string(),
                    args: typed_args.to_vec(),
                },
                ty: KangType::I32,
            };
        }
        match &arg_types[0] {
            KangType::Str | KangType::Array(_) => {
                self.passes += 1;
            }
            _ => {
                self.error(
                    &format!(
                        "len() 参数必须是 str 或数组，得到 {} (A5)",
                        arg_types[0]
                    ),
                    "",
                span);
            }
        }
        TypedExpr {
            kind: TypedExprKind::Call {
                func_name: func_name.to_string(),
                args: typed_args.to_vec(),
            },
            ty: KangType::I32,
        }
    }

    // push 内置: push([T], T) -> void
    fn check_builtin_push(
        &mut self,
        func_name: &str,
        typed_args: &[TypedExpr],
        arg_types: &[KangType],
        span: Range<usize>,
    ) -> TypedExpr {
        // A4: push 第一个参数必须是数组
        if arg_types.len() != 2 {
            self.error("push() 接受 2 个参数: ([T], T)", "", span);
            return TypedExpr {
                kind: TypedExprKind::Call {
                    func_name: func_name.to_string(),
                    args: typed_args.to_vec(),
                },
                ty: KangType::Void,
            };
        }
        match &arg_types[0] {
            KangType::Array(elem) => {
                // A3: push 元素类型必须匹配数组元素类型
                if *elem.as_ref() != arg_types[1] {
                    self.error(
                        &format!(
                            "push() 元素类型不匹配: 数组元素类型 {}，push 的元素类型 {} (A3)",
                            elem, arg_types[1]
                        ),
                        "",
                    span);
                } else {
                    self.passes += 1;
                }
            }
            _ => {
                self.error(
                    &format!("push() 第一个参数必须是数组，得到 {} (A4)", arg_types[0]),
                    "",
                span);
            }
        }
        TypedExpr {
            kind: TypedExprKind::Call {
                func_name: func_name.to_string(),
                args: typed_args.to_vec(),
            },
            ty: KangType::Void,
        }
    }

    // 索引
    fn check_index(&mut self, array: &ast::Expr, index: &ast::Expr, span: Range<usize>) -> TypedExpr {
        let arr_typed = self.check_expr(array, None);
        let idx_typed = self.check_expr(index, Some(KangType::I32));

        // T9/T10: 索引必须是 i32
        if idx_typed.ty != KangType::I32 {
            self.error(
                &format!("索引必须是 i32 类型，得到 {} (T9/T10)", idx_typed.ty),
                "",
            span.clone());
        } else {
            self.passes += 1;
        }

        // T12: str 索引返回单字符 str
        let result_ty = match &arr_typed.ty {
            KangType::Str => KangType::Str,
            KangType::Array(elem) => *elem.clone(),
            _ => {
                self.error(&format!("不能对 {} 类型使用索引", arr_typed.ty), "", span);
                KangType::I32
            }
        };

        TypedExpr {
            kind: TypedExprKind::Index {
                array: Box::new(arr_typed),
                index: Box::new(idx_typed),
            },
            ty: result_ty,
        }
    }

    // 字段访问
    fn check_field_access(&mut self, obj: &ast::Expr, field: &str, span: Range<usize>) -> TypedExpr {
        let obj_typed = self.check_expr(obj, None);

        match &obj_typed.ty {
            KangType::Struct(name) => {
                // ST5: 结构体类型必须已定义
                if let Some(info) = self.structs.get(name) {
                    // ST7: 字段必须存在
                    for (fname, fty) in &info.fields {
                        if fname == field {
                            self.passes += 1;
                            return TypedExpr {
                                kind: TypedExprKind::FieldAccess {
                                    obj: Box::new(obj_typed),
                                    field: field.to_string(),
                                },
                                ty: fty.clone(),
                            };
                        }
                    }
                    self.error(
                        &format!(
                            "结构体 \"{}\" 没有字段 \"{}\" (ST7)",
                            name, field
                        ),
                        field,
                    span);
                } else {
                    self.error(
                        &format!("未定义的结构体类型 \"{}\" (ST5)", name),
                        name,
                    span);
                }
            }
            _ => {
                // ST6: 非结构体不能 .field
                self.error(
                    &format!("非结构体类型 {} 不能使用 .field 访问 (ST6)", obj_typed.ty),
                    field,
                span);
            }
        }

        TypedExpr {
            kind: TypedExprKind::FieldAccess {
                obj: Box::new(obj_typed),
                field: field.to_string(),
            },
            ty: KangType::I32,
        }
    }

    // 数组字面量
    fn check_array_lit(
        &mut self,
        elems: &[ast::Expr],
        expected_type: Option<KangType>,
        span: Range<usize>,
    ) -> TypedExpr {
        if elems.is_empty() {
            // 空数组: 需要从上下文推断元素类型
            let elem_ty = expected_type
                .and_then(|t| match t {
                    KangType::Array(e) => Some(*e),
                    _ => None,
                })
                .unwrap_or(KangType::I32);
            // A1: [void] 检查
            if elem_ty == KangType::Void {
                self.error("数组元素类型不能是 void (A1)", "", span.clone());
            } else {
                self.passes += 1;
            }
            return TypedExpr {
                kind: TypedExprKind::ArrayLit(vec![]),
                ty: KangType::Array(Box::new(elem_ty)),
            };
        }

        // A2: 所有元素类型一致
        let typed_elems: Vec<TypedExpr> = elems.iter().map(|e| self.check_expr(e, None)).collect();
        let first_ty = typed_elems[0].ty.clone();
        let mut all_same = true;
        for (i, te) in typed_elems.iter().enumerate().skip(1) {
            if te.ty != first_ty {
                all_same = false;
                self.error(
                    &format!(
                        "数组字面量元素类型不一致: 期望 {}，第 {} 个元素是 {} (A2)",
                        first_ty,
                        i + 1,
                        te.ty
                    ),
                    "",
                span.clone());
            }
        }
        if all_same {
            self.passes += 1;
        }

        // A1: [void] 检查
        if first_ty == KangType::Void {
            self.error("数组元素类型不能是 void (A1)", "", span);
        }

        TypedExpr {
            kind: TypedExprKind::ArrayLit(typed_elems),
            ty: KangType::Array(Box::new(first_ty.clone())),
        }
    }

    // 结构体字面量
    fn check_struct_lit(
        &mut self,
        name: &str,
        fields: &[(String, ast::Expr)],
        span: Range<usize>,
    ) -> TypedExpr {
        // ST5: 结构体类型必须已定义
        let struct_info = match self.structs.get(name) {
            Some(info) => info.clone(),
            None => {
                self.error(
                    &format!("未定义的结构体类型 \"{}\" (ST5)", name),
                    name,
                span);
                return TypedExpr {
                    kind: TypedExprKind::StructLit {
                        name: name.to_string(),
                        fields: vec![],
                    },
                    ty: KangType::Struct(name.to_string()),
                };
            }
        };

        // ST3: 检查是否缺少字段
        let declared_field_names: Vec<&str> =
            struct_info.fields.iter().map(|(n, _)| n.as_str()).collect();
        let provided_field_names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();

        for df in &declared_field_names {
            if !provided_field_names.contains(df) {
                self.error(
                    &format!(
                        "结构体 \"{}\" 缺少字段 \"{}\" (ST3) — 必须初始化所有字段",
                        name, df
                    ),
                    df,
                span.clone());
            }
        }

        // ST4: 检查是否有多余字段
        for pf in &provided_field_names {
            if !declared_field_names.contains(pf) {
                self.error(
                    &format!(
                        "结构体 \"{}\" 没有字段 \"{}\" (ST4) — 多余字段",
                        name, pf
                    ),
                    pf,
                span.clone());
            }
        }

        // T11: 字段类型匹配
        let typed_fields: Vec<(String, TypedExpr)> = fields
            .iter()
            .map(|(fname, fexpr)| {
                let expected_ty = struct_info
                    .fields
                    .iter()
                    .find(|(n, _)| n == fname)
                    .map(|(_, t)| t.clone());
                let tf = self.check_expr(fexpr, expected_ty.clone());
                if let Some(et) = expected_ty {
                    if tf.ty != et {
                        self.error(
                            &format!(
                                "字段 \"{}\" 类型不匹配: 声明 {}，提供的值 {} (T11)",
                                fname, et, tf.ty
                            ),
                            fname,
                        span.clone());
                    } else {
                        self.passes += 1;
                    }
                }
                (fname.clone(), tf)
            })
            .collect();

        TypedExpr {
            kind: TypedExprKind::StructLit {
                name: name.to_string(),
                fields: typed_fields,
            },
            ty: KangType::Struct(name.to_string()),
        }
    }

    // ── 错误辅助 ───────────────────────────────────────────────────────────

    fn error(&mut self, msg: &str, context: &str, span: Range<usize>) {
        self.failures += 1;
        let full_msg = if context.is_empty() {
            msg.to_string()
        } else {
            format!("{}: \"{}\"", msg, context)
        };
        self.errors.push(SemanticError {
            msg: full_msg,
            line: 0,
            col: 0,
            span,
        });
    }
}
