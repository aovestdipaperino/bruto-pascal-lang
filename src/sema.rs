//! Semantic pass run between parsing (plus `uses` resolution) and code
//! generation.
//!
//! For now it does two things:
//! - folds every identifier to lower case, since Pascal names are
//!   case-insensitive, remembering how each was first spelled so debug
//!   info and the watch window can show the user's spelling;
//! - folds constant expressions so types can use named constants in their
//!   bounds (`array[0..Max-1] of T`, `1..MaxItems`).
//!
//! It is the seed of the full name-and-type resolution pass: code
//! generation still does its own lookups, but anything this pass rewrites
//! reaches codegen in the simple, already-resolved form it has always
//! handled.

use crate::ast::*;
use crate::rename::{Rename, rewrite_block, rewrite_proc, rewrite_program};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// An error found by the semantic pass.
#[derive(Debug, Clone)]
pub struct SemaError {
    pub message: String,
    pub span: Span,
}

impl SemaError {
    fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

/// The output of the semantic pass.
pub struct Resolved {
    /// The program with lower-case identifiers and folded bounds.
    pub program: Program,
    /// Lower-case identifier → the spelling it was first written with.
    pub display_names: HashMap<String, String>,
}

/// Run the semantic pass over a copy of `program`.
pub fn resolve_program(program: &Program) -> Result<Resolved, SemaError> {
    let mut program = program.clone();
    add_system_constants(&mut program);
    let display_names = fold_identifier_case(&mut program);
    strip_unit_qualifiers(&mut program);
    name_overloads(&mut program);
    name_method_overloads(&mut program);
    name_nested_routines(&mut program);
    avoid_c_symbols(&mut program);
    let program = fold_constant_bounds(&program)?;
    Ok(Resolved {
        program,
        display_names,
    })
}

/// FPC's System constants for a Unix target, unless the program declares
/// its own.
fn add_system_constants(program: &mut Program) {
    let declared: HashSet<String> = program
        .consts
        .iter()
        .map(|c| c.name.to_ascii_lowercase())
        .chain(
            program
                .vars
                .iter()
                .flat_map(|v| v.names.iter().map(|n| n.to_ascii_lowercase())),
        )
        .collect();
    let span = program.span;
    let constants: [(&str, Expr); 9] = [
        ("DirectorySeparator", Expr::CharLit(b'/', span)),
        ("DriveSeparator", Expr::StrLit(String::new(), span)),
        ("PathSeparator", Expr::CharLit(b':', span)),
        ("ExtensionSeparator", Expr::CharLit(b'.', span)),
        ("LineEnding", Expr::CharLit(b'\n', span)),
        ("sLineBreak", Expr::CharLit(b'\n', span)),
        ("AllFilesMask", Expr::StrLit("*".to_string(), span)),
        ("MaxPathLen", Expr::IntLit(4096, span)),
        ("FileNameCaseSensitive", Expr::BoolLit(true, span)),
    ];
    let prelude = constants
        .into_iter()
        .filter(|(name, _)| !declared.contains(&name.to_ascii_lowercase()))
        .map(|(name, value)| ConstDecl {
            name: name.to_string(),
            ty: None,
            value,
            span,
        });
    program.consts.splice(0..0, prelude);
}

/// Lower-case every identifier in `program`, returning the first spelling
/// seen for each. Declarations come before uses in the walk, so that is
/// normally the declared spelling.
pub fn fold_identifier_case(program: &mut Program) -> HashMap<String, String> {
    let folder = CaseFolder::default();
    rewrite_program(program, &folder);
    folder.first_spelling.into_inner()
}

/// C functions the generated code and its runtime call. A Pascal routine
/// of the same name would take the C function's place in the module.
const C_SYMBOLS: &[&str] = &[
    "_exit", "atan", "atexit", "atoll", "calloc", "chdir", "exit", "fclose", "feof", "fflush",
    "fgetc", "fgets", "fileno", "fopen", "fprintf", "fputc", "free", "fscanf", "fseek", "ftell",
    "getchar", "getcwd", "main", "malloc", "memcpy", "memmove", "memset", "mkdir", "printf",
    "putchar", "realloc", "rename", "rmdir", "scanf", "signal", "snprintf", "strcat", "strcmp",
    "strcpy", "strdup", "strlen", "strstr", "ungetc", "unlink", "write",
];

/// Rename Pascal routines whose name is a C function the program also
/// uses (`TTYName` calling the C `ttyname`, `GetEnv` calling `getenv`),
/// so each keeps its own symbol: `ttyname@pascal`.
fn avoid_c_symbols(program: &mut Program) {
    let c_symbols: HashSet<String> = C_SYMBOLS
        .iter()
        .map(|s| s.to_string())
        .chain(program.procedures.iter().filter_map(|p| p.external.clone()))
        .collect();
    let names: HashMap<String, String> = program
        .procedures
        .iter()
        .filter(|p| p.owner.is_none() && p.external.is_none() && c_symbols.contains(&p.name))
        .map(|p| (p.name.clone(), format!("{}@pascal", p.name)))
        .collect();
    if names.is_empty() {
        return;
    }
    rewrite_program(
        program,
        &NestedNames {
            names,
            locals: RefCell::new(Vec::new()),
        },
    );
}

/// Give every nested routine a symbol of its own, `outer/inner`, so two
/// routines can each have a nested `DragWindow` in one module. Calls and
/// references in the parent and its other nested routines follow, unless
/// a deeper declaration of the same name hides it.
fn name_nested_routines(program: &mut Program) {
    for proc in &mut program.procedures {
        let symbol = match &proc.owner {
            Some(owner) => format!("{owner}.{}", proc.name),
            None => proc.name.clone(),
        };
        name_nested_in(proc, &symbol);
    }
}

fn name_nested_in(proc: &mut ProcDecl, symbol: &str) {
    if proc.nested_procs.is_empty() {
        return;
    }
    let renamer = NestedNames {
        names: proc
            .nested_procs
            .iter()
            .map(|n| (n.name.clone(), format!("{symbol}/{}", n.name)))
            .collect(),
        locals: RefCell::new(Vec::new()),
    };
    rewrite_block(&mut proc.body, &renamer);
    for nested in &mut proc.nested_procs {
        rewrite_proc(nested, &renamer);
    }
    for nested in &mut proc.nested_procs {
        let symbol = nested.name.clone();
        name_nested_in(nested, &symbol);
    }
}

struct NestedNames {
    /// Nested routine name -> its symbol.
    names: HashMap<String, String>,
    /// Names declared in the routines being walked below the parent.
    locals: RefCell<Vec<HashSet<String>>>,
}

impl Rename for NestedNames {
    fn rename(&self, name: &str) -> String {
        match self.names.get(name) {
            Some(symbol) if !self.locals.borrow().iter().any(|l| l.contains(name)) => {
                symbol.clone()
            }
            _ => name.to_string(),
        }
    }

