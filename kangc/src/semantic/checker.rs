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

/// 解析 import 路径: 相对于当前文件所在目录，或绝对路径
/// 根据 import 路径和当前文件路径，解析出目标文件的绝对路径
///
/// 相对路径以当前源文件的父目录为基准解析。解析后进行规范化，防止 `..` 穿越
/// 到项目目录之外。若路径穿越或无法解析，返回错误描述。
fn resolve_module_path(path: &str, current_file: Option<&str>) -> Result<std::path::PathBuf, String> {
    let raw = std::path::PathBuf::from(path);
    if raw.is_absolute() {
        return Err("import 不允许绝对路径，请使用相对路径".into());
    }
    let resolved = if let Some(file) = current_file {
        let parent = std::path::Path::new(file)
            .parent()
            .unwrap_or(std::path::Path::new("."));
        parent.join(path)
    } else {
        raw
    };
    // 规范化并验证不穿越工作目录
    let canon = resolved.canonicalize().map_err(|e| format!("无法解析导入路径 {}: {}", path, e))?;
    let cwd = std::env::current_dir().map_err(|e| format!("无法获取当前目录: {}", e))?;
    let canon_cwd = cwd.canonicalize().map_err(|e| format!("无法解析当前目录: {}", e))?;
    if !canon.starts_with(&canon_cwd) {
        return Err(format!("导入路径 '{}' 穿越到项目目录之外", path));
    }
    Ok(canon)
}

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
    source_file: Option<String>,
    source_text: Option<String>,
    /// alias → module_name 映射（如 m → math），用于解析 import 调用的代码生成名称
    alias_modules: HashMap<String, String>,
    /// 从导入模块收集的结构体定义, 注入 TypedProgram 供代码生成使用
    imported_structs: Vec<ast::StructDef>,
}

impl Checker {
    pub fn new(file_path: Option<&str>, source_text: Option<&str>) -> Self {
        Checker {
            symbols: SymbolTable::new(),
            structs: HashMap::new(),
            errors: Vec::new(),
            passes: 0,
            failures: 0,
            current_func_return: KangType::Void,
            current_func_name: String::new(),
            in_loop: false,
            source_file: file_path.map(|s| s.to_string()),
            source_text: source_text.map(|s| s.to_string()),
            alias_modules: HashMap::new(),
            imported_structs: Vec::new(),
        }
    }

    pub fn passes(&self) -> usize { self.passes }
    pub fn failures(&self) -> usize { self.failures }
    pub fn symbol_count(&self) -> usize { self.symbols.symbol_count }

    /// 清空累积的错误列表（REPL 每次输入前调用）
    pub fn clear_errors(&mut self) {
        self.errors.clear();
    }

    /// 取出累积的错误并清空
    pub fn take_errors(&mut self) -> Vec<SemanticError> {
        std::mem::take(&mut self.errors)
    }

    // ── 入口: 两遍扫描 ─────────────────────────────────────────────────────

