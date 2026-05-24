// Kang v2 — Recursive Descent Parser Prototype
// EBNF grammar validator. Reads .kang files, reports parse success or first error.

use std::env;
use std::fmt;
use std::fs;
use std::process;

// ── Token ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    // Keywords
    Def, Var, Return, If, Else, Then, For, In, Struct,
    // Types
    TI32, TF64, TStr, TBool, TVoid,
    // Literals
    IntLit(String), FloatLit(String), StrLit(String),
    True, False, Ident(String),
    // Operators & delimiters
    Plus, Minus, Star, Slash, Lt, Le, Gt, Ge, EqEq, Neq,
    AndAnd, OrOr, Bang, Assign,
    LParen, RParen, LBracket, RBracket, LBrace, RBrace,
    Colon, Semi, Comma, Arrow, Dot,
    // Sentinel
    Eof,
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    line: usize,
    col: usize,
}

// ── Lexer ──────────────────────────────────────────────────────────────────

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Lexer { chars: source.chars().collect(), pos: 0, line: 1, col: 1 }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        if let Some(ch) = c {
            self.pos += 1;
            if ch == '\n' { self.line += 1; self.col = 1; }
            else { self.col += 1; }
        }
        c
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(' ') | Some('\t') | Some('\r') | Some('\n') => { self.advance(); }
                Some('/') => {
                    self.advance();
                    if self.peek() == Some('/') {
                        while self.peek().map_or(false, |c| c != '\n') { self.advance(); }
                    } else if self.peek() == Some('*') {
                        self.advance();
                        loop {
                            match self.advance() {
                                None => break,
                                Some('*') if self.peek() == Some('/') => { self.advance(); break; }
                                _ => {}
                            }
                        }
                    } else {
                        // single '/' is not valid, but returned as operator later
                        self.pos -= 1; // rewind
                        self.col -= 1;
                        break;
                    }
                }
                _ => break,
            }
        }
    }

    fn read_while(&mut self, pred: fn(char) -> bool) -> String {
        let mut s = String::new();
        while self.peek().map_or(false, pred) {
            s.push(self.advance().unwrap());
        }
        s
    }

    fn read_string(&mut self) -> (String, bool) {
        // We already consumed the opening '"'
        let mut s = String::new();
        loop {
            match self.advance() {
                None => return (s, false), // unterminated
                Some('"') => return (s, true),
                Some('\\') => {
                    match self.advance() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('\\') => s.push('\\'),
                        Some('"') => s.push('"'),
                        Some('0') => s.push('\0'),
                        Some(c) => { s.push('\\'); s.push(c); }
                        None => return (s, false),
                    }
                }
                Some(c) => s.push(c),
            }
        }
    }

    fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace_and_comments();
            let line = self.line;
            let col = self.col;
            let c = match self.peek() {
                Some(c) => c,
                None => break,
            };

            let kind = match c {
                '(' => { self.advance(); TokenKind::LParen }
                ')' => { self.advance(); TokenKind::RParen }
                '[' => { self.advance(); TokenKind::LBracket }
                ']' => { self.advance(); TokenKind::RBracket }
                '{' => { self.advance(); TokenKind::LBrace }
                '}' => { self.advance(); TokenKind::RBrace }
                ':' => { self.advance(); TokenKind::Colon }
                ';' => { self.advance(); TokenKind::Semi }
                ',' => { self.advance(); TokenKind::Comma }
                '.' => { self.advance(); TokenKind::Dot }
                '+' => { self.advance(); TokenKind::Plus }
                '*' => { self.advance(); TokenKind::Star }
                '/' => { self.advance(); TokenKind::Slash }
                '!' => {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); TokenKind::Neq }
                    else { TokenKind::Bang }
                }
                '=' => {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); TokenKind::EqEq }
                    else { TokenKind::Assign }
                }
                '<' => {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); TokenKind::Le }
                    else { TokenKind::Lt }
                }
                '>' => {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); TokenKind::Ge }
                    else { TokenKind::Gt }
                }
                '&' => {
                    self.advance();
                    if self.peek() == Some('&') { self.advance(); TokenKind::AndAnd }
                    else { panic!("unexpected '&' at {}:{}", line, col) }
                }
                '|' => {
                    self.advance();
                    if self.peek() == Some('|') { self.advance(); TokenKind::OrOr }
                    else { panic!("unexpected '|' at {}:{}", line, col) }
                }
                '-' => {
                    self.advance();
                    if self.peek() == Some('>') { self.advance(); TokenKind::Arrow }
                    else { TokenKind::Minus }
                }
                '"' => {
                    self.advance(); // consume opening quote
                    let (s, ok) = self.read_string();
                    if !ok { panic!("unterminated string at {}:{}", line, col); }
                    TokenKind::StrLit(s)
                }
                c if c.is_ascii_digit() => {
                    let int_part = self.read_while(|c| c.is_ascii_digit());
                    if self.peek() == Some('.') {
                        self.advance();
                        let frac_part = self.read_while(|c| c.is_ascii_digit());
                        TokenKind::FloatLit(format!("{}.{}", int_part, frac_part))
                    } else {
                        TokenKind::IntLit(int_part)
                    }
                }
                c if c.is_ascii_alphabetic() || c == '_' => {
                    let ident = self.read_while(|c| c.is_ascii_alphanumeric() || c == '_');
                    match ident.as_str() {
                        "def" => TokenKind::Def,    "var" => TokenKind::Var,
                        "return" => TokenKind::Return, "if" => TokenKind::If,
                        "else" => TokenKind::Else,  "then" => TokenKind::Then,
                        "for" => TokenKind::For,    "in" => TokenKind::In,
                        "struct" => TokenKind::Struct,
                        "i32" => TokenKind::TI32,   "f64" => TokenKind::TF64,
                        "str" => TokenKind::TStr,   "bool" => TokenKind::TBool,
                        "void" => TokenKind::TVoid,
                        "true" => TokenKind::True,  "false" => TokenKind::False,
                        _ => TokenKind::Ident(ident),
                    }
                }
                _ => panic!("unexpected character '{}' at {}:{}", c, line, col),
            };
            tokens.push(Token { kind, line, col });
        }
        tokens.push(Token { kind: TokenKind::Eof, line: self.line, col: self.col });
        tokens
    }
}

