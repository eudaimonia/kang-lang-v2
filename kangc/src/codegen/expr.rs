// 表达式代码生成 — 将 TypedExpr 转为 LLVM IR 值

use super::context::CodeGenContext;
use crate::ast;
use crate::error::CodeGenError;
use crate::semantic::{KangType, TypedExpr, TypedExprKind};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, IntValue};
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
fn ok<T>(r: std::result::Result<T, inkwell::builder::BuilderError>) -> T {
    r.unwrap()
}

/// 从 CallSiteValue 中提取 BasicValueEnum（非 void 调用）
fn call_val<'ctx>(call: inkwell::values::CallSiteValue<'ctx>) -> BasicValueEnum<'ctx> {
    use inkwell::values::ValueKind;
    match call.try_as_basic_value() {
        ValueKind::Basic(bv) => bv,
        _ => panic!("expected basic value from call"),
    }
}

// ── 字面量 ─────────────────────────────────────────────────────────────────

fn codegen_int_lit<'ctx>(ctx: &CodeGenContext<'ctx>, s: &str) -> Result<BasicValueEnum<'ctx>> {
    let val: i64 = s.parse().map_err(|_| CodeGenError { msg: format!("无效整数: {}", s) })?;
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
    ));

    let len_val = ctx.context.i32_type().const_int(len as u64, true);
    let kstr_type = ctx.kang_type_to_basic(&KangType::Str).into_struct_type();
    Ok(kstr_type.const_named_struct(&[ptr_cast.into(), len_val.into()]).into())
}

fn codegen_bool_lit<'ctx>(ctx: &CodeGenContext<'ctx>, b: bool) -> Result<BasicValueEnum<'ctx>> {
    Ok(ctx.context.bool_type().const_int(b as u64, false).into())
}

fn codegen_ident<'ctx>(ctx: &mut CodeGenContext<'ctx>, name: &str) -> Result<BasicValueEnum<'ctx>> {
    let (ptr, ty) = ctx
        .lookup_var(name)
        .ok_or_else(|| CodeGenError { msg: format!("未定义变量: {}", name) })?;
    let llvm_ty = ctx.kang_type_to_basic(&ty);
    Ok(ok(ctx.builder.build_load(llvm_ty, ptr, name)))
}

// ── 二元运算 ───────────────────────────────────────────────────────────────

