// C ABI 类型定义 — kangrt 与 kangc 之间的 FFI 数据契约
// 所有结构体为 #[repr(C)]，字段对齐与 LLVM IR 代码生成一致

/// Kang 字符串: arena 分配的 null-terminated UTF-8 数据的指针 + 字节长度
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KStr {
    pub ptr: *const u8,
    pub len: i32,
}

/// (str, bool) 二值返回 — 用于 read_file, read_line
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KStrBool {
    pub ptr: *const u8,
    pub len: i32,
    pub ok: i32,
}

/// (i32, bool) 二值返回 — 用于 i32(s: str), file_size
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KI32Bool {
    pub val: i32,
    pub ok: i32,
}

/// (f64, bool) 二值返回 — 用于 f64(s: str)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KF64Bool {
    pub val: f64,
    pub ok: i32,
}

/// (bool, bool) 二值返回 — 用于 bool(s: str)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KBoolBool {
    pub val: i32,
    pub ok: i32,
}

/// push 返回值: 新数组的 (指针, 新长度) — codegen 层使用
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KPtrLen {
    pub ptr: *mut u8,
    pub len: i32,
}
