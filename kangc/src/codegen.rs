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
use crate::stats::CodeGenStats;
use context::CodeGenContext;
use inkwell::context::Context;

/// 将 TypedProgram 代码生成为 LLVM IR 文本
pub fn codegen(program: &TypedProgram, stats: &mut CodeGenStats) -> Result<String, CodeGenError> {
    let llvm_context = Context::create();
    let mut ctx = CodeGenContext::new(&llvm_context, "kang_module");

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

    // 生成函数
    for item in &program.items {
        if let TypedTopLevel::Func(func) = item {
            codegen_func(&mut ctx, &func.name, &func.params, &func.return_type, &func.body)?;
        }
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

    Ok(ir_string)
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