fn codegen_binary<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    left: &TypedExpr,
    op: ast::BinOp,
    right: &TypedExpr,
) -> Result<BasicValueEnum<'ctx>> {
    if matches!(left.ty, KangType::Str) || matches!(right.ty, KangType::Str) {
        return codegen_str_concat(ctx, left, right);
    }

    let lhs = codegen_expr(ctx, left)?;
    let rhs = codegen_expr(ctx, right)?;

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
            let l_bool = ok(ctx.builder.build_int_compare(inkwell::IntPredicate::NE, lhs.into_int_value(), ctx.context.bool_type().const_zero(), "and.l"));
            let r_bool = ok(ctx.builder.build_int_compare(inkwell::IntPredicate::NE, rhs.into_int_value(), ctx.context.bool_type().const_zero(), "and.r"));
            Ok(ok(ctx.builder.build_and(l_bool, r_bool, "and")).into())
        }
        ast::BinOp::Or => {
            let l_bool = ok(ctx.builder.build_int_compare(inkwell::IntPredicate::NE, lhs.into_int_value(), ctx.context.bool_type().const_zero(), "or.l"));
            let r_bool = ok(ctx.builder.build_int_compare(inkwell::IntPredicate::NE, rhs.into_int_value(), ctx.context.bool_type().const_zero(), "or.r"));
            Ok(ok(ctx.builder.build_or(l_bool, r_bool, "or")).into())
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
    FI: FnOnce(&inkwell::builder::Builder<'ctx>, IntValue<'ctx>, IntValue<'ctx>) -> IntValue<'ctx>,
    FF: FnOnce(&inkwell::builder::Builder<'ctx>, inkwell::values::FloatValue<'ctx>, inkwell::values::FloatValue<'ctx>) -> inkwell::values::FloatValue<'ctx>,
{
    match lhs {
        BasicValueEnum::IntValue(l) => Ok(int_op(&ctx.builder, l, rhs.into_int_value()).into()),
        BasicValueEnum::FloatValue(l) => Ok(float_op(&ctx.builder, l, rhs.into_float_value()).into()),
        _ => Err(CodeGenError { msg: "算术运算仅支持 i32/f64".into() }),
    }
}

fn codegen_div<'ctx>(
    ctx: &CodeGenContext<'ctx>,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
) -> Result<BasicValueEnum<'ctx>> {
    match lhs {
        BasicValueEnum::IntValue(l) => Ok(ok(ctx.builder.build_int_signed_div(l, rhs.into_int_value(), "div")).into()),
        BasicValueEnum::FloatValue(l) => Ok(ok(ctx.builder.build_float_div(l, rhs.into_float_value(), "div")).into()),
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
        BasicValueEnum::IntValue(l) => Ok(ok(ctx.builder.build_int_compare(int_pred, l, rhs.into_int_value(), "cmp")).into()),
        BasicValueEnum::FloatValue(l) => Ok(ok(ctx.builder.build_float_compare(float_pred, l, rhs.into_float_value(), "cmp")).into()),
        _ => Err(CodeGenError { msg: "比较运算仅支持 i32/f64".into() }),
    }
}

// ── 字符串拼接 ─────────────────────────────────────────────────────────────

fn codegen_str_concat<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    left: &TypedExpr,
    right: &TypedExpr,
) -> Result<BasicValueEnum<'ctx>> {
    let lhs = codegen_expr(ctx, left)?;
    let rhs = codegen_expr(ctx, right)?;
    let lhs_str = convert_to_str(ctx, lhs, &left.ty);
    let rhs_str = convert_to_str(ctx, rhs, &right.ty);

    let lhs_s = lhs_str.into_struct_value();
    let rhs_s = rhs_str.into_struct_value();
    let l_ptr = ok(ctx.builder.build_extract_value(lhs_s, 0, "l.ptr"));
    let l_len = ok(ctx.builder.build_extract_value(lhs_s, 1, "l.len"));
    let r_ptr = ok(ctx.builder.build_extract_value(rhs_s, 0, "r.ptr"));
    let r_len = ok(ctx.builder.build_extract_value(rhs_s, 1, "r.len"));

    let func = ctx.module.get_function("k_str_concat")
        .ok_or_else(|| CodeGenError { msg: "k_str_concat 未声明".into() })?;
    let call = ok(ctx.builder.build_call(func, &[
        l_ptr.into(), l_len.into_int_value().into(),
        r_ptr.into(), r_len.into_int_value().into(),
    ], "concat"));
    Ok(call_val(call))
}

/// 将非 str 值转为 str（调用内置 str() 函数）
fn convert_to_str<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    ty: &KangType,
) -> BasicValueEnum<'ctx> {
    match ty {
        KangType::Str => val,
        KangType::I32 => {
            let func = ctx.module.get_function("k_str_i32").unwrap();
            call_val(ok(ctx.builder.build_call(func, &[val.into_int_value().into()], "to.str")))
        }
        KangType::F64 => {
            let func = ctx.module.get_function("k_str_f64").unwrap();
            call_val(ok(ctx.builder.build_call(func, &[val.into_float_value().into()], "to.str")))
        }
        KangType::Bool => {
            let func = ctx.module.get_function("k_str_bool").unwrap();
            let i32_val = ok(ctx.builder.build_int_z_extend(val.into_int_value(), ctx.context.i32_type(), "bool.i32"));
            call_val(ok(ctx.builder.build_call(func, &[i32_val.into()], "to.str")))
        }
        _ => val,
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
            BasicValueEnum::IntValue(v) => Ok(ok(ctx.builder.build_int_neg(v, "neg")).into()),
            BasicValueEnum::FloatValue(v) => Ok(ok(ctx.builder.build_float_neg(v, "neg")).into()),
            _ => Err(CodeGenError { msg: "取负仅支持 i32/f64".into() }),
        },
        ast::UnaryOp::Not => Ok(ok(ctx.builder.build_int_compare(
            inkwell::IntPredicate::EQ, val.into_int_value(), ctx.context.bool_type().const_zero(), "not",
        )).into()),
    }
}

