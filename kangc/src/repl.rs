// REPL — 交互式解释执行
// 多行续写、元命令、增量编译执行
// 函数/结构体定义累积执行，语句/表达式即时编译运行

use crate::error::{emit_diagnostic, KangError, ParseError};
use crate::lexer;
use crate::parser::{self, LineResult};
use crate::stats::LexStats;
use crate::{compile_to_stage, PipelineStage};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process;

// ── REPL 状态 ────────────────────────────────────────────────────────────────

struct ReplState {
    /// 累积的函数和结构体定义（源码文本），每次执行语句/表达式时注入
    defs_source: String,
    /// 累积的源码行缓冲区（多行续写）
    line_buffer: String,
    /// 行号计数器
    line_no: usize,
    /// 是否在继续读取多行输入
    continuing: bool,
    /// 临时目录，存放编译产物
    tmp_dir: PathBuf,
    /// 计数器用于生成唯一文件名
    counter: u64,
}

impl ReplState {
    fn new() -> Self {
        let tmp_dir = std::env::temp_dir().join(format!("kang_repl_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp_dir);
        ReplState {
            defs_source: String::new(),
            line_buffer: String::new(),
            line_no: 0,
            continuing: false,
            tmp_dir,
            counter: 0,
        }
    }

    fn next_id(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }
}

impl Drop for ReplState {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp_dir);
    }
}

// ── REPL 入口 ────────────────────────────────────────────────────────────────

pub fn run_repl() {
    let mut state = ReplState::new();

    println!("Kang {} REPL", env!("CARGO_PKG_VERSION"));
    println!("输入 .help 查看帮助, .quit 退出\n");

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        let prompt = if state.continuing { ".. " } else { "> " };
        if write!(stdout, "{}", prompt).is_err() || stdout.flush().is_err() {
            break; // stdout 关闭或断开，退出 REPL
        }

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                // EOF
                println!();
                break;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("读取输入失败: {}", e);
                break;
            }
        }

        let trimmed = line.trim();

        // 空行 → 继续等待（多行模式下保留空行）
        if trimmed.is_empty() {
            if state.continuing {
                state.line_buffer.push('\n');
            }
            continue;
        }

        // 元命令
        if trimmed.starts_with('.') {
            if let Some(cmd) = handle_meta_command(trimmed, &mut state) {
                match cmd {
                    MetaResult::Quit => break,
                    MetaResult::Continue => {}
                }
            }
            // 元命令后清空多行缓冲区
            if state.continuing {
                state.continuing = false;
                state.line_buffer.clear();
            }
            continue;
        }

        // 累积输入行
        state.line_no += 1;
        if state.line_buffer.is_empty() {
            state.line_buffer = trimmed.to_string();
        } else {
            state.line_buffer.push('\n');
            state.line_buffer.push_str(trimmed);
        }

        // 尝试 lex → parse_line
        match try_parse(&state.line_buffer) {
            Ok(line_result) => {
                // 解析成功，执行
                if let Err(e) = execute_line(&mut state, line_result) {
                    emit_diagnostic(&e, &state.line_buffer, "<repl>");
                }
                state.line_buffer.clear();
                state.continuing = false;
            }
            Err(ParseError { is_incomplete: true, .. }) => {
                // 输入不完整，继续读取
                state.continuing = true;
            }
            Err(e) => {
                // 语法错误，报错并清空缓冲区
                emit_diagnostic(
                    &KangError::Parse(e),
                    &state.line_buffer,
                    "<repl>",
                );
                state.line_buffer.clear();
                state.continuing = false;
            }
        }
    }

    // 清理
    let _ = std::fs::remove_dir_all(&state.tmp_dir);
}

// ── 词法 + 行解析 ────────────────────────────────────────────────────────────

fn try_parse(source: &str) -> Result<LineResult, ParseError> {
    let mut lex_stats = LexStats::default();
    let tokens = lexer::tokenize(source, &mut lex_stats).map_err(|e| {
        // 词法错误重新包装为 ParseError，供 REPL 统一错误处理
        ParseError {
            msg: e.msg,
            line: e.line,
            col: e.col,
            span: e.span,
            is_incomplete: false,
        }
    })?;
    parser::parse_line(&tokens)
}

// ── 行执行 ────────────────────────────────────────────────────────────────────

