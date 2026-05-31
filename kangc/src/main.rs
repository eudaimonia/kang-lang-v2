// kangc CLI — Kang 编译器命令行入口
// M1: kang lex / kang parse, M2: kang check, M4: kang codegen, M5: kang build / kang run
// M7: 多文件编译 + import 解析

use kangc::error::emit_diagnostic;
use kangc::stats::CompilerStats;
use kangc::{compile_to_stage, PipelineStage};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    let subcmd = &args[1];
    let rest = &args[2..];

    match subcmd.as_str() {
        "lex" => cmd_lex(rest),
        "parse" => cmd_parse(rest),
        "check" => cmd_check(rest),
        "codegen" | "emit-llvm" => cmd_codegen(rest),
        "build" => cmd_build(rest),
        "run" => cmd_run(rest),
        "repl" => kangc::repl::run_repl(),
        _ => {
            eprintln!("未知子命令: {}", subcmd);
            process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!("用法: kang <子命令> [参数]");
    eprintln!();
    eprintln!("子命令:");
    eprintln!("  lex       <file> [-o <file>] [--stats] [--target=<triple>]    词法分析, 输出 Token Stream");
    eprintln!("  parse     <file> [-o <file>] [--stats] [--target=<triple>]    语法分析, 输出 AST");
    eprintln!("  check     <file> [--stats]                                      语义分析, 报告错误或 OK");
    eprintln!("  codegen   <file> [-o <file>] [--stats] [--target=<triple>]    代码生成, 输出 LLVM IR");
    eprintln!("  emit-llvm <file> [-o <file>] [--stats] [--target=<triple>]    同 codegen");
    eprintln!("  build     <file> [-o <file>] [--stats] [--target=<triple>] [--emit=<stage>]  AOT 编译");
    eprintln!("  run       <file> [--stats] [--target=<triple>]                  编译并执行");
    eprintln!("  repl                                                               启动交互式解释器");
}

// ── 参数解析 ────────────────────────────────────────────────────────────────

struct CompileArgs {
    file_path: PathBuf,
    out_path: Option<PathBuf>,
    show_stats: bool,
    emit: Option<PipelineStage>,
    target: Option<String>,
}

/// 解析编译器 CLI 参数: 位置参数、-o、--stats、--emit=<stage>、--target=<triple>
fn parse_compile_args(args: &[String]) -> CompileArgs {
    let mut file_path = None;
    let mut out_path = None;
    let mut show_stats = false;
    let mut emit = None;
    let mut target = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                i += 1;
                if i < args.len() {
                    out_path = Some(PathBuf::from(&args[i]));
                } else {
                    eprintln!("-o 需要指定输出文件路径");
                    process::exit(1);
                }
            }
            "--stats" => {
                show_stats = true;
            }
            arg if arg.starts_with("--emit=") => {
                let stage_str = &arg["--emit=".len()..];
                emit = Some(PipelineStage::from_emit_flag(stage_str).unwrap_or_else(|| {
                    eprintln!("未知的 --emit 值: {} (支持: tokens, ast, typed-ast, llvm-ir, object)", stage_str);
                    process::exit(1);
                }));
            }
            arg if arg.starts_with("--target=") => {
                target = Some(arg["--target=".len()..].to_string());
            }
            arg if !arg.starts_with('-') && file_path.is_none() => {
                file_path = Some(PathBuf::from(arg));
            }
            _ => {
                eprintln!("未知参数: {}", args[i]);
                process::exit(1);
            }
        }
        i += 1;
    }

    let file_path = file_path.unwrap_or_else(|| {
        eprintln!("需要指定输入文件");
        process::exit(1);
    });

    CompileArgs { file_path, out_path, show_stats, emit, target }
}

// ── 各子命令 ────────────────────────────────────────────────────────────────

fn cmd_lex(args: &[String]) {
    let cargs = parse_compile_args(args);
    let stage = cargs.emit.unwrap_or(PipelineStage::Tokens);
    run_to_stage(cargs, stage, false);
}

