# Kang 语言参考手册 v2

## 概述

Kang 是一门严格静态类型的教学用语言，语法简洁，无未定义行为。支持 REPL 交互执行和 AOT 编译为独立二进制。

### 最小程序

```kang
def main() -> i32 {
    puts("hello, kang!");
    return 0;
}
```

编译运行: `kang run hello.kang`

---

## 1. 类型系统

### 基本类型

| 类型 | 说明 | 默认值 | 字面量示例 |
|------|------|--------|-----------|
| `i32` | 有符号 32 位整数，溢出回绕 | `0` | `42`, `-1`, `0` |
| `f64` | 双精度浮点 (IEEE 754) | `0.0` | `3.14`, `-0.5`, `1.0` |
| `str` | 不可变字符串，Arena 分配 | `""` | `"hello"`, `"line\n"` |
| `bool` | 布尔值 | `false` | `true`, `false` |
| `void` | 无返回值（仅返回类型） | — | — |

### 复合类型

```kang
// 数组: 元素类型可以是基本类型或结构体
// 禁止 [void]、禁止嵌套数组 [[T]]
var a:[i32] = [1, 2, 3];
var b:[str] = ["a", "b"];
var empty:[i32] = [];             // 空数组

// 结构体: 值类型，禁止 void 字段，禁止直接自引用
struct Point {
    x: f64;
    y: f64;
}

// 通过数组间接自引用（允许）
struct Node {
    value: i32;
    children: [Node];
}

// 空结构体: 语法允许但 v1 建议至少一个字段
struct Empty {}  // 不推荐
```

### 类型转换

**禁止隐式转换**。仅内置函数提供显式转换：

| 函数 | 方向 | 示例 |
|------|------|------|
| `str(n: i32) -> str` | i32 → str | `str(42)` → `"42"` |
| `str(n: f64) -> str` | f64 → str | `str(3.14)` → `"3.14"` |
| `str(b: bool) -> str` | bool → str | `str(true)` → `"true"` |
| `i32(s: str) -> (i32, bool)` | str → i32 | `i32("42")` → `(42, true)` |
| `f64(s: str) -> (f64, bool)` | str → f64 | `f64("3.14")` → `(3.14, true)` |
| `bool(s: str) -> (bool, bool)` | str → bool | `bool("true")` → `(true, true)` (仅接受 `"true"` / `"false"`) |
| `i32(n: f64) -> i32` | f64 → i32 | `i32(3.14)` → `3` (向零截断) |
| `f64(n: i32) -> f64` | i32 → f64 | `f64(42)` → `42.0` (无损) |

str→值 的三个转换函数返回 `(T, bool)`，第二个值为 `false` 表示转换失败。
`bool(s)` 仅接受精确小写的 `"true"` 和 `"false"`。

唯一的隐式转换例外: `+` 运算符任一侧为 `str` 时，另一侧自动转为字符串拼接。

```kang
var s:str = "x" + 42;        // → "x42"
var t:str = 1 + " is " + true;  // → "1 is true"
```

---

## 2. 语法参考

### 注释

```kang
// 行注释
/* 块注释 */
```

### 变量声明

```kang
// 单变量: 必须显式类型标注，必须初始化
var name:type = expr;

var x:i32 = 0;
var pi:f64 = 3.14159;
var greeting:str = "hello";
var flag:bool = true;

// 多接收: 解包多返回函数
var val:i32, ok:bool = i32("42");     // val=42, ok=true
var val:i32, _ = i32("abc");          // 只取第一个, _ 丢弃第二个
```

### 赋值

```kang
// 左值: 变量名、数组索引、结构体字段
x = 42;
arr[i] = value;
point.x = 10.0;

// str 不可变，索引不能作为左值:
// s[0] = "x";  ← 编译错误!
```

### 函数

```kang
// 完整形式
def add(a:i32, b:i32) -> i32 {
    return a + b;
}

// void 返回: 用 return; (不带表达式)
def greet(name:str) -> void {
    puts("hello, " + name);
    return;
}

// 无参函数
def answer() -> i32 {
    return 42;
}
```

