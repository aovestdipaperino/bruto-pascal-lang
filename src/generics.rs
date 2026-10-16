//! Generics: `generic TList<T> = class ... end` and its specializations
//! (`specialize TList<Integer>`, or `TList<Integer>` in Delphi mode).
//!
//! As FPC does, each specialization is a copy of the generic type and of
//! its methods with the type parameters replaced, made once per distinct
//! list of arguments and named `tlist<integer>`. Each parameter becomes an
//! alias type (`tlist<integer>$t = integer`), so casts, `SizeOf(T)` and
//! local variables of type `T` keep working through the ordinary type
//! machinery; inside the generic, its own name means the specialization.
//! The copies are placed before the declaration that first uses them, so
//! a class's ancestors and field types are ready before it.

use crate::ast::*;
use crate::rename::{Rename, rewrite_proc, rewrite_type};
use std::collections::{HashMap, HashSet};

/// Replaces the type parameters (and the generic's own name).
struct Substitution {
    names: HashMap<String, String>,
}

impl Rename for Substitution {
    fn rename(&self, name: &str) -> String {
        self.names
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    fn rename_member(&self, name: &str) -> String {
        name.to_string()
    }
}

struct Generics {
    decls: HashMap<String, TypeDecl>,
    methods: HashMap<String, Vec<ProcDecl>>,
    done: HashSet<String>,
    out: Vec<TypeDecl>,
    procs: Vec<ProcDecl>,
    errors: Vec<(String, Span)>,
    /// The type parameters' aliases: alias -> the argument it stands for.
    aliases: HashMap<String, PascalType>,
    /// Specializations written in a generic's code (`specialize
    /// TEnum<T>.Create`): the hidden alias's name -> the generic and its
    /// arguments, made again for each instantiation.
    templated: HashMap<String, (String, Vec<PascalType>)>,
}

/// A short name for a type argument, for the specialization's name (a
/// type parameter's alias stands for its argument).
fn type_key(ty: &PascalType, aliases: &HashMap<String, PascalType>) -> String {
    let type_key = |t: &PascalType| type_key(t, aliases);
    match ty {
        PascalType::Named(n) => match aliases.get(n) {
            Some(arg) => type_key(arg),
            None => n.clone(),
        },
        PascalType::Integer => "integer".into(),
        PascalType::Real => "real".into(),
        PascalType::String => "string".into(),
        PascalType::UnicodeString => "unicodestring".into(),
        PascalType::Boolean => "boolean".into(),
        PascalType::Char => "char".into(),
        PascalType::WideChar => "widechar".into(),
        PascalType::ShortString { max } => format!("string[{max}]"),
        PascalType::SizedInt { bits, signed } => {
            format!("{}int{bits}", if *signed { "" } else { "u" })
        }
        PascalType::Pointer(inner) => format!("^{}", type_key(inner)),
        PascalType::DynArray(inner) => format!("array of {}", type_key(inner)),
        other => {
            let text = format!("{other:?}");
            let hash = text.bytes().fold(0xcbf29ce484222325u64, |h, b| {
                (h ^ b as u64).wrapping_mul(0x100000001b3)
            });
            format!("t{hash:016x}")
        }
    }
}

impl Generics {
    /// Replace the specializations in `ty` by their instantiations' names,
    /// and type parameters' aliases by the arguments themselves (so equal
    /// signatures compare equal).
    fn walk_type(&mut self, ty: &mut PascalType, span: Span) {
        if let PascalType::Named(n) = ty
            && let Some(arg) = self.aliases.get(n).cloned()
        {
            *ty = arg;
            return;
        }
        match ty {
            PascalType::Specialize { name, args } => {
                for a in args.iter_mut() {
                    self.walk_type(a, span);
                }
                let name = name.clone();
                let args = args.clone();
                *ty = PascalType::Named(self.instantiate(&name, &args, span));
            }
            PascalType::Object(obj) => {
                // `class(TBox<T>)`: the parent's specialization.
                if !obj.parent_args.is_empty()
                    && let Some(parent) = obj.parent.clone()
                {
                    let mut args = std::mem::take(&mut obj.parent_args);
                    for a in args.iter_mut() {
                        self.walk_type(a, span);
                    }
                    obj.parent = Some(self.instantiate(&parent, &args, span));
                }
                for (_, t) in obj.fields.iter_mut().chain(obj.class_vars.iter_mut()) {
                    self.walk_type(t, span);
                }
                for m in &mut obj.methods {
                    for g in &mut m.params {
                        self.walk_type(&mut g.ty, span);
                    }
                    if let Some(rt) = &mut m.return_type {
                        self.walk_type(rt, span);
                    }
                }
                for p in &mut obj.properties {
                    for g in &mut p.params {
                        self.walk_type(&mut g.ty, span);
                    }
                    self.walk_type(&mut p.ty, span);
                }
            }
            PascalType::Record {
                fields, variant, ..
            } => {
                for (_, t) in fields {
                    self.walk_type(t, span);
                }
                if let Some(v) = variant {
                    self.walk_type(&mut v.tag_type, span);
                    for (_, vf) in &mut v.variants {
                        for (_, t) in vf {
                            self.walk_type(t, span);
                        }
                    }
                }
            }
            PascalType::Array { elem, .. }
            | PascalType::ArrayExpr { elem, .. }
            | PascalType::ConformantArray { elem, .. }
            | PascalType::Pointer(elem)
            | PascalType::Set { elem }
            | PascalType::File { elem }
            | PascalType::DynArray(elem)
            | PascalType::OpenArray(elem)
            | PascalType::VarParam(elem) => self.walk_type(elem, span),
            PascalType::Proc {
                params,
                return_type,
                ..
            } => {
                for p in params {
                    self.walk_type(p, span);
                }
                if let Some(rt) = return_type {
                    self.walk_type(rt, span);
                }
            }
            _ => {}
        }
    }

