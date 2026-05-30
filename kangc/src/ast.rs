// AST 类型定义 — parser/semantic/codegen 之间的共享数据契约
// 所有节点不含类型标注，Display 实现 S-expression 格式输出

use std::fmt;
use std::ops::Range;

// ── 类型 ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum BaseType {
    I32,
    F64,
    Str,
    Bool,
    Void,
    UserDef(String),
}

impl fmt::Display for BaseType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            BaseType::I32 => write!(f, "i32"),
            BaseType::F64 => write!(f, "f64"),
            BaseType::Str => write!(f, "str"),
            BaseType::Bool => write!(f, "bool"),
            BaseType::Void => write!(f, "void"),
            BaseType::UserDef(name) => write!(f, "{}", name),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Base(BaseType),
    Array(BaseType),
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Type::Base(bt) => write!(f, "(type \"{}\")", bt),
            Type::Array(bt) => write!(f, "(type \"[{}]\")", bt),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReturnType {
    Single(Type),
    Pair(Type, Type),
}

impl fmt::Display for ReturnType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ReturnType::Single(ty) => write!(f, "{}", ty),
            ReturnType::Pair(t1, t2) => write!(f, "({} {})", t1, t2),
        }
    }
}

// ── 表达式 — 每个 variant 携带 span 用于错误诊断
// ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            BinOp::Or => write!(f, "||"),
            BinOp::And => write!(f, "&&"),
            BinOp::Eq => write!(f, "=="),
            BinOp::Neq => write!(f, "!="),
            BinOp::Lt => write!(f, "<"),
            BinOp::Le => write!(f, "<="),
            BinOp::Gt => write!(f, ">"),
            BinOp::Ge => write!(f, ">="),
            BinOp::Add => write!(f, "+"),
            BinOp::Sub => write!(f, "-"),
            BinOp::Mul => write!(f, "*"),
            BinOp::Div => write!(f, "/"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            UnaryOp::Neg => write!(f, "-"),
            UnaryOp::Not => write!(f, "!"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    Binary {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
        span: Range<usize>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Range<usize>,
    },
    Call {
        func: Box<Expr>,
        args: Vec<Expr>,
        span: Range<usize>,
    },
    Index {
        array: Box<Expr>,
        index: Box<Expr>,
        span: Range<usize>,
    },
    FieldAccess {
        obj: Box<Expr>,
        field: String,
        span: Range<usize>,
    },
    IntLit(String, Range<usize>),
    FloatLit(String, Range<usize>),
    StrLit(String, Range<usize>),
    BoolLit(bool, Range<usize>),
    ArrayLit(Vec<Expr>, Range<usize>),
    StructLit {
        name: String,
        fields: Vec<(String, Expr)>,
        span: Range<usize>,
    },
    Ident(String, Range<usize>),
}

impl Expr {
    /// 优先级数字，用于需要时判断是否需要括号（当前 S-expr 输出未使用）
    #[allow(dead_code)]
    fn prec(&self) -> u8 {
        match self {
            Expr::Binary { op, .. } => match op {
                BinOp::Or => 1,
                BinOp::And => 2,
                BinOp::Eq | BinOp::Neq => 3,
                BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 4,
                BinOp::Add | BinOp::Sub => 5,
                BinOp::Mul | BinOp::Div => 6,
            },
            Expr::Unary { .. } => 7,
            _ => 8, // postfix / primary
        }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Expr::IntLit(v, ..) => write!(f, "(int-lit {})", v),
            Expr::FloatLit(v, ..) => write!(f, "(float-lit {})", v),
            Expr::StrLit(v, ..) => write!(f, "(str-lit {:?})", v),
            Expr::BoolLit(v, ..) => write!(f, "(bool-lit {})", v),
            Expr::Ident(name, ..) => write!(f, "{}", name),
            Expr::Binary { left, op, right, .. } => {
                write!(f, "({} {} {})", op, left, right)
            }
            Expr::Unary { op, expr, .. } => {
                write!(f, "({} {})", op, expr)
            }
            Expr::Call { func, args, .. } => {
                write!(f, "(call {} args=(", func)?;
                if let Some((last, rest)) = args.split_last() {
                    for a in rest {
                        write!(f, "{} ", a)?;
                    }
                    write!(f, "{}", last)?;
                }
                write!(f, "))")
            }
            Expr::Index { array, index, .. } => {
                write!(f, "(index {} {})", array, index)
            }
            Expr::FieldAccess { obj, field, .. } => {
                write!(f, "(. {} {})", obj, field)
            }
            Expr::ArrayLit(elems, ..) => {
                write!(f, "(array-lit")?;
                for e in elems {
                    write!(f, " {}", e)?;
                }
                write!(f, ")")
            }
            Expr::StructLit { name, fields, .. } => {
                write!(f, "(struct-lit \"{}\"", name)?;
                for (field_name, val) in fields {
                    write!(f, " ({} {})", field_name, val)?;
                }
                write!(f, ")")
            }
        }
    }
}