fn execute_line(state: &mut ReplState, line: LineResult) -> Result<(), KangError> {
    match line {
        LineResult::FuncDef(func) => {
            // 将函数定义的源码追加到 defs_source
            // 使用简化的序列化：从 AST 节点重建源码
            let func_source = format!("def {}(", func.name);
            let params: Vec<String> = func
                .params
                .iter()
                .map(|(n, t)| format!("{}:{}", n, type_to_str(t)))
                .collect();
            let body_source = stmts_to_source(&func.body);
            let ret_type = return_type_to_str(&func.return_type);
            let full = format!(
                "{}{}) -> {} {{\n{}\n}}\n",
                func_source,
                params.join(", "),
                ret_type,
                body_source
            );
            state.defs_source.push_str(&full);
            println!("<function {}>", func.name);
        }
        LineResult::StructDef(s) => {
            let fields: Vec<String> = s
                .fields
                .iter()
                .map(|(n, t)| format!("{}:{};", n, type_to_str(t)))
                .collect();
            let full = format!("struct {} {{\n{}\n}}\n", s.name, fields.join("\n"));
            state.defs_source.push_str(&full);
            println!("<struct {}>", s.name);
        }
        LineResult::Stmt(stmt) => {
            let stmt_source = stmt_to_source(&stmt);
            let program_source = build_program_source(&state.defs_source, &stmt_source, false);
            compile_and_run(state, &program_source)?;
        }
        LineResult::Import(imp) => {
            // 读取被导入文件，将其定义追加到 REPL 累积源码
            let import_path = resolve_import_path(&imp.path, None);
            match std::fs::read_to_string(&import_path) {
                Ok(source) => {
                    state.defs_source.push_str(&source);
                    state.defs_source.push('\n');
                    println!("<imported {} as {}>", import_path.display(), imp.alias);
                }
                Err(e) => {
                    eprintln!("无法导入 {}: {}", imp.path, e);
                }
            }
        }
        LineResult::Expr(expr) => {
            let expr_source = expr_to_source(&expr);
            // 返回 void 的内置函数调用不需要 puts(str()) 包裹
            let is_void_call = is_void_builtin_call(&expr);
            let program_source = build_program_source(
                &state.defs_source,
                &expr_source,
                !is_void_call,
            );
            compile_and_run(state, &program_source)?;
        }
    }
    Ok(())
}

/// 解析 import 路径相对于当前工作目录
fn resolve_import_path(path: &str, _current_file: Option<&str>) -> PathBuf {
    PathBuf::from(path)
}

/// 构建完整的 Kang 程序源码：累积定义 + 入口函数
fn build_program_source(defs: &str, body: &str, is_expr: bool) -> String {
    let mut source = String::new();
    source.push_str(defs);
    if is_expr {
        // 表达式: 求值并用 puts(str()) 打印结果
        source.push_str(&format!(
            "def main() -> i32 {{\n  puts(str({}));\n  return 0;\n}}\n",
            body
        ));
    } else {
        // 语句: 直接执行
        source.push_str(&format!(
            "def main() -> i32 {{\n{}\n  return 0;\n}}\n",
            body
        ));
    }
    source
}

/// AOT 编译并执行生成的程序
fn compile_and_run(state: &mut ReplState, source: &str) -> Result<(), KangError> {
    let id = state.next_id();
    let src_path = state.tmp_dir.join(format!("repl_{}.kang", id));
    let obj_path = state.tmp_dir.join(format!("repl_{}.o", id));
    let exe_path = state.tmp_dir.join(format!("repl_{}", id));

    // 写入源码文件
    std::fs::write(&src_path, source).map_err(|e| {
        KangError::CodeGen(crate::error::CodeGenError {
            msg: format!("写入临时文件失败: {}", e),
        })
    })?;

    let src_path_str = src_path.to_string_lossy();

    // 编译到目标文件
    match compile_to_stage(source, &src_path_str, None, PipelineStage::Object, Some(&obj_path)) {
        Ok((_, _)) => {}
        Err(e) => {
            let _ = std::fs::remove_file(&src_path);
            return Err(e);
        }
    }

    // 链接
    let kangrt_path = find_or_build_kangrt();
    link_repl_executable(&obj_path, &kangrt_path, &exe_path);
    let _ = std::fs::remove_file(&obj_path);
    let _ = std::fs::remove_file(&src_path);

    // 执行
    let exit_status = process::Command::new(&exe_path).status().unwrap_or_else(|e| {
        eprintln!("无法执行程序: {}", e);
        process::exit(1);
    });

    let _ = std::fs::remove_file(&exe_path);

    if !exit_status.success() {
        // 运行错误不中断 REPL，仅报告
        let code = exit_status.code().unwrap_or(1);
        if code != 0 {
            // 非 0 退出码：程序可能已经通过 k_panic 输出了错误信息
        }
    }

    Ok(())
}

