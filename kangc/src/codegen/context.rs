// CodeGenContext — 封装 inkwell LLVM 上下文、模块、Builder
// 管理变量符号表、结构体类型定义、运行时检查计数

use crate::semantic::KangType;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::BasicType;
use inkwell::types::BasicTypeEnum;
use inkwell::types::StructType;
use inkwell::values::BasicValueEnum;
use inkwell::values::FunctionValue;
use inkwell::values::PointerValue;
use inkwell::AddressSpace;
use std::collections::HashMap;

/// 变量存储信息
struct VarInfo<'ctx> {
    ptr: PointerValue<'ctx>,
    ty: KangType,
}

/// 代码生成上下文，封装一次编译所需的所有 LLVM 状态
pub struct CodeGenContext<'ctx> {
    pub context: &'ctx Context,
    pub module: Module<'ctx>,
    pub builder: Builder<'ctx>,

    // 变量作用域栈，每层是 name → (alloca_ptr, type) 的映射
    scopes: Vec<HashMap<String, VarInfo<'ctx>>>,
    // 结构体定义: name → LLVM struct type
    pub struct_types: HashMap<String, StructType<'ctx>>,
    // 结构体字段信息: name → [(field_name, KangType)]
    pub struct_fields: HashMap<String, Vec<(String, KangType)>>,
    // 已声明的外部函数
    declared_funcs: HashMap<String, FunctionValue<'ctx>>,
    // 运行时检查计数
    pub runtime_checks: usize,
}

