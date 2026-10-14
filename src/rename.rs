//! A walk over every identifier in a program's AST, used by passes that
//! rename names consistently: the obfuscator (meaningless names) and the
//! semantic pass (case folding, since Pascal identifiers are
//! case-insensitive).

use crate::ast::*;

/// Maps an identifier to its replacement. Called once for every
/// occurrence, declarations before uses, in source order.
pub(crate) trait Rename {
    fn rename(&self, name: &str) -> String;
}

pub(crate) fn rewrite_program<R: Rename>(program: &mut Program, n: &R) {
    program.name = n.rename(&program.name);
    for c in &mut program.consts {
        c.name = n.rename(&c.name);
        if let Some(ty) = &mut c.ty {
            rewrite_type(ty, n);
        }
        rewrite_expr(&mut c.value, n);
    }
    for td in &mut program.type_decls {
        td.name = n.rename(&td.name);
        rewrite_type(&mut td.ty, n);
    }
    for v in &mut program.vars {
        for name in &mut v.names {
            *name = n.rename(name);
        }
        rewrite_type(&mut v.ty, n);
    }
    for p in &mut program.procedures {
        rewrite_proc(p, n);
    }
    rewrite_block(&mut program.body, n);
}

fn rewrite_proc<R: Rename>(p: &mut ProcDecl, n: &R) {
    p.name = n.rename(&p.name);
    for group in &mut p.params {
        for name in &mut group.names {
            *name = n.rename(name);
        }
        rewrite_type(&mut group.ty, n);
    }
    if let Some(rt) = &mut p.return_type {
        rewrite_type(rt, n);
    }
    for v in &mut p.vars {
        for name in &mut v.names {
            *name = n.rename(name);
        }
        rewrite_type(&mut v.ty, n);
    }
    for np in &mut p.nested_procs {
        rewrite_proc(np, n);
    }
    rewrite_block(&mut p.body, n);
}

fn rewrite_type<R: Rename>(ty: &mut PascalType, n: &R) {
    match ty {
        PascalType::Enum { name, values } => {
            *name = n.rename(name);
            for v in values {
                *v = n.rename(v);
            }
        }
        PascalType::Record { fields, variant } => {
            for (fname, fty) in fields {
                *fname = n.rename(fname);
                rewrite_type(fty, n);
            }
            if let Some(var) = variant {
                var.tag_name = n.rename(&var.tag_name);
                rewrite_type(&mut var.tag_type, n);
                for (_vals, vfields) in &mut var.variants {
                    for (fname, fty) in vfields {
                        *fname = n.rename(fname);
                        rewrite_type(fty, n);
                    }
                }
            }
        }
        PascalType::ArrayExpr { lo, hi, elem } => {
            rewrite_const_expr(lo, n);
            rewrite_const_expr(hi, n);
            rewrite_type(elem, n);
        }
        PascalType::SubrangeExpr { lo, hi } => {
            rewrite_const_expr(lo, n);
            rewrite_const_expr(hi, n);
        }
        PascalType::Array { elem, .. }
        | PascalType::Pointer(elem)
        | PascalType::Set { elem }
        | PascalType::File { elem } => rewrite_type(elem, n),
        PascalType::ConformantArray {
            lo_name,
            hi_name,
            elem,
        } => {
            *lo_name = n.rename(lo_name);
            *hi_name = n.rename(hi_name);
            rewrite_type(elem, n);
        }
        PascalType::Proc {
            params,
            return_type,
        } => {
            for p in params {
                rewrite_type(p, n);
            }
            if let Some(rt) = return_type {
                rewrite_type(rt, n);
            }
        }
        PascalType::Named(name) => *name = n.rename(name),
        PascalType::Integer
        | PascalType::SizedInt { .. }
        | PascalType::Real
        | PascalType::String
        | PascalType::Boolean
        | PascalType::Char
        | PascalType::Subrange { .. } => {}
    }
}

/// Rename the constants and enum values a type bound refers to.
fn rewrite_const_expr<R: Rename>(e: &mut ConstExpr, n: &R) {
    match e {
        ConstExpr::Name(name) => *name = n.rename(name),
        ConstExpr::Neg(inner) | ConstExpr::Call(_, inner) => rewrite_const_expr(inner, n),
        ConstExpr::Binary(_, l, r) => {
            rewrite_const_expr(l, n);
            rewrite_const_expr(r, n);
        }
        ConstExpr::Int(_) | ConstExpr::Char(_) => {}
    }
}

fn rewrite_block<R: Rename>(block: &mut Block, n: &R) {
    for s in &mut block.statements {
        rewrite_stmt(s, n);
    }
}

fn rewrite_stmts<R: Rename>(stmts: &mut [Statement], n: &R) {
    for s in stmts {
        rewrite_stmt(s, n);
    }
}

