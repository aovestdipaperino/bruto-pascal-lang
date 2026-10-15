//! Units that ship with the compiler: Bruto's versions of the FPC
//! console and Unix units Free Vision is built on. A program's own files
//! come first on the search path; these are the fallback.
//!
//! They are embedded in the compiler and written out on first use, so
//! errors and the debugger can point at real files.

use crate::ast::Program;
use crate::parser::{ParseOptions, Parser};
use std::collections::HashSet;
use std::path::PathBuf;

const SYSTEM_PRELUDE: &str = include_str!("../rtl/system.pp");

const UNITS: &[(&str, &str)] = &[
    ("unixtype.pp", include_str!("../rtl/unixtype.pp")),
    ("baseunix.pp", include_str!("../rtl/baseunix.pp")),
    ("unix.pp", include_str!("../rtl/unix.pp")),
    ("termio.pp", include_str!("../rtl/termio.pp")),
    ("dos.pp", include_str!("../rtl/dos.pp")),
    ("brutoconsole.pp", include_str!("../rtl/brutoconsole.pp")),
    ("video.pp", include_str!("../rtl/video.pp")),
    ("keyboard.pp", include_str!("../rtl/keyboard.pp")),
    ("mouse.pp", include_str!("../rtl/mouse.pp")),
];

/// The file of built-in unit `name`, if there is one.
pub(crate) fn unit_file(name: &str) -> Option<PathBuf> {
    let file = format!("{}.pp", name.to_ascii_lowercase());
    let (_, source) = UNITS.iter().find(|(f, _)| *f == file)?;
    let dir = std::env::temp_dir().join(format!("bruto-rtl-{}", env!("CARGO_PKG_VERSION")));
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
    let path = std::env::temp_dir()
        .join(format!("bruto-rtl-{}", env!("CARGO_PKG_VERSION")))
        .join("system.pp");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(SYSTEM_PRELUDE) {
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let _ = std::fs::write(&path, SYSTEM_PRELUDE);
    }
    let unit = Parser::with_options(
        SYSTEM_PRELUDE,
        &ParseOptions {
            file: Some(path),
            include_dirs: Vec::new(),
        },
    )
    .parse_unit()
    .map_err(|e| format!("parse error in the System prelude: {e}"))?;
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
    let procs = unit.procedures.into_iter().filter(|p| fresh(&p.name));
    program.procedures.splice(0..0, procs);
    Ok(())
}
