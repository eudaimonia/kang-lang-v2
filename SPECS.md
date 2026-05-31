# Kang v2 技术规格书 (SPECS)

## 1. 架构总览

Kang 编译器采用 **分阶段工具流水线架构**，每个阶段是可独立运行的 CLI 子命令，有标准化的中间表示 (IR)，可按需截断、审查、组合。

```
 source.kang
    │
    ▼  kang lex          → Token Stream (.tok)    文本, 一行一个 token
    │
    ▼  kang parse        → AST (.ast)             S-expression 树形
    │
    ▼  kang check        → Typed AST (.tast)      AST + 类型标注
    │
    ▼  kang codegen      → LLVM IR (.ll)          目标无关 IR, 可跨平台传递
    │                          │
    │                    [--target=<triple>]       默认动态检测当前环境
    │                          │
    ▼  [opt]             → 优化后的 LLVM IR       (可选)
    │
    ▼  kang build        → 目标文件 → musl 静态链接 → 自包含可执行文件
```

**设计原则：**
- 每个阶段 = 纯函数: `IR_in → IR_out`
- 输出可读文本，人可肉眼审查每步产出
- 所有阶段内建统计埋点，`--stats` 输出结构化 JSON
- AI 可独立开发/测试一个阶段；人类可在任意阶段审查输出
- REPL 和 AOT 复用前三阶段 (lex→parse→semantic)，代码生成分别走 JIT 和 AOT 路径

---

## 2. 项目结构

```
kang-v2/
├── Cargo.toml                    # workspace: members = ["kangc", "kangrt"]
├── kangc/                        # 编译器 (lib + binary)
│   ├── Cargo.toml                # deps: inkwell, logos, ariadne, serde
│   └── src/
│       ├── main.rs               # CLI 入口
│       ├── lib.rs                # 公共 API
│       ├── lexer.rs              # 词法分析
│       ├── parser.rs             # 语法分析 → AST
│       ├── ast.rs                # AST 类型定义
│       ├── semantic.rs           # 语义分析入口
│       ├── semantic/
│       │   ├── types.rs          # KangType 定义
│       │   ├── scope.rs          # 符号表 + 作用域栈
│       │   ├── checker.rs        # 类型检查器 (26 规则)
│       │   └── flow.rs           # 控制流分析
│       ├── codegen.rs            # LLVM IR 生成入口
│       ├── codegen/
│       │   ├── context.rs        # CodeGenContext (inkwell 封装)
│       │   ├── types.rs          # KangType → LLVM Type
│       │   ├── expr.rs           # 表达式代码生成
│       │   ├── stmt.rs           # 语句代码生成
│       │   ├── builtins.rs       # 内置函数声明
│       │   └── runtime.rs        # 运行时检查插入
│       ├── repl.rs               # REPL 循环 + JIT
│       ├── stats.rs              # 统计数据定义 + 序列化
│       └── error.rs              # 错误类型 + ariadne 格式化
├── kangrt/                        # 运行时库 (独立 crate)
│   ├── Cargo.toml                # 静态链接 musl
│   └── src/
│       ├── lib.rs                # #[no_mangle] k_* 函数
│       ├── arena.rs              # Bump allocator
│       ├── builtins.rs           # 19 个内置函数, 封装 musl libc
│       └── panic.rs              # 异常终止
├── grammar_tests/                # 10 个语法正向测试
├── semantic_tests/               # 7 个语义负向测试
└── tests/                        # 集成测试
```

### 模块依赖

```
                ┌──────────┐    ┌──────────┐
                │  error   │    │  stats   │  ← 公共类型, 全模块共享
                └──────────┘    └──────────┘

┌──────┐    ┌──────┐    ┌──────────┐    ┌─────────┐
│ lexer│───→│parser│───→│ semantic │───→│ codegen │───→ LLVM IR / 二进制
└──┬───┘    └──┬───┘    └────┬─────┘    └────┬────┘
   │           │             │               │
   ▼           ▼             ▼               ▼
LexStats   ParseStats   SemanticStats   CodeGenStats
   │           │             │               │
   └───────────┴─────────────┴───────────────┘
                     │
                     ▼
              CompilerStats (聚合)
```

**解耦规则：**
- AST 是 parser/semantic/codegen 之间的唯一数据契约
- AST 节点不含类型标注；Typed AST 由 semantic 阶段附加
- codegen 不依赖 parser/lexer，只接收 Typed AST
- kangrt 通过 C ABI 交互，无 Rust 层面依赖