    fn rename_member(&self, name: &str) -> String {
        name.to_string()
    }

    fn enter_routine(&self, p: &ProcDecl) {
        let names = p
            .params
            .iter()
            .flat_map(|g| g.names.iter().cloned())
            .chain(p.vars.iter().flat_map(|v| v.names.iter().cloned()))
            .chain(p.consts.iter().map(|c| c.name.clone()))
            .chain(p.type_decls.iter().map(|t| t.name.clone()))
            .chain(p.nested_procs.iter().map(|n| n.name.clone()))
            .collect();
        self.locals.borrow_mut().push(names);
    }

    fn leave_routine(&self) {
        self.locals.borrow_mut().pop();
    }
}

/// Give each distinct signature of an overloaded top-level routine its
/// own name: the first keeps the routine's name, the others become
/// `name$2`, `name$3`... A `forward` header and its implementation share
/// one (the implementation may leave out its parameters). Calls are
/// matched to an overload by argument types during code generation.
fn name_overloads(program: &mut Program) {
    let signature = |p: &ProcDecl| {
        let params: Vec<String> = p
            .params
            .iter()
            .flat_map(|g| {
                g.names
                    .iter()
                    .map(move |_| format!("{:?} {:?}", g.mode, g.ty))
            })
            .collect();
        format!("{params:?} -> {:?}", p.return_type)
    };
    let mut by_name: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for proc in program.procedures.iter_mut().filter(|p| p.owner.is_none()) {
        let variants = by_name.entry(proc.name.clone()).or_default();
        // An implementation without parameters matches its forward header.
        let key = if proc.params.is_empty()
            && proc.return_type.is_none()
            && !proc.is_forward
            && variants.len() == 1
        {
            variants[0].0.clone()
        } else {
            signature(proc)
        };
        let symbol = match variants.iter().find(|(k, _)| *k == key) {
            Some((_, symbol)) => symbol.clone(),
            None => {
                let symbol = if variants.is_empty() {
                    proc.name.clone()
                } else {
                    format!("{}${}", proc.name, variants.len() + 1)
                };
                variants.push((key, symbol.clone()));
                symbol
            }
        };
        proc.name = symbol;
    }
    program.overloads = by_name
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(name, v)| (name, v.into_iter().map(|(_, s)| s).collect()))
        .collect();
}

