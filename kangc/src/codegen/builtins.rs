// 内置函数声明 — 将所有 extern "C" fn 注册到 LLVM Module
// 函数签名与 kangrt C ABI 一致，Kang 类型按 FFI 约定映射为 LLVM 类型
//
// ABI 注意事项:
// Apple ARM64 上 LLVM 将 struct 字段拆分为独立寄存器返回,
// 但 rustc (kangrt) 遵循标准 AAPCS64 — 将字段打包到 8 字节块中。
// 为避免 ABI 不匹配, 将多字段返回类型声明为打包形式:
//   {i32, i32} → i64        (rustc 返回 (ok << 32) | val)
//   {ptr, i32, i32} → {ptr, i64}  (rustc 返回 ptr 在 x0, (ok<<32)|len 在 x1)
//   {f64, i32} → {f64, i64}  (f64 在 x0, ok sign-ext 到 64 位在 x1)

use super::context::CodeGenContext;
use inkwell::AddressSpace;

/// 注册所有内置函数到 module（22 个: 21 个 kangrt + k_panic）
pub fn declare_all(ctx: &mut CodeGenContext) {
    declare_k_panic(ctx);
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

// 注意: Apple ARM64 上 LLVM 将 struct 字段拆分为独立寄存器返回,
// 但 rustc (kangrt) 遵循标准 AAPCS64 — 将字段打包到 8 字节块中。
// 为避免 ABI 不匹配, 将多字段返回类型声明为打包形式:
//   {i32, i32} → i64        (rustc 返回 (ok << 32) | val)
//   {ptr, i32, i32} → {ptr, i64}  (rustc 返回 ptr 在 x0, (ok<<32)|len 在 x1)
//   {f64, i32} → {f64, i64}  (f64 在 x0, ok sign-ext 到 64 位在 x1)
fn kstr_bool_type<'ctx>(ctx: &CodeGenContext<'ctx>) -> inkwell::types::StructType<'ctx> {
    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
    let i64_ty = ctx.context.i64_type();
    ctx.context.struct_type(&[ptr_ty.into(), i64_ty.into()], false)
}

fn ki32_bool_type<'ctx>(ctx: &CodeGenContext<'ctx>) -> inkwell::types::IntType<'ctx> {
    ctx.context.i64_type()
}

fn kf64_bool_type<'ctx>(ctx: &CodeGenContext<'ctx>) -> inkwell::types::StructType<'ctx> {
    let f64_ty = ctx.context.f64_type();
    let i64_ty = ctx.context.i64_type();
    ctx.context.struct_type(&[f64_ty.into(), i64_ty.into()], false)
}

fn kbool_bool_type<'ctx>(ctx: &CodeGenContext<'ctx>) -> inkwell::types::IntType<'ctx> {
    ctx.context.i64_type()
}

fn kstr_params<'ctx>(ctx: &CodeGenContext<'ctx>) -> Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> {
    let ptr_ty: inkwell::types::BasicMetadataTypeEnum =
        ctx.context.ptr_type(AddressSpace::default()).into();
    let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
    vec![ptr_ty, i32_ty]
}

// ── 运行时 panic ───────────────────────────────────────────────────────────

fn declare_k_panic(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ctx.context.void_type().fn_type(&params, false);
    let func = ctx.module.add_function("k_panic", fn_type, None);
    // 存入 context，消除后续按名称查找的时序依赖
    ctx.panic_func = Some(func);
}

// ── 集合操作 ──────────────────────────────────────────────────────────────

/// 在 module 中声明 LLVM 函数，并注册 kang_name → LLVM function 的别名映射。
/// 用于内置函数在 Kang 源码中以 kang_name 调用，LLVM IR 中使用 llvm_name。
fn declare_with_alias<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    llvm_name: &str,
    kang_name: &str,
    fn_type: inkwell::types::FunctionType<'ctx>,
) {
    let func = ctx.module.add_function(llvm_name, fn_type, None);
    ctx.register_func_alias(kang_name, func);
}

fn declare_len_str(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ctx.context.i32_type().fn_type(&params, false);
    declare_with_alias(ctx, "k_len_str", "len", fn_type);
}

