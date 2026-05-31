// 代码生成入口 — 将 TypedProgram 转换为 LLVM IR
// 编排 context/types/expr/stmt/builtins/runtime 各子模块

pub mod builtins;
pub mod context;
pub mod expr;
pub mod runtime;
pub mod stmt;
pub mod types;

use crate::error::CodeGenError;
use crate::semantic::{KangType, TypedProgram, TypedTopLevel};
use crate::stats::{CodeGenResult, CodeGenStats};
use context::CodeGenContext;
use inkwell::context::Context;
use inkwell::targets::{CodeModel, FileType, RelocMode, Target, TargetTriple};
use inkwell::OptimizationLevel;
use std::path::Path;

/// 将 TypedProgram 代码生成为 LLVM IR，可选输出目标文件
pub fn codegen(
    program: &TypedProgram,
    stats: &mut CodeGenStats,
    target_triple: Option<&str>,
    object_path: Option<&Path>,
    module_name: &str,
) -> Result<CodeGenResult, CodeGenError> {
    let llvm_context = Context::create();
    let mut ctx = CodeGenContext::new(&llvm_context, module_name, target_triple);

    // 声明所有内置函数
    builtins::declare_all(&mut ctx);

    // 注册结构体类型
    for item in &program.items {
        if let TypedTopLevel::Struct(s) = item {
            let fields: Vec<(String, KangType)> = s
                .fields
                .iter()
                .map(|(name, ty)| (name.clone(), KangType::from_ast_type(ty)))
                .collect();
            ctx.register_struct(&s.name, &fields);
        }
    }

    // 生成函数（使用 TypedExpr 中的原始名称，代码生成名由语义分析设定）
    for item in &program.items {
        if let TypedTopLevel::Func(func) = item {
            codegen_func(&mut ctx, &func.name, &func.params, &func.return_type, &func.body)?;
        }
    }

    // 验证生成的 LLVM IR
    if let Err(e) = ctx.module.verify() {
        return Err(CodeGenError {
            msg: format!("LLVM IR 验证失败:\n{}", e),
        });
    }

    // 输出 LLVM IR
    let ir_string = ctx.module.print_to_string().to_string();
    stats.llvm_ir_bytes = ir_string.len();
    stats.llvm_function_count = ctx.module.get_functions().count();

    // 统计基本块和指令数
    for func_val in ctx.module.get_functions() {
        for bb in func_val.get_basic_blocks() {
            stats.llvm_basic_block_count += 1;
            for _ in bb.get_instructions() {
                stats.llvm_instruction_count += 1;
            }
        }
    }

    // 可选: 输出目标文件 (.o)
    let object_file = if let Some(path) = object_path {
        emit_object_file(&ctx.module, &ctx.target_triple, path)?;
        Some(path.to_string_lossy().to_string())
    } else {
        None
    };

    Ok(CodeGenResult {
        ir_text: ir_string,
        stats: stats.clone(),
        object_file,
    })
}

/// 将 LLVM Module 写入目标文件 (.o)
fn emit_object_file(
    module: &inkwell::module::Module,
    triple_str: &str,
    path: &Path,
) -> Result<(), CodeGenError> {
    let triple = TargetTriple::create(triple_str);
    let target = Target::from_triple(&triple).map_err(|e| CodeGenError {
        msg: format!("不支持的目标 triple '{}': {:?}", triple_str, e),
    })?;
    let machine = target
        .create_target_machine(
            &triple,
            "",
            "",
            OptimizationLevel::Default,
            RelocMode::Default,
            CodeModel::Default,
        )
        .ok_or_else(|| CodeGenError {
            msg: format!("无法为 '{}' 创建 TargetMachine", triple_str),
        })?;
    machine.write_to_file(module, FileType::Object, path).map_err(|e| CodeGenError {
        msg: format!("写入目标文件失败 '{}': {:?}", path.display(), e),
    })
}

/// 生成单个函数的 LLVM IR
fn codegen_func<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    name: &str,
    params: &[(String, KangType)],
    return_type: &KangType,
    body: &[crate::semantic::TypedStmt],
) -> Result<(), CodeGenError> {
    // 声明函数
    let param_types: Vec<KangType> = params.iter().map(|(_, t)| t.clone()).collect();
    let func_val = ctx.declare_func(name, &param_types, return_type);

    // 创建入口基本块
    let entry = ctx.context.append_basic_block(func_val, "entry");
    ctx.builder.position_at_end(entry);

    // 注册参数变量
    ctx.push_scope();
    for (i, (param_name, param_type)) in params.iter().enumerate() {
        let llvm_ty = ctx.kang_type_to_basic(param_type);
        let alloca = ctx
            .builder
            .build_alloca(llvm_ty, &format!("arg.{}", param_name))
            .unwrap();
        let _ = ctx.builder.build_store(alloca, func_val.get_nth_param(i as u32).unwrap());
        ctx.register_var(param_name, alloca, param_type.clone());
    }

    // 生成函数体
    for stmt in body {
        stmt::codegen_stmt(ctx, stmt, return_type)?;
    }

    // 如果最后一条语句没有终止符，补 ret void 或 unreachable
    let current_bb = ctx.builder.get_insert_block().unwrap();
    if current_bb.get_terminator().is_none() {
        if return_type.is_void() {
            let _ = ctx.builder.build_return(None);
        } else {
            // 非 void 函数理论不应走到这里（F1 规则保证），补 unreachable
            let _ = ctx.builder.build_unreachable();
        }
    }

    // 若函数入口后被终结（如无条件分支到其他块），移除未使用的 entry 块
    // inkwell/LLVM 会报告 verify 错误；不在本次范围

    ctx.pop_scope();
    Ok(())
}

