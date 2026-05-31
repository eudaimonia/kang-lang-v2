// CodeGenContext — 封装 inkwell LLVM 上下文、模块、Builder
// 管理变量符号表、结构体类型定义、运行时检查计数

use super::types;
use crate::semantic::KangType;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{InitializationConfig, Target, TargetData, TargetMachine, TargetTriple};
use inkwell::types::BasicType;
use inkwell::types::BasicTypeEnum;
use inkwell::types::StructType;
use inkwell::values::BasicValueEnum;
use inkwell::values::FunctionValue;
use inkwell::values::PointerValue;
use std::collections::{HashMap, HashSet};
use std::sync::Once;

/// 变量存储信息
struct VarInfo<'ctx> {
    ptr: PointerValue<'ctx>,
    ty: KangType,
}

/// 确保 LLVM native target 已初始化（只执行一次）
static INIT_TARGET: Once = Once::new();

fn ensure_target_initialized() {
    INIT_TARGET.call_once(|| {
        Target::initialize_native(&InitializationConfig::default())
            .expect("failed to initialize native LLVM target");
    });
}

/// 代码生成上下文，封装一次编译所需的所有 LLVM 状态
pub struct CodeGenContext<'ctx> {
    pub context: &'ctx Context,
    pub module: Module<'ctx>,
    pub builder: Builder<'ctx>,
    pub target_triple: String,

    // 变量作用域栈，每层是 name → (alloca_ptr, type) 的映射
    scopes: Vec<HashMap<String, VarInfo<'ctx>>>,
    // 结构体定义: name → LLVM struct type
    pub struct_types: HashMap<String, StructType<'ctx>>,
    // 结构体字段信息: name → [(field_name, KangType)]
    pub struct_fields: HashMap<String, Vec<(String, KangType)>>,
    // 已声明的外部函数
    declared_funcs: HashMap<String, FunctionValue<'ctx>>,
    // Kang 跨模块函数名集合 — 这些函数不应走 C ABI 参数展平
    pub kang_funcs: HashSet<String>,
    // 目标平台数据布局信息（用于类型大小/对齐计算）
    pub target_data: Option<TargetData>,
    // k_panic 函数指针（由 builtins::declare_k_panic 设置，消除按名称查找的时序依赖）
    pub panic_func: Option<FunctionValue<'ctx>>,
    // 运行时检查计数
    pub runtime_checks: usize,
}

impl<'ctx> CodeGenContext<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str, target_triple: Option<&str>) -> Self {
        ensure_target_initialized();

        let module = context.create_module(module_name);
        let builder = context.create_builder();

        let resolved_triple = match target_triple {
            Some(t) => {
                let triple = TargetTriple::create(t);
                module.set_triple(&triple);
                t.to_string()
            }
            None => {
                let default_triple = TargetMachine::get_default_triple();
                module.set_triple(&default_triple);
                default_triple.as_str().to_str().unwrap_or("unknown").to_string()
            }
        };

        // 设置 data layout 并获取 TargetData（用于类型大小/对齐计算）
        let target_data = if let Ok(target) = Target::from_triple(&TargetTriple::create(&resolved_triple)) {
            if let Some(machine) = target.create_target_machine(
                &TargetTriple::create(&resolved_triple),
                "",
                "",
                inkwell::OptimizationLevel::Default,
                inkwell::targets::RelocMode::Default,
                inkwell::targets::CodeModel::Default,
            ) {
                let td = machine.get_target_data();
                let data_layout = td.get_data_layout();
                module.set_data_layout(&data_layout);
                Some(td)
            } else {
                None
            }
        } else {
            None
        };

        CodeGenContext {
            context,
            module,
            builder,
            target_triple: resolved_triple,
            target_data,
            scopes: vec![HashMap::new()],
            struct_types: HashMap::new(),
            struct_fields: HashMap::new(),
            declared_funcs: HashMap::new(),
            kang_funcs: HashSet::new(),
            panic_func: None,
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
        self.kang_funcs.insert(name.to_string());
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
        types::kang_type_to_basic(self, ty)
    }

    /// 获取类型的 LLVM 大小（字节），用于 alloca
    pub fn size_of(&self, ty: &KangType) -> u32 {
        types::size_of(self, ty)
    }

    /// 获取类型的默认值（零初始化），void 不可用
    pub fn default_value(&self, ty: &KangType) -> BasicValueEnum<'ctx> {
        types::default_value(self, ty)
    }
}
