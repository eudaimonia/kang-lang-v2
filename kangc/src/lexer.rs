// 词法分析 — 将 Kang 源码转为 Token 流
// 使用 logos 声明式词法，处理关键字、字面量、运算符、注释、空白

use crate::error::LexError;
use crate::stats::LexStats;
use logos::Logos;
use std::collections::HashMap;
use std::fmt::Write;
use std::ops::Range;
use std::time::Instant;

// ── Token ────────────────────────────────────────────────────────────────────

#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r\n]+")]
pub enum TokenKind {
    // 关键字
    #[token("def")]    Def,
    #[token("var")]    Var,
    #[token("return")] Return,
    #[token("if")]     If,
    #[token("else")]   Else,
    #[token("then")]   Then,
    #[token("for")]    For,
    #[token("in")]     In,
    #[token("struct")] Struct,

    // 类型关键字
    #[token("i32")]  TI32,
    #[token("f64")]  TF64,
    #[token("str")]  TStr,
    #[token("bool")] TBool,
    #[token("void")] TVoid,

    // 布尔字面量
    #[token("true")]  True,
    #[token("false")] False,

    // 运算符 / 分隔符
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

    // 字面量
    #[regex(r"[0-9]+\.[0-9]+", |lex| lex.slice().to_string())]
    FloatLit(String),

    #[regex(r"[0-9]+", |lex| lex.slice().to_string())]
    IntLit(String),

    #[regex(r#""([^"\\]|\\(n|t|\\|"|0))*""#, parse_string)]
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
    let inner = &raw[1..raw.len() - 1]; // 去掉首尾引号
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
                _ => {} // 正则保证不会到这里
            }
        } else {
            result.push(c);
        }
    }
    Some(result)
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
    pub span: Range<usize>,
}

// ── 词法分析入口 ────────────────────────────────────────────────────────────

/// 将源码转为 Token 流，同时收集统计数据
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
                    msg: format!("无法识别的字符 '{}'", &source[span.clone()]),
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
fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let prefix = &source[..offset.min(source.len())];
    let line = prefix.chars().filter(|&c| c == '\n').count() + 1;
    let last_newline = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = prefix[last_newline..].chars().count() + 1;
    (line, col)
}

// ── Token 序列化 ────────────────────────────────────────────────────────────

/// 按 SPECS 4.1 格式输出 Token Stream
pub fn format_tokens(tokens: &[Token]) -> String {
    let mut out = String::new();
    for t in tokens {
        let kind_name = format!("{:?}", t.kind).to_uppercase();
        let lexeme = token_lexeme(t);
        writeln!(&mut out, "{} {:?} @ {}:{}", kind_name, lexeme, t.line, t.col).unwrap();
    }
    out
}

/// 获取 token 的词素文本
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