fn rewrite_stmt<R: Rename>(s: &mut Statement, n: &R) {
    match s {
        Statement::Assignment { target, expr, .. } => {
            *target = n.rename(target);
            rewrite_expr(expr, n);
        }
        Statement::DerefAssignment { target, expr, .. } => {
            *target = n.rename(target);
            rewrite_expr(expr, n);
        }
        Statement::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            rewrite_expr(condition, n);
            rewrite_block(then_branch, n);
            if let Some(eb) = else_branch {
                rewrite_block(eb, n);
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            rewrite_expr(condition, n);
            rewrite_block(body, n);
        }
        Statement::For {
            var,
            from,
            to,
            body,
            ..
        } => {
            *var = n.rename(var);
            rewrite_expr(from, n);
            rewrite_expr(to, n);
            rewrite_block(body, n);
        }
        Statement::RepeatUntil {
            body, condition, ..
        } => {
            rewrite_stmts(body, n);
            rewrite_expr(condition, n);
        }
        Statement::WriteLn { args, .. } | Statement::Write { args, .. } => {
            for a in args {
                rewrite_expr(&mut a.expr, n);
                if let Some(w) = &mut a.width {
                    rewrite_expr(w, n);
                }
                if let Some(p) = &mut a.precision {
                    rewrite_expr(p, n);
                }
            }
        }
        Statement::ReadLn { targets, .. } => {
            for t in targets {
                *t = n.rename(t);
            }
        }
        Statement::Block(b) => rewrite_block(b, n),
        Statement::New { target, .. } | Statement::Dispose { target, .. } => {
            *target = n.rename(target);
        }
        Statement::IndexAssignment {
            target,
            index,
            expr,
            ..
        } => {
            *target = n.rename(target);
            rewrite_expr(index, n);
            rewrite_expr(expr, n);
        }
        Statement::MultiIndexAssignment {
            target,
            indices,
            expr,
            ..
        } => {
            *target = n.rename(target);
            for i in indices {
                rewrite_expr(i, n);
            }
            rewrite_expr(expr, n);
        }
        Statement::FieldAssignment {
            target,
            field,
            expr,
            ..
        } => {
            *target = n.rename(target);
            *field = n.rename(field);
            rewrite_expr(expr, n);
        }
        Statement::ProcCall { name, args, .. } => {
            *name = n.rename(name);
            for a in args {
                rewrite_expr(a, n);
            }
        }
        Statement::Case {
            expr,
            branches,
            else_branch,
            ..
        } => {
            rewrite_expr(expr, n);
            for b in branches {
                for v in &mut b.values {
                    match v {
                        CaseValue::Single(e) => rewrite_expr(e, n),
                        CaseValue::Range(a, c) => {
                            rewrite_expr(a, n);
                            rewrite_expr(c, n);
                        }
                    }
                }
                rewrite_stmts(&mut b.body, n);
            }
            if let Some(eb) = else_branch {
                rewrite_stmts(eb, n);
            }
        }
        Statement::With {
            record_var, body, ..
        } => {
            *record_var = n.rename(record_var);
            rewrite_block(body, n);
        }
        Statement::ChainedAssignment {
            target,
            chain,
            expr,
            ..
        } => {
            *target = n.rename(target);
            for step in chain {
                match step {
                    LValueAccess::Field(f) => *f = n.rename(f),
                    LValueAccess::Index(e) => rewrite_expr(e, n),
                    LValueAccess::Deref => {}
                }
            }
            rewrite_expr(expr, n);
        }
        // No identifiers to rewrite.
        Statement::Exit {
            value: Some(value), ..
        } => rewrite_expr(value, n),
        Statement::Exit { value: None, .. }
        | Statement::Break { .. }
        | Statement::Continue { .. }
        | Statement::Goto { .. }
        | Statement::Label { .. } => {}
    }
}

fn rewrite_expr<R: Rename>(e: &mut Expr, n: &R) {
    match e {
        Expr::Var(name, _) => *name = n.rename(name),
        Expr::BinOp { left, right, .. } => {
            rewrite_expr(left, n);
            rewrite_expr(right, n);
        }
        Expr::UnaryOp { operand, .. } => rewrite_expr(operand, n),
        Expr::Deref(inner, _) => rewrite_expr(inner, n),
        Expr::Call { name, args, .. } => {
            *name = n.rename(name);
            for a in args {
                rewrite_expr(a, n);
            }
        }
        Expr::Index { array, index, .. } => {
            rewrite_expr(array, n);
            rewrite_expr(index, n);
        }
        Expr::FieldAccess { record, field, .. } => {
            rewrite_expr(record, n);
            *field = n.rename(field);
        }
        Expr::SetConstructor { elements, .. } => {
            for el in elements {
                match el {
                    SetElement::Single(e) => rewrite_expr(e, n),
                    SetElement::Range(a, b) => {
                        rewrite_expr(a, n);
                        rewrite_expr(b, n);
                    }
                }
            }
        }
        Expr::IntLit(..)
        | Expr::RealLit(..)
        | Expr::CharLit(..)
        | Expr::StrLit(..)
        | Expr::BoolLit(..)
        | Expr::Nil(_) => {}
    }
}