fn cmd_parse(args: &[String]) {
    let cargs = parse_compile_args(args);
    let stage = cargs.emit.unwrap_or(PipelineStage::Ast);
    run_to_stage(cargs, stage, false);
}

fn cmd_check(args: &[String]) {
    let cargs = parse_compile_args(args);
    let stage = cargs.emit.unwrap_or(PipelineStage::TypedAst);
    run_to_stage(cargs, stage, true);
}

fn cmd_codegen(args: &[String]) {
    let cargs = parse_compile_args(args);
    let stage = cargs.emit.unwrap_or(PipelineStage::LlvmIr);
    run_to_stage(cargs, stage, false);
}

fn cmd_build(args: &[String]) {
    let cargs = parse_compile_args(args);
    let emit = cargs.emit;
    let stage = emit.unwrap_or(PipelineStage::Object);

    // --emit 截断到 IR 或更早阶段: 走文本输出路径
    if stage < PipelineStage::Object {
        run_to_stage(cargs, stage, false);
        return;
    }

    // 可执行文件路径
    let exe_path = cargs.out_path.clone().unwrap_or_else(|| {
        let mut p = cargs.file_path.clone();
        p.set_extension("");
        // 防止无扩展名或纯扩展名文件生成空文件名（如 ".kang" → ""）
        if p.file_name().map_or(true, |n| n.is_empty()) {
            p.set_file_name("a.out");
        }
        p
    });

    let target_triple = cargs.target.as_deref();
    let emit_obj_only = emit == Some(PipelineStage::Object);

    // M7: 收集 import 依赖
    let all_files = collect_imports(&cargs.file_path);

    if all_files.len() == 1 && !emit_obj_only {
        // 单文件: 使用原有简单流程
        let obj_path = cargs.file_path.with_extension("o");
        let source = read_source(&cargs.file_path);

        match compile_to_stage(
            &source,
            &cargs.file_path.to_string_lossy(),
            target_triple,
            PipelineStage::Object,
            Some(&obj_path),
        ) {
            Ok((stats, _)) => {
                let kangrt_path = find_or_build_kangrt(target_triple);
                link_executable(&obj_path, &kangrt_path, &exe_path, target_triple);
                remove_file(&obj_path);
                if cargs.show_stats {
                    print_stats(&stats, PipelineStage::Object);
                }
            }
            Err(e) => {
                emit_diagnostic(&e, &source, &cargs.file_path.to_string_lossy());
                process::exit(1);
            }
        }
    } else {
        // 多文件: 编译所有依赖单元
        if !emit_obj_only {
            eprintln!("发现 {} 个编译单元...", all_files.len());
        }

        let obj_files = compile_all_units(&all_files, target_triple, cargs.show_stats);

        if emit_obj_only {
            // --emit=object: 输出最后一个 .o 文件的路径
            if let (Some(p), Some(last)) = (&cargs.out_path, obj_files.last()) {
                std::fs::copy(last, p).unwrap_or_else(|e| {
                    eprintln!("复制目标文件失败: {}", e);
                    process::exit(1);
                });
            } else if let Some(last) = obj_files.last() {
                println!("{}", last.display());
            }
        } else {
            let kangrt_path = find_or_build_kangrt(target_triple);
            link_multi(&obj_files, &kangrt_path, &exe_path, target_triple);
            // 清理 .o 文件
            for obj in &obj_files {
                remove_file(obj);
            }
            eprintln!("可执行文件: {}", exe_path.display());
        }
    }
}

