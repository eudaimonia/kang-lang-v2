// 语法分析 — 手写递归下降解析器 (LL(1))
// 将 Token 流按 EBNF 文法生成 AST，每个非终结符对应一个 parse_* 函数
// Token 流由 lexer 模块生成，包含 EOF 哨兵

use crate::ast::*;
use crate::error::ParseError;
use crate::lexer::{Token, TokenKind};
use crate::stats::ParseStats;
use std::collections::HashMap;
use std::time::Instant;

// 最大嵌套深度，防止恶意输入导致栈溢出
const MAX_PARSE_DEPTH: usize = 256;

// ── Parser 结构 ─────────────────────────────────────────────────────────────

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Parser { tokens, pos: 0, depth: 0 }
    }

    /// 进入递归层级，超限则报错
    fn enter_depth(&mut self, ctx: &str) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            return Err(self.error(format!(
                "{} 嵌套深度超过限制 {}",
                ctx, MAX_PARSE_DEPTH
            )));
        }
        Ok(())
    }

    fn leave_depth(&mut self) {
        self.depth -= 1;
    }

    // ── 基本操作 ─────────────────────────────────────────────────────────

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn advance(&mut self) -> &Token {
        let t = &self.tokens[self.pos];
        self.pos += 1;
        t
    }

    /// 期望特定 TokenKind，匹配则前进，否则报错
    fn expect(&mut self, expected: &TokenKind) -> Result<(), ParseError> {
        let expected_d = std::mem::discriminant(expected);
        let actual_d = std::mem::discriminant(self.peek_kind());
        if expected_d == actual_d {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.error(format!(
                "期望 {:?}，但得到 {:?}",
                expected, self.peek_kind()
            )))
        }
    }

    /// 如果当前 token 匹配则前进并返回 true
    fn match_kw(&mut self, kw: &TokenKind) -> bool {
        if std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(kw) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn error(&self, msg: String) -> ParseError {
        let t = self.peek();
        ParseError {
            msg,
            line: t.line,
            col: t.col,
            span: t.span.clone(),
        }
    }

    // ── 类型解析 ─────────────────────────────────────────────────────────
    // BaseType = "i32" | "f64" | "str" | "bool" | "void" | IDENT
    // Type     = BaseType | "[" BaseType "]"

    fn parse_base_type(&mut self) -> Result<BaseType, ParseError> {
        match self.peek_kind() {
            TokenKind::TI32 => { self.advance(); Ok(BaseType::I32) }
            TokenKind::TF64 => { self.advance(); Ok(BaseType::F64) }
            TokenKind::TStr => { self.advance(); Ok(BaseType::Str) }
            TokenKind::TBool => { self.advance(); Ok(BaseType::Bool) }
            TokenKind::TVoid => { self.advance(); Ok(BaseType::Void) }
            TokenKind::Ident(name) => {
                let n = name.clone();
                self.advance();
                Ok(BaseType::UserDef(n))
            }
            _ => Err(self.error(format!("期望类型，但得到 {:?}", self.peek_kind()))),
        }
    }

    fn parse_type(&mut self) -> Result<Type, ParseError> {
        if self.match_kw(&TokenKind::LBracket) {
            let bt = self.parse_base_type()?;
            self.expect(&TokenKind::RBracket)?;
            Ok(Type::Array(bt))
        } else {
            Ok(Type::Base(self.parse_base_type()?))
        }
    }

    // ReturnType = Type | "(" Type "," Type ")"
    fn parse_return_type(&mut self) -> Result<ReturnType, ParseError> {
        if self.match_kw(&TokenKind::LParen) {
            let t1 = self.parse_type()?;
            self.expect(&TokenKind::Comma)?;
            let t2 = self.parse_type()?;
            self.expect(&TokenKind::RParen)?;
            Ok(ReturnType::Pair(t1, t2))
        } else {
            Ok(ReturnType::Single(self.parse_type()?))
        }
    }

    // ── 表达式解析 (按优先级) ────────────────────────────────────────────
    // Expr = OrExpr

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.enter_depth("表达式")?;
        let result = self.parse_or_expr();
        self.leave_depth();
        result
    }

    // OrExpr = AndExpr { "||" AndExpr }
    fn parse_or_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and_expr()?;
        while self.match_kw(&TokenKind::OrOr) {
            let right = self.parse_and_expr()?;
            left = Expr::Binary { left: Box::new(left), op: BinOp::Or, right: Box::new(right) };
        }
        Ok(left)
    }

    // AndExpr = EqExpr { "&&" EqExpr }
    fn parse_and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_eq_expr()?;
        while self.match_kw(&TokenKind::AndAnd) {
            let right = self.parse_eq_expr()?;
            left = Expr::Binary { left: Box::new(left), op: BinOp::And, right: Box::new(right) };
        }
        Ok(left)
    }

    // EqExpr = CmpExpr { ("==" | "!=") CmpExpr }
    fn parse_eq_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_cmp_expr()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::EqEq => { self.advance(); BinOp::Eq }
                TokenKind::Neq => { self.advance(); BinOp::Neq }
                _ => break,
            };
            let right = self.parse_cmp_expr()?;
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right) };
        }
        Ok(left)
    }

    // CmpExpr = AddExpr { ("<" | "<=" | ">" | ">=") AddExpr }
    fn parse_cmp_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_add_expr()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Lt => { self.advance(); BinOp::Lt }
                TokenKind::Le => { self.advance(); BinOp::Le }
                TokenKind::Gt => { self.advance(); BinOp::Gt }
                TokenKind::Ge => { self.advance(); BinOp::Ge }
                _ => break,
            };
            let right = self.parse_add_expr()?;
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right) };
        }
        Ok(left)
    }

    // AddExpr = MulExpr { ("+" | "-") MulExpr }
    fn parse_add_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_mul_expr()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Plus => { self.advance(); BinOp::Add }
                TokenKind::Minus => { self.advance(); BinOp::Sub }
                _ => break,
            };
            let right = self.parse_mul_expr()?;
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right) };
        }
        Ok(left)
    }

    // MulExpr = UnaryExpr { ("*" | "/") UnaryExpr }
    fn parse_mul_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary_expr()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Star => { self.advance(); BinOp::Mul }
                TokenKind::Slash => { self.advance(); BinOp::Div }
                _ => break,
            };
            let right = self.parse_unary_expr()?;
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right) };
        }
        Ok(left)
    }

    // UnaryExpr = ("-" | "!") UnaryExpr | PostfixExpr
    fn parse_unary_expr(&mut self) -> Result<Expr, ParseError> {
        if self.match_kw(&TokenKind::Minus) {
            let expr = self.parse_unary_expr()?;
            Ok(Expr::Unary { op: UnaryOp::Neg, expr: Box::new(expr) })
        } else if self.match_kw(&TokenKind::Bang) {
            let expr = self.parse_unary_expr()?;
            Ok(Expr::Unary { op: UnaryOp::Not, expr: Box::new(expr) })
        } else {
            self.parse_postfix_expr()
        }
    }

    // PostfixExpr = Primary { "(" [Args] ")" | "[" Expr "]" | "." IDENT }
    fn parse_postfix_expr(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek_kind() {
                TokenKind::LParen => {
                    self.advance();
                    let args = self.parse_args()?;
                    self.expect(&TokenKind::RParen)?;
                    expr = Expr::Call { func: Box::new(expr), args };
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket)?;
                    expr = Expr::Index { array: Box::new(expr), index: Box::new(index) };
                }
                TokenKind::Dot => {
                    self.advance();
                    let field = self.expect_ident()?;
                    expr = Expr::FieldAccess { obj: Box::new(expr), field };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    // Primary = INT_LIT | FLOAT_LIT | STR_LIT | "true" | "false"
    //         | ArrayLit | StructLit | IDENT | "(" Expr ")"
    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek_kind() {
            TokenKind::IntLit(v) => {
                let val = v.clone();
                self.advance();
                Ok(Expr::IntLit(val))
            }
            TokenKind::FloatLit(v) => {
                let val = v.clone();
                self.advance();
                Ok(Expr::FloatLit(val))
            }
            TokenKind::StrLit(v) => {
                let val = v.clone();
                self.advance();
                Ok(Expr::StrLit(val))
            }
            TokenKind::True => { self.advance(); Ok(Expr::BoolLit(true)) }
            TokenKind::False => { self.advance(); Ok(Expr::BoolLit(false)) }
            TokenKind::Ident(name) => {
                let n = name.clone();
                self.advance();
                // 判断是否为结构体构造: Name { ... }
                if self.peek_kind() == &TokenKind::LBrace {
                    self.parse_struct_lit_tail(&n)
                } else {
                    Ok(Expr::Ident(n))
                }
            }
            // 类型名也可作为函数名出现在调用表达式中 (如 i32("42"))
            TokenKind::TI32 => { self.advance(); self.check_call_or_ident("i32") }
            TokenKind::TF64 => { self.advance(); self.check_call_or_ident("f64") }
            TokenKind::TStr => { self.advance(); self.check_call_or_ident("str") }
            TokenKind::TBool => { self.advance(); self.check_call_or_ident("bool") }
            TokenKind::LBracket => {
                self.advance();
                let elems = self.parse_args()?;
                self.expect(&TokenKind::RBracket)?;
                Ok(Expr::ArrayLit(elems))
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(expr)
            }
            _ => Err(self.error(format!(
                "表达式开头有未预期的 token: {:?}",
                self.peek_kind()
            ))),
        }
    }

    /// 类型关键字后可能接 `{` (结构体构造) 或后缀操作 (函数调用)
    fn check_call_or_ident(&mut self, name: &str) -> Result<Expr, ParseError> {
        if self.peek_kind() == &TokenKind::LBrace {
            self.parse_struct_lit_tail(name)
        } else {
            Ok(Expr::Ident(name.to_string()))
        }
    }

    // StructLit = IDENT "{" [FieldInits] "}"
    fn parse_struct_lit_tail(&mut self, name: &str) -> Result<Expr, ParseError> {
        self.expect(&TokenKind::LBrace)?;
        let fields = if self.peek_kind() == &TokenKind::RBrace {
            vec![]
        } else {
            self.parse_field_inits()?
        };
        self.expect(&TokenKind::RBrace)?;
        Ok(Expr::StructLit { name: name.to_string(), fields })
    }

    // FieldInits = FieldInit { "," FieldInit }
    fn parse_field_inits(&mut self) -> Result<Vec<(String, Expr)>, ParseError> {
        let mut fields = vec![self.parse_field_init()?];
        while self.match_kw(&TokenKind::Comma) {
            fields.push(self.parse_field_init()?);
        }
        Ok(fields)
    }

    // FieldInit = IDENT ":" Expr
    fn parse_field_init(&mut self) -> Result<(String, Expr), ParseError> {
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Colon)?;
        let val = self.parse_expr()?;
        Ok((name, val))
    }

    // ── Args ──────────────────────────────────────────────────────────────

    fn parse_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        if self.is_at_arg_end() {
            return Ok(vec![]);
        }
        let mut args = vec![self.parse_expr()?];
        while self.match_kw(&TokenKind::Comma) {
            args.push(self.parse_expr()?);
        }
        Ok(args)
    }

    /// 参数列表的终止符
    fn is_at_arg_end(&self) -> bool {
        matches!(
            self.peek_kind(),
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace
        )
    }

    // ── 语句解析 ─────────────────────────────────────────────────────────
    // Stmt = VarDecl | AssignStmt | ReturnStmt | IfStmt | ForStmt | ExprStmt | Block

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek_kind() {
            TokenKind::Var => self.parse_var_decl(),
            TokenKind::Return => self.parse_return_stmt(),
            TokenKind::If => self.parse_if_stmt(),
            TokenKind::For => self.parse_for_stmt(),
            TokenKind::LBrace => self.parse_block(),
            _ => self.parse_expr_or_assign(),
        }
    }

    // Block = "{" { Stmt } "}"
    fn parse_block(&mut self) -> Result<Stmt, ParseError> {
        self.enter_depth("语句块")?;
        // 闭包确保 leave_depth 在所有返回路径上执行
        let result = (|| {
            self.expect(&TokenKind::LBrace)?;
            let mut stmts = Vec::new();
            while self.peek_kind() != &TokenKind::RBrace && self.peek_kind() != &TokenKind::Eof {
                stmts.push(self.parse_stmt()?);
            }
            self.expect(&TokenKind::RBrace)?;
            Ok(Stmt::Block(stmts))
        })();
        self.leave_depth();
        result
    }

    // VarDecl = "var" VarBinding [ "," VarBinding ] "=" Expr ";"
    fn parse_var_decl(&mut self) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::Var)?;
        let mut bindings = vec![self.parse_var_binding()?];
        if self.match_kw(&TokenKind::Comma) {
            bindings.push(self.parse_var_binding()?);
        }
        self.expect(&TokenKind::Assign)?;
        let init = self.parse_expr()?;
        self.expect(&TokenKind::Semi)?;
        Ok(Stmt::VarDecl { bindings, init: Box::new(init) })
    }

    // VarBinding = IDENT ":" Type | "_"
    fn parse_var_binding(&mut self) -> Result<VarBinding, ParseError> {
        // 先看是不是 "_"
        if let TokenKind::Ident(name) = self.peek_kind() {
            if name == "_" {
                self.advance();
                return Ok(VarBinding::Discard);
            }
        }
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Colon)?;
        let ty = self.parse_type()?;
        Ok(VarBinding::Named { name, ty })
    }

    // 表达式语句或赋值语句
    // 语法层将任意 Expr 作为左值放行，语义层负责检查 LValue 合法性
    fn parse_expr_or_assign(&mut self) -> Result<Stmt, ParseError> {
        let mark = self.pos;
        // 先解析为表达式
        let expr = self.parse_expr()?;

        // 如果后面是 `=`, 则转为赋值语句
        if self.match_kw(&TokenKind::Assign) {
            let value = self.parse_expr()?;
            self.expect(&TokenKind::Semi)?;
            let lvalue = expr_to_lvalue(expr, mark, self.tokens)?;
            return Ok(Stmt::Assign { lvalue, value: Box::new(value) });
        }

        // 否则是表达式语句，需要分号
        self.expect(&TokenKind::Semi)?;
        Ok(Stmt::Expr(Box::new(expr)))
    }

    // ReturnStmt = "return" [ Expr [ "," Expr ] ] ";"
    fn parse_return_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::Return)?;
        let values = if self.peek_kind() == &TokenKind::Semi {
            vec![]
        } else {
            let mut vals = vec![self.parse_expr()?];
            if self.match_kw(&TokenKind::Comma) {
                vals.push(self.parse_expr()?);
            }
            vals
        };
        self.expect(&TokenKind::Semi)?;
        Ok(Stmt::Return { values })
    }

    // IfStmt = "if" Expr "then" Stmt [ "else" Stmt ]
    fn parse_if_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::If)?;
        let condition = self.parse_expr()?;
        self.expect(&TokenKind::Then)?;
        let then_branch = self.parse_stmt()?;
        let else_branch = if self.match_kw(&TokenKind::Else) {
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };
        Ok(Stmt::If { condition: Box::new(condition), then_branch: Box::new(then_branch), else_branch })
    }

    // ForStmt = "for" "var" IDENT ":" Type "=" Expr ","
    //           Expr ","
    //           AssignStmtNoSemi
    //           "in" Block
    fn parse_for_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::For)?;
        self.expect(&TokenKind::Var)?;
        let var_name = self.expect_ident()?;
        self.expect(&TokenKind::Colon)?;
        let var_type = self.parse_type()?;
        self.expect(&TokenKind::Assign)?;
        let start = self.parse_expr()?;
        self.expect(&TokenKind::Comma)?;
        let end = self.parse_expr()?;
        self.expect(&TokenKind::Comma)?;
        // step: 赋值语句不带分号
        let step_mark = self.pos;
        let step_expr_full = self.parse_expr()?;
        self.expect(&TokenKind::Assign)?;
        let step_val = self.parse_expr()?;
        let step_lvalue = expr_to_lvalue(step_expr_full, step_mark, self.tokens)?;
        self.expect(&TokenKind::In)?;
        let body = self.parse_block()?;
        Ok(Stmt::For {
            var_name,
            var_type,
            start: Box::new(start),
            end: Box::new(end),
            step_lvalue,
            step_expr: Box::new(step_val),
            body: Box::new(body),
        })
    }

    // ── 顶层解析 ─────────────────────────────────────────────────────────
    // StructDef = "struct" IDENT "{" { Field } "}"
    // Field     = IDENT ":" Type ";"

    fn parse_struct_def(&mut self) -> Result<StructDef, ParseError> {
        self.expect(&TokenKind::Struct)?;
        let name = self.expect_ident()?;
        self.expect(&TokenKind::LBrace)?;
        let mut fields = Vec::new();
        while self.peek_kind() != &TokenKind::RBrace && self.peek_kind() != &TokenKind::Eof {
            let field_name = self.expect_ident()?;
            self.expect(&TokenKind::Colon)?;
            let field_type = self.parse_type()?;
            self.expect(&TokenKind::Semi)?;
            fields.push((field_name, field_type));
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(StructDef { name, fields })
    }

    // FuncDef = "def" IDENT "(" [ Params ] ")" "->" ReturnType Block
    fn parse_func_def(&mut self) -> Result<FuncDef, ParseError> {
        self.expect(&TokenKind::Def)?;
        let name = self.expect_ident()?;
        self.expect(&TokenKind::LParen)?;
        let params = if self.peek_kind() == &TokenKind::RParen {
            vec![]
        } else {
            self.parse_params()?
        };
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::Arrow)?;
        let return_type = self.parse_return_type()?;
        let body = match self.parse_block()? {
            Stmt::Block(stmts) => stmts,
            _ => unreachable!(),
        };
        Ok(FuncDef { name, params, return_type, body })
    }

    // Params = Param { "," Param }
    fn parse_params(&mut self) -> Result<Vec<(String, Type)>, ParseError> {
        let mut params = vec![self.parse_param()?];
        while self.match_kw(&TokenKind::Comma) {
            params.push(self.parse_param()?);
        }
        Ok(params)
    }

    // Param = IDENT ":" Type
    fn parse_param(&mut self) -> Result<(String, Type), ParseError> {
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Colon)?;
        let ty = self.parse_type()?;
        Ok((name, ty))
    }

    // Program = { TopLevel }
    fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut items = Vec::new();
        while self.peek_kind() != &TokenKind::Eof {
            let item = match self.peek_kind() {
                TokenKind::Struct => TopLevel::Struct(self.parse_struct_def()?),
                TokenKind::Def => TopLevel::Func(self.parse_func_def()?),
                _ => {
                    return Err(self.error(format!(
                        "期望 struct 或 def，但得到 {:?}",
                        self.peek_kind()
                    )));
                }
            };
            items.push(item);
        }
        Ok(Program { items })
    }

    // ── 辅助 ──────────────────────────────────────────────────────────────

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek_kind() {
            TokenKind::Ident(name) => {
                let n = name.clone();
                self.advance();
                Ok(n)
            }
            _ => Err(self.error(format!("期望标识符，但得到 {:?}", self.peek_kind()))),
        }
    }
}

