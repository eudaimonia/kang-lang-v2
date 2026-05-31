# Kang v2 代码审计报告 (第二轮)

**审计日期**: 2026-05-31
**审查范围**: kangc (编译器) + kangrt (运行时) 全量源码，含 M7 模块系统新增代码
**测试基线**: 210 个单元测试 + 16 个集成测试全部通过
**发现总数**: 52 项 (CRITICAL 9 / HIGH 10 / MEDIUM 16 / LOW 17)

---

## 变更摘要 (相比第一轮审计)

| 类别 | 第一轮 | 新增 | 合计 |
|------|--------|------|------|
| CRITICAL | 4 | 5 | 9 |
| HIGH | 9 | 5 | 10 (其中 C1 降级为 H4 的新子项) |
| MEDIUM | 10 | 8 | 16 (含 2 项升级) |
| LOW | 11 | 6 | 17 |

**本轮新发现的关键问题**:
1. **64 位平台指针截断** (新 C1): 数组/字符串索引操作将指针截断为 i32，在所有 64 位平台上产生错误内存访问
2. **路径穿越** (新 C2): import 路径无校验，可读取任意文件
3. **M7 模块系统相关问题**: 递归栈溢出风险、导入解析重复、静默吞错误

---

## 一、总体评价

编译器管线设计清晰，各阶段通过标准化 IR 解耦。模块职责单一，符合 CLAUDE.md 五项原则。

M7 模块系统实现后，新增风险集中在三个领域：
1. **代码生成正确性**: 跨模块调用的指针算术类型不一致 (i32 vs i64)
2. **文件系统安全**: import 路径穿越、CC 环境变量注入、执行路径检查不完整
3. **错误处理韧性**: codegen/stmt.rs unwrap、Void 类型 panic、静默吞导入错误

---

## 二、CRITICAL (9 项)

### C1. [NEW] 64 位平台上数组/字符串索引的指针截断
**文件**: `kangc/src/codegen/expr.rs:543-546`
**影响**: 所有运行在 64 位平台上的 Kang 程序

```rust
let arr_addr = ok(ctx.builder.build_ptr_to_int(
    arr_ptr.into_pointer_value(),
    ctx.context.i32_type(),  // BUG: 64 位指针截断为 32 位
    "arr.addr",
))?;
```

后续的 `int_add`、`int_mul`、`int_to_ptr` 都基于截断后的 i32 地址，产生指向错误内存区域的指针。与此形成对比的是，同一文件中的 `resolve_array_ptr` (stmt.rs:120) 和 `codegen_array_lit` (expr.rs:629) 正确使用了 i64。

**修复方案**: 将 i32 改为 i64，同步更新所有相关的整数运算操作数类型。

### C2. [NEW] Import 路径穿越 (任意文件读取)
**文件**: `kangc/src/main.rs:462-466`, `kangc/src/semantic/checker.rs:15-27`

```rust
let base_dir = entry.parent().unwrap_or(Path::new("."));
let dep_path = base_dir.join(&imp.path);
```

`resolve_module_path` 和 `collect_imports` 均未对 import 路径做任何校验。恶意 Kang 源文件可通过 `import m { x } from "../../../etc/passwd"` 读取任意系统文件。虽仅解析不执行，但文件内容和存在性会泄露到编译输出。

**修复方案**: 规范化路径后，验证其在项目根目录或指定的源文件基础目录内。

### C3. [NEW] 空结构体字面量导致 panic
**文件**: `kangc/src/codegen/expr.rs:669-670`

```rust
let mut field_values: Vec<BasicValueEnum> =
    vec![ctx.default_value(&field_defs[0].1); field_defs.len()];
```

`struct Empty {}` 是合法语法 (parser.rs 测试用例)。当 `field_defs` 为空时，`field_defs[0]` 触发 index out of bounds panic。从 `grammar_tests/` 可触发。

**修复方案**: 添加 `if field_defs.is_empty() { return Ok(...) }` 守卫。

### C4. [NEW] 零长度 Token 流的越界访问
**文件**: `kangc/src/parser.rs:50-58`