fn cmd_run(args: &[String]) {
    let cargs = parse_compile_args(args);
    let target_triple = cargs.target.as_deref();

    let exe_path = cargs.file_path.with_extension("");
    let obj_path = cargs.file_path.with_extension("o");

    // M7: 收集 import 依赖
    let all_files = collect_imports(&cargs.file_path);

    if all_files.len() == 1 {
        // 单文件: 原有简单流程
        let source = read_source(&cargs.file_path);
        match compile_to_stage(
            &source,
            &cargs.file_path.to_string_lossy(),
            target_triple,
            PipelineStage::Object,
            Some(&obj_path),
        ) {
            Ok((stats, _)) => {
                let kangrt_path = find_or_build_kangrt(target_triple);
                link_executable(&obj_path, &kangrt_path, &exe_path, target_triple);
                remove_file(&obj_path);

                if cargs.show_stats {
                    print_stats(&stats, PipelineStage::Object);
                }
                run_exe(&exe_path);
            }
            Err(e) => {
                emit_diagnostic(&e, &source, &cargs.file_path.to_string_lossy());
                process::exit(1);
            }
        }
    } else {
        // 多文件: 编译所有依赖单元
        let obj_files = compile_all_units(&all_files, target_triple, cargs.show_stats);
        let kangrt_path = find_or_build_kangrt(target_triple);
        link_multi(&obj_files, &kangrt_path, &exe_path, target_triple);

        for obj in &obj_files {
            remove_file(obj);
        }

        run_exe(&exe_path);
    }
}

/// 执行生成的可执行文件并清理
///
/// 规范化路径以消除 `..` 和符号链接绕过。拒绝在系统目录中执行，
/// 防止通过 `-o /bin/../home/user/evil` 等方式写入系统路径。
fn run_exe(exe_path: &Path) {
    // 规范化路径以消除 `..` 和符号链接绕过
    let canon = exe_path.canonicalize().unwrap_or_else(|_| exe_path.to_path_buf());
    let exe_parent = canon.parent().unwrap_or(Path::new("."));

    // 拒绝在系统二进制目录中执行（规范化后做前缀匹配）
    let is_system_dir = exe_parent.starts_with("/usr/bin")
        || exe_parent.starts_with("/bin")
        || exe_parent.starts_with("/sbin")
        || exe_parent.starts_with("/usr/sbin")
        || exe_parent.starts_with("/usr/local/bin")
        || exe_parent.starts_with("/opt/homebrew/bin");
    if is_system_dir {
        eprintln!("安全拒绝: 不会在系统目录 ({}) 中执行二进制", exe_parent.display());
        process::exit(1);
    }

    let exit_status = process::Command::new(&canon)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("无法执行程序 {}: {}", canon.display(), e);
            remove_file(&canon);
            process::exit(1);
        });

    remove_file(&canon);

    if !exit_status.success() {
        process::exit(exit_status.code().unwrap_or(1));
    }
}

// ── 共享管线执行 ────────────────────────────────────────────────────────────

/// 执行编译管线到指定阶段，处理输出和统计
fn run_to_stage(cargs: CompileArgs, stage: PipelineStage, check_mode: bool) {
    let source = read_source(&cargs.file_path);

    let object_path = if stage >= PipelineStage::Object {
        cargs.out_path.as_deref()
    } else {
        None
    };

    match compile_to_stage(
        &source,
        &cargs.file_path.to_string_lossy(),
        cargs.target.as_deref(),
        stage,
        object_path,
    ) {
        Ok((stats, output)) => {
            if check_mode {
                println!("OK");
            } else if let Some(text) = output {
                match &cargs.out_path {
                    Some(path) => {
                        std::fs::write(path, &text).unwrap_or_else(|e| {
                            eprintln!("写入文件失败 {}: {}", path.display(), e);
                            process::exit(1);
                        });
                    }
                    None => println!("{}", text),
                }
            }
            if cargs.show_stats {
                print_stats(&stats, stage);
            }
        }
        Err(e) => {
            emit_diagnostic(&e, &source, &cargs.file_path.to_string_lossy());
            process::exit(1);
        }
    }
}

// ── 统计输出 ────────────────────────────────────────────────────────────────