**多返回值**:
```kang
// 返回类型用括号包裹两个类型
def divide(a:i32, b:i32) -> (i32, i32) {
    var q:i32 = a / b;
    var r:i32 = a - q * b;
    return q, r;
}

// 调用方: 二值接收解包
var quot:i32, rem:i32 = divide(10, 3);  // quot=3, rem=1

// 单接收取第一个值 (其余丢弃)
var q:i32 = divide(10, 3);              // q=3

// _ 占位丢弃
var _, rem:i32 = divide(10, 3);         // 只取余数
var q:i32, _ = divide(10, 3);           // 只取商
```

**规则**:
- 所有参数和返回值必须显式标注类型
- 非 void 函数的所有代码路径必须 return，返回值数量(1或2)/类型必须匹配
- 函数名与变量名共享命名空间，不可重名
- 不支持用户自定义函数重载（仅内置函数可重载）

### 块与作用域

```kang
// 词法作用域，内层可遮蔽外层变量
{
    var x:i32 = 1;
    {
        var x:i32 = 2;  // 遮蔽外层 x
        x = 3;          // 修改内层 x
    }
    // x 恢复为 1
}
```

### 分支

```kang
// if-then (无 else)
if x > 0 then puts("positive");

// if-then-else
if x >= 0 then {
    puts("non-negative");
} else {
    puts("negative");
}

// else 可选; 嵌套时 else 绑定最近的 if
if a > 0 then
    if b > 0 then puts("both positive");
    else puts("b <= 0");           // 绑定内层 if
```

**条件必须是 `bool` 类型**，不支持隐式真值判断。

### 循环

```kang
// for var name:type = init, cond, step in { body }
// 循环变量在循环体内有效，循环结束后不可访问

// 经典计数循环
for var i:i32 = 0, i < 10, i = i + 1 in {
    var s:str = str(i);
    puts(s);
}

// 倒数
for var n:i32 = 10, n > 0, n = n - 1 in {
    puts(str(n));
}
```

**语义**:
1. `init`: 循环开始前求值一次
2. `cond`: 每次迭代前求值，false 时退出
3. `step`: 每次迭代后执行
4. 无 `break`/`continue` (v1)

### 结构体

```kang
// 定义 (仅限顶层)
struct Token {
    kind: i32;
    lexeme: str;
    line: i32;
}

// 构造 (必须为每个字段提供值)
var t:Token = Token{kind: 1, lexeme: "if", line: 1};

// 字段访问 (读/写)
var k:i32 = t.kind;
t.lexeme = "else";
t.line = t.line + 1;
```

### 模块导入

```kang
// 导入其他文件中的顶层定义
// 语法: import alias { item1, item2 } from "path/file.kang";
import math { add, sub } from "./math.kang";
import io { puts, read_file } from "../io/utils.kang";

// 通过 alias.item 访问导入项
def main() -> i32 {
    var result:i32 = math.add(1, 2);
    io.puts(str(result));
    return 0;
}
```

**规则**:
- `import` 语句仅限顶层，不可出现在函数体内
- `alias` 为命名空间前缀，导入后通过 `alias.item` 访问
- `{ }` 内列出需要导入的顶层函数名或结构体名，逗号分隔
- 路径相对于当前源文件所在目录解析
- 所有顶层项默认可导入，无需 `pub`/`priv` 修饰符
- 被导入模块独立编译，链接期合并
- 循环导入通过分离编译支持

### 入口点

```kang
// AOT 编译入口: 必须返回 i32 作为进程退出码
def main() -> i32 {
    return 0;
}
```

---

## 3. 表达式

### 优先级表 (低 → 高)

