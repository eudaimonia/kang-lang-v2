// M5 集成测试 — 管线阶段截断、目标文件生成、端到端编译

use kangc::{compile_to_stage, PipelineStage};
use std::path::Path;
use std::process::Command;

// ── 辅助 ────────────────────────────────────────────────────────────────────

/// 编译到指定阶段，返回 (stats, output_text)
fn compile(source: &str, stage: PipelineStage) -> Result<(kangc::stats::CompilerStats, Option<String>), kangc::error::KangError> {
    compile_to_stage(source, "<test>", None, stage, None)
}

/// 编译到 Object 阶段，写入临时 .o 文件
fn compile_to_object_file(source: &str, obj_path: &Path) -> Result<kangc::stats::CompilerStats, kangc::error::KangError> {
    let (stats, _) = compile_to_stage(source, "<test>", None, PipelineStage::Object, Some(obj_path))?;
    Ok(stats)
}

// ── 阶段截断测试 ────────────────────────────────────────────────────────────

#[test]
fn stage_tokens_returns_formatted_tokens() {
    let (_, output) = compile("def f() -> i32 { return 42; }", PipelineStage::Tokens).unwrap();
    let text = output.unwrap();
    assert!(text.contains("DEF"), "应包含 DEF token，got: {}", text);
    assert!(text.contains("RETURN"), "应包含 RETURN token");
    assert!(!text.contains("program"), "不应包含 AST 内容");
}

#[test]
fn stage_ast_returns_s_expr() {
    let (_, output) = compile("def f() -> i32 { return 42; }", PipelineStage::Ast).unwrap();
    let text = output.unwrap();
    assert!(text.contains("(program"), "应包含 s-expr 格式 AST");
    assert!(text.contains("(func-def"), "应包含函数定义");
    assert!(!text.contains("DEF"), "不应包含 token 格式内容");
}

#[test]
fn stage_typed_ast_returns_typed_program_dump() {
    let (_, output) = compile("def f() -> i32 { return 42; }", PipelineStage::TypedAst).unwrap();
    let text = output.unwrap();
    assert!(text.contains("TypedProgram"), "应包含 TypedProgram 调试输出，got: {}", text);
}

#[test]
fn stage_llvm_ir_returns_module() {
    let (_, output) = compile("def f() -> i32 { return 42; }", PipelineStage::LlvmIr).unwrap();
    let text = output.unwrap();
    assert!(text.contains("source_filename"), "LLVM IR 应包含 source_filename");
    assert!(text.contains("ModuleID"), "LLVM IR 应包含 ModuleID");
}

#[test]
fn stage_object_writes_file() {
    let source = "def f() -> i32 { return 42; }";
    let tmp = std::env::temp_dir().join("test_kang_m5.o");
    let _ = std::fs::remove_file(&tmp);

    let (_stats, output) = compile_to_stage(
        source, "<test>", None,
        PipelineStage::Object,
        Some(&tmp),
    ).unwrap();

    assert!(tmp.exists(), ".o 文件应被创建");
    let meta = std::fs::metadata(&tmp).unwrap();
    assert!(meta.len() > 0, ".o 文件不应为空");

    // output 是 .o 路径
    assert!(output.is_some(), "Object 阶段应返回 .o 路径");

    let _ = std::fs::remove_file(&tmp);
}

// ── 编译器统计测试 ──────────────────────────────────────────────────────────

#[test]
fn stats_populated_for_each_stage() {
    let (stats, _) = compile("def f() -> i32 { return 42; }", PipelineStage::LlvmIr).unwrap();

    // 每个阶段都已填充（到 LlvmIr 为止）
    assert!(stats.source.total_bytes > 0, "应统计源文件大小");
    assert!(stats.lex.token_count > 0, "应统计 token 数量");
    assert!(stats.parse.func_count > 0, "应统计函数数量");
    assert!(stats.semantic.symbol_count > 0, "应统计符号数量");
    assert!(stats.codegen.llvm_ir_bytes > 0, "应统计 IR 大小");
}

#[test]
fn stats_truncated_at_earlier_stage() {
    // 到 Tokens 阶段时，codegen 统计应为默认值
    let (stats, _) = compile("def f() -> i32 { return 42; }", PipelineStage::Tokens).unwrap();
    assert!(stats.lex.token_count > 0, "lex 统计应填充");
    assert_eq!(stats.parse.func_count, 0, "parse 统计应未填充");
    assert_eq!(stats.codegen.llvm_ir_bytes, 0, "codegen 统计应未填充");
}