```rust
fn peek(&self) -> &Token {
    if self.pos < self.tokens.len() {
        &self.tokens[self.pos]
    } else {
        &self.tokens[self.tokens.len() - 1]  // 空切片时 panic
    }
}
```

若 `Parser::new` 收到空 token 切片，`self.tokens.len() - 1` 下溢到 `usize::MAX`。虽然 lexer 总是附加 EOF 哨兵，但 `parse_stmt` 和 `parse_expr` 公开 API 接受任意 token 切片。

**修复方案**: `new()` 中加 `debug_assert!(!self.tokens.is_empty())`，或返回静态 EOF 哨兵。

### C5. 可变参数 FFI 是未定义行为 (原 C1)
**文件**: `kangrt/src/builtins.rs:42`

```rust
fn snprintf(s: *mut u8, n: usize, format: *const u8, ...) -> i32;
```

Rust FFI 规范明确：C 可变参数通过 `extern "C"` 声明为 `...` 是 UB。实践上正常工作但编译器有权生成错误代码。

**修复方案**: 用 Rust crate (`itoa`/`ryu`) 替代 FFI 调用，或编写 C 包装函数编译进 kangrt。

### C6. 执行路径来自用户输入 (原 C2)
**文件**: `kangc/src/main.rs:226`

`exe_path` 完全派生自 CLI 参数，可能指向系统目录。

**修复方案**: 编译到安全临时目录，执行前验证二进制完整性。

### C7. REPL 临时文件 TOCTOU 竞态 (原 C3)
**文件**: `kangc/src/repl.rs:247-274`

临时文件使用可预测路径 `/tmp/kang_repl_{pid}/repl_N`，链接和执行之间存在窗口期。

**修复方案**: 使用 `O_EXCL` 创建文件 + 执行前验证 inode；或使用 JIT 绕过临时文件。

### C8. 无同步的全局 mutable statics (原 C4)
**文件**: `kangrt/src/arena.rs:16-18`

三个 `static mut` 变量无并发保护。当前单线程假设正确，但未来扩展有风险。

### C9. [NEW] codegen 中未检查的内置函数参数索引
**文件**: `kangc/src/codegen/expr.rs:470, 479-483`

```rust
fn codegen_builtin_len(ctx, args) -> Result<...> {
    let arg = codegen_expr(ctx, &args[0])?;  // 空 args 时 panic
```

`codegen_builtin_len` 和 `codegen_builtin_push` 对 `args` 做无保护索引。虽然语义检查器保证参数数量正确，但若 codegen 被独立调用 (测试、重构)，将 panic。

**修复方案**: 添加 `if args.is_empty()` 检查和错误返回。

---

## 三、HIGH (10 项)

### H1. [NEW] CC 环境变量代码注入
**文件**: `kangc/src/main.rs:418, 517`, `kangc/src/repl.rs:547`

```rust
let linker = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
```

恶意设置的 `CC=/tmp/evil` 导致任意代码执行。CI/CD 和构建脚本环境中尤为危险。

**修复方案**: 验证 CC 路径在可信目录中，或改用 lld 作为内置链接器。

### H2. [NEW] 系统目录执行防护的可绕过前缀检查
**文件**: `kangc/src/main.rs:270-278`

```rust
let is_system_dir = exe_parent.starts_with("/usr/bin")
    || exe_parent.starts_with("/bin")
    || exe_parent.starts_with("/sbin")
    || exe_parent.starts_with("/usr/sbin");
```

不规范化路径、不解析符号链接、macOS 专属目录 (如 `/usr/local/bin`) 未覆盖。`-o /bin/../home/user/evil` 可绕过。

**修复方案**: 规范化路径后验证在 workspace 输出目录内。

### H3. [NEW] Arena 对齐契约在 release 模式未强制
**文件**: `kangrt/src/arena.rs:56, 109-112`

```rust
debug_assert!(align.is_power_of_two());  // release 模式跳过
```

传入非 2 的幂对齐值触发 UB (`align_offset` 行为未定义)。当前 codegen 传硬编码值 (8/16)，但未来 bug 或重构可能导致非法值。

