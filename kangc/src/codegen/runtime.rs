// 运行时安全检查 — 插入索引越界、除零等运行时检查

use super::context::CodeGenContext;
use inkwell::values::{FloatValue, IntValue};
use inkwell::AddressSpace;

/// 获取或创建全局错误消息字符串，返回 (ptr, len)
fn panic_msg_global<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    name: &str,
    msg: &[u8],
) -> (inkwell::values::PointerValue<'ctx>, IntValue<'ctx>) {
    if let Some(g) = ctx.module.get_global(name) {
        let ptr = ctx.builder.build_bit_cast(
            g.as_pointer_value(),
            ctx.context.ptr_type(AddressSpace::default()),
            &format!("{}.ptr", name),
        ).unwrap().into_pointer_value();
        let len = ctx.context.i32_type().const_int(msg.len() as u64, true);
        return (ptr, len);
    }

    let array_type = ctx.context.i8_type().array_type(msg.len() as u32);
    let global = ctx.module.add_global(array_type, None, name);
    global.set_linkage(inkwell::module::Linkage::Private);
    let bytes: Vec<IntValue> = msg.iter().map(|&b| ctx.context.i8_type().const_int(b as u64, true)).collect();
    global.set_initializer(&ctx.context.i8_type().const_array(&bytes));

    let ptr = ctx.builder.build_bit_cast(
        global.as_pointer_value(),
        ctx.context.ptr_type(AddressSpace::default()),
        &format!("{}.ptr", name),
    ).unwrap().into_pointer_value();
    let len = ctx.context.i32_type().const_int(msg.len() as u64, true);
    (ptr, len)
}

/// 在 fail 基本块中调用 k_panic(msg) 并 unreachable
fn call_panic<'ctx>(ctx: &mut CodeGenContext<'ctx>, msg: &[u8], tag: &str) {
    let global_name = format!("panic.msg.{}", tag);
    let (ptr, len) = panic_msg_global(ctx, &global_name, msg);
    let panic_func = ctx.panic_func
        .expect("k_panic 应在 builtins::declare_k_panic 中已设置（通过 ctx.panic_func）");
    let _ = ctx.builder.build_call(panic_func, &[ptr.into(), len.into()], "panic");
    let _ = ctx.builder.build_unreachable();
}

/// 插入数组索引越界检查: 0 <= index < len
/// 检查失败时调用 k_panic 输出诊断信息
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

    // 失败路径: k_panic
    ctx.builder.position_at_end(fail_bb);
    call_panic(ctx, b"index out of bounds", "bounds");

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

    // 失败路径: k_panic
    ctx.builder.position_at_end(fail_bb);
    call_panic(ctx, b"division by zero", "divz");

    // 正常路径
    ctx.builder.position_at_end(ok_bb);
}

/// 插入浮点除零检查: divisor != 0.0
pub fn insert_float_div_zero_check<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    divisor: FloatValue<'ctx>,
) {
    ctx.runtime_checks += 1;

    let current_fn = ctx.builder.get_insert_block().unwrap().get_parent().unwrap();

    let zero = ctx.context.f64_type().const_float(0.0);
    let is_zero = ctx.builder.build_float_compare(
        inkwell::FloatPredicate::OEQ, divisor, zero, "fdivz",
    ).unwrap();

    let fail_bb = ctx.context.append_basic_block(current_fn, "fdivz.fail");
    let ok_bb = ctx.context.append_basic_block(current_fn, "fdivz.ok");

    let _ = ctx.builder.build_conditional_branch(is_zero, fail_bb, ok_bb);

    ctx.builder.position_at_end(fail_bb);
    call_panic(ctx, b"float division by zero", "fdivz");

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

    // 失败路径: k_panic
    ctx.builder.position_at_end(fail_bb);
    call_panic(ctx, b"integer overflow", "overflow");

    // 正常路径
    ctx.builder.position_at_end(ok_bb);
}
