// 语句代码生成 — 将 TypedStmt 转为 LLVM IR 指令

use super::context::CodeGenContext;
use super::expr::{codegen_expr, size_of_kang};
use crate::ast::{LValue};
use crate::error::CodeGenError;
use crate::semantic::{KangType, TypedExpr, TypedStmt, TypedStmtKind};
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

type Result<T> = std::result::Result<T, CodeGenError>;

/// 将 LLVM BuilderError 转为 CodeGenError 传播。
///
/// LLVM builder 错误（类型不匹配等）在执行正确类型检查的 Kang 程序中不应出现。
/// 语义检查在代码生成之前保证了类型正确性，因此此处的错误意味着编译器自身存在 bug。
/// 使用此辅助函数而非直接 unwrap，是为了将此类内部一致性错误优雅地报告给用户。
fn ok<T>(r: std::result::Result<T, inkwell::builder::BuilderError>) -> Result<T> {
    r.map_err(|e| CodeGenError { msg: format!("LLVM builder error: {}", e) })
}

/// 生成语句的 LLVM IR。根据语句类型分发到具体代码生成函数。
///
/// 函数返回值类型 func_return 传入 if/for 等复合语句，用于正确终结内部分支
/// （非 void 函数的所有路径都需要 return）。
pub fn codegen_stmt<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    stmt: &TypedStmt,
    func_return: &KangType,
) -> Result<()> {
    match &stmt.kind {
        TypedStmtKind::VarDecl { bindings, init } => codegen_var_decl(ctx, bindings, init),
        TypedStmtKind::Assign { lvalue, value } => codegen_assign(ctx, lvalue, value),
        TypedStmtKind::Return { values } => codegen_return(ctx, values, func_return),
        TypedStmtKind::If { condition, then_branch, else_branch } => {
            codegen_if(ctx, condition, then_branch, else_branch, func_return)
        }
        TypedStmtKind::For { var_name, var_type, start, end, step_lvalue, step_expr, body } => {
            codegen_for(ctx, var_name, var_type, start, end, step_lvalue, step_expr, body, func_return)
        }
        TypedStmtKind::Expr(e) => {
            codegen_expr(ctx, e)?;
            Ok(())
        }
        TypedStmtKind::Block(stmts) => codegen_block(ctx, stmts, func_return),
    }
}

// ── VarDecl ────────────────────────────────────────────────────────────────

fn codegen_var_decl<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    bindings: &[(String, KangType, bool)], // (name, type, is_discard)
    init: &TypedExpr,
) -> Result<()> {
    let init_val = codegen_expr(ctx, init)?;

    match &init.ty {
        KangType::Pair(_, _) => {
            // 二值解包
            let struct_val = init_val.into_struct_value();
            let v0 = ok(ctx.builder.build_extract_value(struct_val, 0, "unpack.0"))?;
            let v1 = ok(ctx.builder.build_extract_value(struct_val, 1, "unpack.1"))?;
            let values = vec![v0.into(), v1.into()];
            for (i, (name, ty, is_discard)) in bindings.iter().enumerate() {
                if !is_discard {
                    let val = values.get(i).copied().unwrap_or_else(|| ctx.default_value(ty));
                    let alloca = ok(ctx.builder.build_alloca(ctx.kang_type_to_basic(ty), &format!("var.{}", name)))?;
                    let _ = ctx.builder.build_store(alloca, val);
                    ctx.register_var(name, alloca, ty.clone());
                }
            }
        }
        _ => {
            // 单值: 可能单接收从二值返回（取第一值）
            for (name, ty, is_discard) in bindings {
                if !is_discard {
                    let alloca = ok(ctx.builder.build_alloca(ctx.kang_type_to_basic(&ty), &format!("var.{}", name)))?;
                    let _ = ctx.builder.build_store(alloca, init_val);
                    ctx.register_var(name, alloca, ty.clone());
                }
            }
        }
    }
    Ok(())
}

// ── Assign ────────────────────────────────────────────────────────────────

fn codegen_assign<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    lvalue: &LValue,
    value: &TypedExpr,
) -> Result<()> {
    let val = codegen_expr(ctx, value)?;
    let ptr = codegen_lvalue_ptr(ctx, lvalue)?;
    let _ = ctx.builder.build_store(ptr, val);
    Ok(())
}