    /// 类型检查整个程序 — 两遍扫描: 先收集声明, 再检查函数体
    ///
    /// 内部调用 `collect_declarations` 和 `check_func_def`。
    /// 调用者可以直接使用此方法，无需手动准备。
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
                ast::TopLevel::Import(_) => {
                    // import 语句由模块系统单独处理，语义检查中跳过
                }
            }
        }

        // 将导入模块的结构体定义注入 TypedProgram，供代码生成使用
        for s in &self.imported_structs {
            items.push(TypedTopLevel::Struct(s.clone()));
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
                ast::TopLevel::Import(imp) => {
                    // 注册导入模块的符号（由模块系统解析后调用）
                    self.register_import_decl(imp);
                }
            }
        }
    }

    /// 注册导入项的符号声明
    fn register_import_decl(&mut self, imp: &ast::ImportStmt) {
        let import_path = match resolve_module_path(&imp.path, self.source_file.as_deref()) {
            Ok(p) => p,
            Err(msg) => {
                self.error(&msg, "", imp.span.clone());
                return;
            }
        };

        let source = match std::fs::read_to_string(&import_path) {
            Ok(s) => s,
            Err(_) => {
                self.error(&format!("无法找到导入文件: {}", imp.path), "", imp.span.clone());
                return;
            }
        };
        let mut lex_stats = crate::stats::LexStats::default();
        let tokens = match crate::lexer::tokenize(&source, &mut lex_stats) {
            Ok(t) => t,
            Err(e) => {
                self.errors.push(SemanticError {
                    msg: format!("导入文件 '{}' 词法错误: {:?}", imp.path, e),
                    line: 0, col: 0, span: 0..0,
                });
                return;
            }
        };
        let mut parse_stats = crate::stats::ParseStats::default();
        let imported_program = match crate::parser::parse(&tokens, &mut parse_stats) {
            Ok(p) => p,
            Err(e) => {
                self.errors.push(SemanticError {
                    msg: format!("导入文件 '{}' 语法错误: {:?}", imp.path, e),
                    line: 0, col: 0, span: 0..0,
                });
                return;
            }
        };

        for item_name in &imp.items {
            let found = imported_program.items.iter().find(|item| match item {
                ast::TopLevel::Func(f) => &f.name == item_name,
                ast::TopLevel::Struct(s) => &s.name == item_name,
                ast::TopLevel::Import(_) => false,
            });

            // 语义查找名（用户侧），代码生成用原名（链接时符号匹配）
            let semantic_name = format!("{}.{}", imp.alias, item_name);

            match found {
                Some(ast::TopLevel::Func(f)) => {
                    let params: Vec<(String, KangType)> = f.params.iter().map(|(n, t)|
                        (n.clone(), KangType::from_ast_type(t))
                    ).collect();
                    let ret = KangType::from_ast_return_type(&f.return_type);
                    let sig = super::scope::FuncSignature {
                        params,
                        return_type: ret,
                        is_builtin: false,
                        overloads: vec![],
                    };
                    let _ = self.symbols.insert(
                        &semantic_name,
                        super::scope::SymbolKind::Function(sig),
                        super::scope::ScopeHint::Normal,
                    );
                    // 记录 semantic_name → 原名 映射，供 check_call 生成代码名
                    self.alias_modules.insert(semantic_name.clone(), item_name.clone());
                }
                Some(ast::TopLevel::Struct(s)) => {
                    let fields: Vec<(String, KangType)> = s.fields.iter().map(|(n, t)|
                        (n.clone(), KangType::from_ast_type(t))
                    ).collect();
                    // 同时注册原名和限定名: Point 和 g.Point 都可引用
                    self.structs.insert(semantic_name.clone(), super::scope::StructInfo { fields: fields.clone() });
                    self.structs.entry(s.name.clone()).or_insert(super::scope::StructInfo { fields: fields.clone() });
                    let _ = self.symbols.insert(
                        &semantic_name,
                        super::scope::SymbolKind::Struct(super::scope::StructInfo { fields }),
                        super::scope::ScopeHint::Normal,
                    );
                    // 记录结构体定义, 注入 TypedProgram 供代码生成
                    self.imported_structs.push(s.clone());
                }
                _ => {
                    self.errors.push(SemanticError {
                        msg: format!("模块 \"{}\" 中未找到 \"{}\"", imp.path, item_name),
                        line: 0, col: 0, span: 0..0,
                    });
                }
            }
        }
    }

    // ST1: 字段不能是 void
    // ST2: 禁止直接自引用（允许 [Self] 间接引用）
    // ST5: 结构体类型必须在使用前定义（由两遍扫描自然保证，struct 在 pass1 注册）
    /// 注册结构体声明 — 必须在 `check_func_def` 之前调用，否则函数体中的类型引用会失败
    pub fn check_struct_decl(&mut self, s: &ast::StructDef) {
        let mut fields = Vec::new();
        for (name, ty) in &s.fields {
            let kt = KangType::from_ast_type(ty);
            // ST1: 字段不能是 void
            if kt == KangType::Void {
                let context = format!("struct {} 的字段 {}", s.name, name);
                self.error("结构体字段类型不能是 void (ST1)", &context, 0..0);
                continue;
            }
            // ST2: 禁止直接自引用
            if let KangType::Struct(type_name) = &kt {
                if type_name == &s.name {
                    let context = format!("struct {} 的字段 {}", s.name, name);
                    self.error(
                        &format!(
                            "结构体 \"{}\" 不能直接包含自身类型的字段 (ST2)，请使用 [{}]",
                            s.name, s.name
                        ),
                        &context,
                    0..0);
                    continue;
                }
            }
            fields.push((name.clone(), kt));
        }

        let info = StructInfo { fields };
        self.structs.insert(s.name.clone(), info.clone());

        // 注册为类型符号
        if let Err(e) = self.symbols.insert(
            &s.name,
            SymbolKind::Struct(info),
            ScopeHint::Normal,
        ) {
            self.error(&e, &s.name, 0..0);
        }
    }

    // F3: 禁止用户函数重载（同名函数只能定义一个）
    pub fn register_func_decl(&mut self, f: &ast::FuncDef) {
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
        if let Err(e) = self.symbols.insert(
            &f.name,
            SymbolKind::Function(sig),
            ScopeHint::Normal,
        ) {
            self.error(&e, &f.name, 0..0);
        }
    }

    // ── 第二遍: 函数体检查 ─────────────────────────────────────────────────

    /// 类型检查单个函数定义 — 调用前必须先通过 `collect_declarations` 注册所有函数签名，
    /// 通过 `check_struct_decl` 注册所有结构体类型，否则函数体中的调用和类型引用无法解析。
    pub fn check_func_def(&mut self, func: &ast::FuncDef) -> TypedFuncDef {
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

    pub fn check_stmt(&mut self, s: &ast::Stmt) -> TypedStmt {
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

    /// 检查变量声明语句 `var <bindings> = <init>;`。
    ///
    /// 先求值初始化表达式获取类型和值，然后验证:
    ///   - 接收数量与返回值数量匹配（M4: 单返回不能两接收）
    ///   - 不能从 void 函数接收返回值（M6）
    ///   - 各变量的接收类型与返回值类型匹配
    ///   - 无变量重声明（S1）
    ///   - 变量不与函数同名（S4）
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
                        } else if expected_count > 1 {
                            // 多接收时在此处计 passes（pair 解包检查）
                            // 单接收的 passes 已由 check_expr 内部计数，避免重复
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
                        } else {
                            let _ = self.symbols.insert(
                                name,
                                SymbolKind::Variable(kt.clone()),
                                ScopeHint::Normal,
                            );
                        }
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

    /// 检查赋值语句 `<lvalue> = <value>;`。
    ///
    /// 先解析左值类型（变量查找/数组索引/字段访问），再检查表达式，
    /// 验证左右两侧类型一致（AS2/AS5/AS6）。
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

    /// 解析左值的类型，用于赋值类型检查。
    ///
    /// Ident → 查符号表找变量类型（S2: 未声明报错；AS4: 函数名不可赋值）。
    /// Index → 检查字符串不可变（AS1），返回数组元素类型。
    /// FieldAccess → 查结构体字段类型（ST6/ST7）。
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
                // 使用 peek_expr_type 避免重复计 pass（类型解析不计入 passes）
                let arr_ty = self.peek_expr_type(array);
                if matches!(arr_ty, KangType::Str) {
                    self.error("字符串不可变，s[i] 不能作为赋值左值 (AS1)", "", span_of_lvalue(lv));
                    return None;
                }
                // 数组索引: 返回元素类型
                if let KangType::Array(elem) = &arr_ty {
                    Some(*elem.clone())
                } else {
                    None
                }
            }
            ast::LValue::FieldAccess { obj, field, .. } => {
                // ST6/ST7: 字段访问须是结构体且字段存在
                // 使用 peek_expr_type 避免重复计 pass
                let obj_ty = self.peek_expr_type(obj);
                if let KangType::Struct(name) = &obj_ty {
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

    /// 检查 return 语句: `return` 或 `return v1, v2`。
    ///
    /// 验证:
    ///   - void 函数不能返回值（F2）
    ///   - 返回值数量与声明匹配（M1/M2）
    ///   - 各返回值类型与声明类型匹配（M3）
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

    /// 检查 if/then/else 语句。
    ///
    /// 条件表达式必须为 bool 类型（T3）。then 和 else 分支在各自的作用域中检查，
    /// 返回类型无约束（Kang 的 if 是语句而非表达式）。
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

    /// 检查 for 循环: `for var v:T = start, condition, step in { body }`。
    ///
    /// 循环变量在专用作用域中注册为 LoopVar（S3: 循环结束后不可访问）。
    /// 结束条件必须是 bool（T4）。循环体内设置 in_loop 标志供 break/continue 检查。
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

    /// 检查表达式，可选的 expected_type 用于上下文类型推断（如空数组 [] 需要从变量声明推断元素类型）。
    pub fn check_expr(&mut self, expr: &ast::Expr, expected_type: Option<KangType>) -> TypedExpr {
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
            match &e.kind {
                SymbolKind::Variable(kt) => ("var", kt.clone(), e.hint.clone()),
                SymbolKind::Function(_) => ("func", KangType::Void, ScopeHint::Normal),
                _ => ("other", KangType::I32, ScopeHint::Normal),
            }
        });

        match lookup_result {
            Some(("var", kt, hint)) => {
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
            Some(("func", _, _)) => {
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

    /// 检查二元表达式: 算术、比较、逻辑运算的分发点。
    ///
    /// 特殊处理字符串拼接（+ 任一操作数为 str 时自动转字符串拼接）。
    /// 各运算规则:
    ///   - &&/||: 操作数须为 bool（T5）
    ///   - ==/!=: 操作数须同类型（T7/T8）
    ///   - </<=/>/>=: 操作数须同为 i32 或 f64（T2）
    ///  算术运算: 操作数须同为 i32 或 f64（T1）
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

    /// 快速窥探表达式的类型，仅用于类型推断（如字符串拼接判断）。
    ///
    /// 递归处理嵌套表达式但不产生 side effect（不计 passes），
    /// 分离此函数是为了避免在 check_binary 中同时借用 self 的不可变和可变引用。
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
            ast::Expr::Call { func, .. } => {
                // func 是 Box<Expr>，可以是 Ident("f") 或 FieldAccess { obj: Ident("m"), field: "f" }
                match func.as_ref() {
                    ast::Expr::Ident(name, ..) => self
                        .symbols
                        .lookup(name)
                        .and_then(|e| match &e.kind {
                            SymbolKind::Function(sig) => Some(sig.return_type.clone()),
                            _ => None,
                        })
                        .unwrap_or(KangType::I32),
                    _ => KangType::I32,
                }
            }
            ast::Expr::FieldAccess { obj, field, .. } => {
                match self.peek_expr_type(obj) {
                    KangType::Struct(name) => self.structs.get(&name)
                        .and_then(|info| info.fields.iter()
                            .find(|(fn_, _)| *fn_ == *field)
                            .map(|(_, ft)| ft.clone())
                        )
                        .unwrap_or(KangType::I32),
                    _ => KangType::I32,
                }
            }
            ast::Expr::Index { array, .. } => {
                match self.peek_expr_type(array) {
                    KangType::Array(elem) => *elem.clone(),
                    KangType::Str => KangType::Str,
                    _ => KangType::I32,
                }
            }
            ast::Expr::Binary { left, op, right, .. } => {
                match op {
                    BinOp::Eq | BinOp::Neq | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::And | BinOp::Or => KangType::Bool,
                    BinOp::Add => {
                        let lt = self.peek_expr_type(left);
                        let rt = self.peek_expr_type(right);
                        if lt == KangType::Str || rt == KangType::Str {
                            KangType::Str
                        } else if lt == KangType::F64 || rt == KangType::F64 {
                            KangType::F64
                        } else {
                            KangType::I32
                        }
                    }
                    _ => {
                        let lt = self.peek_expr_type(left);
                        if lt == KangType::F64 { KangType::F64 } else { KangType::I32 }
                    }
                }
            }
            ast::Expr::Unary { expr: inner, .. } => self.peek_expr_type(inner),
            ast::Expr::ArrayLit(elems, ..) => {
                if let Some(first) = elems.first() {
                    KangType::Array(Box::new(self.peek_expr_type(first)))
                } else {
                    KangType::Array(Box::new(KangType::I32))
                }
            }
            ast::Expr::StructLit { name, .. } => KangType::Struct(name.clone()),
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

    /// 检查函数调用表达式: `func(args...)` 或 `module.func(args...)`。
    ///
    /// 提取函数名，支持直接调用和 module.func 导入调用。特殊处理 len 和 push 内置函数。
    /// 验证参数数量（F4/F5）和参数类型（F6），返回函数的声明返回值类型。
    fn check_call(&mut self, func: &ast::Expr, args: &[ast::Expr], span: Range<usize>) -> TypedExpr {
        // 提取函数名，支持直接调用和 module.func 导入调用
        // lookup_name: 符号表查找用 (如 m.add)
        // codegen_name: 代码生成用 (如 add，即导入模块中的原名)
        let (lookup_name, codegen_name) = match func {
            ast::Expr::Ident(name, ..) => (name.clone(), name.clone()),
            ast::Expr::FieldAccess { obj, field, .. } => {
                match obj.as_ref() {
                    ast::Expr::Ident(alias, ..) => {
                        let semantic_name = format!("{}.{}", alias, field);
                        // 如果是 import 的函数，用原名查符号表，用原名生成代码
                        let codegen = self.alias_modules.get(&semantic_name).cloned().unwrap_or(semantic_name.clone());
                        (semantic_name, codegen)
                    }
                    _ => {
                        self.error("调用目标必须是函数名或 module.func", "", span);
                        return TypedExpr {
                            kind: TypedExprKind::Call { func_name: "<unknown>".into(), args: vec![] },
                            ty: KangType::I32,
                        };
                    }
                }
            }
            _ => {
                self.error("调用目标必须是函数名或 module.func", "", span);
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
        if lookup_name == "len" {
            return self.check_builtin_len(&codegen_name, &typed_args, &arg_types, span);
        }
        if lookup_name == "push" {
            return self.check_builtin_push(&codegen_name, &typed_args, &arg_types, span);
        }

        // 查找函数签名（用语义名查符号表）
        let sig_data = self.symbols.lookup_function(&lookup_name, &arg_types)
            .map(|sig| (sig.params.clone(), sig.return_type.clone(), sig.params.len()));

        match sig_data {
            Some((params, return_type, param_count)) => {
                // F4/F5: 参数数量
                if param_count != arg_types.len() {
                    self.error(
                        &format!(
                            "函数 \"{}\" 参数数量不匹配: 期望 {} 个，传入 {} 个 (F4/F5)",
                            lookup_name,
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
                                lookup_name,
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
                        func_name: codegen_name.clone(),
                        args: typed_args,
                    },
                    ty: return_type,
                }
            }
            None => {
                self.error(&format!("未定义的函数 \"{}\"", lookup_name), &lookup_name, span);
                TypedExpr {
                    kind: TypedExprKind::Call {
                        func_name: codegen_name.clone(),
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

    /// 检查数组/字符串索引表达式: `arr[i]` 或 `s[i]`。
    ///
    /// 索引必须是 i32（T9/T10）。数组索引返回元素类型，字符串索引返回 str（T12）。
    /// 非数组/字符串类型不能索引。
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

    /// 检查结构体字段访问: `obj.field`。
    ///
    /// obj 必须是结构体类型（ST6），field 必须在结构体中定义（ST7），
    /// 结构体类型必须先定义（ST5）。
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

    /// 检查数组字面量: `[elem1, elem2, ...]`。
    ///
    /// 空数组从期望类型推断元素类型。非空数组要求所有元素类型一致（A2）。
    /// 数组元素不能是 void（A1）。
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

    /// 检查结构体字面量: `TypeName{field1: val1, field2: val2}`。
    ///
    /// 类型必须已定义（ST5）。必须提供所有字段（ST3），不能有多余字段（ST4）。
    /// 各字段值类型必须与声明匹配（T11）。
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

        // ST4: 检查是否有多余字段，使用字段值表达式的 span 而非整体 struct-lit span
        for (pf, pf_expr) in fields {
            if !declared_field_names.contains(&pf.as_str()) {
                self.error(
                    &format!(
                        "结构体 \"{}\" 没有字段 \"{}\" (ST4) — 多余字段",
                        name, pf
                    ),
                    pf,
                span_of_expr(pf_expr));
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
        let (line, col) = self.source_text.as_ref()
            .map(|src| line_col_from_span(src, &span))
            .unwrap_or((0, 0));
        self.errors.push(SemanticError {
            msg: full_msg,
            line,
            col,
            span,
        });
    }
}

/// 根据源码和 byte span 计算 (行号, 列号)，均为 1-based
/// 用于在错误消息中提供精确的源码位置
fn line_col_from_span(source: &str, span: &Range<usize>) -> (usize, usize) {
    let offset = span.start.min(source.len());
    let safe = if source.is_char_boundary(offset) {
        offset
    } else {
        (0..offset).rev().find(|&i| source.is_char_boundary(i)).unwrap_or(0)
    };
    let prefix = &source[..safe];
    let line = prefix.chars().filter(|&c| c == '\n').count() + 1;
    let last_nl = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = prefix[last_nl..].chars().count() + 1;
    (line, col)
}