// ── 左值转换 ────────────────────────────────────────────────────────────────

/// 将解析好的表达式转换为左值
/// 语法层在此检查 LValue 形式合法性(AS3/AS4)，不合法的左值在解析阶段直接拒绝
fn expr_to_lvalue(expr: Expr, mark: usize, tokens: &[Token]) -> Result<LValue, ParseError> {
    match expr {
        Expr::Ident(name) => Ok(LValue::Ident(name)),
        Expr::Index { array, index } => Ok(LValue::Index { array, index }),
        Expr::FieldAccess { obj, field } => Ok(LValue::FieldAccess { obj, field }),
        _ => {
            let t = &tokens[mark];
            Err(ParseError {
                msg: format!("赋值左侧必须是变量、索引或字段访问，但得到表达式"),
                line: t.line,
                col: t.col,
                span: t.span.clone(),
            })
        }
    }
}

// ── 统计收集 ────────────────────────────────────────────────────────────────

/// 计算 AST 深度
fn ast_depth(program: &Program) -> usize {
    fn expr_depth(e: &Expr) -> usize {
        match e {
            Expr::Binary { left, right, .. } => 1 + expr_depth(left).max(expr_depth(right)),
            Expr::Unary { expr, .. } => 1 + expr_depth(expr),
            Expr::Call { func, args } => {
                let arg_max = args.iter().map(|a| expr_depth(a)).max().unwrap_or(0);
                1 + expr_depth(func).max(arg_max)
            }
            Expr::Index { array, index } => 1 + expr_depth(array).max(expr_depth(index)),
            Expr::FieldAccess { obj, .. } => 1 + expr_depth(obj),
            Expr::StructLit { fields, .. } => {
                1 + fields.iter().map(|(_, v)| expr_depth(v)).max().unwrap_or(0)
            }
            Expr::ArrayLit(elems) => {
                1 + elems.iter().map(|e| expr_depth(e)).max().unwrap_or(0)
            }
            Expr::IntLit(_) | Expr::FloatLit(_) | Expr::StrLit(_)
            | Expr::BoolLit(_) | Expr::Ident(_) => 1,
        }
    }

    fn stmt_depth(s: &Stmt) -> usize {
        match s {
            Stmt::VarDecl { bindings: _, init } => 1 + expr_depth(init),
            Stmt::Assign { lvalue: _, value } => 1 + expr_depth(value),
            Stmt::Return { values } => {
                1 + values.iter().map(|v| expr_depth(v)).max().unwrap_or(0)
            }
            Stmt::If { condition, then_branch, else_branch } => {
                let else_d = else_branch.as_ref().map(|s| stmt_depth(s)).unwrap_or(0);
                1 + expr_depth(condition).max(stmt_depth(then_branch)).max(else_d)
            }
            Stmt::For { start, end, step_expr, body, .. } => {
                1 + expr_depth(start).max(expr_depth(end))
                    .max(expr_depth(step_expr))
                    .max(stmt_depth(body))
            }
            Stmt::Expr(e) => 1 + expr_depth(e),
            Stmt::Block(stmts) => {
                1 + stmts.iter().map(|s| stmt_depth(s)).max().unwrap_or(0)
            }
        }
    }

    program.items.iter().map(|item| {
        match item {
            TopLevel::Struct(_) => 2, // struct-def + fields
            TopLevel::Func(f) => {
                2 + f.body.iter().map(|s| stmt_depth(s)).max().unwrap_or(0)
            }
        }
    }).max().unwrap_or(0)
}

