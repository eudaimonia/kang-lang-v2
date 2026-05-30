// 符号表与作用域栈 — 管理变量、函数、结构体的声明和查找
// checker 通过 SymbolTable 追踪标识符的生命周期和类型信息

use super::types::KangType;
use std::collections::HashMap;

// ── 符号条目 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FuncSignature {
    pub params: Vec<(String, KangType)>,
    pub return_type: KangType,
    pub is_builtin: bool,
    /// 内置函数重载变体（用户函数此字段为空）
    pub overloads: Vec<FuncSignature>,
}

/// 结构体字段信息
#[derive(Debug, Clone)]
pub struct StructInfo {
    pub fields: Vec<(String, KangType)>,
}

/// 符号表中存储的条目
#[derive(Debug, Clone)]
pub enum SymbolKind {
    Variable(KangType),
    Function(FuncSignature),
    Struct(StructInfo),
}

/// 是否有特殊的作用域限制
#[derive(Debug, Clone, PartialEq)]
pub enum ScopeHint {
    Normal,
    LoopVar, // for 循环变量，循环结束后不可访问
}

#[derive(Debug, Clone)]
pub struct SymbolEntry {
    pub name: String,
    pub kind: SymbolKind,
    pub hint: ScopeHint,
}

// ── 作用域 ────────────────────────────────────────────────────────────────────

struct Scope {
    symbols: HashMap<String, SymbolEntry>,
}

impl Scope {
    fn new() -> Self {
        Scope { symbols: HashMap::new() }
    }
}

// ── 符号表 ────────────────────────────────────────────────────────────────────

pub struct SymbolTable {
    scopes: Vec<Scope>,
    pub symbol_count: usize,
}

impl SymbolTable {
    pub fn new() -> Self {
        let mut table = SymbolTable { scopes: vec![Scope::new()], symbol_count: 0 };
        table.register_builtins();
        table
    }

    // ── 作用域操作 ──────────────────────────────────────────────────────────

    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    pub fn pop_scope(&mut self) -> Vec<SymbolEntry> {
        // 调试断言: 不应弹出全局作用域（至少保留一个）
        debug_assert!(self.scopes.len() > 1, "pop_scope: 不能弹出全局作用域");
        let scope = self.scopes.pop().unwrap_or_else(|| Scope::new());
        scope.symbols.into_values().collect()
    }

    /// 弹出作用域，但保留非 LoopVar 的条目提升到外层
    /// 用于 for 循环结束后仅清除循环变量
    pub fn pop_scope_keep_non_loop(&mut self) {
        if let Some(top) = self.scopes.pop() {
            if let Some(parent) = self.scopes.last_mut() {
                for (name, entry) in top.symbols {
                    if entry.hint != ScopeHint::LoopVar {
                        parent.symbols.insert(name, entry);
                    }
                }
            }
        }
    }

    // ── 插入 ────────────────────────────────────────────────────────────────

    /// 在当前作用域插入符号。同名冲突返回 Err
    pub fn insert(
        &mut self,
        name: &str,
        kind: SymbolKind,
        hint: ScopeHint,
    ) -> Result<(), String> {
        let scope = self.scopes.last_mut().expect("至少存在一个作用域");
        if scope.symbols.contains_key(name) {
            return Err(format!("符号 \"{}\" 已在当前作用域声明", name));
        }
        scope.symbols.insert(
            name.to_string(),
            SymbolEntry { name: name.to_string(), kind, hint },
        );
        self.symbol_count += 1;
        Ok(())
    }

    // ── 查找 ────────────────────────────────────────────────────────────────

    /// 从内到外在所有作用域中查找符号
    pub fn lookup(&self, name: &str) -> Option<&SymbolEntry> {
        for scope in self.scopes.iter().rev() {
            if let Some(entry) = scope.symbols.get(name) {
                return Some(entry);
            }
        }
        None
    }

    /// 仅在当前作用域查找
    pub fn lookup_current(&self, name: &str) -> Option<&SymbolEntry> {
        self.scopes.last().and_then(|s| s.symbols.get(name))
    }

    /// 查找函数，支持内置函数重载解析
    /// 返回匹配的 FuncSignature（用户函数优先，内置函数按参数类型匹配）
    pub fn lookup_function(&self, name: &str, arg_types: &[KangType]) -> Option<&FuncSignature> {
        for scope in self.scopes.iter().rev() {
            if let Some(entry) = scope.symbols.get(name) {
                if let SymbolKind::Function(sig) = &entry.kind {
                    // 先尝试匹配主签名
                    if params_match(&sig.params, arg_types) {
                        return Some(sig);
                    }
                    // 再尝试重载
                    for overload in &sig.overloads {
                        if params_match(&overload.params, arg_types) {
                            return Some(overload);
                        }
                    }
                    // 函数名存在但参数不匹配 — 返回主签名（调用方报参数错误）
                    return Some(sig);
                }
            }
        }
        None
    }

