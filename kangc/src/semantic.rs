// 语义分析入口 — 编排类型检查、作用域分析、控制流分析
// 将 AST 转为 TypedProgram，每个表达式节点携带解析后的类型

pub mod checker;
pub mod flow;
pub mod scope;
pub mod types;

use crate::ast;
use crate::error::SemanticError;
use crate::stats::SemanticStats;
use checker::Checker;
use std::time::Instant;

pub use types::*;

/// 对 AST 进行语义检查，返回类型标注的 TypedProgram
/// 收集所有错误后一并返回（不因单条错误中断）
/// - program: parser 产出的 AST
/// - stats: 写入耗时、错误数、符号数等统计
/// - file_path: 用于错误消息中的文件引用
pub fn check(program: &ast::Program, stats: &mut SemanticStats, file_path: &str) -> Result<TypedProgram, Vec<SemanticError>> {
    let start = Instant::now();
    let mut checker = Checker::new(Some(file_path));

    let result = checker.check_program(program);

    stats.duration_us = start.elapsed().as_micros() as u64;
    stats.error_count = checker.failures();
    stats.warning_count = 0;
    stats.symbol_count = checker.symbol_count();
    stats.type_check_passes = checker.passes();
    stats.type_check_failures = checker.failures();

    result
}

// ── 语义测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer;
    use crate::parser;
    use crate::stats::LexStats;
    use crate::stats::ParseStats;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    /// 辅助: 读测试文件 → lex → parse → check → 返回错误数量
    fn check_file(filename: &str) -> (usize, Vec<SemanticError>) {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("../semantic_tests");
        path.push(filename);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("无法读取测试文件 {}: {}", path.display(), e));

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

        let tokens = lexer::tokenize(&source, &mut lex_stats).expect("词法分析应成功");
        let program = parser::parse(&tokens, &mut parse_stats).expect("语法分析应成功");

        let mut semantic_stats = SemanticStats {
            duration_us: 0,
            error_count: 0,
            warning_count: 0,
            symbol_count: 0,
            type_check_passes: 0,
            type_check_failures: 0,
        };

        match check(&program, &mut semantic_stats, &path.to_string_lossy()) {
            Ok(_) => (0, vec![]),
            Err(errors) => (errors.len(), errors),
        }
    }

    /// 统计测试文件中 // ERROR: 注释标记的数量
    /// 用于验证语义检查器产生了预期的错误数量
    fn count_error_markers(filename: &str) -> usize {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("../semantic_tests");
        path.push(filename);
        let source = fs::read_to_string(&path).unwrap();
        source.lines().filter(|l| l.contains("// ERROR:")).count()
    }

    // ── 01_type_errors.kang ─────────────────────────────────────────────────

    #[test]
    fn test_01_type_errors() {
        let (count, _) = check_file("01_type_errors.kang");
        let expected = count_error_markers("01_type_errors.kang");
        assert!(
            count >= expected,
            "01_type_errors: 至少应有 {} 个错误，实际 {} 个",
            expected,
            count
        );
    }

    // ── 02_scope_errors.kang ─────────────────────────────────────────────────

    #[test]
    fn test_02_scope_errors() {
        let (count, _) = check_file("02_scope_errors.kang");
        let expected = count_error_markers("02_scope_errors.kang");
        assert!(
            count >= expected,
            "02_scope_errors: 至少应有 {} 个错误，实际 {} 个",
            expected,
            count
        );
    }

    // ── 03_multi_return_errors.kang ─────────────────────────────────────────

    #[test]
    fn test_03_multi_return_errors() {
        let (count, _) = check_file("03_multi_return_errors.kang");
        let expected = count_error_markers("03_multi_return_errors.kang");
        assert!(
            count >= expected,
            "03_multi_return: 至少应有 {} 个错误，实际 {} 个",
            expected,
            count
        );
    }

    // ── 04_struct_errors.kang ───────────────────────────────────────────────

    #[test]
    fn test_04_struct_errors() {
        let (count, _) = check_file("04_struct_errors.kang");
        let expected = count_error_markers("04_struct_errors.kang");
        assert!(
            count >= expected,
            "04_struct: 至少应有 {} 个错误，实际 {} 个",
            expected,
            count
        );
    }

    // ── 05_func_errors.kang ─────────────────────────────────────────────────

    #[test]
    fn test_05_func_errors() {
        let (count, _) = check_file("05_func_errors.kang");
        let expected = count_error_markers("05_func_errors.kang");
        assert!(
            count >= expected,
            "05_func: 至少应有 {} 个错误，实际 {} 个",
            expected,
            count
        );
    }

    // ── 06_array_errors.kang ────────────────────────────────────────────────

    #[test]
    fn test_06_array_errors() {
        let (count, _) = check_file("06_array_errors.kang");
        let expected = count_error_markers("06_array_errors.kang");
        assert!(
            count >= expected,
            "06_array: 至少应有 {} 个错误，实际 {} 个",
            expected,
            count
        );
    }

    // ── 07_assign_errors.kang ───────────────────────────────────────────────

    #[test]
    fn test_07_assign_errors() {
        let (count, _) = check_file("07_assign_errors.kang");
        let expected = count_error_markers("07_assign_errors.kang");
        assert!(
            count >= expected,
            "07_assign: 至少应有 {} 个错误，实际 {} 个",
            expected,
            count
        );
    }

    // ── 10 个语法正向测试全部应通过语义检查 ────────────────────────────────

    #[test]
    fn test_grammar_files_pass_semantic() {
        for i in 1..=10 {
            let filename = format!("{:02}_", i);
            let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            path.push("../grammar_tests");

            // 找到对应编号的文件
            let entries = fs::read_dir(&path).unwrap();
            let file = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .find(|n| n.starts_with(&filename))
                .unwrap();

            path.push(&file);
            let source = fs::read_to_string(&path).unwrap();

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

            let tokens = lexer::tokenize(&source, &mut lex_stats).unwrap();
            let program = parser::parse(&tokens, &mut parse_stats).unwrap();

            let mut semantic_stats = SemanticStats {
                duration_us: 0,
                error_count: 0,
                warning_count: 0,
                symbol_count: 0,
                type_check_passes: 0,
                type_check_failures: 0,
            };

            match check(&program, &mut semantic_stats, &path.to_string_lossy()) {
                Ok(_) => {} // 通过
                Err(errors) => {
                    panic!(
                        "{} 应有 0 个错误，实际 {} 个:\n{}",
                        file,
                        errors.len(),
                        errors
                            .iter()
                            .map(|e| e.msg.clone())
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                }
            }
        }
    }
}
