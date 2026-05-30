// kangc — Kang 编译器库
// 提供各编译阶段的公共 API: tokenize → parse → check → codegen
// M1 实现 lexer + parser 阶段

pub mod ast;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod stats;

use error::{KangError, LexError, ParseError};
use lexer::tokenize as lex_tokenize;
use parser::parse as parse_tokens;
use stats::{LexStats, ParseStats, SourceStats};

/// 词法分析: 源码 → Token 流
pub fn tokenize(source: &str, stats: &mut LexStats) -> Result<Vec<lexer::Token>, LexError> {
    lex_tokenize(source, stats)
}

/// 语法分析: Token 流 → AST
pub fn parse(tokens: &[lexer::Token], stats: &mut ParseStats) -> Result<ast::Program, ParseError> {
    parse_tokens(tokens, stats)
}

/// 编译全流程: 源码 → AST (后续阶段在 M2-M4 接入)
pub fn compile_full(source: &str, file_path: &str) -> Result<(ast::Program, SourceStats, LexStats, ParseStats), KangError> {
    let total_lines = source.lines().count();

    let source_stats = SourceStats {
        file_path: file_path.to_string(),
        total_bytes: source.len(),
        total_lines,
    };

    let mut lex_stats = LexStats {
        duration_us: 0,
        token_count: 0,
        token_counts_by_kind: std::collections::HashMap::new(),
        comment_bytes: 0,
    };

    let mut parse_stats = ParseStats {
        duration_us: 0,
        ast_node_count: 0,
        ast_max_depth: 0,
        node_counts_by_kind: std::collections::HashMap::new(),
        func_count: 0,
        struct_count: 0,
    };

    let tokens = tokenize(source, &mut lex_stats).map_err(KangError::Lex)?;
    let program = parse_tokens(&tokens, &mut parse_stats).map_err(KangError::Parse)?;

    Ok((program, source_stats, lex_stats, parse_stats))
}