/// 获取左值的内存指针，用于赋值写入。
///
/// Ident → 返回变量的 alloca 指针。
/// Index → 计算 arr[i] 的内存地址（跳过 4 字节长度头，按元素大小偏移）。
/// FieldAccess → 通过 struct GEP 获取字段指针。
fn codegen_lvalue_ptr<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    lvalue: &LValue,
) -> Result<inkwell::values::PointerValue<'ctx>> {
    match lvalue {
        LValue::Ident(name, ..) => {
            let (ptr, _) = ctx
                .lookup_var(name)
                .ok_or_else(|| CodeGenError { msg: format!("未定义变量: {}", name) })?;
            Ok(ptr)
        }
        LValue::Index { array, index, .. } => {
            // 数组索引: arr[i] → 计算偏移后的元素指针
            // array 是 raw Expr，在语义阶段已校验，此处取 ident + 类型
            let (arr_ptr, arr_ty) = resolve_array_ptr(ctx, array)?;
            let idx_val = codegen_expr_raw(ctx, index)?;
            let idx = idx_val.into_int_value();

            let elem_ty = match &arr_ty {
                KangType::Array(e) => &**e,
                _ => return Err(CodeGenError { msg: "只能对数组使用索引赋值".into() }),
            };
            let elem_size = size_of_kang(ctx, elem_ty);
            let elem_size_val = ctx.context.i32_type().const_int(elem_size as u64, true);
            let offset = ok(ctx.builder.build_int_mul(idx, elem_size_val, "offset"))?;

            let raw_int = ok(ctx.builder.build_ptr_to_int(arr_ptr, ctx.context.i64_type(), "ptr.int"))?;
            let base = ok(ctx.builder.build_int_add(raw_int, ctx.context.i64_type().const_int(4u64, false), "base"))?;
            let offset64 = ok(ctx.builder.build_int_z_extend(offset, ctx.context.i64_type(), "offset.64"))?;
            let elem_int = ok(ctx.builder.build_int_add(base, offset64, "elem.addr"))?;
            let elem_ptr = ok(ctx.builder.build_int_to_ptr(
                elem_int,
                ctx.context.ptr_type(AddressSpace::default()),
                "elem.ptr",
            ))?;
            let typed_ptr = ok(ctx.builder.build_pointer_cast(
                elem_ptr,
                ctx.context.ptr_type(AddressSpace::default()),
                "elem.typed",
            ))?;
            Ok(typed_ptr)
        }
        LValue::FieldAccess { obj, field, .. } => {
            // obj 是 raw Expr (通常为 Ident)，查找其 alloca + 类型
            let (struct_ptr, struct_ty) = resolve_struct_ptr(ctx, obj)?;
            let struct_name = match &struct_ty {
                KangType::Struct(name) => name.clone(),
                _ => return Err(CodeGenError { msg: "只能对结构体使用字段赋值".into() }),
            };
            let fields = ctx
                .struct_fields
                .get(&struct_name)
                .ok_or_else(|| CodeGenError { msg: format!("未定义结构体: {}", struct_name) })?;
            let field_idx = fields
                .iter()
                .position(|(n, _)| n == field)
                .ok_or_else(|| CodeGenError { msg: format!("字段不存在: {}", field) })?;

            let struct_type = ctx.lookup_struct_type(&struct_name)
                .ok_or_else(|| CodeGenError { msg: format!("结构体类型未注册: {}", struct_name) })?;
            let gep = ok(ctx.builder.build_struct_gep(
                struct_type,
                struct_ptr,
                field_idx as u32,
                "field.ptr",
            ))?;
            Ok(gep)
        }
    }
}

