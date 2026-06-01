## checker.rs 问题总结

| 文件 | 行号 | 问题描述 |
|------|------|----------|
| `kangc/src/semantic/checker.rs` | L576-L605 | **双重检查**：`resolve_lvalue_type` 中调用 `check_expr` 导致左值子表达式被检查两次，passes 计数多计 |
| `kangc/src/semantic/checker.rs` | L487-L498 | **passes 计数不一致**：多接收 var 声明中每个 binding 各计一次 passes，单接收只计一次 |
| `kangc/src/semantic/checker.rs` | L993-L1001 | **字符串拼接漏判**：`peek_expr_type` 对嵌套表达式保守返回 `i32`，可能导致 `f() + "hello"` 误报 |
| `kangc/src/semantic/checker.rs` | L1482-L1494 | **span 不精确**：结构体缺少/多余字段的错误始终指向整个 struct-lit，而非具体字段位置 |
| `kangc/src/semantic/checker.rs` | L168-L173 | **span 硬编码 0..0**：import 错误没有带位置信息 |
| `kangc/src/semantic/checker.rs` | L1548-L1560 | **line/col 始终为 0**：`error()` 辅助函数不填充 line/col，影响未来 LSP 等场景 |
