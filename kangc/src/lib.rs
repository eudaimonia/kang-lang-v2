// kangc — Kang 编译器库
// 提供各编译阶段的公共 API: tokenize → parse → check → codegen

pub mod ast;
pub mod codegen;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod semantic;
pub mod stats;

use error::{CodeGenError, KangError, LexError, ParseError, SemanticError};
use lexer::tokenize as lex_tokenize;
use parser::parse as parse_tokens;
use stats::{CodeGenResult, CodeGenStats, LexStats, ParseStats, SemanticStats, SourceStats};

/// 词法分析: 源码 → Token 流
pub fn tokenize(source: &str, stats: &mut LexStats) -> Result<Vec<lexer::Token>, LexError> {
    lex_tokenize(source, stats)
}

/// 语法分析: Token 流 → AST
pub fn parse(tokens: &[lexer::Token], stats: &mut ParseStats) -> Result<ast::Program, ParseError> {
    parse_tokens(tokens, stats)
}

/// 语义分析: AST → TypedProgram
pub fn check(program: &ast::Program, stats: &mut SemanticStats) -> Result<semantic::TypedProgram, Vec<SemanticError>> {
    semantic::check(program, stats)
}

/// 代码生成: TypedProgram → CodeGenResult
pub fn codegen(program: &semantic::TypedProgram, stats: &mut CodeGenStats) -> Result<CodeGenResult, CodeGenError> {
    codegen::codegen(program, stats)
}

/// 编译全流程: 源码 → 语义检查后的 TypedProgram + IR
pub fn compile_full(
    source: &str,
    file_path: &str,
) -> Result<(semantic::TypedProgram, CodeGenResult, SourceStats, LexStats, ParseStats, SemanticStats, CodeGenStats), KangError> {
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

    let mut sem_stats = SemanticStats {
        duration_us: 0,
        error_count: 0,
        warning_count: 0,
        symbol_count: 0,
        type_check_passes: 0,
        type_check_failures: 0,
    };

    let mut cg_stats = CodeGenStats {
        duration_us: 0,
        llvm_ir_bytes: 0,
        llvm_instruction_count: 0,
        llvm_basic_block_count: 0,
        llvm_function_count: 0,
        runtime_check_insertions: 0,
    };

    let tokens = tokenize(source, &mut lex_stats).map_err(KangError::Lex)?;
    let program = parse_tokens(&tokens, &mut parse_stats).map_err(KangError::Parse)?;
    let typed = match semantic::check(&program, &mut sem_stats) {
        Ok(tp) => tp,
        Err(errors) => return Err(KangError::Semantic(errors.into_iter().next().unwrap())),
    };
    let result = codegen(&typed, &mut cg_stats).map_err(KangError::CodeGen)?;

    Ok((typed, result, source_stats, lex_stats, parse_stats, sem_stats, cg_stats))
}
