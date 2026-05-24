## Kang 语言项目需求

### 定位
教学用 toy 语言，严格静态类型，支持 JIT REPL 和 AOT 编译到二进制。

### 类型系统

**基本类型**
- `i32`：有符号 32 位整数
- `f64`：双精度浮点
- `str`：不可变字符串，全局 Arena 管理
- `bool`：true/false
- `void`：无返回值

**复合类型**
- `[T]`：数组，连续存储，元素类型 T 无限制（支持基本类型和结构体）
- 结构体：`struct Name { field:Type; ... }`，值类型，可包含自身类型字段的数组（如 `[ASTNode]`）

**转换规则**
- 所有变量和函数参数必须显式标注类型，禁止隐式转换
- 例外：`+` 运算中任一操作数为 `str` 时，另一操作数自动转换为字符串用于拼接

### 语法

**字面量**
- 整数：`42`、`-1`
- 浮点：`3.14`、`-1.0`
- 字符串：`"hello"`（双引号包裹）
- 布尔：`true`、`false`
- 数组：`[elem0, elem1, ...]`

**注释**
- 行注释：`// 注释内容`
- 块注释：`/* 注释内容 */`

**语句**
- 块：`{ }` 包裹多条语句，用分号 `;` 分隔或换行
- 函数：`def name(param:type) -> type { body }`
- 变量：`var name:type = expr;`（必须初始化）
- 返回：`return expr;` 终止函数并返回值
- 赋值：`lvalue = expr;`（lvalue 为变量名、`arr[i]`、`obj.field`）
- 分支：`if cond then ... else ...`（条件必须是 bool，else 可选）
- 循环：`for var = init, cond, step in { body }`
  - `init` 在循环前求值
  - `cond` 每次迭代前求值，必须是 bool，`false` 时退出
  - `step` 每次迭代后执行（赋值语句）
- 结构体定义：`struct Name { field:Type; ... }`
- 结构体构造：`Name{field: expr, ...}`

**表达式**
- 字面量 | 变量引用 | 二元运算 | 一元 `-expr`
| 函数调用 `name(args)` | 分组 `(expr)`
| 索引 `a[i]`（str 和数组，索引必须是 i32）
| 字段访问 `obj.field`

**作用域**
- 词法作用域，内层块可遮蔽外层变量

### 运算符
- 算术：一元 `-expr`、二元 `+ - * /`（i32 之间或 f64 之间，不可混用）
- 比较：`< <= > >=`（i32 之间或 f64 之间，不可混用）
- 索引：`a[i]`，适用于 str（返回单字符 str）和数组（返回元素类型），索引必须是 i32
- 等值：`== !=`（同类型 i32/f64/str/bool）
- 字符串：`+` 拼接，任一操作数为 `str` 时，另一操作数（i32/f64/bool）自动转为字符串后拼接（如 `"x" + 1` → `"x1"`，`"x" + true` → `"xtrue"`）
- 逻辑：`&& || !`（bool）

### 内置函数（封装 libc，12 个）
- 长度：`len(s)` — 返回字符串长度或数组长度（按参数类型重载）
- 数组：`push(arr, elem)` — 向数组末尾追加元素
- 输出：`puts(s)`、`print(s)`、`eprint(s)`（stderr）
- 输入：`read_file(path)`、`read_line()`
- 写入：`write_file(path, content)`、`append_file(path, content)`
- 查询：`file_exists(path) -> bool`、`file_size(path) -> i32`
- 转换（按参数类型重载区分）：
  - `str(n: i32)` — 整数转字符串
  - `str(n: f64)` — 浮点转字符串
  - `str(b: bool)` — 布尔转字符串
  - `i32(s: str)` — 字符串转整数
  - `f64(s: str)` — 字符串转浮点
  - `bool(s: str)` — 字符串转布尔
- 函数重载：仅内置函数支持按参数类型重载，用户自定义函数不支持
- 错误处理用返回值哨兵（空串/false/-1）。哨兵可能与合法值重叠（如空文件返回空串），v1 接受此限制

### 内存管理
全局 Arena，程序退出统一回收，用户无感知。

### REPL 模式
- repl 支持完整的gnu readline绑定，内部支持bash快捷键
- 多行输入，不完整时续行提示 `....>`
- 支持 `:quit`、`:show`、`:reset`、`:load` 等命令
- JIT 增量编译执行，符号表跨输入累积

### AOT 模式
- 入口：`def main() -> i32`，返回值作为退出码
- 流程：源码 → LLVM IR → 优化 → 目标文件 → 链接 runtime → 可执行文件
- CLI：`kang build`/`kang run`/`kang check`/`kang emit-llvm`

### 实现
- 语言：Rust
- LLVM 绑定：inkwell
- 词法分析：logos
- 错误报告：ariadne
- 架构：词法→语法→语义→代码生成 四阶段，REPL 和 AOT 复用前三阶段
- 运行时：独立 Rust 库，函数前缀 `k_`，静态链接 musl libc，编译为 `libkangrt.a`
