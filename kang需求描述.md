## Kang 语言项目需求

### 定位
教学用 toy 语言，严格静态类型，支持 JIT REPL 和 AOT 编译到二进制。

### 类型系统

**基本类型**
- `i32`：有符号 32 位整数，溢出采用二进制补码回绕（wrapping）
- `f64`：双精度浮点
- `str`：不可变字符串，全局 Arena 管理
- `bool`：true/false
- `void`：无返回值

**复合类型**
- `[T]`：数组，元素类型 T 可以是 i32/f64/str/bool/结构体，禁止 `[void]`
- 结构体：`struct Name { field:Type; ... }`，值类型
- 二值返回类型：`(T1, T2)` 仅用于函数返回类型，不可作为变量类型或结构体字段
  - 字段类型禁止 `void`
  - 禁止直接或间接自引用（`struct Node { next: Node }`），仅允许通过数组间接引用（`struct Node { children: [Node] }`）

**默认值**
每种类型有确定的零值，杜绝未定义行为：
- `i32` → `0`
- `f64` → `0.0`
- `str` → `""`
- `bool` → `false`
- 结构体 → 逐字段递归取默认值
- 数组 → `[]`（空数组）

**转换规则**
- 所有变量和函数参数必须显式标注类型，禁止隐式转换
- 例外：`+` 运算中任一操作数为 `str` 时，另一操作数自动转换为字符串用于拼接

### 语法

**字面量**
- 整数：`42`、`-1`
- 浮点：`3.14`、`-1.0`
- 字符串：`"hello"`（双引号包裹），支持转义 `\n` `\t` `\\` `\"` `\0`
- 布尔：`true`、`false`
- 数组：`[elem0, elem1, ...]`

**注释**
- 行注释：`// 注释内容`
- 块注释：`/* 注释内容 */`

**语句**
- 块：`{ }` 包裹多条语句，用分号 `;` 分隔或换行
- 函数：`def name(params) -> (T1, T2) { body }`，单返回可省略括号
- 变量（单接收）：`var name:type = expr;`（必须初始化）
- 变量（二值接收）：`var n1:T1, n2:T2 = expr;`，最多两个，`_` 占位丢弃某个值
  - 单接收取第一个值：`var n:T = f()`，若 f 返回二值则取第一值、丢弃第二值
- 返回：`return e1, e2;`（二值返回）、`return expr;`（单返回）、`return;`（void）
- 非 void 函数所有代码路径必须显式 return，返回值的数量/类型必须匹配声明
- 赋值：`lvalue = expr;`（lvalue 为变量名、`arr[i]`、`obj.field`；str 不可变，`s[i]` 禁止作为左值）
- 分支：`if cond then ... else ...`（条件必须是 bool，else 可选）
- 循环：`for var name:type = init, cond, step in { body }`
  - 循环变量在循环体内作用域有效，循环结束后不可访问
  - `init` 在循环前求值
  - `cond` 每次迭代前求值，必须是 bool，`false` 时退出
  - `step` 每次迭代后执行（赋值语句）
- 结构体定义（仅限顶层）：`struct Name { field:Type; ... }`
- 结构体构造：`Name{field: expr, ...}`，必须为每个字段提供值，禁止部分初始化
- 函数调用语句：`name(args);`（用于 `-> void` 函数，忽略返回值也可）

**表达式**
- 字面量 | 变量引用 | 二元运算 | 一元 `-expr`
| 函数调用 `name(args)` | 分组 `(expr)`
| 索引 `a[i]`（str 和数组，索引必须是 i32）
| 字段访问 `obj.field`

**命名规则**
- 函数名和变量名共享命名空间，同名冲突 → 编译错误
- 函数参数在函数体内不可被 `var` 重新声明
- 内层块变量可遮蔽外层同名变量
- `_` 为 discard 标识符，仅用于多接收 var 中占位，不绑定变量

**作用域**
- 词法作用域

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
- 输入（多返回值，第二个值为成功标记）：
  - `read_file(path) -> (str, bool)` — 读取文件，bool 为 false 表示失败
  - `read_line() -> (str, bool)` — 读取一行
- 写入：`write_file(path, content)`、`append_file(path, content)`
- 查询：`file_exists(path) -> bool`、`file_size(path) -> (i32, bool)`
- 转换（按参数类型重载区分）：
  - `str(n: i32) -> str` — 整数转字符串
  - `str(n: f64) -> str` — 浮点转字符串
  - `str(b: bool) -> str` — 布尔转字符串
  - `i32(s: str) -> (i32, bool)` — 字符串转整数，失败时 bool 为 false
  - `f64(s: str) -> (f64, bool)` — 字符串转浮点，失败时 bool 为 false
  - `bool(s: str) -> (bool, bool)` — 字符串转布尔（仅接受精确 `"true"` / `"false"`），失败时第二个 bool 为 false
  - `i32(n: f64) -> i32` — 浮点转整数（向零截断，NaN/Inf → panic）
  - `f64(n: i32) -> f64` — 整数转浮点（无损）
- 函数重载：仅内置函数支持按参数类型重载，用户自定义函数不支持

### 运行时安全
所有行为必须有确定结果，杜绝未定义行为：
- 数组/字符串索引越界 → panic（立即终止并输出错误信息）
- 整数除零 → panic
- i32 加/减/乘溢出 → 二进制补码回绕
- i32 除法溢出（`INT_MIN / -1`）→ panic
- f64 运算 → 遵循 IEEE 754（含 NaN/Inf 语义）

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
