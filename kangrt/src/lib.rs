// kangrt — Kang 语言运行时库
// 提供 Kang 程序执行所需的: 内存管理 (arena)、内置函数 (builtins)、异常终止 (panic)
//
// 编译为静态库 libkangrt.a，通过 C ABI 与 kangc 生成的 LLVM IR 链接
// 所有公开函数使用 #[unsafe(no_mangle)] + unsafe extern "C"，签名与 SPECS 10.3 一致

#![cfg_attr(not(test), no_std)]

pub mod arena;
pub mod builtins;
pub mod panic;
pub mod types;
