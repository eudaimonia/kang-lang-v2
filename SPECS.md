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
    ▼  kang codegen      → LLVM IR (.ll)          标准 LLVM IR 文本
    │
    ▼  kang build        → 可执行文件              (内置 llc + link)
```

**设计原则：**
- 每个阶段 = 纯函数: `IR_in → IR_out`
- 输出可读文本，人可肉眼审查每步产出
- 所有阶段内建统计埋点，`--stats` 输出结构化 JSON
- AI 可独立开发/测试一个阶段；人类可在任意阶段审查输出

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
│       ├── builtins.rs           # 19 个内置函数
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
kang lex     <file> [-o <file>] [--stats]    仅词法分析, 输出 Token Stream
kang parse   <file> [-o <file>] [--stats]    词法+语法, 输出 AST
kang check   <file> [--stats]                词法+语法+语义, 仅报告错误/OK
kang codegen <file> [-o <file>] [--stats]    全前端+代码生成, 输出 LLVM IR
kang build   <file> [-o <binary>] [--stats]  全管线, 输出可执行文件
kang run     <file> [--stats]                编译+执行 (入口 def main)
kang repl                                    交互式 REPL

选项:
  --emit=tokens|ast|typed-ast|llvm-ir|object  在指定阶段截断并输出 IR
  --stats                                      输出各阶段统计数据 (JSON)
  -o <file>                                    输出文件路径
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

pub fn tokenize(source: &str, stats: &mut LexStats) -> Result<Vec<Token>, LexError>;
pub fn parse(tokens: &[Token], stats: &mut ParseStats) -> Result<ast::Program, ParseError>;
pub fn check(program: &ast::Program, stats: &mut SemanticStats) -> Result<semantic::TypedProgram, Vec<SemanticError>>;
pub fn codegen(program: &semantic::TypedProgram, rt_path: &Path, stats: &mut CodeGenStats) -> Result<CodeGenResult, CodeGenError>;

// 便捷包装
pub fn compile_full(source: &str, rt_path: &Path) -> Result<(Vec<u8>, CompilerStats), Error>;
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

## 7. 里程碑

| M# | 名称 | 模块 | LOC | 估时 | 关键交付物 |
|----|------|------|-----|------|-----------|
| M1 | 前端 | lexer + parser + AST + stats | 1,600 | 1 周 | `kang lex` / `kang parse` 可运行, Token Stream 和 AST 可序列化 |
| M2 | 语义 | semantic (types/scope/checker/flow) | 900 | 1 周 | `kang check` 可运行, 17 个测试文件正确分类 |
| M3 | 运行时 | kangrt (arena/builtins/panic) | 500 | 3-4天 | `libkangrt.a` 可独立编译, 内置函数可测试 |
| M4 | 代码生成 | codegen (context/types/expr/stmt/builtins/runtime) | 1,800 | 1.5-2周 | `kang codegen` 输出合法 LLVM IR |
| M5 | AOT+CLI | main.rs (子命令/--emit/--stats/管线串联) | 300 | 2-3天 | `kang run hello.kang` 端到端 |
| M6 | REPL | repl.rs (readline/JIT/符号表持久化) | 500 | 3-4天 | REPL 交互多行输入 |
| M7 | 测试 | tests/ (集成测试/边界/README) | 1,000 | 3-4天 | 全部回归通过 |
| **总计** | | | **~6,600** | **5-6周** |

### 依赖图

```
M1 (前端)
  │
  ▼
M2 (语义) ──┐
             │
M3 (运行时) ─┤  ← 可与 M1/M2 并行
             │
             ▼
          M4 (代码生成)
             │
        ┌────┴────┐
        ▼         ▼
     M5 (AOT)  M6 (REPL)
        │         │
        └────┬────┘
             ▼
          M7 (测试)
```

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
| 运行时 | musl libc | 静态链接, libkangrt.a |