// ── 元命令 ────────────────────────────────────────────────────────────────────

enum MetaResult {
    Quit,
    Continue,
}

fn handle_meta_command(cmd: &str, state: &mut ReplState) -> Option<MetaResult> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    match parts[0] {
        ".quit" | ".exit" => {
            return Some(MetaResult::Quit);
        }
        ".help" => {
            println!("元命令:");
            println!("  .help       显示帮助");
            println!("  .quit       退出 REPL");
            println!("  .defs       显示已注册的定义");
            println!("  .clear      清空已注册的定义和多行缓冲");
            println!("  .stats      显示 REPL 统计信息");
            println!();
            println!("支持多行续写: 输入不完整的表达式或函数定义时自动进入续写模式");
            println!("表达式直接求值并打印结果, 语句编译后立即执行");
        }
        ".defs" => {
            if state.defs_source.is_empty() {
                println!("(无已注册定义)");
            } else {
                println!("{}", state.defs_source);
            }
        }
        ".clear" => {
            state.defs_source.clear();
            state.line_buffer.clear();
            state.continuing = false;
            println!("已清空所有定义和缓冲区");
        }
        ".stats" => {
            let def_count = state.defs_source.lines().count();
            println!("累积定义行数: {}", def_count);
            println!("已执行行数: {}", state.line_no);
        }
        _ => {
            eprintln!("未知元命令: {} (输入 .help 查看帮助)", parts[0]);
        }
    }
    Some(MetaResult::Continue)
}

// ── AST → 源码序列化（简化，用于重建可编译的源码） ──────────────────────────

use crate::ast;

/// 判断表达式是否是返回 void 的内置函数调用（puts, print 等）
fn is_void_builtin_call(expr: &ast::Expr) -> bool {
    /// 返回 void 的内置函数，调用这些函数时不需要 puts(str(...)) 包装
    const VOID_RETURN_BUILTINS: &[&str] = &["puts", "print", "eprint", "write_file", "append_file", "push"];
    if let ast::Expr::Call { func, .. } = expr {
        if let ast::Expr::Ident(name, ..) = func.as_ref() {
            return VOID_RETURN_BUILTINS.contains(&name.as_str());
        }
    }
    false
}

fn type_to_str(ty: &ast::Type) -> String {
    match ty {
        ast::Type::Base(bt) => format!("{}", bt),
        ast::Type::Array(bt) => format!("[{}]", bt),
    }
}

fn return_type_to_str(rt: &ast::ReturnType) -> String {
    match rt {
        ast::ReturnType::Single(ty) => type_to_str(ty),
        ast::ReturnType::Pair(t1, t2) => {
            format!("({}, {})", type_to_str(t1), type_to_str(t2))
        }
    }
}

fn stmt_to_source(s: &ast::Stmt) -> String {
    match s {
        ast::Stmt::VarDecl { bindings, init, .. } => {
            let bind_strs: Vec<String> = bindings
                .iter()
                .map(|b| match b {
                    ast::VarBinding::Named { name, ty } => {
                        format!("{}:{}", name, type_to_str(ty))
                    }
                    ast::VarBinding::Discard => "_".to_string(),
                })
                .collect();
            format!("var {} = {};", bind_strs.join(", "), expr_to_source(init))
        }
        ast::Stmt::Assign { lvalue, value, .. } => {
            format!("{} = {};", lvalue_to_source(lvalue), expr_to_source(value))
        }
        ast::Stmt::Return { values, .. } => {
            if values.is_empty() {
                "return;".to_string()
            } else {
                let vals: Vec<String> = values.iter().map(expr_to_source).collect();
                format!("return {};", vals.join(", "))
            }
        }
        ast::Stmt::Expr(e, ..) => format!("{};", expr_to_source(e)),
        ast::Stmt::If { condition, then_branch, else_branch, .. } => {
            let mut s = format!(
                "if {} then {}",
                expr_to_source(condition),
                stmt_to_source(then_branch)
            );
            if let Some(eb) = else_branch {
                s.push_str(&format!(" else {}", stmt_to_source(eb)));
            }
            s
        }
        ast::Stmt::For {
            var_name, var_type, start, end,
            step_lvalue, step_expr: _, body, ..
        } => {
            format!(
                "for var {}:{} = {}, {}, {} in {}",
                var_name,
                type_to_str(var_type),
                expr_to_source(start),
                expr_to_source(end),
                lvalue_to_source(step_lvalue),
                stmt_to_source(body)
            )
        }
        ast::Stmt::Block(stmts, ..) => {
            let inner: Vec<String> = stmts.iter().map(stmt_to_source).collect();
            format!("{{\n{}\n}}", inner.join("\n"))
        }
    }
}