/// 从 raw AST Expr（当前仅支持 Ident）解析数组变量，返回堆数据指针和 Kang 类型。
///
/// 流程: 加载变量的 {ptr, len} 结构体 → 提取 ptr 字段（field 0）→ 返回堆数据的 i8*。
/// 在赋值 arr[i] = x 的左值指针解析中使用。
fn resolve_array_ptr<'ctx>(
    ctx: &CodeGenContext<'ctx>,
    array: &crate::ast::Expr,
) -> Result<(inkwell::values::PointerValue<'ctx>, KangType)> {
    match array {
        crate::ast::Expr::Ident(name, ..) => {
            let (alloca, ty) = ctx
                .lookup_var(name)
                .ok_or_else(|| CodeGenError { msg: format!("未定义变量: {}", name) })?;
            // 加载数组 struct {i8*, i32}, 提取 data 指针
            let arr_struct = ok(ctx.builder.build_load(ctx.kang_type_to_basic(&ty), alloca, "arr.load"))?;
            let data_ptr = ok(ctx.builder.build_extract_value(
                arr_struct.into_struct_value(),
                0,
                "arr.data",
            ))?;
            Ok((data_ptr.into_pointer_value(), ty))
        }
        _ => Err(CodeGenError { msg: "数组索引左值必须是变量".into() }),
    }
}

/// 从 raw AST Expr（当前仅支持 Ident）解析结构体变量，返回 alloca 指针和 Kang 类型。
///
/// 与 resolve_array_ptr 不同，此函数返回 alloca 指针而非数据指针，
/// 因为 struct GEP 操作直接作用于结构体变量的 alloca 地址。
fn resolve_struct_ptr<'ctx>(
    ctx: &CodeGenContext<'ctx>,
    obj: &crate::ast::Expr,
) -> Result<(inkwell::values::PointerValue<'ctx>, KangType)> {
    match obj {
        crate::ast::Expr::Ident(name, ..) => {
            let (ptr, ty) = ctx
                .lookup_var(name)
                .ok_or_else(|| CodeGenError { msg: format!("未定义变量: {}", name) })?;
            Ok((ptr, ty))
        }
        _ => Err(CodeGenError { msg: "字段访问左值必须是变量".into() }),
    }
}

/// 对 raw AST Expr 做最小编码生成，仅处理 lvalue 中出现的简单表达式。
///
/// 赋值语句的左值索引（如 arr[i] = x）中的 i 在语法分析后是 AST Expr，
/// 未被转换为 TypedExpr。此函数直接处理这些简单的原始表达式（变量引用、整数常量），
/// 避免对未类型化的 AST 节点调用完整的 codegen_expr。
fn codegen_expr_raw<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    expr: &crate::ast::Expr,
) -> Result<BasicValueEnum<'ctx>> {
    match expr {
        crate::ast::Expr::Ident(name, ..) => {
            let (alloca, ty) = ctx
                .lookup_var(name)
                .ok_or_else(|| CodeGenError { msg: format!("未定义变量: {}", name) })?;
            let val = ok(ctx.builder.build_load(ctx.kang_type_to_basic(&ty), alloca, "lval.load"))?;
            Ok(val)
        }
        crate::ast::Expr::IntLit(v, ..) => {
            let n: i64 = v.parse().unwrap_or(0);
            Ok(ctx.context.i32_type().const_int(n as u64, true).into())
        }
        _ => Err(CodeGenError { msg: "左值中不支持的表达式类型".into() }),
    }
}

// ── Return ────────────────────────────────────────────────────────────────

fn codegen_return<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    values: &[TypedExpr],
    func_return: &KangType,
) -> Result<()> {
    if values.is_empty() {
        let _ = ctx.builder.build_return(None);
        return Ok(());
    }

    match func_return {
        KangType::Pair(_, _) => {
            // 多值返回: 打包为 struct
            let v0 = codegen_expr(ctx, &values[0])?;
            let v1 = codegen_expr(ctx, &values[1])?;
            let pair_type = ctx.kang_type_to_basic(func_return).into_struct_type();
            let undef = pair_type.const_zero();
            let s1 = ok(ctx.builder.build_insert_value(undef, v0, 0, "pair.packed.0"))?.into_struct_value();
            let packed = ok(ctx.builder.build_insert_value(s1, v1, 1, "pair.packed.1"))?.into_struct_value();
            let _ = ctx.builder.build_return(Some(&packed));
        }
        _ => {
            let val = codegen_expr(ctx, &values[0])?;
            let _ = ctx.builder.build_return(Some(&val));
        }
    }
    Ok(())
}

// ── If/Else ───────────────────────────────────────────────────────────────