impl<'ctx> CodeGenContext<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();

        CodeGenContext {
            context,
            module,
            builder,
            scopes: vec![HashMap::new()],
            struct_types: HashMap::new(),
            struct_fields: HashMap::new(),
            declared_funcs: HashMap::new(),
            runtime_checks: 0,
        }
    }

    // ── 作用域管理 ──────────────────────────────────────────────────────────

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// 在当前作用域注册变量
    pub fn register_var(&mut self, name: &str, ptr: PointerValue<'ctx>, ty: KangType) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), VarInfo { ptr, ty });
        }
    }

    /// 查找变量的 alloca 指针（从内层向外层）
    pub fn lookup_var(&self, name: &str) -> Option<(PointerValue<'ctx>, KangType)> {
        for scope in self.scopes.iter().rev() {
            if let Some(info) = scope.get(name) {
                return Some((info.ptr, info.ty.clone()));
            }
        }
        None
    }

    // ── 结构体 ──────────────────────────────────────────────────────────────

    /// 注册结构体定义，返回 LLVM struct type
    pub fn register_struct(&mut self, name: &str, fields: &[(String, KangType)]) -> StructType<'ctx> {
        let field_types: Vec<BasicTypeEnum> = fields
            .iter()
            .map(|(_, ty)| self.kang_type_to_basic(ty))
            .collect();

        let struct_type = self.context.opaque_struct_type(name);
        struct_type.set_body(&field_types, false);

        self.struct_types.insert(name.to_string(), struct_type);
        self.struct_fields.insert(name.to_string(), fields.to_vec());

        struct_type
    }

    /// 查找已定义的结构体类型
    pub fn lookup_struct_type(&self, name: &str) -> Option<StructType<'ctx>> {
        self.struct_types.get(name).copied()
    }

    // ── 函数声明 ────────────────────────────────────────────────────────────

    /// 声明一个函数，返回 FunctionValue
    pub fn declare_func(
        &mut self,
        name: &str,
        param_kang_types: &[KangType],
        return_kang_type: &KangType,
    ) -> FunctionValue<'ctx> {
        if let Some(func) = self.declared_funcs.get(name) {
            return *func;
        }

        let llvm_params: Vec<inkwell::types::BasicMetadataTypeEnum> = param_kang_types
            .iter()
            .map(|t| self.kang_type_to_basic(t).into())
            .collect();

        let fn_type = if return_kang_type.is_void() {
            self.context.void_type().fn_type(&llvm_params, false)
        } else {
            self.kang_type_to_basic(return_kang_type).fn_type(&llvm_params, false)
        };

        let func = self.module.add_function(name, fn_type, None);
        self.declared_funcs.insert(name.to_string(), func);
        func
    }

    /// 注册外部函数的别名映射（Kang 名称 → LLVM 名称）
    pub fn register_func_alias(&mut self, kang_name: &str, llvm_func: FunctionValue<'ctx>) {
        self.declared_funcs.insert(kang_name.to_string(), llvm_func);
    }

    /// 查找已声明的函数，优先精确匹配，其次尝试 k_ 前缀（内置函数）
    pub fn lookup_func(&self, name: &str) -> Option<FunctionValue<'ctx>> {
        self.declared_funcs.get(name).copied()
            .or_else(|| {
                let k_name = format!("k_{}", name);
                self.declared_funcs.get(&k_name).copied()
            })
    }

    // ── 类型映射 ────────────────────────────────────────────────────────────

    /// 将 Kang 类型映射为 LLVM 基本类型（void 不可映射，由调用者处理）
    pub fn kang_type_to_basic(&self, ty: &KangType) -> BasicTypeEnum<'ctx> {
        match ty {
            KangType::I32 => self.context.i32_type().into(),
            KangType::F64 => self.context.f64_type().into(),
            KangType::Bool => self.context.bool_type().into(),
            KangType::Void => panic!("void 不可映射为 LLVM 基本类型"),
            KangType::Str => {
                // str → {i8*, i32}
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let fields = vec![ptr_type.into(), self.context.i32_type().into()];
                self.context.struct_type(&fields, false).into()
            }
            KangType::Array(_) => {
                // [T] → {i8*, i32} 堆分配数组
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let fields = vec![ptr_type.into(), self.context.i32_type().into()];
                self.context.struct_type(&fields, false).into()
            }
            KangType::Struct(name) => {
                if let Some(st) = self.struct_types.get(name) {
                    (*st).into()
                } else {
                    let st = self.context.opaque_struct_type(name);
                    st.into()
                }
            }
            KangType::Pair(t1, t2) => {
                let f1 = self.kang_type_to_basic(t1);
                let f2 = self.kang_type_to_basic(t2);
                let fields = vec![f1, f2];
                self.context.struct_type(&fields, false).into()
            }
        }
    }

    /// 获取类型的 LLVM 大小（字节），用于 alloca
    pub fn size_of(&self, ty: &KangType) -> u32 {
        match ty {
            KangType::I32 => 4,
            KangType::F64 => 8,
            KangType::Bool => 1,
            KangType::Str | KangType::Array(_) => 16, // {ptr, i32} = 12 + padding → 16 on 64-bit
            KangType::Struct(name) => {
                let fields = self.struct_fields.get(name)
                    .expect("结构体应在代码生成前已注册");
                let mut total = 0u32;
                for (_, fty) in fields {
                    total += self.size_of(fty);
                }
                // 简化对齐: 8 字节对齐
                (total + 7) / 8 * 8
            }
            KangType::Pair(t1, t2) => {
                let s1 = self.size_of(t1);
                let s2 = self.size_of(t2);
                // 对齐到最大字段的倍数
                let align = s1.max(s2);
                ((s1 + s2 + align - 1) / align) * align
            }
            KangType::Void => 0,
        }
    }

    /// 获取类型的默认值（零初始化），void 不可用
    pub fn default_value(&self, ty: &KangType) -> BasicValueEnum<'ctx> {
        match ty {
            KangType::I32 => self.context.i32_type().const_zero().into(),
            KangType::F64 => self.context.f64_type().const_zero().into(),
            KangType::Bool => self.context.bool_type().const_zero().into(),
            KangType::Str | KangType::Array(_) => {
                let llvm_type = self.kang_type_to_basic(ty);
                llvm_type.into_struct_type().const_zero().into()
            }
            KangType::Struct(name) => {
                let st = self.struct_types
                    .get(name)
                    .expect("结构体类型应在代码生成前已注册");
                st.const_zero().into()
            }
            KangType::Pair(_, _) => {
                // Pair 总是从函数返回值打包/解包，默认值用零
                self.kang_type_to_basic(ty).into_struct_type().const_zero().into()
            }
            KangType::Void => panic!("void 类型无默认值"),
        }
    }
}
