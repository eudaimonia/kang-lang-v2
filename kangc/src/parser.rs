// 语法分析 — 手写递归下降解析器 (LL(1))
// 将 Token 流按 EBNF 文法生成 AST，每个非终结符对应一个 parse_* 函数
// Token 流由 lexer 模块生成，包含 EOF 哨兵

use crate::ast::*;
use crate::error::ParseError;
use crate::lexer::{Token, TokenKind};
use crate::stats::ParseStats;
use std::collections::HashMap;
use std::ops::Range;
use std::time::Instant;

// 限制 token 数防超大输入耗尽内存；1M tokens ≈ 50MB 源码。可通过 KANG_MAX_TOKENS 环境变量覆盖
fn max_token_count() -> usize {
    use std::sync::LazyLock;
    static LIMIT: LazyLock<usize> = LazyLock::new(|| read_env_limit("KANG_MAX_TOKENS", 1_000_000));
    *LIMIT
}

// 限制嵌套深度防栈溢出。可通过 KANG_MAX_DEPTH 环境变量覆盖
fn max_parse_depth() -> usize {
    use std::sync::LazyLock;
    static LIMIT: LazyLock<usize> = LazyLock::new(|| read_env_limit("KANG_MAX_DEPTH", 256));
    *LIMIT
}

/// 从环境变量读取解析限制值，或使用默认值
fn read_env_limit(env_name: &str, default: usize) -> usize {
    if let Ok(val) = std::env::var(env_name) {
        if let Ok(n) = val.parse::<usize>() {
            if n > 0 { return n; }
        }
    }
    default
}

// ── Parser 结构 ─────────────────────────────────────────────────────────────

