//! Process-wide table of source files, so a `Span` can name the file it
//! came from with a small integer.
//!
//! Id 0 is the program being compiled, whatever path the build gives it.
//! Every other file (units and `{$I}` includes) is interned here once and
//! keeps its id for the life of the process.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The file id of the program being compiled.
pub const MAIN_FILE: u32 = 0;

static FILES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// The id for `path`, adding it to the table the first time it is seen.
pub fn intern(path: &Path) -> u32 {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = files.iter().position(|p| *p == path) {
        return i as u32 + 1;
    }
    files.push(path);
    files.len() as u32
}

/// The path of a file id, or `None` for the main program.
pub fn path_of(id: u32) -> Option<PathBuf> {
    if id == MAIN_FILE {
        return None;
    }
    let files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    files.get(id as usize - 1).cloned()
}

/// Read a source file. Text that isn't valid UTF-8 (Free Vision's sources
/// are in code page 437) is read as Latin-1, one char per byte, so stray
/// bytes in comments don't stop the build.
pub fn read_source(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => {
            LATIN1
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(path.to_path_buf());
            e.into_bytes().iter().map(|&b| b as char).collect()
        }
    })
}

/// The source files read as Latin-1 (not UTF-8): their characters
/// 128..=255 are bytes.
static LATIN1: std::sync::Mutex<std::collections::BTreeSet<std::path::PathBuf>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Whether the file at `path` was read as Latin-1.
pub fn is_latin1(path: &Path) -> bool {
    LATIN1
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(path)
}

/// `line L:C`, followed by ` in <file>` when the location is in a unit or
/// an included file rather than the main program.
pub fn describe(line: u32, column: u32, file: u32) -> String {
    match path_of(file) {
        Some(path) => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            format!(
                "line {line}:{column} in {}",
                name.unwrap_or_else(|| path.display().to_string())
            )
        }
        None => format!("line {line}:{column}"),
    }
}