/// 计算各类型 AST 节点数
fn count_nodes(program: &Program) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for item in &program.items {
        match item {
            TopLevel::Struct(_) => *counts.entry("struct-def".into()).or_insert(0) += 1,
            TopLevel::Func(f) => {
                *counts.entry("func-def".into()).or_insert(0) += 1;
                count_stmt_nodes(&f.body, &mut counts);
            }
        }
    }
    counts
}

fn count_stmt_nodes(stmts: &[Stmt], counts: &mut HashMap<String, usize>) {
    for s in stmts {
        match s {
            Stmt::VarDecl { bindings: _, init } => {
                *counts.entry("var-decl".into()).or_insert(0) += 1;
                count_expr_nodes(init, counts);
            }
            Stmt::Assign { lvalue: _, value } => {
                *counts.entry("assign".into()).or_insert(0) += 1;
                count_expr_nodes(value, counts);
            }
            Stmt::Return { values } => {
                *counts.entry("return".into()).or_insert(0) += 1;
                for v in values { count_expr_nodes(v, counts); }
            }
            Stmt::If { condition, then_branch, else_branch } => {
                *counts.entry("if".into()).or_insert(0) += 1;
                count_expr_nodes(condition, counts);
                count_stmt_nodes(std::slice::from_ref(then_branch), counts);
                if let Some(else_s) = else_branch {
                    count_stmt_nodes(std::slice::from_ref(else_s), counts);
                }
            }
            Stmt::For { start, end, step_expr, body, .. } => {
                *counts.entry("for".into()).or_insert(0) += 1;
                count_expr_nodes(start, counts);
                count_expr_nodes(end, counts);
                count_expr_nodes(step_expr, counts);
                count_stmt_nodes(std::slice::from_ref(body), counts);
            }
            Stmt::Expr(e) => {
                *counts.entry("expr-stmt".into()).or_insert(0) += 1;
                count_expr_nodes(e, counts);
            }
            Stmt::Block(inner) => {
                *counts.entry("block".into()).or_insert(0) += 1;
                count_stmt_nodes(inner, counts);
            }
        }
    }
}