| 优先级 | 运算符 | 结合性 | 说明 |
|--------|--------|--------|------|
| 1 | `\|\|` | 左 | 逻辑或 |
| 2 | `&&` | 左 | 逻辑与 |
| 3 | `==` `!=` | 左 | 等值比较 (同类型) |
| 4 | `<` `<=` `>` `>=` | 左 | 大小比较 (i32/f64) |
| 5 | `+` `-` | 左 | 加法/减法/拼接 |
| 6 | `*` `/` | 左 | 乘法/除法 |
| 7 | `-` `!` | 右 (一元) | 取负/逻辑非 |
| 8 | `a[i]` `f()` `obj.f` | 左 (后缀) | 索引/调用/字段 |

### 算术运算

```kang
// i32 之间或 f64 之间，不可混用
var a:i32 = 1 + 2 * 3;       // → 7
var b:i32 = (1 + 2) * 3;     // → 9
var c:f64 = 3.14 * 2.0;      // → 6.28
var d:i32 = -a;              // 取负
var e:i32 = 7 / 2;           // → 3 (整数除，向零截断)
var f:i32 = -7 / 2;          // → -3 (向零截断，非 -4)

// 混用 i32/f64 → 编译错误:
// var bad:f64 = 1 + 2.0;    // ❌
```

### 字符串拼接

```kang
var s:str = "hello, " + "world";   // → "hello, world"
var t:str = "x" + 42;              // → "x42" (自动转换)
var u:str = true + " is true";     // → "true is true"
```

### 索引

```kang
// 字符串索引: 返回单字符 str
var ch:str = "hello"[0];           // → "h"

// 数组索引
var x:i32 = arr[0];
arr[0] = 99;                       // 修改数组元素

// 索引越界 → 运行时 panic
```

### 逻辑运算

```kang
// 操作数必须是 bool
var r1:bool = true && false;       // → false
var r2:bool = true || false;       // → true
var r3:bool = !true;               // → false
var r4:bool = (1 > 0) && (2 < 3); // → true
```

---

## 4. 内置函数

### 输出

| 函数 | 说明 |
|------|------|
| `puts(s: str)` | 输出字符串到 stdout，自动追加换行 |
| `print(s: str)` | 输出字符串到 stdout，不追加换行 |
| `eprint(s: str)` | 输出到 stderr，自动追加换行 |

### 输入

| 函数 | 说明 |
|------|------|
| `read_file(path: str) -> (str, bool)` | 读取文件内容，bool=false 表示失败 |
| `read_line() -> (str, bool)` | 从 stdin 读一行，bool=false 表示失败 |

### 写入

| 函数 | 说明 |
|------|------|
| `write_file(path: str, content: str)` | 覆盖写入文件 |
| `append_file(path: str, content: str)` | 追加到文件末尾 |

### 查询

| 函数 | 说明 |
|------|------|
| `file_exists(path: str) -> bool` | 文件是否存在 |
| `file_size(path: str) -> (i32, bool)` | 文件字节数，bool=false 表示失败 |

### 数组

| 函数 | 说明 |
|------|------|
| `len(a: [T]) -> i32` | 数组元素个数 |
| `len(s: str) -> i32` | 字符串长度 (字符数) |
| `push(arr: [T], elem: T)` | 在数组末尾追加元素 |

### 多返回值与错误处理

输入/查询/str转换 函数返回 `(value, bool)`，第二个值为 `false` 表示操作失败。调用方通过多接收解包检查：

```kang
// 显式错误检查
var content:str, ok:bool = read_file("config.kang");
if ok then {
    puts(content);
} else {
    eprint("failed to read file");
}

// 忽略错误 (单接收取第一值)
var content:str = read_file("config.kang");  // 失败时 content=""

// 只检查成败
var _, ok:bool = read_file("config.kang");
```

`f64(n: i32)` 和 `str()` 系列永不失败。`i32(n: f64)` NaN/Inf 时 panic。

---

## 5. 运行时安全

Kang 设计原则: **所有行为必须有确定结果，零未定义行为**。

