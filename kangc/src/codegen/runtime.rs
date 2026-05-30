// 运行时安全检查 — 插入索引越界、除零等运行时检查

use super::context::CodeGenContext;
use inkwell::values::IntValue;

/// 插入数组索引越界检查: 0 <= index < len
/// 检查失败时调用 @llvm.trap 中止程序
pub fn insert_bounds_check<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    index: IntValue<'ctx>,
    len: IntValue<'ctx>,
) {
    ctx.runtime_checks += 1;

    let current_fn = ctx.builder.get_insert_block().unwrap().get_parent().unwrap();

    // 检查 index >= 0
    let zero = ctx.context.i32_type().const_int(0, true);
    let is_neg = ctx.builder.build_int_compare(
        inkwell::IntPredicate::SLT,
        index,
        zero,
        "bounds.lo",
    ).unwrap();

    // 检查 index >= len
    let is_oob = ctx.builder.build_int_compare(
        inkwell::IntPredicate::SGE,
        index,
        len,
        "bounds.hi",
    ).unwrap();

    // 合并条件: index < 0 || index >= len
    let is_fail = ctx.builder.build_or(is_neg, is_oob, "bounds.fail").unwrap();

    let fail_bb = ctx.context.append_basic_block(current_fn, "bounds.fail");
    let ok_bb = ctx.context.append_basic_block(current_fn, "bounds.ok");

    let _ = ctx.builder.build_conditional_branch(is_fail, fail_bb, ok_bb);

    // 失败路径: trap
    ctx.builder.position_at_end(fail_bb);
    let _ = ctx.builder.build_call(
        ctx.module.get_function("llvm.trap").unwrap_or_else(|| {
            let trap_type = ctx.context.void_type().fn_type(&[], false);
            ctx.module.add_function("llvm.trap", trap_type, None)
        }),
        &[],
        "trap",
    );
    let _ = ctx.builder.build_unreachable();

    // 正常路径
    ctx.builder.position_at_end(ok_bb);
}

/// 插入除零检查: divisor != 0
pub fn insert_div_zero_check<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    divisor: IntValue<'ctx>,
) {
    ctx.runtime_checks += 1;

    let current_fn = ctx.builder.get_insert_block().unwrap().get_parent().unwrap();

    let zero = ctx.context.i32_type().const_int(0, true);
    let is_zero = ctx.builder.build_int_compare(
        inkwell::IntPredicate::EQ,
        divisor,
        zero,
        "divz",
    ).unwrap();

    let fail_bb = ctx.context.append_basic_block(current_fn, "divz.fail");
    let ok_bb = ctx.context.append_basic_block(current_fn, "divz.ok");

    let _ = ctx.builder.build_conditional_branch(is_zero, fail_bb, ok_bb);

    // 失败路径: trap
    ctx.builder.position_at_end(fail_bb);
    let _ = ctx.builder.build_call(
        ctx.module.get_function("llvm.trap").unwrap_or_else(|| {
            let trap_type = ctx.context.void_type().fn_type(&[], false);
            ctx.module.add_function("llvm.trap", trap_type, None)
        }),
        &[],
        "trap",
    );
    let _ = ctx.builder.build_unreachable();

    // 正常路径
    ctx.builder.position_at_end(ok_bb);
}

/// 插入 INT_MIN / -1 溢出检查 (R4)
/// i32::MIN / -1 会溢出，因为 i32 范围是 [-2147483648, 2147483647]
pub fn insert_int_min_check<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    dividend: IntValue<'ctx>,
    divisor: IntValue<'ctx>,
) {
    ctx.runtime_checks += 1;

    let current_fn = ctx.builder.get_insert_block().unwrap().get_parent().unwrap();

    let int_min = ctx.context.i32_type().const_int(i32::MIN as u64, true);
    let neg_one = ctx.context.i32_type().const_int((-1i32) as u64, true);

    let is_int_min = ctx.builder.build_int_compare(
        inkwell::IntPredicate::EQ,
        dividend,
        int_min,
        "imin.dividend",
    ).unwrap();

    let is_neg_one = ctx.builder.build_int_compare(
        inkwell::IntPredicate::EQ,
        divisor,
        neg_one,
        "imin.divisor",
    ).unwrap();

    let is_overflow = ctx.builder.build_and(is_int_min, is_neg_one, "imin.overflow").unwrap();

    let fail_bb = ctx.context.append_basic_block(current_fn, "imin.fail");
    let ok_bb = ctx.context.append_basic_block(current_fn, "imin.ok");

    let _ = ctx.builder.build_conditional_branch(is_overflow, fail_bb, ok_bb);

    // 失败路径: trap
    ctx.builder.position_at_end(fail_bb);
    let _ = ctx.builder.build_call(
        ctx.module.get_function("llvm.trap").unwrap_or_else(|| {
            let trap_type = ctx.context.void_type().fn_type(&[], false);
            ctx.module.add_function("llvm.trap", trap_type, None)
        }),
        &[],
        "trap",
    );
    let _ = ctx.builder.build_unreachable();

    // 正常路径
    ctx.builder.position_at_end(ok_bb);
}