// ── 错误处理测试 ────────────────────────────────────────────────────────────

#[test]
fn lex_error_halts_pipeline() {
    let result = compile("def f() -> i32 { return @; }", PipelineStage::Ast);
    assert!(result.is_err(), "非法字符应导致词法错误");
    match result {
        Err(kangc::error::KangError::Lex(_)) => {} // expected
        _ => panic!("应返回 Lex 错误"),
    }
}

#[test]
fn semantic_error_halts_pipeline() {
    // 类型不匹配：return 类型与函数签名不一致
    let result = compile("def f() -> i32 { return \"hello\"; }", PipelineStage::LlvmIr);
    assert!(result.is_err(), "类型错误应导致语义错误");
    match result {
        Err(kangc::error::KangError::Semantic(_)) => {} // expected
        _ => panic!("应返回 Semantic 错误, got: {:?}", result.err()),
    }
}

// ── PipelineStage 测试 ──────────────────────────────────────────────────────

#[test]
fn pipeline_stage_from_emit_flag() {
    assert_eq!(PipelineStage::from_emit_flag("tokens"), Some(PipelineStage::Tokens));
    assert_eq!(PipelineStage::from_emit_flag("ast"), Some(PipelineStage::Ast));
    assert_eq!(PipelineStage::from_emit_flag("typed-ast"), Some(PipelineStage::TypedAst));
    assert_eq!(PipelineStage::from_emit_flag("llvm-ir"), Some(PipelineStage::LlvmIr));
    assert_eq!(PipelineStage::from_emit_flag("object"), Some(PipelineStage::Object));
    assert_eq!(PipelineStage::from_emit_flag(""), None);
    assert_eq!(PipelineStage::from_emit_flag("garbage"), None);
}

#[test]
fn pipeline_stage_ordering() {
    assert!(PipelineStage::Tokens < PipelineStage::Ast);
    assert!(PipelineStage::Ast < PipelineStage::TypedAst);
    assert!(PipelineStage::TypedAst < PipelineStage::LlvmIr);
    assert!(PipelineStage::LlvmIr < PipelineStage::Object);
}

// ── 目标文件测试 ────────────────────────────────────────────────────────────

#[test]
fn object_file_is_valid_macho() {
    let source = "def main() -> i32 { return 0; }";
    let tmp = std::env::temp_dir().join("test_m5_macho.o");
    let _ = std::fs::remove_file(&tmp);

    compile_to_object_file(source, &tmp).unwrap();

    // macOS 上 .o 文件应为 Mach-O 格式
    let bytes = std::fs::read(&tmp).unwrap();
    let magic = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    // Mach-O 64-bit magic: 0xFEEDFACF (reverse: CF FA ED FE)
    assert!(
        magic == 0xFEEDFACF || magic == 0xCEFAEDFE || magic == 0xFEEDFACE || magic == 0xCEFAEDFE,
        "应包含合法的 Mach-O magic, got 0x{:08X}", magic
    );

    let _ = std::fs::remove_file(&tmp);
}

// ── 端到端 AOT 测试 ─────────────────────────────────────────────────────────
// 通过 cargo run 调用 kangc CLI，验证编译+执行全流程
// 标记为 #[ignore] 因为需要完整的构建环境

