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
        // It bakes in a default output path via __bruto_prof_set_output, the
        // same way codegen does for real profile builds.
        let default_out = dir.join("driver.bruto-prof");
        let driver = dir.join("driver.c");
        std::fs::write(
            &driver,
            format!(
                r#"
#include <stdint.h>
void __bruto_prof_enter(uint32_t); void __bruto_prof_exit(void); void __bruto_prof_line(uint32_t);
void __bruto_prof_set_output(const char *);
int main(void) {{
    __bruto_prof_set_output("{default_out}");
    __bruto_prof_enter(1);
    __bruto_prof_line(2);
    for (int i = 0; i < 3; i++) {{ __bruto_prof_enter(3); __bruto_prof_line(4); __bruto_prof_exit(); }}
    __bruto_prof_exit();
    return 0;
}}
"#,
                default_out = default_out.display()
            ),
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

        let out = default_out.clone();
        let _ = std::fs::remove_file(&out);
        let status = std::process::Command::new(&exe).status().unwrap();
        assert!(status.success());
        assert!(
            out.exists(),
            "the baked-in default output path must be used when BRUTO_PROF_OUT is unset"
        );

        // A second run with BRUTO_PROF_OUT set must write there instead.
        let override_out = dir.join("driver_override.bruto-prof");
        let _ = std::fs::remove_file(&override_out);
        let _ = std::fs::remove_file(&out);
        let status = std::process::Command::new(&exe)
            .env("BRUTO_PROF_OUT", &override_out)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(
            override_out.exists(),
            "BRUTO_PROF_OUT must override the baked-in default path"
        );
        assert!(
            !out.exists(),
            "the default path must not be written when BRUTO_PROF_OUT overrides it"
        );

        // Re-run once more without the env var so the rest of this test
        // exercises fresh data at the default path.
        let status = std::process::Command::new(&exe).status().unwrap();
        assert!(status.success());

        let bytes = std::fs::read(&out).expect("profile written");
        assert_eq!(&bytes[0..4], b"BPRF");
        let count = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        // Nodes: routine 1, line 2, routine 3 (once, 3 calls), line 4.
        assert_eq!(count, 4, "expected one node per (parent, loc) pair");

        #[derive(Debug, Clone, Copy)]
        struct Node {
            kind: u8,
            loc: u32,
            parent: u32,
            calls: u64,
            self_ns: u64,
            total_ns: u64,
        }

        let mut nodes = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            let at = 24 + i * 33;
            let kind = bytes[at];
            let loc =
                u32::from_le_bytes([bytes[at + 1], bytes[at + 2], bytes[at + 3], bytes[at + 4]]);
            let parent =
                u32::from_le_bytes([bytes[at + 5], bytes[at + 6], bytes[at + 7], bytes[at + 8]]);
            let mut c = [0u8; 8];
            c.copy_from_slice(&bytes[at + 9..at + 17]);
            let calls = u64::from_le_bytes(c);
            let mut s = [0u8; 8];
            s.copy_from_slice(&bytes[at + 17..at + 25]);
            let self_ns = u64::from_le_bytes(s);
            let mut t = [0u8; 8];
            t.copy_from_slice(&bytes[at + 25..at + 33]);
            let total_ns = u64::from_le_bytes(t);
            nodes.push(Node {
                kind,
                loc,
                parent,
                calls,
                self_ns,
                total_ns,
            });
        }

        let idx_of = |kind: u8, loc: u32| -> usize {
            nodes
                .iter()
                .position(|n| n.kind == kind && n.loc == loc)
                .unwrap_or_else(|| panic!("node kind={kind} loc={loc} not found in {nodes:?}"))
        };

        let idx_r1 = idx_of(1, 1);
        let idx_l2 = idx_of(2, 2);
        let idx_r3 = idx_of(1, 3);
        let idx_l4 = idx_of(2, 4);

        let r1 = nodes[idx_r1];
        let l2 = nodes[idx_l2];
        let r3 = nodes[idx_r3];
        let l4 = nodes[idx_l4];

        // 1. Routine 1 is the tree root, entered once.
        assert_eq!(
            r1.parent,
            u32::MAX,
            "routine 1 must be a root; nodes = {nodes:?}"
        );
        assert_eq!(r1.calls, 1, "routine 1 calls; nodes = {nodes:?}");

        // 2. Line 2 runs inside routine 1, once.
        assert_eq!(
            l2.parent, idx_r1 as u32,
            "line 2 parent must be routine 1; nodes = {nodes:?}"
        );
        assert_eq!(l2.calls, 1, "line 2 calls; nodes = {nodes:?}");

        // 3. Routine 3 is entered while line 2 is the running line, 3 times.
        assert_eq!(
            r3.parent, idx_l2 as u32,
            "routine 3 parent must be line 2 (the running line at entry); nodes = {nodes:?}"
        );
        assert_eq!(r3.calls, 3, "routine 3 calls; nodes = {nodes:?}");

        // 4. Line 4 runs inside routine 3, once per call.
        assert_eq!(
            l4.parent, idx_r3 as u32,
            "line 4 parent must be routine 3; nodes = {nodes:?}"
        );
        assert_eq!(l4.calls, 3, "line 4 calls; nodes = {nodes:?}");

        // 5. Self/total bookkeeping: self = total - (child routine time), exactly.
        assert_eq!(
            r1.self_ns,
            r1.total_ns - r3.total_ns,
            "routine 1 self_ns must equal total_ns minus routine 3's total_ns; nodes = {nodes:?}"
        );
        assert_eq!(
            l2.self_ns,
            l2.total_ns - r3.total_ns,
            "line 2 self_ns must equal total_ns minus routine 3's total_ns; nodes = {nodes:?}"
        );
        // Routine 3 has no routine children, so self == total exactly (line 4 is
        // a line node, not a routine, and does not subtract from routine 3's self).
        assert_eq!(
            r3.self_ns, r3.total_ns,
            "routine 3 has no callee routines, so self_ns must equal total_ns; nodes = {nodes:?}"
        );
        assert!(
            l4.total_ns <= r3.total_ns,
            "line 4's total_ns must not exceed its enclosing routine 3's total_ns; nodes = {nodes:?}"
        );

        // 6. Sanity: parent total dominates child total, and total >= self everywhere.
        assert!(
            r1.total_ns >= r3.total_ns,
            "routine 1 total_ns must be >= routine 3 total_ns (3 calls nested inside); nodes = {nodes:?}"
        );
        for n in &nodes {
            assert!(
                n.total_ns >= n.self_ns,
                "total_ns must be >= self_ns for every node; node = {n:?}; nodes = {nodes:?}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
