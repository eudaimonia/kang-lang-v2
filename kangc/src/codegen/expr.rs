// 表达式代码生成 — 将 TypedExpr 转为 LLVM IR 值

use super::context::CodeGenContext;
use super::runtime;
use crate::ast;
use crate::error::CodeGenError;
use crate::semantic::{KangType, TypedExpr, TypedExprKind};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FloatValue, IntValue};
use inkwell::AddressSpace;

type Result<T> = std::result::Result<T, CodeGenError>;

/// 生成表达式的 LLVM IR 值
pub fn codegen_expr<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    expr: &TypedExpr,
) -> Result<BasicValueEnum<'ctx>> {
    match &expr.kind {
        TypedExprKind::IntLit(s) => codegen_int_lit(ctx, s),
        TypedExprKind::FloatLit(s) => codegen_float_lit(ctx, s),
        TypedExprKind::StrLit(s) => codegen_str_lit(ctx, s),
        TypedExprKind::BoolLit(b) => codegen_bool_lit(ctx, *b),
        TypedExprKind::Ident(name) => codegen_ident(ctx, name),
        TypedExprKind::Binary { left, op, right } => codegen_binary(ctx, left, *op, right),
        TypedExprKind::Unary { op, expr: inner } => codegen_unary(ctx, *op, inner),
        TypedExprKind::Call { func_name, args } => codegen_call(ctx, func_name, args, &expr.ty),
        TypedExprKind::Index { array, index } => codegen_index(ctx, array, index, &expr.ty),
        TypedExprKind::FieldAccess { obj, field } => codegen_field_access(ctx, obj, field),
        TypedExprKind::ArrayLit(elems) => codegen_array_lit(ctx, elems, &expr.ty),
        TypedExprKind::StructLit { name, fields } => codegen_struct_lit(ctx, name, fields),
    }
}

// Builder 操作仅当无 insert point 时失败，编译器保证始终有位置
// 若失败则返回 CodeGenError 而非 panic（防御性编程）
fn ok<T>(r: std::result::Result<T, inkwell::builder::BuilderError>) -> Result<T> {
    r.map_err(|e| CodeGenError { msg: format!("LLVM builder error: {}", e) })
}

/// 从 CallSiteValue 中提取 BasicValueEnum。
///
/// 调用者必须在调用前检查返回值类型不是 void（语义检查保证非 void 函数才有返回值）。
/// `codegen_call` 已做 `return_ty.is_void()` 守卫，其余调用点用于已知非 void 的内置函数。
fn call_val<'ctx>(call: inkwell::values::CallSiteValue<'ctx>) -> Result<BasicValueEnum<'ctx>> {
    use inkwell::values::ValueKind;
    match call.try_as_basic_value() {
        ValueKind::Basic(bv) => Ok(bv),
        _ => Err(CodeGenError { msg: "期望函数返回值，但得到 void".into() }),
    }
}

// ── 字面量 ─────────────────────────────────────────────────────────────────

fn codegen_int_lit<'ctx>(ctx: &CodeGenContext<'ctx>, s: &str) -> Result<BasicValueEnum<'ctx>> {
    let val: i64 = s.parse().map_err(|_| CodeGenError { msg: format!("无效整数: {}", s) })?;
    if val < i32::MIN as i64 || val > i32::MAX as i64 {
        return Err(CodeGenError { msg: format!("整数超出 i32 范围: {}", s) });
    }
    Ok(ctx.context.i32_type().const_int(val as u64, true).into())
}

fn codegen_float_lit<'ctx>(ctx: &CodeGenContext<'ctx>, s: &str) -> Result<BasicValueEnum<'ctx>> {
    let val: f64 = s.parse().map_err(|_| CodeGenError { msg: format!("无效浮点数: {}", s) })?;
    Ok(ctx.context.f64_type().const_float(val).into())
}

fn codegen_str_lit<'ctx>(ctx: &mut CodeGenContext<'ctx>, s: &str) -> Result<BasicValueEnum<'ctx>> {
    let bytes = s.as_bytes();
    let len = bytes.len() as i32;

    let array_type = ctx.context.i8_type().array_type(bytes.len() as u32);
    let global = ctx.module.add_global(array_type, None, ".str");
    // 跨模块链接时避免重复符号冲突: 每个 .o 文件内部生成 .str / .str.1 / ...
    global.set_linkage(inkwell::module::Linkage::Private);
    let init_vals: Vec<IntValue> = bytes
        .iter()
        .map(|&b| ctx.context.i8_type().const_int(b as u64, false))
        .collect();
    global.set_initializer(&ctx.context.i8_type().const_array(&init_vals));

    let ptr = global.as_pointer_value();
    let ptr_cast = ok(ctx.builder.build_pointer_cast(
        ptr,
        ctx.context.ptr_type(AddressSpace::default()),
        "str.ptr",
    ))?;

    let len_val = ctx.context.i32_type().const_int(len as u64, true);
    let kstr_type = ctx.kang_type_to_basic(&KangType::Str).into_struct_type();
    let undef = kstr_type.const_zero();
    let s1 = ok(ctx.builder.build_insert_value(undef, ptr_cast, 0, "str.packed.ptr"))?.into_struct_value();
    let s2 = ok(ctx.builder.build_insert_value(s1, len_val, 1, "str.packed"))?.into_struct_value();
    Ok(s2.into())
}

fn codegen_bool_lit<'ctx>(ctx: &CodeGenContext<'ctx>, b: bool) -> Result<BasicValueEnum<'ctx>> {
    Ok(ctx.context.bool_type().const_int(b as u64, false).into())
}

fn codegen_ident<'ctx>(ctx: &mut CodeGenContext<'ctx>, name: &str) -> Result<BasicValueEnum<'ctx>> {
    let (ptr, ty) = ctx
        .lookup_var(name)
        .ok_or_else(|| CodeGenError { msg: format!("未定义变量: {}", name) })?;
    let llvm_ty = ctx.kang_type_to_basic(&ty);
    Ok(ok(ctx.builder.build_load(llvm_ty, ptr, name))?)
}

// ── 二元运算 ───────────────────────────────────────────────────────────────

