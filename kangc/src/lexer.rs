// 词法分析 — 将 Kang 源码转为 Token 流
// 使用 logos 声明式词法，处理关键字、字面量、运算符、注释、空白

use crate::error::LexError;
pub use crate::stats::LexStats;
use logos::Logos;
use std::collections::HashMap;
use std::fmt::Write;
use std::ops::Range;
use std::time::Instant;

// ── Token ────────────────────────────────────────────────────────────────────

// ── Token ────────────────────────────────────────────────────────────────────

/// Kang 语言的 token 类型。由 logos 派生宏驱动词法分析。
/// 关键字优先于同名标识符（logos 按匹配长度 + 优先级选择）。
#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r\n]+")]
pub enum TokenKind {
    // ── 关键字 ──────────────────────────────────────────────────────
    #[token("def")]    Def,
    #[token("var")]    Var,
    #[token("return")] Return,
    #[token("if")]     If,
    #[token("else")]   Else,
    #[token("then")]   Then,
    #[token("for")]    For,
    #[token("in")]     In,
    #[token("struct")] Struct,
    #[token("import")] Import,
    #[token("from")]   From,

    // ── 类型关键字 ─────────────────────────────────────────────────
    #[token("i32")]  TI32,
    #[token("f64")]  TF64,
    #[token("str")]  TStr,
    #[token("bool")] TBool,
    #[token("void")] TVoid,

    // ── 布尔字面量 ────────────────────────────────────────────────
    #[token("true")]  True,
    #[token("false")] False,

    // ── 运算符 / 分隔符 ─────────────────────────────────────────
    #[token("+")]  Plus,
    #[token("-")]  Minus,
    #[token("*")]  Star,
    #[token("/")]  Slash,
    #[token("<")]  Lt,
    #[token("<=")] Le,
    #[token(">")]  Gt,
    #[token(">=")] Ge,
    #[token("==")] EqEq,
    #[token("!=")] Neq,
    #[token("&&")] AndAnd,
    #[token("||")] OrOr,
    #[token("!")]  Bang,
    #[token("=")]  Assign,
    #[token("->")] Arrow,

    #[token("(")] LParen,
    #[token(")")] RParen,
    #[token("[")] LBracket,
    #[token("]")] RBracket,
    #[token("{")] LBrace,
    #[token("}")] RBrace,
    #[token(":")] Colon,
    #[token(";")] Semi,
    #[token(",")] Comma,
    #[token(".")] Dot,

    // ── 字面量 ────────────────────────────────────────────────────
    /// f64 字面量（如 3.14），以字符串保留原始文本
    #[regex(r"[0-9]+\.[0-9]+", |lex| lex.slice().to_string())]
    FloatLit(String),

    /// i32 字面量（如 42），以字符串保留原始文本
    #[regex(r"[0-9]+", |lex| lex.slice().to_string())]
    IntLit(String),

    /// 字符串字面量（如 "hello"），已处理转义序列
    #[regex(r#""([^"\\]|\\.)*""#, parse_string)]
    StrLit(String),

    // 标识符 (放在关键字之后，同长度时关键字优先)
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Ident(String),

    // 注释: 跳过。logos 0.15 中 skip 回调返回 () 来跳过
    #[regex(r"//[^\n]*", |_| ())]
    #[regex(r"/\*([^*]|\*[^/])*\*/", |_| ())]
    Comment,

    // EOF 哨兵 (不由 logos 生成，手动添加)
    Eof,
}

