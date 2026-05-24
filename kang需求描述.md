## Kang 语言项目需求

### 定位
教学用 toy 语言，严格静态类型，支持 JIT REPL 和 AOT 编译到二进制。

### 类型系统
- `num`：双精度浮点
- `str`：不可变字符串，全局 Arena 管理
- `bool`：true/false
- `void`：无返回值
- 所有变量和函数参数必须显式标注类型，禁止隐式转换

### 语法
- 函数：`def name(param:type) -> type body`
- 变量：`var name:type = expr`（必须初始化）
- 分支：`if cond then ... else ...`（条件必须是 bool）
- 循环：`for var = start, cond, step in body`

### 运算符
- 数值：`+ - * / < <= > >=`（num 之间）
- 比较：`== !=`（同类型 num/str/bool）
- 字符串：`+` 拼接（仅 str）
- 逻辑：`&& || !`（bool）

### 内置函数（封装 libc，12 个）
- 字符串：`len(s)`、`str(n)`
- 输出：`puts(s)`、`print(s)`、`eprint(s)`（stderr）
- 输入：`read_file(path)`、`read_line()`
- 写入：`write_file(path, content)`、`append_file(path, content)`
- 查询：`file_exists(path)`、`file_size(path)`
- 转换：`num(s)`（字符串转数字）
- 错误处理用返回值哨兵（空串/false/-1.0）

### 内存管理
全局 Arena，程序退出统一回收，用户无感知。

### REPL 模式
- repl 支持完整的gnu readline绑定，内部支持bash快捷键
- 多行输入，不完整时续行提示 `....>`
- 支持 `:quit`、`:show`、`:reset`、`:load` 等命令
- JIT 增量编译执行，符号表跨输入累积

### AOT 模式
- 入口：`def main() -> num`，返回值作为退出码
- 流程：源码 → LLVM IR → 优化 → 目标文件 → 链接 runtime → 可执行文件
- CLI：`kang build`/`kang run`/`kang check`/`kang emit-llvm`

### 实现
- 语言：Rust
- LLVM 绑定：inkwell
- 词法分析：logos
- 错误报告：ariadne
- 架构：词法→语法→语义→代码生成 四阶段，REPL 和 AOT 复用前三阶段
- 运行时：独立 Rust 库，函数前缀 `k_`，编译为 `libkangrt.a`