// 忽略 span 比较，因为 span 是位置元数据，非语义内容
impl PartialEq for Expr {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Expr::Binary { left: l1, op: o1, right: r1, .. },
             Expr::Binary { left: l2, op: o2, right: r2, .. }) => l1 == l2 && o1 == o2 && r1 == r2,
            (Expr::Unary { op: o1, expr: e1, .. },
             Expr::Unary { op: o2, expr: e2, .. }) => o1 == o2 && e1 == e2,
            (Expr::Call { func: f1, args: a1, .. },
             Expr::Call { func: f2, args: a2, .. }) => f1 == f2 && a1 == a2,
            (Expr::Index { array: a1, index: i1, .. },
             Expr::Index { array: a2, index: i2, .. }) => a1 == a2 && i1 == i2,
            (Expr::FieldAccess { obj: o1, field: f1, .. },
             Expr::FieldAccess { obj: o2, field: f2, .. }) => o1 == o2 && f1 == f2,
            (Expr::IntLit(v1, _), Expr::IntLit(v2, _)) => v1 == v2,
            (Expr::FloatLit(v1, _), Expr::FloatLit(v2, _)) => v1 == v2,
            (Expr::StrLit(v1, _), Expr::StrLit(v2, _)) => v1 == v2,
            (Expr::BoolLit(v1, _), Expr::BoolLit(v2, _)) => v1 == v2,
            (Expr::ArrayLit(v1, _), Expr::ArrayLit(v2, _)) => v1 == v2,
            (Expr::StructLit { name: n1, fields: f1, .. },
             Expr::StructLit { name: n2, fields: f2, .. }) => n1 == n2 && f1 == f2,
            (Expr::Ident(v1, _), Expr::Ident(v2, _)) => v1 == v2,
            _ => false,
        }
    }
}

// ── 左值 ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum LValue {
    Ident(String, Range<usize>),
    Index { array: Box<Expr>, index: Box<Expr>, span: Range<usize> },
    FieldAccess { obj: Box<Expr>, field: String, span: Range<usize> },
}

impl fmt::Display for LValue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LValue::Ident(name, ..) => write!(f, "{}", name),
            LValue::Index { array, index, .. } => write!(f, "(lvalue-index {} {})", array, index),
            LValue::FieldAccess { obj, field, .. } => {
                write!(f, "(lvalue-field {} {})", obj, field)
            }
        }
    }
}

// 忽略 span 比较
impl PartialEq for LValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (LValue::Ident(v1, _), LValue::Ident(v2, _)) => v1 == v2,
            (LValue::Index { array: a1, index: i1, .. },
             LValue::Index { array: a2, index: i2, .. }) => a1 == a2 && i1 == i2,
            (LValue::FieldAccess { obj: o1, field: f1, .. },
             LValue::FieldAccess { obj: o2, field: f2, .. }) => o1 == o2 && f1 == f2,
            _ => false,
        }
    }
}