| 情况 | 结果 |
|------|------|
| 数组/字符串索引越界 | 运行时 panic |
| 整数除零 | 运行时 panic |
| `INT_MIN / -1` 溢出 | 运行时 panic |
| `i32(n: f64)` NaN/Inf | 运行时 panic |
| i32 加减乘溢出 | 二进制补码回绕 |
| f64 运算 | IEEE 754 (含 NaN/Inf) |
| 未初始化变量 | 编译期强制初始化 + 默认零值 |
| return 遗漏 | 编译错误 |
| 内存释放 | 全局 Arena, 程序退出时统一回收 |

---

## 6. 常用模式

### 遍历字符串

```kang
def each_char(s:str) -> void {
    for var i:i32 = 0, i < len(s), i = i + 1 in {
        var ch:str = s[i];
        // 处理 ch
    }
    return;
}
```

### 构建数组

```kang
def collect_numbers() -> [i32] {
    var result:[i32] = [];
    for var i:i32 = 0, i < 10, i = i + 1 in {
        push(result, i);
    }
    return result;
}
```

### 简单词法分析器骨架

```kang
struct Token {
    kind: i32;
    lexeme: str;
    line: i32;
}

// Token kind 常量
def TK_NUMBER() -> i32 { return 1; }
def TK_IDENT() -> i32 { return 2; }
def TK_STRING() -> i32 { return 3; }

def is_digit(ch:str) -> bool {
    return ch == "0" || ch == "1" || ch == "2" || ch == "3" || ch == "4"
        || ch == "5" || ch == "6" || ch == "7" || ch == "8" || ch == "9";
}

def lex(source:str) -> [Token] {
    var tokens:[Token] = [];
    var line:i32 = 1;

    for var i:i32 = 0, i < len(source), i = i + 1 in {
        var ch:str = source[i];

        if ch == "\n" then line = line + 1;

        if is_digit(ch) then {
            var tok:Token = Token{kind: TK_NUMBER(), lexeme: ch, line: line};
            push(tokens, tok);
        }
    }
    return tokens;
}
```

### 多返回值错误检查

```kang
def safe_read(path:str) -> void {
    var content:str, ok:bool = read_file(path);
    if ok then {
        puts(content);
    } else {
        eprint("failed to read: " + path);
    }
    return;
}

// 单接收取第一值 (不检查错误，适合原型代码)
def quick_read(path:str) -> str {
    var content:str = read_file(path);
    return content;
}
```

### 结构体全字段更新

```kang
// 结构体是值类型，逐字段修改
struct Point { x: f64; y: f64; }

def move_right(p:Point, dx:f64) -> Point {
    p.x = p.x + dx;
    return p;
}
```

---

## 7. 限制与注意事项

1. **无指针**: 所有数据通过值和 Arena 管理，无手动内存操作
2. **模块系统**: 支持 `import alias { items } from "path"` 跨文件导入，编译单元分离编译 + 链接
3. **无 break/continue (v1)**: 循环退出须通过条件判断
4. **无泛型**: 每种类型组合需要各自处理
5. **无闭包/lambda**: 函数不能嵌套定义，不能捕获外部变量
6. **结构体仅限顶层**: 不能在函数内部定义结构体
7. **禁止 `[void]`**: 数组元素不能是 void 类型
8. **禁止嵌套数组**: `[[i32]]` 不合法，嵌套结构需用结构体包装
9. **i32/f64 不可混用**: `1 + 2.0` 是编译错误
10. **i32 除法向零截断**: `-7 / 2` → `-3`，非向下取整
11. **多返回类型不可作为变量类型**: `(i32, bool)` 仅用于函数返回，不能 `var x:(i32,bool) = f()`
12. **函数不支持重载**: 每个函数名在作用域内唯一 (内置函数除外)

---

## 8. 与 C 语言的差异速查

| C | Kang |
|---|------|
| `int` | `i32` (固定 32 位) |
| `double` | `f64` |
| `char*` | `str` (不可变，Arena) |
| `int arr[]` | `[i32]` |
| `struct` | `struct` (无指针字段，值语义) |
| `NULL` | 无 (默认零值代替) |
| `printf` | `puts` / `print` / `eprint` |
| `malloc/free` | 无 (Arena 自动管理) |
| undefined behavior | **零 UB**: panic 或 wrapping |
| 错误处理哨兵 | 多返回值 `(T, bool)` |
| 隐式类型转换 | 禁止 (除 str +) |
| 单文件 | 多文件模块系统 + `import` |
| 链接 | 分离编译 + 静态链接 |