fn count_expr_nodes(e: &Expr, counts: &mut HashMap<String, usize>) {
    match e {
        Expr::Binary { left, right, .. } => {
            *counts.entry("binary".into()).or_insert(0) += 1;
            count_expr_nodes(left, counts);
            count_expr_nodes(right, counts);
        }
        Expr::Unary { expr, .. } => {
            *counts.entry("unary".into()).or_insert(0) += 1;
            count_expr_nodes(expr, counts);
        }
        Expr::Call { func, args } => {
            *counts.entry("call".into()).or_insert(0) += 1;
            count_expr_nodes(func, counts);
            for a in args { count_expr_nodes(a, counts); }
        }
        Expr::Index { array, index } => {
            *counts.entry("index".into()).or_insert(0) += 1;
            count_expr_nodes(array, counts);
            count_expr_nodes(index, counts);
        }
        Expr::FieldAccess { obj, .. } => {
            *counts.entry("field-access".into()).or_insert(0) += 1;
            count_expr_nodes(obj, counts);
        }
        Expr::IntLit(_) => { *counts.entry("int-lit".into()).or_insert(0) += 1; }
        Expr::FloatLit(_) => { *counts.entry("float-lit".into()).or_insert(0) += 1; }
        Expr::StrLit(_) => { *counts.entry("str-lit".into()).or_insert(0) += 1; }
        Expr::BoolLit(_) => { *counts.entry("bool-lit".into()).or_insert(0) += 1; }
        Expr::Ident(_) => { *counts.entry("ident".into()).or_insert(0) += 1; }
        Expr::ArrayLit(elems) => {
            *counts.entry("array-lit".into()).or_insert(0) += 1;
            for e in elems { count_expr_nodes(e, counts); }
        }
        Expr::StructLit { fields, .. } => {
            *counts.entry("struct-lit".into()).or_insert(0) += 1;
            for (_, v) in fields { count_expr_nodes(v, counts); }
        }
    }
}

