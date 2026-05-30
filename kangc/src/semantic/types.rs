// 语义类型系统 — KangType 定义 + TypedAST 节点
// checker 产出 TypedProgram，codegen 以此为唯一输入

use crate::ast;
use std::fmt;

// ── KangType ──────────────────────────────────────────────────────────────────

/// 语义层的类型表示，将 AST 中的类型名解析为具体类别
#[derive(Debug, Clone, PartialEq)]
pub enum KangType {
    I32,
    F64,
    Str,
    Bool,
    Void,
    Array(Box<KangType>),
    Struct(String),
    Pair(Box<KangType>, Box<KangType>),
}

impl KangType {
    /// 将 AST 类型转换为语义类型
    pub fn from_ast_type(ty: &ast::Type) -> Self {
        match ty {
            ast::Type::Base(bt) => Self::from_ast_base_type(bt),
            ast::Type::Array(bt) => KangType::Array(Box::new(Self::from_ast_base_type(bt))),
        }
    }

    pub fn from_ast_base_type(bt: &ast::BaseType) -> Self {
        match bt {
            ast::BaseType::I32 => KangType::I32,
            ast::BaseType::F64 => KangType::F64,
            ast::BaseType::Str => KangType::Str,
            ast::BaseType::Bool => KangType::Bool,
            ast::BaseType::Void => KangType::Void,
            ast::BaseType::UserDef(name) => KangType::Struct(name.clone()),
        }
    }

    /// 将 AST 返回类型转换为语义类型
    pub fn from_ast_return_type(rt: &ast::ReturnType) -> Self {
        match rt {
            ast::ReturnType::Single(ty) => Self::from_ast_type(ty),
            ast::ReturnType::Pair(t1, t2) => {
                KangType::Pair(Box::new(Self::from_ast_type(t1)), Box::new(Self::from_ast_type(t2)))
            }
        }
    }

    pub fn is_void(&self) -> bool {
        matches!(self, KangType::Void)
    }
}

impl fmt::Display for KangType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            KangType::I32 => write!(f, ":i32"),
            KangType::F64 => write!(f, ":f64"),
            KangType::Str => write!(f, ":str"),
            KangType::Bool => write!(f, ":bool"),
            KangType::Void => write!(f, ":void"),
            KangType::Array(elem) => write!(f, ":[{}]", elem),
            KangType::Struct(name) => write!(f, ":{}", name),
            KangType::Pair(t1, t2) => write!(f, "({} {})", t1, t2),
        }
    }
}

// ── TypedExpr ─────────────────────────────────────────────────────────────────

/// 带类型标注的表达式，每个节点携带解析后的 KangType
#[derive(Debug, Clone)]
pub struct TypedExpr {
    pub kind: TypedExprKind,
    pub ty: KangType,
}

#[derive(Debug, Clone)]
pub enum TypedExprKind {
    IntLit(String),
    FloatLit(String),
    StrLit(String),
    BoolLit(bool),
    Ident(String),
    Binary {
        left: Box<TypedExpr>,
        op: ast::BinOp,
        right: Box<TypedExpr>,
    },
    Unary {
        op: ast::UnaryOp,
        expr: Box<TypedExpr>,
    },
    Call {
        func_name: String,
        args: Vec<TypedExpr>,
    },
    Index {
        array: Box<TypedExpr>,
        index: Box<TypedExpr>,
    },
    FieldAccess {
        obj: Box<TypedExpr>,
        field: String,
    },
    ArrayLit(Vec<TypedExpr>),
    StructLit {
        name: String,
        fields: Vec<(String, TypedExpr)>,
    },
}

impl fmt::Display for TypedExpr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.kind {
            TypedExprKind::IntLit(v) => write!(f, "(int-lit {} {})", v, self.ty),
            TypedExprKind::FloatLit(v) => write!(f, "(float-lit {} {})", v, self.ty),
            TypedExprKind::StrLit(v) => write!(f, "(str-lit {:?} {})", v, self.ty),
            TypedExprKind::BoolLit(v) => write!(f, "(bool-lit {} {})", v, self.ty),
            TypedExprKind::Ident(name) => write!(f, "{} {}", name, self.ty),
            TypedExprKind::Binary { left, op, right } => {
                write!(f, "({} {} {} {})", op, left, right, self.ty)
            }
            TypedExprKind::Unary { op, expr } => {
                write!(f, "({} {} {})", op, expr, self.ty)
            }
            TypedExprKind::Call { func_name, args } => {
                write!(f, "(call {} args=(", func_name)?;
                if let Some((last, rest)) = args.split_last() {
                    for a in rest {
                        write!(f, "{} ", a)?;
                    }
                    write!(f, "{}", last)?;
                }
                write!(f, ") {})", self.ty)
            }
            TypedExprKind::Index { array, index } => {
                write!(f, "(index {} {} {})", array, index, self.ty)
            }
            TypedExprKind::FieldAccess { obj, field } => {
                write!(f, "(. {} {} {})", obj, field, self.ty)
            }
            TypedExprKind::ArrayLit(elems) => {
                write!(f, "(array-lit")?;
                for e in elems {
                    write!(f, " {}", e)?;
                }
                write!(f, " {})", self.ty)
            }
            TypedExprKind::StructLit { name, fields } => {
                write!(f, "(struct-lit \"{}\"", name)?;
                for (field_name, val) in fields {
                    write!(f, " ({} {})", field_name, val)?;
                }
                write!(f, " {})", self.ty)
            }
        }
    }
}