/// 输出编译器统计数据到 stderr (JSON)，按执行阶段截断
fn print_stats(stats: &CompilerStats, stage: PipelineStage) {
    let mut json = serde_json::json!({
        "source": &stats.source,
        "lex": &stats.lex,
    });

    if stage >= PipelineStage::Ast {
        json["parse"] = serde_json::to_value(&stats.parse).unwrap_or_default();
    }
    if stage >= PipelineStage::TypedAst {
        json["semantic"] = serde_json::to_value(&stats.semantic).unwrap_or_default();
    }
    if stage >= PipelineStage::LlvmIr {
        json["codegen"] = serde_json::to_value(&stats.codegen).unwrap_or_default();
    }

    let stats_json = serde_json::to_string_pretty(&json).expect("stats 序列化不应失败");
    eprintln!("{}", stats_json);
}

// ── 链接基础设施 ────────────────────────────────────────────────────────────

/// 查找 workspace 根目录（kangc/Cargo.toml 的父目录）
///
/// 优先使用 KANG_HOME 环境变量，其次使用 CARGO_MANIFEST_DIR 编译期路径。
/// 若二进制被移出 cargo target 目录独立发布，设置 KANG_HOME 即可。

/// 查找或构建 libkangrt.a，返回 .a 文件路径
fn find_or_build_kangrt(target_triple: Option<&str>) -> PathBuf {
    let project_root = kangc::find_project_root();

    let lib_dir = match target_triple {
        Some(t) => project_root.join("target").join(t).join("release"),
        None => project_root.join("target").join("release"),
    };

    let lib_path = lib_dir.join("libkangrt.a");
    if lib_path.exists() {
        return lib_path;
    }

    // 未找到，构建 kangrt
    let display_triple = target_triple.unwrap_or("host");
    eprintln!("正在构建 kangrt (target: {})...", display_triple);
    // 使用 CARGO 环境变量或从 PATH 解析 cargo，避免 PATH 劫持
    let cargo_bin = resolve_cargo();
    let mut cmd = process::Command::new(&cargo_bin);
    cmd.args(["build", "--release", "-p", "kangrt"]);
    if let Some(t) = target_triple {
        cmd.args(["--target", t]);
    }
    cmd.current_dir(&project_root);

    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("无法启动 cargo 构建 kangrt: {}", e);
        process::exit(1);
    });

    if !status.success() {
        eprintln!("kangrt 构建失败");
        process::exit(1);
    }

    lib_path
}

/// 获取并校验链接器路径：返回绝对路径或在不可信时拒绝（CLI 包装，失败时 exit）
fn validate_linker() -> String {
    kangc::find_linker().unwrap_or_else(|msg| {
        eprintln!("安全拒绝: {}", msg);
        process::exit(1);
    })
}

/// 解析 cargo 可执行文件路径：优先 CARGO 环境变量，否则从 PATH 查找
fn resolve_cargo() -> String {
    if let Ok(cargo) = std::env::var("CARGO") {
        let p = PathBuf::from(&cargo);
        if p.is_file() {
            let canon = p.canonicalize().unwrap_or(p);
            return canon.to_string_lossy().to_string();
        }
    }
    match kangc::resolve_from_path("cargo") {
        Some(p) => p.to_string_lossy().to_string(),
        None => {
            eprintln!("无法找到 cargo 构建工具。请设置 CARGO 环境变量或确保 cargo 在 PATH 中");
            process::exit(1);
        }
    }
}

/// 使用系统 cc 链接 .o 文件 + libkangrt.a → 可执行文件
///
/// 当 target_triple 设置时，追加 `-target <triple>` 以支持交叉编译（需要 clang）。
///
/// CC 环境变量的值必须指向可信目录中的链接器，防止任意代码执行。
fn link_executable(obj_path: &Path, kangrt_path: &Path, out_path: &Path, target_triple: Option<&str>) {
    let linker = validate_linker();
    let mut cmd = process::Command::new(&linker);
    cmd.arg(obj_path).arg(kangrt_path).arg("-o").arg(out_path);
    if let Some(t) = target_triple {
        cmd.arg("-target").arg(t);
    }
    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("无法启动链接器 {}: {}", linker, e);
        process::exit(1);
    });

    if !status.success() {
        eprintln!("链接失败");
        process::exit(1);
    }
}