    fn walk_proc(&mut self, p: &mut ProcDecl) {
        let span = p.span;
        for g in &mut p.params {
            self.walk_type(&mut g.ty, span);
        }
        if let Some(rt) = &mut p.return_type {
            self.walk_type(rt, span);
        }
        for td in &mut p.type_decls {
            self.walk_type(&mut td.ty, span);
        }
        for v in &mut p.vars {
            self.walk_type(&mut v.ty, span);
        }
        for c in &mut p.consts {
            if let Some(t) = &mut c.ty {
                self.walk_type(t, span);
            }
        }
        for n in &mut p.nested_procs {
            self.walk_proc(n);
        }
    }

    /// The name of `name<args>`, made (with its methods) if new.
    fn instantiate(&mut self, name: &str, args: &[PascalType], span: Span) -> String {
        let keys: Vec<String> = args.iter().map(|a| type_key(a, &self.aliases)).collect();
        let mangled = format!("{name}<{}>", keys.join(","));
        if self.done.contains(&mangled) {
            return mangled;
        }
        let Some(generic) = self.decls.get(name).cloned() else {
            self.errors
                .push((format!("'{name}' is not a generic type"), span));
            return name.to_string();
        };
        if generic.type_params.len() != args.len() {
            self.errors.push((
                format!(
                    "'{name}' takes {} type argument(s), found {}",
                    generic.type_params.len(),
                    args.len()
                ),
                span,
            ));
            return name.to_string();
        }
        self.done.insert(mangled.clone());
        let mut names = HashMap::new();
        names.insert(name.to_string(), mangled.clone());
        let mut aliases = Vec::new();
        for (param, arg) in generic.type_params.iter().zip(args) {
            let alias = format!("{mangled}${param}");
            names.insert(param.clone(), alias.clone());
            self.aliases.insert(alias.clone(), arg.clone());
            aliases.push(TypeDecl {
                name: alias,
                ty: arg.clone(),
                span: generic.span,
                type_params: Vec::new(),
            });
        }
        // The specializations in its code that use its parameters.
        let own_params: HashSet<String> = generic.type_params.iter().cloned().collect();
        for (hidden, (g2, args2)) in self.templated.clone() {
            if !args2.iter().any(|a| mentions(a, &own_params)) {
                continue;
            }
            let partial = Substitution {
                names: names.clone(),
            };
            let mut args2 = args2.clone();
            for a in args2.iter_mut() {
                rewrite_type(a, &partial);
                self.walk_type(a, span);
            }
            let made = self.instantiate(&g2, &args2, span);
            names.insert(hidden, made);
        }
        let subst = Substitution { names };
        let mut inst = generic.clone();
        inst.name = mangled.clone();
        inst.type_params.clear();
        rewrite_type(&mut inst.ty, &subst);
        // What it uses comes first.
        self.walk_type(&mut inst.ty, span);
        for mut m in self.methods.get(name).cloned().unwrap_or_default() {
            rewrite_proc(&mut m, &subst);
            m.owner = Some(mangled.clone());
            self.walk_proc(&mut m);
            self.procs.push(m);
        }
        self.out.extend(aliases);
        self.out.push(inst);
        mangled
    }
}

/// Whether `ty` names one of `params`.
fn mentions(ty: &PascalType, params: &HashSet<String>) -> bool {
    match ty {
        PascalType::Named(n) => params.contains(n),
        PascalType::Specialize { args, .. } => args.iter().any(|a| mentions(a, params)),
        PascalType::Pointer(t) | PascalType::DynArray(t) | PascalType::OpenArray(t) => {
            mentions(t, params)
        }
        _ => false,
    }
}

fn has_specialization(ty: &PascalType) -> bool {
    format!("{ty:?}").contains("Specialize")
}

/// Make the specializations `program` uses and drop the generics
/// themselves. Errors name an unknown generic or a wrong argument count.
pub(crate) fn instantiate_generics(program: &mut Program) -> Result<(), (String, Span)> {
    if !program.type_decls.iter().any(|t| !t.type_params.is_empty())
        && !format!("{:?}", program.type_decls).contains("Specialize")
        && !format!("{:?}", program.procedures).contains("Specialize")
        && !program.vars.iter().any(|v| has_specialization(&v.ty))
    {
        return Ok(());
    }
    let (generic, rest): (Vec<TypeDecl>, Vec<TypeDecl>) = std::mem::take(&mut program.type_decls)
        .into_iter()
        .partition(|t| !t.type_params.is_empty());
    let generic_names: HashSet<String> = generic.iter().map(|t| t.name.clone()).collect();
    let (methods, procs): (Vec<ProcDecl>, Vec<ProcDecl>) = std::mem::take(&mut program.procedures)
        .into_iter()
        .partition(|p| p.owner.as_ref().is_some_and(|o| generic_names.contains(o)));
    let mut by_owner: HashMap<String, Vec<ProcDecl>> = HashMap::new();
    for m in methods {
        by_owner
            .entry(m.owner.clone().unwrap())
            .or_default()
            .push(m);
    }
    let mut g = Generics {
        decls: generic.into_iter().map(|t| (t.name.clone(), t)).collect(),
        methods: by_owner,
        done: HashSet::new(),
        out: Vec::new(),
        procs: Vec::new(),
        errors: Vec::new(),
        aliases: HashMap::new(),
        templated: HashMap::new(),
    };
    // Hidden aliases of specializations naming a generic's parameters
    // belong to that generic's code.
    let params: HashSet<String> = g
        .decls
        .values()
        .flat_map(|d| d.type_params.iter().cloned())
        .collect();
    let mut kept = Vec::new();
    for td in rest {
        match &td.ty {
            PascalType::Specialize { name, args }
                if td.name.starts_with("specialize$")
                    && args.iter().any(|a| mentions(a, &params)) =>
            {
                g.templated
                    .insert(td.name.clone(), (name.clone(), args.clone()));
            }
            _ => kept.push(td),
        }
    }
    let rest = kept;
    for mut td in rest {
        let span = td.span;
        g.walk_type(&mut td.ty, span);
        g.out.push(td);
    }
    for v in &mut program.vars {
        let span = v.span;
        g.walk_type(&mut v.ty, span);
    }
    for c in &mut program.consts {
        if let Some(t) = &mut c.ty {
            g.walk_type(t, c.span);
        }
    }
    let mut procs = procs;
    for p in &mut procs {
        g.walk_proc(p);
    }
    if let Some(e) = g.errors.into_iter().next() {
        return Err(e);
    }
    program.type_decls = g.out;
    procs.extend(g.procs);
    program.procedures = procs;
    Ok(())
}
