// 内置函数声明 — 将所有 21 个 extern "C" fn 注册到 LLVM Module
// 函数签名与 kangrt C ABI 一致，Kang 类型按 FFI 约定映射为 LLVM 类型

use super::context::CodeGenContext;
use inkwell::AddressSpace;

/// 注册所有 21 个内置函数到 module
pub fn declare_all(ctx: &mut CodeGenContext) {
    declare_len_str(ctx);
    declare_push(ctx);
    declare_str_concat(ctx);
    declare_arena_alloc(ctx);
    declare_puts(ctx);
    declare_print(ctx);
    declare_eprint(ctx);
    declare_read_file(ctx);
    declare_read_line(ctx);
    declare_write_file(ctx);
    declare_append_file(ctx);
    declare_file_exists(ctx);
    declare_file_size(ctx);
    declare_str_i32(ctx);
    declare_str_f64(ctx);
    declare_str_bool(ctx);
    declare_i32_str(ctx);
    declare_i32_f64(ctx);
    declare_f64_str(ctx);
    declare_f64_i32(ctx);
    declare_bool_str(ctx);
}

// ── 辅助类型构造 ──────────────────────────────────────────────────────────

fn kstr_type<'ctx>(ctx: &CodeGenContext<'ctx>) -> inkwell::types::StructType<'ctx> {
    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
    let i32_ty = ctx.context.i32_type();
    ctx.context.struct_type(&[ptr_ty.into(), i32_ty.into()], false)
}

fn kptrlen_type<'ctx>(ctx: &CodeGenContext<'ctx>) -> inkwell::types::StructType<'ctx> {
    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
    let i32_ty = ctx.context.i32_type();
    ctx.context.struct_type(&[ptr_ty.into(), i32_ty.into()], false)
}

fn kstr_bool_type<'ctx>(ctx: &CodeGenContext<'ctx>) -> inkwell::types::StructType<'ctx> {
    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
    let i32_ty = ctx.context.i32_type();
    ctx.context.struct_type(&[ptr_ty.into(), i32_ty.into(), i32_ty.into()], false)
}

fn ki32_bool_type<'ctx>(ctx: &CodeGenContext<'ctx>) -> inkwell::types::StructType<'ctx> {
    let i32_ty = ctx.context.i32_type();
    ctx.context.struct_type(&[i32_ty.into(), i32_ty.into()], false)
}

fn kf64_bool_type<'ctx>(ctx: &CodeGenContext<'ctx>) -> inkwell::types::StructType<'ctx> {
    let f64_ty = ctx.context.f64_type();
    let i32_ty = ctx.context.i32_type();
    ctx.context.struct_type(&[f64_ty.into(), i32_ty.into()], false)
}

fn kbool_bool_type<'ctx>(ctx: &CodeGenContext<'ctx>) -> inkwell::types::StructType<'ctx> {
    let i32_ty = ctx.context.i32_type();
    ctx.context.struct_type(&[i32_ty.into(), i32_ty.into()], false)
}

fn kstr_params<'ctx>(ctx: &CodeGenContext<'ctx>) -> Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> {
    let ptr_ty: inkwell::types::BasicMetadataTypeEnum =
        ctx.context.ptr_type(AddressSpace::default()).into();
    let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
    vec![ptr_ty, i32_ty]
}

// ── 集合操作 ──────────────────────────────────────────────────────────────

fn declare_len_str(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ctx.context.i32_type().fn_type(&params, false);
    ctx.module.add_function("k_len_str", fn_type, None);
}

fn declare_push(ctx: &mut CodeGenContext) {
    let ptr_ty: inkwell::types::BasicMetadataTypeEnum =
        ctx.context.ptr_type(AddressSpace::default()).into();
    let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
    let params = vec![ptr_ty, i32_ty, ptr_ty, i32_ty];
    let fn_type = kptrlen_type(ctx).fn_type(&params, false);
    ctx.module.add_function("k_push", fn_type, None);
}

// ── 字符串操作 ──────────────────────────────────────────────────────────

