// 内置函数实现 — 19 个 C ABI 函数，封装 libc 实现 Kang 语言内置功能
// 所有字符串/数组操作通过 arena 分配内存

use crate::arena::k_arena_alloc;
use crate::types::*;

// ── libc 外部声明 ──────────────────────────────────────────────────────────────

/// FILE 不透明类型
#[repr(C)]
struct FILE {
    _private: [u8; 0],
}

unsafe extern "C" {
    // 标准 I/O 流 — macOS 使用 __stdinp/__stdoutp/__stderrp，Linux 使用 stdin/stdout/stderr
    #[cfg(target_os = "macos")]
    static __stdinp: *mut FILE;
    #[cfg(target_os = "macos")]
    static __stdoutp: *mut FILE;
    #[cfg(target_os = "macos")]
    static __stderrp: *mut FILE;

    #[cfg(not(target_os = "macos"))]
    static stdin: *mut FILE;
    #[cfg(not(target_os = "macos"))]
    static stdout: *mut FILE;
    #[cfg(not(target_os = "macos"))]
    static stderr: *mut FILE;

    // stdio
    fn fopen(path: *const u8, mode: *const u8) -> *mut FILE;
    fn fclose(stream: *mut FILE) -> i32;
    fn fread(ptr: *mut u8, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    fn fgets(s: *mut u8, n: i32, stream: *mut FILE) -> *mut u8;
    fn fputs(s: *const u8, stream: *mut FILE) -> i32;
    fn fputc(c: i32, stream: *mut FILE) -> i32;
    fn fseek(stream: *mut FILE, offset: i64, whence: i32) -> i32;
    fn ftell(stream: *mut FILE) -> i64;
    fn snprintf(s: *mut u8, n: usize, format: *const u8, ...) -> i32;

    // string
    fn strlen(s: *const u8) -> usize;
    fn strcmp(s1: *const u8, s2: *const u8) -> i32;

    // stdlib
    fn strtol(nptr: *const u8, endptr: *mut *mut u8, base: i32) -> i64;
    fn strtod(nptr: *const u8, endptr: *mut *mut u8) -> f64;
    fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8;

    // unistd
    fn access(path: *const u8, mode: i32) -> i32;
}

/// 平台无关的 stdin/stdout/stderr 访问（macOS 与 Linux 符号名不同）
fn stdin_ptr() -> *mut FILE {
    #[cfg(target_os = "macos")]
    unsafe { __stdinp }
    #[cfg(not(target_os = "macos"))]
    unsafe { stdin }
}

fn stdout_ptr() -> *mut FILE {
    #[cfg(target_os = "macos")]
    unsafe { __stdoutp }
    #[cfg(not(target_os = "macos"))]
    unsafe { stdout }
}

fn stderr_ptr() -> *mut FILE {
    #[cfg(target_os = "macos")]
    unsafe { __stderrp }
    #[cfg(not(target_os = "macos"))]
    unsafe { stderr }
}

const F_OK: i32 = 0;
const SEEK_SET: i32 = 0;
const SEEK_END: i32 = 2;

// ── 辅助函数 ────────────────────────────────────────────────────────────────────

/// Rust str 数据 → C null-terminated 字符串（arena 分配）
/// 若 len < 0（编译器 bug），返回空字符串避免 OOM
unsafe fn to_c_str(s: *const u8, len: i32) -> *mut u8 {
    let actual_len = if len < 0 { 0 } else { len };
    unsafe {
        let buf = k_arena_alloc(actual_len as usize + 1);
        memcpy(buf, s, actual_len as usize);
        *buf.add(actual_len as usize) = 0;
        buf
    }
}

/// 截断字符串末尾的换行符
unsafe fn strip_newline(s: *mut u8, len: *mut i32) {
    unsafe {
        if *len > 0 && *s.add(*len as usize - 1) == b'\n' {
            *s.add(*len as usize - 1) = 0;
            *len -= 1;
        }
    }
}

// ── 数组操作 ────────────────────────────────────────────────────────────────────

/// len(s: str) -> i32 — 直接返回 len 参数
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_len_str(_ptr: *const u8, len: i32) -> i32 {
    len
}

/// push(a: [T], elem: T) -> void — 分配新数组，拷贝旧元素 + 新元素
/// 返回新数组的 (ptr, len)，codegen 负责更新变量
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_push(
    arr: *const u8,
    arr_len: i32,
    elem: *const u8,
    elem_size: i32,
) -> KPtrLen {
    unsafe {
        // 防御性校验: 编译器不应生成非法参数，但运行时做底线防御
        let safe_arr_len = if arr_len < 0 { 0 } else { arr_len };
        let safe_elem_size = if elem_size <= 0 { 1 } else { elem_size };
        let new_len = safe_arr_len + 1;
        let new_arr = k_arena_alloc(4 + (new_len * safe_elem_size) as usize);
        // 写入新长度
        *(new_arr as *mut i32) = new_len;
        // 拷贝旧元素
        if safe_arr_len > 0 && !arr.is_null() {
            memcpy(new_arr.add(4), arr.add(4), (safe_arr_len * safe_elem_size) as usize);
        }
        // 拷贝新元素
        if !elem.is_null() {
            memcpy(new_arr.add(4 + (safe_arr_len * safe_elem_size) as usize), elem, safe_elem_size as usize);
        }
        KPtrLen { ptr: new_arr, len: new_len }
    }
}

// ── 输出 ────────────────────────────────────────────────────────────────────────

/// puts(s: str) -> void — 输出字符串 + 换行到 stdout
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_puts(s: *const u8, len: i32) {
    unsafe {
        let c_str = to_c_str(s, len);
        fputs(c_str, stdout_ptr());
        fputc(b'\n' as i32, stdout_ptr());
    }
}

/// print(s: str) -> void — 输出字符串到 stdout
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_print(s: *const u8, len: i32) {
    unsafe {
        let c_str = to_c_str(s, len);
        fputs(c_str, stdout_ptr());
    }
}

/// eprint(s: str) -> void — 输出字符串到 stderr
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_eprint(s: *const u8, len: i32) {
    unsafe {
        let c_str = to_c_str(s, len);
        fputs(c_str, stderr_ptr());
    }
}

// ── 文件 I/O ────────────────────────────────────────────────────────────────────

/// read_file(path: str) -> (str, bool) — 读取整个文件到 arena
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_read_file(path: *const u8, path_len: i32) -> KStrBool {
    unsafe {
        let path_c = to_c_str(path, path_len);
        let file = fopen(path_c, b"rb\0".as_ptr());
        if file.is_null() {
            return KStrBool { ptr: core::ptr::null(), len: 0, ok: 0 };
        }
        fseek(file, 0, SEEK_END);
        let size = ftell(file);
        fseek(file, 0, SEEK_SET);
        if size < 0 {
            fclose(file);
            return KStrBool { ptr: core::ptr::null(), len: 0, ok: 0 };
        }
        let buf = k_arena_alloc(size as usize + 1);
        let read = fread(buf, 1, size as usize, file);
        fclose(file);
        *buf.add(read) = 0;
        KStrBool { ptr: buf, len: read as i32, ok: 1 }
    }
}

/// read_line() -> (str, bool) — 从 stdin 读取一行
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_read_line() -> KStrBool {
    unsafe {
        let mut buf = [0u8; 4096];
        let result = fgets(buf.as_mut_ptr(), 4096, stdin_ptr());
        if result.is_null() {
            return KStrBool { ptr: core::ptr::null(), len: 0, ok: 0 };
        }
        let mut len = strlen(buf.as_ptr()) as i32;
        strip_newline(buf.as_mut_ptr(), &mut len);
        let out = k_arena_alloc(len as usize + 1);
        memcpy(out, buf.as_ptr(), len as usize);
        *out.add(len as usize) = 0;
        KStrBool { ptr: out, len, ok: 1 }
    }
}

/// write_file(path: str, content: str) -> void
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_write_file(
    path: *const u8,
    path_len: i32,
    content: *const u8,
    content_len: i32,
) {
    unsafe {
        let path_c = to_c_str(path, path_len);
        let content_c = to_c_str(content, content_len);
        let file = fopen(path_c, b"w\0".as_ptr());
        if file.is_null() {
            return;
        }
        fputs(content_c, file);
        fclose(file);
    }
}

/// append_file(path: str, content: str) -> void
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_append_file(
    path: *const u8,
    path_len: i32,
    content: *const u8,
    content_len: i32,
) {
    unsafe {
        let path_c = to_c_str(path, path_len);
        let content_c = to_c_str(content, content_len);
        let file = fopen(path_c, b"a\0".as_ptr());
        if file.is_null() {
            return;
        }
        fputs(content_c, file);
        fclose(file);
    }
}

/// file_exists(path: str) -> bool — 使用 access(F_OK)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_file_exists(path: *const u8, path_len: i32) -> i32 {
    unsafe {
        let path_c = to_c_str(path, path_len);
        (access(path_c, F_OK) == 0) as i32
    }
}