// ── 公共入口 ────────────────────────────────────────────────────────────────

/// 将 token 流解析为 AST，同时收集统计数据
pub fn parse(tokens: &[Token], stats: &mut ParseStats) -> Result<Program, ParseError> {
    let start = Instant::now();
    let mut parser = Parser::new(tokens);
    let program = parser.parse_program()?;

    stats.duration_us = start.elapsed().as_micros() as u64;
    stats.ast_max_depth = ast_depth(&program);
    stats.node_counts_by_kind = count_nodes(&program);
    stats.ast_node_count = stats.node_counts_by_kind.values().sum();
    stats.func_count = program.items.iter().filter(|i| matches!(i, TopLevel::Func(_))).count();
    stats.struct_count = program.items.iter().filter(|i| matches!(i, TopLevel::Struct(_))).count();

    Ok(program)
}

// ── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{self, LexStats};
    use std::collections::HashMap;

    /// 辅助: 源码 → AST
    fn parse_source(source: &str) -> Result<Program, ParseError> {
        let mut lex_stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let tokens = lexer::tokenize(source, &mut lex_stats).unwrap();
        let mut stats = ParseStats {
            duration_us: 0, ast_node_count: 0, ast_max_depth: 0,
            node_counts_by_kind: HashMap::new(), func_count: 0, struct_count: 0,
        };
        parse(&tokens, &mut stats)
    }

    /// 辅助: 源码 → 单条语句
    fn parse_stmt(source: &str) -> Result<Stmt, ParseError> {
        // 包裹成一个函数体以便解析
        let full = format!("def _test() -> void {{ {} }}", source);
        let program = parse_source(&full)?;
        match &program.items[0] {
            TopLevel::Func(f) => Ok(f.body[0].clone()),
            _ => unreachable!(),
        }
    }

    /// 辅助: 源码 → 表达式
    fn parse_expr(source: &str) -> Result<Expr, ParseError> {
        // 放在 return 语句中解析
        let full = format!("def _test() -> i32 {{ return {}; }}", source);
        let program = parse_source(&full)?;
        match &program.items[0] {
            TopLevel::Func(f) => match &f.body[0] {
                Stmt::Return { values } => Ok(values[0].clone()),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    }

    // ── 类型解析 ─────────────────────────────────────────────────────────

    #[test]
    fn parse_base_i32() {
        let p = parse_source("def f() -> i32 { return 0; }").unwrap();
        let f = match &p.items[0] { TopLevel::Func(f) => f, _ => unreachable!() };
        assert_eq!(f.return_type, ReturnType::Single(Type::Base(BaseType::I32)));
    }

    #[test]
    fn parse_base_f64() {
        let p = parse_source("def f() -> f64 { return 0.0; }").unwrap();
        let f = match &p.items[0] { TopLevel::Func(f) => f, _ => unreachable!() };
        assert_eq!(f.return_type, ReturnType::Single(Type::Base(BaseType::F64)));
    }

    #[test]
    fn parse_type_array() {
        let p = parse_source("def f(a:[i32]) -> void { return; }").unwrap();
        let f = match &p.items[0] { TopLevel::Func(f) => f, _ => unreachable!() };
        assert_eq!(f.params[0].1, Type::Array(BaseType::I32));
    }

    #[test]
    fn parse_ret_type_pair() {
        let p = parse_source("def f() -> (i32, bool) { return 1, true; }").unwrap();
        let f = match &p.items[0] { TopLevel::Func(f) => f, _ => unreachable!() };
        assert_eq!(
            f.return_type,
            ReturnType::Pair(Type::Base(BaseType::I32), Type::Base(BaseType::Bool))
        );
    }

    #[test]
    fn parse_user_type() {
        let p = parse_source("def f(x:MyType) -> void { return; }").unwrap();
        let f = match &p.items[0] { TopLevel::Func(f) => f, _ => unreachable!() };
        assert_eq!(f.params[0].1, Type::Base(BaseType::UserDef("MyType".into())));
    }

    // ── 表达式: 字面量 ───────────────────────────────────────────────────

    #[test] fn expr_int_lit()    { assert_eq!(parse_expr("42").unwrap(), Expr::IntLit("42".into())); }
    #[test] fn expr_float_lit()  { assert_eq!(parse_expr("3.14").unwrap(), Expr::FloatLit("3.14".into())); }
    #[test] fn expr_str_lit()    { assert_eq!(parse_expr("\"hi\"").unwrap(), Expr::StrLit("hi".into())); }
    #[test] fn expr_true()       { assert_eq!(parse_expr("true").unwrap(), Expr::BoolLit(true)); }
    #[test] fn expr_false()      { assert_eq!(parse_expr("false").unwrap(), Expr::BoolLit(false)); }
    #[test] fn expr_ident()      { assert_eq!(parse_expr("x").unwrap(), Expr::Ident("x".into())); }

    // ── 表达式: 二元运算符(优先级) ────────────────────────────────────────

    #[test]
    fn expr_binary_add() {
        assert_eq!(
            parse_expr("a + b").unwrap(),
            Expr::Binary {
                left: Box::new(Expr::Ident("a".into())),
                op: BinOp::Add,
                right: Box::new(Expr::Ident("b".into())),
            }
        );
    }

    #[test]
    fn expr_binary_sub_mul_precedence() {
        // a + b * c  ≡  a + (b * c)
        let e = parse_expr("a + b * c").unwrap();
        assert_eq!(
            e,
            Expr::Binary {
                left: Box::new(Expr::Ident("a".into())),
                op: BinOp::Add,
                right: Box::new(Expr::Binary {
                    left: Box::new(Expr::Ident("b".into())),
                    op: BinOp::Mul,
                    right: Box::new(Expr::Ident("c".into())),
                }),
            }
        );
    }

    #[test]
    fn expr_mul_before_add() {
        // a * b + c  ≡  (a * b) + c
        let e = parse_expr("a * b + c").unwrap();
        assert_eq!(
            e,
            Expr::Binary {
                left: Box::new(Expr::Binary {
                    left: Box::new(Expr::Ident("a".into())),
                    op: BinOp::Mul,
                    right: Box::new(Expr::Ident("b".into())),
                }),
                op: BinOp::Add,
                right: Box::new(Expr::Ident("c".into())),
            }
        );
    }

    #[test]
    fn expr_paren_overrides_precedence() {
        // (a + b) * c  — 不同于 a + b * c
        let e = parse_expr("(a + b) * c").unwrap();
        assert_eq!(
            e,
            Expr::Binary {
                left: Box::new(Expr::Binary {
                    left: Box::new(Expr::Ident("a".into())),
                    op: BinOp::Add,
                    right: Box::new(Expr::Ident("b".into())),
                }),
                op: BinOp::Mul,
                right: Box::new(Expr::Ident("c".into())),
            }
        );
    }

    #[test]
    fn expr_cmp_before_eq() {
        // a < b == true  ≡  (a < b) == true
        let e = parse_expr("a < b == true").unwrap();
        assert_eq!(e, Expr::Binary {
            left: Box::new(Expr::Binary {
                left: Box::new(Expr::Ident("a".into())),
                op: BinOp::Lt,
                right: Box::new(Expr::Ident("b".into())),
            }),
            op: BinOp::Eq,
            right: Box::new(Expr::BoolLit(true)),
        });
    }

    #[test]
    fn expr_or_below_and() {
        // x || y && false  ≡  x || (y && false)
        let e = parse_expr("x || y && false").unwrap();
        assert_eq!(e, Expr::Binary {
            left: Box::new(Expr::Ident("x".into())),
            op: BinOp::Or,
            right: Box::new(Expr::Binary {
                left: Box::new(Expr::Ident("y".into())),
                op: BinOp::And,
                right: Box::new(Expr::BoolLit(false)),
            }),
        });
    }

    #[test]
    fn expr_comparison_chain() {
        // a < b < c  ≡  (a < b) < c  (左结合)
        let e = parse_expr("a < b < c").unwrap();
        match e {
            Expr::Binary { left, op: BinOp::Lt, right } => {
                assert_eq!(*left, Expr::Binary {
                    left: Box::new(Expr::Ident("a".into())),
                    op: BinOp::Lt,
                    right: Box::new(Expr::Ident("b".into())),
                });
                assert_eq!(*right, Expr::Ident("c".into()));
            }
            _ => panic!("expected binary"),
        }
    }

    // ── 表达式: 一元运算符 ────────────────────────────────────────────────

    #[test]
    fn expr_neg() {
        assert_eq!(
            parse_expr("-a").unwrap(),
            Expr::Unary { op: UnaryOp::Neg, expr: Box::new(Expr::Ident("a".into())) }
        );
    }

    #[test]
    fn expr_not() {
        assert_eq!(
            parse_expr("!x").unwrap(),
            Expr::Unary { op: UnaryOp::Not, expr: Box::new(Expr::Ident("x".into())) }
        );
    }

    #[test]
    fn expr_double_neg() {
        assert_eq!(
            parse_expr("--42").unwrap(),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(Expr::IntLit("42".into())),
                }),
            }
        );
    }

    #[test]
    fn expr_neg_mul_precedence() {
        // -a * b  ≡  (-a) * b
        let e = parse_expr("-a * b").unwrap();
        assert_eq!(e, Expr::Binary {
            left: Box::new(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(Expr::Ident("a".into())),
            }),
            op: BinOp::Mul,
            right: Box::new(Expr::Ident("b".into())),
        });
    }

    // ── 表达式: 后缀操作 ─────────────────────────────────────────────────

    #[test]
    fn expr_call_no_args() {
        assert_eq!(
            parse_expr("f()").unwrap(),
            Expr::Call { func: Box::new(Expr::Ident("f".into())), args: vec![] }
        );
    }

    #[test]
    fn expr_call_one_arg() {
        assert_eq!(
            parse_expr("f(1)").unwrap(),
            Expr::Call {
                func: Box::new(Expr::Ident("f".into())),
                args: vec![Expr::IntLit("1".into())],
            }
        );
    }

    #[test]
    fn expr_call_multi_args() {
        let e = parse_expr("add(1, 2, 3)").unwrap();
        match e {
            Expr::Call { func, args } => {
                assert_eq!(*func, Expr::Ident("add".into()));
                assert_eq!(args.len(), 3);
            }
            _ => panic!("expected call"),
        }
    }

    #[test]
    fn expr_index() {
        assert_eq!(
            parse_expr("arr[0]").unwrap(),
            Expr::Index {
                array: Box::new(Expr::Ident("arr".into())),
                index: Box::new(Expr::IntLit("0".into())),
            }
        );
    }

    #[test]
    fn expr_field_access() {
        assert_eq!(
            parse_expr("obj.field").unwrap(),
            Expr::FieldAccess {
                obj: Box::new(Expr::Ident("obj".into())),
                field: "field".into(),
            }
        );
    }

    #[test]
    fn expr_chained_field_access() {
        // a.b.c
        let e = parse_expr("a.b.c").unwrap();
        assert_eq!(e, Expr::FieldAccess {
            obj: Box::new(Expr::FieldAccess {
                obj: Box::new(Expr::Ident("a".into())),
                field: "b".into(),
            }),
            field: "c".into(),
        });
    }

    #[test]
    fn expr_call_chained() {
        // f().g(1).h
        let e = parse_expr("f().g(1).h").unwrap();
        assert_eq!(e, Expr::FieldAccess {
            obj: Box::new(Expr::Call {
                func: Box::new(Expr::FieldAccess {
                    obj: Box::new(Expr::Call {
                        func: Box::new(Expr::Ident("f".into())),
                        args: vec![],
                    }),
                    field: "g".into(),
                }),
                args: vec![Expr::IntLit("1".into())],
            }),
            field: "h".into(),
        });
    }

    // ── 表达式: 数组/结构体字面量 ─────────────────────────────────────────

    #[test]
    fn expr_array_lit_empty() {
        assert_eq!(parse_expr("[]").unwrap(), Expr::ArrayLit(vec![]));
    }

    #[test]
    fn expr_array_lit_elements() {
        let e = parse_expr("[1, 2, 3]").unwrap();
        match e {
            Expr::ArrayLit(elems) => assert_eq!(elems.len(), 3),
            _ => panic!("expected array lit"),
        }
    }

    #[test]
    fn expr_struct_lit_empty() {
        let e = parse_expr("Point{}").unwrap();
        match e {
            Expr::StructLit { name, fields } => {
                assert_eq!(name, "Point");
                assert!(fields.is_empty());
            }
            _ => panic!("expected struct lit"),
        }
    }

    #[test]
    fn expr_struct_lit_with_fields() {
        let e = parse_expr("Point{x: 1, y: 2}").unwrap();
        match e {
            Expr::StructLit { name, fields } => {
                assert_eq!(name, "Point");
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].0, "x");
                assert_eq!(fields[1].0, "y");
            }
            _ => panic!("expected struct lit"),
        }
    }

    // ── 语句解析 ─────────────────────────────────────────────────────────

    #[test]
    fn stmt_var_decl_single() {
        let s = parse_stmt("var x:i32 = 42;").unwrap();
        match s {
            Stmt::VarDecl { bindings, init } => {
                assert_eq!(bindings.len(), 1);
                assert_eq!(bindings[0], VarBinding::Named { name: "x".into(), ty: Type::Base(BaseType::I32) });
                assert_eq!(*init, Expr::IntLit("42".into()));
            }
            _ => panic!("expected var decl"),
        }
    }

    #[test]
    fn stmt_var_decl_double() {
        let s = parse_stmt("var a:i32, b:i32 = f();").unwrap();
        match s {
            Stmt::VarDecl { bindings, init: _ } => {
                assert_eq!(bindings.len(), 2);
                assert_eq!(bindings[0], VarBinding::Named { name: "a".into(), ty: Type::Base(BaseType::I32) });
                assert_eq!(bindings[1], VarBinding::Named { name: "b".into(), ty: Type::Base(BaseType::I32) });
            }
            _ => panic!("expected var decl"),
        }
    }

    #[test]
    fn stmt_var_decl_discard() {
        let s = parse_stmt("var _, r:i32 = f();").unwrap();
        match s {
            Stmt::VarDecl { bindings, .. } => {
                assert_eq!(bindings.len(), 2);
                assert_eq!(bindings[0], VarBinding::Discard);
            }
            _ => panic!("expected var decl"),
        }
    }

    #[test]
    fn stmt_assign() {
        let s = parse_stmt("x = 42;").unwrap();
        match s {
            Stmt::Assign { lvalue, value } => {
                assert_eq!(lvalue, LValue::Ident("x".into()));
                assert_eq!(*value, Expr::IntLit("42".into()));
            }
            _ => panic!("expected assign"),
        }
    }

    #[test]
    fn stmt_assign_index() {
        let s = parse_stmt("arr[i] = 5;").unwrap();
        match s {
            Stmt::Assign { lvalue, .. } => {
                assert_eq!(lvalue, LValue::Index {
                    array: Box::new(Expr::Ident("arr".into())),
                    index: Box::new(Expr::Ident("i".into())),
                });
            }
            _ => panic!("expected assign"),
        }
    }

    #[test]
    fn stmt_assign_field() {
        let s = parse_stmt("obj.field = 5;").unwrap();
        match s {
            Stmt::Assign { lvalue, .. } => {
                assert_eq!(lvalue, LValue::FieldAccess {
                    obj: Box::new(Expr::Ident("obj".into())),
                    field: "field".into(),
                });
            }
            _ => panic!("expected assign"),
        }
    }

    #[test]
    fn stmt_return_void() {
        let s = parse_stmt("return;").unwrap();
        assert_eq!(s, Stmt::Return { values: vec![] });
    }

    #[test]
    fn stmt_return_single() {
        let s = parse_stmt("return 0;").unwrap();
        assert_eq!(s, Stmt::Return { values: vec![Expr::IntLit("0".into())] });
    }

    #[test]
    fn stmt_return_pair() {
        let s = parse_stmt("return 1, true;").unwrap();
        assert_eq!(s, Stmt::Return {
            values: vec![Expr::IntLit("1".into()), Expr::BoolLit(true)],
        });
    }

    #[test]
    fn stmt_if_simple() {
        let s = parse_stmt("if x then return;").unwrap();
        match s {
            Stmt::If { condition, then_branch, else_branch } => {
                assert_eq!(*condition, Expr::Ident("x".into()));
                assert_eq!(*then_branch, Stmt::Return { values: vec![] });
                assert!(else_branch.is_none());
            }
            _ => panic!("expected if"),
        }
    }

    #[test]
    fn stmt_if_else() {
        let s = parse_stmt("if x then return 0; else return 1;").unwrap();
        match s {
            Stmt::If { condition, then_branch: _, else_branch } => {
                assert_eq!(*condition, Expr::Ident("x".into()));
                assert!(else_branch.is_some());
            }
            _ => panic!("expected if"),
        }
    }

    #[test]
    fn stmt_block_empty() {
        let s = parse_stmt("{}").unwrap();
        assert_eq!(s, Stmt::Block(vec![]));
    }

    #[test]
    fn stmt_block_multiple() {
        let s = parse_stmt("{ return; return 1; }").unwrap();
        match s {
            Stmt::Block(stmts) => assert_eq!(stmts.len(), 2),
            _ => panic!("expected block"),
        }
    }

    #[test]
    fn stmt_expr_stmt() {
        let s = parse_stmt("puts(\"hi\");").unwrap();
        match s {
            Stmt::Expr(_) => {} // 仅验证解析不报错
            _ => panic!("expected expr stmt"),
        }
    }

    #[test]
    fn stmt_for_loop() {
        let source = "for var i:i32 = 0, i < 10, i = i + 1 in { }";
        let s = parse_stmt(source).unwrap();
        match s {
            Stmt::For { var_name, var_type, start, end, step_lvalue, step_expr: _, body } => {
                assert_eq!(var_name, "i");
                assert_eq!(var_type, Type::Base(BaseType::I32));
                assert_eq!(*start, Expr::IntLit("0".into()));
                assert_eq!(*end, Expr::Binary {
                    left: Box::new(Expr::Ident("i".into())),
                    op: BinOp::Lt,
                    right: Box::new(Expr::IntLit("10".into())),
                });
                assert_eq!(step_lvalue, LValue::Ident("i".into()));
                assert_eq!(*body, Stmt::Block(vec![]));
            }
            _ => panic!("expected for"),
        }
    }

    // ── 顶层解析 ─────────────────────────────────────────────────────────

    #[test]
    fn top_struct_def() {
        let p = parse_source("struct Point { x: i32; y: i32; }").unwrap();
        assert_eq!(p.items.len(), 1);
        match &p.items[0] {
            TopLevel::Struct(s) => {
                assert_eq!(s.name, "Point");
                assert_eq!(s.fields.len(), 2);
                assert_eq!(s.fields[0].0, "x");
            }
            _ => panic!("expected struct"),
        }
    }

    #[test]
    fn top_func_def() {
        let p = parse_source("def main() -> i32 { return 0; }").unwrap();
        assert_eq!(p.items.len(), 1);
        match &p.items[0] {
            TopLevel::Func(f) => {
                assert_eq!(f.name, "main");
                assert!(f.params.is_empty());
                assert_eq!(f.return_type, ReturnType::Single(Type::Base(BaseType::I32)));
            }
            _ => panic!("expected func"),
        }
    }

    #[test]
    fn top_func_with_params() {
        let p = parse_source("def add(a:i32, b:i32) -> i32 { return a + b; }").unwrap();
        match &p.items[0] {
            TopLevel::Func(f) => {
                assert_eq!(f.params.len(), 2);
                assert_eq!(f.params[0].0, "a");
                assert_eq!(f.params[1].0, "b");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn top_multiple_items() {
        let p = parse_source("struct A {} def f() -> void { return; } struct B {}").unwrap();
        assert_eq!(p.items.len(), 3);
    }

    // ── 嵌套结构 ─────────────────────────────────────────────────────────

    #[test]
    fn nested_blocks() {
        let p = parse_source("def f() -> i32 { { { return 1; } } }").unwrap();
        match &p.items[0] {
            TopLevel::Func(f) => {
                match &f.body[0] {
                    Stmt::Block(outer) => {
                        match &outer[0] {
                            Stmt::Block(inner) => {
                                assert!(matches!(inner[0], Stmt::Return { .. }));
                            }
                            _ => panic!("expected inner block"),
                        }
                    }
                    _ => panic!("expected outer block"),
                }
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn if_in_if() {
        let s = parse_stmt("if x then if y then return 0; else return 1;").unwrap();
        match s {
            Stmt::If { then_branch, .. } => {
                assert!(matches!(*then_branch, Stmt::If { .. }));
            }
            _ => panic!("expected if"),
        }
    }

    // ── 边界情况 ─────────────────────────────────────────────────────────

    #[test]
    fn empty_program() {
        let p = parse_source("").unwrap();
        assert!(p.items.is_empty());
    }

    #[test]
    fn empty_struct() {
        let p = parse_source("struct Empty {}").unwrap();
        match &p.items[0] {
            TopLevel::Struct(s) => assert!(s.fields.is_empty()),
            _ => panic!("expected struct"),
        }
    }

    #[test]
    fn func_no_params() {
        let p = parse_source("def f() -> void { return; }").unwrap();
        match &p.items[0] {
            TopLevel::Func(f) => assert!(f.params.is_empty()),
            _ => unreachable!(),
        }
    }

    #[test]
    fn expr_int_negative() {
        // -1 是一元负号作用于 1
        let e = parse_expr("-1").unwrap();
        assert_eq!(e, Expr::Unary { op: UnaryOp::Neg, expr: Box::new(Expr::IntLit("1".into())) });
    }

    // ── 错误路径 ─────────────────────────────────────────────────────────

    #[test]
    fn error_missing_semicolon() {
        // return x } — x 是合法表达式, 但缺少分号
        let result = parse_source("def f() -> i32 { return x }");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.msg.contains("期望 Semi"), "msg: {}", err.msg);
    }

    #[test]
    fn error_unexpected_token_at_top() {
        let result = parse_source("42");
        assert!(result.is_err());
        assert!(result.unwrap_err().msg.contains("struct 或 def"));
    }

    #[test]
    fn error_missing_rparen() {
        let result = parse_source("def f( -> void { return; }");
        assert!(result.is_err());
    }

    #[test]
    fn error_wrong_close() {
        let result = parse_source("def f() -> void { return; ]");
        assert!(result.is_err());
    }

    #[test]
    fn error_bad_assignment_target() {
        // 42 = x; — 语法层会将 42 解析为表达式, 转左值时失败
        let result = parse_stmt("42 = x;");
        assert!(result.is_err());
        assert!(result.unwrap_err().msg.contains("赋值左侧"));
    }

    #[test]
    fn error_unterminated_block() {
        let result = parse_source("def f() -> void { return;");
        assert!(result.is_err());
    }

    #[test]
    fn error_missing_arrow() {
        let result = parse_source("def f() i32 { return 0; }");
        assert!(result.is_err());
    }

    // ── 统计 ─────────────────────────────────────────────────────────────

    #[test]
    fn stats_func_count() {
        let source = "def f() -> void { return; } def g() -> i32 { return 0; }";
        let mut lex_stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let tokens = lexer::tokenize(source, &mut lex_stats).unwrap();
        let mut stats = ParseStats {
            duration_us: 0, ast_node_count: 0, ast_max_depth: 0,
            node_counts_by_kind: HashMap::new(), func_count: 0, struct_count: 0,
        };
        parse(&tokens, &mut stats).unwrap();
        assert_eq!(stats.func_count, 2);
        assert_eq!(stats.struct_count, 0);
    }

    #[test]
    fn stats_struct_count() {
        let source = "struct A { x: i32; } struct B { y: f64; }";
        let mut lex_stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let tokens = lexer::tokenize(source, &mut lex_stats).unwrap();
        let mut stats = ParseStats {
            duration_us: 0, ast_node_count: 0, ast_max_depth: 0,
            node_counts_by_kind: HashMap::new(), func_count: 0, struct_count: 0,
        };
        parse(&tokens, &mut stats).unwrap();
        assert_eq!(stats.struct_count, 2);
        assert_eq!(stats.func_count, 0);
    }

    #[test]
    fn stats_depth_for_nested() {
        let source = "def f() -> i32 { { { return 1; } } }";
        let mut lex_stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let tokens = lexer::tokenize(source, &mut lex_stats).unwrap();
        let mut stats = ParseStats {
            duration_us: 0, ast_node_count: 0, ast_max_depth: 0,
            node_counts_by_kind: HashMap::new(), func_count: 0, struct_count: 0,
        };
        parse(&tokens, &mut stats).unwrap();
        assert!(stats.ast_max_depth >= 5); // program → func-def → block → block → block → return
    }
}
