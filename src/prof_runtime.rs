//! The profiler runtime that instrumented programs link against.
//!
//! The C source lives in `prof_runtime.c` and is embedded here so the
//! published crate is self-contained. On a profile build the source is
//! written to the temp dir and compiled with the same `cc` the linker
//! step already requires. Normal builds never touch it.

use std::path::{Path, PathBuf};

pub const RUNTIME_SOURCE: &str = include_str!("prof_runtime.c");

/// Write the embedded C source to `dir/bruto_prof_runtime.c`.
pub fn write_runtime_source(dir: &Path) -> Result<PathBuf, String> {
    let path = dir.join("bruto_prof_runtime.c");
    std::fs::write(&path, RUNTIME_SOURCE).map_err(|e| format!("profiler runtime: write: {e}"))?;
    Ok(path)
}

/// Spawn `cc -c` (or `clang -c` on Windows) turning the runtime source into
/// an object file. The caller polls the child like the linker.
pub fn spawn_runtime_compile(
    c_path: &Path,
    obj_path: &Path,
) -> Result<std::process::Child, String> {
    let cc = if cfg!(target_os = "windows") {
        "clang"
    } else {
        "cc"
    };
    std::process::Command::new(cc)
        .arg("-O2")
        .arg("-c")
        .arg(c_path)
        .arg("-o")
        .arg(obj_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("profiler runtime: failed to spawn {cc}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_compiles_and_records_a_tree() {
        let dir = std::env::temp_dir().join("bruto_prof_runtime_test");
        let _ = std::fs::create_dir_all(&dir);
        let c_path = write_runtime_source(&dir).unwrap();
        let obj = dir.join("rt.o");
        let status = spawn_runtime_compile(&c_path, &obj)
            .unwrap()
            .wait()
            .unwrap();
        assert!(status.success(), "cc -c failed");

        // A tiny driver: main -> enter(1) line(2) enter(3) line(4) exit exit.
        let driver = dir.join("driver.c");
        std::fs::write(
            &driver,
            r#"
#include <stdint.h>
void __bruto_prof_enter(uint32_t); void __bruto_prof_exit(void); void __bruto_prof_line(uint32_t);
int main(void) {
    __bruto_prof_enter(1);
    __bruto_prof_line(2);
    for (int i = 0; i < 3; i++) { __bruto_prof_enter(3); __bruto_prof_line(4); __bruto_prof_exit(); }
    __bruto_prof_exit();
    return 0;
}
"#,
        )
        .unwrap();
        let exe = dir.join("driver");
        let cc = if cfg!(target_os = "windows") {
            "clang"
        } else {
            "cc"
        };
        let status = std::process::Command::new(cc)
            .arg(&driver)
            .arg(&obj)
            .arg("-o")
            .arg(&exe)
            .status()
            .unwrap();
        assert!(status.success(), "link failed");

        let out = dir.join("driver.bruto-prof");
        let _ = std::fs::remove_file(&out);
        let status = std::process::Command::new(&exe)
            .env("BRUTO_PROF_OUT", &out)
            .status()
            .unwrap();
        assert!(status.success());

        let bytes = std::fs::read(&out).expect("profile written");
        assert_eq!(&bytes[0..4], b"BPRF");
        let count = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        // Nodes: routine 1, line 2, routine 3 (once, 3 calls), line 4.
        assert_eq!(count, 4, "expected one node per (parent, loc) pair");
        // Routine 3 must have calls == 3: scan nodes for kind 1 / loc 3.
        let mut found = false;
        for i in 0..count as usize {
            let at = 24 + i * 33;
            let kind = bytes[at];
            let loc =
                u32::from_le_bytes([bytes[at + 1], bytes[at + 2], bytes[at + 3], bytes[at + 4]]);
            if kind == 1 && loc == 3 {
                let mut c = [0u8; 8];
                c.copy_from_slice(&bytes[at + 9..at + 17]);
                assert_eq!(u64::from_le_bytes(c), 3);
                found = true;
            }
        }
        assert!(found, "routine 3 node missing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