**修复方案**: 改为运行时检查，无效对齐调用 `k_panic`。

### H4. [NEW] 导入解析错误静默忽略
**文件**: `kangc/src/semantic/checker.rs:143-151`

```rust
let tokens = match crate::lexer::tokenize(&source, &mut lex_stats) {
    Ok(t) => t,
    Err(_) => return,  // 静默丢弃 lex 错误
};
let imported_program = match crate::parser::parse(&tokens, &mut parse_stats) {
    Ok(p) => p,
    Err(_) => return,  // 静默丢弃 parse 错误
};
```

依赖库的 lex/parse 错误被静默吞掉，用户只看到"函数未找到"的误导信息。

**修复方案**: 收集错误写入 checker 的错误列表。

### H5. codegen/stmt.rs 的 ok() 吞 LLVM 错误 (原 H1)
**文件**: `kangc/src/codegen/stmt.rs:13-15`

约 30+ 处调用点，任一 LLVM builder 错误导致编译器 panic。

**修复方案**: 改用 expr.rs 版本的返回 `Result<T, CodeGenError>`。

### H6. call_val() 对 void 返回值 panic (原 H2)
**文件**: `kangc/src/codegen/expr.rs:43-47`

Void 函数返回值 `try_as_basic_value()` 为 None 触发 panic，依赖调用方隐含约定。

**修复方案**: 返回 `Result`，在调用方区分 void/有值。

### H7. Void 类型传播到 codegen 触发 panic (原 H3)
**文件**: `kangc/src/codegen/types.rs:15, 89`

```
KangType::Void => panic!("void 不可映射为 LLVM 基本类型")
```

**修复方案**: 返回 `CodeGenError`。

### H8. k_panic 查找的脆弱 expect (原 H4)
**文件**: `kangc/src/codegen/runtime.rs:42`

builtins 注册与运行时检查插入之间无编译期依赖保证。

### H9. strtol/off_t 平台类型不匹配 (原 H8/H9)
**文件**: `kangrt/src/builtins.rs:38, 50`

32 位平台上 `c_long` 为 i32，但 FFI 声明为 i64。未定义 `_FILE_OFFSET_BITS=64` 的平台上 ABI 不匹配。

### H10. [NEW] 硬编码 CC 链接器忽略 --target
**文件**: `kangc/src/main.rs:361`

LLVM 按 target triple 生成代码，但链接始终用宿主 cc，交叉编译不可用。

---

## 四、MEDIUM (16 项)

| ID | 文件 | 说明 |
|----|------|------|
| M1 | `codegen/expr.rs:509` | 复合类型元素大小硬编码 16 字节，仅 64 位有效 |
| M2 | `codegen/expr.rs:225-227` | `memcmp` 依赖 libc，kangrt 不提供，no_std 目标无法链接 |
| M3 | `codegen/types.rs:50` | 手动 struct 对齐 `(total+7)/8*8` 忽略 LLVM data layout |
| M4 | `main.rs:391-411` | `find_or_build_kangrt` 通过 PATH 找 cargo，可被篡改 |
| M5 | `main.rs:439-476` | `collect_imports` 递归实现，深度导入链可能栈溢出 |
| M6 | `main.rs:480-513` | 多文件编译中途失败不清理已生成的 .o 文件 |
| M7 | `main.rs:352-356` | stats JSON 序列化使用 `.unwrap()`，失败时 panic |
| M8 | `main.rs:149` | `is_some()` 检查和后续 `unwrap()` 有逻辑间隙 |
| M9 | `repl.rs:157` | 词法错误包装为 ParseError，诊断输出不准确 |
| M10 | `repl.rs:530` | kangrt 构建失败调用 `process::exit(1)` 终止 REPL |
| M11 | `repl.rs:70` | stdout 写入失败被 `let _` 静默忽略 |
| M12 | `parser.rs:71` | `expect()` 用 `discriminant` 比较，`Ident("foo")` 匹配任意标识符 |
| M13 | `checker.rs:109` | 结构体错误 span 为 `0..0`，ariadne 无法高亮 |
| M14 | `codegen.rs:55` | LLVM verify 错误丢弃完整诊断，只取第一行 |
| M15 | `lib.rs:83` | semantic check 返回空错误列表时 `.unwrap()` panic |
| M16 | `checker.rs:28` | `check_func_def` 公开但依赖 `collect_declarations` 先执行 |