fn codegen_if<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    condition: &TypedExpr,
    then_branch: &TypedStmt,
    else_branch: &Option<Box<TypedStmt>>,
    func_return: &KangType,
) -> Result<()> {
    let cond = codegen_expr(ctx, condition)?;
    let cond_bool = ok(ctx.builder.build_int_compare(
        inkwell::IntPredicate::NE,
        cond.into_int_value(),
        ctx.context.bool_type().const_zero(),
        "if.cond",
    ))?;

    let current_fn = ctx.builder.get_insert_block().unwrap().get_parent().unwrap();

    let then_bb = ctx.context.append_basic_block(current_fn, "if.then");
    let else_bb = ctx.context.append_basic_block(current_fn, "if.else");
    let merge_bb = ctx.context.append_basic_block(current_fn, "if.merge");

    let _ = ctx.builder.build_conditional_branch(cond_bool, then_bb, else_bb);

    // then 分支
    ctx.builder.position_at_end(then_bb);
    ctx.push_scope();
    codegen_stmt(ctx, then_branch, func_return)?;
    ctx.pop_scope();
    if ctx.builder.get_insert_block().unwrap().get_terminator().is_none() {
        let _ = ctx.builder.build_unconditional_branch(merge_bb);
    }

    // else 分支
    ctx.builder.position_at_end(else_bb);
    if let Some(else_s) = else_branch {
        ctx.push_scope();
        codegen_stmt(ctx, else_s, func_return)?;
        ctx.pop_scope();
    }
    if ctx.builder.get_insert_block().unwrap().get_terminator().is_none() {
        let _ = ctx.builder.build_unconditional_branch(merge_bb);
    }

    ctx.builder.position_at_end(merge_bb);
    Ok(())
}

// ── For 循环 ─────────────────────────────────────────────────────────────

fn codegen_for<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    var_name: &str,
    var_type: &KangType,
    start: &TypedExpr,
    end: &TypedExpr,
    step_lvalue: &LValue,
    step_expr: &TypedExpr,
    body: &TypedStmt,
    func_return: &KangType,
) -> Result<()> {
    let start_val = codegen_expr(ctx, start)?;

    let current_fn = ctx.builder.get_insert_block().unwrap().get_parent().unwrap();

    // 初始化循环变量
    let alloca = ok(ctx.builder.build_alloca(ctx.kang_type_to_basic(var_type), var_name))?;
    let _ = ctx.builder.build_store(alloca, start_val);
    ctx.push_scope();
    ctx.register_var(var_name, alloca, var_type.clone());

    let cond_bb = ctx.context.append_basic_block(current_fn, "for.cond");
    let body_bb = ctx.context.append_basic_block(current_fn, "for.body");
    let end_bb = ctx.context.append_basic_block(current_fn, "for.end");

    // 跳转到条件检查
    let _ = ctx.builder.build_unconditional_branch(cond_bb);

    // 条件块: 每次迭代重新求值 end 表达式（可引用循环变量）
    ctx.builder.position_at_end(cond_bb);
    let cond_val = codegen_expr(ctx, end)?;
    let cond_bool = ok(ctx.builder.build_int_compare(
        inkwell::IntPredicate::NE,
        cond_val.into_int_value(),
        ctx.context.bool_type().const_zero(),
        "for.cond",
    ))?;
    let _ = ctx.builder.build_conditional_branch(cond_bool, body_bb, end_bb);

    // 循环体
    ctx.builder.position_at_end(body_bb);
    codegen_stmt(ctx, body, func_return)?;

    // 步进
    codegen_assign(ctx, step_lvalue, step_expr)?;

    if ctx.builder.get_insert_block().unwrap().get_terminator().is_none() {
        let _ = ctx.builder.build_unconditional_branch(cond_bb);
    }

    ctx.builder.position_at_end(end_bb);
    ctx.pop_scope();
    Ok(())
}

// ── Block ────────────────────────────────────────────────────────────────

fn codegen_block<'ctx>(
    ctx: &mut CodeGenContext<'ctx>,
    stmts: &[TypedStmt],
    func_return: &KangType,
) -> Result<()> {
    ctx.push_scope();
    for stmt in stmts {
        codegen_stmt(ctx, stmt, func_return)?;
    }
    ctx.pop_scope();
    Ok(())
}