// ── 函数调用 ───────────────────────────────────────────────────────────────

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

    let arg_values: Vec<BasicValueEnum> = args
        .iter()
        .map(|a| codegen_expr(ctx, a))
        .collect::<Result<Vec<_>>>()?;

    let func = ctx
        .module
        .get_function(func_name)
        .or_else(|| ctx.lookup_func(func_name))
        .unwrap_or_else(|| panic!("函数 {} 应在语义阶段已声明", func_name));

    let llvm_args: Vec<BasicMetadataValueEnum> =
        arg_values.iter().map(|v| (*v).into()).collect();

    let call = ok(ctx.builder.build_call(func, &llvm_args, "call"));
    if return_ty.is_void() {
        Ok(ctx.context.i32_type().const_zero().into())
    } else {
        Ok(call_val(call))
    }
}

fn codegen_builtin_len<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    args: &[TypedExpr],
) -> Result<BasicValueEnum<'ctx>> {
    let arg = codegen_expr(ctx, &args[0])?;
    let struct_val = arg.into_struct_value();
    let len = ok(ctx.builder.build_extract_value(struct_val, 1, "len"));
    Ok(len.into())
}

fn codegen_builtin_push<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    args: &[TypedExpr],
    _return_ty: &KangType,
) -> Result<BasicValueEnum<'ctx>> {
    let arr = codegen_expr(ctx, &args[0])?;
    let elem = codegen_expr(ctx, &args[1])?;

    let arr_struct = arr.into_struct_value();
    let arr_ptr = ok(ctx.builder.build_extract_value(arr_struct, 0, "arr.ptr"));
    let arr_len = ok(ctx.builder.build_extract_value(arr_struct, 1, "arr.len"));

    let elem_llvm_ty = ctx.kang_type_to_basic(&args[1].ty);
    let alloca = ok(ctx.builder.build_alloca(elem_llvm_ty, "elem.alloca"));
    let _ = ctx.builder.build_store(alloca, elem);
    let elem_ptr = ok(ctx.builder.build_pointer_cast(
        alloca,
        ctx.context.ptr_type(AddressSpace::default()),
        "elem.cast",
    ));

    let elem_size = size_of_kang(&args[1].ty);
    let elem_size_val = ctx.context.i32_type().const_int(elem_size as u64, true);

    let func = ctx.module.get_function("k_push").expect("k_push 应在初始化时声明");
    let call = ok(ctx.builder.build_call(func, &[
        arr_ptr.into_int_value().into(),
        arr_len.into_int_value().into(),
        elem_ptr.into(),
        elem_size_val.into(),
    ], "push"));
    Ok(call_val(call))
}

pub fn size_of_kang(ty: &KangType) -> i32 {
    match ty {
        KangType::I32 | KangType::Bool => 4,
        KangType::F64 => 8,
        KangType::Str | KangType::Array(_) | KangType::Struct(_) | KangType::Pair(_, _) => 16,
        KangType::Void => 0,
    }
}

// ── 索引 ───────────────────────────────────────────────────────────────────

fn codegen_index<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    array: &TypedExpr,
    index: &TypedExpr,
    result_ty: &KangType,
) -> Result<BasicValueEnum<'ctx>> {
    let arr_val = codegen_expr(ctx, array)?;
    let idx_val = codegen_expr(ctx, index)?;

    let arr_struct = arr_val.into_struct_value();
    let arr_ptr = ok(ctx.builder.build_extract_value(arr_struct, 0, "arr.ptr"));
    let idx = idx_val.into_int_value();

    let elem_size = size_of_kang(result_ty);
    let offset = ok(ctx.builder.build_int_mul(
        idx,
        ctx.context.i32_type().const_int(elem_size as u64, true),
        "offset",
    ));

    let base_addr = ok(ctx.builder.build_int_add(
        arr_ptr.into_int_value(),
        ctx.context.i32_type().const_int(4u64, true),
        "base",
    ));
    let elem_addr = ok(ctx.builder.build_int_add(base_addr, offset, "elem.addr"));
    let elem_ptr = ok(ctx.builder.build_int_to_ptr(
        elem_addr,
        ctx.context.ptr_type(AddressSpace::default()),
        "elem.ptr",
    ));
    Ok(ok(ctx.builder.build_load(ctx.kang_type_to_basic(result_ty), elem_ptr, "elem")))
}