fn codegen_binary<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    left: &TypedExpr,
    op: ast::BinOp,
    right: &TypedExpr,
) -> Result<BasicValueEnum<'ctx>> {
    if matches!(op, ast::BinOp::Add) && (matches!(left.ty, KangType::Str) || matches!(right.ty, KangType::Str)) {
        return codegen_str_concat(ctx, left, right);
    }

    let lhs = codegen_expr(ctx, left)?;
    let rhs = codegen_expr(ctx, right)?;

    // 字符串 == / != 需要特殊处理
    if matches!(op, ast::BinOp::Eq) && matches!(left.ty, KangType::Str) {
        return codegen_str_eq(ctx, lhs, rhs, false);
    }
    if matches!(op, ast::BinOp::Neq) && matches!(left.ty, KangType::Str) {
        return codegen_str_eq(ctx, lhs, rhs, true);
    }

    match op {
        ast::BinOp::Add => codegen_arith(ctx, lhs, rhs, |b, l, r| ok(b.build_int_add(l, r, "add")), |b, l, r| ok(b.build_float_add(l, r, "add"))),
        ast::BinOp::Sub => codegen_arith(ctx, lhs, rhs, |b, l, r| ok(b.build_int_sub(l, r, "sub")), |b, l, r| ok(b.build_float_sub(l, r, "sub"))),
        ast::BinOp::Mul => codegen_arith(ctx, lhs, rhs, |b, l, r| ok(b.build_int_mul(l, r, "mul")), |b, l, r| ok(b.build_float_mul(l, r, "mul"))),
        ast::BinOp::Div => codegen_div(ctx, lhs, rhs),
        ast::BinOp::Eq => codegen_cmp(ctx, lhs, rhs, inkwell::IntPredicate::EQ, inkwell::FloatPredicate::OEQ),
        ast::BinOp::Neq => codegen_cmp(ctx, lhs, rhs, inkwell::IntPredicate::NE, inkwell::FloatPredicate::ONE),
        ast::BinOp::Lt => codegen_cmp(ctx, lhs, rhs, inkwell::IntPredicate::SLT, inkwell::FloatPredicate::OLT),
        ast::BinOp::Le => codegen_cmp(ctx, lhs, rhs, inkwell::IntPredicate::SLE, inkwell::FloatPredicate::OLE),
        ast::BinOp::Gt => codegen_cmp(ctx, lhs, rhs, inkwell::IntPredicate::SGT, inkwell::FloatPredicate::OGT),
        ast::BinOp::Ge => codegen_cmp(ctx, lhs, rhs, inkwell::IntPredicate::SGE, inkwell::FloatPredicate::OGE),
        ast::BinOp::And => {
            let l_bool = ok(ctx.builder.build_int_compare(inkwell::IntPredicate::NE, lhs.into_int_value(), ctx.context.bool_type().const_zero(), "and.l"))?;
            let r_bool = ok(ctx.builder.build_int_compare(inkwell::IntPredicate::NE, rhs.into_int_value(), ctx.context.bool_type().const_zero(), "and.r"))?;
            Ok(ok(ctx.builder.build_and(l_bool, r_bool, "and"))?.into())
        }
        ast::BinOp::Or => {
            let l_bool = ok(ctx.builder.build_int_compare(inkwell::IntPredicate::NE, lhs.into_int_value(), ctx.context.bool_type().const_zero(), "or.l"))?;
            let r_bool = ok(ctx.builder.build_int_compare(inkwell::IntPredicate::NE, rhs.into_int_value(), ctx.context.bool_type().const_zero(), "or.r"))?;
            Ok(ok(ctx.builder.build_or(l_bool, r_bool, "or"))?.into())
        }
    }
}

fn codegen_arith<'ctx, FI, FF>(
    ctx: &CodeGenContext<'ctx>,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
    int_op: FI,
    float_op: FF,
) -> Result<BasicValueEnum<'ctx>>
where
    FI: FnOnce(&inkwell::builder::Builder<'ctx>, IntValue<'ctx>, IntValue<'ctx>) -> Result<IntValue<'ctx>>,
    FF: FnOnce(&inkwell::builder::Builder<'ctx>, FloatValue<'ctx>, FloatValue<'ctx>) -> Result<FloatValue<'ctx>>,
{
    match lhs {
        BasicValueEnum::IntValue(l) => Ok(int_op(&ctx.builder, l, rhs.into_int_value())?.into()),
        BasicValueEnum::FloatValue(l) => Ok(float_op(&ctx.builder, l, rhs.into_float_value())?.into()),
        _ => Err(CodeGenError { msg: "算术运算仅支持 i32/f64".into() }),
    }
}

/// 生成除法/求余运算的 LLVM IR，包含运行时安全检查。
///
/// 整数除法前插入除零检查（R3）和 INT_MIN / -1 溢出检查（R4），
/// 浮点除法前插入除零检查。安全检查失败时调用 k_panic 终止程序。
fn codegen_div<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
) -> Result<BasicValueEnum<'ctx>> {
    match lhs {
        BasicValueEnum::IntValue(l) => {
            let r = rhs.into_int_value();
            // R3: 除零检查
            runtime::insert_div_zero_check(ctx, r);
            // R4: INT_MIN / -1 溢出检查
            runtime::insert_int_min_check(ctx, l, r);
            Ok(ok(ctx.builder.build_int_signed_div(l, r, "div"))?.into())
        }
        BasicValueEnum::FloatValue(l) => {
            let r = rhs.into_float_value();
            runtime::insert_float_div_zero_check(ctx, r);
            Ok(ok(ctx.builder.build_float_div(l, r, "div"))?.into())
        }
        _ => Err(CodeGenError { msg: "除法仅支持 i32/f64".into() }),
    }
}