---

## 9. 模块系统

Kang v2 通过编译单元分离编译 + 静态链接实现跨文件代码组织。

### 基本用法

```kang
// === math.kang ===
def add(a:i32, b:i32) -> i32 {
    return a + b;
}

def sub(a:i32, b:i32) -> i32 {
    return a - b;
}

// === main.kang ===
import m { add, sub } from "./math.kang";

def main() -> i32 {
    var x:i32 = m.add(1, 2);
    var y:i32 = m.sub(5, 3);
    puts(str(x + y));  // 输出 "5"
    return 0;
}
```

编译: `kang build main.kang` — 编译器自动发现依赖、编译 `math.kang`、链接所有 .o。

### 导入语法

```
import alias { item1, item2, ... } from "path/file.kang";
```

- `alias`: 命名空间前缀，在当前文件中唯一
- `{ }`: 显式导入列表，仅导入需要的顶层函数名或结构体名
- `"path"`: 被导入文件的路径，相对于导入文件所在目录解析

### 访问导入项

```kang
import lexer { Token, next_token } from "./lexer.kang";

def main() -> i32 {
    // 结构体类型通过 alias.TypeName 使用
    var tok:lexer.Token = lexer.next_token();
    var kind:i32 = tok.kind;
    return 0;
}
```

### 可见性规则

- 所有顶层函数和结构体默认可导入，无需 `pub`/`priv` 修饰符
- 导入方通过 `alias.item` 访问，当前文件的顶层项通过原名访问
- 不存在"重导出"或"传递导入"——导入项仅在导入文件中可见

### 编译模型

```
main.kang ──→ lex/parse/semantic/codegen ──→ main.o ──┐
                                                        ├──→ link ──→ a.out
math.kang ──→ lex/parse/semantic/codegen ──→ math.o ──┘
```

- 每个源文件独立编译为目标文件
- 导入模块时，先编译被导入文件，再编译导入方
- 链接阶段合并所有 .o + libkangrt.a

### 循环导入

循环导入通过分离编译支持：

```kang
// === a.kang ===
import b { f } from "./b.kang";

// === b.kang ===
import a { g } from "./a.kang";
```

编译器策略：被导入模块的签名先注册到符号表，体在链接期解析。v1 不验证循环导入的正确性（由开发者保证），运行时行为由 LLVM 链接器决定。

### 与 C 风格的对比

| 概念 | C (#include) | Kang (import) |
|------|-------------|---------------|
| 包含方式 | 文本复制/粘贴 | 独立编译单元 + 符号链接 |
| 命名空间 | 全局，易冲突 | `alias.item`，隔离 |
| 依赖图 | 隐式（头文件链） | 显式导入列表 |
| 接口/实现分离 | .h + .c | 不需要（统一源文件） |
| 循环引用 | 需头文件保护 | 签名先注册 + 链接期解析 |

### 自举示例

Kang 编译器本身可由多个 Kang 源文件组成：

```kang
// === lexer.kang ===
struct Token { kind: i32; lexeme: str; line: i32; }
def lex(source:str) -> [Token] { ... }

// === parser.kang ===
import lex { Token } from "./lexer.kang";
struct AstNode { kind: i32; children: [AstNode]; }
def parse(tokens:[lex.Token]) -> AstNode { ... }

// === main.kang ===
import lex { lex } from "./lexer.kang";
import parse { parse, AstNode } from "./parser.kang";
// AstNode 可通过 parse.AstNode 访问

def main() -> i32 {
    var source:str = read_file("input.kang");
    var tokens:[lex.Token] = lex.lex(source);
    var ast:parse.AstNode = parse.parse(tokens);
    return 0;
}
```