fn stmts_to_source(stmts: &[ast::Stmt]) -> String {
    stmts.iter().map(stmt_to_source).collect::<Vec<_>>().join("\n")
}

fn expr_to_source(e: &ast::Expr) -> String {
    match e {
        ast::Expr::Ident(name, ..) => name.clone(),
        ast::Expr::IntLit(val, ..) => val.clone(),
        ast::Expr::FloatLit(val, ..) => val.clone(),
        ast::Expr::BoolLit(val, ..) => val.to_string(),
        ast::Expr::StrLit(val, ..) => format!("\"{}\"", val),
        ast::Expr::Binary { left, op, right, .. } => {
            format!("{} {} {}", expr_to_source(left), bin_op_str(op), expr_to_source(right))
        }
        ast::Expr::Unary { op, expr, .. } => {
            format!("{}{}", unary_op_str(op), expr_to_source(expr))
        }
        ast::Expr::Call { func, args, .. } => {
            let args_str: Vec<String> = args.iter().map(expr_to_source).collect();
            format!("{}({})", expr_to_source(func), args_str.join(", "))
        }
        ast::Expr::Index { array, index, .. } => {
            format!("{}[{}]", expr_to_source(array), expr_to_source(index))
        }
        ast::Expr::FieldAccess { obj, field, .. } => {
            format!("{}.{}", expr_to_source(obj), field)
        }
        ast::Expr::StructLit { name, fields, .. } => {
            let fields_str: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{}:{}", k, expr_to_source(v)))
                .collect();
            format!("{} {{ {} }}", name, fields_str.join(", "))
        }
        ast::Expr::ArrayLit(elems, ..) => {
            let elems_str: Vec<String> = elems.iter().map(expr_to_source).collect();
            format!("[{}]", elems_str.join(", "))
        }
    }
}

fn lvalue_to_source(lv: &ast::LValue) -> String {
    match lv {
        ast::LValue::Ident(name, ..) => name.clone(),
        ast::LValue::Index { array, index, .. } => {
            format!("{}[{}]", expr_to_source(array), expr_to_source(index))
        }
        ast::LValue::FieldAccess { obj, field, .. } => {
            format!("{}.{}", expr_to_source(obj), field)
        }
    }
}

fn bin_op_str(op: &ast::BinOp) -> &'static str {
    match op {
        ast::BinOp::Add => "+",
        ast::BinOp::Sub => "-",
        ast::BinOp::Mul => "*",
        ast::BinOp::Div => "/",
        ast::BinOp::Eq => "==",
        ast::BinOp::Neq => "!=",
        ast::BinOp::Lt => "<",
        ast::BinOp::Le => "<=",
        ast::BinOp::Gt => ">",
        ast::BinOp::Ge => ">=",
        ast::BinOp::And => "&&",
        ast::BinOp::Or => "||",
    }
}

fn unary_op_str(op: &ast::UnaryOp) -> &'static str {
    match op {
        ast::UnaryOp::Neg => "-",
        ast::UnaryOp::Not => "!",
    }
}

// ── 链接 ──────────────────────────────────────────────────────────────────────

fn find_or_build_kangrt() -> PathBuf {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("找不到 workspace 根目录")
        .to_path_buf();

    let lib_dir = project_root.join("target").join("release");
    let lib_path = lib_dir.join("libkangrt.a");

    if lib_path.exists() {
        return lib_path;
    }

    eprintln!("正在构建 kangrt (首次需要编译，后续将复用)...");
    let status = process::Command::new("cargo")
        .args(["build", "--release", "-p", "kangrt"])
        .current_dir(&project_root)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("无法启动 cargo 构建 kangrt: {}", e);
            process::exit(1);
        });

    if !status.success() {
        eprintln!("kangrt 构建失败");
        process::exit(1);
    }

    lib_path
}

fn link_repl_executable(obj_path: &PathBuf, kangrt_path: &PathBuf, out_path: &PathBuf) {
    let status = process::Command::new("cc")
        .arg(obj_path)
        .arg(kangrt_path)
        .arg("-o")
        .arg(out_path)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("无法启动链接器 cc: {}", e);
            process::exit(1);
        });

    if !status.success() {
        eprintln!("链接失败");
        process::exit(1);
    }
}