fn codegen_cmp<'ctx>(
    ctx: &CodeGenContext<'ctx>,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
    int_pred: inkwell::IntPredicate,
    float_pred: inkwell::FloatPredicate,
) -> Result<BasicValueEnum<'ctx>> {
    match lhs {
        BasicValueEnum::IntValue(l) => Ok(ok(ctx.builder.build_int_compare(int_pred, l, rhs.into_int_value(), "cmp"))?.into()),
        BasicValueEnum::FloatValue(l) => Ok(ok(ctx.builder.build_float_compare(float_pred, l, rhs.into_float_value(), "cmp"))?.into()),
        _ => Err(CodeGenError { msg: "比较运算仅支持 i32/f64".into() }),
    }
}

// ── 字符串相等比较 ─────────────────────────────────────────────────────────

fn codegen_str_eq<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
    negate: bool,
) -> Result<BasicValueEnum<'ctx>> {
    let lhs_s = lhs.into_struct_value();
    let rhs_s = rhs.into_struct_value();

    let l_len = ok(ctx.builder.build_extract_value(lhs_s, 1, "str.eq.l.len"))?.into_int_value();
    let r_len = ok(ctx.builder.build_extract_value(rhs_s, 1, "str.eq.r.len"))?.into_int_value();

    let len_eq = ok(ctx.builder.build_int_compare(
        inkwell::IntPredicate::EQ, l_len, r_len, "str.eq.len",
    ))?;

    let i32_ty = ctx.context.i32_type();

    let current_fn = ctx.builder.get_insert_block().unwrap().get_parent().unwrap();
    let entry_bb = ctx.builder.get_insert_block().unwrap();
    let loop_init_bb = ctx.context.append_basic_block(current_fn, "str.eq.init");
    let loop_cond_bb = ctx.context.append_basic_block(current_fn, "str.eq.cond");
    let loop_body_bb = ctx.context.append_basic_block(current_fn, "str.eq.body");
    let merge_bb = ctx.context.append_basic_block(current_fn, "str.eq.merge");

    let _ = ctx.builder.build_conditional_branch(len_eq, loop_init_bb, merge_bb);

    // 循环初始化: i = 0
    ctx.builder.position_at_end(loop_init_bb);
    let i_alloca = ok(ctx.builder.build_alloca(i32_ty, "str.eq.i"))?;
    let _ = ctx.builder.build_store(i_alloca, i32_ty.const_zero());
    let _ = ctx.builder.build_unconditional_branch(loop_cond_bb);

    // 循环条件: i < len
    ctx.builder.position_at_end(loop_cond_bb);
    let i_val = ok(ctx.builder.build_load(i32_ty, i_alloca, "str.eq.i.val"))?.into_int_value();
    let not_done = ok(ctx.builder.build_int_compare(
        inkwell::IntPredicate::SLT, i_val, l_len, "str.eq.cond",
    ))?;
    let _ = ctx.builder.build_conditional_branch(not_done, loop_body_bb, merge_bb);

    // 循环体: 逐字节比较
    ctx.builder.position_at_end(loop_body_bb);
    let l_ptr = ok(ctx.builder.build_extract_value(lhs_s, 0, "str.eq.l.ptr"))?.into_pointer_value();
    let r_ptr = ok(ctx.builder.build_extract_value(rhs_s, 0, "str.eq.r.ptr"))?.into_pointer_value();
    // GEP: 计算 l_ptr + i 和 r_ptr + i
    let l_gep = unsafe {
        ctx.builder.build_in_bounds_gep(
            ctx.context.i8_type(),
            l_ptr,
            &[i_val],
            "str.eq.l.gep",
        )
    }.unwrap();
    let r_gep = unsafe {
        ctx.builder.build_in_bounds_gep(
            ctx.context.i8_type(),
            r_ptr,
            &[i_val],
            "str.eq.r.gep",
        )
    }.unwrap();
    let l_byte = ok(ctx.builder.build_load(ctx.context.i8_type(), l_gep, "str.eq.lb"))?.into_int_value();
    let r_byte = ok(ctx.builder.build_load(ctx.context.i8_type(), r_gep, "str.eq.rb"))?.into_int_value();
    let bytes_eq = ok(ctx.builder.build_int_compare(
        inkwell::IntPredicate::EQ, l_byte, r_byte, "str.eq.beq",
    ))?;

    let loop_inc_bb = ctx.context.append_basic_block(current_fn, "str.eq.inc");
    let _ = ctx.builder.build_conditional_branch(bytes_eq, loop_inc_bb, merge_bb);

    // i++
    ctx.builder.position_at_end(loop_inc_bb);
    let i_next = ok(ctx.builder.build_int_add(
        i_val, i32_ty.const_int(1, true), "str.eq.inc",
    ))?;
    let _ = ctx.builder.build_store(i_alloca, i_next);
    let _ = ctx.builder.build_unconditional_branch(loop_cond_bb);

    // 合并: phi [false, entry], [false, loop_body], [true, loop_cond]
    ctx.builder.position_at_end(merge_bb);
    let phi = ok(ctx.builder.build_phi(ctx.context.bool_type(), "str.eq.result"))?;
    let false_val = ctx.context.bool_type().const_zero();
    let true_val = ctx.context.bool_type().const_int(1, true);
    phi.add_incoming(&[(&false_val, entry_bb), (&false_val, loop_body_bb), (&true_val, loop_cond_bb)]);

    let result: BasicValueEnum = phi.as_basic_value().into();
    if negate {
        Ok(ok(ctx.builder.build_xor(
            result.into_int_value(),
            ctx.context.bool_type().const_int(1, false),
            "str.ne",
        ))?.into())
    } else {
        Ok(result.into())
    }
}

// ── 字符串拼接 ─────────────────────────────────────────────────────────────

// ── 字符串拼接 ─────────────────────────────────────────────────────────────