### 模块内部依赖

```
main.rs
  ├── lexer.rs          (无内部依赖)
  ├── parser.rs         → ast.rs
  ├── semantic.rs       → ast.rs, error.rs
  │     ├── types.rs    (无内部依赖)
  │     ├── scope.rs    → ast.rs
  │     ├── checker.rs  → types.rs, scope.rs, error.rs
  │     └── flow.rs     → ast.rs, error.rs
  ├── codegen.rs        → ast.rs, codegen/*, error.rs
  │     ├── context.rs  (无内部依赖, 封装 inkwell)
  │     ├── types.rs    → ast.rs
  │     ├── expr.rs     → context.rs, types.rs
  │     ├── stmt.rs     → context.rs, types.rs
  │     ├── builtins.rs → context.rs, types.rs
  │     └── runtime.rs  → context.rs
  ├── repl.rs           → lexer, parser, semantic, codegen, stats
  ├── stats.rs          (无内部依赖)
  └── error.rs          (无内部依赖)
```

---

## 3. CLI 规范

```
kang lex       <file> [-o <file>] [--stats]    仅词法分析, 输出 Token Stream
kang parse     <file> [-o <file>] [--stats]    词法+语法, 输出 AST
kang check     <file> [--stats]                词法+语法+语义, 仅报告错误/OK
kang codegen   <file> [-o <file>] [--stats]    全前端+代码生成, 输出 LLVM IR
kang emit-llvm <file> [-o <file>] [--stats]    kang codegen 的别名
kang build     <file> [-o <binary>] [--stats]  全管线 → 可执行文件
kang run       <file> [--stats]                编译+执行
kang repl                                      交互式 REPL

选项:
  --emit=tokens|ast|typed-ast|llvm-ir|object  在指定阶段截断并输出 IR
  --target=<triple>                            目标平台三元组 (默认: 编译环境动态检测)
  --stats                                      输出各阶段统计数据 (JSON)
  -o <file>                                    输出文件路径

AOT 入口: def main() -> i32，返回值作为进程退出码。
全管线: 源码 → lex → parse → semantic → codegen → LLVM IR → opt → .o → link kangrt → 可执行文件
```

### 流水线截断示例

```bash
kang lex hello.kang                         # Token Stream → stdout
kang parse hello.kang                       # AST → stdout
kang check hello.kang                       # "OK" 或错误列表
kang codegen hello.kang -o hello.ll         # LLVM IR → hello.ll
kang build hello.kang -o hello              # 可执行文件 → ./hello
kang run hello.kang                         # 编译并执行
kang parse hello.kang --stats               # AST + 统计数据 JSON
```

---

## 4. 中间表示 (IR) 格式

### 4.1 Token Stream (.tok)

文本格式，一行一个 token：

```
KIND "lexeme" @ L:C
```

示例：
```
DEF "def" @ 1:1
IDENT "main" @ 1:5
LPAREN "(" @ 1:9
RPAREN ")" @ 1:10
ARROW "->" @ 1:12
TI32 "i32" @ 1:15
LBRACE "{" @ 1:19
RETURN "return" @ 2:5
INT_LIT "0" @ 2:12
SEMI ";" @ 2:13
RBRACE "}" @ 3:1
EOF "" @ 3:2
```

### 4.2 AST (.ast)

S-expression 格式，树形缩进：

```lisp
(program
  (func-def "main" [] -> (type "i32")
    (block
      (return (int-lit 0)))))
```

### 4.3 Typed AST (.tast)

AST + 每个表达式节点的类型标注：

```lisp
(program
  (func-def "main" [] -> :i32
    (block
      (return (int-lit 0 :i32) :i32))))
```

### 4.4 LLVM IR (.ll)

标准 LLVM IR 文本格式，可直接喂给 `llc`、`opt`。

---

## 5. Library API

