# Kang 交叉编译指南 — 以 Linux x86_64 (musl) 为例

## 概述

Kang 编译器支持通过 `--target=<triple>` 参数在 macOS 上编译产生 Linux 平台的可执行文件。
本指南演示从 macOS (AArch64) 编译到 Linux x86_64 musl 自包含静态可执行文件的完整流程。

## 前置准备

### 1. 安装 Rust 交叉编译 target

```bash
rustup target add x86_64-unknown-linux-musl
```

### 2. 安装 LLVM (可选，非必须)

macOS 上 kangc 的内建 LLVM 后端（inkwell）仅包含 AArch64 target。
对于 x86_64-linux 等目标，kangc 会自动回退到外部 `llc` 工具链。

```bash
brew install llvm
# 安装后 llc 位于 /opt/homebrew/opt/llvm/bin/llc
```

macoOS 的 Homebrew LLVM 未包含 `ld.lld`，kangc 会自动从 Rust 工具链
`~/.rustup/toolchains/*/lib/rustlib/*/bin/gcc-ld/ld.lld` 查找交叉链接器。

## 编译示例

### Hello World

```kang
// hello.kang
def main() -> i32 {
    puts("Hello from Kang on Linux x86_64!");
    print("This binary was cross-compiled on macOS.\n");
    return 0;
}
```

### 编译命令

```bash
kangc build hello.kang -o hello-linux --target=x86_64-unknown-linux-musl --stats
```

### 管线分解

等价的手动分步流程：

```bash
# 1. 生成 LLVM IR（平台无关）
kangc codegen hello.kang -o hello.ll --target=x86_64-unknown-linux-musl

# 2. IR → 目标文件（外部 llc）
/opt/homebrew/opt/llvm/bin/llc hello.ll -o hello.o \
    -filetype=obj --mtriple=x86_64-unknown-linux-musl

# 3. 构建 kangrt 运行时库
cargo build --release -p kangrt --target x86_64-unknown-linux-musl

# 4. 静态链接
RUST_LLD=~/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/\
          aarch64-apple-darwin/bin/gcc-ld/ld.lld
MUSL_LIB=~/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/\
          x86_64-unknown-linux-musl/lib/self-contained

"$RUST_LLD" \
  "$MUSL_LIB/crt1.o" "$MUSL_LIB/crti.o" "$MUSL_LIB/crtbegin.o" \
  hello.o \
  target/x86_64-unknown-linux-musl/release/libkangrt.a \
  "$MUSL_LIB/libc.a" "$MUSL_LIB/libc.a" \
  "$MUSL_LIB/crtend.o" "$MUSL_LIB/crtn.o" \
  -o hello-linux -m elf_x86_64 --hash-style=gnu --eh-frame-hdr -static
```

### 验证产物

```bash
$ file hello-linux
hello-linux: ELF 64-bit LSB executable, x86-64, version 1 (SYSV),
             statically linked, with debug_info, not stripped

$ ls -la hello-linux
-rwxr-xr-x  1 user  staff  199616  May 31 23:21 hello-linux
```

产物是 ~195KB 的完全静态可执行文件，无任何运行时依赖，可部署到任意 x86_64 Linux 环境。

## 支持的交叉编译目标

| 目标平台 | triple | kangrt 构建 | 说明 |
|---------|--------|-----------|------|
| Linux x86_64 (musl) | `x86_64-unknown-linux-musl` | `cargo build --release -p kangrt --target x86_64-unknown-linux-musl` | 完全静态链接 |
| Linux AArch64 (musl) | `aarch64-unknown-linux-musl` | `cargo build --release -p kangrt --target aarch64-unknown-linux-musl` | 完全静态链接 |
| macOS x86_64 | `x86_64-apple-darwin` | 同 OS 不需交叉链接 | 使用系统 cc |
| macOS AArch64 | `aarch64-apple-darwin` | 默认 host | 本地编译 |

## 内建编译 vs 外部工具链

`kangc build --target=...` 会根据目标自动选择路径:

```
同一 OS (如 macOS → macOS)：
  kangc codegen → inkwell TargetMachine 直接生成 .o → 系统 cc 链接

跨 OS (如 macOS → Linux)：
  kangc codegen → llc 生成 .o → Rust 工具链 lld + musl libc 静态链接
```

## 故障排查

| 问题 | 解决方案 |
|------|---------|
| `无法找到交叉链接器 ld.lld` | 安装 Rust musl target: `rustup target add x86_64-unknown-linux-musl` |
| 链接时 `undefined symbol: memcpy` | musl libc 未链接，确认 Rust target 已安装 |
| `unsupported target triple` (IR 阶段) | LLVM IR 生成不依赖 target backend，此错误不应出现在 IR 阶段 |
| `unsupported target triple` (.o 阶段) | 安装 Homebrew LLVM: `brew install llvm` |