// ── TypedStmt ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TypedStmt {
    pub kind: TypedStmtKind,
}

#[derive(Debug, Clone)]
pub enum TypedStmtKind {
    VarDecl {
        bindings: Vec<(String, KangType, bool)>, // (name, type, is_discard)
        init: Box<TypedExpr>,
    },
    Assign {
        lvalue: ast::LValue,
        value: Box<TypedExpr>,
    },
    Return {
        values: Vec<TypedExpr>,
    },
    If {
        condition: Box<TypedExpr>,
        then_branch: Box<TypedStmt>,
        else_branch: Option<Box<TypedStmt>>,
    },
    For {
        var_name: String,
        var_type: KangType,
        start: Box<TypedExpr>,
        end: Box<TypedExpr>,
        step_lvalue: ast::LValue,
        step_expr: Box<TypedExpr>,
        body: Box<TypedStmt>,
    },
    Expr(Box<TypedExpr>),
    Block(Vec<TypedStmt>),
}

impl fmt::Display for TypedStmt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.kind {
            TypedStmtKind::VarDecl { bindings, init } => {
                write!(f, "(var-decl (")?;
                for (i, (name, ty, is_discard)) in bindings.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    if *is_discard {
                        write!(f, "_")?;
                    } else {
                        write!(f, "{} {}", name, ty)?;
                    }
                }
                write!(f, ") = {})", init)
            }
            TypedStmtKind::Assign { lvalue, value } => {
                write!(f, "(assign {} {})", lvalue, value)
            }
            TypedStmtKind::Return { values } => {
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
            TypedStmtKind::If { condition, then_branch, else_branch } => {
                write!(f, "(if {} (then {})", condition, then_branch)?;
                if let Some(else_s) = else_branch {
                    write!(f, " (else {})", else_s)?;
                }
                write!(f, ")")
            }
            TypedStmtKind::For { var_name, var_type, start, end, step_lvalue, step_expr, body } => {
                write!(f, "(for {} {} = {} , {} , {} {} in {})",
                    var_name, var_type, start, end, step_lvalue, step_expr, body)
            }
            TypedStmtKind::Expr(e) => write!(f, "(expr-stmt {})", e),
            TypedStmtKind::Block(stmts) => {
                write!(f, "(block")?;
                for s in stmts {
                    write!(f, "\n    {}", s)?;
                }
                write!(f, ")")
            }
        }
    }
}

// ── TypedTopLevel / TypedProgram ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TypedTopLevel {
    Struct(ast::StructDef),
    Func(TypedFuncDef),
}

#[derive(Debug, Clone)]
pub struct TypedFuncDef {
    pub name: String,
    pub params: Vec<(String, KangType)>,
    pub return_type: KangType,
    pub body: Vec<TypedStmt>,
}

#[derive(Debug, Clone)]
pub struct TypedProgram {
    pub items: Vec<TypedTopLevel>,
}

impl fmt::Display for TypedProgram {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(program")?;
        for item in &self.items {
            match item {
                TypedTopLevel::Struct(s) => write!(f, "\n  {}", s)?,
                TypedTopLevel::Func(func) => {
                    write!(f, "\n  (func-def \"{}\" [", func.name)?;
                    if let Some((first, rest)) = func.params.split_first() {
                        write!(f, "({} {})", first.0, first.1)?;
                        for (name, ty) in rest {
                            write!(f, " ({} {})", name, ty)?;
                        }
                    }
                    write!(f, "] -> {}", func.return_type)?;
                    if let Some(first) = func.body.first() {
                        write!(f, "\n    {}", first)?;
                    }
                    write!(f, ")")?;
                }
            }
        }
        write!(f, ")")
    }
}