    /// 查找所有同名函数的重载列表（用于错误消息中列出候选）
    pub fn lookup_all_overloads(&self, name: &str) -> Vec<FuncSignature> {
        if let Some(entry) = self.lookup(name) {
            if let SymbolKind::Function(sig) = &entry.kind {
                let mut all = vec![sig.clone()];
                all.extend(sig.overloads.clone());
                return all;
            }
        }
        vec![]
    }
}

/// 检查参数类型列表是否匹配
fn params_match(params: &[(String, KangType)], arg_types: &[KangType]) -> bool {
    if params.len() != arg_types.len() {
        return false;
    }
    params.iter().zip(arg_types).all(|((_, pt), at)| *pt == *at)
}

// ── 内置函数注册 ──────────────────────────────────────────────────────────────

impl SymbolTable {
    fn register_builtins(&mut self) {
        // 直接注册内置函数，不使用闭包（避免 borrow checker 冲突）

        // len(str) -> i32 的具体重载。len([T]) -> i32 由 checker 特殊处理
        self.register_builtin("len", vec![("s", KangType::Str)], KangType::I32, vec![]);
        // push([T], T) -> void 由 checker 特殊处理（泛型数组）
        self.register_builtin("push", vec![("a", KangType::Array(Box::new(KangType::I32))), ("elem", KangType::I32)], KangType::Void, vec![]);

        // 输出: puts, print, eprint
        self.register_builtin("puts", vec![("s", KangType::Str)], KangType::Void, vec![]);
        self.register_builtin("print", vec![("s", KangType::Str)], KangType::Void, vec![]);
        self.register_builtin("eprint", vec![("s", KangType::Str)], KangType::Void, vec![]);

        // 文件 I/O
        self.register_builtin("read_file", vec![("path", KangType::Str)],
            KangType::Pair(Box::new(KangType::Str), Box::new(KangType::Bool)), vec![]);
        self.register_builtin("read_line", vec![],
            KangType::Pair(Box::new(KangType::Str), Box::new(KangType::Bool)), vec![]);
        self.register_builtin("write_file", vec![("path", KangType::Str), ("content", KangType::Str)], KangType::Void, vec![]);
        self.register_builtin("append_file", vec![("path", KangType::Str), ("content", KangType::Str)], KangType::Void, vec![]);
        self.register_builtin("file_exists", vec![("path", KangType::Str)], KangType::Bool, vec![]);
        self.register_builtin("file_size", vec![("path", KangType::Str)],
            KangType::Pair(Box::new(KangType::I32), Box::new(KangType::Bool)), vec![]);

        // 类型转换: str 有 3 个重载
        self.register_builtin("str", vec![("n", KangType::I32)], KangType::Str, vec![
            (vec![("n", KangType::F64)], KangType::Str),
            (vec![("b", KangType::Bool)], KangType::Str),
        ]);
        // i32 有 2 个重载
        self.register_builtin("i32", vec![("s", KangType::Str)],
            KangType::Pair(Box::new(KangType::I32), Box::new(KangType::Bool)), vec![
            (vec![("n", KangType::F64)], KangType::I32),
        ]);
        // f64 有 2 个重载
        self.register_builtin("f64", vec![("s", KangType::Str)],
            KangType::Pair(Box::new(KangType::F64), Box::new(KangType::Bool)), vec![
            (vec![("n", KangType::I32)], KangType::F64),
        ]);
        // bool(s: str) -> (bool, bool)
        self.register_builtin("bool", vec![("s", KangType::Str)],
            KangType::Pair(Box::new(KangType::Bool), Box::new(KangType::Bool)), vec![]);
    }

    /// 注册一个内置函数（可能有重载变体）
    fn register_builtin(
        &mut self,
        name: &str,
        params: Vec<(&str, KangType)>,
        ret: KangType,
        overloads: Vec<(Vec<(&str, KangType)>, KangType)>,
    ) {
        let scope = self.scopes.last_mut().expect("至少存在一个作用域");
        let convert_params = |p: Vec<(&str, KangType)>| -> Vec<(String, KangType)> {
            p.into_iter().map(|(n, t)| (n.to_string(), t)).collect()
        };

        let main = FuncSignature {
            params: convert_params(params),
            return_type: ret,
            is_builtin: true,
            overloads: overloads.into_iter().map(|(p, r)| FuncSignature {
                params: convert_params(p),
                return_type: r,
                is_builtin: true,
                overloads: vec![],
            }).collect(),
        };

        scope.symbols.insert(
            name.to_string(),
            SymbolEntry {
                name: name.to_string(),
                kind: SymbolKind::Function(main),
                hint: ScopeHint::Normal,
            },
        );
        self.symbol_count += 1;
    }
}