/// Give each distinct signature of an overloaded method its own name, as
/// `name_overloads` does for routines: `get`, `get$2`... A method with
/// the signature of an ancestor's keeps the ancestor's name (so
/// `override` finds its slot); the implementations are renamed to match.
/// Code generation picks the overload for each call by argument types.
fn name_method_overloads(program: &mut Program) {
    fn signature(params: &[ParamGroup]) -> String {
        let params: Vec<String> = params
            .iter()
            .flat_map(|g| {
                g.names
                    .iter()
                    .map(move |_| format!("{:?} {:?}", g.mode, g.ty))
            })
            .collect();
        format!("{params:?}")
    }
    // Per type: method name -> (signature, key) for every overload, own
    // and inherited.
    let mut known: HashMap<String, HashMap<String, Vec<(String, String)>>> = HashMap::new();
    // Per type: (key, base name, signature) of its own methods.
    let mut own: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    let mut renamed = false;
    for td in program.type_decls.iter_mut() {
        let PascalType::Object(obj) = &mut td.ty else {
            continue;
        };
        if obj.is_forward {
            continue;
        }
        let parent = obj.parent.clone().or_else(|| {
            obj.is_class
                .then(|| "tobject".to_string())
                .filter(|p| *p != td.name)
        });
        let mut table = parent
            .and_then(|p| known.get(&p).cloned())
            .unwrap_or_default();
        let mut mine: Vec<(String, String, String)> = Vec::new();
        let is_class = obj.is_class;
        for m in obj.methods.iter_mut() {
            let sig = signature(&m.params);
            let declared_here = mine.iter().any(|(_, base, _)| *base == m.name);
            let variants = table.entry(m.name.clone()).or_default();
            let key = match variants.iter().find(|(s, _)| *s == sig) {
                Some((_, key)) => key.clone(),
                // In a class, another signature is an overload, callable
                // beside the ancestor's. In a TP object it hides the
                // ancestor's method, unless the type declares both.
                None => {
                    let key = if variants.is_empty() || (!is_class && !declared_here) {
                        m.name.clone()
                    } else {
                        renamed = true;
                        format!("{}${}", m.name, variants.len() + 1)
                    };
                    variants.push((sig.clone(), key.clone()));
                    key
                }
            };
            mine.push((key.clone(), m.name.clone(), sig));
            m.name = key;
        }
        known.insert(td.name.clone(), table);
        own.insert(td.name.clone(), mine);
    }
    if !renamed {
        return;
    }
    for proc in program.procedures.iter_mut() {
        let Some(owner) = &proc.owner else { continue };
        let Some(methods) = own.get(owner) else {
            continue;
        };
        let candidates: Vec<&(String, String, String)> = methods
            .iter()
            .filter(|(_, base, _)| *base == proc.name)
            .collect();
        if candidates.len() < 2 {
            if let Some((key, _, _)) = candidates.first() {
                proc.name = key.clone();
            }
            continue;
        }
        let sig = signature(&proc.params);
        if let Some((key, _, _)) = candidates.iter().find(|(_, _, s)| *s == sig) {
            proc.name = key.clone();
        }
    }
}