/// file_size(path: str) -> (i32, bool) — 使用 fseek/ftell
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_file_size(path: *const u8, path_len: i32) -> KI32Bool {
    unsafe {
        let path_c = to_c_str(path, path_len);
        let file = fopen(path_c, b"rb\0".as_ptr());
        if file.is_null() {
            return KI32Bool { val: 0, ok: 0 };
        }
        fseek(file, 0, SEEK_END);
        let size = ftell(file);
        fclose(file);
        if size < 0 || size > i32::MAX as i64 {
            return KI32Bool { val: 0, ok: 0 };
        }
        KI32Bool { val: size as i32, ok: 1 }
    }
}

// ── 字符串操作 ──────────────────────────────────────────────────────────────────

/// str_concat(a: str, b: str) -> str — 拼接两个字符串到 arena
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_str_concat(
    a_ptr: *const u8,
    a_len: i32,
    b_ptr: *const u8,
    b_len: i32,
) -> KPtrLen {
    unsafe {
        let safe_a_len = if a_len < 0 { 0 } else { a_len };
        let safe_b_len = if b_len < 0 { 0 } else { b_len };
        let total = safe_a_len + safe_b_len;
        let buf = k_arena_alloc(total as usize + 1);
        if safe_a_len > 0 && !a_ptr.is_null() {
            memcpy(buf, a_ptr, safe_a_len as usize);
        }
        if safe_b_len > 0 && !b_ptr.is_null() {
            memcpy(buf.add(safe_a_len as usize), b_ptr, safe_b_len as usize);
        }
        *buf.add(total as usize) = 0;
        KPtrLen { ptr: buf, len: total }
    }
}

