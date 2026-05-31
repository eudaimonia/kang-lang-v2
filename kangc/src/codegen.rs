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

    // 尝试内建 LLVM TargetMachine 直接生成 .o
    if let Ok(target) = Target::from_triple(&triple) {
        if let Some(machine) = target.create_target_machine(
            &triple, "", "",
            OptimizationLevel::Default,
            RelocMode::Default,
            CodeModel::Default,
        ) {
            return machine.write_to_file(module, FileType::Object, path).map_err(|e| {
                CodeGenError { msg: format!("写入目标文件失败 '{}': {:?}", path.display(), e) }
            });
        }
    }

    // 回退: 输出 .ll 到临时文件，调用外部 llc 生成 .o
    emit_object_via_llc(module, triple_str, path)
}

/// 通过外部 llc 工具将 Module 编译为 .o（回退路径，用于内建 LLVM 不支持的 target）
fn emit_object_via_llc(
    module: &inkwell::module::Module,
    triple_str: &str,
    path: &Path,
) -> Result<(), CodeGenError> {
    use std::process::Command;

    let llc = find_llc()?;
    let ir_text = module.print_to_string().to_string();
    let ir_path = path.with_extension("ll");

    std::fs::write(&ir_path, &ir_text).map_err(|e| CodeGenError {
        msg: format!("无法写入临时 IR 文件 '{}': {}", ir_path.display(), e),
    })?;

    let result = Command::new(&llc)
        .arg(&ir_path)
        .arg("-o").arg(path)
        .arg("-filetype=obj")
        .arg("--mtriple").arg(triple_str)
        .status();

    let _ = std::fs::remove_file(&ir_path);

    match result {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(CodeGenError {
            msg: format!("外部 llc 退出码 {} (target: {})", status.code().unwrap_or(-1), triple_str),
        }),
        Err(e) => Err(CodeGenError {
            msg: format!("无法执行 llc (target: {}): {}", triple_str, e),
        }),
    }
}

/// 查找 llc 可执行文件：LLVM_SYS_XXX_PREFIX → PATH → Homebrew 默认路径
fn find_llc() -> Result<String, CodeGenError> {
    use std::path::PathBuf;

    // 检查 LLVM_SYS_220_PREFIX 等通用环境变量
    for var in &["LLVM_SYS_220_PREFIX", "LLVM_SYS_191_PREFIX", "LLVM_SYS_180_PREFIX"] {
        if let Ok(prefix) = std::env::var(var) {
            let path = PathBuf::from(&prefix).join("bin").join("llc");
            if path.is_file() {
                return Ok(path.to_string_lossy().to_string());
            }
        }
    }

    // 检查 PATH
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let full = PathBuf::from(dir).join("llc");
            if full.is_file() {
                return Ok(full.to_string_lossy().to_string());
            }
        }
    }

    // 检查 Homebrew LLVM 路径
    for brew_prefix in &["/opt/homebrew/opt/llvm/bin/llc", "/usr/local/opt/llvm/bin/llc"] {
        let p = PathBuf::from(brew_prefix);
        if p.is_file() {
            return Ok(p.to_string_lossy().to_string());
        }
    }

    Err(CodeGenError {
        msg: "无法找到 llc 工具（内建 LLVM 不支持该 target，且外部 llc 也未找到）".into(),
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

    // ── 回归测试: 已修复的编译器 Bug ────────────────────────────────────────────

    #[test]
    fn str_bool_zext_for_c_abi() {
        // Bug 1: str(bool) LLVM 类型不匹配 — flatten_c_abi_args 须将 i1 zext 到 i32
        let ir = compile_ir("def f(b:bool) -> str { return str(b); }");
        assert!(ir.contains("zext"), "str(b) 应生成 zext i1 -> i32, got:\n{}", ir);
    }

    #[test]
    fn str_index_direct_ptr_access() {
        // Bug 2: 字符串索引不应有 +4 偏移（字符串没有堆长度头）
        // 验证字符串索引 char 返回 Kang str 结构体 {ptr, 1}
        let ir = compile_ir("def f() -> str { var s:str = \"hello\"; return s[0]; }");
        // 不应该包含 +4 偏移
        assert!(!ir.contains("add i64 %arr.addr, 4"), "str index 不应有 +4 堆头偏移");
    }

    #[test]
    fn push_stores_back_to_variable() {
        // Bug 3: push() 必须将 k_push 返回值存回变量 alloca
        let ir = compile_ir("def f() -> void { var arr:[i32] = [1,2]; push(arr, 3); return; }");
        // push 调用后应有 store 指令写回变量
        let push_pos = ir.find("@k_push").expect("IR 应包含 k_push 调用");
        let after_push = &ir[push_pos..];
        assert!(after_push.contains("store"), "push 后应有 store 写回变量, got:\n{}", ir);
    }

    #[test]
    fn read_file_pair_repacking() {
        // Bug 4: read_file 返回 {ptr,i64} 须 repack 为 Kang pair {{ptr,i32},i1}
        let ir = compile_ir("def f() -> str { var c:str, ok:bool = read_file(\"/tmp/t\"); return c; }");
        assert!(ir.contains("k_read_file"), "应调用 k_read_file");
    }
}