/// 空 token 流回退哨兵 — Parser 公开 API 接受任意切片，lexer 总是产出 EOF，但防御性编程
static EOF_SENTINEL: Token = Token {
    kind: TokenKind::Eof,
    line: 0,
    col: 0,
    span: 0..0,
};

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        debug_assert!(!tokens.is_empty(), "Token 流不应为空，至少包含 EOF 哨兵");
        Parser { tokens, pos: 0, depth: 0 }
    }

    /// 进入递归层级，超限则报错
    fn enter_depth(&mut self, ctx: &str) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > max_parse_depth() {
            return Err(self.error(format!(
                "{} 嵌套深度超过限制 {}",
                ctx, max_parse_depth()
            )));
        }
        Ok(())
    }

    fn leave_depth(&mut self) {
        self.depth -= 1;
    }

    // ── 基本操作 ─────────────────────────────────────────────────────────

    fn peek(&self) -> &Token {
        if self.pos < self.tokens.len() {
            &self.tokens[self.pos]
        } else {
            // 空 token 流或越界：返回静态 EOF 哨兵避免 panic
            &EOF_SENTINEL
        }
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn advance(&mut self) -> &Token {
        if self.tokens.is_empty() {
            return &EOF_SENTINEL;
        }
        let t = &self.tokens[self.pos];
        // 防止越过 EOF 导致 peek panic
        self.pos = (self.pos + 1).min(self.tokens.len() - 1);
        t
    }

    /// 期望特定 TokenKind，匹配则前进，否则报错
    /// 使用全等比较而非 discriminant，避免 `Ident("foo")` 误匹配 `Ident("bar")`
    fn expect(&mut self, expected: &TokenKind) -> Result<(), ParseError> {
        if self.peek_kind() == expected {
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
    /// TokenKind 的所有关键字变体都是 unit variant，故全等比较与 discriminant 等价
    fn match_kw(&mut self, kw: &TokenKind) -> bool {
        if self.peek_kind() == kw {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn current_span(&self) -> Range<usize> {
        self.peek().span.clone()
    }

    fn error(&self, msg: String) -> ParseError {
        let t = self.peek();
        ParseError {
            msg,
            line: t.line,
            col: t.col,
            span: t.span.clone(),
            is_incomplete: matches!(t.kind, TokenKind::Eof),
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
                let mut n = name.clone();
                self.advance();
                // 支持 module.Type 形式的导入类型引用
                if self.match_kw(&TokenKind::Dot) {
                    let suffix = self.expect_ident()?;
                    n = format!("{}.{}", n, suffix);
                }
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
            left = Expr::Binary { left: Box::new(left), op: BinOp::Or, right: Box::new(right), span: self.current_span() };
        }
        Ok(left)
    }

    // AndExpr = EqExpr { "&&" EqExpr }
    fn parse_and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_eq_expr()?;
        while self.match_kw(&TokenKind::AndAnd) {
            let right = self.parse_eq_expr()?;
            left = Expr::Binary { left: Box::new(left), op: BinOp::And, right: Box::new(right), span: self.current_span() };
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
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span: self.current_span() };
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
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span: self.current_span() };
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
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span: self.current_span() };
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
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), span: self.current_span() };
        }
        Ok(left)
    }

    // UnaryExpr = ("-" | "!") UnaryExpr | PostfixExpr
    fn parse_unary_expr(&mut self) -> Result<Expr, ParseError> {
        if self.match_kw(&TokenKind::Minus) {
            let expr = self.parse_unary_expr()?;
            Ok(Expr::Unary { op: UnaryOp::Neg, expr: Box::new(expr), span: self.current_span() })
        } else if self.match_kw(&TokenKind::Bang) {
            let expr = self.parse_unary_expr()?;
            Ok(Expr::Unary { op: UnaryOp::Not, expr: Box::new(expr), span: self.current_span() })
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
                    expr = Expr::Call { func: Box::new(expr), args, span: self.current_span() };
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket)?;
                    expr = Expr::Index { array: Box::new(expr), index: Box::new(index), span: self.current_span() };
                }
                TokenKind::Dot => {
                    self.advance();
                    let field = self.expect_ident()?;
                    expr = Expr::FieldAccess { obj: Box::new(expr), field, span: self.current_span() };
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
                Ok(Expr::IntLit(val, self.current_span()))
            }
            TokenKind::FloatLit(v) => {
                let val = v.clone();
                self.advance();
                Ok(Expr::FloatLit(val, self.current_span()))
            }
            TokenKind::StrLit(v) => {
                let val = v.clone();
                self.advance();
                Ok(Expr::StrLit(val, self.current_span()))
            }
            TokenKind::True => { self.advance(); Ok(Expr::BoolLit(true, self.current_span())) }
            TokenKind::False => { self.advance(); Ok(Expr::BoolLit(false, self.current_span())) }
            TokenKind::Ident(name) => {
                let mut n = name.clone();
                self.advance();
                // 支持 module.Type 形式, 用于结构体构造: module.Type{...}
                if self.peek_kind() == &TokenKind::Dot {
                    let saved_pos = self.pos;
                    self.advance(); // consume .
                    if let TokenKind::Ident(suffix) = self.peek_kind() {
                        let suffix = suffix.clone();
                        self.advance();
                        if self.peek_kind() == &TokenKind::LBrace {
                            n = format!("{}.{}", n, suffix);
                            return self.parse_struct_lit_tail(&n);
                        }
                        // 不是 struct lit, 回退: 返回 Ident, 让 postfix 解析 .field
                        self.pos = saved_pos;
                    } else {
                        self.pos = saved_pos;
                    }
                }
                // 判断是否为结构体构造: Name { ... }
                if self.peek_kind() == &TokenKind::LBrace {
                    self.parse_struct_lit_tail(&n)
                } else {
                    Ok(Expr::Ident(n, self.current_span()))
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
                Ok(Expr::ArrayLit(elems, self.current_span()))
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
            Ok(Expr::Ident(name.to_string(), self.current_span()))
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
        Ok(Expr::StructLit { name: name.to_string(), fields, span: self.current_span() })
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
            Ok(Stmt::Block(stmts, self.current_span()))
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
        Ok(Stmt::VarDecl { bindings, init: Box::new(init), span: self.current_span() })
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
            return Ok(Stmt::Assign { lvalue, value: Box::new(value), span: self.current_span() });
        }

        // 否则是表达式语句，需要分号
        self.expect(&TokenKind::Semi)?;
        Ok(Stmt::Expr(Box::new(expr), self.current_span()))
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
        Ok(Stmt::Return { values, span: self.current_span() })
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
        Ok(Stmt::If { condition: Box::new(condition), then_branch: Box::new(then_branch), else_branch, span: self.current_span() })
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
            span: self.current_span(),
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
        // parse_block 保证返回 Stmt::Block
        let body = match self.parse_block()? {
            Stmt::Block(stmts, ..) => stmts,
            _ => return Err(self.error("内部错误: 期望 Block 语句".into())),
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

    // ImportStmt = "import" IDENT "{" ImportItems "}" "from" STR_LIT ";"
    fn parse_import_stmt(&mut self) -> Result<ImportStmt, ParseError> {
        self.expect(&TokenKind::Import)?;
        let alias = self.expect_ident()?;
        self.expect(&TokenKind::LBrace)?;
        let items = self.parse_import_items()?;
        self.expect(&TokenKind::RBrace)?;
        self.expect(&TokenKind::From)?;
        let path = match self.peek_kind() {
            TokenKind::StrLit(s) => {
                let p = s.clone();
                self.advance();
                p
            }
            _ => return Err(self.error(format!("期望字符串路径，但得到 {:?}", self.peek_kind()))),
        };
        self.expect(&TokenKind::Semi)?;
        Ok(ImportStmt { alias, items, path })
    }

    // ImportItems = ImportItem { "," ImportItem }
    fn parse_import_items(&mut self) -> Result<Vec<String>, ParseError> {
        if self.peek_kind() == &TokenKind::RBrace {
            return Ok(vec![]);
        }
        let mut items = vec![self.expect_ident()?];
        while self.match_kw(&TokenKind::Comma) {
            items.push(self.expect_ident()?);
        }
        Ok(items)
    }

    // Program = { TopLevel }
    fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut items = Vec::new();
        while self.peek_kind() != &TokenKind::Eof {
            let item = match self.peek_kind() {
                TokenKind::Struct => TopLevel::Struct(self.parse_struct_def()?),
                TokenKind::Def => TopLevel::Func(self.parse_func_def()?),
                TokenKind::Import => TopLevel::Import(self.parse_import_stmt()?),
                _ => {
                    return Err(self.error(format!(
                        "期望 struct、def 或 import，但得到 {:?}",
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

/// 将解析好的表达式转换为左值（赋值目标）。
///
/// 语法层在此检查 LValue 的形式合法性（AS3/AS4）——只有 Ident、Index、FieldAccess
/// 三种 Expression 可以转为左值。函数调用、字面量等作为赋值目标时在解析阶段直接拒绝。
/// 无法在语法层区分的合法性问题（如字符串索引不可赋值 AS1）留给语义层。
fn expr_to_lvalue(expr: Expr, mark: usize, tokens: &[Token]) -> Result<LValue, ParseError> {
    match expr {
        Expr::Ident(name, ..) => Ok(LValue::Ident(name, tokens[mark].span.clone())),
        Expr::Index { array, index , ..} => Ok(LValue::Index { array, index, span: tokens[mark].span.clone() }),
        Expr::FieldAccess { obj, field , ..} => Ok(LValue::FieldAccess { obj, field, span: tokens[mark].span.clone() }),
        _ => {
            let t = &tokens[mark];
            Err(ParseError {
                msg: format!("赋值左侧必须是变量、索引或字段访问，但得到表达式"),
                line: t.line,
                col: t.col,
                span: t.span.clone(),
                is_incomplete: false,
            })
        }
    }
}

// ── 统计收集 ────────────────────────────────────────────────────────────────

/// 计算 AST 的最大深度（嵌套层次数），用于统计和嵌套限制验证。
///
/// 递归遍历表达式和语句的子树，取各层级的最大值。struct 定义深度固定为 2，
/// import 深度为 1，函数深度为 2 + 函数体语句的最大深度。
fn ast_depth(program: &Program) -> usize {
    fn expr_depth(e: &Expr) -> usize {
        match e {
            Expr::Binary { left, right, .. } => 1 + expr_depth(left).max(expr_depth(right)),
            Expr::Unary { expr, .. } => 1 + expr_depth(expr),
            Expr::Call { func, args , ..} => {
                let arg_max = args.iter().map(|a| expr_depth(a)).max().unwrap_or(0);
                1 + expr_depth(func).max(arg_max)
            }
            Expr::Index { array, index , ..} => 1 + expr_depth(array).max(expr_depth(index)),
            Expr::FieldAccess { obj, .. } => 1 + expr_depth(obj),
            Expr::StructLit { fields, .. } => {
                1 + fields.iter().map(|(_, v)| expr_depth(v)).max().unwrap_or(0)
            }
            Expr::ArrayLit(elems, ..) => {
                1 + elems.iter().map(|e| expr_depth(e)).max().unwrap_or(0)
            }
            Expr::IntLit(..) | Expr::FloatLit(..) | Expr::StrLit(..)
            | Expr::BoolLit(..) | Expr::Ident(..) => 1,
        }
    }

    fn stmt_depth(s: &Stmt) -> usize {
        match s {
            Stmt::VarDecl { bindings: _, init , ..} => 1 + expr_depth(init),
            Stmt::Assign { lvalue: _, value , ..} => 1 + expr_depth(value),
            Stmt::Return { values , ..} => {
                1 + values.iter().map(|v| expr_depth(v)).max().unwrap_or(0)
            }
            Stmt::If { condition, then_branch, else_branch , ..} => {
                let else_d = else_branch.as_ref().map(|s| stmt_depth(s)).unwrap_or(0);
                1 + expr_depth(condition).max(stmt_depth(then_branch)).max(else_d)
            }
            Stmt::For { start, end, step_expr, body, .. } => {
                1 + expr_depth(start).max(expr_depth(end))
                    .max(expr_depth(step_expr))
                    .max(stmt_depth(body))
            }
            Stmt::Expr(e, ..) => 1 + expr_depth(e),
            Stmt::Block(stmts, ..) => {
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
            TopLevel::Import(_) => 1, // import 单节点
        }
    }).max().unwrap_or(0)
}

/// 按类型统计 AST 节点数量，用于编译器统计输出。
///
/// 遍历所有顶层项、函数体中的语句和表达式，统计每种节点类型出现的次数
/// （如 "func-def": 2, "var-decl": 5, "binary": 3）。
fn count_nodes(program: &Program) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for item in &program.items {
        match item {
            TopLevel::Struct(_) => *counts.entry("struct-def".into()).or_insert(0) += 1,
            TopLevel::Func(f) => {
                *counts.entry("func-def".into()).or_insert(0) += 1;
                count_stmt_nodes(&f.body, &mut counts);
            }
            TopLevel::Import(_) => *counts.entry("import".into()).or_insert(0) += 1,
        }
    }
    counts
}

fn count_stmt_nodes(stmts: &[Stmt], counts: &mut HashMap<String, usize>) {
    for s in stmts {
        match s {
            Stmt::VarDecl { bindings: _, init , ..} => {
                *counts.entry("var-decl".into()).or_insert(0) += 1;
                count_expr_nodes(init, counts);
            }
            Stmt::Assign { lvalue: _, value , ..} => {
                *counts.entry("assign".into()).or_insert(0) += 1;
                count_expr_nodes(value, counts);
            }
            Stmt::Return { values , ..} => {
                *counts.entry("return".into()).or_insert(0) += 1;
                for v in values { count_expr_nodes(v, counts); }
            }
            Stmt::If { condition, then_branch, else_branch , ..} => {
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
            Stmt::Expr(e, ..) => {
                *counts.entry("expr-stmt".into()).or_insert(0) += 1;
                count_expr_nodes(e, counts);
            }
            Stmt::Block(inner, ..) => {
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
        Expr::Call { func, args , ..} => {
            *counts.entry("call".into()).or_insert(0) += 1;
            count_expr_nodes(func, counts);
            for a in args { count_expr_nodes(a, counts); }
        }
        Expr::Index { array, index , ..} => {
            *counts.entry("index".into()).or_insert(0) += 1;
            count_expr_nodes(array, counts);
            count_expr_nodes(index, counts);
        }
        Expr::FieldAccess { obj, .. } => {
            *counts.entry("field-access".into()).or_insert(0) += 1;
            count_expr_nodes(obj, counts);
        }
        Expr::IntLit(_, ..) => { *counts.entry("int-lit".into()).or_insert(0) += 1; }
        Expr::FloatLit(_, ..) => { *counts.entry("float-lit".into()).or_insert(0) += 1; }
        Expr::StrLit(_, ..) => { *counts.entry("str-lit".into()).or_insert(0) += 1; }
        Expr::BoolLit(_, ..) => { *counts.entry("bool-lit".into()).or_insert(0) += 1; }
        Expr::Ident(_, ..) => { *counts.entry("ident".into()).or_insert(0) += 1; }
        Expr::ArrayLit(elems, ..) => {
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

/// 将 token 流解析为完整的 Program AST，同时收集解析统计数据。
///
/// 先检查 token 数量是否超过限制（防御超大输入），再创建 Parser 实例进行递归下降解析。
/// 解析完成后计算 AST 深度和各类型节点数写入统计信息。
pub fn parse(tokens: &[Token], stats: &mut ParseStats) -> Result<Program, ParseError> {
    if tokens.len() > max_token_count() {
        return Err(ParseError {
            msg: format!("token 数量 {} 超过限制 {}", tokens.len(), max_token_count()),
            line: 0,
            col: 0,
            span: 0..0,
            is_incomplete: false,
        });
    }
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

// ── REPL 行解析 ───────────────────────────────────────────────────────────────

/// REPL 单行解析结果，区分不同输入类型的处理路径。
///
/// - FuncDef/StructDef/Import: 注册到 REPL 的累积定义
/// - Stmt: 编译并执行（无输出）
/// - Expr: 求值并用 puts(str()) 打印结果
#[derive(Debug, Clone)]
pub enum LineResult {
    FuncDef(FuncDef),
    StructDef(StructDef),
    Import(ImportStmt),
    Stmt(Stmt),
    /// 裸表达式 — REPL 求值后打印结果
    Expr(Expr),
}

/// 解析单个语句，确保消耗所有 token。
///
/// 若 token 流中还有未消耗内容（如额外分号或表达式），返回错误。
/// 用于 REPL 和测试中的增量解析。
pub fn parse_stmt(tokens: &[Token]) -> Result<Stmt, ParseError> {
    let mut parser = Parser::new(tokens);
    let stmt = parser.parse_stmt()?;
    if parser.peek_kind() != &TokenKind::Eof {
        return Err(parser.error("语句后有额外输入".to_string()));
    }
    Ok(stmt)
}

/// 解析单个表达式，确保消耗所有 token。
///
/// 用于 REPL 中的表达式解析和测试。
pub fn parse_expr(tokens: &[Token]) -> Result<Expr, ParseError> {
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expr()?;
    if parser.peek_kind() != &TokenKind::Eof {
        return Err(parser.error("表达式后有额外输入".to_string()));
    }
    Ok(expr)
}

/// REPL 行解析：智能判断输入类型
/// - def/struct → 顶层声明
/// - var/return/if/for/{ → 语句
/// - 表达式后跟 = → 赋值语句
/// - 表达式后跟 ; → 表达式语句
/// - 表达式后跟 EOF → 裸表达式（REPL 求值并打印）
pub fn parse_line(tokens: &[Token]) -> Result<LineResult, ParseError> {
    if tokens.len() > max_token_count() {
        return Err(ParseError {
            msg: format!("token 数量 {} 超过限制 {}", tokens.len(), max_token_count()),
            line: 0,
            col: 0,
            span: 0..0,
            is_incomplete: false,
        });
    }

    // 空输入 → 继续等待
    if tokens.len() <= 1 {
        return Err(ParseError {
            msg: "空输入".to_string(),
            line: 0,
            col: 0,
            span: 0..0,
            is_incomplete: true,
        });
    }

    let mut parser = Parser::new(tokens);

    // def / struct / import 声明
    match parser.peek_kind() {
        TokenKind::Def => {
            return Ok(LineResult::FuncDef(parser.parse_func_def()?));
        }
        TokenKind::Struct => {
            return Ok(LineResult::StructDef(parser.parse_struct_def()?));
        }
        TokenKind::Import => {
            return Ok(LineResult::Import(parser.parse_import_stmt()?));
        }
        TokenKind::Var | TokenKind::Return | TokenKind::If |
        TokenKind::For | TokenKind::LBrace => {
            return Ok(LineResult::Stmt(parser.parse_stmt()?));
        }
        _ => {}
    }

    // 表达式 → 根据后续 token 判断语义
    let expr_start = parser.pos;
    let expr = parser.parse_expr()?;

    match parser.peek_kind() {
        TokenKind::Assign => {
            parser.advance();
            let value = parser.parse_expr()?;
            parser.expect(&TokenKind::Semi)?;
            let lvalue = expr_to_lvalue(expr, expr_start, parser.tokens)?;
            Ok(LineResult::Stmt(Stmt::Assign {
                lvalue,
                value: Box::new(value),
                span: parser.current_span(),
            }))
        }
        TokenKind::Semi => {
            parser.advance();
            Ok(LineResult::Stmt(Stmt::Expr(Box::new(expr), parser.current_span())))
        }
        TokenKind::Eof => {
            Ok(LineResult::Expr(expr))
        }
        _ => Err(parser.error(format!(
            "期望 ; = 或表达式结束，但得到 {:?}",
            parser.peek_kind()
        ))),
    }
}

// ── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const S: std::ops::Range<usize> = 0..1;
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
                Stmt::Return { values , ..} => Ok(values[0].clone()),
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

    #[test] fn expr_int_lit()    { assert_eq!(parse_expr("42").unwrap(), Expr::IntLit("42".into(), S)); }
    #[test] fn expr_float_lit()  { assert_eq!(parse_expr("3.14").unwrap(), Expr::FloatLit("3.14".into(), S)); }
    #[test] fn expr_str_lit()    { assert_eq!(parse_expr("\"hi\"").unwrap(), Expr::StrLit("hi".into(), S)); }
    #[test] fn expr_true()       { assert_eq!(parse_expr("true").unwrap(), Expr::BoolLit(true, S)); }
    #[test] fn expr_false()      { assert_eq!(parse_expr("false").unwrap(), Expr::BoolLit(false, S)); }
    #[test] fn expr_ident()      { assert_eq!(parse_expr("x").unwrap(), Expr::Ident("x".into(), S)); }

    // ── 表达式: 二元运算符(优先级) ────────────────────────────────────────

    #[test]
    fn expr_binary_add() {
        assert_eq!(
            parse_expr("a + b").unwrap(),
            Expr::Binary {
                left: Box::new(Expr::Ident("a".into(), S)),
                op: BinOp::Add,
                right: Box::new(Expr::Ident("b".into(), S)),
             span: S,}
        );
    }

    #[test]
    fn expr_binary_sub_mul_precedence() {
        // a + b * c  ≡  a + (b * c)
        let e = parse_expr("a + b * c").unwrap();
        assert_eq!(
            e,
            Expr::Binary {
                left: Box::new(Expr::Ident("a".into(), S)),
                op: BinOp::Add,
                right: Box::new(Expr::Binary {
                    left: Box::new(Expr::Ident("b".into(), S)),
                    op: BinOp::Mul,
                    right: Box::new(Expr::Ident("c".into(), S)),
                 span: S,}),
             span: S,}
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
                    left: Box::new(Expr::Ident("a".into(), S)),
                    op: BinOp::Mul,
                    right: Box::new(Expr::Ident("b".into(), S)),
                 span: S,}),
                op: BinOp::Add,
                right: Box::new(Expr::Ident("c".into(), S)),
             span: S,}
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
                    left: Box::new(Expr::Ident("a".into(), S)),
                    op: BinOp::Add,
                    right: Box::new(Expr::Ident("b".into(), S)),
                 span: S,}),
                op: BinOp::Mul,
                right: Box::new(Expr::Ident("c".into(), S)),
             span: S,}
        );
    }

    #[test]
    fn expr_cmp_before_eq() {
        // a < b == true  ≡  (a < b) == true
        let e = parse_expr("a < b == true").unwrap();
        assert_eq!(e, Expr::Binary {
            left: Box::new(Expr::Binary {
                left: Box::new(Expr::Ident("a".into(), S)),
                op: BinOp::Lt,
                right: Box::new(Expr::Ident("b".into(), S)),
             span: S,}),
            op: BinOp::Eq,
            right: Box::new(Expr::BoolLit(true, S)),
         span: S,});
    }

    #[test]
    fn expr_or_below_and() {
        // x || y && false  ≡  x || (y && false)
        let e = parse_expr("x || y && false").unwrap();
        assert_eq!(e, Expr::Binary {
            left: Box::new(Expr::Ident("x".into(), S)),
            op: BinOp::Or,
            right: Box::new(Expr::Binary {
                left: Box::new(Expr::Ident("y".into(), S)),
                op: BinOp::And,
                right: Box::new(Expr::BoolLit(false, S)),
             span: S,}),
         span: S,});
    }

    #[test]
    fn expr_comparison_chain() {
        // a < b < c  ≡  (a < b) < c  (左结合)
        let e = parse_expr("a < b < c").unwrap();
        match e {
            Expr::Binary { left, op: BinOp::Lt, right , ..} => {
                assert_eq!(*left, Expr::Binary {
                    left: Box::new(Expr::Ident("a".into(), S)),
                    op: BinOp::Lt,
                    right: Box::new(Expr::Ident("b".into(), S)),
                 span: S,});
                assert_eq!(*right, Expr::Ident("c".into(), S));
            }
            _ => panic!("expected binary"),
        }
    }

    // ── 表达式: 一元运算符 ────────────────────────────────────────────────

    #[test]
    fn expr_neg() {
        assert_eq!(
            parse_expr("-a").unwrap(),
            Expr::Unary { op: UnaryOp::Neg, expr: Box::new(Expr::Ident("a".into(), S)) ,
                    span: S}
        );
    }

    #[test]
    fn expr_not() {
        assert_eq!(
            parse_expr("!x").unwrap(),
            Expr::Unary { op: UnaryOp::Not, expr: Box::new(Expr::Ident("x".into(), S)) ,
                    span: S}
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
                    expr: Box::new(Expr::IntLit("42".into(), S)),
                 span: S,}),
             span: S,}
        );
    }

    #[test]
    fn expr_neg_mul_precedence() {
        // -a * b  ≡  (-a) * b
        let e = parse_expr("-a * b").unwrap();
        assert_eq!(e, Expr::Binary {
            left: Box::new(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(Expr::Ident("a".into(), S)),
             span: S,}),
            op: BinOp::Mul,
            right: Box::new(Expr::Ident("b".into(), S)),
         span: S,});
    }

    // ── 表达式: 后缀操作 ─────────────────────────────────────────────────

    #[test]
    fn expr_call_no_args() {
        assert_eq!(
            parse_expr("f()").unwrap(),
            Expr::Call { func: Box::new(Expr::Ident("f".into(), S)), args: vec![] ,
                    span: S}
        );
    }

    #[test]
    fn expr_call_one_arg() {
        assert_eq!(
            parse_expr("f(1)").unwrap(),
            Expr::Call {
                func: Box::new(Expr::Ident("f".into(), S)),
                args: vec![Expr::IntLit("1".into(), S)],
                span: S,
            }
        );
    }

    #[test]
    fn expr_call_multi_args() {
        let e = parse_expr("add(1, 2, 3)").unwrap();
        match e {
            Expr::Call { func, args , ..} => {
                assert_eq!(*func, Expr::Ident("add".into(), S));
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
                array: Box::new(Expr::Ident("arr".into(), S)),
                index: Box::new(Expr::IntLit("0".into(), S)),
             span: S,}
        );
    }

    #[test]
    fn expr_field_access() {
        assert_eq!(
            parse_expr("obj.field").unwrap(),
            Expr::FieldAccess {
                obj: Box::new(Expr::Ident("obj".into(), S)),
                field: "field".into(),
             span: S,}
        );
    }

    #[test]
    fn expr_chained_field_access() {
        // a.b.c
        let e = parse_expr("a.b.c").unwrap();
        assert_eq!(e, Expr::FieldAccess {
            obj: Box::new(Expr::FieldAccess {
                obj: Box::new(Expr::Ident("a".into(), S)),
                field: "b".into(),
             span: S,}),
            field: "c".into(),
         span: S,});
    }

    #[test]
    fn expr_call_chained() {
        // f().g(1).h
        let e = parse_expr("f().g(1).h").unwrap();
        assert_eq!(e, Expr::FieldAccess {
            obj: Box::new(Expr::Call {
                func: Box::new(Expr::FieldAccess {
                    obj: Box::new(Expr::Call {
                        func: Box::new(Expr::Ident("f".into(), S)),
                        args: vec![],
                     span: S,}),
                    field: "g".into(),
                 span: S,}),
                args: vec![Expr::IntLit("1".into(), S)],
                span: S,
            }),
            field: "h".into(),
         span: S,});
    }

    // ── 表达式: 数组/结构体字面量 ─────────────────────────────────────────

    #[test]
    fn expr_array_lit_empty() {
        assert_eq!(parse_expr("[]").unwrap(), Expr::ArrayLit(vec![], S));
    }

    #[test]
    fn expr_array_lit_elements() {
        let e = parse_expr("[1, 2, 3]").unwrap();
        match e {
            Expr::ArrayLit(elems, _) => assert_eq!(elems.len(), 3),
            _ => panic!("expected array lit"),
        }
    }

    #[test]
    fn expr_struct_lit_empty() {
        let e = parse_expr("Point{}").unwrap();
        match e {
            Expr::StructLit { name, fields , ..} => {
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
            Expr::StructLit { name, fields , ..} => {
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
            Stmt::VarDecl { bindings, init , ..} => {
                assert_eq!(bindings.len(), 1);
                assert_eq!(bindings[0], VarBinding::Named { name: "x".into(), ty: Type::Base(BaseType::I32) });
                assert_eq!(*init, Expr::IntLit("42".into(), S));
            }
            _ => panic!("expected var decl"),
        }
    }

    #[test]
    fn stmt_var_decl_double() {
        let s = parse_stmt("var a:i32, b:i32 = f();").unwrap();
        match s {
            Stmt::VarDecl { bindings, init: _ , ..} => {
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
            Stmt::Assign { lvalue, value , ..} => {
                assert_eq!(lvalue, LValue::Ident("x".into(), S));
                assert_eq!(*value, Expr::IntLit("42".into(), S));
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
                    array: Box::new(Expr::Ident("arr".into(), S)),
                    index: Box::new(Expr::Ident("i".into(), S)),
                 span: S,});
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
                    obj: Box::new(Expr::Ident("obj".into(), S)),
                    field: "field".into(),
                 span: S,});
            }
            _ => panic!("expected assign"),
        }
    }

    #[test]
    fn stmt_return_void() {
        let s = parse_stmt("return;").unwrap();
        assert_eq!(s, Stmt::Return { values: vec![] ,
                    span: S});
    }

    #[test]
    fn stmt_return_single() {
        let s = parse_stmt("return 0;").unwrap();
        assert_eq!(s, Stmt::Return { values: vec![Expr::IntLit("0".into(), S)], span: S });
    }

    #[test]
    fn stmt_return_pair() {
        let s = parse_stmt("return 1, true;").unwrap();
        assert_eq!(s, Stmt::Return {
            values: vec![Expr::IntLit("1".into(), S), Expr::BoolLit(true, S)],
            span: S,
        });
    }

    #[test]
    fn stmt_if_simple() {
        let s = parse_stmt("if x then return;").unwrap();
        match s {
            Stmt::If { condition, then_branch, else_branch , ..} => {
                assert_eq!(*condition, Expr::Ident("x".into(), S));
                assert_eq!(*then_branch, Stmt::Return { values: vec![] ,
                    span: S});
                assert!(else_branch.is_none());
            }
            _ => panic!("expected if"),
        }
    }

    #[test]
    fn stmt_if_else() {
        let s = parse_stmt("if x then return 0; else return 1;").unwrap();
        match s {
            Stmt::If { condition, then_branch: _, else_branch , ..} => {
                assert_eq!(*condition, Expr::Ident("x".into(), S));
                assert!(else_branch.is_some());
            }
            _ => panic!("expected if"),
        }
    }

    #[test]
    fn stmt_block_empty() {
        let s = parse_stmt("{}").unwrap();
        assert_eq!(s, Stmt::Block(vec![], S));
    }

    #[test]
    fn stmt_block_multiple() {
        let s = parse_stmt("{ return; return 1; }").unwrap();
        match s {
            Stmt::Block(stmts, _) => assert_eq!(stmts.len(), 2),
            _ => panic!("expected block"),
        }
    }

    #[test]
    fn stmt_expr_stmt() {
        let s = parse_stmt("puts(\"hi\");").unwrap();
        match s {
            Stmt::Expr(_, _) => {} // 仅验证解析不报错
            _ => panic!("expected expr stmt"),
        }
    }

    #[test]
    fn stmt_for_loop() {
        let source = "for var i:i32 = 0, i < 10, i = i + 1 in { }";
        let s = parse_stmt(source).unwrap();
        match s {
            Stmt::For { var_name, var_type, start, end, step_lvalue, step_expr: _, body , ..} => {
                assert_eq!(var_name, "i");
                assert_eq!(var_type, Type::Base(BaseType::I32));
                assert_eq!(*start, Expr::IntLit("0".into(), S));
                assert_eq!(*end, Expr::Binary {
                    left: Box::new(Expr::Ident("i".into(), S)),
                    op: BinOp::Lt,
                    right: Box::new(Expr::IntLit("10".into(), S)),
                 span: S,});
                assert_eq!(step_lvalue, LValue::Ident("i".into(), S));
                assert_eq!(*body, Stmt::Block(vec![], S));
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

    #[test]
    fn top_import_single() {
        let p = parse_source("import m { add } from \"./math.kang\";").unwrap();
        assert_eq!(p.items.len(), 1);
        match &p.items[0] {
            TopLevel::Import(i) => {
                assert_eq!(i.alias, "m");
                assert_eq!(i.items, vec!["add"]);
                assert_eq!(i.path, "./math.kang");
            }
            _ => panic!("expected import"),
        }
    }

    #[test]
    fn top_import_multi_items() {
        let p = parse_source("import m { add, sub, mul } from \"./math.kang\";").unwrap();
        match &p.items[0] {
            TopLevel::Import(i) => {
                assert_eq!(i.alias, "m");
                assert_eq!(i.items.len(), 3);
            }
            _ => panic!("expected import"),
        }
    }

    #[test]
    fn top_import_mixed_with_def() {
        let p = parse_source("import m { f } from \"./lib.kang\"; def main() -> i32 { return 0; }").unwrap();
        assert_eq!(p.items.len(), 2);
        assert!(matches!(&p.items[0], TopLevel::Import(_)));
        assert!(matches!(&p.items[1], TopLevel::Func(_)));
    }

    #[test]
    fn error_import_in_body() {
        // import 不可出现在函数体内
        let result = parse_source("def f() -> void { import m { x } from \"./lib.kang\"; return; }");
        assert!(result.is_err());
    }

    #[test]
    fn error_bad_assignment_target() {
        // 42 = x; — 语法层会将 42 解析为表达式, 转左值时失败
        let result = parse_stmt("42 = x;");
        assert!(result.is_err());
        assert!(result.unwrap_err().msg.contains("赋值左侧"));
    }

    // ── 嵌套结构 ─────────────────────────────────────────────────────────

    #[test]
    fn nested_blocks() {
        let p = parse_source("def f() -> i32 { { { return 1; } } }").unwrap();
        match &p.items[0] {
            TopLevel::Func(f) => {
                match &f.body[0] {
                    Stmt::Block(outer, _) => {
                        match &outer[0] {
                            Stmt::Block(inner, _) => {
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
        assert_eq!(e, Expr::Unary { op: UnaryOp::Neg, expr: Box::new(Expr::IntLit("1".into(), S)) ,
                    span: S});
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
        assert!(result.unwrap_err().msg.contains("struct"));
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

    // ── parse_line 测试 ──────────────────────────────────────────────────

    /// 辅助: 源码 → parse_line 结果
    fn parse_line_source(source: &str) -> Result<LineResult, ParseError> {
        let mut lex_stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let tokens = lexer::tokenize(source, &mut lex_stats).unwrap();
        parse_line(&tokens)
    }

    #[test]
    fn repl_expr_int_lit() {
        let r = parse_line_source("42").unwrap();
        assert!(matches!(r, LineResult::Expr(_)));
    }

    #[test]
    fn repl_expr_binary_add() {
        let r = parse_line_source("1 + 2").unwrap();
        assert!(matches!(r, LineResult::Expr(_)));
    }

    #[test]
    fn repl_expr_call() {
        let r = parse_line_source("puts(\"hello\")").unwrap();
        assert!(matches!(r, LineResult::Expr(_)));
    }

    #[test]
    fn repl_stmt_let() {
        let r = parse_line_source("var x:i32 = 5;").unwrap();
        assert!(matches!(r, LineResult::Stmt(_)));
    }

    #[test]
    fn repl_stmt_assign() {
        let r = parse_line_source("x = 10;").unwrap();
        assert!(matches!(r, LineResult::Stmt(_)));
    }

    #[test]
    fn repl_expr_with_semi_is_stmt() {
        let r = parse_line_source("1 + 2;").unwrap();
        assert!(matches!(r, LineResult::Stmt(_)));
    }

    #[test]
    fn repl_incomplete_if() {
        let r = parse_line_source("if true then");
        assert!(r.is_err());
        assert!(r.unwrap_err().is_incomplete);
    }

    #[test]
    fn repl_incomplete_var() {
        let r = parse_line_source("var x");
        assert!(r.is_err());
        assert!(r.unwrap_err().is_incomplete);
    }
}