/// 使用 kangc CLI 编译并执行 Kang 程序
/// 需要先 `cargo build -p kangc` 构建二进制，然后运行:
/// `cargo test -p kangc --test m5_pipeline_tests -- --ignored`
fn kang_run_e2e(source: &str) -> (String, String, i32) {
    let id: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let src_file = std::env::temp_dir().join(format!("test_e2e_{}.kang", id));
    std::fs::write(&src_file, source).unwrap();

    let exe_path = src_file.with_extension("");

    // 通过 cargo run 执行（需要完整的 cargo 构建环境）
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();

    let output = Command::new("cargo")
        .args(["run", "-q", "-p", "kangc", "--", "run", src_file.to_str().unwrap()])
        .current_dir(workspace_root)
        .output()
        .unwrap();

    let _ = std::fs::remove_file(&src_file);
    let _ = std::fs::remove_file(&exe_path);
    let _ = std::fs::remove_file(src_file.with_extension("o"));

    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
#[ignore] // 需完整构建环境，运行较慢
fn e2e_puts_hello() {
    let source = "def main() -> i32 { puts(\"hello\"); return 0; }";
    let (stdout, stderr, exit_code) = kang_run_e2e(source);
    assert_eq!(exit_code, 0, "应成功退出, stderr: {}", stderr);
    assert!(stdout.contains("hello"), "stdout 应包含 'hello', got: '{}', stderr: '{}'", stdout, stderr);
}

#[test]
#[ignore]
fn e2e_return_exit_code() {
    let source = "def main() -> i32 { return 42; }";
    let (stdout, stderr, exit_code) = kang_run_e2e(source);
    assert_eq!(exit_code, 42, "退出码应为 42, got: {}, stdout: '{}', stderr: '{}'", exit_code, stdout, stderr);
}

#[test]
#[ignore]
fn e2e_arithmetic() {
    let source = "def main() -> i32 { var x: i32 = 10 + 32; puts(str(x)); return 0; }";
    let (stdout, stderr, exit_code) = kang_run_e2e(source);
    assert_eq!(exit_code, 0, "应成功退出, stderr: {}", stderr);
    assert!(stdout.contains("42"), "应输出 42, got: '{}', stderr: '{}'", stdout, stderr);
}

// ── M7 多文件编译集成测试 ──────────────────────────────────────────────────

/// 创建临时测试目录，包含多个 .kang 源文件，运行 CLI 编译执行
fn kang_run_multi(sources: &[(&str, &str)], main_file: &str) -> (String, String, i32) {
    // 确保目录名唯一，防止并行测试冲突
    let id: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("test_m7_{}", id));
    let _ = std::fs::create_dir_all(&dir);

    for (name, content) in sources {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
    }

    let main_path = dir.join(main_file);

    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();

    let output = Command::new("cargo")
        .args(["run", "-q", "-p", "kangc", "--", "run", main_path.to_str().unwrap()])
        .current_dir(workspace_root)
        .output()
        .unwrap();

    // 清理
    let _ = std::fs::remove_dir_all(&dir);

    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
#[ignore]
fn e2e_m7_simple_import() {
    let sources = [
        ("math.kang", "def add(a:i32, b:i32) -> i32 { return a + b; }\ndef sub(a:i32, b:i32) -> i32 { return a - b; }\n"),
        ("main.kang", "import m { add, sub } from \"./math.kang\";\ndef main() -> i32 {\n    var x:i32 = m.add(10, 20);\n    var y:i32 = m.sub(x, 5);\n    puts(str(y));\n    return 0;\n}\n"),
    ];
    let (stdout, stderr, exit_code) = kang_run_multi(&sources, "main.kang");
    assert_eq!(exit_code, 0, "应成功退出, stderr: {}", stderr);
    assert!(stdout.contains("25"), "应输出 25, got: '{}'", stdout);
}

#[test]
#[ignore]
fn e2e_m7_multi_module() {
    let sources = [
        ("mod1.kang", "def one() -> i32 { return 1; }\n"),
        ("mod2.kang", "def two() -> i32 { return 2; }\n"),
        ("main.kang", "import a { one } from \"./mod1.kang\";\nimport b { two } from \"./mod2.kang\";\ndef main() -> i32 {\n    var x:i32 = a.one() + b.two();\n    puts(str(x));\n    return 0;\n}\n"),
    ];
    let (stdout, stderr, exit_code) = kang_run_multi(&sources, "main.kang");
    assert_eq!(exit_code, 0, "应成功退出, stderr: {}", stderr);
    assert!(stdout.contains("3"), "应输出 3, got: '{}'", stdout);
}

#[test]
#[ignore]
fn e2e_m7_import_error_file_not_found() {
    let sources = [
        ("main.kang", "import m { add } from \"./nonexistent.kang\";\ndef main() -> i32 { return 0; }\n"),
    ];
    let (_stdout, stderr, exit_code) = kang_run_multi(&sources, "main.kang");
    assert_ne!(exit_code, 0, "应失败退出");
    assert!(stderr.contains("无法找到导入文件"), "应报告文件不存在, stderr: {}", stderr);
}

#[test]
#[ignore]
fn e2e_m7_import_error_item_not_found() {
    let sources = [
        ("lib.kang", "def real_func() -> i32 { return 1; }\n"),
        ("main.kang", "import m { nonexistent } from \"./lib.kang\";\ndef main() -> i32 { return 0; }\n"),
    ];
    let (_stdout, stderr, exit_code) = kang_run_multi(&sources, "main.kang");
    assert_ne!(exit_code, 0, "应失败退出");
    assert!(stderr.contains("未找到"), "应报告未找到, stderr: {}", stderr);
}
