// 运行时 panic — 异常终止处理
// 编译后的 Kang 程序通过 k_panic 上报致命错误（OOB、除零等）

#[cfg(not(test))]
use core::panic::PanicInfo;

unsafe extern "C" {
    fn write(fd: i32, buf: *const u8, count: usize) -> isize;
    fn abort() -> !;
}

const STDERR: i32 = 2;
const PANIC_PREFIX: &[u8] = b"Kang runtime panic: ";
const NEWLINE: &[u8] = b"\n";

/// 输出 panic 消息到 stderr 并终止程序
pub(crate) fn k_panic_impl(msg: *const u8, len: usize) -> ! {
    unsafe {
        let _ = write(STDERR, PANIC_PREFIX.as_ptr(), PANIC_PREFIX.len());
        let _ = write(STDERR, msg, len);
        let _ = write(STDERR, NEWLINE.as_ptr(), NEWLINE.len());
        abort();
    }
}

/// C ABI: Kang 程序运行时错误入口
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_panic(msg: *const u8, msg_len: i32) -> ! {
    let safe_len = if msg_len < 0 { 0 } else { msg_len as usize };
    k_panic_impl(msg, safe_len);
}

/// no_std panic handler — Rust 侧 panic 时调用
/// test 模式下由 std 提供 panic handler，跳过以避免冲突
#[cfg(not(test))]
#[panic_handler]
fn panic_handler(_info: &PanicInfo) -> ! {
    let msg = b"internal runtime error";
    k_panic_impl(msg.as_ptr(), msg.len());
}