fn declare_push(ctx: &mut CodeGenContext) {
    let ptr_ty: inkwell::types::BasicMetadataTypeEnum =
        ctx.context.ptr_type(AddressSpace::default()).into();
    let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
    let params = vec![ptr_ty, i32_ty, ptr_ty, i32_ty];
    let fn_type = kptrlen_type(ctx).fn_type(&params, false);
    declare_with_alias(ctx, "k_push", "push", fn_type);
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
    declare_with_alias(ctx, "k_puts", "puts", fn_type);
}

fn declare_print(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ctx.context.void_type().fn_type(&params, false);
    declare_with_alias(ctx, "k_print", "print", fn_type);
}

fn declare_eprint(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ctx.context.void_type().fn_type(&params, false);
    declare_with_alias(ctx, "k_eprint", "eprint", fn_type);
}

// ── 文件 I/O ───────────────────────────────────────────────────────────────

fn declare_read_file(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = kstr_bool_type(ctx).fn_type(&params, false);
    declare_with_alias(ctx, "k_read_file", "read_file", fn_type);
}

fn declare_read_line(ctx: &mut CodeGenContext) {
    let params: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![];
    let fn_type = kstr_bool_type(ctx).fn_type(&params, false);
    declare_with_alias(ctx, "k_read_line", "read_line", fn_type);
}

fn declare_write_file(ctx: &mut CodeGenContext) {
    let mut params = kstr_params(ctx);
    params.extend(kstr_params(ctx));
    let fn_type = ctx.context.void_type().fn_type(&params, false);
    declare_with_alias(ctx, "k_write_file", "write_file", fn_type);
}

fn declare_append_file(ctx: &mut CodeGenContext) {
    let mut params = kstr_params(ctx);
    params.extend(kstr_params(ctx));
    let fn_type = ctx.context.void_type().fn_type(&params, false);
    declare_with_alias(ctx, "k_append_file", "append_file", fn_type);
}

fn declare_file_exists(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ctx.context.i32_type().fn_type(&params, false);
    declare_with_alias(ctx, "k_file_exists", "file_exists", fn_type);
}

fn declare_file_size(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ki32_bool_type(ctx).fn_type(&params, false);
    declare_with_alias(ctx, "k_file_size", "file_size", fn_type);
}

// ── 类型转换 ──────────────────────────────────────────────────────────────

fn declare_str_i32(ctx: &mut CodeGenContext) {
    let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
    let fn_type = kstr_type(ctx).fn_type(&[i32_ty], false);
    declare_with_alias(ctx, "k_str_i32", "str_i32", fn_type);
}

fn declare_str_f64(ctx: &mut CodeGenContext) {
    let f64_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.f64_type().into();
    let fn_type = kstr_type(ctx).fn_type(&[f64_ty], false);
    declare_with_alias(ctx, "k_str_f64", "str_f64", fn_type);
}

fn declare_str_bool(ctx: &mut CodeGenContext) {
    let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
    let fn_type = kstr_type(ctx).fn_type(&[i32_ty], false);
    declare_with_alias(ctx, "k_str_bool", "str_bool", fn_type);
}

fn declare_i32_str(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = ki32_bool_type(ctx).fn_type(&params, false);
    declare_with_alias(ctx, "k_i32_str", "i32_str", fn_type);
}

fn declare_i32_f64(ctx: &mut CodeGenContext) {
    let f64_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.f64_type().into();
    let fn_type = ctx.context.i32_type().fn_type(&[f64_ty], false);
    declare_with_alias(ctx, "k_i32_f64", "i32_f64", fn_type);
}

fn declare_f64_str(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = kf64_bool_type(ctx).fn_type(&params, false);
    declare_with_alias(ctx, "k_f64_str", "f64_str", fn_type);
}

fn declare_f64_i32(ctx: &mut CodeGenContext) {
    let i32_ty: inkwell::types::BasicMetadataTypeEnum = ctx.context.i32_type().into();
    let fn_type = ctx.context.f64_type().fn_type(&[i32_ty], false);
    declare_with_alias(ctx, "k_f64_i32", "f64_i32", fn_type);
}

fn declare_bool_str(ctx: &mut CodeGenContext) {
    let params = kstr_params(ctx);
    let fn_type = kbool_bool_type(ctx).fn_type(&params, false);
    declare_with_alias(ctx, "k_bool_str", "bool_str", fn_type);
}