/// 生成字符串拼接的 LLVM IR: left + right。
///
/// 两个操作数先转为 str（非 str 操作数在 binary 解析时自动转换），
/// 提取各自的 {ptr, len}，调用 C 函数 k_str_concat 在 arena 上分配新字符串，
/// 返回新的 {ptr, len}。
fn codegen_str_concat<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    left: &TypedExpr,
    right: &TypedExpr,
) -> Result<BasicValueEnum<'ctx>> {
    let lhs = codegen_expr(ctx, left)?;
    let rhs = codegen_expr(ctx, right)?;
    let lhs_str = convert_to_str(ctx, lhs, &left.ty)?;
    let rhs_str = convert_to_str(ctx, rhs, &right.ty)?;

    let lhs_s = lhs_str.into_struct_value();
    let rhs_s = rhs_str.into_struct_value();
    let l_ptr = ok(ctx.builder.build_extract_value(lhs_s, 0, "l.ptr"))?;
    let l_len = ok(ctx.builder.build_extract_value(lhs_s, 1, "l.len"))?;
    let r_ptr = ok(ctx.builder.build_extract_value(rhs_s, 0, "r.ptr"))?;
    let r_len = ok(ctx.builder.build_extract_value(rhs_s, 1, "r.len"))?;

    let func = ctx.module.get_function("k_str_concat")
        .ok_or_else(|| CodeGenError { msg: "k_str_concat 未声明".into() })?;
    let call = ok(ctx.builder.build_call(func, &[
        l_ptr.into(), l_len.into_int_value().into(),
        r_ptr.into(), r_len.into_int_value().into(),
    ], "concat"))?;
    call_val(call)
}

/// 将非 str 值转为 str（调用内置 str() 函数）。
///
/// 用于字符串拼接（+）时自动转换另一操作数为字符串。i32 调用 k_str_i32，
/// f64 调用 k_str_f64，bool 先 zext 到 i32 再调用 k_str_bool。
fn convert_to_str<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    ty: &KangType,
) -> Result<BasicValueEnum<'ctx>> {
    match ty {
        KangType::Str => Ok(val),
        KangType::I32 => {
            let func = ctx.module.get_function("k_str_i32")
                .ok_or_else(|| CodeGenError { msg: "k_str_i32 未声明".into() })?;
            call_val(ok(ctx.builder.build_call(func, &[val.into_int_value().into()], "to.str"))?)
        }
        KangType::F64 => {
            let func = ctx.module.get_function("k_str_f64")
                .ok_or_else(|| CodeGenError { msg: "k_str_f64 未声明".into() })?;
            call_val(ok(ctx.builder.build_call(func, &[val.into_float_value().into()], "to.str"))?)
        }
        KangType::Bool => {
            let func = ctx.module.get_function("k_str_bool")
                .ok_or_else(|| CodeGenError { msg: "k_str_bool 未声明".into() })?;
            let i32_val = ok(ctx.builder.build_int_z_extend(val.into_int_value(), ctx.context.i32_type(), "bool.i32"))?;
            call_val(ok(ctx.builder.build_call(func, &[i32_val.into()], "to.str"))?)
        }
        _ => Ok(val),
    }
}

// ── 一元运算 ───────────────────────────────────────────────────────────────

fn codegen_unary<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    op: ast::UnaryOp,
    expr: &TypedExpr,
) -> Result<BasicValueEnum<'ctx>> {
    let val = codegen_expr(ctx, expr)?;
    match op {
        ast::UnaryOp::Neg => match val {
            BasicValueEnum::IntValue(v) => Ok(ok(ctx.builder.build_int_neg(v, "neg"))?.into()),
            BasicValueEnum::FloatValue(v) => Ok(ok(ctx.builder.build_float_neg(v, "neg"))?.into()),
            _ => Err(CodeGenError { msg: "取负仅支持 i32/f64".into() }),
        },
        ast::UnaryOp::Not => Ok(ok(ctx.builder.build_int_compare(
            inkwell::IntPredicate::EQ, val.into_int_value(), ctx.context.bool_type().const_zero(), "not",
        ))?.into()),
    }
}

// ── 函数调用 ───────────────────────────────────────────────────────────────

/// C ABI 调用时将 Kang 复合类型（Str/Array/Pair）展平为标量参数。
///
/// ARM64 AAPCS64 规定结构体在寄存器中传递时不打包，每个字段占一个寄存器。
/// 但 Kang 的 Str/Array 类型是 {ptr, len} 结构体，传递时必须拆为 (ptr, len) 两个标量参数
/// 才能与 C 函数签名匹配。Bool 需要 zext 到 i32（C 的 _Bool 在 ARM64 上也是 i32）。
/// Pair 作为参数时只取第一值（F6 规则）。
fn flatten_c_abi_args<'ctx>(
    ctx: &CodeGenContext<'ctx>,
    arg_values: &[BasicValueEnum<'ctx>],
    args: &[TypedExpr],
) -> Result<Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>> {
    let mut flat: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
    for (val, arg) in arg_values.iter().zip(args) {
        match &arg.ty {
            KangType::Str | KangType::Array(_) => {
                let sv = val.into_struct_value();
                let ptr = ok(ctx.builder.build_extract_value(sv, 0, "arg.ptr"))?;
                let len = ok(ctx.builder.build_extract_value(sv, 1, "arg.len"))?;
                flat.push(ptr.into());
                flat.push(len.into());
            }
            KangType::Pair(_, _) => {
                // F6: Pair 作参数时取第一值
                let sv = val.into_struct_value();
                let v0 = ok(ctx.builder.build_extract_value(sv, 0, "arg.pair.0"))?;
                flat.push(v0.into());
            }
            KangType::Bool => {
                // C ABI: bool 以 i32 传递, 将 i1 zext 到 i32
                let i32_val = ok(ctx.builder.build_int_z_extend(
                    val.into_int_value(),
                    ctx.context.i32_type(),
                    "bool.i32",
                ))?;
                flat.push(i32_val.into());
            }
            _ => {
                flat.push((*val).into());
            }
        }
    }
    Ok(flat)
}