---

## 五、LOW (17 项)

| ID | 文件 | 说明 |
|----|------|------|
| L1 | `main.rs:317` | `env!("CARGO_MANIFEST_DIR")` — 二进制移走后路径失效 |
| L2 | `codegen/types.rs:31` | 未注册 struct 退化为 opaque struct |
| L3 | `repl.rs:346` | `VOID_BUILTINS` 命名误导 (push 有返回值) |
| L4 | `lexer.rs:94` | `unwrap_or("")` 掩盖字符串解析失败 |
| L5 | `ast.rs:159` | `prec()` 方法标记 `#[allow(dead_code)]`，未使用 |
| L6 | `parser.rs:579` | `unreachable!()` 对 `parse_block()` 总是返回 Block |
| L7 | `main.rs:382` | `read_file` 直接 exit 而非返回 Result |
| L8 | `checker.rs:666` | `hint` 字段被 clone 两次 |
| L9 | `builtins.rs:219` | `k_read_line` 固定 4096 字节缓冲区 |
| L10 | `parser.rs:13` | `MAX_TOKEN_COUNT` / `MAX_PARSE_DEPTH` 硬编码 |
| L11 | `Cargo.toml:4` | `edition = "2024"` 需要 Rust 1.85+ |
| L12 | `builtins.rs:356` | `k_write_file` 丢弃 fputs/fclose 返回值，写入失败无提示 |
| L13 | `builtins.rs:152` | `format_f64` 对 ≥ i64::MAX 的值返回 "inf" |
| L14 | `builtins.rs:382` | `k_file_exists` 对目录返回 true，名称有误导性 |
| L15 | `kangrt/src/arena.rs:21` | Arena 全局变量标记 `static mut`，无并发保护 |
| L16 | `main.rs:219` | 无扩展名源文件生成空白可执行文件名 |
| L17 | `checker.rs:19` | `resolve_module_path` 忽略绝对路径检测 |

---

## 六、M7 模块系统专项分析

### 设计问题

**D1. 导入依赖被重复解析**
`checker.rs:131` 和 `main.rs:451` 各自对导入文件做 lex+parse。N 个文件导入同一库时，库被解析 N 次。

**D2. 递归导入的栈溢出风险**
`collect_imports` 使用递归 DFS，深度导入链可能栈溢出。应改为迭代 BFS。

**D3. 导入错误传播不足**
导入文件中的 lex/parse 错误被静默忽略，用户只看到间接的"函数未找到"。

**D4. 代码生成缺少跨模块符号表**
codegen 通过 `ctx.module.get_function()` 查找函数，跨模块调用依赖函数名精确匹配(导出原名)。缺乏模块级符号管理。

**D5. 无版本/缓存机制**
全量重编译，无增量编译或 artifact 缓存。

### 安全性

**D6. 路径穿越**
`resolve_module_path` 和 `collect_imports` 均未验证 import 路径在安全范围内。

---

## 七、修复优先级

### P0 — 立即修复 (影响正确性)

| 序号 | ID | 任务 | 预估 |
|------|----|------|------|
| 1 | C1 | 修复 64 位指针截断: `i32` → `i64` | 30m |
| 2 | C2 | Import 路径校验，防止目录穿越 | 30m |
| 3 | C3 | 空结构体 codegen 守卫 | 15m |
| 4 | C4 | Parser `peek()` 空 token 防御 | 10m |
| 5 | H3 | Arena 对齐运行时检查 | 15m |

### P1 — 本周修复

