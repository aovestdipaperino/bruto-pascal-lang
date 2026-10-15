//! Units that ship with the compiler: Bruto's versions of the FPC
//! console and Unix units Free Vision is built on. A program's own files
//! come first on the search path; these are the fallback.
//!
//! They are embedded in the compiler and written out on first use, so
//! errors and the debugger can point at real files.

use crate::ast::{PascalType, Program};
use crate::parser::{ParseOptions, Parser};
use std::collections::HashSet;
use std::path::PathBuf;

const SYSTEM_PRELUDE: &str = include_str!("../rtl/system.pp");

/// The string routines code generation calls; every program gets them.
const STRINGS: &str = include_str!("../rtl/brutostrings.pp");

/// `TObject` and `TClass`, for programs that declare classes.
const OBJPAS: &str = include_str!("../rtl/objpas.pp");

const UNITS: &[(&str, &str)] = &[
    ("unixtype.pp", include_str!("../rtl/unixtype.pp")),
    ("baseunix.pp", include_str!("../rtl/baseunix.pp")),
    ("unix.pp", include_str!("../rtl/unix.pp")),
    ("termio.pp", include_str!("../rtl/termio.pp")),
    ("dos.pp", include_str!("../rtl/dos.pp")),
    ("cwstring.pp", include_str!("../rtl/cwstring.pp")),
    ("fpwidestring.pp", include_str!("../rtl/fpwidestring.pp")),
    (
        "graphemebreakproperty.pp",
        include_str!("../rtl/graphemebreakproperty.pp"),
    ),
    ("sysutils.pp", include_str!("../rtl/sysutils.pp")),
    ("brutoconsole.pp", include_str!("../rtl/brutoconsole.pp")),
    ("video.pp", include_str!("../rtl/video.pp")),
    ("keyboard.pp", include_str!("../rtl/keyboard.pp")),
    ("mouse.pp", include_str!("../rtl/mouse.pp")),
    ("sockets.pp", include_str!("../rtl/sockets.pp")),
    ("netdb.pp", include_str!("../rtl/netdb.pp")),
    ("netobjects.pp", include_str!("../rtl/netobjects.pp")),
];

/// The directory the built-in units are written to.
fn rtl_dir() -> PathBuf {
    std::env::temp_dir().join(format!("bruto-rtl-{}", env!("CARGO_PKG_VERSION")))
}

/// Whether `path` is one of the compiler's own runtime files (written to
/// the `bruto-rtl-<version>` directory, however the temp directory is
/// spelled).
pub(crate) fn is_runtime_file(path: &std::path::Path) -> bool {
    let dir = rtl_dir();
    let name = dir.file_name().unwrap_or_default();
    path.parent().and_then(|p| p.file_name()) == Some(name)
}

/// Whether `path` is the string runtime (`rtl/brutostrings.pp`).
pub(crate) fn is_string_runtime(path: &std::path::Path) -> bool {
    is_runtime_file(path) && path.file_name().is_some_and(|n| n == "brutostrings.pp")
}

/// The file of built-in unit `name`, if there is one.
pub(crate) fn unit_file(name: &str) -> Option<PathBuf> {
    let file = format!("{}.pp", name.to_ascii_lowercase());
    let (_, source) = UNITS.iter().find(|(f, _)| *f == file)?;
    let dir = rtl_dir();
    let path = dir.join(&file);
    // Rewrite it if a different compiler build left another version.
    if std::fs::read_to_string(&path).ok().as_deref() != Some(*source) {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&path, source).ok()?;
    }
    Some(path)
}

/// Add the System routines and types written in Pascal (`rtl/system.pp`)
/// that the program and its units don't declare themselves.
pub(crate) fn add_system_prelude(program: &mut Program) -> Result<(), String> {
    add_prelude(program, "system.pp", SYSTEM_PRELUDE)
}

/// Add the string routines (`rtl/brutostrings.pp`), and for a program
/// that declares classes, `TObject` (`rtl/objpas.pp`).
pub(crate) fn add_string_runtime(program: &mut Program) -> Result<(), String> {
    let has_classes = program.uses_exceptions
        || program
            .type_decls
            .iter()
            .any(|t| matches!(&t.ty, PascalType::Object(o) if o.is_class));
    if has_classes {
        // A program with a TObject of its own (Free Vision's object type)
        // gets the root class as BrutoTObject.
        let own_tobject = program
            .type_decls
            .iter()
            .any(|t| t.name.eq_ignore_ascii_case("tobject"));
        if own_tobject {
            add_prelude(
                program,
                "objpas.pp",
                &OBJPAS.replace("TObject", "BrutoTObject"),
            )?;
        } else {
            add_prelude(program, "objpas.pp", OBJPAS)?;
        }
    }
    add_prelude(program, "brutostrings.pp", STRINGS)
}

/// Add the types and routines of the prelude unit `source` that the
/// program doesn't declare itself.
fn add_prelude(program: &mut Program, file: &str, source: &str) -> Result<(), String> {
    let path = rtl_dir().join(file);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(source) {
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let _ = std::fs::write(&path, source);
    }
    let unit = Parser::with_options(
        source,
        &ParseOptions {
            file: Some(path),
            include_dirs: Vec::new(),
        },
    )
    .parse_unit()
    .map_err(|e| format!("parse error in the System prelude ({file}): {e}"))?;
    let declared: HashSet<String> = program
        .consts
        .iter()
        .map(|c| c.name.to_ascii_lowercase())
        .chain(
            program
                .type_decls
                .iter()
                .map(|t| t.name.to_ascii_lowercase()),
        )
        .chain(
            program
                .vars
                .iter()
                .flat_map(|v| v.names.iter().map(|n| n.to_ascii_lowercase())),
        )
        .chain(
            program
                .procedures
                .iter()
                .filter(|p| p.owner.is_none())
                .map(|p| p.name.to_ascii_lowercase()),
        )
        .collect();
    let fresh = |name: &str| !declared.contains(&name.to_ascii_lowercase());
    let types = unit
        .interface_types
        .into_iter()
        .chain(unit.impl_types)
        .filter(|t| fresh(&t.name));
    program.type_decls.splice(0..0, types);
    let consts = unit
        .interface_consts
        .into_iter()
        .chain(unit.impl_consts)
        .filter(|c| fresh(&c.name));
    program.consts.splice(0..0, consts);
    let vars = unit
        .interface_vars
        .into_iter()
        .chain(unit.impl_vars)
        .filter(|v| v.names.iter().all(|n| fresh(n)));
    program.vars.splice(0..0, vars);
    let procs = unit.procedures.into_iter().filter(|p| fresh(&p.name));
    program.procedures.splice(0..0, procs);
    Ok(())
}
