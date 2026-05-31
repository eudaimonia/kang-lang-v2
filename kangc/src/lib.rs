// kangc — Kang 编译器库
// 提供各编译阶段的公共 API: tokenize → parse → check → codegen

pub mod ast;
pub mod codegen;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod repl;
pub mod semantic;
pub mod stats;

use error::{CodeGenError, KangError, LexError, ParseError, SemanticError};
use lexer::tokenize as lex_tokenize;
use parser::parse as parse_tokens;
use std::path::{Path, PathBuf};
use stats::{CodeGenResult, CodeGenStats, CompilerStats, LexStats, ParseStats, SemanticStats, SourceStats};

// ── 管线阶段 ────────────────────────────────────────────────────────────────

/// 编译管线截断阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PipelineStage {
    Tokens = 0,
    Ast = 1,
    TypedAst = 2,
    LlvmIr = 3,
    Object = 4,
}

impl PipelineStage {
    /// 从 --emit 字符串解析阶段
    pub fn from_emit_flag(s: &str) -> Option<Self> {
        match s {
            "tokens" => Some(PipelineStage::Tokens),
            "ast" => Some(PipelineStage::Ast),
            "typed-ast" => Some(PipelineStage::TypedAst),
            "llvm-ir" => Some(PipelineStage::LlvmIr),
            "object" => Some(PipelineStage::Object),
            _ => None,
        }
    }
}

// ── 共享管线 ────────────────────────────────────────────────────────────────

/// 运行编译管线到指定阶段，返回全量统计数据与可选的阶段输出文本
pub fn compile_to_stage(
    source: &str,
    file_path: &str,
    target_triple: Option<&str>,
    stage: PipelineStage,
    object_path: Option<&Path>,
) -> Result<(CompilerStats, Option<String>), KangError> {
    let source_stats = SourceStats {
        file_path: file_path.to_string(),
        total_bytes: source.len(),
        total_lines: source.lines().count(),
    };

    // Lex
    let mut lex_stats = LexStats::default();
    let tokens = tokenize(source, &mut lex_stats).map_err(KangError::Lex)?;
    if stage == PipelineStage::Tokens {
        let output = lexer::format_tokens(&tokens);
        let stats = CompilerStats { source: source_stats, lex: lex_stats, ..Default::default() };
        return Ok((stats, Some(output)));
    }

    // Parse
    let mut parse_stats = ParseStats::default();
    let program = parse_tokens(&tokens, &mut parse_stats).map_err(KangError::Parse)?;
    if stage == PipelineStage::Ast {
        let output = format!("{}", program);
        let stats = CompilerStats { source: source_stats, lex: lex_stats, parse: parse_stats, ..Default::default() };
        return Ok((stats, Some(output)));
    }

    // Semantic
    let mut sem_stats = SemanticStats::default();
    let typed = match semantic::check(&program, &mut sem_stats, file_path) {
        Ok(tp) => tp,
        Err(errors) => {
            let first = errors.into_iter().next().unwrap_or_else(|| SemanticError {
                msg: "语义检查失败（无具体错误信息）".into(),
                line: 0,
                col: 0,
                span: 0..0,
            });
            return Err(KangError::Semantic(first));
        }
    };
    if stage == PipelineStage::TypedAst {
        let output = format!("{:?}", typed);
        let stats = CompilerStats {
            source: source_stats, lex: lex_stats, parse: parse_stats, semantic: sem_stats,
            ..Default::default()
        };
        return Ok((stats, Some(output)));
    }

    // Codegen
    let module_name = std::path::Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("kang_module");
    let mut cg_stats = CodeGenStats::default();
    let cg_result = codegen::codegen(&typed, &mut cg_stats, target_triple, object_path, module_name).map_err(KangError::CodeGen)?;

    let stats = CompilerStats {
        source: source_stats, lex: lex_stats, parse: parse_stats, semantic: sem_stats, codegen: cg_stats,
    };

    if stage == PipelineStage::LlvmIr {
        Ok((stats, Some(cg_result.ir_text)))
    } else {
        // Object: output is the .o path
        Ok((stats, cg_result.object_file))
    }
}

// ── 公共 API ─────────────────────────────────────────────────────────────────