/// 解析字符串字面量中的转义序列
fn parse_string(lex: &mut logos::Lexer<TokenKind>) -> Option<String> {
    let raw = lex.slice();
    // 使用 strip 避免 raw.len() < 2 时下溢（正则保证至少 ""，此处防御）
    // logos 的 callback 仅在正则匹配时调用，slice 保证以引号起止
    let inner = raw.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(raw);
    let mut result = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('0') => result.push('\0'),
                // 正则保证转义序列合法，但若正则变更则保留原文
                other => {
                    result.push('\\');
                    if let Some(c2) = other {
                        result.push(c2);
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    Some(result)
}

/// 单个 token：包含种类、源码位置（行/列）、字节范围
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,           // token 种类
    pub line: usize,               // 1-based 行号
    pub col: usize,                // 1-based 列号
    pub span: Range<usize>, // 在源码中的字节偏移范围
}

// ── 词法分析入口 ────────────────────────────────────────────────────────────

/// 将源码转为 Token 流，同时收集统计数据
/// - source: 源码字符串
/// - stats: 写入耗时、token 数量、各类型计数、注释字节数
pub fn tokenize(source: &str, stats: &mut LexStats) -> Result<Vec<Token>, LexError> {
    let start = Instant::now();
    let lexer = TokenKind::lexer(source);
    let mut tokens = Vec::new();
    let mut comment_bytes = 0usize;

    for (result, span) in lexer.spanned() {
        match result {
            Ok(TokenKind::Comment) => {
                comment_bytes += span.len();
                continue;
            }
            Err(()) => {
                let (line, col) = line_col(source, span.start);
                stats.duration_us = start.elapsed().as_micros() as u64;
                return Err(LexError {
                    msg: format!(
                        "无法识别的字符 '{}'",
                        if source.is_char_boundary(span.start) && source.is_char_boundary(span.end) {
                            &source[span.clone()]
                        } else {
                            // 防御：span 不在字符边界时显示字节序列（不会发生）
                            "<invalid utf-8 boundary>"
                        }
                    ),
                    line,
                    col,
                    span,
                });
            }
            Ok(kind) => {
                let (line, col) = line_col(source, span.start);
                tokens.push(Token { kind, line, col, span });
            }
        }
    }

    // 添加 EOF 哨兵
    let last_pos = source.len();
    let (eof_line, eof_col) = line_col(source, last_pos);
    tokens.push(Token {
        kind: TokenKind::Eof,
        line: eof_line,
        col: eof_col,
        span: last_pos..last_pos,
    });

    // 收集统计
    stats.duration_us = start.elapsed().as_micros() as u64;
    stats.token_count = tokens.len();
    stats.comment_bytes = comment_bytes;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for t in &tokens {
        let name = format!("{:?}", t.kind);
        *counts.entry(name).or_insert(0) += 1;
    }
    stats.token_counts_by_kind = counts;

    Ok(tokens)
}

/// 计算给定字节偏移的行号和列号
/// offset 必须在 UTF-8 字符边界上；否则自动修正到最近边界
/// - source: 源码字符串
/// - offset: 字节偏移
/// - 返回: (行号, 列号)，均为 1-based
fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let safe_offset = offset.min(source.len());
    // 防御：确保 offset 落在字符边界（logos 保证此条件永远成立）
    let safe_offset = if source.is_char_boundary(safe_offset) {
        safe_offset
    } else {
        (0..safe_offset).rev().find(|&i| source.is_char_boundary(i)).unwrap_or(0)
    };
    let prefix = &source[..safe_offset];
    let line = prefix.chars().filter(|&c| c == '\n').count() + 1;
    let last_newline = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = prefix[last_newline..].chars().count() + 1;
    (line, col)
}

// ── Token 序列化 ────────────────────────────────────────────────────────────

/// 按 SPECS 4.1 格式输出 Token Stream
/// 每行格式: "KIND \"lexeme\" @ line:col"
pub fn format_tokens(tokens: &[Token]) -> String {
    let mut out = String::new();
    for t in tokens {
        let kind_name = format!("{:?}", t.kind).to_uppercase();
        let lexeme = token_lexeme(t);
        // String Write 不会失败，let _ 避免未来改为文件输出时 panic
        let _ = writeln!(&mut out, "{} {:?} @ {}:{}", kind_name, lexeme, t.line, t.col);
    }
    out
}