/// C ABI 返回的 Pair 结构从扁平 C struct 转为 Kang 嵌套 struct。
///
/// 运行时函数（如 k_read_file）通过 C ABI 返回扁平结构体（i32 表示 bool），
/// 但 Kang 的 Pair 类型是嵌套结构（i1 表示 bool）。这层转换将两种布局桥接：
///   - KStrBool:  {ptr, i64}  →  {{ptr, i32}, i1}  （len 和 ok 打包在 i64 中）
///   - KI32Bool:  i64         →  {i32, i1}          （val 和 ok 打包在 i64 中）
///   - KF64Bool:  {f64, i64}  →  {f64, i1}          （ok 符号扩展到 i64）
///   - KBoolBool: i64         →  {i1, i1}            （两个 bool 打包在 i64 中）
fn repack_c_pair_return<'ctx>(
    ctx: &CodeGenContext<'ctx>,
    call: inkwell::values::CallSiteValue<'ctx>,
    return_ty: &KangType,
) -> Result<BasicValueEnum<'ctx>> {
    // C ABI 类型声明为打包形式以匹配 rustc 的 AAPCS64 布局:
    //   {i32, i32} → i64 (ok << 32 | val)
    //   {ptr, i32, i32} → {ptr, i64} (ptr 在 x0, (ok<<32)|len 在 x1)
    //   {f64, i32} → {f64, i64} (f64 在 x0, ok sign-ext 到 64 位在 x1)
    let c_val = call_val(call)?;
    let kang_pair = ctx.kang_type_to_basic(return_ty).into_struct_type();
    let kang_undef = kang_pair.const_zero();

    match return_ty {
        KangType::Pair(first, second) => {
            match (first.as_ref(), second.as_ref()) {
                // KStrBool: {ptr, i64} 其中 i64 = (ok << 32) | len
                (KangType::Str, KangType::Bool) => {
                    let c_struct = c_val.into_struct_value();
                    let ptr = ok(ctx.builder.build_extract_value(c_struct, 0, "cp.ptr"))?;
                    let packed = ok(ctx.builder.build_extract_value(c_struct, 1, "cp.packed"))?;
                    let packed_val = packed.into_int_value();
                    let len = ok(ctx.builder.build_int_truncate(packed_val, ctx.context.i32_type(), "cp.len"))?;
                    let ok_shifted = ok(ctx.builder.build_int_truncate(
                        ok(ctx.builder.build_int_unsigned_div(packed_val, ctx.context.i64_type().const_int(0x100000000u64, false), "ok.shift"))?,
                        ctx.context.i32_type(), "ok.i32",
                    ))?;
                    let ok_i1 = ok(ctx.builder.build_int_truncate(ok_shifted, ctx.context.bool_type(), "ok.bool"))?;
                    build_kang_str_pair(ctx, kang_pair, ptr, len, ok_i1)
                }
                // KI32Bool / KBoolBool: i64 = (ok << 32) | val
                (KangType::I32, KangType::Bool) => {
                    let packed = c_val.into_int_value();
                    let val = ok(ctx.builder.build_int_truncate(packed, ctx.context.i32_type(), "cp.val"))?;
                    let ok_shifted = ok(ctx.builder.build_int_truncate(
                        ok(ctx.builder.build_int_unsigned_div(packed, ctx.context.i64_type().const_int(0x100000000u64, false), "ok.shift"))?,
                        ctx.context.i32_type(), "ok.i32",
                    ))?;
                    let ok_i1 = ok(ctx.builder.build_int_truncate(ok_shifted, ctx.context.bool_type(), "ok.bool"))?;
                    let p1 = ok(ctx.builder.build_insert_value(kang_undef, val, 0, "rp.pair.0"))?;
                    let p2 = ok(ctx.builder.build_insert_value(p1, ok_i1, 1, "rp.pair.1"))?;
                    Ok(p2.into_struct_value().into())
                }
                // KF64Bool: {f64, i64} 其中 i64 = ok sign-extended
                (KangType::F64, KangType::Bool) => {
                    let c_struct = c_val.into_struct_value();
                    let val = ok(ctx.builder.build_extract_value(c_struct, 0, "cp.val"))?;
                    let ok_i64 = ok(ctx.builder.build_extract_value(c_struct, 1, "cp.ok"))?;
                    let ok_i1 = ok(ctx.builder.build_int_truncate(ok_i64.into_int_value(), ctx.context.bool_type(), "ok.bool"))?;
                    let p1 = ok(ctx.builder.build_insert_value(kang_undef, val, 0, "rp.pair.0"))?;
                    let p2 = ok(ctx.builder.build_insert_value(p1, ok_i1, 1, "rp.pair.1"))?;
                    Ok(p2.into_struct_value().into())
                }
                // KBoolBool: i64 = (ok << 32) | val, 两个都是 bool
                (KangType::Bool, KangType::Bool) => {
                    let packed = c_val.into_int_value();
                    let val_i32 = ok(ctx.builder.build_int_truncate(packed, ctx.context.i32_type(), "cp.val"))?;
                    let ok_shifted = ok(ctx.builder.build_int_truncate(
                        ok(ctx.builder.build_int_unsigned_div(packed, ctx.context.i64_type().const_int(0x100000000u64, false), "ok.shift"))?,
                        ctx.context.i32_type(), "ok.i32",
                    ))?;
                    let val_i1 = ok(ctx.builder.build_int_truncate(val_i32, ctx.context.bool_type(), "val.bool"))?;
                    let ok_i1 = ok(ctx.builder.build_int_truncate(ok_shifted, ctx.context.bool_type(), "ok.bool"))?;
                    let p1 = ok(ctx.builder.build_insert_value(kang_undef, val_i1, 0, "rp.pair.0"))?;
                    let p2 = ok(ctx.builder.build_insert_value(p1, ok_i1, 1, "rp.pair.1"))?;
                    Ok(p2.into_struct_value().into())
                }
                _ => Ok(c_val.into()),
            }
        }
        _ => Ok(c_val.into()),
    }
}

