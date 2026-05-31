// 内存管理 — Bump 分配器，管理 Kang 程序中所有堆分配（字符串、数组）
// 所有分配在程序结束时统一回收，无 free 操作

use crate::panic::k_panic_impl;

const CHUNK_SIZE: usize = 64 * 1024; // 每个 chunk 64KB

/// 已分配的内存块链表节点
struct Chunk {
    data: *mut u8,
    size: usize,
    next: *mut Chunk,
}

/// 全局分配器状态
///
/// # 设计约束
/// kangrt 永久单线程运行。Kang 程序无多线程模型，信号处理器不触发分配。
/// 这三个 static mut 在单线程访问下是安全的，无需同步开销。
/// 若未来引入多线程，必须重构为 `UnsafeCell` + 适当的同步原语。
static mut HEAD: *mut Chunk = core::ptr::null_mut();
static mut BUMP: *mut u8 = core::ptr::null_mut();
static mut REMAINING: usize = 0;

unsafe extern "C" {
    fn malloc(size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
}

/// 分配一个新的 chunk
unsafe fn new_chunk(size: usize) -> *mut Chunk {
    let chunk_ptr = unsafe { malloc(core::mem::size_of::<Chunk>()) as *mut Chunk };
    if chunk_ptr.is_null() {
        return core::ptr::null_mut();
    }
    let data = unsafe { malloc(size) };
    if data.is_null() {
        unsafe { free(chunk_ptr as *mut u8) };
        return core::ptr::null_mut();
    }
    unsafe {
        (*chunk_ptr).data = data;
        (*chunk_ptr).size = size;
        (*chunk_ptr).next = HEAD;
        HEAD = chunk_ptr;
        BUMP = data;
        REMAINING = size;
    }
    chunk_ptr
}

/// 在 arena 中分配 size 字节对齐内存，返回指针（失败时 panic）
///
/// # Safety
/// `align` 必须是 2 的幂，否则为未定义行为
unsafe fn alloc(size: usize, align: usize) -> *mut u8 {
    debug_assert!(align.is_power_of_two(), "align 必须是 2 的幂，got: {align}");

    let bump = unsafe { BUMP };
    let remaining = unsafe { REMAINING };

    let offset = bump.align_offset(align);
    let needed = offset.checked_add(size).unwrap_or(usize::MAX);

    if needed <= remaining {
        let ptr = unsafe { bump.add(offset) };
        unsafe {
            BUMP = ptr.add(size);
            REMAINING = remaining - needed;
            // 零初始化
            core::ptr::write_bytes(ptr, 0, size);
        }
        return ptr;
    }

    // 当前 chunk 不够，分配新的 chunk
    let chunk_size = if size + align > CHUNK_SIZE {
        size.checked_add(align)
            .and_then(|v| v.checked_add(1024))
            .unwrap_or(usize::MAX)
    } else {
        CHUNK_SIZE
    };

    if unsafe { new_chunk(chunk_size).is_null() } {
        k_panic_impl("out of memory\0".as_ptr(), 13);
    }

    // 在新 chunk 中分配
    let bump = unsafe { BUMP };
    let offset = bump.align_offset(align);
    let ptr = unsafe { bump.add(offset) };
    unsafe {
        BUMP = ptr.add(size);
        REMAINING -= offset.checked_add(size).unwrap_or(usize::MAX);
        core::ptr::write_bytes(ptr, 0, size);
    }
    ptr
}

// ── 公共 C ABI ──────────────────────────────────────────────────────────────────

/// 分配 size 字节，8 字节对齐。OOM 时调用 k_panic 终止
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_arena_alloc(size: usize) -> *mut u8 {
    unsafe { alloc(size, 8) }
}

/// 分配 size 字节，align 字节对齐
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_arena_alloc_aligned(size: usize, align: usize) -> *mut u8 {
    unsafe { alloc(size, align) }
}

/// 重置 arena，释放所有已分配内存
#[unsafe(no_mangle)]
pub unsafe extern "C" fn k_arena_reset() {
    let mut chunk = unsafe { HEAD };
    while !chunk.is_null() {
        unsafe {
            free((*chunk).data);
            let next = (*chunk).next;
            free(chunk as *mut u8);
            chunk = next;
        }
    }
    unsafe {
        HEAD = core::ptr::null_mut();
        BUMP = core::ptr::null_mut();
        REMAINING = 0;
    }
}

// ── 测试 ────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_returns_non_null() {
        unsafe {
            let ptr = k_arena_alloc(16);
            assert!(!ptr.is_null());
            k_arena_reset();
        }
    }

    #[test]
    fn alloc_returns_zeroed_memory() {
        unsafe {
            let ptr = k_arena_alloc(64);
            for i in 0..64 {
                assert_eq!(*ptr.add(i), 0, "byte {i} not zero");
            }
            k_arena_reset();
        }
    }

    #[test]
    fn multiple_allocs_return_distinct_pointers() {
        unsafe {
            let a = k_arena_alloc(32);
            let b = k_arena_alloc(32);
            assert!(a != b);
            // 两者应在不同偏移且不重叠
            let diff = if a > b { a as usize - b as usize } else { b as usize - a as usize };
            assert!(diff >= 32);
            k_arena_reset();
        }
    }

    #[test]
    fn alloc_aligned_respects_alignment() {
        unsafe {
            let ptr = k_arena_alloc_aligned(8, 16);
            assert_eq!(ptr as usize % 16, 0);
            k_arena_reset();
        }
    }

    #[test]
    fn reset_allows_reallocation() {
        unsafe {
            let a = k_arena_alloc(128);
            *a = 0xAB;
            k_arena_reset();
            let b = k_arena_alloc(128);
            // reset 后重新分配，指针可能不同，但功能正常
            assert!(!b.is_null());
            *b = 0xCD;
            assert_eq!(*b, 0xCD);
            k_arena_reset();
        }
    }

    #[test]
    fn large_allocation_exceeds_chunk() {
        unsafe {
            // 分配超过默认 64KB chunk 的内存
            let ptr = k_arena_alloc(128 * 1024);
            assert!(!ptr.is_null());
            // 应该可以写入最后一个字节
            *ptr.add(128 * 1024 - 1) = 0xFF;
            assert_eq!(*ptr.add(128 * 1024 - 1), 0xFF);
            k_arena_reset();
        }
    }
}