```rust
// kangc/src/lib.rs — 每个阶段是独立函数, stats 作为可变引用贯穿

// 编译管线阶段标识
pub enum PipelineStage { Tokens, Ast, TypedAst, LlvmIr, Object }

// 共享管线: 运行到指定阶段，返回统计数据 + 可选阶段输出
pub fn compile_to_stage(
    source: &str,
    file_path: &str,
    target_triple: Option<&str>,
    stage: PipelineStage,
    object_path: Option<&Path>,
) -> Result<(CompilerStats, Option<String>), KangError>;

// 各阶段独立函数
pub fn tokenize(source: &str, stats: &mut LexStats) -> Result<Vec<Token>, LexError>;
pub fn parse(tokens: &[Token], stats: &mut ParseStats) -> Result<ast::Program, ParseError>;
pub fn check(program: &ast::Program, stats: &mut SemanticStats) -> Result<semantic::TypedProgram, Vec<SemanticError>>;
pub fn codegen(
    program: &semantic::TypedProgram,
    stats: &mut CodeGenStats,
    target_triple: Option<&str>,
    object_path: Option<&Path>,
) -> Result<CodeGenResult, CodeGenError>;

// 便捷包装: 全流程编译，返回各阶段产物与统计
pub fn compile_full(
    source: &str,
    file_path: &str,
) -> Result<(TypedProgram, CodeGenResult, SourceStats, LexStats, ParseStats, SemanticStats, CodeGenStats), KangError>;

// 统一错误类型
pub enum KangError { Lex(LexError), Parse(ParseError), Semantic(SemanticError), CodeGen(CodeGenError) }
```

---

## 6. 统计埋点

### 6.1 数据结构

```rust
// kangc/src/stats.rs

#[derive(Serialize)]
pub struct CompilerStats {
    pub source: SourceStats,
    pub lex: LexStats,
    pub parse: ParseStats,
    pub semantic: SemanticStats,
    pub codegen: CodeGenStats,
}

#[derive(Serialize)]
pub struct SourceStats {
    pub file_path: String,
    pub total_bytes: usize,
    pub total_lines: usize,
}

#[derive(Serialize)]
pub struct LexStats {
    pub duration_us: u64,
    pub token_count: usize,
    pub token_counts_by_kind: HashMap<String, usize>,
    pub comment_bytes: usize,
}

#[derive(Serialize)]
pub struct ParseStats {
    pub duration_us: u64,
    pub ast_node_count: usize,
    pub ast_max_depth: usize,
    pub node_counts_by_kind: HashMap<String, usize>,
    pub func_count: usize,
    pub struct_count: usize,
}

#[derive(Serialize)]
pub struct SemanticStats {
    pub duration_us: u64,
    pub error_count: usize,
    pub warning_count: usize,
    pub symbol_count: usize,
    pub type_check_passes: usize,
    pub type_check_failures: usize,
}

#[derive(Serialize)]
pub struct CodeGenStats {
    pub duration_us: u64,
    pub llvm_ir_bytes: usize,
    pub llvm_instruction_count: usize,
    pub llvm_basic_block_count: usize,
    pub llvm_function_count: usize,
    pub runtime_check_insertions: usize,
}
```

### 6.2 输出示例

`kang build --stats hello.kang`:

```json
{
  "source": {
    "file_path": "hello.kang",
    "total_bytes": 86,
    "total_lines": 5
  },
  "lex": {
    "duration_us": 42,
    "token_count": 18,
    "token_counts_by_kind": {
      "Def": 1, "Ident": 2, "IntLit": 1, "StrLit": 1, "Semi": 1, "Eof": 1
    },
    "comment_bytes": 0
  },
  "parse": {
    "duration_us": 85,
    "ast_node_count": 6,
    "ast_max_depth": 4,
    "func_count": 1,
    "struct_count": 0
  },
  "semantic": {
    "duration_us": 120,
    "error_count": 0,
    "warning_count": 0,
    "symbol_count": 2,
    "type_check_passes": 3,
    "type_check_failures": 0
  },
  "codegen": {
    "duration_us": 450,
    "llvm_ir_bytes": 1024,
    "llvm_instruction_count": 15,
    "llvm_basic_block_count": 2,
    "llvm_function_count": 1,
    "runtime_check_insertions": 0
  }
}
```

### 6.3 阶段截断时的统计输出

```bash
kang lex --stats file.kang        # 只有 source + lex 有数据
kang check --stats file.kang      # source + lex + parse + semantic
kang build --stats file.kang      # 全量五阶段数据
```

---

## 7. 里程碑与审查节点

本项目采用 **AI 主导实施、人类里程碑审查** 的协作模式。每个里程碑 AI 完成代码+测试后提交，人类审查签字后进入下一阶段。

### 审查流程

```
AI 实现模块 + 自测 → 提交代码 → 人类审查 → 签字/反馈 → 下一里程碑
```

