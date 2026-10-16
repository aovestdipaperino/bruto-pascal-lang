//! Unit scopes.
//!
//! A program and all its units are compiled as one module, so two units
//! that declare the same top-level name would collide. Free Vision relies
//! on that being allowed: `Drivers` declares its own `InitVideo` and
//! `GetKeyEvent` and calls the ones in `Video` and `Keyboard` as
//! `Video.InitVideo`. Before the units are merged, every name declared in
//! more than one of them gets a symbol per unit (`initvideo@video`), and
//! each reference is resolved as Pascal does: a local declaration first,
//! then the unit's own, then the last unit in its `uses` clause that
//! exports the name; `Unit.Name` picks that unit's. Names declared only
//! once are left alone.

use crate::ast::*;
use crate::rename::{Rename, rewrite_program, rewrite_unit};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// What one unit (or the program) declares.
struct Scope {
    name: String,
    uses: Vec<String>,
    /// Everything at top level, implementation included.
    declared: HashSet<String>,
    /// What other units see: the interface.
    exported: HashSet<String>,
}

/// Give names declared in more than one of `units` and `program` a
/// symbol per unit, and point every reference at the right one.
pub(crate) fn separate_unit_scopes(program: &mut Program, units: &mut [Unit]) {
    let mut scopes: Vec<Scope> = units.iter().map(unit_scope).collect();
    scopes.push(program_scope(program));

    let mut seen: HashSet<String> = HashSet::new();
    let mut conflicted: HashSet<String> = HashSet::new();
    for scope in &scopes {
        for name in &scope.declared {
            if !seen.insert(name.clone()) {
                conflicted.insert(name.clone());
            }
        }
    }
    if conflicted.is_empty() {
        return;
    }

    let unit_names: HashSet<String> = units.iter().map(|u| lower(&u.name)).collect();
    let qualified: HashMap<(String, String), String> = scopes
        .iter()
        .flat_map(|s| {
            s.exported
                .intersection(&conflicted)
                .map(|n| ((s.name.clone(), n.clone()), mangle(n, &s.name)))
        })
        .collect();
    let members = object_members(units, program);

    let renamer_for = |scope: &Scope| {
        let mut visible = HashMap::new();
        for name in &conflicted {
            if scope.declared.contains(name) {
                visible.insert(name.clone(), mangle(name, &scope.name));
                continue;
            }
            // The last unit in the `uses` clause wins.
            for used in &scope.uses {
                if let Some(s) = scopes.iter().find(|s| s.name == *used)
                    && s.exported.contains(name)
                {
                    visible.insert(name.clone(), mangle(name, used));
                }
            }
        }
        ScopeRenamer {
            conflicted: &conflicted,
            visible,
            qualified: &qualified,
            unit_names: &unit_names,
            members: &members,
            locals: RefCell::new(Vec::new()),
        }
    };
    for (unit, scope) in units.iter_mut().zip(&scopes) {
        rewrite_unit(unit, &renamer_for(scope));
    }
    rewrite_program(program, &renamer_for(scopes.last().unwrap()));
}

fn lower(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn mangle(name: &str, unit: &str) -> String {
    format!("{name}@{unit}")
}

fn unit_scope(unit: &Unit) -> Scope {
    let mut exported = HashSet::new();
    add_decls(
        &mut exported,
        &unit.interface_consts,
        &unit.interface_types,
        &unit.interface_vars,
        &unit.interface_proc_headers,
    );
    let mut declared = exported.clone();
    add_decls(
        &mut declared,
        &unit.impl_consts,
        &unit.impl_types,
        &unit.impl_vars,
        &unit.procedures,
    );
    Scope {
        name: lower(&unit.name),
        uses: unit.uses.iter().map(|u| lower(u)).collect(),
        declared,
        exported,
    }
}

fn program_scope(program: &Program) -> Scope {
    let mut declared = HashSet::new();
    add_decls(
        &mut declared,
        &program.consts,
        &program.type_decls,
        &program.vars,
        &program.procedures,
    );
    Scope {
        // Not a unit name, so nothing can qualify a name with it.
        name: "program".to_string(),
        uses: program.uses.iter().map(|u| lower(u)).collect(),
        declared,
        exported: HashSet::new(),
    }
}

fn add_decls(
    out: &mut HashSet<String>,
    consts: &[ConstDecl],
    types: &[TypeDecl],
    vars: &[VarDecl],
    procs: &[ProcDecl],
) {
    out.extend(consts.iter().map(|c| lower(&c.name)));
    for t in types {
        out.insert(lower(&t.name));
        enum_values(&t.ty, out);
    }
    out.extend(vars.iter().flat_map(|v| v.names.iter().map(|n| lower(n))));
    out.extend(
        procs
            .iter()
            .filter(|p| p.owner.is_none())
            .map(|p| lower(&p.name)),
    );
}

/// The values of enumerations declared inside `ty`, which are names in
/// the enclosing scope.
fn enum_values(ty: &PascalType, out: &mut HashSet<String>) {
    ty.for_each_enum(&mut |values, _| out.extend(values.iter().map(|v| lower(v))));
}

/// Every object type's parent and own fields and methods, which a
/// method body sees without qualification.
fn object_members(
    units: &[Unit],
    program: &Program,
) -> HashMap<String, (Option<String>, HashSet<String>)> {
    let types = units
        .iter()
        .flat_map(|u| u.interface_types.iter().chain(&u.impl_types))
        .chain(&program.type_decls);
    let mut members = HashMap::new();
    for t in types {
        if let PascalType::Object(obj) = &t.ty {
            let names = obj
                .fields
                .iter()
                .map(|(f, _)| lower(f))
                .chain(obj.methods.iter().map(|m| lower(&m.name)))
                .collect();
            members.insert(lower(&t.name), (obj.parent.as_deref().map(lower), names));
        }
    }
    members
}

struct ScopeRenamer<'a> {
    conflicted: &'a HashSet<String>,
    /// Conflicted name -> the symbol it means here.
    visible: HashMap<String, String>,
    /// (unit, name) -> symbol, for `Unit.Name`.
    qualified: &'a HashMap<(String, String), String>,
    unit_names: &'a HashSet<String>,
    members: &'a HashMap<String, (Option<String>, HashSet<String>)>,
    /// Names declared by the routines being walked, innermost last.
    locals: RefCell<Vec<HashSet<String>>>,
}