// ── Parser ─────────────────────────────────────────────────────────────────

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

#[derive(Debug)]
struct ParseError {
    msg: String,
    line: usize,
    col: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Parse error at {}:{} — {}", self.line, self.col, self.msg)
    }
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self { Parser { tokens, pos: 0 } }

    fn peek(&self) -> &Token { &self.tokens[self.pos] }
    fn peek_kind(&self) -> &TokenKind { &self.tokens[self.pos].kind }

    fn advance(&mut self) -> &Token {
        let t = &self.tokens[self.pos];
        self.pos += 1;
        t
    }

    fn expect(&mut self, expected: TokenKind) -> Result<&Token, ParseError> {
        let t = &self.tokens[self.pos];
        // Compare kind only (not value for literals/idents)
        if std::mem::discriminant(&t.kind) == std::mem::discriminant(&expected) {
            self.pos += 1;
            Ok(t)
        } else {
            Err(ParseError {
                msg: format!("expected {:?}, got {:?}", expected, t.kind),
                line: t.line,
                col: t.col,
            })
        }
    }

    fn expect_kw(&mut self, kw: TokenKind) -> Result<(), ParseError> {
        let t = &self.tokens[self.pos];
        if t.kind == kw { self.pos += 1; Ok(()) }
        else {
            Err(ParseError {
                msg: format!("expected {:?}, got {:?}", kw, t.kind),
                line: t.line,
                col: t.col,
            })
        }
    }

    fn match_kw(&mut self, kw: TokenKind) -> bool {
        if self.peek_kind() == &kw { self.pos += 1; true }
        else { false }
    }

    // ── Type ────────────────────────────────────────────────────────────
    // BaseType   = "i32" | "f64" | "str" | "bool" | "void" | IDENT
    // Type       = BaseType | "[" BaseType "]"
    // ReturnType = Type | "(" Type "," Type ")"

    fn parse_type(&mut self) -> Result<(), ParseError> {
        if self.match_kw(TokenKind::LBracket) {
            self.parse_basetype()?;
            self.expect_kw(TokenKind::RBracket)?;
        } else {
            self.parse_basetype()?;
        }
        Ok(())
    }

    fn parse_return_type(&mut self) -> Result<(), ParseError> {
        if self.match_kw(TokenKind::LParen) {
            // (T1, T2) — exactly two types
            self.parse_type()?;
            self.expect_kw(TokenKind::Comma)?;
            self.parse_type()?;
            self.expect_kw(TokenKind::RParen)?;
        } else {
            self.parse_type()?;
        }
        Ok(())
    }

    fn parse_basetype(&mut self) -> Result<(), ParseError> {
        match self.peek_kind() {
            TokenKind::TI32 | TokenKind::TF64 | TokenKind::TStr
            | TokenKind::TBool | TokenKind::TVoid => { self.advance(); Ok(()) }
            TokenKind::Ident(_) => { self.advance(); Ok(()) }
            _ => Err(ParseError {
                msg: format!("expected type, got {:?}", self.peek_kind()),
                line: self.peek().line, col: self.peek().col,
            }),
        }
    }

    // ── Expressions (EBNF §6) ───────────────────────────────────────────

    fn parse_expr(&mut self) -> Result<(), ParseError> {
        self.parse_or_expr()
    }

    fn parse_or_expr(&mut self) -> Result<(), ParseError> {
        self.parse_and_expr()?;
        while self.match_kw(TokenKind::OrOr) {
            self.parse_and_expr()?;
        }
        Ok(())
    }

    fn parse_and_expr(&mut self) -> Result<(), ParseError> {
        self.parse_eq_expr()?;
        while self.match_kw(TokenKind::AndAnd) {
            self.parse_eq_expr()?;
        }
        Ok(())
    }

    fn parse_eq_expr(&mut self) -> Result<(), ParseError> {
        self.parse_cmp_expr()?;
        while self.peek_kind() == &TokenKind::EqEq || self.peek_kind() == &TokenKind::Neq {
            self.advance();
            self.parse_cmp_expr()?;
        }
        Ok(())
    }

    fn parse_cmp_expr(&mut self) -> Result<(), ParseError> {
        self.parse_add_expr()?;
        while matches!(self.peek_kind(), TokenKind::Lt | TokenKind::Le | TokenKind::Gt | TokenKind::Ge) {
            self.advance();
            self.parse_add_expr()?;
        }
        Ok(())
    }

    fn parse_add_expr(&mut self) -> Result<(), ParseError> {
        self.parse_mul_expr()?;
        while self.peek_kind() == &TokenKind::Plus || self.peek_kind() == &TokenKind::Minus {
            self.advance();
            self.parse_mul_expr()?;
        }
        Ok(())
    }

    fn parse_mul_expr(&mut self) -> Result<(), ParseError> {
        self.parse_unary_expr()?;
        while self.peek_kind() == &TokenKind::Star || self.peek_kind() == &TokenKind::Slash {
            self.advance();
            self.parse_unary_expr()?;
        }
        Ok(())
    }

    fn parse_unary_expr(&mut self) -> Result<(), ParseError> {
        if self.peek_kind() == &TokenKind::Minus || self.peek_kind() == &TokenKind::Bang {
            self.advance();
            self.parse_unary_expr()?;
        } else {
            self.parse_postfix_expr()?;
        }
        Ok(())
    }

    fn parse_postfix_expr(&mut self) -> Result<(), ParseError> {
        self.parse_primary()?;
        loop {
            match self.peek_kind() {
                TokenKind::LParen => { self.advance(); self.parse_args()?; self.expect_kw(TokenKind::RParen)?; }
                TokenKind::LBracket => { self.advance(); self.parse_expr()?; self.expect_kw(TokenKind::RBracket)?; }
                TokenKind::Dot => { self.advance(); self.expect(TokenKind::Ident("".into()))?; }
                _ => break,
            }
        }
        Ok(())
    }

    fn is_ident_or_builtin(k: &TokenKind) -> bool {
        matches!(k, TokenKind::Ident(_) | TokenKind::TI32 | TokenKind::TF64
                     | TokenKind::TStr | TokenKind::TBool)
    }

    fn parse_primary(&mut self) -> Result<(), ParseError> {
        match self.peek_kind() {
            TokenKind::IntLit(_) | TokenKind::FloatLit(_) | TokenKind::StrLit(_)
            | TokenKind::True | TokenKind::False => { self.advance(); Ok(()) }
            k if Self::is_ident_or_builtin(k) => {
                self.advance();
                // Check for struct literal: Ident {
                if self.peek_kind() == &TokenKind::LBrace {
                    self.parse_struct_lit_tail()?;
                }
                Ok(())
            }
            TokenKind::LBracket => {
                self.advance();
                self.parse_args()?; // same separator as args, allows empty
                self.expect_kw(TokenKind::RBracket)?;
                Ok(())
            }
            TokenKind::LParen => {
                self.advance();
                self.parse_expr()?;
                self.expect_kw(TokenKind::RParen)?;
                Ok(())
            }
            _ => Err(ParseError {
                msg: format!("unexpected token in expression: {:?}", self.peek_kind()),
                line: self.peek().line, col: self.peek().col,
            }),
        }
    }

    fn parse_struct_lit_tail(&mut self) -> Result<(), ParseError> {
        // We've seen Ident {, now parse field inits
        self.advance(); // consume {
        if self.peek_kind() != &TokenKind::RBrace {
            self.parse_field_inits()?;
        }
        self.expect_kw(TokenKind::RBrace)?;
        Ok(())
    }

    fn parse_field_inits(&mut self) -> Result<(), ParseError> {
        self.parse_field_init()?;
        while self.match_kw(TokenKind::Comma) {
            self.parse_field_init()?;
        }
        Ok(())
    }

    fn parse_field_init(&mut self) -> Result<(), ParseError> {
        self.expect(TokenKind::Ident("".into()))?;
        self.expect_kw(TokenKind::Colon)?;
        self.parse_expr()?;
        Ok(())
    }

    // ── Args ────────────────────────────────────────────────────────────

    fn parse_args(&mut self) -> Result<(), ParseError> {
        // Empty args list is Ok
        if self.peek_kind() == &TokenKind::RParen || self.peek_kind() == &TokenKind::RBracket
            || self.peek_kind() == &TokenKind::RBrace
        {
            return Ok(());
        }
        self.parse_expr()?;
        while self.match_kw(TokenKind::Comma) {
            self.parse_expr()?;
        }
        Ok(())
    }

    // ── Statements (EBNF §5) ────────────────────────────────────────────

    fn parse_block(&mut self) -> Result<(), ParseError> {
        self.expect_kw(TokenKind::LBrace)?;
        while self.peek_kind() != &TokenKind::RBrace && self.peek_kind() != &TokenKind::Eof {
            self.parse_stmt()?;
        }
        self.expect_kw(TokenKind::RBrace)?;
        Ok(())
    }

    fn parse_stmt(&mut self) -> Result<(), ParseError> {
        match self.peek_kind() {
            TokenKind::Var => self.parse_var_decl(),
            TokenKind::Return => self.parse_return_stmt(),
            TokenKind::If => self.parse_if_stmt(),
            TokenKind::For => self.parse_for_stmt(),
            TokenKind::LBrace => self.parse_block(),
            // Expression statement or assignment: starts with ident/literal/prefix
            _ => self.parse_expr_or_assign(),
        }
    }

    fn parse_var_decl(&mut self) -> Result<(), ParseError> {
        // VarDecl = "var" VarBinding [ "," VarBinding ] "=" Expr ";"
        self.expect_kw(TokenKind::Var)?;
        self.parse_var_binding()?;
        if self.match_kw(TokenKind::Comma) {
            self.parse_var_binding()?;
        }
        self.expect_kw(TokenKind::Assign)?;
        self.parse_expr()?;
        self.expect_kw(TokenKind::Semi)?;
        Ok(())
    }

    fn parse_var_binding(&mut self) -> Result<(), ParseError> {
        // VarBinding = IDENT ":" Type  |  "_"
        let id = self.expect(TokenKind::Ident("".into()))?;
        // Check if this is the discard "_" (no type annotation)
        if let TokenKind::Ident(ref name) = id.kind {
            if name == "_" {
                return Ok(());
            }
        }
        self.expect_kw(TokenKind::Colon)?;
        self.parse_type()?;
        Ok(())
    }

    fn parse_expr_or_assign(&mut self) -> Result<(), ParseError> {
        // Parse LHS as expression first, then decide
        // Try parsing as expression
        if let Err(_) = self.parse_expr() {
            return Err(ParseError {
                msg: "expected statement".into(),
                line: self.peek().line, col: self.peek().col,
            });
        }

        // If '=' follows, it's an assignment (the LHS must be a valid lvalue)
        if self.peek_kind() == &TokenKind::Assign {
            self.advance();
            self.parse_expr()?;
            self.expect_kw(TokenKind::Semi)?;
            return Ok(());
        }

        // If postfix ops follow on the assign path, need to rewind and re-parse
        // Simple case: single ident = expr
        // For a[i] = expr or obj.f = expr, the expression includes the postfix ops
        // So we check: did the expression parse consume tokens past the LValue?
        // For the prototype, we handle the common cases:
        //   ident = expr
        //   ident[expr] = expr
        //   ident.expr = expr (but .expr is an expression... this is a known limitation)

        // Actually, the expression parser already consumed a[i] and obj.field as
        // postfix expressions. If '=' follows, we look back at whether the expression
        // was a valid lvalue shape. For the prototype, we accept all and let semantic
        // analysis catch invalid lvalues later.

        // Oops — we already parsed the expression. Now we see no '=', so it's an
        // expression statement. Need semicolon.
        self.expect_kw(TokenKind::Semi)?;
        Ok(())
    }

    fn parse_return_stmt(&mut self) -> Result<(), ParseError> {
        // ReturnStmt = "return" [ Expr [ "," Expr ] ] ";"
        self.expect_kw(TokenKind::Return)?;
        if self.peek_kind() == &TokenKind::Semi {
            self.advance();
        } else {
            self.parse_expr()?;
            if self.match_kw(TokenKind::Comma) {
                self.parse_expr()?;
            }
            self.expect_kw(TokenKind::Semi)?;
        }
        Ok(())
    }

    fn parse_if_stmt(&mut self) -> Result<(), ParseError> {
        self.expect_kw(TokenKind::If)?;
        self.parse_expr()?;
        self.expect_kw(TokenKind::Then)?;
        self.parse_stmt()?;
        if self.match_kw(TokenKind::Else) {
            self.parse_stmt()?;
        }
        Ok(())
    }

    fn parse_for_stmt(&mut self) -> Result<(), ParseError> {
        self.expect_kw(TokenKind::For)?;
        self.expect_kw(TokenKind::Var)?;
        self.expect(TokenKind::Ident("".into()))?;
        self.expect_kw(TokenKind::Colon)?;
        self.parse_type()?;
        self.expect_kw(TokenKind::Assign)?;
        self.parse_expr()?;
        self.expect_kw(TokenKind::Comma)?;
        self.parse_expr()?;
        self.expect_kw(TokenKind::Comma)?;
        // step: assignment without semicolon
        self.parse_expr()?; // LHS of assignment
        self.expect_kw(TokenKind::Assign)?;
        self.parse_expr()?;
        self.expect_kw(TokenKind::In)?;
        self.parse_block()?;
        Ok(())
    }

    // ── Top Level (EBNF §1, §3, §4) ─────────────────────────────────────

    fn parse_struct_def(&mut self) -> Result<(), ParseError> {
        self.expect_kw(TokenKind::Struct)?;
        self.expect(TokenKind::Ident("".into()))?;
        self.expect_kw(TokenKind::LBrace)?;
        while self.peek_kind() != &TokenKind::RBrace && self.peek_kind() != &TokenKind::Eof {
            self.parse_field()?;
        }
        self.expect_kw(TokenKind::RBrace)?;
        Ok(())
    }

    fn parse_field(&mut self) -> Result<(), ParseError> {
        self.expect(TokenKind::Ident("".into()))?;
        self.expect_kw(TokenKind::Colon)?;
        self.parse_type()?;
        self.expect_kw(TokenKind::Semi)?;
        Ok(())
    }

    fn parse_func_def(&mut self) -> Result<(), ParseError> {
        self.expect_kw(TokenKind::Def)?;
        self.expect(TokenKind::Ident("".into()))?;
        self.expect_kw(TokenKind::LParen)?;
        if self.peek_kind() != &TokenKind::RParen {
            self.parse_params()?;
        }
        self.expect_kw(TokenKind::RParen)?;
        self.expect_kw(TokenKind::Arrow)?;
        self.parse_return_type()?;
        self.parse_block()?;
        Ok(())
    }

    fn parse_params(&mut self) -> Result<(), ParseError> {
        self.parse_param()?;
        while self.match_kw(TokenKind::Comma) {
            self.parse_param()?;
        }
        Ok(())
    }

    fn parse_param(&mut self) -> Result<(), ParseError> {
        self.expect(TokenKind::Ident("".into()))?;
        self.expect_kw(TokenKind::Colon)?;
        self.parse_type()?;
        Ok(())
    }

    fn parse_program(&mut self) -> Result<(), ParseError> {
        while self.peek_kind() != &TokenKind::Eof {
            match self.peek_kind() {
                TokenKind::Struct => self.parse_struct_def()?,
                TokenKind::Def => self.parse_func_def()?,
                _ => return Err(ParseError {
                    msg: format!("expected struct or def, got {:?}", self.peek_kind()),
                    line: self.peek().line, col: self.peek().col,
                }),
            }
        }
        Ok(())
    }
}

// ── Main ───────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <file.kang>...", args[0]);
        process::exit(1);
    }

    let mut total = 0;
    let mut passed = 0;

    for path in &args[1..] {
        total += 1;
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => { eprintln!("FAIL {} — {}", path, e); continue; }
        };

        let mut lexer = Lexer::new(&source);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);

        match parser.parse_program() {
            Ok(()) => {
                println!("PASS {}", path);
                passed += 1;
            }
            Err(e) => {
                println!("FAIL {}", path);
                println!("     {}", e);
            }
        }
    }

    println!("\n───\n{} / {} files passed", passed, total);
    if passed < total { process::exit(1); }
}