// ── 类型转换 ────────────────────────────────────────────────────────────────────

/// str(n: i32) -> str — snprintf 到 arena
/// i32 最大值 "−2147483648" 为 11 字符 + null = 12，32 字节足够
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_str_i32(n: i32) -> KStr {
    unsafe {
        let buf = k_arena_alloc(32);
        let len = snprintf(buf, 32, b"%d\0".as_ptr(), n);
        // snprintf 返回应写入的字节数；若 >=32 说明被截断，这里不应发生
        let safe_len = if len >= 32 { 31 } else { len };
        KStr { ptr: buf, len: safe_len as i32 }
    }
}

/// str(n: f64) -> str — snprintf 到 arena
/// "%.10g" 格式最多约 25 字符，64 字节缓冲区足够
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_str_f64(n: f64) -> KStr {
    unsafe {
        let buf = k_arena_alloc(64);
        let len = snprintf(buf, 64, b"%.10g\0".as_ptr(), n);
        let safe_len = if len >= 64 { 63 } else { len };
        KStr { ptr: buf, len: safe_len as i32 }
    }
}

/// str(b: bool) -> str — 返回 "true" 或 "false"（arena 分配）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_str_bool(b: i32) -> KStr {
    unsafe {
        let (src, len) = if b != 0 {
            (b"true\0".as_ptr(), 4i32)
        } else {
            (b"false\0".as_ptr(), 5i32)
        };
        let buf = k_arena_alloc(len as usize + 1);
        memcpy(buf, src, len as usize + 1);
        KStr { ptr: buf, len }
    }
}