impl ScopeRenamer<'_> {
    fn is_local(&self, name: &str) -> bool {
        self.locals.borrow().iter().any(|s| s.contains(name))
    }

    /// The symbol for `unit.name`, if `unit_expr` names a unit and the
    /// name is one that was split.
    fn qualified_symbol(&self, unit_expr: &Expr, name: &str) -> Option<String> {
        let Expr::Var(unit, _) = unit_expr else {
            return None;
        };
        let unit = lower(unit);
        if !self.unit_names.contains(&unit) || self.is_local(&unit) {
            return None;
        }
        self.qualified.get(&(unit, lower(name))).cloned()
    }
}

impl Rename for ScopeRenamer<'_> {
    fn rename(&self, name: &str) -> String {
        // A qualified type name, `Video.TVideoMode`.
        if let Some((unit, base)) = name.split_once('.') {
            return self
                .qualified
                .get(&(lower(unit), lower(base)))
                .cloned()
                .unwrap_or_else(|| name.to_string());
        }
        let key = lower(name);
        if !self.conflicted.contains(&key) || self.is_local(&key) {
            return name.to_string();
        }
        self.visible
            .get(&key)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    fn rename_member(&self, name: &str) -> String {
        name.to_string()
    }

    fn enter_routine(&self, p: &ProcDecl) {
        let mut names = HashSet::new();
        for g in &p.params {
            names.extend(g.names.iter().map(|n| lower(n)));
        }
        add_decls(
            &mut names,
            &p.consts,
            &p.type_decls,
            &p.vars,
            &p.nested_procs,
        );
        names.insert("result".to_string());
        // A method sees its object's fields and methods, inherited ones
        // included.
        let mut owner = p.owner.as_deref().map(lower);
        while let Some(o) = owner {
            let Some((parent, fields)) = self.members.get(&o) else {
                break;
            };
            names.extend(fields.iter().cloned());
            owner = parent.clone();
        }
        self.locals.borrow_mut().push(names);
    }

    fn leave_routine(&self) {
        self.locals.borrow_mut().pop();
    }

    fn rewrite_expr_node(&self, expr: &mut Expr) {
        let replacement = match expr {
            Expr::FieldAccess {
                record,
                field,
                span,
            } => match self.qualified_symbol(record, field) {
                Some(symbol) => Expr::Var(symbol, *span),
                None => return,
            },
            Expr::MethodCall {
                object,
                method,
                args,
                span,
            } => match self.qualified_symbol(object, method) {
                Some(symbol) => Expr::Call {
                    name: symbol,
                    args: std::mem::take(args),
                    span: *span,
                },
                None => return,
            },
            _ => return,
        };
        *expr = replacement;
    }

    fn rewrite_stmt_node(&self, stmt: &mut Statement) {
        if let Statement::MethodCall {
            object,
            method,
            args,
            span,
        } = stmt
            && let Some(symbol) = self.qualified_symbol(object, method)
        {
            *stmt = Statement::ProcCall {
                name: symbol,
                args: std::mem::take(args),
                span: *span,
            };
        }
    }
}
