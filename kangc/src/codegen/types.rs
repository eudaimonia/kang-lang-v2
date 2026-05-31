// Kang 类型到 LLVM 类型的映射、大小计算、默认值

use super::context::CodeGenContext;
use crate::semantic::KangType;
use inkwell::types::BasicTypeEnum;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

/// 将 Kang 类型映射为 LLVM 基本类型（void 不可映射，由调用者处理）
pub fn kang_type_to_basic<'ctx>(ctx: &CodeGenContext<'ctx>, ty: &KangType) -> BasicTypeEnum<'ctx> {
    match ty {
        KangType::I32 => ctx.context.i32_type().into(),
        KangType::F64 => ctx.context.f64_type().into(),
        KangType::Bool => ctx.context.bool_type().into(),
        KangType::Void => panic!("void 不可映射为 LLVM 基本类型 — 语义检查应拦截"),
        KangType::Str => {
            let ptr_type = ctx.context.ptr_type(AddressSpace::default());
            let fields = vec![ptr_type.into(), ctx.context.i32_type().into()];
            ctx.context.struct_type(&fields, false).into()
        }
        KangType::Array(_) => {
            let ptr_type = ctx.context.ptr_type(AddressSpace::default());
            let fields = vec![ptr_type.into(), ctx.context.i32_type().into()];
            ctx.context.struct_type(&fields, false).into()
        }
        KangType::Struct(name) => {
            if let Some(st) = ctx.struct_types.get(name) {
                (*st).into()
            } else {
                // 回退为字面量 struct（而非 opaque），保持与注册类型一致的布局
                let fields: Vec<BasicTypeEnum> = ctx
                    .struct_fields
                    .get(name)
                    .map(|fs| fs.iter().map(|(_, t)| kang_type_to_basic(ctx, t)).collect())
                    .unwrap_or_default();
                ctx.context.struct_type(&fields, false).into()
            }
        }
        KangType::Pair(t1, t2) => {
            let f1 = kang_type_to_basic(ctx, t1);
            let f2 = kang_type_to_basic(ctx, t2);
            let fields = vec![f1, f2];
            ctx.context.struct_type(&fields, false).into()
        }
    }
}

/// 获取类型的对齐要求（字节）
fn alignment_of(ctx: &CodeGenContext, ty: &KangType) -> u32 {
    match ty {
        KangType::I32 => 4,
        KangType::F64 => 8,
        KangType::Bool => 1,
        KangType::Str | KangType::Array(_) => 8,
        KangType::Struct(name) => {
            let fields = ctx.struct_fields
                .get(name)
                .expect("结构体应在代码生成前已注册");
            fields.iter()
                .map(|(_, fty)| alignment_of(ctx, fty))
                .max()
                .unwrap_or(8)
        }
        KangType::Pair(t1, t2) => alignment_of(ctx, t1).max(alignment_of(ctx, t2)),
        KangType::Void => 1,
    }
}

/// 获取 Kang 类型对应的 LLVM 存储大小（字节）
pub fn size_of(ctx: &CodeGenContext, ty: &KangType) -> u32 {
    match ty {
        KangType::I32 => 4,
        KangType::F64 => 8,
        KangType::Bool => 1,
        KangType::Str | KangType::Array(_) => 16,
        KangType::Struct(name) => {
            let fields = ctx.struct_fields
                .get(name)
                .expect("结构体应在代码生成前已注册");
            let mut total = 0u32;
            for (_, fty) in fields {
                let align = alignment_of(ctx, fty);
                total = (total + align - 1) / align * align;
                total += size_of(ctx, fty);
            }
            let struct_align = fields.iter()
                .map(|(_, fty)| alignment_of(ctx, fty))
                .max()
                .unwrap_or(8);
            (total + struct_align - 1) / struct_align * struct_align
        }
        KangType::Pair(t1, t2) => {
            let s1 = size_of(ctx, t1);
            let s2 = size_of(ctx, t2);
            let a2 = alignment_of(ctx, t2);
            // 第二个字段按自身对齐要求放置
            let offset_s2 = (s1 + a2 - 1) / a2 * a2;
            let total = offset_s2 + s2;
            let align = alignment_of(ctx, t1).max(a2);
            (total + align - 1) / align * align
        }
        KangType::Void => 0,
    }
}

/// 获取 Kang 类型的 LLVM 零值
pub fn default_value<'ctx>(ctx: &CodeGenContext<'ctx>, ty: &KangType) -> BasicValueEnum<'ctx> {
    match ty {
        KangType::I32 => ctx.context.i32_type().const_zero().into(),
        KangType::F64 => ctx.context.f64_type().const_zero().into(),
        KangType::Bool => ctx.context.bool_type().const_zero().into(),
        KangType::Str | KangType::Array(_) => {
            let llvm_type = kang_type_to_basic(ctx, ty);
            llvm_type.into_struct_type().const_zero().into()
        }
        KangType::Struct(name) => {
            let st = ctx.struct_types
                .get(name)
                .expect("结构体类型应在代码生成前已注册");
            st.const_zero().into()
        }
        KangType::Pair(_, _) => {
            kang_type_to_basic(ctx, ty).into_struct_type().const_zero().into()
        }
        KangType::Void => panic!("void 类型无默认值 — 语义检查应拦截"),
    }
}