/// 从 ptr、len 和 ok 标志构建 Kang 的 (str, bool) 嵌套结构 {{ptr, i32}, i1}。
///
/// 由 `repack_c_pair_return` 调用，用于将 C ABI 返回的扁平对转换为 Kang 的嵌套对类型。
/// 先构造内层 str {ptr, len}，再插入外层 pair {str, bool}。
fn build_kang_str_pair<'ctx>(
    ctx: &CodeGenContext<'ctx>,
    struct_ty: inkwell::types::StructType<'ctx>,
    ptr: inkwell::values::BasicValueEnum<'ctx>,
    len: inkwell::values::IntValue<'ctx>,
    ok_i1: inkwell::values::IntValue<'ctx>,
) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
    let str_undef = ctx.context.struct_type(
        &[ctx.context.ptr_type(AddressSpace::default()).into(), ctx.context.i32_type().into()],
        false,
    ).const_zero();
    let s1 = ok(ctx.builder.build_insert_value(str_undef, ptr, 0, "rp.ptr"))?;
    let s2 = ok(ctx.builder.build_insert_value(s1, len, 1, "rp.str"))?;
    let pair_undef = struct_ty.const_zero();
    let p1 = ok(ctx.builder.build_insert_value(pair_undef, s2, 0, "rp.pair.0"))?;
    let p2 = ok(ctx.builder.build_insert_value(p1, ok_i1, 1, "rp.pair.1"))?;
    Ok(p2.into_struct_value().into())
}

/// 生成函数调用指令。
///
/// 特殊处理 len/push 内置函数。对于外部 C ABI 函数，将 Str/Array/Bool 参数展平；
/// Kang 跨模块函数调用保持参数原样。Pair 返回值需要从 C 扁平布局 repack 为 Kang 嵌套布局。
fn codegen_call<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    func_name: &str,
    args: &[TypedExpr],
    return_ty: &KangType,
) -> Result<BasicValueEnum<'ctx>> {
    if func_name == "len" {
        return codegen_builtin_len(ctx, args);
    }
    if func_name == "push" {
        return codegen_builtin_push(ctx, args, return_ty);
    }

    let resolved_name = resolve_overloaded_name(func_name, args);

    let arg_values: Vec<BasicValueEnum> = args
        .iter()
        .map(|a| codegen_expr(ctx, a))
        .collect::<Result<Vec<_>>>()?;

    let func = ctx
        .module
        .get_function(&resolved_name)
        .or_else(|| ctx.lookup_func(&resolved_name))
        .unwrap_or_else(|| {
            // 跨模块调用: 声明为外部函数，链接时解析
            let arg_kang_types: Vec<KangType> = args.iter().map(|a| a.ty.clone()).collect();
            ctx.declare_func(&resolved_name, &arg_kang_types, return_ty)
        });

    // 仅对 C ABI 外部函数展平复合类型参数；Kang 跨模块调用保持原样
    let is_extern = func.get_first_basic_block().is_none();
    let is_kang = ctx.kang_funcs.contains(&resolved_name);
    let llvm_args: Vec<BasicMetadataValueEnum> = if is_extern && !is_kang {
        flatten_c_abi_args(ctx, &arg_values, args)?
    } else {
        arg_values.iter().map(|v| (*v).into()).collect()
    };

    let call = ok(ctx.builder.build_call(func, &llvm_args, "call"))?;
    if return_ty.is_void() {
        Ok(ctx.context.i32_type().const_zero().into())
    } else if func.get_first_basic_block().is_none() && matches!(return_ty, KangType::Pair(_, _)) {
        repack_c_pair_return(ctx, call, return_ty)
    } else {
        call_val(call)
    }
}

/// F6: Pair 自动解包取第一值（多返回值作单值参数）。
///
/// 当多返回值函数被用于期望单个值的上下文时（如 `str.substr(...)` 作为参数传给 `puts`），
/// 编译器自动取 Pair 的第一值，忽略第二值。
fn unpack_pair_first(ty: &KangType) -> &KangType {
    match ty {
        KangType::Pair(first, _) => first.as_ref(),
        other => other,
    }
}

/// 将重载函数名映射为带后缀的 LLVM 名。
///
/// Kang 的类型转换函数（str()、i32()、f64()、bool()）根据参数类型重载，
/// 但 LLVM IR 不支持重载，因此需要将 str(i32) 映射为 k_str_i32、str(f64) 映射为 k_str_f64 等。
fn resolve_overloaded_name(func_name: &str, args: &[TypedExpr]) -> String {
    if args.is_empty() {
        return func_name.to_string();
    }
    let first_ty = unpack_pair_first(&args[0].ty);
    match func_name {
        "str" => match first_ty {
            KangType::I32 => "str_i32".into(),
            KangType::F64 => "str_f64".into(),
            KangType::Bool => "str_bool".into(),
            _ => func_name.into(),
        },
        "i32" => match first_ty {
            KangType::Str => "i32_str".into(),
            KangType::F64 => "i32_f64".into(),
            _ => func_name.into(),
        },
        "f64" => match first_ty {
            KangType::Str => "f64_str".into(),
            KangType::I32 => "f64_i32".into(),
            _ => func_name.into(),
        },
        "bool" => match first_ty {
            KangType::Str => "bool_str".into(),
            _ => func_name.into(),
        },
        _ => func_name.into(),
    }
}

/// 生成 len() 内置函数的 LLVM IR。
///
/// len 返回数组或字符串的第二个字段（长度）。数组和字符串的结构都是 {ptr, len}，
/// 因此从 struct_value 提取 field 1 即可得到长度。
fn codegen_builtin_len<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    args: &[TypedExpr],
) -> Result<BasicValueEnum<'ctx>> {
    let arg = args
        .first()
        .ok_or_else(|| CodeGenError { msg: "builtin len 需要 1 个参数".into() })?;
    let arg = codegen_expr(ctx, arg)?;
    let struct_val = arg.into_struct_value();
    let len = ok(ctx.builder.build_extract_value(struct_val, 1, "len"))?;
    Ok(len.into())
}