/// Drop `Unit.` qualifiers: units share one namespace here, so
/// `Video.InitVideo` is just `InitVideo`. Only the names of loaded units
/// (and `System`) count, and only when the program doesn't declare
/// something of the same name itself.
fn strip_unit_qualifiers(program: &mut Program) {
    let declared: HashSet<String> = program
        .vars
        .iter()
        .flat_map(|v| v.names.iter().cloned())
        .chain(program.consts.iter().map(|c| c.name.clone()))
        .chain(program.type_decls.iter().map(|t| t.name.clone()))
        .chain(program.procedures.iter().map(|p| p.name.clone()))
        .collect();
    let units: HashSet<String> = program
        .units
        .iter()
        .map(|u| u.to_ascii_lowercase())
        .chain(std::iter::once("system".to_string()))
        .filter(|u| !declared.contains(u))
        .collect();
    rewrite_program(
        program,
        &UnitQualifiers {
            units,
            locals: RefCell::new(Vec::new()),
        },
    );
}

struct UnitQualifiers {
    units: HashSet<String>,
    /// Names declared by the routines being walked: a local `Mouse` is
    /// not the unit.
    locals: RefCell<Vec<HashSet<String>>>,
}

impl UnitQualifiers {
    fn is_unit(&self, expr: &Expr) -> bool {
        matches!(expr, Expr::Var(name, _)
            if self.units.contains(name)
                && !self.locals.borrow().iter().any(|l| l.contains(name)))
    }

    /// `System.X` must reach the built-in even where a method or routine
    /// named X would otherwise win, so it keeps a `system.` prefix that
    /// code generation understands. Other units' qualifiers just go.
    fn qualified(&self, unit: &Expr, name: &str) -> String {
        match unit {
            Expr::Var(u, _) if u == "system" => format!("system.{name}"),
            _ => name.to_string(),
        }
    }
}

impl Rename for UnitQualifiers {
    fn enter_routine(&self, p: &ProcDecl) {
        let names = p
            .params
            .iter()
            .flat_map(|g| g.names.iter().cloned())
            .chain(p.vars.iter().flat_map(|v| v.names.iter().cloned()))
            .chain(p.consts.iter().map(|c| c.name.clone()))
            .chain(p.nested_procs.iter().map(|n| n.name.clone()))
            .collect();
        self.locals.borrow_mut().push(names);
    }

    fn leave_routine(&self) {
        self.locals.borrow_mut().pop();
    }

    fn rename(&self, name: &str) -> String {
        // A qualified type name, `objects.trect`. (`system.x` is kept: it
        // marks a call that must reach the built-in.)
        match name.split_once('.') {
            Some((unit, base)) if unit != "system" && self.units.contains(unit) => base.to_string(),
            _ => name.to_string(),
        }
    }

