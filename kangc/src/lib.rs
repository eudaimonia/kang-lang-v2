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
/// compile_to_stage 在指定阶段停止并返回中间产物
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PipelineStage {
    Tokens = 0,    // 词法分析后停止，输出 Token Stream
    Ast = 1,       // 语法分析后停止，输出 AST
    TypedAst = 2,  // 语义分析后停止，输出 TypedProgram
    LlvmIr = 3,    // LLVM IR 生成后停止，输出 IR 文本
    Object = 4,    // 生成目标文件后停止，输出 .o 文件路径
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
/// - source: 源码文本
/// - file_path: 源码路径（用于错误报告和模块名）
/// - target_triple: 目标平台 triple（None = 宿主平台）
/// - stage: 终止阶段
/// - object_path: Object 阶段时输出的 .o 文件路径
/// - 返回: (CompilerStats, 可选输出文本)
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

/// 编译全流程: 源码 → 语义检查后的 TypedProgram + IR + 各阶段统计
/// 等效于 compile_to_stage(source, file_path, None, LlvmIr, None)
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

// ── 工程工具 ────────────────────────────────────────────────────────────────

/// 查找 workspace 根目录（kangc/Cargo.toml 的父目录）
///
/// 优先使用 KANG_HOME 环境变量，其次使用 CARGO_MANIFEST_DIR 编译期路径。
/// 供 CLI 和 REPL 共用，保持 kangrt 运行时库查找策略一致。
pub fn find_project_root() -> PathBuf {
    if let Ok(home) = std::env::var("KANG_HOME") {
        let p = PathBuf::from(&home);
        if p.is_dir() {
            return p;
        }
        eprintln!("警告: KANG_HOME 指向的路径不存在: {}", home);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("找不到 workspace 根目录，请设置 KANG_HOME 环境变量")
        .to_path_buf()
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

/// 查找交叉链接器 lld（Rust 工具链自带）
///
/// 当目标平台的 OS 与宿主不同时（如 macOS → Linux），系统 cc 无法链接，
/// 需要使用 lld 交叉链接器。
pub fn find_cross_linker() -> Result<String, String> {
    // 优先使用 LD 环境变量
    if let Ok(ld) = std::env::var("LD") {
        let p = PathBuf::from(&ld);
        if p.is_file() {
            return Ok(p.to_string_lossy().to_string());
        }
    }

    // 查找 Rust 工具链中的 lld
    let rustup_home = std::env::var("RUSTUP_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        format!("{}/.rustup", home)
    });
    if let Some(lld) = find_in_dir(&PathBuf::from(&rustup_home), "ld.lld") {
        return Ok(lld.to_string_lossy().to_string());
    }

    // 查找 Homebrew 中的 lld
    for prefix in &["/opt/homebrew/opt/llvm/bin/ld.lld", "/usr/local/opt/llvm/bin/ld.lld"] {
        let p = PathBuf::from(prefix);
        if p.is_file() {
            return Ok(p.to_string_lossy().to_string());
        }
    }

    // PATH 中查找
    if let Some(p) = resolve_from_path("ld.lld") {
        return Ok(p.to_string_lossy().to_string());
    }

    Err("无法找到交叉链接器 ld.lld。请安装 LLVM (brew install llvm) 或设置 LD 环境变量".into())
}

/// 在目录中递归查找文件名匹配的可执行文件（深度限制 4 层）
fn find_in_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    use std::fs;
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() && path.file_name().map_or(false, |n| n == name) {
            return Some(path);
        }
        if path.is_dir() && path.components().count() <= dir.components().count() + 4 {
            if let Some(found) = find_in_dir(&path, name) {
                return Some(found);
            }
        }
    }
    None
}

/// 从 target triple 提取 OS 类别
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TargetOs {
    Linux,
    MacOs,
    Windows,
    Unknown,
}

/// 从 target triple 提取目标 OS
pub fn target_os_from_triple(triple: &str) -> TargetOs {
    let t = triple.to_lowercase();
    if t.contains("linux") { TargetOs::Linux }
    else if t.contains("apple") || t.contains("darwin") { TargetOs::MacOs }
    else if t.contains("windows") || t.contains("msvc") { TargetOs::Windows }
    else { TargetOs::Unknown }
}

/// 当前宿主 OS（编译期已知）
pub fn host_os() -> TargetOs {
    match std::env::consts::OS {
        "macos" => TargetOs::MacOs,
        "linux" => TargetOs::Linux,
        "windows" => TargetOs::Windows,
        _ => TargetOs::Unknown,
    }
}

/// 判断 target triple 是否需要交叉链接（宿主 OS ≠ 目标 OS）
pub fn is_cross_os(target_triple: &str) -> bool {
    host_os() != target_os_from_triple(target_triple)
}

/// lld 的 -m 参数值，根据 target triple 和目标格式推断
pub fn lld_machine(target_triple: &str, target_os: TargetOs) -> &'static str {
    match target_os {
        TargetOs::Linux => {
            if target_triple.contains("x86_64") { "elf_x86_64" }
            else if target_triple.contains("aarch64") { "aarch64linux" }
            else if target_triple.contains("arm") { "armelf_linux_eabi" }
            else { "elf_x86_64" }
        }
        TargetOs::MacOs => {
            if target_triple.contains("aarch64") || target_triple.contains("arm64") { "arm64" }
            else if target_triple.contains("x86_64") { "x86_64" }
            else { "arm64" }
        }
        _ => "elf_x86_64",
    }
}

/// 查找目标平台的 sysroot / 运行时库
///
/// - Linux musl: 返回 self-contained 目录（含 musl libc.a、crt*.o）
/// - macOS: 返回 Apple SDK 路径或 None（宿主 macOS 时系统 cc 自带 SDK）
pub fn find_target_sysroot(target_triple: &str) -> Option<PathBuf> {
    let target_os = target_os_from_triple(target_triple);
    let rustup_home = std::env::var("RUSTUP_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        format!("{}/.rustup", home)
    });
    let rustup = PathBuf::from(&rustup_home);

    match target_os {
        TargetOs::Linux => {
            // musl: 查找 self-contained/libc.a
            let toolchains = rustup.join("toolchains");
            for entry in std::fs::read_dir(&toolchains).ok()? {
                let entry = entry.ok()?;
                let lib_dir = entry.path().join("lib").join("rustlib").join(target_triple).join("lib").join("self-contained");
                if lib_dir.join("libc.a").is_file() {
                    return Some(lib_dir);
                }
            }
            None
        }
        TargetOs::MacOs => {
            // macOS SDK: SDKROOT 环境变量 → xcrun → 常见路径
            if let Ok(sdk) = std::env::var("SDKROOT") {
                let p = PathBuf::from(&sdk);
                if p.is_dir() { return Some(p); }
            }
            // 尝试 xcrun（仅 macOS）
            if let Ok(output) = std::process::Command::new("xcrun")
                .args(["--show-sdk-path", "-sdk", "macosx"])
                .output()
            {
                if output.status.success() {
                    let p = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
                    if p.is_dir() { return Some(p); }
                }
            }
            // 常见 SDK 路径
            for sdk in &[
                "/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk",
                "/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk",
            ] {
                let p = PathBuf::from(sdk);
                if p.is_dir() { return Some(p); }
            }
            // 宿主 macOS 使用系统 cc 时不需要 SDK 路径（cc 内部已知）
            // 仅交叉编译（Linux → macOS）时需提示设置 SDKROOT
            None
        }
        _ => None,
    }
}
