// 控制流分析 — 检查函数所有执行路径是否都有 return 语句 (F1)

use crate::ast::Stmt;

/// 检查语句列表是否在所有路径上保证 return
/// 用于 F1: 非 void 函数必须所有代码路径都返回
pub fn all_paths_return(body: &[Stmt]) -> bool {
    stmts_return(body)
}

/// 语句序列是否在某条语句处保证 return
fn stmts_return(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| stmt_returns(s))
}

/// 单条语句是否保证 return（覆盖从该语句出发的所有路径）
fn stmt_returns(s: &Stmt) -> bool {
    match s {
        Stmt::Return { .. } => true,

        Stmt::If { then_branch, else_branch, .. } => {
            // if-then-else: 两个分支都保证 return 才行
            match else_branch {
                Some(else_s) => stmt_returns(then_branch) && stmt_returns(else_s),
                None => false, // 无 else: false 路径不返回
            }
        }

        Stmt::Block(stmts, ..) => stmts_return(stmts),

        // for 循环体可能不执行，不保证 return
        Stmt::For { .. } => false,

        // 表达式语句、变量声明、赋值：不保证 return
        Stmt::Expr(..) | Stmt::VarDecl { .. } | Stmt::Assign { .. } => false,
    }
}