/// i32(s: str) -> (i32, bool) — strtol，失败时 ok=false
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_i32_str(s: *const u8, len: i32) -> KI32Bool {
    unsafe {
        let c_str = to_c_str(s, len);
        let mut end: *mut u8 = core::ptr::null_mut();
        let val = strtol(c_str, &mut end, 10);
        if end.is_null() || end == c_str {
            return KI32Bool { val: 0, ok: 0 };
        }
        // 检查剩余字符是否仅为空白
        let mut p = end;
        while *p != 0 {
            if *p != b' ' && *p != b'\t' && *p != b'\n' && *p != b'\r' {
                return KI32Bool { val: 0, ok: 0 };
            }
            p = p.add(1);
        }
        if val < i32::MIN as i64 || val > i32::MAX as i64 {
            return KI32Bool { val: 0, ok: 0 };
        }
        KI32Bool { val: val as i32, ok: 1 }
    }
}

/// i32(n: f64) -> i32 — 向零截断；NaN/Inf → panic
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_i32_f64(n: f64) -> i32 {
    // 使用 core 内建方法（LLVM 内在函数），不依赖 C math 库
    if n.is_nan() || n.is_infinite() {
        crate::panic::k_panic_impl(b"cannot convert NaN/Inf to i32\0".as_ptr(), 28);
    }
    n as i32
}

/// f64(s: str) -> (f64, bool) — strtod，失败时 ok=false
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_f64_str(s: *const u8, len: i32) -> KF64Bool {
    unsafe {
        let c_str = to_c_str(s, len);
        let mut end: *mut u8 = core::ptr::null_mut();
        let val = strtod(c_str, &mut end);
        if end.is_null() || end == c_str {
            return KF64Bool { val: 0.0, ok: 0 };
        }
        // 检查剩余字符是否仅为空白
        let mut p = end;
        while *p != 0 {
            if *p != b' ' && *p != b'\t' && *p != b'\n' && *p != b'\r' {
                return KF64Bool { val: 0.0, ok: 0 };
            }
            p = p.add(1);
        }
        KF64Bool { val, ok: 1 }
    }
}

/// f64(n: i32) -> f64 — 无损转换
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_f64_i32(n: i32) -> f64 {
    n as f64
}

/// bool(s: str) -> (bool, bool) — 仅接受 "true" / "false"
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_bool_str(s: *const u8, len: i32) -> KBoolBool {
    unsafe {
        let c_str = to_c_str(s, len);
        let true_str = b"true\0".as_ptr();
        let false_str = b"false\0".as_ptr();
        if strcmp(c_str, true_str) == 0 {
            KBoolBool { val: 1, ok: 1 }
        } else if strcmp(c_str, false_str) == 0 {
            KBoolBool { val: 0, ok: 1 }
        } else {
            KBoolBool { val: 0, ok: 0 }
        }
    }
}

