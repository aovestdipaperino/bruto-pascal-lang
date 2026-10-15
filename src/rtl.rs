//! Units that ship with the compiler: Bruto's versions of the FPC
//! console and Unix units Free Vision is built on. A program's own files
//! come first on the search path; these are the fallback.
//!
//! They are embedded in the compiler and written out on first use, so
//! errors and the debugger can point at real files.

use std::path::PathBuf;

const UNITS: &[(&str, &str)] = &[
    ("unixtype.pp", include_str!("../rtl/unixtype.pp")),
    ("baseunix.pp", include_str!("../rtl/baseunix.pp")),
    ("unix.pp", include_str!("../rtl/unix.pp")),
    ("termio.pp", include_str!("../rtl/termio.pp")),
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