- AI 在每个里程碑内同步完成单元测试和语义测试，不设独立测试里程碑
- 人类审查聚焦：接口契约是否正确、语义覆盖是否完整、代码是否可理解
- 审查通过则进入下一里程碑；有反馈则 AI 修订后重新提交
- 单个里程碑如超过 1,500 LOC，建议分两轮提交以降低审查负担

### 里程碑规划

| M# | 名称 | 模块 | LOC | 审查重点 | 关键交付物 |
|----|------|------|-----|---------|-----------|
| M1 | 前端 | lexer + parser + AST + stats + error + 10 grammar tests | 1,700 | 词法定义完整、AST 结构合理、语法覆盖所有合法/非法边界 | `kang lex` / `kang parse` 可运行，Token Stream 和 AST 可序列化，10 个正向语法测试通过 |
| M2 | 语义 | semantic (types/scope/checker/flow) + 7 semantic tests | 1,000 | 46 条语义规则覆盖完整、错误信息清晰有定位、作用域模型正确 | `kang check` 可运行，7 个语义测试文件正确分类（合法通过、非法报错含位置） |
| M3 | 运行时 | kangrt (arena/builtins/panic) | 500 | C ABI 签名与 SPECS 一致、内存安全无 UB、libc 封装正确 | `libkangrt.a` 独立编译，19 个内置函数功能测试通过 |
| M4 | 代码生成 | codegen (context/types/expr/stmt/builtins/runtime) | 1,800 | LLVM IR 正确可执行、6 项运行时检查齐全、二值返回打包/解包正确 | `kang codegen` 输出合法 LLVM IR，可通过 `lli` 执行，含运行时安全检查 |
| M5 | CLI + AOT | main.rs (子命令/--emit/--stats/管线串联/链接) | 400 | CLI 接口一致、管线阶段衔接正确、统计输出完整 | `kang run hello.kang` 端到端编译+执行，`--stats` 输出完整 JSON |
| M6 | REPL + 集成 | repl.rs + tests/ (端到端集成测试) | 1,200 | REPL 交互流畅、JIT 正确性、符号表跨行持久化 | REPL 交互多行输入，`:quit`/`:show`/`:reset`/`:load`，全部集成测试通过 |
| **总计** | | | **~6,600** | | |

### 并行化说明

- **M3 (运行时)** 与 M1/M2 无依赖，AI 可并行推进（如 M1+M3 或 M2+M3 同期实施）
- **M5 和 M6** 在 M4 完成后可并行推进（两者都依赖完整的编译管线）
- 人类审查仍按里程碑逐一进行，不因 AI 并行而合并审查轮次

### 依赖图

```
M1 (前端)
  │
  ▼
M2 (语义) ──┐
             │
M3 (运行时) ─┤  ← 与 M1/M2 并行
             │
             ▼
          M4 (代码生成)
             │
        ┌────┴────┐
        ▼         ▼
     M5 (CLI)  M6 (REPL+集成)
```

### 审查节奏

- **AI 实施**: 单个里程碑数小时到 1-2 天（不含人类等待时间）
- **人类审查**: 每轮建议 1-2 天内完成反馈，避免 AI 上下文因等待过长而冷却
- **修订轮次**: 审查反馈后 AI 修订通常数小时内完成
- **整体预期**: 6 个里程碑 × (1-2 天 AI 实施 + 1-2 天人类审查) ≈ 2-4 周日历时间，其中 AI 活跃时间约 3-5 天

---

## 8. 接口契约

### 阶段边界

| 边界 | 输入类型 | 输出类型 | 错误类型 |
|------|---------|---------|---------|
| lexer → parser | `&str` | `Vec<Token>` | `LexError` |
| parser → semantic | `Vec<Token>` | `ast::Program` | `ParseError` |
| semantic → codegen | `ast::Program` | `TypedProgram` | `Vec<SemanticError>` |
| codegen → 外部 | `TypedProgram` | `CodeGenResult` | `CodeGenError` |

### 公共数据类型 (ast.rs)

所有阶段共享 AST 类型，不含类型标注。`semantic::TypedProgram` 是 AST + 类型标注的组合。

### 公共基础设施

- `error.rs` — 错误类型定义 + ariadne 格式化，全模块共用
- `stats.rs` — 统计数据结构 + serde 序列化，全模块共用

---

## 9. 技术栈