// ── 语句 ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum VarBinding {
    Named { name: String, ty: Type },
    Discard,
}

impl fmt::Display for VarBinding {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            VarBinding::Named { name, ty } => write!(f, "({} {})", name, ty),
            VarBinding::Discard => write!(f, "_"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Stmt {
    VarDecl {
        bindings: Vec<VarBinding>,
        init: Box<Expr>,
        span: Range<usize>,
    },
    Assign {
        lvalue: LValue,
        value: Box<Expr>,
        span: Range<usize>,
    },
    Return {
        values: Vec<Expr>,
        span: Range<usize>,
    },
    If {
        condition: Box<Expr>,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
        span: Range<usize>,
    },
    For {
        var_name: String,
        var_type: Type,
        start: Box<Expr>,
        end: Box<Expr>,
        step_lvalue: LValue,
        step_expr: Box<Expr>,
        body: Box<Stmt>,
        span: Range<usize>,
    },
    Expr(Box<Expr>, Range<usize>),
    Block(Vec<Stmt>, Range<usize>),
}

impl fmt::Display for Stmt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Stmt::VarDecl { bindings, init, .. } => {
                write!(f, "(var-decl ")?;
                for b in bindings {
                    write!(f, "{} ", b)?;
                }
                write!(f, "= {})", init)
            }
            Stmt::Assign { lvalue, value, .. } => {
                write!(f, "(assign {} {})", lvalue, value)
            }
            Stmt::Return { values, .. } => {
                if values.is_empty() {
                    write!(f, "(return)")
                } else {
                    write!(f, "(return")?;
                    for v in values {
                        write!(f, " {}", v)?;
                    }
                    write!(f, ")")
                }
            }
            Stmt::If { condition, then_branch, else_branch, .. } => {
                write!(f, "(if {} (then {})", condition, then_branch)?;
                if let Some(else_s) = else_branch {
                    write!(f, " (else {})", else_s)?;
                }
                write!(f, ")")
            }
            Stmt::For { var_name, var_type, start, end, step_lvalue, step_expr, body, .. } => {
                write!(f, "(for {} {} = {} , {} , {} {} in {})",
                    var_name, var_type, start, end, step_lvalue, step_expr, body)
            }
            Stmt::Expr(e, ..) => write!(f, "(expr-stmt {})", e),
            Stmt::Block(stmts, ..) => {
                write!(f, "(block")?;
                for s in stmts {
                    write!(f, "\n    {}", s)?;
                }
                write!(f, ")")
            }
        }
    }
}

// 忽略 span 比较
impl PartialEq for Stmt {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Stmt::VarDecl { bindings: b1, init: i1, .. },
             Stmt::VarDecl { bindings: b2, init: i2, .. }) => b1 == b2 && i1 == i2,
            (Stmt::Assign { lvalue: l1, value: v1, .. },
             Stmt::Assign { lvalue: l2, value: v2, .. }) => l1 == l2 && v1 == v2,
            (Stmt::Return { values: v1, .. },
             Stmt::Return { values: v2, .. }) => v1 == v2,
            (Stmt::If { condition: c1, then_branch: t1, else_branch: e1, .. },
             Stmt::If { condition: c2, then_branch: t2, else_branch: e2, .. }) => {
                c1 == c2 && t1 == t2 && e1 == e2
            }
            (Stmt::For { var_name: vn1, var_type: vt1, start: s1, end: e1,
                         step_lvalue: sl1, step_expr: se1, body: b1, .. },
             Stmt::For { var_name: vn2, var_type: vt2, start: s2, end: e2,
                         step_lvalue: sl2, step_expr: se2, body: b2, .. }) => {
                vn1 == vn2 && vt1 == vt2 && s1 == s2 && e1 == e2
                    && sl1 == sl2 && se1 == se2 && b1 == b2
            }
            (Stmt::Expr(e1, _), Stmt::Expr(e2, _)) => e1 == e2,
            (Stmt::Block(s1, _), Stmt::Block(s2, _)) => s1 == s2,
            _ => false,
        }
    }
}

