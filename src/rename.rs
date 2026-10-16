//! A walk over every identifier in a program's AST, used by passes that
//! rename names consistently: the obfuscator (meaningless names) and the
//! semantic pass (case folding, since Pascal identifiers are
//! case-insensitive).

use crate::ast::*;

/// Maps an identifier to its replacement. Called once for every
/// occurrence, declarations before uses, in source order.
pub(crate) trait Rename {
    fn rename(&self, name: &str) -> String;

    /// Maps the name of a field or method, which lives in its type rather
    /// than in a scope. The same as [`Rename::rename`] unless overridden.
    fn rename_member(&self, name: &str) -> String {
        self.rename(name)
    }

    /// Called when the walk enters a routine, after its own name is
    /// renamed and before its parameters and declarations are.
    fn enter_routine(&self, _proc: &ProcDecl) {}

    /// Called when the walk leaves the routine last entered.
    fn leave_routine(&self) {}

    /// Called on every expression before its parts are walked, so a pass
    /// can rewrite the node itself.
    fn rewrite_expr_node(&self, _expr: &mut Expr) {}

    /// Called on every statement before its parts are walked.
    fn rewrite_stmt_node(&self, _stmt: &mut Statement) {}
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
        for p in &mut td.type_params {
            *p = n.rename(p);
        }
        rewrite_type(&mut td.ty, n);
    }
    for v in &mut program.vars {
        for name in &mut v.names {
            *name = n.rename(name);
        }
        if let Some(target) = &mut v.absolute {
            *target = n.rename(target);
        }
        rewrite_type(&mut v.ty, n);
    }
    for p in &mut program.procedures {
        rewrite_proc(p, n);
    }
    rewrite_block(&mut program.body, n);
}

pub(crate) fn rewrite_unit<R: Rename>(unit: &mut Unit, n: &R) {
    for c in unit
        .interface_consts
        .iter_mut()
        .chain(&mut unit.impl_consts)
    {
        c.name = n.rename(&c.name);
        if let Some(ty) = &mut c.ty {
            rewrite_type(ty, n);
        }
        rewrite_expr(&mut c.value, n);
    }
    for td in unit.interface_types.iter_mut().chain(&mut unit.impl_types) {
        td.name = n.rename(&td.name);
        for p in &mut td.type_params {
            *p = n.rename(p);
        }
        rewrite_type(&mut td.ty, n);
    }
    for v in unit.interface_vars.iter_mut().chain(&mut unit.impl_vars) {
        for name in &mut v.names {
            *name = n.rename(name);
        }
        if let Some(target) = &mut v.absolute {
            *target = n.rename(target);
        }
        rewrite_type(&mut v.ty, n);
    }
    for p in unit
        .interface_proc_headers
        .iter_mut()
        .chain(&mut unit.procedures)
    {
        rewrite_proc(p, n);
    }
    if let Some(init) = &mut unit.init {
        rewrite_block(init, n);
    }
}

pub(crate) fn rewrite_proc<R: Rename>(p: &mut ProcDecl, n: &R) {
    if let Some(owner) = &mut p.owner {
        p.name = n.rename_member(&p.name);
        *owner = n.rename(owner);
    } else {
        p.name = n.rename(&p.name);
    }
    n.enter_routine(p);
    for c in &mut p.consts {
        c.name = n.rename(&c.name);
        if let Some(ty) = &mut c.ty {
            rewrite_type(ty, n);
        }
        rewrite_expr(&mut c.value, n);
    }
    for td in &mut p.type_decls {
        td.name = n.rename(&td.name);
        for p in &mut td.type_params {
            *p = n.rename(p);
        }
        rewrite_type(&mut td.ty, n);
    }
    for group in &mut p.params {
        for name in &mut group.names {
            *name = n.rename(name);
        }
        rewrite_type(&mut group.ty, n);
        if let Some(d) = &mut group.default {
            rewrite_expr(&mut d.0, n);
        }
    }
    if let Some(rt) = &mut p.return_type {
        rewrite_type(rt, n);
    }
    for v in &mut p.vars {
        for name in &mut v.names {
            *name = n.rename(name);
        }
        if let Some(target) = &mut v.absolute {
            *target = n.rename(target);
        }
        rewrite_type(&mut v.ty, n);
    }
    for np in &mut p.nested_procs {
        rewrite_proc(np, n);
    }
    rewrite_block(&mut p.body, n);
    n.leave_routine();
}