| 组件 | 技术 | 说明 |
|------|------|------|
| 实现语言 | Rust | edition 2024 |
| 构建系统 | Cargo workspace | kangc + kangrt |
| 词法分析 | logos | 声明式词法, 零依赖 |
| 语法分析 | 手写递归下降 | LL(1), ~900 LOC |
| 错误报告 | ariadne | 多行标注, 彩色输出 |
| 代码生成 | inkwell | LLVM C API 的 Rust 安全封装 |
| 序列化 | serde + serde_json | stats JSON 输出 |
| 运行时 | musl libc | 静态链接, 内置函数封装 musl 实现, 无 glibc 依赖, 跨平台编译 |
| 目标平台 | LLVM target triple | 默认动态检测当前环境; `--target` 可指定任意 LLVM 支持的目标 |

---

## 10. 语言特性实现规范

本章从实施角度列出编译器各模块必须支持的语言特性，作为需求文档到代码的桥接。每项特性标注了负责模块。

### 10.1 类型系统

需在 `semantic/types.rs` 中定义的完整类型集合：

**基本类型 (5 种)**

| 类型 | 说明 | 默认值 |
|------|------|--------|
| `i32` | 有符号 32 位整数，溢出 wrapping | `0` |
| `f64` | 双精度浮点，IEEE 754 | `0.0` |
| `str` | 不可变字符串，Arena 管理 | `""` |
| `bool` | 布尔值 | `false` |
| `void` | 无返回值，不可作为变量/字段类型 | — |

**复合类型 (3 种)**

| 类型 | 语法 | 说明 | 负责模块 |
|------|------|------|---------|
| 数组 | `[T]` | T ∈ {i32, f64, str, bool, 结构体}；禁止 `[void]`、禁止嵌套 `[[T]]` | parser + semantic |
| 结构体 | `struct Name { field:Type; ... }` | 值类型；字段禁止 void；禁止直接自引用，允许 `[Self]` 间接引用 | parser + semantic |
| 二值返回 | `(T1, T2)` | 仅用于函数返回类型，不可作变量/参数/字段类型 | parser + semantic |

**类型相容性规则** (实现在 `semantic/checker.rs`):
- 所有转换必须通过内置函数显式执行，禁止隐式转换
- 唯一例外：`+` 运算中任一操作数为 `str` 时，另一操作数自动转字符串拼接
- `==` / `!=` 要求左右类型相同
- 算术/比较运算符要求操作数同为 i32 或同为 f64，不可混用

**默认值初始化** (实现在 `codegen/expr.rs`):
- 所有变量编译期强制初始化；未显式初始化时使用类型默认值
- 结构体逐字段递归取默认值；数组默认 `[]`

### 10.2 二值返回机制

`(T1, T2)` 是跨 parser → semantic → codegen 的横切关注点。

**语法层** (`parser.rs`):
- `ReturnType` 规则：`Type | "(" Type "," Type ")"` — 仅 return 位置允许二值
- `ReturnStmt`: `return e1, e2;` 最多两个表达式
- `VarDecl`: `var n1:T1, n2:T2 = f();` 最多两个绑定
- `VarBinding`: `_` 占位丢弃某个值（`_` 不含类型标注）

**语义层** (`semantic/checker.rs`):
- 二值返回函数必须恰好 return 两个表达式；单返回函数恰好一个
- 二值接收的变量数必须匹配函数返回值数
- 单接收从二值返回：取第一值、丢弃第二值
- `_` 为 discard，不绑定变量，不能在后续表达式中使用
- void 函数不可用于 var 接收

**代码生成层** (`codegen/expr.rs`, `codegen/stmt.rs`):
- 二值返回在 LLVM IR 层打包为匿名结构体 `{T1, T2}`
- 二值接收解包为 `extractvalue` 指令
- 单接收从二值：`extractvalue 0`，丢弃索引 1
- `_` 占位：对应的 extractvalue 结果直接丢弃

### 10.3 内置函数目录

共 15 个函数名、19 个重载签名。所有函数通过 `extern "C" fn k_*` 调用 kangrt 中的 musl libc 封装。

编译器端声明在 `codegen/builtins.rs`，运行时实现在 `kangrt/builtins.rs`。

**集合操作 (2)**

| 函数 | 签名 | libc 映射 |
|------|------|----------|
| `len` | `len(s: str) -> i32` | `strlen` |
| `len` | `len(a: [T]) -> i32` | 运行时存 length 字段 |
| `push` | `push(a: [T], elem: T) -> void` | Arena realloc + memcpy |

