// 错误类型定义 + ariadne 诊断格式化，全模块共用

use ariadne::{Color, Label, Report, ReportKind, Source};
use std::ops::Range;

// ── 错误类型 ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum KangError {
    Lex(LexError),
    Parse(ParseError),
}

#[derive(Debug, Clone)]
pub struct LexError {
    pub msg: String,
    pub line: usize,
    pub col: usize,
    pub span: Range<usize>,
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub msg: String,
    pub line: usize,
    pub col: usize,
    pub span: Range<usize>,
}

impl std::fmt::Display for KangError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            KangError::Lex(e) => write!(f, "词法错误: {} at {}:{}", e.msg, e.line, e.col),
            KangError::Parse(e) => write!(f, "语法错误: {} at {}:{}", e.msg, e.line, e.col),
        }
    }
}

impl std::error::Error for KangError {}

// ── 诊断格式化 ──────────────────────────────────────────────────────────────

/// 用 ariadne 输出彩色诊断报告到 stderr
pub fn emit_diagnostic(err: &KangError, source: &str, file_path: &str) {
    match err {
        KangError::Lex(e) => {
            Report::build(ReportKind::Error, file_path, e.span.start)
                .with_message("词法分析错误")
                .with_label(
                    Label::new((file_path, e.span.clone()))
                        .with_message(&e.msg)
                        .with_color(Color::Red),
                )
                .finish()
                .eprint((file_path, Source::from(source)))
                .unwrap();
        }
        KangError::Parse(e) => {
            Report::build(ReportKind::Error, file_path, e.span.start)
                .with_message("语法分析错误")
                .with_label(
                    Label::new((file_path, e.span.clone()))
                        .with_message(&e.msg)
                        .with_color(Color::Red),
                )
                .finish()
                .eprint((file_path, Source::from(source)))
                .unwrap();
        }
    }
}