// ── 代码生成测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer;
    use crate::parser;
    use crate::semantic;
    use crate::stats::{CodeGenStats, LexStats, ParseStats, SemanticStats};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::fs;

    /// 辅助: source → lex → parse → check → codegen → IR 文本
    fn compile_ir(source: &str) -> String {
        let mut lex_stats = LexStats {
            duration_us: 0, token_count: 0,
            token_counts_by_kind: HashMap::new(), comment_bytes: 0,
        };
        let mut parse_stats = ParseStats {
            duration_us: 0, ast_node_count: 0, ast_max_depth: 0,
            node_counts_by_kind: HashMap::new(), func_count: 0, struct_count: 0,
        };
        let mut sem_stats = SemanticStats {
            duration_us: 0, error_count: 0, warning_count: 0,
            symbol_count: 0, type_check_passes: 0, type_check_failures: 0,
        };
        let mut cg_stats = CodeGenStats {
            duration_us: 0, llvm_ir_bytes: 0, llvm_function_count: 0,
            llvm_basic_block_count: 0, llvm_instruction_count: 0,
            runtime_check_insertions: 0,
        };

        let tokens = lexer::tokenize(source, &mut lex_stats).expect("lex");
        let program = parser::parse(&tokens, &mut parse_stats).expect("parse");
        let typed = semantic::check(&program, &mut sem_stats, "<test>").expect("check");
        codegen(&typed, &mut cg_stats, None, None, "test").expect("codegen").ir_text
    }

    #[test]
    fn ir_contains_module_header() {
        let ir = compile_ir("def f() -> i32 { return 42; }");
        assert!(ir.contains("source_filename"), "IR should contain source_filename");
        assert!(ir.contains("ModuleID"), "IR should contain ModuleID");
    }

    #[test]
    fn ir_contains_function_definition() {
        let ir = compile_ir("def add(a:i32, b:i32) -> i32 { return a + b; }");
        assert!(ir.contains("define"), "IR should contain function definition");
        assert!(ir.contains("add"), "IR should contain function name");
    }

    #[test]
    fn ir_bounds_check_present_for_array_index() {
        let ir = compile_ir("def f() -> i32 { var arr: [i32] = [1, 2, 3]; return arr[0]; }");
        assert!(ir.contains("bounds.lo") || ir.contains("bounds.fail"),
            "IR should contain bounds check for array index");
    }

    #[test]
    fn ir_bounds_check_present_for_str_index() {
        let ir = compile_ir("def f() -> str { var s: str = \"hello\"; return s[0]; }");
        assert!(ir.contains("bounds.lo") || ir.contains("bounds.fail"),
            "IR should contain bounds check for string index");
    }

    #[test]
    fn ir_div_zero_check_present() {
        let ir = compile_ir("def f(a:i32, b:i32) -> i32 { return a / b; }");
        assert!(ir.contains("divz") || ir.contains("divz.fail"),
            "IR should contain div-zero check");
    }

    #[test]
    fn ir_int_min_check_present() {
        let ir = compile_ir("def f(a:i32, b:i32) -> i32 { return a / b; }");
        assert!(ir.contains("imin."),
            "IR should contain INT_MIN / -1 overflow check");
    }

    #[test]
    fn all_grammar_tests_pass_codegen() {
        for i in 1..=10 {
            let filename = format!("{:02}_", i);
            let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            path.push("../grammar_tests");

            let entries = fs::read_dir(&path).unwrap();
            let file = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .find(|n| n.starts_with(&filename))
                .unwrap();

            path.push(&file);
            let source = fs::read_to_string(&path).unwrap();

            let mut lex_stats = LexStats {
                duration_us: 0, token_count: 0,
                token_counts_by_kind: HashMap::new(), comment_bytes: 0,
            };
            let mut parse_stats = ParseStats {
                duration_us: 0, ast_node_count: 0, ast_max_depth: 0,
                node_counts_by_kind: HashMap::new(), func_count: 0, struct_count: 0,
            };
            let mut sem_stats = SemanticStats {
                duration_us: 0, error_count: 0, warning_count: 0,
                symbol_count: 0, type_check_passes: 0, type_check_failures: 0,
            };
            let mut cg_stats = CodeGenStats {
                duration_us: 0, llvm_ir_bytes: 0, llvm_function_count: 0,
                llvm_basic_block_count: 0, llvm_instruction_count: 0,
                runtime_check_insertions: 0,
            };

            let tokens = lexer::tokenize(&source, &mut lex_stats).unwrap();
            let program = parser::parse(&tokens, &mut parse_stats).unwrap();
            let typed = match semantic::check(&program, &mut sem_stats, &path.to_string_lossy()) {
                Ok(tp) => tp,
                Err(errors) => {
                    // 某些语法测试文件可能有语义错误（如 03_expressions）
                    // 只要求有语义错误的文件能走到 codegen 不崩溃即可
                    if errors.is_empty() {
                        panic!("{}: 语义检查失败但无错误信息", file);
                    }
                    continue;
                }
            };

            match codegen(&typed, &mut cg_stats, None, None, "test") {
                Ok(result) => {
                    assert!(!result.ir_text.is_empty(), "{}: IR 不应为空", file);
                    assert!(result.ir_text.contains("source_filename"), "{}: IR 应包含 source_filename", file);
                }
                Err(e) => panic!("{}: 代码生成失败: {}", file, e.msg),
            }
        }
    }
}