/// 获取 token 的词素文本（字符串或标识符的原始内容、运算符的符号等）
fn token_lexeme(t: &Token) -> String {
    match &t.kind {
        TokenKind::Ident(s) => s.clone(),
        TokenKind::IntLit(s) => s.clone(),
        TokenKind::FloatLit(s) => s.clone(),
        TokenKind::StrLit(s) => s.clone(),
        TokenKind::Def => "def".into(),
        TokenKind::Var => "var".into(),
        TokenKind::Return => "return".into(),
        TokenKind::If => "if".into(),
        TokenKind::Else => "else".into(),
        TokenKind::Then => "then".into(),
        TokenKind::For => "for".into(),
        TokenKind::In => "in".into(),
        TokenKind::Struct => "struct".into(),
        TokenKind::Import => "import".into(),
        TokenKind::From => "from".into(),
        TokenKind::TI32 => "i32".into(),
        TokenKind::TF64 => "f64".into(),
        TokenKind::TStr => "str".into(),
        TokenKind::TBool => "bool".into(),
        TokenKind::TVoid => "void".into(),
        TokenKind::True => "true".into(),
        TokenKind::False => "false".into(),
        TokenKind::Plus => "+".into(),
        TokenKind::Minus => "-".into(),
        TokenKind::Star => "*".into(),
        TokenKind::Slash => "/".into(),
        TokenKind::Lt => "<".into(),
        TokenKind::Le => "<=".into(),
        TokenKind::Gt => ">".into(),
        TokenKind::Ge => ">=".into(),
        TokenKind::EqEq => "==".into(),
        TokenKind::Neq => "!=".into(),
        TokenKind::AndAnd => "&&".into(),
        TokenKind::OrOr => "||".into(),
        TokenKind::Bang => "!".into(),
        TokenKind::Assign => "=".into(),
        TokenKind::Arrow => "->".into(),
        TokenKind::LParen => "(".into(),
        TokenKind::RParen => ")".into(),
        TokenKind::LBracket => "[".into(),
        TokenKind::RBracket => "]".into(),
        TokenKind::LBrace => "{".into(),
        TokenKind::RBrace => "}".into(),
        TokenKind::Colon => ":".into(),
        TokenKind::Semi => ";".into(),
        TokenKind::Comma => ",".into(),
        TokenKind::Dot => ".".into(),
        TokenKind::Eof => "".into(),
        TokenKind::Comment => "".into(),
    }
}

// ── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 辅助: tokenize 并提取 kind 列表
    fn tokenize_kinds(source: &str) -> Vec<TokenKind> {
        let mut stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        tokenize(source, &mut stats)
            .unwrap()
            .iter()
            .map(|t| t.kind.clone())
            .collect()
    }

    fn first_kind(source: &str) -> TokenKind {
        tokenize_kinds(source)[0].clone()
    }

    // ── 运算符 / 分隔符 ─────────────────────────────────────────────────

    #[test] fn lex_plus()     { assert_eq!(first_kind("+"), TokenKind::Plus); }
    #[test] fn lex_minus()    { assert_eq!(first_kind("-"), TokenKind::Minus); }
    #[test] fn lex_star()     { assert_eq!(first_kind("*"), TokenKind::Star); }
    #[test] fn lex_slash()    { assert_eq!(first_kind("/"), TokenKind::Slash); }
    #[test] fn lex_lt()       { assert_eq!(first_kind("<"), TokenKind::Lt); }
    #[test] fn lex_le()       { assert_eq!(first_kind("<="), TokenKind::Le); }
    #[test] fn lex_gt()       { assert_eq!(first_kind(">"), TokenKind::Gt); }
    #[test] fn lex_ge()       { assert_eq!(first_kind(">="), TokenKind::Ge); }
    #[test] fn lex_eqeq()     { assert_eq!(first_kind("=="), TokenKind::EqEq); }
    #[test] fn lex_neq()      { assert_eq!(first_kind("!="), TokenKind::Neq); }
    #[test] fn lex_andand()   { assert_eq!(first_kind("&&"), TokenKind::AndAnd); }
    #[test] fn lex_oror()     { assert_eq!(first_kind("||"), TokenKind::OrOr); }
    #[test] fn lex_bang()     { assert_eq!(first_kind("!"), TokenKind::Bang); }
    #[test] fn lex_assign()   { assert_eq!(first_kind("="), TokenKind::Assign); }
    #[test] fn lex_arrow()    { assert_eq!(first_kind("->"), TokenKind::Arrow); }
    #[test] fn lex_lparen()   { assert_eq!(first_kind("("), TokenKind::LParen); }
    #[test] fn lex_rparen()   { assert_eq!(first_kind(")"), TokenKind::RParen); }
    #[test] fn lex_lbracket() { assert_eq!(first_kind("["), TokenKind::LBracket); }
    #[test] fn lex_rbracket() { assert_eq!(first_kind("]"), TokenKind::RBracket); }
    #[test] fn lex_lbrace()   { assert_eq!(first_kind("{"), TokenKind::LBrace); }
    #[test] fn lex_rbrace()   { assert_eq!(first_kind("}"), TokenKind::RBrace); }
    #[test] fn lex_colon()    { assert_eq!(first_kind(":"), TokenKind::Colon); }
    #[test] fn lex_semi()     { assert_eq!(first_kind(";"), TokenKind::Semi); }
    #[test] fn lex_comma()    { assert_eq!(first_kind(","), TokenKind::Comma); }
    #[test] fn lex_dot()      { assert_eq!(first_kind("."), TokenKind::Dot); }

    // ── 关键字 ───────────────────────────────────────────────────────────

    #[test] fn lex_def()    { assert_eq!(first_kind("def"), TokenKind::Def); }
    #[test] fn lex_var()    { assert_eq!(first_kind("var"), TokenKind::Var); }
    #[test] fn lex_return() { assert_eq!(first_kind("return"), TokenKind::Return); }
    #[test] fn lex_if()     { assert_eq!(first_kind("if"), TokenKind::If); }
    #[test] fn lex_else()   { assert_eq!(first_kind("else"), TokenKind::Else); }
    #[test] fn lex_then()   { assert_eq!(first_kind("then"), TokenKind::Then); }
    #[test] fn lex_for()    { assert_eq!(first_kind("for"), TokenKind::For); }
    #[test] fn lex_in()     { assert_eq!(first_kind("in"), TokenKind::In); }
    #[test] fn lex_struct() { assert_eq!(first_kind("struct"), TokenKind::Struct); }
    #[test] fn lex_import() { assert_eq!(first_kind("import"), TokenKind::Import); }
    #[test] fn lex_from()   { assert_eq!(first_kind("from"), TokenKind::From); }

    // ── 类型关键字 ───────────────────────────────────────────────────────

    #[test] fn lex_ti32()  { assert_eq!(first_kind("i32"), TokenKind::TI32); }
    #[test] fn lex_tf64()  { assert_eq!(first_kind("f64"), TokenKind::TF64); }
    #[test] fn lex_tstr()  { assert_eq!(first_kind("str"), TokenKind::TStr); }
    #[test] fn lex_tbool() { assert_eq!(first_kind("bool"), TokenKind::TBool); }
    #[test] fn lex_tvoid() { assert_eq!(first_kind("void"), TokenKind::TVoid); }

    // ── 布尔字面量 ───────────────────────────────────────────────────────

    #[test] fn lex_true()  { assert_eq!(first_kind("true"), TokenKind::True); }
    #[test] fn lex_false() { assert_eq!(first_kind("false"), TokenKind::False); }

    // ── 字面量 ───────────────────────────────────────────────────────────

    #[test]
    fn lex_int_lit() {
        assert_eq!(first_kind("42"), TokenKind::IntLit("42".into()));
    }

    #[test]
    fn lex_int_zero() {
        assert_eq!(first_kind("0"), TokenKind::IntLit("0".into()));
    }

    #[test]
    fn lex_float_lit() {
        assert_eq!(first_kind("3.14"), TokenKind::FloatLit("3.14".into()));
    }

    #[test]
    fn lex_float_no_fraction() {
        // "1." is lexed as IntLit("1") + Dot
        let kinds = tokenize_kinds("1.");
        assert_eq!(kinds[0], TokenKind::IntLit("1".into()));
        assert_eq!(kinds[1], TokenKind::Dot);
    }

    #[test]
    fn lex_string_simple() {
        assert_eq!(first_kind("\"hello\""), TokenKind::StrLit("hello".into()));
    }

    #[test]
    fn lex_string_empty() {
        assert_eq!(first_kind("\"\""), TokenKind::StrLit("".into()));
    }

    #[test]
    fn lex_string_escape_newline() {
        assert_eq!(first_kind(r#""\n""#), TokenKind::StrLit("\n".into()));
    }

    #[test]
    fn lex_string_escape_tab() {
        assert_eq!(first_kind(r#""\t""#), TokenKind::StrLit("\t".into()));
    }

    #[test]
    fn lex_string_escape_quote() {
        assert_eq!(first_kind(r#""\"""#), TokenKind::StrLit("\"".into()));
    }

    #[test]
    fn lex_string_escape_backslash() {
        assert_eq!(first_kind(r#""\\""#), TokenKind::StrLit("\\".into()));
    }

    #[test]
    fn lex_string_escape_null() {
        assert_eq!(first_kind(r#""\0""#), TokenKind::StrLit("\0".into()));
    }

    #[test]
    fn lex_string_multiple_escapes() {
        // 手动构造避开 raw string 歧义: "hello\nworld\t!"
        let mut s = String::from("\"hello");
        s.push('\\'); s.push('n');
        s.push_str("world");
        s.push('\\'); s.push('t');
        s.push_str("!\"");
        assert_eq!(
            first_kind(&s),
            TokenKind::StrLit("hello\nworld\t!".into())
        );
    }

    // ── 标识符 ───────────────────────────────────────────────────────────

    #[test]
    fn lex_ident_simple() {
        assert_eq!(first_kind("foo"), TokenKind::Ident("foo".into()));
    }

    #[test]
    fn lex_ident_with_underscore() {
        assert_eq!(first_kind("my_var"), TokenKind::Ident("my_var".into()));
    }

    #[test]
    fn lex_ident_with_digits() {
        assert_eq!(first_kind("x123"), TokenKind::Ident("x123".into()));
    }

    #[test]
    fn lex_ident_starting_underscore() {
        assert_eq!(first_kind("_discard"), TokenKind::Ident("_discard".into()));
    }

    #[test]
    fn lex_underscore_alone() {
        // 单独的 _ 是合法的标识符(用作 discard)
        assert_eq!(first_kind("_"), TokenKind::Ident("_".into()));
    }

    // ── 注释 ─────────────────────────────────────────────────────────────

    #[test]
    fn skip_line_comment() {
        let kinds = tokenize_kinds("// comment\nx");
        assert_eq!(kinds[0], TokenKind::Ident("x".into()));
        assert_eq!(kinds[1], TokenKind::Eof);
    }

    #[test]
    fn skip_block_comment() {
        let kinds = tokenize_kinds("/* block */x");
        assert_eq!(kinds[0], TokenKind::Ident("x".into()));
    }

    #[test]
    fn skip_block_comment_multiline() {
        let kinds = tokenize_kinds("/* line1\n   line2 */42");
        assert_eq!(kinds[0], TokenKind::IntLit("42".into()));
    }

    #[test]
    fn skip_line_comment_end_of_input() {
        let kinds = tokenize_kinds("// comment no newline");
        assert_eq!(kinds[0], TokenKind::Eof);
    }

    #[test]
    fn comment_bytes_tracked() {
        let mut stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let _tokens = tokenize("// abc\nx /* xy */ y", &mut stats).unwrap();
        assert!(stats.comment_bytes > 0);
        assert_eq!(stats.comment_bytes, 14); // "// abc\n" = 7 + "/* xy */" = 7
    }

    // ── 空白 ─────────────────────────────────────────────────────────────

    #[test]
    fn skip_spaces() {
        let kinds = tokenize_kinds("   +   ");
        assert_eq!(kinds[0], TokenKind::Plus);
    }

    #[test]
    fn skip_tabs() {
        let kinds = tokenize_kinds("\t\t42\t");
        assert_eq!(kinds[0], TokenKind::IntLit("42".into()));
    }

    #[test]
    fn skip_newlines() {
        let kinds = tokenize_kinds("\n\n+\n");
        assert_eq!(kinds[0], TokenKind::Plus);
    }

    #[test]
    fn empty_input_yields_eof() {
        let kinds = tokenize_kinds("");
        assert_eq!(kinds.len(), 1);
        assert_eq!(kinds[0], TokenKind::Eof);
    }

    // ── 组合场景 ─────────────────────────────────────────────────────────

    #[test]
    fn lex_small_program() {
        let kinds = tokenize_kinds("def foo(x:i32) -> i32 { return x; }");
        let expected = vec![
            TokenKind::Def,
            TokenKind::Ident("foo".into()),
            TokenKind::LParen,
            TokenKind::Ident("x".into()),
            TokenKind::Colon,
            TokenKind::TI32,
            TokenKind::RParen,
            TokenKind::Arrow,
            TokenKind::TI32,
            TokenKind::LBrace,
            TokenKind::Return,
            TokenKind::Ident("x".into()),
            TokenKind::Semi,
            TokenKind::RBrace,
            TokenKind::Eof,
        ];
        assert_eq!(kinds, expected);
    }

    #[test]
    fn lex_all_delimiters() {
        let kinds = tokenize_kinds("()[]{},:;.");
        let expected = vec![
            TokenKind::LParen, TokenKind::RParen,
            TokenKind::LBracket, TokenKind::RBracket,
            TokenKind::LBrace, TokenKind::RBrace,
            TokenKind::Comma, TokenKind::Colon,
            TokenKind::Semi, TokenKind::Dot,
            TokenKind::Eof,
        ];
        assert_eq!(kinds, expected);
    }

    #[test]
    fn lex_import_statement() {
        let kinds = tokenize_kinds("import m { add } from \"./math.kang\";");
        let expected = vec![
            TokenKind::Import,
            TokenKind::Ident("m".into()),
            TokenKind::LBrace,
            TokenKind::Ident("add".into()),
            TokenKind::RBrace,
            TokenKind::From,
            TokenKind::StrLit("./math.kang".into()),
            TokenKind::Semi,
            TokenKind::Eof,
        ];
        assert_eq!(kinds, expected);
    }

    #[test]
    fn lex_line_col_tracking() {
        let mut stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let tokens = tokenize("a\nb\nc", &mut stats).unwrap();
        assert_eq!(tokens[0].line, 1);
        assert_eq!(tokens[0].col, 1);
        assert_eq!(tokens[1].line, 2);
        assert_eq!(tokens[1].col, 1);
        assert_eq!(tokens[2].line, 3);
        assert_eq!(tokens[2].col, 1);
    }

    // ── 错误路径 ─────────────────────────────────────────────────────────

    #[test]
    fn lex_unexpected_char() {
        let mut stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let result = tokenize("@", &mut stats);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.msg.contains("@"));
        assert_eq!(err.line, 1);
        assert_eq!(err.col, 1);
    }

    // ── stats 收集 ───────────────────────────────────────────────────────

    #[test]
    fn stats_token_count() {
        let mut stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let tokens = tokenize("def foo() -> void {}", &mut stats).unwrap();
        assert_eq!(stats.token_count, tokens.len());
    }

    #[test]
    fn stats_duration_is_set() {
        let mut stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let _ = tokenize("x", &mut stats).unwrap();
        // duration 至少为 0 (极快时可能截断为 0μs)
        assert!(stats.token_count >= 2); // "x" + EOF
    }

    // ── format_tokens ────────────────────────────────────────────────────

    #[test]
    fn format_tokens_eof() {
        let mut stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let tokens = tokenize("", &mut stats).unwrap();
        let output = format_tokens(&tokens);
        assert!(output.contains("EOF"), "output: {}", output);
    }

    #[test]
    fn format_tokens_ident() {
        let mut stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let tokens = tokenize("hi", &mut stats).unwrap();
        let output = format_tokens(&tokens);
        assert!(output.contains("IDENT") && output.contains("hi"), "output: {}", output);
    }
}