**输出 (3)**

| 函数 | 签名 | libc 映射 |
|------|------|----------|
| `puts` | `puts(s: str) -> void` | `fputs(stdout)` + `"\n"` |
| `print` | `print(s: str) -> void` | `fputs(stdout)` |
| `eprint` | `eprint(s: str) -> void` | `fputs(stderr)` |

**文件 I/O (5)**

| 函数 | 签名 | libc 映射 |
|------|------|----------|
| `read_file` | `read_file(path: str) -> (str, bool)` | `fopen` + `fread` + `fclose` |
| `read_line` | `read_line() -> (str, bool)` | `fgets(stdin)` |
| `write_file` | `write_file(path: str, content: str) -> void` | `fopen("w")` + `fputs` + `fclose` |
| `append_file` | `append_file(path: str, content: str) -> void` | `fopen("a")` + `fputs` + `fclose` |
| `file_exists` | `file_exists(path: str) -> bool` | `access(F_OK)` |
| `file_size` | `file_size(path: str) -> (i32, bool)` | `fseek(SEEK_END)` + `ftell` |

**类型转换 (8)**

| 函数 | 签名 | libc 映射 / 说明 |
|------|------|-----------------|
| `str` | `str(n: i32) -> str` | `snprintf` |
| `str` | `str(n: f64) -> str` | `snprintf` |
| `str` | `str(b: bool) -> str` | 返回 `"true"` 或 `"false"` |
| `i32` | `i32(s: str) -> (i32, bool)` | `strtol`，失败时 bool=false |
| `i32` | `i32(n: f64) -> i32` | 向零截断；NaN/Inf → panic |
| `f64` | `f64(s: str) -> (f64, bool)` | `strtod`，失败时 bool=false |
| `f64` | `f64(n: i32) -> f64` | 无损转换 |
| `bool` | `bool(s: str) -> (bool, bool)` | 仅接受 `"true"` / `"false"`，否则第二个 bool=false |

**重载规则**: 仅内置函数支持按参数类型重载；用户自定义函数不支持重载（`semantic/checker.rs` 检查）。

### 10.4 语义规则清单

46 条规则实现在 `semantic/checker.rs` (类型/表达式) + `semantic/flow.rs` (控制流)，7 个测试文件按类别覆盖。

**类型规则 (12)** — `semantic_tests/01_type_errors.kang`

| # | 规则 | 错误触发条件 |
|---|------|------------|
| T1 | 算术运算类型 | `i32` 与 `f64` 混用 `+ - * /` |
| T2 | 比较运算类型 | `i32` 与 `f64` 混用 `< <= > >=` |
| T3 | if 条件类型 | 条件表达式非 `bool` |
| T4 | for 条件类型 | 循环条件非 `bool` |
| T5 | `&&` `\|\|` 操作数 | 操作数非 `bool` |
| T6 | `!` 操作数 | 操作数非 `bool` |
| T7 | `==` `!=` 类型 | 左右操作数类型不同 |
| T8 | `==` `!=` 跨类型 | `str == i32` 等不同类型比较 |
| T9 | 数组索引类型 | 索引非 `i32` (f64/bool/...) |
| T10 | 字符串索引类型 | 索引非 `i32` |
| T11 | 结构体字段类型 | 字段赋值的表达式类型与声明不匹配 |
| T12 | 索引类型 (str) | str 索引返回单字符 str |

**作用域规则 (4)** — `semantic_tests/02_scope_errors.kang`

| # | 规则 | 错误触发条件 |
|---|------|------------|
| S1 | 参数不可重声明 | `var` 重新声明函数参数 |
| S2 | 变量先声明后使用 | 使用未声明的标识符 |
| S3 | 循环变量作用域 | for 循环结束后访问循环变量 |
| S4 | 函数/变量命名冲突 | 变量名与函数名共享命名空间，同名冲突 |

**二值返回规则 (6)** — `semantic_tests/03_multi_return_errors.kang`

| # | 规则 | 错误触发条件 |
|---|------|------------|
| M1 | 返回数量匹配 | 声明二值返回，return 只有一个表达式 |
| M2 | 返回数量匹配 | 声明单返回，return 有两个表达式 |
| M3 | 返回类型匹配 | 声明 `(i32, i32)`，返回 `(str, i32)` |
| M4 | 接收数量匹配 | 函数返 1 值，var 试图 2 接收 |
| M5 | `_` 不可作变量 | `_` 为 discard，不能在表达式中引用 |
| M6 | void 不可接收 | 从 void 函数 var 接收返回值 |