// ── 顶层 ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<(String, Type)>,
}

impl fmt::Display for StructDef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(struct-def \"{}\"", self.name)?;
        for (name, ty) in &self.fields {
            write!(f, "\n    (field \"{}\" {})", name, ty)?;
        }
        write!(f, ")")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub return_type: ReturnType,
    pub body: Vec<Stmt>,
}

impl fmt::Display for FuncDef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(func-def \"{}\" [", self.name)?;
        if let Some((first, rest)) = self.params.split_first() {
            write!(f, "({} {})", first.0, first.1)?;
            for (name, ty) in rest {
                write!(f, " ({} {})", name, ty)?;
            }
        }
        write!(f, "] -> {}", self.return_type)?;
        write!(f, "\n    (block")?;
        for s in &self.body {
            write!(f, "\n        {}", s)?;
        }
        write!(f, "))")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopLevel {
    Struct(StructDef),
    Func(FuncDef),
}

impl fmt::Display for TopLevel {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TopLevel::Struct(s) => write!(f, "{}", s),
            TopLevel::Func(func) => write!(f, "{}", func),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub items: Vec<TopLevel>,
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(program")?;
        for item in &self.items {
            write!(f, "\n  {}", item)?;
        }
        write!(f, ")")
    }
}

// ── Span 辅助函数 ─────────────────────────────────────────────────────────

pub fn span_of_expr(expr: &Expr) -> Range<usize> {
    match expr {
        Expr::Binary { span, .. } => span.clone(),
        Expr::Unary { span, .. } => span.clone(),
        Expr::Call { span, .. } => span.clone(),
        Expr::Index { span, .. } => span.clone(),
        Expr::FieldAccess { span, .. } => span.clone(),
        Expr::StructLit { span, .. } => span.clone(),
        Expr::IntLit(_, span) => span.clone(),
        Expr::FloatLit(_, span) => span.clone(),
        Expr::StrLit(_, span) => span.clone(),
        Expr::BoolLit(_, span) => span.clone(),
        Expr::ArrayLit(_, span) => span.clone(),
        Expr::Ident(_, span) => span.clone(),
    }
}

pub fn span_of_stmt(stmt: &Stmt) -> Range<usize> {
    match stmt {
        Stmt::VarDecl { span, .. } => span.clone(),
        Stmt::Assign { span, .. } => span.clone(),
        Stmt::Return { span, .. } => span.clone(),
        Stmt::If { span, .. } => span.clone(),
        Stmt::For { span, .. } => span.clone(),
        Stmt::Expr(_, span) => span.clone(),
        Stmt::Block(_, span) => span.clone(),
    }
}

pub fn span_of_lvalue(lv: &LValue) -> Range<usize> {
    match lv {
        LValue::Ident(_, span) => span.clone(),
        LValue::Index { span, .. } => span.clone(),
        LValue::FieldAccess { span, .. } => span.clone(),
    }
}

// ── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    const S: Range<usize> = 0..1;

    // ── Type Display ───────────────────────────────────────────────────

    #[test]
    fn display_base_i32() {
        assert_eq!(format!("{}", Type::Base(BaseType::I32)), "(type \"i32\")");
    }

    #[test]
    fn display_base_void() {
        assert_eq!(format!("{}", Type::Base(BaseType::Void)), "(type \"void\")");
    }

    #[test]
    fn display_array() {
        assert_eq!(format!("{}", Type::Array(BaseType::I32)), "(type \"[i32]\")");
    }

    #[test]
    fn display_return_type_pair() {
        let rt = ReturnType::Pair(Type::Base(BaseType::I32), Type::Base(BaseType::Bool));
        assert_eq!(format!("{}", rt), "((type \"i32\") (type \"bool\"))");
    }

    // ── Expr Display ───────────────────────────────────────────────────

    #[test]
    fn display_int_lit() {
        assert_eq!(format!("{}", Expr::IntLit("42".into(), S)), "(int-lit 42)");
    }

    #[test]
    fn display_float_lit() {
        assert_eq!(format!("{}", Expr::FloatLit("3.14".into(), S)), "(float-lit 3.14)");
    }

    #[test]
    fn display_str_lit() {
        assert_eq!(format!("{}", Expr::StrLit("hello".into(), S)), "(str-lit \"hello\")");
    }

    #[test]
    fn display_bool_lit() {
        assert_eq!(format!("{}", Expr::BoolLit(true, S)), "(bool-lit true)");
        assert_eq!(format!("{}", Expr::BoolLit(false, S)), "(bool-lit false)");
    }

    #[test]
    fn display_ident() {
        assert_eq!(format!("{}", Expr::Ident("main".into(), S)), "main");
    }

    #[test]
    fn display_binary_expr() {
        let e = Expr::Binary {
            left: Box::new(Expr::Ident("a".into(), S)),
            op: BinOp::Add,
            right: Box::new(Expr::IntLit("1".into(), S)),
            span: S,
        };
        assert_eq!(format!("{}", e), "(+ a (int-lit 1))");
    }

    #[test]
    fn display_unary_expr() {
        let e = Expr::Unary { op: UnaryOp::Neg, expr: Box::new(Expr::Ident("x".into(), S)), span: S };
        assert_eq!(format!("{}", e), "(- x)");
    }

    #[test]
    fn display_call() {
        let e = Expr::Call {
            func: Box::new(Expr::Ident("f".into(), S)),
            args: vec![Expr::IntLit("1".into(), S), Expr::IntLit("2".into(), S)],
            span: S,
        };
        let s = format!("{}", e);
        assert!(s.contains("call"), "output: {}", s);
        assert!(s.contains("int-lit 1"), "output: {}", s);
    }

    #[test]
    fn display_call_no_args() {
        let e = Expr::Call { func: Box::new(Expr::Ident("f".into(), S)), args: vec![], span: S };
        assert_eq!(format!("{}", e), "(call f args=())");
    }

    #[test]
    fn display_index() {
        let e = Expr::Index {
            array: Box::new(Expr::Ident("arr".into(), S)),
            index: Box::new(Expr::IntLit("0".into(), S)),
            span: S,
        };
        assert_eq!(format!("{}", e), "(index arr (int-lit 0))");
    }

    #[test]
    fn display_field_access() {
        let e = Expr::FieldAccess {
            obj: Box::new(Expr::Ident("obj".into(), S)),
            field: "field".into(),
            span: S,
        };
        assert_eq!(format!("{}", e), "(. obj field)");
    }

    #[test]
    fn display_array_lit() {
        let e = Expr::ArrayLit(vec![Expr::IntLit("1".into(), S), Expr::IntLit("2".into(), S)], S);
        assert_eq!(format!("{}", e), "(array-lit (int-lit 1) (int-lit 2))");
    }

    #[test]
    fn display_array_lit_empty() {
        assert_eq!(format!("{}", Expr::ArrayLit(vec![], S)), "(array-lit)");
    }

    #[test]
    fn display_struct_lit() {
        let e = Expr::StructLit {
            name: "Point".into(),
            fields: vec![("x".into(), Expr::IntLit("1".into(), S))],
            span: S,
        };
        let s = format!("{}", e);
        assert!(s.contains("struct-lit \"Point\""), "output: {}", s);
        assert!(s.contains("(x (int-lit 1))"), "output: {}", s);
    }

    // ── Stmt Display ───────────────────────────────────────────────────

    #[test]
    fn display_return_void() {
        assert_eq!(format!("{}", Stmt::Return { values: vec![], span: S }), "(return)");
    }

    #[test]
    fn display_return_single() {
        let s = Stmt::Return { values: vec![Expr::IntLit("0".into(), S)], span: S };
        assert_eq!(format!("{}", s), "(return (int-lit 0))");
    }

    #[test]
    fn display_var_decl() {
        let s = Stmt::VarDecl {
            bindings: vec![VarBinding::Named { name: "x".into(), ty: Type::Base(BaseType::I32) }],
            init: Box::new(Expr::IntLit("42".into(), S)),
            span: S,
        };
        let out = format!("{}", s);
        assert!(out.contains("var-decl"), "output: {}", out);
        assert!(out.contains("x"), "output: {}", out);
    }

    #[test]
    fn display_assign() {
        let s = Stmt::Assign {
            lvalue: LValue::Ident("x".into(), S),
            value: Box::new(Expr::IntLit("1".into(), S)),
            span: S,
        };
        assert_eq!(format!("{}", s), "(assign x (int-lit 1))");
    }

    #[test]
    fn display_if_with_else() {
        let s = Stmt::If {
            condition: Box::new(Expr::BoolLit(true, S)),
            then_branch: Box::new(Stmt::Return { values: vec![], span: S }),
            else_branch: Some(Box::new(Stmt::Return { values: vec![Expr::IntLit("0".into(), S)], span: S })),
            span: S,
        };
        let out = format!("{}", s);
        assert!(out.contains("(if"), "output: {}", out);
        assert!(out.contains("(then"), "output: {}", out);
        assert!(out.contains("(else"), "output: {}", out);
    }

    #[test]
    fn display_block_empty() {
        assert_eq!(format!("{}", Stmt::Block(vec![], S)), "(block)");
    }

    // ── TopLevel Display ───────────────────────────────────────────────

    #[test]
    fn display_struct_def() {
        let s = StructDef { name: "Point".into(), fields: vec![] };
        assert_eq!(format!("{}", s), "(struct-def \"Point\")");
    }

    #[test]
    fn display_func_def_minimal() {
        let f = FuncDef {
            name: "main".into(),
            params: vec![],
            return_type: ReturnType::Single(Type::Base(BaseType::I32)),
            body: vec![],
        };
        let out = format!("{}", f);
        assert!(out.contains("func-def \"main\""), "output: {}", out);
        assert!(out.contains("(type \"i32\")"), "output: {}", out);
    }

    #[test]
    fn display_program_empty() {
        assert_eq!(format!("{}", Program { items: vec![] }), "(program)");
    }

    #[test]
    fn display_program_with_items() {
        let p = Program {
            items: vec![
                TopLevel::Struct(StructDef { name: "A".into(), fields: vec![] }),
            ],
        };
        let out = format!("{}", p);
        assert!(out.contains("(program"), "output: {}", out);
        assert!(out.contains("struct-def"), "output: {}", out);
    }

    // ── VarBinding Display ─────────────────────────────────────────────

    #[test]
    fn display_var_binding_named() {
        let v = VarBinding::Named { name: "x".into(), ty: Type::Base(BaseType::I32) };
        assert_eq!(format!("{}", v), "(x (type \"i32\"))");
    }

    #[test]
    fn display_var_binding_discard() {
        assert_eq!(format!("{}", VarBinding::Discard), "_");
    }

    // ── LValue Display ─────────────────────────────────────────────────

    #[test]
    fn display_lvalue_ident() {
        assert_eq!(format!("{}", LValue::Ident("x".into(), S)), "x");
    }

    #[test]
    fn display_lvalue_index() {
        let l = LValue::Index {
            array: Box::new(Expr::Ident("arr".into(), S)),
            index: Box::new(Expr::IntLit("0".into(), S)),
            span: S,
        };
        assert_eq!(format!("{}", l), "(lvalue-index arr (int-lit 0))");
    }

    #[test]
    fn display_lvalue_field() {
        let l = LValue::FieldAccess {
            obj: Box::new(Expr::Ident("obj".into(), S)),
            field: "f".into(),
            span: S,
        };
        assert_eq!(format!("{}", l), "(lvalue-field obj f)");
    }

    // ── BaseType Display ───────────────────────────────────────────────

    #[test]
    fn display_basetype_user() {
        assert_eq!(format!("{}", BaseType::UserDef("MyType".into())), "MyType");
    }
}