    fn rewrite_expr_node(&self, expr: &mut Expr) {
        let replacement = match expr {
            Expr::FieldAccess {
                record,
                field,
                span,
            } if self.is_unit(record) => Expr::Var(self.qualified(record, field), *span),
            Expr::MethodCall {
                object,
                method,
                args,
                span,
            } if self.is_unit(object) => Expr::Call {
                name: self.qualified(object, method),
                args: std::mem::take(args),
                span: *span,
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
            && self.is_unit(object)
        {
            *stmt = Statement::ProcCall {
                name: self.qualified(object, method),
                args: std::mem::take(args),
                span: *span,
            };
        }
    }
}

#[derive(Default)]
struct CaseFolder {
    first_spelling: RefCell<HashMap<String, String>>,
}

impl Rename for CaseFolder {
    fn rename(&self, name: &str) -> String {
        let lower = name.to_ascii_lowercase();
        self.first_spelling
            .borrow_mut()
            .entry(lower.clone())
            .or_insert_with(|| name.to_string());
        lower
    }
}

/// Return a copy of `program` with every constant-expression bound folded.
fn fold_constant_bounds(program: &Program) -> Result<Program, SemaError> {
    let mut env = ConstEnv {
        user_types: program.type_decls.iter().map(|t| t.name.clone()).collect(),
        ..ConstEnv::default()
    };
    for td in &program.type_decls {
        env.add_enum_values(&td.ty);
    }
    for var in &program.vars {
        env.add_enum_values(&var.ty);
    }
    for c in &program.consts {
        // Typed constants are initialised variables in Turbo Pascal, so
        // they can't appear where a true constant is required.
        if c.ty.is_none() {
            if let Expr::StrLit(text, _) = &c.value {
                env.string_lengths
                    .insert(c.name.clone(), text.chars().count() as i64);
            }
            if let Some(value) = ConstExpr::from_expr(&c.value).and_then(|e| env.eval(&e).ok()) {
                env.values.insert(c.name.clone(), value);
            }
        }
    }
    for td in &program.type_decls {
        env.add_type_bounds(&td.name, &td.ty);
    }

    let mut out = program.clone();
    for c in &mut out.consts {
        if let Some(ty) = &mut c.ty {
            env.resolve_type(ty, c.span)?;
        }
    }
    for td in &mut out.type_decls {
        env.resolve_type(&mut td.ty, td.span)?;
    }
    for var in &mut out.vars {
        env.resolve_type(&mut var.ty, var.span)?;
    }
    for proc in &mut out.procedures {
        env.resolve_proc(proc)?;
    }
    Ok(out)
}

#[derive(Default, Clone)]
struct ConstEnv {
    /// Integer value of every foldable untyped constant and enum value.
    values: HashMap<String, i64>,
    /// Names of the program's own types, which hide built-in type names.
    user_types: HashSet<String>,
    /// Lengths of untyped string constants, for `length(C)`.
    string_lengths: HashMap<String, i64>,
    /// Bounds of enumeration and subrange types, for `low(T)` / `high(T)`.
    type_bounds: HashMap<String, (i64, i64)>,
}

/// The four basic types that are keywords in Bruto's lexer, by name.
fn basic_type(name: &str) -> Option<PascalType> {
    Some(match name {
        "integer" => PascalType::Integer,
        "real" => PascalType::Real,
        "char" => PascalType::Char,
        "boolean" => PascalType::Boolean,
        "string" => PascalType::String,
        _ => return None,
    })
}

/// The size in bytes of a built-in type, for `sizeof` in constants.
fn builtin_size(name: &str) -> Option<i64> {
    Some(match name {
        "byte" | "shortint" | "int8" | "uint8" | "char" | "ansichar" | "boolean" => 1,
        "word" | "smallint" | "int16" | "uint16" | "widechar" | "unicodechar" => 2,
        "longint" | "longword" | "cardinal" | "dword" | "int32" | "uint32" => 4,
        "integer" | "int64" | "qword" | "uint64" | "ptrint" | "ptruint" | "sizeint"
        | "sizeuint" | "nativeint" | "nativeuint" | "real" | "double" | "pointer"
        | "codepointer" | "string" | "unicodestring" | "widestring" | "ansistring" => 8,
        _ => return None,
    })
}

/// The built-in type a (lower-case) name stands for, if it is one of
/// FPC's fixed-width integers or `Pointer`.
pub(crate) fn builtin_type(name: &str) -> Option<PascalType> {
    let sized = |bits, signed| PascalType::SizedInt { bits, signed };
    // `PByte`, `PChar`, `PString`, `PPointer`...: a pointer to a built-in.
    if let Some(target) = name.strip_prefix('p')
        && !target.is_empty()
        && name != "pointer"
        && let Some(target) = basic_type(target).or_else(|| builtin_type(target))
    {
        return Some(PascalType::Pointer(Box::new(target)));
    }
    Some(match name {
        "ansichar" => PascalType::Char,
        "widechar" | "unicodechar" => PascalType::WideChar,
        // `OpenString` (a `var` parameter taking any short string) is
        // treated as the longest one.
        "shortstring" | "openstring" => PascalType::ShortString { max: 255 },
        "ansistring" | "rawbytestring" | "utf8string" => PascalType::String,
        "unicodestring" | "widestring" => PascalType::UnicodeString,
        "single" | "double" | "extended" | "currency" | "comp" => PascalType::Real,
        // A file variable is one pointer to the runtime's file record;
        // seen as FPC's `FileRec` / `TextRec`, that pointer is the handle.
        "filerec" | "textrec" => PascalType::Record {
            fields: vec![("handle".to_string(), PascalType::Integer)],
            variant: None,
            object: None,
        },
        "bytebool" | "wordbool" | "longbool" => PascalType::Boolean,
        "shortint" | "int8" => sized(8, true),
        "byte" | "uint8" => sized(8, false),
        "smallint" | "int16" => sized(16, true),
        "word" | "uint16" => sized(16, false),
        "longint" | "int32" => sized(32, true),
        "longword" | "cardinal" | "dword" | "uint32" => sized(32, false),
        "qword" | "uint64" | "ptruint" | "sizeuint" | "nativeuint" => sized(64, false),
        "int64" | "ptrint" | "sizeint" | "nativeint" => PascalType::Integer,
        "pointer" | "codepointer" => PascalType::Pointer(Box::new(PascalType::Untyped)),
        "text" => PascalType::File {
            elem: Box::new(PascalType::Char),
        },
        _ => return None,
    })
}

impl ConstEnv {
    fn add_enum_values(&mut self, ty: &PascalType) {
        ty.for_each_enum(&mut |values| {
            for (i, name) in values.iter().enumerate() {
                self.values.insert(name.clone(), i as i64);
            }
        });
    }

    fn eval(&self, e: &ConstExpr) -> Result<i64, String> {
        Ok(match e {
            ConstExpr::Int(n) => *n,
            ConstExpr::Char(c) => *c as i64,
            ConstExpr::Name(name) => match self.values.get(name) {
                Some(v) => *v,
                None if name.eq_ignore_ascii_case("maxint") => i64::MAX,
                None => return Err(format!("'{name}' is not a constant")),
            },
            ConstExpr::Neg(inner) => -self.eval(inner)?,
            ConstExpr::Binary(op, l, r) => {
                let (l, r) = (self.eval(l)?, self.eval(r)?);
                match op {
                    ConstOp::Add => l.wrapping_add(r),
                    ConstOp::Sub => l.wrapping_sub(r),
                    ConstOp::Mul => l.wrapping_mul(r),
                    ConstOp::Div | ConstOp::Mod if r == 0 => {
                        return Err("division by zero in constant expression".into());
                    }
                    ConstOp::Div => l / r,
                    ConstOp::Mod => l % r,
                    ConstOp::And => l & r,
                    ConstOp::Or => l | r,
                    ConstOp::Xor => l ^ r,
                    ConstOp::Shl => l.wrapping_shl(r as u32),
                    ConstOp::Shr => ((l as u64).wrapping_shr(r as u32)) as i64,
                }
            }
            ConstExpr::Call(name, arg)
                if matches!(name.as_str(), "length" | "sizeof" | "low" | "high") =>
            {
                let ConstExpr::Name(subject) = arg.as_ref() else {
                    return Err(format!("{name}() of this isn't a constant"));
                };
                match name.as_str() {
                    "length" => *self
                        .string_lengths
                        .get(subject)
                        .ok_or_else(|| format!("'{subject}' is not a string constant"))?,
                    "sizeof" => builtin_size(subject)
                        .ok_or_else(|| format!("sizeof({subject}) isn't known here"))?,
                    _ => {
                        let (lo, hi) = self
                            .bounds_of(subject)
                            .ok_or_else(|| format!("'{subject}' has no bounds"))?;
                        if name == "low" { lo } else { hi }
                    }
                }
            }
            ConstExpr::Call(name, arg) => {
                let v = self.eval(arg)?;
                match name.as_str() {
                    "succ" => v + 1,
                    "pred" => v - 1,
                    "abs" => v.abs(),
                    "lo" => v & 0xFF,
                    "hi" => (v >> 8) & 0xFF,
                    // `ord` and `chr` only change the type, not the value.
                    _ => v,
                }
            }
        })
    }

    /// Record the bounds of an enumeration or subrange type, or of an
    /// alias of a type whose bounds are known.
    fn add_type_bounds(&mut self, name: &str, ty: &PascalType) {
        let bounds = match ty {
            PascalType::Enum { values, .. } => Some((0, values.len() as i64 - 1)),
            PascalType::Subrange { lo, hi } => Some((*lo, *hi)),
            PascalType::SubrangeExpr { lo, hi } => self.eval(lo).ok().zip(self.eval(hi).ok()),
            PascalType::Named(other) => self.bounds_of(other),
            _ => None,
        };
        if let Some(bounds) = bounds {
            self.type_bounds.insert(name.to_string(), bounds);
        }
    }

    /// `low(T)..high(T)` for an ordinal type name.
    fn bounds_of(&self, name: &str) -> Option<(i64, i64)> {
        if let Some(b) = self.type_bounds.get(name) {
            return Some(*b);
        }
        Some(match name {
            "boolean" => (0, 1),
            "char" | "byte" | "uint8" | "ansichar" => (0, 255),
            "shortint" | "int8" => (-128, 127),
            "smallint" | "int16" => (-32768, 32767),
            "word" | "uint16" => (0, 65535),
            "longint" | "int32" => (i32::MIN as i64, i32::MAX as i64),
            "longword" | "cardinal" | "dword" | "uint32" => (0, u32::MAX as i64),
            "integer" | "int64" | "ptrint" | "sizeint" | "nativeint" => (i64::MIN, i64::MAX),
            _ => return None,
        })
    }

    fn eval_at(&self, e: &ConstExpr, span: Span) -> Result<i64, SemaError> {
        self.eval(e).map_err(|m| SemaError::new(m, span))
    }

    fn resolve_proc(&self, proc: &mut ProcDecl) -> Result<(), SemaError> {
        // Local types and constants are visible in this routine only.
        let mut local;
        let env = if proc.consts.is_empty() && proc.type_decls.is_empty() {
            self
        } else {
            local = self.clone();
            for td in &proc.type_decls {
                local.user_types.insert(td.name.clone());
                local.add_enum_values(&td.ty);
            }
            for c in &proc.consts {
                if c.ty.is_none()
                    && let Some(value) =
                        ConstExpr::from_expr(&c.value).and_then(|e| local.eval(&e).ok())
                {
                    local.values.insert(c.name.clone(), value);
                }
            }
            for td in &proc.type_decls {
                local.add_type_bounds(&td.name, &td.ty);
            }
            for td in &mut proc.type_decls {
                local.resolve_type(&mut td.ty, td.span)?;
            }
            for c in &mut proc.consts {
                if let Some(ty) = &mut c.ty {
                    local.resolve_type(ty, c.span)?;
                }
            }
            &local
        };
        env.resolve_proc_body(proc)
    }

    fn resolve_proc_body(&self, proc: &mut ProcDecl) -> Result<(), SemaError> {
        let span = proc.span;
        for group in &mut proc.params {
            self.resolve_type(&mut group.ty, span)?;
        }
        if let Some(ret) = &mut proc.return_type {
            self.resolve_type(ret, span)?;
        }
        for var in &mut proc.vars {
            self.resolve_type(&mut var.ty, var.span)?;
        }
        for nested in &mut proc.nested_procs {
            self.resolve_proc(nested)?;
        }
        Ok(())
    }

    /// Rewrite `ty` in place, folding every constant-expression bound.
    fn resolve_type(&self, ty: &mut PascalType, span: Span) -> Result<(), SemaError> {
        match ty {
            PascalType::Object(obj) => {
                for (_, t) in &mut obj.fields {
                    self.resolve_type(t, span)?;
                }
                for m in &mut obj.methods {
                    for group in &mut m.params {
                        self.resolve_type(&mut group.ty, m.span)?;
                    }
                    if let Some(rt) = &mut m.return_type {
                        self.resolve_type(rt, m.span)?;
                    }
                }
                for p in &mut obj.properties {
                    for group in &mut p.params {
                        self.resolve_type(&mut group.ty, p.span)?;
                    }
                    self.resolve_type(&mut p.ty, p.span)?;
                }
            }
            PascalType::ClassOf(_) => {}
            PascalType::VarParam(inner)
            | PascalType::DynArray(inner)
            | PascalType::OpenArray(inner) => self.resolve_type(inner, span)?,
            PascalType::ArrayExpr { lo, hi, elem } => {
                let lo = self.eval_at(lo, span)?;
                let hi = self.eval_at(hi, span)?;
                let mut elem = std::mem::replace(elem, Box::new(PascalType::Integer));
                self.resolve_type(&mut elem, span)?;
                *ty = PascalType::Array { lo, hi, elem };
            }
            PascalType::SubrangeExpr { lo, hi } => {
                let lo = self.eval_at(lo, span)?;
                let hi = self.eval_at(hi, span)?;
                *ty = PascalType::Subrange { lo, hi };
            }
            PascalType::Array { elem, .. }
            | PascalType::Pointer(elem)
            | PascalType::Set { elem }
            | PascalType::File { elem }
            | PascalType::ConformantArray { elem, .. } => self.resolve_type(elem, span)?,
            PascalType::Record {
                fields, variant, ..
            } => {
                for (_, t) in fields {
                    self.resolve_type(t, span)?;
                }
                if let Some(v) = variant {
                    self.resolve_type(&mut v.tag_type, span)?;
                    for (labels, vfields) in &mut v.variants {
                        for label in labels.iter_mut() {
                            *label = ConstExpr::Int(self.eval_at(label, span)?);
                        }
                        for (_, t) in vfields {
                            self.resolve_type(t, span)?;
                        }
                    }
                }
            }
            PascalType::Proc {
                params,
                return_type,
                ..
            } => {
                for p in params {
                    self.resolve_type(p, span)?;
                }
                if let Some(r) = return_type {
                    self.resolve_type(r, span)?;
                }
            }
            PascalType::Integer
            | PascalType::Real
            | PascalType::String
            | PascalType::Boolean
            | PascalType::Char
            | PascalType::WideChar
            | PascalType::UnicodeString
            | PascalType::Enum { .. }
            | PascalType::Subrange { .. }
            | PascalType::SizedInt { .. }
            | PascalType::ShortString { .. }
            | PascalType::Untyped => {}
            PascalType::Named(name) => {
                if !self.user_types.contains(name)
                    && let Some(builtin) = builtin_type(name)
                {
                    *ty = builtin;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn resolve(src: &str) -> Result<Program, SemaError> {
        let program = Parser::new(src).parse_program().expect("parse");
        resolve_program(&program).map(|r| r.program)
    }

    #[test]
    fn folds_named_constant_bounds() {
        let p = resolve(
            "program T;\nconst Max = 10;\n  Last = Max - 1;\ntype Buf = array[0..Last] of integer;\n  Idx = 1..Max * 2;\nvar b: Buf;\nbegin\nend.\n",
        )
        .unwrap();
        assert!(matches!(
            p.type_decls[0].ty,
            PascalType::Array { lo: 0, hi: 9, .. }
        ));
        assert_eq!(p.type_decls[1].ty, PascalType::Subrange { lo: 1, hi: 20 });
    }

    #[test]
    fn folds_enum_and_char_bounds() {
        let p = resolve(
            "program T;\ntype Color = (Red, Green, Blue);\n  Shades = array[Red..Blue] of integer;\n  Codes = array[0..ord('A')] of char;\nbegin\nend.\n",
        )
        .unwrap();
        assert!(matches!(
            p.type_decls[1].ty,
            PascalType::Array { lo: 0, hi: 2, .. }
        ));
        assert!(matches!(
            p.type_decls[2].ty,
            PascalType::Array { lo: 0, hi: 65, .. }
        ));
    }

    #[test]
    fn reports_unknown_constant_in_bound() {
        let err =
            resolve("program T;\nvar a: array[1..Missing] of integer;\nbegin\nend.\n").unwrap_err();
        assert!(err.message.contains("missing"), "{}", err.message);
        assert_eq!(err.span.line, 2);
    }

    #[test]
    fn typed_constants_are_not_usable_as_bounds() {
        let err = resolve(
            "program T;\nconst N: integer = 3;\nvar a: array[1..N] of integer;\nbegin\nend.\n",
        )
        .unwrap_err();
        assert!(err.message.contains("'n'"), "{}", err.message);
    }

    #[test]
    fn identifiers_fold_to_lower_case_and_keep_first_spelling() {
        let program = Parser::new(
            "program T;\nvar Counter: integer;\nbegin\n  COUNTER := 1;\n  WriteLn(counter)\nend.\n",
        )
        .parse_program()
        .unwrap();
        let r = resolve_program(&program).unwrap();
        assert_eq!(r.program.vars[0].names, ["counter"]);
        assert_eq!(r.display_names["counter"], "Counter");
        let Statement::Assignment { target, .. } = &r.program.body.statements[0] else {
            panic!("expected assignment");
        };
        assert_eq!(target, "counter");
    }
}