**结构体规则 (7)** — `semantic_tests/04_struct_errors.kang`

| # | 规则 | 错误触发条件 |
|---|------|------------|
| ST1 | 字段非 void | 结构体字段声明为 void |
| ST2 | 禁止直接自引用 | 结构体包含自身类型字段（非数组） |
| ST3 | 完整初始化 | 构造表达式缺少字段 |
| ST4 | 禁止多余字段 | 构造表达式含未声明字段 |
| ST5 | 类型必须定义 | 使用未定义的结构体类型 |
| ST6 | 字段访问类型检查 | 对非结构体类型使用 `.field` |
| ST7 | 字段必须存在 | 访问结构体未声明的字段 |

**函数规则 (6)** — `semantic_tests/05_func_errors.kang`

| # | 规则 | 错误触发条件 |
|---|------|------------|
| F1 | 所有路径 return | 非 void 函数存在未 return 的代码路径 |
| F2 | void 不可带值 | void 函数 return 带表达式 |
| F3 | 禁止用户重载 | 同名函数已定义 |
| F4 | 参数数量 | 调用参数数量不匹配 (少) |
| F5 | 参数数量 | 调用参数数量不匹配 (多) |
| F6 | 参数类型 | 调用参数类型与声明不匹配 |

**数组规则 (5)** — `semantic_tests/06_array_errors.kang`

| # | 规则 | 错误触发条件 |
|---|------|------------|
| A1 | 元素非 void | 声明 `[void]` |
| A2 | 元素类型一致 | 声明 `[i32]`，字面量含 `f64` |
| A3 | push 类型匹配 | push 元素类型与数组元素类型不一致 |
| A4 | push 参数数量/类型 | push 第一个参数非数组 |
| A5 | len 参数类型 | len 不接受 i32 (仅数组和 str) |

**赋值规则 (6)** — `semantic_tests/07_assign_errors.kang`

| # | 规则 | 错误触发条件 |
|---|------|------------|
| AS1 | str 元素不可赋值 | `s[i]` 作左值 (str 不可变) |
| AS2 | 赋值类型匹配 | 左右类型不一致 |
| AS3 | 字面量非左值 | 对字面量赋值 |
| AS4 | 表达式非左值 | 对非左值表达式赋值 |
| AS5 | 字段赋值类型 | 结构体字段赋值类型不匹配 |
| AS6 | 数组元素赋值类型 | 数组索引赋值类型不匹配 |

**总计: 46 规则** = T(12) + S(4) + M(6) + ST(7) + F(6) + A(5) + AS(6)

### 10.5 运行时安全检查

需在 `codegen/runtime.rs` 中插入 LLVM IR 检查，失败时调用 `k_panic(msg)` 终止程序。

| # | 检查项 | 触发条件 | 插入位置 | 行为 |
|---|--------|---------|---------|------|
| R1 | 数组索引越界 | `arr[i]` 且 `i < 0` 或 `i >= len` | 每次索引操作前 | `k_panic("index out of bounds")` |
| R2 | 字符串索引越界 | `s[i]` 且 `i < 0` 或 `i >= strlen` | 每次索引操作前 | `k_panic("index out of bounds")` |
| R3 | 整数除零 | `a / b` 且 `b == 0` | 除法指令前 | `k_panic("division by zero")` |
| R4 | `INT_MIN / -1` 溢出 | i32 除法且被除数为 `INT_MIN`、除数为 `-1` | 除法指令前 | `k_panic("integer overflow")` |
| R5 | i32 加减乘溢出 | wrapping 行为，不需要 panic | — | 使用 LLVM `add`/`sub`/`mul` (无 `nsw`) |
| R6 | f64→i32 NaN/Inf | `i32(n: f64)` 且 n 为 NaN 或 Inf | 内置函数内 | `k_panic("cannot convert NaN/Inf to i32")` |

**非安全检查 (编译器保证或语言设计规避):**
- f64 运算遵循 IEEE 754，不检查
- 变量强制初始化，无未初始化内存
- 无指针/无 NULL，无 use-after-free
- Arena 统一回收，无 double-free

---

## 11. 跨平台编译

### 11.1 设计目标