pub(crate) fn rewrite_type<R: Rename>(ty: &mut PascalType, n: &R) {
    match ty {
        PascalType::Object(obj) => {
            if let Some(parent) = &mut obj.parent {
                *parent = n.rename(parent);
            }
            for (fname, fty) in &mut obj.fields {
                *fname = n.rename_member(fname);
                rewrite_type(fty, n);
            }
            for m in &mut obj.methods {
                m.name = n.rename_member(&m.name);
                for group in &mut m.params {
                    for name in &mut group.names {
                        *name = n.rename(name);
                    }
                    rewrite_type(&mut group.ty, n);
                    if let Some(d) = &mut group.default {
                        rewrite_expr(&mut d.0, n);
                    }
                }
                if let Some(rt) = &mut m.return_type {
                    rewrite_type(rt, n);
                }
            }
            for a in &mut obj.parent_args {
                rewrite_type(a, n);
            }
            for (vname, vty) in &mut obj.class_vars {
                *vname = n.rename_member(vname);
                rewrite_type(vty, n);
            }
            for p in &mut obj.properties {
                p.name = n.rename_member(&p.name);
                for group in &mut p.params {
                    for name in &mut group.names {
                        *name = n.rename(name);
                    }
                    rewrite_type(&mut group.ty, n);
                    if let Some(d) = &mut group.default {
                        rewrite_expr(&mut d.0, n);
                    }
                }
                rewrite_type(&mut p.ty, n);
                for accessor in [&mut p.read, &mut p.write].into_iter().flatten() {
                    *accessor = accessor
                        .split('.')
                        .map(|part| n.rename_member(part))
                        .collect::<Vec<_>>()
                        .join(".");
                }
            }
        }
        PascalType::ClassOf(name) => *name = n.rename(name),
        PascalType::Specialize { name, args } => {
            *name = n.rename(name);
            for a in args {
                rewrite_type(a, n);
            }
        }
        PascalType::VarParam(inner)
        | PascalType::DynArray(inner)
        | PascalType::OpenArray(inner) => rewrite_type(inner, n),
        PascalType::Enum { name, values } => {
            *name = n.rename(name);
            for v in values {
                *v = n.rename(v);
            }
        }
        PascalType::Record {
            fields, variant, ..
        } => {
            for (fname, fty) in fields {
                *fname = n.rename_member(fname);
                rewrite_type(fty, n);
            }
            if let Some(var) = variant {
                var.tag_name = n.rename_member(&var.tag_name);
                rewrite_type(&mut var.tag_type, n);
                for (labels, vfields) in &mut var.variants {
                    for label in labels.iter_mut() {
                        rewrite_const_expr(label, n);
                    }
                    for (fname, fty) in vfields {
                        *fname = n.rename_member(fname);
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
            ..
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
        | PascalType::Untyped
        | PascalType::ShortString { .. }
        | PascalType::SizedInt { .. }
        | PascalType::Real
        | PascalType::String
        | PascalType::Boolean
        | PascalType::Char
        | PascalType::WideChar
        | PascalType::UnicodeString
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

pub(crate) fn rewrite_block<R: Rename>(block: &mut Block, n: &R) {
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
    n.rewrite_stmt_node(s);
    match s {
        Statement::Try { body, handler, .. } => {
            rewrite_block(body, n);
            match handler {
                TryHandler::Except {
                    handlers,
                    otherwise,
                } => {
                    for h in handlers {
                        if let Some(var) = &mut h.var {
                            *var = n.rename(var);
                        }
                        h.class = n.rename(&h.class);
                        rewrite_block(&mut h.body, n);
                    }
                    if let Some(b) = otherwise {
                        rewrite_block(b, n);
                    }
                }
                TryHandler::Finally(b) => rewrite_block(b, n),
            }
        }
        Statement::Raise { exception, .. } => {
            if let Some(e) = exception {
                rewrite_expr(e, n);
            }
        }
        Statement::IndirectCall { call, .. } => rewrite_expr(call, n),
        Statement::Str { value, target, .. } => {
            rewrite_expr(&mut value.expr, n);
            if let Some(w) = &mut value.width {
                rewrite_expr(w, n);
            }
            if let Some(p) = &mut value.precision {
                rewrite_expr(p, n);
            }
            rewrite_expr(target, n);
        }
        Statement::MethodCall {
            object,
            method,
            args,
            ..
        } => {
            rewrite_expr(object, n);
            *method = n.rename_member(method);
            for a in args {
                rewrite_expr(a, n);
            }
        }
        Statement::Inherited { method, args, .. } => {
            *method = n.rename_member(method);
            for a in args {
                rewrite_expr(a, n);
            }
        }
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
        Statement::ForIn {
            var,
            collection,
            body,
            ..
        } => {
            *var = n.rename(var);
            rewrite_expr(collection, n);
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
        Statement::New {
            target,
            constructor: method,
            ..
        }
        | Statement::Dispose {
            target,
            destructor: method,
            ..
        } => {
            rewrite_expr(target, n);
            if let Some((name, args)) = method {
                *name = n.rename_member(name);
                for a in args {
                    rewrite_expr(a, n);
                }
            }
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
            *field = n.rename_member(field);
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
        Statement::With { target, body, .. } => {
            rewrite_expr(target, n);
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
                    LValueAccess::Field(f) => *f = n.rename_member(f),
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
        Statement::PlaceAssignment { target, expr, .. } => {
            rewrite_expr(target, n);
            rewrite_expr(expr, n);
        }
        Statement::Exit { value: None, .. }
        | Statement::Break { .. }
        | Statement::Continue { .. }
        | Statement::Goto { .. }
        | Statement::Label { .. } => {}
    }
}

fn rewrite_expr<R: Rename>(e: &mut Expr, n: &R) {
    n.rewrite_expr_node(e);
    match e {
        Expr::IndirectCall { callee, args, .. } => {
            rewrite_expr(callee, n);
            for a in args {
                rewrite_expr(a, n);
            }
        }
        Expr::RecordInit { fields, .. } => {
            for (name, value) in fields {
                *name = n.rename_member(name);
                rewrite_expr(value, n);
            }
        }
        Expr::ArrayInit { elements, .. } => {
            for e in elements {
                rewrite_expr(e, n);
            }
        }
        Expr::MethodCall {
            object,
            method,
            args,
            ..
        } => {
            rewrite_expr(object, n);
            *method = n.rename_member(method);
            for a in args {
                rewrite_expr(a, n);
            }
        }
        Expr::Inherited { method, args, .. } => {
            *method = n.rename_member(method);
            for a in args {
                rewrite_expr(a, n);
            }
        }
        Expr::Var(name, _) => *name = n.rename(name),
        Expr::BinOp { left, right, .. } => {
            rewrite_expr(left, n);
            rewrite_expr(right, n);
        }
        Expr::UnaryOp { operand, .. } => rewrite_expr(operand, n),
        Expr::Deref(inner, _) | Expr::AddrOf(inner, _) => rewrite_expr(inner, n),
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
            *field = n.rename_member(field);
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
        | Expr::WideCharLit(..)
        | Expr::StrLit(..)
        | Expr::CodePageStrLit(..)
        | Expr::BoolLit(..)
        | Expr::Nil(_) => {}
    }
}