/// 生成 push() 内置函数的 LLVM IR。
///
/// push 的过程: 将数组 {ptr, len} 和待追加元素的指针、元素大小传给 C 函数 k_push，
/// k_push 分配新内存、拷贝旧元素和新元素，返回新的 {ptr, len}。
/// 关键: codegen 必须将 k_push 返回的新 {ptr, len} 写回原数组变量的 alloca，
/// 否则变量中的 ptr/len 不会更新。推送到字面量数组时返回值被丢弃（语义检查应拦截）。
fn codegen_builtin_push<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    args: &[TypedExpr],
    _return_ty: &KangType,
) -> Result<BasicValueEnum<'ctx>> {
    if args.len() < 2 {
        return Err(CodeGenError { msg: "builtin push 需要 2 个参数".into() });
    }
    let arr = codegen_expr(ctx, &args[0])?;
    let elem = codegen_expr(ctx, &args[1])?;

    let arr_struct = arr.into_struct_value();
    let arr_ptr = ok(ctx.builder.build_extract_value(arr_struct, 0, "arr.ptr"))?;
    let arr_len = ok(ctx.builder.build_extract_value(arr_struct, 1, "arr.len"))?;

    let elem_llvm_ty = ctx.kang_type_to_basic(&args[1].ty);
    let alloca = ok(ctx.builder.build_alloca(elem_llvm_ty, "elem.alloca"))?;
    let _ = ctx.builder.build_store(alloca, elem);
    let elem_ptr = ok(ctx.builder.build_pointer_cast(
        alloca,
        ctx.context.ptr_type(AddressSpace::default()),
        "elem.cast",
    ))?;

    let elem_size = size_of_kang(ctx, &args[1].ty);
    let elem_size_val = ctx.context.i32_type().const_int(elem_size as u64, true);

    let func = ctx.module.get_function("k_push")
        .ok_or_else(|| CodeGenError { msg: "k_push 未声明".into() })?;
    let call = ok(ctx.builder.build_call(func, &[
        arr_ptr.into(),
        arr_len.into(),
        elem_ptr.into(),
        elem_size_val.into(),
    ], "push"))?;

    // k_push 返回新的 {ptr, len}, 需要写回数组变量的 alloca
    if let TypedExprKind::Ident(var_name) = &args[0].kind {
        if let Some((var_ptr, _)) = ctx.lookup_var(var_name) {
            let new_arr = call_val(call)?;
            let _ = ctx.builder.build_store(var_ptr, new_arr);
            return Ok(ctx.context.i32_type().const_zero().into());
        }
    }

    // 非 Ident 参数: 函数返回值虽被丢弃但语义上无意义(如 push(字面量, x))
    // 编译器语义检查阶段应拦截此类用法, 此处仅防御
    call_val(call)?;
    Ok(ctx.context.i32_type().const_zero().into())
}

/// Kang 类型的 LLVM 存储大小（字节），用于数组元素偏移计算。
///
/// 通过 CodeGenContext::size_of 获取 LLVM TargetData 中类型的存储大小，
/// 或者在缺少 TargetData 时使用手动对齐计算。返回 i32 以简化与 LLVM i32 运算的互操作。
pub fn size_of_kang(ctx: &CodeGenContext, ty: &KangType) -> i32 {
    ctx.size_of(ty) as i32
}

// ── 索引 ───────────────────────────────────────────────────────────────────

/// 生成数组/字符串索引的 LLVM IR: arr[i] 或 s[i]。
///
/// 数组内存布局: [4 字节长度头][元素 0][元素 1]...
/// 字符串内存布局: [字符 0][字符 1]...（无长度头）
/// 因此数组索引跳过前 4 字节，字符串索引从开头算起。字符串索引返回单个字符的
/// Kang str {ptr, len=1}，数组索引返回元素值。
fn codegen_index<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    array: &TypedExpr,
    index: &TypedExpr,
    result_ty: &KangType,
) -> Result<BasicValueEnum<'ctx>> {
    let arr_val = codegen_expr(ctx, array)?;
    let idx_val = codegen_expr(ctx, index)?;

    let arr_struct = arr_val.into_struct_value();
    let arr_ptr = ok(ctx.builder.build_extract_value(arr_struct, 0, "arr.ptr"))?;
    let arr_len = ok(ctx.builder.build_extract_value(arr_struct, 1, "arr.len"))?;
    let idx = idx_val.into_int_value();

    // R1/R2: 数组/字符串索引越界检查
    runtime::insert_bounds_check(ctx, idx, arr_len.into_int_value());

    let is_str = matches!(array.ty, KangType::Str);

    // 字符串: 指针直接指向数据, 每元素 1 字节; 数组: 跳过 4 字节长度头
    let elem_size = if is_str { 1 } else { size_of_kang(ctx, result_ty) };

    let arr_addr = ok(ctx.builder.build_ptr_to_int(
        arr_ptr.into_pointer_value(),
        ctx.context.i64_type(),
        "arr.addr",
    ))?;

    let base_addr = if is_str {
        arr_addr
    } else {
        ok(ctx.builder.build_int_add(
            arr_addr,
            ctx.context.i64_type().const_int(4u64, false),
            "base",
        ))?
    };

    let offset = ok(ctx.builder.build_int_mul(
        idx,
        ctx.context.i32_type().const_int(elem_size as u64, true),
        "offset",
    ))?;
    let offset_64 = ok(ctx.builder.build_int_z_extend(offset, ctx.context.i64_type(), "offset64"))?;
    let elem_addr = ok(ctx.builder.build_int_add(base_addr, offset_64, "elem.addr"))?;
    let elem_ptr = ok(ctx.builder.build_int_to_ptr(
        elem_addr,
        ctx.context.ptr_type(AddressSpace::default()),
        "elem.ptr",
    ))?;

    // 字符串索引返回 Kang str 结构体 {ptr, 1}，数组索引返回元素值
    if is_str {
        let kstr_type = ctx.kang_type_to_basic(result_ty).into_struct_type();
        let undef = kstr_type.const_zero();
        let len_val = ctx.context.i32_type().const_int(1, true);
        let s1 = ok(ctx.builder.build_insert_value(undef, elem_ptr, 0, "ch.ptr"))?.into_struct_value();
        let s2 = ok(ctx.builder.build_insert_value(s1, len_val, 1, "ch.str"))?.into_struct_value();
        Ok(s2.into())
    } else {
        Ok(ok(ctx.builder.build_load(ctx.kang_type_to_basic(result_ty), elem_ptr, "elem"))?)
    }
}

// ── 字段访问 ──────────────────────────────────────────────────────────────

