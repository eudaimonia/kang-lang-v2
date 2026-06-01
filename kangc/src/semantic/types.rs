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
    pub ty: KangType,    // 表达式求值后的类型（TypedStmt 中的表达式也携带类型）
}

/// 类型标注后的表达式种类（对应 ast::Expr，但去除了 ReturnType 等非表达式构造）
#[derive(Debug, Clone)]
pub enum TypedExprKind {
    /// i32 字面量，String 保留原始文本（如 "42"），codegen 解析为 LLVM 常量
    IntLit(String),
    /// f64 字面量，String 保留原始文本
    FloatLit(String),
    /// 字符串字面量，已处理转义（如 "\n" → 实际换行符）
    StrLit(String),
    /// 布尔字面量
    BoolLit(bool),
    /// 标识符引用，指向作用域中已声明的变量或函数
    Ident(String),
    /// 二元运算（算数/比较/逻辑），左右操作数类型已通过 T5-T8 检查
    Binary {
        left: Box<TypedExpr>,
        op: ast::BinOp,
        right: Box<TypedExpr>,
    },
    /// 一元运算（取负 Neg / 逻辑非 Not）
    Unary {
        op: ast::UnaryOp,
        expr: Box<TypedExpr>,
    },
    /// 函数调用，func_name 可能是用户 def 或内置函数
    Call {
        func_name: String,
        args: Vec<TypedExpr>,
    },
    /// 数组索引 a[i]，返回元素类型（T9: a 必须是数组，i 必须是 i32）
    Index {
        array: Box<TypedExpr>,
        index: Box<TypedExpr>,
    },
    /// 结构体字段访问 obj.field（ST6: obj 必须是结构体，field 必须存在）
    FieldAccess {
        obj: Box<TypedExpr>,
        field: String,
    },
    /// 数组字面量 [e1, e2, ...]，所有元素类型一致（A1/A2）
    ArrayLit(Vec<TypedExpr>),
    /// 结构体字面量 StructType { field1: val1, ... }
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

/// 类型标注后的语句种类（对应 ast::Stmt）
#[derive(Debug, Clone)]
pub enum TypedStmtKind {
    /// var 声明，bindings 为 (name, type, is_discard) 列表
    /// is_discard 为 true 时用 _ 忽略绑定值
    VarDecl {
        bindings: Vec<(String, KangType, bool)>,
        init: Box<TypedExpr>,
    },
    /// 赋值语句，lvalue 保持 AST 形式（语义检查验证过可赋值性）
    Assign {
        lvalue: ast::LValue,
        value: Box<TypedExpr>,
    },
    /// return 语句，values 类型已与函数返回类型匹配（F2/F3）
    Return {
        values: Vec<TypedExpr>,
    },
    /// if-then-else，condition 必须是 bool（T3）
    If {
        condition: Box<TypedExpr>,
        then_branch: Box<TypedStmt>,
        else_branch: Option<Box<TypedStmt>>,
    },
    /// for var V:T = start, end, step_expr in body（T4: start/end 类型匹配）
    For {
        var_name: String,
        var_type: KangType,
        start: Box<TypedExpr>,
        end: Box<TypedExpr>,
        step_lvalue: ast::LValue,
        step_expr: Box<TypedExpr>,
        body: Box<TypedStmt>,
    },
    /// 表达式语句（丢弃求值结果）
    Expr(Box<TypedExpr>),
    /// 语句块，包含多条顺序执行的语句
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

/// 类型标注后的顶层声明（结构体原样保留，函数变为 TypedFuncDef）
#[derive(Debug, Clone)]
pub enum TypedTopLevel {
    /// 结构体定义，原样保留（结构体类型在符号表中已有记录）
    Struct(ast::StructDef),
    /// 函数定义，参数和返回值已完成类型解析
    Func(TypedFuncDef),
}

/// 类型标注后的函数定义
#[derive(Debug, Clone)]
pub struct TypedFuncDef {
    pub name: String,
    /// 参数列表 (name, type)
    pub params: Vec<(String, KangType)>,
    /// 返回类型（Void / Single / Pair）
    pub return_type: KangType,
    /// 函数体语句，已完成类型检查
    pub body: Vec<TypedStmt>,
}

/// 类型标注后的完整程序，作为 codegen 的唯一输入
/// check() 的产出，codegen::codegen() 的直接输入
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