fn declare_str_concat(ctx: &mut CodeGenContext) {
    let ptr_ty: inkwell::types::BasicMetadataTypeEnum =
        ctx.context.ptr_type(AddressSpace::default()).into();
    let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
    let params = vec![ptr_ty, i32_ty, ptr_ty, i32_ty];
    let fn_type = kptrlen_type(ctx).fn_type(&params, false);
    ctx.module.add_function("k_str_concat", fn_type, None);
}

// ── 内存管理 ──────────────────────────────────────────────────────────────

fn declare_arena_alloc(ctx: &mut CodeGenContext) {
    let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
    let fn_type = ptr_ty.fn_type(&[i32_ty, i32_ty], false);
    ctx.module.add_function("k_arena_alloc_aligned", fn_type, None);
}

// ── 输出 ──────────────────────────────────────────────────────────────────

fn declare_puts(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ctx.context.void_type().fn_type(&params, false);
    ctx.module.add_function("k_puts", fn_type, None);
}

fn declare_print(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ctx.context.void_type().fn_type(&params, false);
    ctx.module.add_function("k_print", fn_type, None);
}

fn declare_eprint(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ctx.context.void_type().fn_type(&params, false);
    ctx.module.add_function("k_eprint", fn_type, None);
}

// ── 文件 I/O ───────────────────────────────────────────────────────────────

fn declare_read_file(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = kstr_bool_type(ctx).fn_type(&params, false);
    ctx.module.add_function("k_read_file", fn_type, None);
}

fn declare_read_line(ctx: &mut CodeGenContext) {
    let params: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![];
    let fn_type = kstr_bool_type(ctx).fn_type(&params, false);
    ctx.module.add_function("k_read_line", fn_type, None);
}

fn declare_write_file(ctx: &mut CodeGenContext) {
    let mut params = kstr_params(ctx);
    params.extend(kstr_params(ctx));
    let fn_type = ctx.context.void_type().fn_type(&params, false);
    ctx.module.add_function("k_write_file", fn_type, None);
}

fn declare_append_file(ctx: &mut CodeGenContext) {
    let mut params = kstr_params(ctx);
    params.extend(kstr_params(ctx));
    let fn_type = ctx.context.void_type().fn_type(&params, false);
    ctx.module.add_function("k_append_file", fn_type, None);
}

fn declare_file_exists(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ctx.context.i32_type().fn_type(&params, false);
    ctx.module.add_function("k_file_exists", fn_type, None);
}

fn declare_file_size(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ki32_bool_type(ctx).fn_type(&params, false);
    ctx.module.add_function("k_file_size", fn_type, None);
}

// ── 类型转换 ──────────────────────────────────────────────────────────────

fn declare_str_i32(ctx: &mut CodeGenContext) {
    let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
    let fn_type = kstr_type(ctx).fn_type(&[i32_ty], false);
    ctx.module.add_function("k_str_i32", fn_type, None);
}

fn declare_str_f64(ctx: &mut CodeGenContext) {
    let f64_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.f64_type().into();
    let fn_type = kstr_type(ctx).fn_type(&[f64_ty], false);
    ctx.module.add_function("k_str_f64", fn_type, None);
}

fn declare_str_bool(ctx: &mut CodeGenContext) {
    let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
    let fn_type = kstr_type(ctx).fn_type(&[i32_ty], false);
    ctx.module.add_function("k_str_bool", fn_type, None);
}

fn declare_i32_str(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ki32_bool_type(ctx).fn_type(&params, false);
    ctx.module.add_function("k_i32_str", fn_type, None);
}

fn declare_i32_f64(ctx: &mut CodeGenContext) {
    let f64_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.f64_type().into();
    let fn_type = ctx.context.i32_type().fn_type(&[f64_ty], false);
    ctx.module.add_function("k_i32_f64", fn_type, None);
}

fn declare_f64_str(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = kf64_bool_type(ctx).fn_type(&params, false);
    ctx.module.add_function("k_f64_str", fn_type, None);
}

fn declare_f64_i32(ctx: &mut CodeGenContext) {
    let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
    let fn_type = ctx.context.f64_type().fn_type(&[i32_ty], false);
    ctx.module.add_function("k_f64_i32", fn_type, None);
}

fn declare_bool_str(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = kbool_bool_type(ctx).fn_type(&params, false);
    ctx.module.add_function("k_bool_str", fn_type, None);
}
