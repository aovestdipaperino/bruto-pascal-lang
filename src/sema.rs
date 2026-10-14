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
use crate::rename::{Rename, rewrite_program};
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
    let display_names = fold_identifier_case(&mut program);
    let program = fold_constant_bounds(&program)?;
    Ok(Resolved {
        program,
        display_names,
    })
}

/// Lower-case every identifier in `program`, returning the first spelling
/// seen for each. Declarations come before uses in the walk, so that is
/// normally the declared spelling.
pub fn fold_identifier_case(program: &mut Program) -> HashMap<String, String> {
    let folder = CaseFolder::default();
    rewrite_program(program, &folder);
    folder.first_spelling.into_inner()
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
        if c.ty.is_none()
            && let Some(value) = ConstExpr::from_expr(&c.value).and_then(|e| env.eval(&e).ok())
        {
            env.values.insert(c.name.clone(), value);
        }
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

#[derive(Default)]
struct ConstEnv {
    /// Integer value of every foldable untyped constant and enum value.
    values: HashMap<String, i64>,
    /// Names of the program's own types, which hide built-in type names.
    user_types: HashSet<String>,
}

/// The built-in type a (lower-case) name stands for, if it is one of
/// FPC's fixed-width integers or `Pointer`.
pub(crate) fn builtin_type(name: &str) -> Option<PascalType> {
    let sized = |bits, signed| PascalType::SizedInt { bits, signed };
    Some(match name {
        "shortint" | "int8" => sized(8, true),
        "byte" | "uint8" => sized(8, false),
        "smallint" | "int16" => sized(16, true),
        "word" | "uint16" => sized(16, false),
        "longint" | "int32" => sized(32, true),
        "longword" | "cardinal" | "dword" | "uint32" => sized(32, false),
        "qword" | "uint64" | "ptruint" | "sizeuint" | "nativeuint" => sized(64, false),
        "int64" | "ptrint" | "sizeint" | "nativeint" => PascalType::Integer,
        "pointer" | "codepointer" => PascalType::Pointer(Box::new(PascalType::Untyped)),
        _ => return None,
    })
}

impl ConstEnv {
    fn add_enum_values(&mut self, ty: &PascalType) {
        if let PascalType::Enum { values, .. } = ty {
            for (i, name) in values.iter().enumerate() {
                self.values.insert(name.clone(), i as i64);
            }
        }
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
            ConstExpr::Call(name, arg) => {
                let v = self.eval(arg)?;
                match name.as_str() {
                    "succ" => v + 1,
                    "pred" => v - 1,
                    "abs" => v.abs(),
                    // `ord` and `chr` only change the type, not the value.
                    _ => v,
                }
            }
        })
    }

    fn eval_at(&self, e: &ConstExpr, span: Span) -> Result<i64, SemaError> {
        self.eval(e).map_err(|m| SemaError::new(m, span))
    }

    fn resolve_proc(&self, proc: &mut ProcDecl) -> Result<(), SemaError> {
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
            PascalType::Record { fields, variant } => {
                for (_, t) in fields {
                    self.resolve_type(t, span)?;
                }
                if let Some(v) = variant {
                    self.resolve_type(&mut v.tag_type, span)?;
                    for (_, vfields) in &mut v.variants {
                        for (_, t) in vfields {
                            self.resolve_type(t, span)?;
                        }
                    }
                }
            }
            PascalType::Proc {
                params,
                return_type,
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
            | PascalType::Enum { .. }
            | PascalType::Subrange { .. }
            | PascalType::SizedInt { .. }
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