// ── 测试 ────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::k_arena_reset;

    /// KStr → Rust &str
    unsafe fn kstr_to_str(k: &KStr) -> &str {
        let slice = unsafe { core::slice::from_raw_parts(k.ptr, k.len as usize) };
        core::str::from_utf8(slice).unwrap()
    }

    // ── k_str_i32 ───────────────────────────────────────────────────────────────

    #[test]
    fn str_i32_positive() {
        unsafe {
            let k = k_str_i32(42);
            assert_eq!(kstr_to_str(&k), "42");
            k_arena_reset();
        }
    }

    #[test]
    fn str_i32_negative() {
        unsafe {
            let k = k_str_i32(-17);
            assert_eq!(kstr_to_str(&k), "-17");
            k_arena_reset();
        }
    }

    #[test]
    fn str_i32_zero() {
        unsafe {
            let k = k_str_i32(0);
            assert_eq!(kstr_to_str(&k), "0");
            k_arena_reset();
        }
    }

    #[test]
    fn str_i32_min() {
        unsafe {
            let k = k_str_i32(i32::MIN);
            assert_eq!(kstr_to_str(&k), "-2147483648");
            k_arena_reset();
        }
    }

    // ── k_str_f64 ───────────────────────────────────────────────────────────────

    #[test]
    fn str_f64_integer() {
        unsafe {
            let k = k_str_f64(3.0);
            let s = kstr_to_str(&k);
            // "%.10g" 应输出 "3" 或类似形式
            assert!(s.starts_with('3'), "got: {s}");
            k_arena_reset();
        }
    }

    #[test]
    fn str_f64_fraction() {
        unsafe {
            let k = k_str_f64(3.14);
            let s = kstr_to_str(&k);
            assert!(s.contains("3.14"), "got: {s}");
            k_arena_reset();
        }
    }

    #[test]
    fn str_f64_negative() {
        unsafe {
            let k = k_str_f64(-2.5);
            let s = kstr_to_str(&k);
            assert!(s.starts_with('-'), "got: {s}");
            k_arena_reset();
        }
    }

    // ── k_str_bool ──────────────────────────────────────────────────────────────

    #[test]
    fn str_bool_true() {
        unsafe {
            let k = k_str_bool(1);
            assert_eq!(kstr_to_str(&k), "true");
            k_arena_reset();
        }
    }

    #[test]
    fn str_bool_false() {
        unsafe {
            let k = k_str_bool(0);
            assert_eq!(kstr_to_str(&k), "false");
            k_arena_reset();
        }
    }

    // ── k_i32_str ───────────────────────────────────────────────────────────────

    #[test]
    fn i32_str_valid() {
        unsafe {
            let s = "123";
            let k = k_i32_str(s.as_ptr(), s.len() as i32);
            assert_eq!(k.val, 123);
            assert_eq!(k.ok, 1);
            k_arena_reset();
        }
    }

    #[test]
    fn i32_str_negative() {
        unsafe {
            let s = "-456";
            let k = k_i32_str(s.as_ptr(), s.len() as i32);
            assert_eq!(k.val, -456);
            assert_eq!(k.ok, 1);
            k_arena_reset();
        }
    }

    #[test]
    fn i32_str_invalid() {
        unsafe {
            let s = "not_a_number";
            let k = k_i32_str(s.as_ptr(), s.len() as i32);
            assert_eq!(k.ok, 0);
            k_arena_reset();
        }
    }

    #[test]
    fn i32_str_trailing_garbage() {
        unsafe {
            let s = "123abc";
            let k = k_i32_str(s.as_ptr(), s.len() as i32);
            assert_eq!(k.ok, 0, "尾部非空白应失败");
            k_arena_reset();
        }
    }

    #[test]
    fn i32_str_overflow() {
        unsafe {
            let s = "9999999999";
            let k = k_i32_str(s.as_ptr(), s.len() as i32);
            assert_eq!(k.ok, 0, "超出 i32 范围应失败");
            k_arena_reset();
        }
    }

    // ── k_i32_f64 ───────────────────────────────────────────────────────────────

    #[test]
    fn i32_f64_truncate() {
        unsafe {
            let result = k_i32_f64(3.9);
            assert_eq!(result, 3, "应向零截断");
        }
    }

    #[test]
    fn i32_f64_negative_truncate() {
        unsafe {
            let result = k_i32_f64(-3.9);
            assert_eq!(result, -3, "应向零截断");
        }
    }

    // ── k_f64_i32 ───────────────────────────────────────────────────────────────

    #[test]
    fn f64_i32_positive() {
        unsafe {
            let result = k_f64_i32(42);
            assert!((result - 42.0).abs() < 0.001);
        }
    }

    #[test]
    fn f64_i32_negative() {
        unsafe {
            let result = k_f64_i32(-10);
            assert!((result + 10.0).abs() < 0.001);
        }
    }

    // ── k_f64_str ───────────────────────────────────────────────────────────────

    #[test]
    fn f64_str_valid() {
        unsafe {
            let s = "3.14";
            let k = k_f64_str(s.as_ptr(), s.len() as i32);
            assert_eq!(k.ok, 1);
            assert!((k.val - 3.14).abs() < 0.001);
            k_arena_reset();
        }
    }

    #[test]
    fn f64_str_invalid() {
        unsafe {
            let s = "xyz";
            let k = k_f64_str(s.as_ptr(), s.len() as i32);
            assert_eq!(k.ok, 0);
            k_arena_reset();
        }
    }

    // ── k_bool_str ──────────────────────────────────────────────────────────────

    #[test]
    fn bool_str_true() {
        unsafe {
            let s = "true";
            let k = k_bool_str(s.as_ptr(), s.len() as i32);
            assert_eq!(k.val, 1);
            assert_eq!(k.ok, 1);
            k_arena_reset();
        }
    }

    #[test]
    fn bool_str_false() {
        unsafe {
            let s = "false";
            let k = k_bool_str(s.as_ptr(), s.len() as i32);
            assert_eq!(k.val, 0);
            assert_eq!(k.ok, 1);
            k_arena_reset();
        }
    }

    #[test]
    fn bool_str_invalid() {
        unsafe {
            let s = "maybe";
            let k = k_bool_str(s.as_ptr(), s.len() as i32);
            assert_eq!(k.ok, 0);
            k_arena_reset();
        }
    }

    #[test]
    fn bool_str_case_sensitive() {
        unsafe {
            let s = "True";
            let k = k_bool_str(s.as_ptr(), s.len() as i32);
            assert_eq!(k.ok, 0, "大小写应与 C 的 true/false 严格一致");
            k_arena_reset();
        }
    }

    // ── k_len_str ───────────────────────────────────────────────────────────────

    #[test]
    fn len_str_empty() {
        unsafe {
            let result = k_len_str("".as_ptr(), 0);
            assert_eq!(result, 0);
        }
    }

    #[test]
    fn len_str_non_empty() {
        unsafe {
            let result = k_len_str("hello".as_ptr(), 5);
            assert_eq!(result, 5);
        }
    }

    // ── k_push ──────────────────────────────────────────────────────────────────

    #[test]
    fn push_creates_array() {
        unsafe {
            let elem: i32 = 42;
            let result = k_push(
                core::ptr::null(), // 空数组
                0,                 // arr_len = 0
                &elem as *const i32 as *const u8,
                4,                 // elem_size = 4
            );
            assert_eq!(result.len, 1);
            // 新数组首 4 字节是长度
            assert_eq!(*(result.ptr as *const i32), 1);
            // 接下来的 4 字节是元素
            assert_eq!(*(result.ptr.add(4) as *const i32), 42);
            k_arena_reset();
        }
    }

    #[test]
    fn push_appends_to_array() {
        unsafe {
            let elem1: i32 = 10;
            let r1 = k_push(core::ptr::null(), 0, &elem1 as *const i32 as *const u8, 4);
            assert_eq!(r1.len, 1);

            let elem2: i32 = 20;
            let r2 = k_push(r1.ptr, 1, &elem2 as *const i32 as *const u8, 4);
            assert_eq!(r2.len, 2);
            assert_eq!(*(r2.ptr as *const i32), 2);
            assert_eq!(*(r2.ptr.add(4) as *const i32), 10);
            assert_eq!(*(r2.ptr.add(8) as *const i32), 20);
            k_arena_reset();
        }
    }

    // ── 文件 I/O ────────────────────────────────────────────────────────────────

    #[test]
    fn file_exists_on_known_file() {
        unsafe {
            // Cargo.toml 应当在项目根目录存在
            let path = "Cargo.toml";
            let result = k_file_exists(path.as_ptr(), path.len() as i32);
            assert_eq!(result, 1);
        }
    }

    #[test]
    fn file_exists_on_nonexistent_file() {
        unsafe {
            let path = "/nonexistent/path/to/file/xyzzy.test";
            let result = k_file_exists(path.as_ptr(), path.len() as i32);
            assert_eq!(result, 0);
        }
    }

    #[test]
    fn write_and_read_file_roundtrip() {
        unsafe {
            let path = "kangrt_test_temp.txt";
            let content = "Hello, Kang!";

            // 写入
            k_write_file(
                path.as_ptr(),
                path.len() as i32,
                content.as_ptr(),
                content.len() as i32,
            );

            // 验证存在
            assert_eq!(k_file_exists(path.as_ptr(), path.len() as i32), 1);

            // 读取
            let result = k_read_file(path.as_ptr(), path.len() as i32);
            assert_eq!(result.ok, 1);
            assert_eq!(result.len, content.len() as i32);
            let kstr = KStr { ptr: result.ptr, len: result.len };
            let got = kstr_to_str(&kstr);
            assert_eq!(got, content);

            // 清理
            std::fs::remove_file(path).unwrap();
            k_arena_reset();
        }
    }
}