// ── 多文件编译 (M7) ────────────────────────────────────────────────────────────

/// 收集入口文件的 import 依赖（迭代 BFS，避免深度导入链栈溢出），返回编译顺序的文件列表
fn collect_imports(entry: &Path) -> Vec<PathBuf> {
    use std::collections::VecDeque;

    let mut result: Vec<PathBuf> = Vec::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();

    let canon = entry.canonicalize().unwrap_or_else(|_| entry.to_path_buf());
    visited.insert(canon.clone());
    result.push(canon.clone());
    queue.push_back(canon);

    while let Some(current) = queue.pop_front() {
        let source = match std::fs::read_to_string(&current) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let mut lex_stats = kangc::stats::LexStats::default();
        let tokens = match kangc::lexer::tokenize(&source, &mut lex_stats) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let mut parse_stats = kangc::stats::ParseStats::default();
        let program = match kangc::parser::parse(&tokens, &mut parse_stats) {
            Ok(p) => p,
            Err(_) => continue,
        };

        let base_dir = current.parent().unwrap_or(Path::new("."));

        for item in &program.items {
            if let kangc::ast::TopLevel::Import(imp) = item {
                let dep_path = base_dir.join(&imp.path);
                if !dep_path.exists() {
                    eprintln!("警告: 导入文件不存在: {} (来自 {})", imp.path, current.display());
                    continue;
                }
                let dep_canon = dep_path.canonicalize().unwrap_or_else(|_| dep_path.clone());
                if visited.insert(dep_canon.clone()) {
                    result.push(dep_canon.clone());
                    queue.push_back(dep_canon);
                }
            }
        }
    }

    result
}

/// 编译所有源文件到 .o 文件，返回 .o 文件路径列表
fn compile_all_units(
    files: &[PathBuf],
    target_triple: Option<&str>,
    show_stats: bool,
) -> Vec<PathBuf> {
    let mut obj_files = Vec::new();

    for file in files {
        let obj_path = file.with_extension("o");
        let source = read_source(file);

        match compile_to_stage(
            &source,
            &file.to_string_lossy(),
            target_triple,
            PipelineStage::Object,
            Some(&obj_path),
        ) {
            Ok((stats, _)) => {
                obj_files.push(obj_path.clone());
                if show_stats {
                    eprintln!("[{}] 编译完成", file.display());
                    print_stats(&stats, PipelineStage::Object);
                }
            }
            Err(e) => {
                // 清理已生成的 .o 文件，避免残留中间产物
                for obj in &obj_files {
                    let _ = std::fs::remove_file(obj);
                }
                emit_diagnostic(&e, &source, &file.to_string_lossy());
                process::exit(1);
            }
        }
    }

    obj_files
}

/// 链接多个 .o 文件 + libkangrt.a → 可执行文件
fn link_multi(obj_files: &[PathBuf], kangrt_path: &Path, out_path: &Path, target_triple: Option<&str>) {
    let linker = validate_linker();
    let mut cmd = process::Command::new(&linker);
    for obj in obj_files {
        cmd.arg(obj);
    }
    cmd.arg(kangrt_path).arg("-o").arg(out_path);
    if let Some(t) = target_triple {
        cmd.arg("-target").arg(t);
    }

    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("无法启动链接器 {}: {}", linker, e);
        process::exit(1);
    });

    if !status.success() {
        eprintln!("链接失败");
        process::exit(1);
    }
}

// ── 公共辅助 ────────────────────────────────────────────────────────────────

fn read_file(path: &PathBuf) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("读取文件失败 {}: {}", path.display(), e))
}

fn read_source(path: &PathBuf) -> String {
    read_file(path).unwrap_or_else(|msg| {
        eprintln!("{}", msg);
        process::exit(1);
    })
}

fn remove_file(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        eprintln!("警告: 无法删除临时文件 {}: {}", path.display(), e);
    }
}