Kang 编译器产出的可执行文件是跨平台的。通过指定目标平台三元组，可在任意开发机上编译出运行在另一平台的自包含可执行文件。

**核心策略：**

- **LLVM IR** 是目标无关的中间表示，`kang codegen` 产出的 `.ll` 文件可在任意平台间传递
- **LLVM target triple** 决定最终机器码的目标架构和 OS，由编译器在 codegen 阶段设置
- **musl libc** 作为 C 运行时，静态链接进可执行文件，消除对系统 glibc 版本的依赖
- **kangrt** 用 musl 编译，随目标平台交叉编译

### 11.2 目标平台检测

默认目标平台在编译器**编译时动态检测**当前环境：

| 检测项 | 来源 | 示例 |
|--------|------|------|
| CPU 架构 | `std::env::consts::ARCH` / LLVM host triple | `aarch64`, `x86_64` |
| 操作系统 | `std::env::consts::OS` | `macos`, `linux`, `windows` |
| ABI | 平台推断 | `gnu`, `musl`, `msvc` |

默认 target triple 由 `inkwell::targets::TargetMachine::get_default_triple()` 在运行时获取，对应 LLVM 对当前宿主机的检测结果。

### 11.3 指定目标平台

通过 `--target=<triple>` 参数覆盖默认目标，支持任意 LLVM 兼容的 target triple：

```bash
# 在本机 (macOS arm64) 上编译 Linux x86_64 可执行文件
kang build hello.kang --target=x86_64-unknown-linux-musl -o hello

# 在 Linux 上编译 macOS arm64 可执行文件
kang build hello.kang --target=aarch64-apple-darwin -o hello

# 生成目标无关的 LLVM IR (无 --target 时默认 host triple)
kang codegen hello.kang -o hello.ll

# 为特定目标生成 LLVM IR
kang codegen hello.kang --target=x86_64-unknown-linux-musl -o hello.ll
```

### 11.4 musl 静态链接

全部内置函数 (`k_*`) 链接 musl libc 而非系统 libc：

| 对比 | musl | glibc |
|------|------|-------|
| 许可证 | MIT | LGPL |
| 静态链接 | 一等支持 | 不推荐（nss/iconv 等需动态加载） |
| 可执行文件大小 | ~100KB-500KB | 动态链接依系统版本 |
| 跨发行版兼容 | 完全自包含 | glibc 版本绑定 |
| 目标平台 | 广泛 (Linux/BSDs/minimal) | 仅 Linux (glibc) |

**实现方式：**

- `kangrt` 以 musl 为目标编译，`Cargo.toml` 中指定 `target = "x86_64-unknown-linux-musl"` 等
- 交叉编译时，Rust 通过 `rustup target add <target>` 安装对应 musl target
- 链接阶段将 `libkangrt.a` 与 musl 静态链接，产出自包含可执行文件

### 11.5 支持的平台矩阵

Kang 编译器自身运行在 macOS / Linux，可为目标平台交叉编译：

| 目标平台 | triple | 状态 |
|----------|--------|------|
| Linux x86_64 (musl) | `x86_64-unknown-linux-musl` | 支持 |
| Linux aarch64 (musl) | `aarch64-unknown-linux-musl` | 支持 |
| macOS x86_64 | `x86_64-apple-darwin` | 支持 |
| macOS aarch64 | `aarch64-apple-darwin` | 支持 (默认) |
| Windows x86_64 | `x86_64-pc-windows-msvc` | 待支持 |

### 11.6 编译器自身的跨平台编译流程 (AI 实施要点)

编译器 (`kangc`) 代码中不硬编码 target triple。实现要点：

1. **`codegen/context.rs`** — `CodeGenContext::new()` 初始化时接收 `target_triple: Option<&str>`，为 `None` 时调用 `TargetMachine::get_default_triple()` 动态检测
2. **`kang build` 子命令** — 设置 `TargetMachine` 的 triple、CPU、features，生成 `.o` 后调用系统链接器
3. **kangrt 交叉编译** — `kang build --target=x86_64-unknown-linux-musl` 时自动检测是否需要交叉编译 kangrt；首次使用时通过 `rustup` + `cargo build --target` 构建目标平台的 `libkangrt.a`
4. **链接阶段** — 调用 `clang` 或 `ld.lld` 将 `kangc 产出的 .o` + `libkangrt.a` + `musl` 链接为最终可执行文件
