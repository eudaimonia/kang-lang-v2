// kangc CLI — Kang 编译器命令行入口
// M1: kang lex / kang parse 子命令, 支持 --stats / -o

use kangc::error::emit_diagnostic;
use kangc::lexer::format_tokens;
use kangc::stats::{LexStats, ParseStats, SourceStats};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: kang <子命令> [参数]");
        eprintln!();
        eprintln!("子命令:");
        eprintln!("  lex       <file> [-o <file>] [--stats]    词法分析, 输出 Token Stream");
        eprintln!("  parse     <file> [-o <file>] [--stats]    语法分析, 输出 AST");
        process::exit(1);
    }

    let subcmd = &args[1];
    let rest = &args[2..];

    match subcmd.as_str() {
        "lex" => cmd_lex(rest),
        "parse" => cmd_parse(rest),
        _ => {
            eprintln!("未知子命令: {}", subcmd);
            process::exit(1);
        }
    }
}

// ── kang lex ────────────────────────────────────────────────────────────────

fn cmd_lex(args: &[String]) {
    let (file_path, out_path, show_stats) = parse_common_args(args);

    let source = read_file(&file_path);
    let total_lines = source.lines().count();
    let mut stats = LexStats {
        duration_us: 0,
        token_count: 0,
        token_counts_by_kind: HashMap::new(),
        comment_bytes: 0,
    };

    let tokens = match kangc::lexer::tokenize(&source, &mut stats) {
        Ok(t) => t,
        Err(e) => {
            emit_diagnostic(
                &kangc::error::KangError::Lex(e),
                &source,
                &file_path.to_string_lossy(),
            );
            process::exit(1);
        }
    };

    let output = format_tokens(&tokens);

    match &out_path {
        Some(path) => {
            std::fs::write(path, &output).unwrap_or_else(|e| {
                eprintln!("写入文件失败 {}: {}", path.display(), e);
                process::exit(1);
            });
        }
        None => {
            println!("{}", output);
        }
    }

    if show_stats {
        let source_stats = SourceStats {
            file_path: file_path.to_string_lossy().to_string(),
            total_bytes: source.len(),
            total_lines,
        };
        let stats_json = serde_json::to_string_pretty(&serde_json::json!({
            "source": source_stats,
            "lex": stats,
        }))
        .unwrap();
        eprintln!("{}", stats_json);
    }
}

// ── kang parse ──────────────────────────────────────────────────────────────

fn cmd_parse(args: &[String]) {
    let (file_path, out_path, show_stats) = parse_common_args(args);

    let source = read_file(&file_path);
    let total_lines = source.lines().count();

    let mut lex_stats = LexStats {
        duration_us: 0,
        token_count: 0,
        token_counts_by_kind: HashMap::new(),
        comment_bytes: 0,
    };
    let mut parse_stats = ParseStats {
        duration_us: 0,
        ast_node_count: 0,
        ast_max_depth: 0,
        node_counts_by_kind: HashMap::new(),
        func_count: 0,
        struct_count: 0,
    };

    let tokens = match kangc::lexer::tokenize(&source, &mut lex_stats) {
        Ok(t) => t,
        Err(e) => {
            emit_diagnostic(
                &kangc::error::KangError::Lex(e),
                &source,
                &file_path.to_string_lossy(),
            );
            process::exit(1);
        }
    };

    let program = match kangc::parser::parse(&tokens, &mut parse_stats) {
        Ok(p) => p,
        Err(e) => {
            emit_diagnostic(
                &kangc::error::KangError::Parse(e),
                &source,
                &file_path.to_string_lossy(),
            );
            process::exit(1);
        }
    };

    let output = format!("{}", program);

    match &out_path {
        Some(path) => {
            std::fs::write(path, &output).unwrap_or_else(|e| {
                eprintln!("写入文件失败 {}: {}", path.display(), e);
                process::exit(1);
            });
        }
        None => {
            println!("{}", output);
        }
    }

    if show_stats {
        let source_stats = SourceStats {
            file_path: file_path.to_string_lossy().to_string(),
            total_bytes: source.len(),
            total_lines,
        };
        let stats_json = serde_json::to_string_pretty(&serde_json::json!({
            "source": source_stats,
            "lex": lex_stats,
            "parse": parse_stats,
        }))
        .unwrap();
        eprintln!("{}", stats_json);
    }
}

// ── 公共辅助 ────────────────────────────────────────────────────────────────

fn parse_common_args(args: &[String]) -> (PathBuf, Option<PathBuf>, bool) {
    let mut file_path = None;
    let mut out_path = None;
    let mut show_stats = false;

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

    (file_path, out_path, show_stats)
}

fn read_file(path: &PathBuf) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("读取文件失败 {}: {}", path.display(), e);
        process::exit(1);
    })
}