| 序号 | ID | 任务 | 预估 |
|------|----|------|------|
| 6 | C1 (原) | 移除 variadic `snprintf` FFI | 2h |
| 7 | H1 (原) | `codegen/stmt.rs` 的 `ok()` → Result | 1h |
| 8 | H2/H3 (原) | `call_val()` + Void panic → CodeGenError | 1h |
| 9 | H4 | 导入解析错误传播 | 30m |
| 10 | M5 | 递归导入改为迭代 | 30m |

### P2 — 下个里程碑

| 序号 | ID | 任务 | 预估 |
|------|----|------|------|
| 11 | H1 | CC 环境变量校验 | 1h |
| 12 | H2 | 执行路径规范化 | 30m |
| 13 | H7 (原) | `--target` 选择交叉链接器 | 1h |
| 14 | M1-M3 | REPL 错误处理加固 | 1h |
| 15 | H8/H9 (原) | 条件编译处理 32 位 FFI | 1h |

### P3 — 适时清理

| 序号 | 任务 | 预估 |
|------|------|------|
| 16 | 移除 dead code (`prec()`, `VOID_BUILTINS` 重命名) | 15m |
| 17 | 文档化 `CARGO_MANIFEST_DIR` 开发约束 | 5m |
| 18 | LLVM verify 保留完整错误信息 | 15m |
| 19 | 其余 LOW 项在日常开发中逐步修复 | -- |

---

## 八、模块健康度

| 模块 | 问题数 | 评级 | 说明 |
|------|--------|------|------|
| `lexer.rs` | 1 | 好 | 仅 1 个低级问题 |
| `parser.rs` | 4 | 较好 | `expect()` discriminant + peek 防御 + 低级问题 |
| `ast.rs` | 1 | 好 | 1 个 dead code |
| `semantic/` | 5 | 需关注 | span 参数、API 隐含约定、导入错误静默、路径安全 |
| `codegen/expr.rs` | 6 | **需修复** | 指针截断 (C1)、空结构体 panic、内置函数索引、内联声明 |
| `codegen/stmt.rs` | 2 | **需修复** | `ok()` unwrap 约 30+ 处 |
| `codegen/types.rs` | 3 | 需关注 | Void panic + 手动对齐 + opaque struct |
| `codegen/runtime.rs` | 1 | 较好 | `expect` 依赖约定 |
| `codegen.rs` | 1 | 好 | LLVM 错误截断 |
| `main.rs` | 9 | **需修复** | 路径穿越、链接器注入、执行路径、递归栈、cargo 依赖 |
| `repl.rs` | 6 | 需关注 | TOCTOU、错误处理、临时文件 |
| `error.rs` | 0 | 好 | — |
| `stats.rs` | 0 | 好 | — |
| `lib.rs` | 1 | 好 | unwrap 防御 |
| `kangrt/arena.rs` | 2 | 需关注 | mutable statics + 对齐契约 |
| `kangrt/builtins.rs` | 6 | **需修复** | FFI UB + 平台兼容 + 缓冲区 + 返回值丢弃 |

---

## 九、静态分析维度

| 维度 | 状态 |
|------|------|
| **unsafe 代码** | 编译器侧无；kangrt 有 FFI 声明和 C 交互 |
| **unwrap/expect** | codegen/stmt.rs (~30)、main.rs (~10)、codegen/runtime.rs (1) |
| **panic 调用** | codegen/expr.rs (call_val)、codegen/types.rs (Void ×2) |
| **process::exit** | main.rs (read_file, build_kangrt)、repl.rs (build_kangrt) |
| **FFI 声明** | kangrt/builtins.rs — variadic UB、32 位类型不匹配 |
| **全局可变状态** | kangrt/arena.rs — 3 个 static mut |
| **文件系统安全** | main.rs (路径穿越)、repl.rs (可预测路径、TOCTOU) |
| **环境变量注入** | main.rs (CC, PATH→cargo) |
| **内存安全** | Arena bump allocator 无释放、无 use-after-free |
| **并发安全** | kangrt 单线程假设，无并发保护 |
| **测试覆盖** | 210 单元 + 16 集成测试全部通过 |

---

*报告由 Claude Code 生成，结合两轮静态代码分析和自动化测试验证。*