// ── 字段访问 ──────────────────────────────────────────────────────────────

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
    let field_val = ok(ctx.builder.build_extract_value(struct_val, field_idx as u32, field));
    Ok(field_val.into())
}

// ── 数组字面量 ─────────────────────────────────────────────────────────────

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
    let elem_bytes = size_of_kang(elem_ty) as usize;
    let total_bytes = 4 + (elem_count as usize) * elem_bytes;

    let size_val = ctx.context.i32_type().const_int(total_bytes as u64, true);
    let align_val = ctx.context.i32_type().const_int(8u64, true);

    let alloc_func = ctx.module.get_function("k_arena_alloc_aligned").unwrap_or_else(|| {
        let _ptr_ty: inkwell::types::BasicMetadataTypeEnum =
            ctx.context.ptr_type(AddressSpace::default()).into();
        let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
        let fn_type = ctx
            .context
            .ptr_type(AddressSpace::default())
            .fn_type(&[i32_ty, i32_ty], false);
        ctx.module.add_function("k_arena_alloc_aligned", fn_type, None)
    });

    let alloc_call = ok(ctx.builder.build_call(alloc_func, &[size_val.into(), align_val.into()], "arr.alloc"));
    let raw_ptr = call_val(alloc_call).into_pointer_value();

    // 写入长度头
    let len_val = ctx.context.i32_type().const_int(elem_count as u64, true);
    let len_ptr = ok(ctx.builder.build_pointer_cast(
        raw_ptr,
        ctx.context.ptr_type(AddressSpace::default()),
        "len.ptr",
    ));
    let _ = ctx.builder.build_store(len_ptr, len_val);

    // 写入每个元素
    for (i, elem) in elems.iter().enumerate() {
        let elem_val = codegen_expr(ctx, elem)?;
        let offset = 4 + (i * elem_bytes);
        let offset_val = ctx.context.i64_type().const_int(offset as u64, false);
        let raw_int = ok(ctx.builder.build_ptr_to_int(raw_ptr, ctx.context.i64_type(), "raw.int"));
        let elem_int = ok(ctx.builder.build_int_add(raw_int, offset_val, "elem.offset"));
        let elem_ptr = ok(ctx.builder.build_int_to_ptr(
            elem_int,
            ctx.context.ptr_type(AddressSpace::default()),
            "elem.ptr",
        ));
        let typed_elem_ptr = ok(ctx.builder.build_pointer_cast(
            elem_ptr,
            ctx.context.ptr_type(AddressSpace::default()),
            "typed.elem",
        ));
        let _ = ctx.builder.build_store(typed_elem_ptr, elem_val);
    }

    let kptrlen_type = ctx.kang_type_to_basic(array_ty).into_struct_type();
    let count_val = ctx.context.i32_type().const_int(elem_count as u64, true);
    Ok(kptrlen_type.const_named_struct(&[raw_ptr.into(), count_val.into()]).into())
}

// ── 结构体字面量 ───────────────────────────────────────────────────────────

fn codegen_struct_lit<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    name: &str,
    fields: &[(String, TypedExpr)],
) -> Result<BasicValueEnum<'ctx>> {
    let _struct_type = ctx
        .lookup_struct_type(name)
        .ok_or_else(|| CodeGenError { msg: format!("未定义的结构体: {}", name) })?;

    let field_defs = ctx
        .struct_fields
        .get(name)
        .ok_or_else(|| CodeGenError { msg: format!("未定义的结构体: {}", name) })?
        .clone(); // 克隆以释放 ctx 的不可变借用

    let mut field_values: Vec<BasicValueEnum> =
        vec![ctx.default_value(&field_defs[0].1); field_defs.len()];

    for (i, (fname, fty)) in field_defs.iter().enumerate() {
        if let Some((_, fexpr)) = fields.iter().find(|(n, _)| n == fname) {
            field_values[i] = codegen_expr(ctx, fexpr)?;
        } else {
            field_values[i] = ctx.default_value(fty);
        }
    }

    Ok(_struct_type.const_named_struct(&field_values).into())
}