/// 词法分析: 源码 → Token 流
pub fn tokenize(source: &str, stats: &mut LexStats) -> Result<Vec<lexer::Token>, LexError> {
    lex_tokenize(source, stats)
}

/// 语法分析: Token 流 → AST
pub fn parse(tokens: &[lexer::Token], stats: &mut ParseStats) -> Result<ast::Program, ParseError> {
    parse_tokens(tokens, stats)
}

/// 语义分析: AST → TypedProgram
pub fn check(program: &ast::Program, stats: &mut SemanticStats, file_path: &str) -> Result<semantic::TypedProgram, Vec<SemanticError>> {
    semantic::check(program, stats, file_path)
}

/// 代码生成: TypedProgram → CodeGenResult
pub fn codegen(
    program: &semantic::TypedProgram,
    stats: &mut CodeGenStats,
    target_triple: Option<&str>,
    object_path: Option<&Path>,
) -> Result<CodeGenResult, CodeGenError> {
    codegen::codegen(program, stats, target_triple, object_path, "kang_module")
}

/// 编译全流程: 源码 → 语义检查后的 TypedProgram + IR
pub fn compile_full(
    source: &str,
    file_path: &str,
) -> Result<(semantic::TypedProgram, CodeGenResult, SourceStats, LexStats, ParseStats, SemanticStats, CodeGenStats), KangError> {
    let source_stats = SourceStats {
        file_path: file_path.to_string(),
        total_bytes: source.len(),
        total_lines: source.lines().count(),
    };

    let mut lex_stats = LexStats::default();
    let mut parse_stats = ParseStats::default();
    let mut sem_stats = SemanticStats::default();
    let mut cg_stats = CodeGenStats::default();

    let tokens = tokenize(source, &mut lex_stats).map_err(KangError::Lex)?;
    let program = parse_tokens(&tokens, &mut parse_stats).map_err(KangError::Parse)?;
    let typed = match semantic::check(&program, &mut sem_stats, file_path) {
        Ok(tp) => tp,
        Err(errors) => {
            let first = errors.into_iter().next().unwrap_or_else(|| SemanticError {
                msg: "语义检查失败（无具体错误信息）".into(),
                line: 0,
                col: 0,
                span: 0..0,
            });
            return Err(KangError::Semantic(first));
        }
    };
    let module_name = std::path::Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("kang_module");
    let result = codegen::codegen(&typed, &mut cg_stats, None, None, module_name).map_err(KangError::CodeGen)?;

    Ok((typed, result, source_stats, lex_stats, parse_stats, sem_stats, cg_stats))
}

// ── 链接器工具 ────────────────────────────────────────────────────────────────

/// 可信链接器目录白名单
pub const TRUSTED_LINKER_DIRS: &[&str] = &[
    "/usr/bin",
    "/usr/local/bin",
    "/opt/homebrew/bin",
];

/// 从 PATH 环境变量中查找可执行文件，返回绝对路径
pub fn resolve_from_path(bin: &str) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in path_var.split(':') {
        let full = PathBuf::from(dir).join(bin);
        if full.is_file() {
            return Some(full);
        }
    }
    None
}

/// 获取并校验链接器路径，返回绝对路径或错误描述。
///
/// 供 CLI 和 REPL 共用，保持链接器安全策略一致。
pub fn find_linker() -> Result<String, String> {
    let linker = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let resolved = if linker.contains('/') {
        PathBuf::from(&linker)
    } else {
        match resolve_from_path(&linker) {
            Some(p) => p,
            None => return Err(format!("无法在 PATH 中找到链接器 '{}'", linker)),
        }
    };
    let canon = resolved.canonicalize().map_err(|e| {
        format!("无法解析链接器路径 '{}': {}", resolved.display(), e)
    })?;
    let parent = canon.parent().unwrap_or(Path::new("/"));
    if !TRUSTED_LINKER_DIRS.iter().any(|d| parent.starts_with(d)) {
        return Err(format!(
            "链接器 '{}' 不在可信目录中。请使用系统 cc 或设置 CC 为可信路径",
            canon.display()
        ));
    }
    Ok(canon.to_string_lossy().to_string())
}