/// 生成结构体字段访问的 LLVM IR: obj.field。
///
/// 查找 obj 的结构体类型中的字段索引，使用 extract_value 从 LLVM struct 值中提取字段值。
/// 语义检查已保证 obj 是结构体且 field 存在。
fn codegen_field_access<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    obj: &TypedExpr,
    field: &str,
) -> Result<BasicValueEnum<'ctx>> {
    let obj_val = codegen_expr(ctx, obj)?;
    let struct_name = match &obj.ty {
        KangType::Struct(name) => name.clone(),
        _ => return Err(CodeGenError { msg: "只能对结构体使用 .field".into() }),
    };

    let fields = ctx
        .struct_fields
        .get(&struct_name)
        .ok_or_else(|| CodeGenError { msg: format!("未定义的结构体: {}", struct_name) })?;

    let field_idx = fields
        .iter()
        .position(|(name, _)| name == field)
        .ok_or_else(|| CodeGenError { msg: format!("结构体 {} 无字段 {}", struct_name, field) })?;

    let struct_val = obj_val.into_struct_value();
    let field_val = ok(ctx.builder.build_extract_value(struct_val, field_idx as u32, field))?;
    Ok(field_val.into())
}

// ── 数组字面量 ─────────────────────────────────────────────────────────────

/// 生成数组字面量的 LLVM IR，如 [1, 2, 3]。
///
/// 通过 k_arena_alloc_aligned 从运行时 arena 分配连续内存，
/// 写入长度头（前 4 字节），然后依次写入每个元素。
/// 返回 {ptr, len} 结构体 — Kang 数组类型的运行时表示。
fn codegen_array_lit<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    elems: &[TypedExpr],
    array_ty: &KangType,
) -> Result<BasicValueEnum<'ctx>> {
    let elem_ty = match array_ty {
        KangType::Array(e) => &**e,
        _ => return Err(CodeGenError { msg: "数组类型错误".into() }),
    };

    let elem_count = elems.len() as i32;
    let elem_bytes = size_of_kang(ctx, elem_ty) as usize;
    let total_bytes = 4 + (elem_count as usize) * elem_bytes;

    let size_val = ctx.context.i32_type().const_int(total_bytes as u64, true);
    let align_val = ctx.context.i32_type().const_int(8u64, true);

    let alloc_func = ctx.module.get_function("k_arena_alloc_aligned")
        .ok_or_else(|| CodeGenError { msg: "k_arena_alloc_aligned 未声明 — builtins::declare_all 可能未调用".into() })?;

    let alloc_call = ok(ctx.builder.build_call(alloc_func, &[size_val.into(), align_val.into()], "arr.alloc"))?;
    let raw_ptr = call_val(alloc_call)?.into_pointer_value();

    // 写入长度头
    let len_val = ctx.context.i32_type().const_int(elem_count as u64, true);
    let len_ptr = ok(ctx.builder.build_pointer_cast(
        raw_ptr,
        ctx.context.ptr_type(AddressSpace::default()),
        "len.ptr",
    ))?;
    let _ = ctx.builder.build_store(len_ptr, len_val);

    // 写入每个元素
    for (i, elem) in elems.iter().enumerate() {
        let elem_val = codegen_expr(ctx, elem)?;
        let offset = 4 + (i * elem_bytes);
        let offset_val = ctx.context.i64_type().const_int(offset as u64, false);
        let raw_int = ok(ctx.builder.build_ptr_to_int(raw_ptr, ctx.context.i64_type(), "raw.int"))?;
        let elem_int = ok(ctx.builder.build_int_add(raw_int, offset_val, "elem.offset"))?;
        let elem_ptr = ok(ctx.builder.build_int_to_ptr(
            elem_int,
            ctx.context.ptr_type(AddressSpace::default()),
            "elem.ptr",
        ))?;
        let typed_elem_ptr = ok(ctx.builder.build_pointer_cast(
            elem_ptr,
            ctx.context.ptr_type(AddressSpace::default()),
            "typed.elem",
        ))?;
        let _ = ctx.builder.build_store(typed_elem_ptr, elem_val);
    }

    let kptrlen_type = ctx.kang_type_to_basic(array_ty).into_struct_type();
    let count_val = ctx.context.i32_type().const_int(elem_count as u64, true);
    let undef = kptrlen_type.const_zero();
    let s1 = ok(ctx.builder.build_insert_value(undef, raw_ptr, 0, "arr.packed.ptr"))?.into_struct_value();
    let s2 = ok(ctx.builder.build_insert_value(s1, count_val, 1, "arr.packed"))?.into_struct_value();
    Ok(s2.into())
}

// ── 结构体字面量 ───────────────────────────────────────────────────────────

/// 生成结构体字面量的 LLVM IR，如 Point{x: 1, y: 2}。
///
/// 初始化所有字段：用户提供的字段使用表达式求值，未提供的字段使用类型默认值（零初始化）。
/// 通过 insert_value 指令逐个字段填充 LLVM 结构体值。
fn codegen_struct_lit<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    name: &str,
    fields: &[(String, TypedExpr)],
) -> Result<BasicValueEnum<'ctx>> {
    let struct_type = ctx
        .lookup_struct_type(name)
        .ok_or_else(|| CodeGenError { msg: format!("未定义的结构体: {}", name) })?;

    let field_defs = ctx
        .struct_fields
        .get(name)
        .ok_or_else(|| CodeGenError { msg: format!("未定义的结构体: {}", name) })?
        .clone(); // 克隆以释放 ctx 的不可变借用

    // 空结构体直接返回 zero init
    if field_defs.is_empty() {
        return Ok(struct_type.const_zero().into());
    }

    let mut field_values: Vec<BasicValueEnum> =
        vec![ctx.default_value(&field_defs[0].1); field_defs.len()];

    for (i, (fname, fty)) in field_defs.iter().enumerate() {
        if let Some((_, fexpr)) = fields.iter().find(|(n, _)| n == fname) {
            field_values[i] = codegen_expr(ctx, fexpr)?;
        } else {
            field_values[i] = ctx.default_value(fty);
        }
    }

    let mut packed = struct_type.const_zero();
    for (i, val) in field_values.iter().enumerate() {
        packed = ok(ctx.builder.build_insert_value(packed, *val, i as u32, &format!("struct.packed.{}", i)))?.into_struct_value();
    }
    Ok(packed.into())
}
