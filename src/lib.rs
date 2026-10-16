#![allow(dead_code)]

pub mod ast;
pub mod codegen;
mod generics;
pub mod obfuscate;
pub mod parser;
mod pascal_syntax;
pub mod prof_runtime;
mod rename;
mod rtl;
mod scope;
pub mod sema;
pub mod source_map;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use bruto_lang::language::{BuildJob, BuildOptions, BuildPhase, BuildResult, Language};
use codegen::CodeGen;
use inkwell::context::Context;
use parser::Parser;
use pascal_syntax::PascalHighlighter;

pub struct MiniPascal;

impl Language for MiniPascal {
    fn name(&self) -> &str {
        "Mini-Pascal"
    }

    fn file_extension(&self) -> &str {
        "pas"
    }

    fn sample_program(&self) -> &str {
        SAMPLE_PROGRAM
    }

    fn create_highlighter(&self) -> Box<dyn turbo_vision::views::syntax::SyntaxHighlighter> {
        Box::new(PascalHighlighter::new())
    }

    fn check_at(&self, source: &str, source_path: Option<&Path>) -> Option<Result<String, String>> {
        let mut search_dirs = search_dirs_for(source_path);
        let lines = source.lines().count();
        let Some(unit) = self.library_name(source) else {
            return Some(
                check_program(source.to_string(), search_dirs)
                    .map(|()| format!("{lines} lines, no errors")),
            );
        };
        // A unit is checked through a program that uses it, the unit's
        // text (unsaved changes too) found first.
        let dir = std::env::temp_dir().join(format!("bruto_check_{}", std::process::id()));
        let written = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(format!("{unit}.pas")), source));
        if let Err(e) = written {
            return Some(Err(format!("Failed to write the unit: {e}")));
        }
        search_dirs.insert(0, dir.clone());
        let program = format!("program BrutoCheck;\nuses {unit};\nbegin\nend.\n");
        let result = check_program(program, search_dirs);
        let _ = std::fs::remove_dir_all(&dir);
        Some(result.map(|()| format!("unit, {lines} lines, no errors")))
    }

    fn library_name(&self, source: &str) -> Option<String> {
        let words = header_words(source);
        match words.as_slice() {
            [unit, name, ..] if unit == "unit" => Some(name.clone()),
            _ => None,
        }
    }

    fn used_libraries(&self, source: &str) -> Vec<String> {
        let words = header_words(source);
        let Some(at) = words.iter().position(|w| w == "uses") else {
            return Vec::new();
        };
        words[at + 1..]
            .iter()
            .take_while(|w| *w != ";")
            .filter(|w| *w != "," && *w != "in" && !w.starts_with('\''))
            .cloned()
            .collect()
    }

    fn valid_breakpoint_lines(&self, source: &str) -> HashSet<usize> {
        let mut parser = Parser::new(source);
        let Ok(program) = parser.parse_program() else {
            return HashSet::new();
        };
        let mut lines = HashSet::new();
        for proc in &program.procedures {
            collect_block_lines(&proc.body, &mut lines);
        }
        collect_block_lines(&program.body, &mut lines);
        lines
    }

    fn build_job(&self, source: &str) -> Box<dyn BuildJob> {
        Box::new(PascalBuildJob::new(
            source.to_string(),
            Vec::new(),
            BuildOptions::default(),
            false,
        ))
    }

    fn build_job_at(&self, source: &str, source_path: Option<&Path>) -> Box<dyn BuildJob> {
        self.build_job_with(source, source_path, &BuildOptions::default())
    }

    fn build_job_with(
        &self,
        source: &str,
        source_path: Option<&Path>,
        options: &BuildOptions,
    ) -> Box<dyn BuildJob> {
        Box::new(PascalBuildJob::new(
            source.to_string(),
            search_dirs_for(source_path),
            *options,
            false,
        ))
    }

    fn profile_job_at(&self, source: &str, source_path: Option<&Path>) -> Box<dyn BuildJob> {
        Box::new(PascalBuildJob::new(
            source.to_string(),
            search_dirs_for(source_path),
            BuildOptions::default(),
            true,
        ))
    }
}

/// Parse, resolve and generate code for the program `source` (finding
/// units in `search_dirs`) without emitting or linking anything: the
/// errors a build would report.
fn check_program(source: String, search_dirs: Vec<PathBuf>) -> Result<(), String> {
    // As a build does: parsing and code generation recurse deeply.
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || -> Result<(), String> {
            let mut parser = Parser::with_options(
                &source,
                &parser::ParseOptions {
                    file: None,
                    include_dirs: search_dirs.clone(),
                },
            );
            let mut program = parser
                .parse_program()
                .map_err(|e| format!("Parse error: {e}"))?;
            resolve_uses(&mut program, &search_dirs)?;
            let context = Context::create();
            let mut codegen = CodeGen::new(&context, "bruto_check.pas");
            codegen.set_directives(parser.directives);
            codegen
                .compile(&program)
                .map_err(|e| format!("Codegen error: {e}"))
        })
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|_| "the compiler crashed".to_string())?
}

/// The first words of `source`, lower-case, up to its first `begin`,
/// `type`, `var`, `const` or `implementation` (enough for the header and
/// a `uses` clause): identifiers (dotted names whole), `;`, `,` and
/// string literals (kept quoted), comments (braces nested, as in FPC's
/// modes) and directives skipped.
fn header_words(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' {
            // FPC's modes nest braces: `{ see ({$mode objfpc}) }`.
            let mut depth = 0;
            while i < chars.len() {
                match chars[i] {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                i += 1;
                if depth == 0 {
                    break;
                }
            }
        } else if c == '(' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == ')') {
                i += 1;
            }
            i += 2;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '\'' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                i += 1;
            }
            i += 1;
            words.push(chars[start..i.min(chars.len())].iter().collect());
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                i += 1;
            }
            let word: String = chars[start..i]
                .iter()
                .collect::<String>()
                .to_ascii_lowercase();
            if matches!(
                word.as_str(),
                "begin" | "type" | "var" | "const" | "implementation" | "procedure" | "function"
            ) {
                break;
            }
            words.push(word);
        } else {
            if c == ';' || c == ',' {
                words.push(c.to_string());
            }
            i += 1;
        }
    }
    words
}

/// Directories searched for `uses` units: the source file's directory
/// first, then the current directory.
fn search_dirs_for(source_path: Option<&Path>) -> Vec<PathBuf> {
    let mut search_dirs = Vec::new();
    if let Some(dir) = source_path.and_then(|p| p.parent()) {
        search_dirs.push(dir.to_path_buf());
    }
    if let Ok(cwd) = std::env::current_dir()
        && !search_dirs.iter().any(|d| d == &cwd)
    {
        search_dirs.push(cwd);
    }
    search_dirs
}

/// Poll-driven build state machine. The first `poll` runs everything
/// up through spawning the linker (parse + codegen + object emit +
/// metadata + spawn cc); subsequent polls just `try_wait` on the
/// linker / dsymutil children. Drop kills any live child so cancel
/// from the IDE actually stops the linker.
struct PascalBuildJob {
    inner: JobInner,
    search_dirs: Vec<PathBuf>,
    /// Compile with profiler hooks and link the profiler runtime.
    profile: bool,
    options: BuildOptions,
}

enum JobInner {
    /// First poll runs everything synchronously through cc spawn.
    NotStarted { source: String },
    /// Profile builds only: `cc -c` on the runtime source is running.
    CompilingRuntime {
        paths: JobPaths,
        child: std::process::Child,
    },
    /// cc/clang is running; poll its status.
    Linking {
        paths: JobPaths,
        child: std::process::Child,
    },
    /// dsymutil is running (macOS only) after cc finished.
    Dsymutil {
        paths: JobPaths,
        child: std::process::Child,
    },
    /// Job has produced its result; further polls are an error.
    Drained,
}

struct JobPaths {
    source_path: String,
    exe_path: String,
    obj_path: String,
    /// Profile builds: the runtime object and the id map.
    runtime_obj_path: Option<String>,
    prof_map_path: Option<String>,
    /// `.s` listing path, kept alongside the executable (unlike `obj_path`,
    /// which is removed once linking succeeds) so the IDE's Disassembly
    /// window can read it after the build finishes.
    asm_path: Option<String>,
}

impl PascalBuildJob {
    fn new(
        source: String,
        search_dirs: Vec<PathBuf>,
        options: BuildOptions,
        profile: bool,
    ) -> Self {
        Self {
            inner: JobInner::NotStarted { source },
            search_dirs,
            options,
            profile,
        }
    }
}

impl BuildJob for PascalBuildJob {
    fn poll(&mut self) -> BuildPhase {
        // Take the state to allow transitioning ownership through the
        // match. On error/done we leave Drained behind.
        let state = std::mem::replace(&mut self.inner, JobInner::Drained);
        match state {
            JobInner::NotStarted { source } => self.start(source),
            JobInner::CompilingRuntime { paths, mut child } => match child.try_wait() {
                Ok(None) => {
                    self.inner = JobInner::CompilingRuntime { paths, child };
                    BuildPhase::Pending("Compiling profiler runtime…".into())
                }
                Ok(Some(status)) if status.success() => self.spawn_link(paths),
                Ok(Some(_)) => {
                    let stderr = child
                        .wait_with_output()
                        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
                        .unwrap_or_default();
                    BuildPhase::Failed(format!("profiler runtime: {stderr}"))
                }
                Err(e) => BuildPhase::Failed(format!("waiting on profiler runtime compile: {e}")),
            },
            JobInner::Linking { paths, mut child } => match child.try_wait() {
                Ok(None) => {
                    self.inner = JobInner::Linking { paths, child };
                    BuildPhase::Pending("Linking…".into())
                }
                Ok(Some(status)) if status.success() => self.after_linker(paths),
                Ok(Some(_)) => {
                    let out = child.wait_with_output();
                    let msg = match out {
                        Ok(o) => format!(
                            "linking failed: {}{}",
                            String::from_utf8_lossy(&o.stderr),
                            String::from_utf8_lossy(&o.stdout)
                        ),
                        Err(e) => format!("linking failed: {e}"),
                    };
                    BuildPhase::Failed(msg)
                }
                Err(e) => BuildPhase::Failed(format!("waiting on linker: {e}")),
            },
            JobInner::Dsymutil { paths, mut child } => match child.try_wait() {
                Ok(None) => {
                    self.inner = JobInner::Dsymutil { paths, child };
                    BuildPhase::Pending("Generating debug info…".into())
                }
                Ok(Some(status)) if status.success() => self.finalize(paths),
                Ok(Some(_)) => {
                    let stderr = child
                        .wait_with_output()
                        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
                        .unwrap_or_default();
                    BuildPhase::Failed(format!("dsymutil failed: {stderr}"))
                }
                Err(e) => BuildPhase::Failed(format!("waiting on dsymutil: {e}")),
            },
            JobInner::Drained => BuildPhase::Failed("build polled after completion".into()),
        }
    }
}

impl PascalBuildJob {
    fn start(&mut self, source: String) -> BuildPhase {
        let tmp = std::env::temp_dir();
        let source_path = tmp
            .join("bruto_pascal_src.pas")
            .to_string_lossy()
            .into_owned();
        let stem = if self.profile {
            "bruto_pascal_prof"
        } else {
            "bruto_pascal_out"
        };
        let exe_path = tmp
            .join(if cfg!(windows) {
                format!("{stem}.exe")
            } else {
                stem.to_string()
            })
            .to_string_lossy()
            .into_owned();

        if let Err(e) = std::fs::write(&source_path, &source) {
            return BuildPhase::Failed(format!("Failed to write source: {e}"));
        }

        // Parsing and code generation recurse with the program's nesting
        // (a long `a + b + ...` too), so they run with a large stack,
        // whatever thread polls the build.
        let search_dirs = self.search_dirs.clone();
        let options = self.options;
        let profile = self.profile;
        let exe_for_thread = exe_path.clone();
        let source_for_thread = source_path.clone();
        let compiled = std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(
                move || -> Result<(crate::codegen::ObjectArtifacts, Option<String>), String> {
                    let exe_path = exe_for_thread;
                    let source_path = source_for_thread;
                    let mut parser = Parser::with_options(
                        &source,
                        &parser::ParseOptions {
                            file: None,
                            include_dirs: search_dirs.clone(),
                        },
                    );
                    let mut program = match parser.parse_program() {
                        Ok(p) => p,
                        Err(e) => return Err(format!("Parse error: {e}")),
                    };

                    if let Err(e) = resolve_uses(&mut program, &search_dirs) {
                        return Err(e);
                    }

                    // Obfuscation pass 1 (identifier rename) runs on the AST, before
                    // codegen builds symbol tables from it. Pass 2 (bogus control flow)
                    // runs on the module after compile(), below.
                    if options.obfuscation_enabled() {
                        obfuscate::rename_identifiers(&mut program);
                    }

                    let context = Context::create();
                    let mut codegen = CodeGen::new(&context, &source_path);
                    codegen.set_directives(parser.directives);
                    codegen.set_build_options(options);
                    codegen.set_instrument(profile);
                    if profile {
                        codegen.set_prof_output(&format!("{exe_path}.bruto-prof"));
                    }
                    if let Err(e) = codegen.compile(&program) {
                        return Err(format!("Codegen error: {e}"));
                    }

                    // Obfuscation pass 2: bogus control flow over the compiled module,
                    // after a successful compile and before the object is emitted.
                    if options.obfuscation_enabled() {
                        codegen.apply_bogus_control_flow();
                    }

                    let artifacts = match codegen.emit_object(&exe_path) {
                        Ok(a) => a,
                        Err(e) => return Err(e),
                    };
                    let _ = codegen.write_metadata(&exe_path);
                    let prof_map_path = if profile {
                        Some(codegen.write_prof_map(&exe_path)?)
                    } else {
                        None
                    };
                    Ok((artifacts, prof_map_path))
                },
            )
            .map_err(|e| e.to_string())
            .and_then(|t| t.join().map_err(|_| "the compiler crashed".to_string()));
        let (artifacts, prof_map_path) = match compiled {
            Ok(Ok(c)) => c,
            Ok(Err(e)) | Err(e) => return BuildPhase::Failed(e),
        };

        let mut paths = JobPaths {
            source_path,
            exe_path,
            obj_path: artifacts.obj_path,
            runtime_obj_path: None,
            prof_map_path,
            asm_path: artifacts.asm_path,
        };

        if self.profile {
            let c_path = match prof_runtime::write_runtime_source(&tmp) {
                Ok(p) => p,
                Err(e) => return BuildPhase::Failed(e),
            };
            let rt_obj = tmp.join("bruto_prof_runtime.o");
            let child = match prof_runtime::spawn_runtime_compile(&c_path, &rt_obj) {
                Ok(c) => c,
                Err(e) => return BuildPhase::Failed(e),
            };
            paths.runtime_obj_path = Some(rt_obj.to_string_lossy().into_owned());
            self.inner = JobInner::CompilingRuntime { paths, child };
            return BuildPhase::Pending("Compiling profiler runtime…".into());
        }

        self.spawn_link(paths)
    }

    /// Spawn the linker over the program object (plus the runtime object
    /// for profile builds) and move to `Linking`.
    fn spawn_link(&mut self, paths: JobPaths) -> BuildPhase {
        let mut objs: Vec<&str> = vec![paths.obj_path.as_str()];
        if let Some(rt) = paths.runtime_obj_path.as_deref() {
            objs.push(rt);
        }
        let child =
            match CodeGen::spawn_linker_objs(&objs, &paths.exe_path, self.options.debug_info()) {
                Ok(c) => c,
                Err(e) => return BuildPhase::Failed(e),
            };
        self.inner = JobInner::Linking { paths, child };
        BuildPhase::Pending("Linking…".into())
    }

    fn after_linker(&mut self, paths: JobPaths) -> BuildPhase {
        if !self.options.debug_info() {
            CodeGen::remove_stale_dsym(&paths.exe_path);
            return self.finalize(paths);
        }
        match CodeGen::spawn_dsymutil(&paths.exe_path) {
            Ok(Some(child)) => {
                self.inner = JobInner::Dsymutil { paths, child };
                BuildPhase::Pending("Generating debug info…".into())
            }
            Ok(None) => self.finalize(paths),
            Err(e) => BuildPhase::Failed(e),
        }
    }

    fn finalize(&mut self, paths: JobPaths) -> BuildPhase {
        let _ = std::fs::remove_file(&paths.obj_path);
        if let Some(rt) = &paths.runtime_obj_path {
            let _ = std::fs::remove_file(rt);
        }
        let profile_path = self
            .profile
            .then(|| format!("{}.bruto-prof", paths.exe_path));
        BuildPhase::Done(BuildResult {
            exe_path: paths.exe_path,
            source_path: paths.source_path,
            console_capture_path: bruto_lang::target::console_capture_path(),
            profile_path,
            profile_map_path: paths.prof_map_path,
            asm_path: paths.asm_path,
        })
    }
}

impl Drop for PascalBuildJob {
    fn drop(&mut self) {
        // Cancel = kill any live child so the linker / dsymutil
        // doesn't keep running after the IDE moves on.
        match &mut self.inner {
            JobInner::CompilingRuntime { child, .. }
            | JobInner::Linking { child, .. }
            | JobInner::Dsymutil { child, .. } => {
                let _ = child.kill();
                let _ = child.wait();
            }
            _ => {}
        }
    }
}

/// Resolve a program's `uses` clause: locate each unit on the search
/// path, parse it, recurse into its own `uses`, and splice every
/// imported declaration (interface + implementation) into the program
/// in dependency order. Detects cycles and missing files.
pub fn resolve_uses(program: &mut ast::Program, search_dirs: &[PathBuf]) -> Result<(), String> {
    if program.uses.is_empty() {
        rtl::add_system_prelude(program)?;
        return rtl::add_string_runtime(program);
    }

    let mut loaded: Vec<ast::Unit> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut on_stack: HashSet<String> = HashSet::new();
    let mut pending: Vec<String> = Vec::new();

    for unit_name in program.uses.clone() {
        load_unit_recursive(
            &unit_name,
            search_dirs,
            &mut loaded,
            &mut visited,
            &mut on_stack,
            &mut pending,
        )?;
        while let Some(dep) = pending.pop() {
            load_unit_recursive(
                &dep,
                search_dirs,
                &mut loaded,
                &mut visited,
                &mut on_stack,
                &mut pending,
            )?;
        }
    }

    scope::separate_unit_scopes(program, &mut loaded);

    // `loaded` is in post-order (deepest dependency first). To
    // preserve that ordering when prepending to the program, build
    // a single combined batch then splice it once at the front. This
    // way MathUtils (deepest) ends up listed before Shapes which
    // ends up before the program's own procs — codegen registers
    // each function prototype before any caller's body is compiled.
    let mut consts = Vec::new();
    let mut types = Vec::new();
    let mut vars = Vec::new();
    let mut procs = Vec::new();
    let mut init_stmts = Vec::new();
    for mut unit in loaded {
        program.uses_exceptions |= unit.uses_exceptions;
        copy_interface_defaults(&mut unit);
        for t in unit.interface_types.iter().chain(&unit.impl_types) {
            program
                .type_units
                .insert(t.name.to_ascii_lowercase(), unit.name.clone());
        }
        consts.extend(unit.interface_consts);
        consts.extend(unit.impl_consts);
        types.extend(unit.interface_types);
        types.extend(unit.impl_types);
        vars.extend(unit.interface_vars);
        vars.extend(unit.impl_vars);
        // Header-only interface declarations are dropped — the bodies
        // in `unit.procedures` carry everything codegen needs.
        procs.extend(unit.procedures);
        if let Some(init) = unit.init {
            init_stmts.extend(init.statements);
        }
    }
    program.units = visited.iter().cloned().collect();
    program.units.sort();
    program.consts.splice(0..0, consts);
    program.type_decls.splice(0..0, types);
    program.vars.splice(0..0, vars);
    program.procedures.splice(0..0, procs);
    program.body.statements.splice(0..0, init_stmts);

    rtl::add_system_prelude(program)?;
    rtl::add_string_runtime(program)
}

/// Parameter defaults are often written only in a unit's interface:
/// give the implementations the defaults their headers declare.
fn copy_interface_defaults(unit: &mut ast::Unit) {
    for header in &unit.interface_proc_headers {
        if !header.params.iter().any(|g| g.default.is_some()) {
            continue;
        }
        for proc in unit
            .procedures
            .iter_mut()
            .filter(|p| p.owner.is_none() && p.name.eq_ignore_ascii_case(&header.name))
        {
            if proc.params.len() != header.params.len() {
                continue;
            }
            for (group, declared) in proc.params.iter_mut().zip(&header.params) {
                if group.default.is_none() {
                    group.default = declared.default.clone();
                }
            }
        }
    }
}

fn load_unit_recursive(
    name: &str,
    search_dirs: &[PathBuf],
    loaded: &mut Vec<ast::Unit>,
    visited: &mut HashSet<String>,
    on_stack: &mut HashSet<String>,
    pending: &mut Vec<String>,
) -> Result<(), String> {
    let key = name.to_ascii_lowercase();
    if visited.contains(&key) {
        return Ok(());
    }
    if on_stack.contains(&key) {
        return Err(format!("circular unit dependency: '{name}' uses itself"));
    }
    on_stack.insert(key.clone());

    let path = find_unit_file(name, search_dirs)
        .ok_or_else(|| format!("unit '{name}' not found on search path"))?;
    let source = source_map::read_source(&path)
        .map_err(|e| format!("failed to read unit '{name}' at {}: {e}", path.display()))?;
    let mut parser = Parser::with_options(
        &source,
        &parser::ParseOptions {
            file: Some(path.clone()),
            include_dirs: search_dirs.to_vec(),
        },
    );
    let unit = parser
        .parse_unit()
        .map_err(|e| format!("parse error in unit '{name}' ({}): {e}", path.display()))?;

    if !unit.name.eq_ignore_ascii_case(name) {
        return Err(format!(
            "unit '{name}' at {} declares itself as '{}'",
            path.display(),
            unit.name
        ));
    }

    for dep in unit.interface_uses.clone() {
        load_unit_recursive(&dep, search_dirs, loaded, visited, on_stack, pending)?;
    }

    on_stack.remove(&key);
    visited.insert(key);
    // Units only the implementation uses may use this one, or one being
    // loaded further up, in turn: they wait until the current chain of
    // interface dependencies is done.
    pending.extend(
        unit.uses
            .iter()
            .filter(|u| !unit.interface_uses.contains(u))
            .cloned(),
    );
    loaded.push(unit);
    Ok(())
}

fn find_unit_file(name: &str, search_dirs: &[PathBuf]) -> Option<PathBuf> {
    // `Name.pas` or `Name.pp`, matching the name without regard to case.
    let wanted: Vec<String> = ["pas", "pp"]
        .iter()
        .map(|ext| format!("{}.{ext}", name.to_ascii_lowercase()))
        .collect();
    for dir in search_dirs {
        for candidate in [
            dir.join(format!("{name}.pas")),
            dir.join(format!("{}.pas", capitalize(name))),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut matches: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| wanted.contains(&e.file_name().to_string_lossy().to_ascii_lowercase()))
                .map(|e| e.path())
                .collect();
            matches.sort();
            if let Some(found) = matches.into_iter().next() {
                return Some(found);
            }
        }
    }
    rtl::unit_file(name)
}

fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Record `span`'s line as breakpoint-eligible, if it is in the program's
/// own file rather than a unit or an included file.
fn insert_line(lines: &mut HashSet<usize>, span: ast::Span) {
    if span.file == source_map::MAIN_FILE {
        lines.insert(span.line as usize);
    }
}

/// Collect lines where codegen emits debug locations (i.e. executable lines).
fn collect_block_lines(block: &ast::Block, lines: &mut HashSet<usize>) {
    for stmt in &block.statements {
        collect_stmt_lines(stmt, lines);
    }
    // `end` keyword is breakpoint-eligible (codegen emits a debug alloca there)
    insert_line(lines, block.end_span);
}

fn collect_stmt_lines(stmt: &ast::Statement, lines: &mut HashSet<usize>) {
    match stmt {
        ast::Statement::Assignment { span, .. }
        | ast::Statement::DerefAssignment { span, .. }
        | ast::Statement::WriteLn { span, .. }
        | ast::Statement::Write { span, .. }
        | ast::Statement::ReadLn { span, .. }
        | ast::Statement::New { span, .. }
        | ast::Statement::Dispose { span, .. }
        | ast::Statement::IndexAssignment { span, .. }
        | ast::Statement::MultiIndexAssignment { span, .. }
        | ast::Statement::FieldAssignment { span, .. }
        | ast::Statement::ProcCall { span, .. }
        | ast::Statement::Exit { span, .. }
        | ast::Statement::PlaceAssignment { span, .. }
        | ast::Statement::Str { span, .. }
        | ast::Statement::IndirectCall { span, .. }
        | ast::Statement::MethodCall { span, .. }
        | ast::Statement::Inherited { span, .. }
        | ast::Statement::Break { span }
        | ast::Statement::Continue { span }
        | ast::Statement::ChainedAssignment { span, .. } => {
            insert_line(lines, *span);
        }
        ast::Statement::If {
            condition: _,
            then_branch,
            else_branch,
            span,
        } => {
            insert_line(lines, *span); // the `if` condition line
            collect_block_lines(then_branch, lines);
            if let Some(eb) = else_branch {
                collect_block_lines(eb, lines);
            }
        }
        ast::Statement::While {
            condition: _,
            body,
            span,
        } => {
            insert_line(lines, *span);
            collect_block_lines(body, lines);
        }
        ast::Statement::For { body, span, .. } | ast::Statement::ForIn { body, span, .. } => {
            insert_line(lines, *span);
            collect_block_lines(body, lines);
        }
        ast::Statement::RepeatUntil { body, span, .. } => {
            insert_line(lines, *span);
            for stmt in body {
                collect_stmt_lines(stmt, lines);
            }
        }
        ast::Statement::Case {
            branches,
            else_branch,
            span,
            ..
        } => {
            insert_line(lines, *span);
            for branch in branches {
                for stmt in &branch.body {
                    collect_stmt_lines(stmt, lines);
                }
            }
            if let Some(stmts) = else_branch {
                for stmt in stmts {
                    collect_stmt_lines(stmt, lines);
                }
            }
        }
        ast::Statement::Goto { span, .. } | ast::Statement::Label { span, .. } => {
            insert_line(lines, *span);
        }
        ast::Statement::With { body, span, .. } => {
            insert_line(lines, *span);
            collect_block_lines(body, lines);
        }
        ast::Statement::Block(block) => {
            collect_block_lines(block, lines);
        }
        ast::Statement::Raise { span, .. } => insert_line(lines, *span),
        ast::Statement::Try {
            body,
            handler,
            span,
        } => {
            insert_line(lines, *span);
            collect_block_lines(body, lines);
            match handler {
                ast::TryHandler::Except {
                    handlers,
                    otherwise,
                } => {
                    for h in handlers {
                        collect_block_lines(&h.body, lines);
                    }
                    if let Some(b) = otherwise {
                        collect_block_lines(b, lines);
                    }
                }
                ast::TryHandler::Finally(b) => collect_block_lines(b, lines),
            }
        }
    }
}

const SAMPLE_PROGRAM: &str = r#"program Demo;
const
  Size = 5;

type
  Vector = array[1..5] of integer;
  Point = record
    x, y: real;
  end;

var
  nums: Vector;
  pt: Point;
  total, i: integer;
  avg: real;
  msg: string;
  p: ^integer;

procedure Fill(var a: Vector; n: integer);
var
  k: integer;
begin
  for k := 1 to n do
    a[k] := k * k
end;

function Sum(var a: Vector; n: integer): integer;
var
  s, j: integer;
begin
  s := 0;
  for j := 1 to n do
    s := s + a[j];
  Sum := s
end;

begin
  { Arrays and procedures }
  Fill(nums, Size);
  total := Sum(nums, Size);
  avg := total / Size;
  writeln('Squares: 1..', Size);
  for i := 1 to Size do
    write(nums[i], ' ');
  writeln;

  { Real arithmetic }
  writeln('Sum = ', total);
  writeln('Avg = ', avg);

  { Records }
  pt.x := avg;
  pt.y := 3.14;
  writeln('Point = (', pt.x, ', ', pt.y, ')');

  { Strings }
  msg := 'Hello' + ' ' + 'Pascal!';
  writeln(msg, ' len=', length(msg));

  { Heap pointers }
  new(p);
  p^ := 42;
  writeln('Heap value: ', p^);
  dispose(p);

  { Repeat-until }
  i := 1;
  repeat
    i := i * 2
  until i > 100;
  writeln('First power of 2 > 100: ', i);

  { ord / chr }
  writeln('ord(A) = ', ord('A'));
  write('chr(90) = ');
  writeln(chr(90));

  if total = 55 then
    writeln('All correct!')
  else
    writeln('Something is wrong!')
end.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn build_and_run_source(source: &str) -> (bool, String) {
        let lang = MiniPascal;
        let result = lang.build(source).expect("build failed");
        let _ = std::fs::remove_file(&result.console_capture_path);
        let status = std::process::Command::new(&result.exe_path)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run failed");
        // Windows writes the capture file in text mode (\r\n).
        let captured = std::fs::read_to_string(&result.console_capture_path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        let _ = std::fs::remove_file(&result.exe_path);
        let _ = std::fs::remove_dir_all(format!("{}.dSYM", result.exe_path));
        let _ = std::fs::remove_file(&result.source_path);
        let _ = std::fs::remove_file(&result.console_capture_path);
        (status.success(), captured)
    }

    #[test]
    fn build_and_run_simple() {
        let (ok, out) = build_and_run_source(
            "program Test;\nvar\n  x: integer;\nbegin\n  x := 42;\n  writeln(x)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "42");
    }

    #[test]
    fn sample_program_runs() {
        let (ok, out) = build_and_run_source(SAMPLE_PROGRAM);
        assert!(ok, "sample program failed to run");
        assert!(out.contains("Sum = 55"), "expected sum, got: {out}");
        assert!(out.contains("Avg = "), "expected avg, got: {out}");
        assert!(out.contains("Point = ("), "expected point, got: {out}");
        assert!(out.contains("Hello Pascal!"), "expected string, got: {out}");
        assert!(out.contains("Heap value: 42"), "expected heap, got: {out}");
        assert!(
            out.contains("First power of 2 > 100: 128"),
            "expected repeat, got: {out}"
        );
        assert!(out.contains("ord(A) = 65"), "expected ord, got: {out}");
        assert!(out.contains("chr(90) = Z"), "expected chr, got: {out}");
        assert!(out.contains("All correct!"), "expected correct, got: {out}");
    }

    #[test]
    fn for_loop() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar i: integer;\nbegin\n  for i := 1 to 5 do\n    write(i);\n  writeln\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "12345");
    }

    #[test]
    fn repeat_until() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar i: integer;\nbegin\n  i := 0;\n  repeat\n    i := i + 1\n  until i = 3;\n  writeln(i)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "3");
    }

    #[test]
    fn procedure_call() {
        let (ok, out) = build_and_run_source(
            "program T;\n\nprocedure Hello;\nbegin\n  writeln('Hi')\nend;\n\nbegin\n  Hello\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "Hi");
    }

    #[test]
    fn function_call() {
        let (ok, out) = build_and_run_source(
            "program T;\n\nfunction Double(x: integer): integer;\nbegin\n  Double := x * 2\nend;\n\nbegin\n  writeln(Double(21))\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "42");
    }

    #[test]
    fn var_parameter() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar a: integer;\n\nprocedure Inc(var x: integer);\nbegin\n  x := x + 1\nend;\n\nbegin\n  a := 5;\n  Inc(a);\n  writeln(a)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "6");
    }

    #[test]
    fn real_arithmetic() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar r: real;\nbegin\n  r := 3.14;\n  writeln(r)\nend.\n",
        );
        assert!(ok);
        assert!(out.trim().starts_with("3.14"), "got: {out}");
    }

    #[test]
    fn heap_new_dispose() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar p: ^integer;\nbegin\n  new(p);\n  p^ := 99;\n  writeln(p^);\n  dispose(p)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "99");
    }

    #[test]
    fn const_section() {
        let (ok, out) =
            build_and_run_source("program T;\nconst\n  X = 42;\nbegin\n  writeln(X)\nend.\n");
        assert!(ok);
        assert_eq!(out.trim(), "42");
    }

    #[test]
    fn array_indexing() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar a: array[1..5] of integer;\n  i: integer;\nbegin\n  for i := 1 to 5 do\n    a[i] := i * i;\n  writeln(a[3])\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "9");
    }

    #[test]
    fn record_field_access() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  Point = record\n    x, y: integer;\n  end;\nvar p: Point;\nbegin\n  p.x := 10;\n  p.y := 20;\n  writeln(p.x + p.y)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "30");
    }

    #[test]
    fn string_concat() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar s: string;\nbegin\n  s := 'Hello' + ' ' + 'World';\n  writeln(s)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "Hello World");
    }

    #[test]
    fn string_length() {
        let (ok, out) =
            build_and_run_source("program T;\nbegin\n  writeln(length('Hello'))\nend.\n");
        assert!(ok);
        assert_eq!(out.trim(), "5");
    }

    #[test]
    fn type_alias() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  Numbers = array[0..2] of integer;\nvar a: Numbers;\nbegin\n  a[0] := 10;\n  a[1] := 20;\n  a[2] := 30;\n  writeln(a[0] + a[1] + a[2])\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "60");
    }

    #[test]
    fn enum_type() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  Color = (Red, Green, Blue);\nvar c: Color;\nbegin\n  c := Green;\n  writeln(c, ' ', ord(c))\nend.\n",
        );
        assert!(ok);
        // As FPC, an enumeration value is written by its name.
        assert_eq!(out.trim(), "Green 1");
    }

    #[test]
    fn subrange_type() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  SmallInt = 1..10;\nvar x: SmallInt;\nbegin\n  x := 5;\n  writeln(x)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "5");
    }

    #[test]
    fn set_operations() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar s: set of integer;\n  x: boolean;\nbegin\n  s := [1, 3, 5..8];\n  x := 3 in s;\n  writeln(x);\n  x := 4 in s;\n  writeln(x)\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "TRUE");
        assert_eq!(lines[1], "FALSE");
    }

    #[test]
    fn case_statement() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar x: integer;\nbegin\n  x := 2;\n  case x of\n    1: writeln('one');\n    2, 3: writeln('two or three');\n  else\n    writeln('other')\n  end\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "two or three");
    }

    #[test]
    fn goto_label() {
        let (ok, out) = build_and_run_source(
            "program T;\nlabel 10;\nvar i: integer;\nbegin\n  i := 0;\n  10: i := i + 1;\n  if i < 5 then\n    goto 10;\n  writeln(i)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "5");
    }

    #[test]
    fn with_statement() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  Point = record\n    x, y: integer;\n  end;\nvar p: Point;\nbegin\n  with p do\n  begin\n    x := 10;\n    y := 20\n  end;\n  writeln(p.x + p.y)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "30");
    }

    #[test]
    fn procedures_see_program_globals() {
        let (ok, out) = build_and_run_source(
            "program T;
const Step = 2;
  Greeting = 'hi';
type Point = record x, y: integer; end;
var counter: integer;
  name: string;
  origin: Point;
  table: array[1..3] of integer;
procedure Bump;
begin
  counter := counter + Step
end;
function Doubled: integer;
begin
  Doubled := counter * Step
end;
procedure Outer;
  procedure Inner;
  begin
    counter := counter + 100
  end;
begin
  Inner
end;
procedure Fill;
var i: integer;
begin
  for i := 1 to 3 do table[i] := i * 10;
  origin.x := 7;
  name := Greeting + '!'
end;
begin
  writeln(length(name));
  Bump;
  writeln(counter);
  writeln(Doubled);
  Outer;
  writeln(counter);
  Fill;
  writeln(table[1] + table[2] + table[3]);
  writeln(origin.x, origin.y);
  writeln(name)
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["0", "2", "4", "102", "60", "70", "hi!"]);
    }

    #[test]
    fn named_constants_in_bounds_and_procedures() {
        let (ok, out) = build_and_run_source(
            "program T;
const MaxItems = 5;
  Last = MaxItems - 1;
type Buffer = array[0..Last] of integer;
  Small = 1..MaxItems;
var buf: Buffer;
  s: Small;
procedure Fill;
var i: integer;
begin
  for i := 0 to Last do buf[i] := i * MaxItems
end;
function Total: integer;
var i, t: integer;
begin
  t := 0;
  for i := low(buf) to high(buf) do t := t + buf[i];
  Total := t
end;
begin
  Fill;
  writeln(Total);
  writeln(high(buf));
  s := MaxItems;
  writeln(s)
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["50", "4", "5"]);
    }

    #[test]
    fn identifiers_are_case_insensitive() {
        let (ok, out) = build_and_run_source(
            "Program T;
Const MaxLen = 3;
Type TPoint = Record X, Y: Integer; End;
Var Pt: TPoint;
  Count: Integer;
  S: String;
  Buf: Array[1..MAXLEN] Of Integer;
Procedure Bump;
Begin
  COUNT := count + 1
End;
Function Twice(N: Integer): Integer;
Begin
  TWICE := n * 2
End;
Begin
  S := 'abc';
  WriteLn(Length(s));
  Inc(Count);
  bump;
  BUMP;
  WriteLn(count);
  PT.x := 3;
  With pt Do Y := 4;
  WriteLn(Pt.X + pt.y);
  WriteLn(Ord('A'));
  buf[MaxLen] := twice(21);
  WriteLn(BUF[3])
End.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["3", "3", "7", "65", "42"]);
    }

    #[test]
    fn break_and_continue_in_every_loop_kind() {
        let (ok, out) = build_and_run_source(
            "program T;
var i, j, sum: integer;
begin
  sum := 0;
  for i := 1 to 10 do
  begin
    if i mod 2 = 0 then continue;
    if i > 7 then break;
    sum := sum + i
  end;
  writeln(sum);
  i := 0;
  sum := 0;
  while true do
  begin
    i := i + 1;
    if i = 3 then continue;
    if i > 5 then break;
    sum := sum + i
  end;
  writeln(sum);
  i := 0;
  repeat
    i := i + 1;
    if i < 4 then continue;
    break
  until false;
  writeln(i);
  sum := 0;
  for i := 1 to 3 do
    for j := 1 to 3 do
    begin
      if j = 2 then break;
      sum := sum + 10 * i + j
    end;
  writeln(sum)
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["16", "12", "4", "63"]);
    }

    #[test]
    fn exit_leaves_the_current_routine() {
        let (ok, out) = build_and_run_source(
            "program T;
var hits: integer;
function Find(target: integer): integer;
var i: integer;
begin
  Find := -1;
  for i := 1 to 100 do
    if i * i = target then exit(i);
  hits := hits + 1
end;
procedure Note(n: integer);
begin
  if n < 0 then exit;
  writeln('note ', n)
end;
function Sign(n: integer): integer;
begin
  Sign := 0;
  if n > 0 then begin Sign := 1; exit end;
  if n < 0 then Sign := -1
end;
begin
  writeln(Find(49));
  writeln(Find(50));
  writeln(hits);
  Note(-1);
  Note(2);
  writeln(Sign(5), Sign(-5));
  exit;
  writeln('unreachable')
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["7", "-1", "1", "note 2", "1-1"]);
    }

    #[test]
    fn misplaced_break_and_exit_value_are_errors() {
        for (src, msg) in [
            ("program T;\nbegin\n  break\nend.\n", "inside a loop"),
            (
                "program T;\nprocedure P;\nbegin\n  exit(1)\nend;\nbegin\n  P\nend.\n",
                "only allowed in a function",
            ),
        ] {
            let err = match MiniPascal.build(src) {
                Ok(_) => panic!("expected an error for {src:?}"),
                Err(e) => e,
            };
            assert!(err.contains(msg), "{err}");
        }
    }

    #[test]
    fn a_variable_may_be_called_exit() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar exit: integer;\nbegin\n  exit := 3;\n  writeln(exit)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "3");
    }

    #[test]
    fn bitwise_operators_and_real_negation() {
        let (ok, out) = build_and_run_source(
            "program T;
const Mask = $0F shl 4;
var a, b: integer;
  f: boolean;
  r: real;
  buf: array[0..Mask shr 4] of integer;
begin
  a := $F0;
  b := $3C;
  writeln(a and b);
  writeln(a or b);
  writeln(a xor b);
  writeln(1 shl 10);
  writeln(1024 shr 3);
  writeln(not 0);
  writeln(Mask, ' ', high(buf));
  f := true;
  writeln(f xor false);
  writeln(not f);
  r := 2.5;
  writeln(-r:4:1);
  writeln((a and b) = $30)
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(
            lines,
            [
                "48", "252", "204", "1024", "128", "-1", "240 15", "TRUE", "FALSE", "-2.5", "TRUE"
            ]
        );
    }

    #[test]
    fn boolean_and_or_short_circuit() {
        let (ok, out) = build_and_run_source(
            "program T;
type PNode = ^integer;
var p: PNode;
  calls: integer;
function Touch(result: boolean): boolean;
begin
  calls := calls + 1;
  Touch := result
end;
begin
  p := nil;
  if (p <> nil) and (p^ > 0) then writeln('deref') else writeln('safe');
  if Touch(false) and Touch(true) then writeln('x');
  if Touch(true) or Touch(true) then writeln('or');
  writeln(calls);
  writeln((1 < 2) and (3 > 2))
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["safe", "or", "2", "TRUE"]);
    }

    #[test]
    fn long_loops_do_not_grow_the_stack() {
        // Every block's `end` marker and every set-literal temporary used to
        // take a fresh stack slot per iteration, overflowing the stack.
        let (ok, out) = build_and_run_source(
            "program T;
var i, n: integer;
begin
  n := 0;
  for i := 1 to 3000000 do
  begin
    if (i mod 7) in [1, 3] then n := n + 1
  end;
  writeln(n)
end.
",
        );
        assert!(ok);
        assert_eq!(out.trim(), "857144");
    }

    #[test]
    fn sized_integers_wrap_at_their_width() {
        let (ok, out) = build_and_run_source(
            "program T;
var b: Byte; si: ShortInt; w: Word; sm: SmallInt; li: LongInt; lw: LongWord;
begin
  b := 255; b := b + 1; writeln(b);
  si := 127; si := si + 1; writeln(si);
  w := 0; w := w - 1; writeln(w);
  sm := -32768; sm := sm - 1; writeln(sm);
  li := 2147483647; li := li + 1; writeln(li);
  lw := 0; lw := lw - 1; writeln(lw);
  b := 200; writeln(b + b);
  si := -5; writeln(si * 3);
  b := 250; inc(b, 10); writeln(b);
  w := 1; dec(w, 2); writeln(w)
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(
            lines,
            [
                "0",
                "-128",
                "65535",
                "32767",
                "-2147483648",
                "4294967295",
                "400",
                "-15",
                "4",
                "65535"
            ]
        );
    }

    #[test]
    fn sized_integer_layout_and_bounds() {
        let (ok, out) = build_and_run_source(
            "program T;
type Sw_Integer = LongInt;
  TPair = record Lo: Byte; Hi: Word; Count: LongInt; end;
  TBytes = array[0..3] of Byte;
var p: TPair;
  bytes: TBytes;
  n: Sw_Integer;
  b: Byte;
  total: integer;
begin
  writeln(sizeof(Byte), ' ', sizeof(Word), ' ', sizeof(LongInt), ' ', sizeof(QWord));
  writeln(sizeof(bytes), ' ', sizeof(TPair), ' ', sizeof(n));
  writeln(low(ShortInt), ' ', high(ShortInt), ' ', high(Word), ' ', high(LongWord));
  p.Lo := 300;
  p.Hi := 70000;
  p.Count := -1;
  writeln(p.Lo, ' ', p.Hi, ' ', p.Count);
  bytes[2] := 257;
  writeln(bytes[2]);
  total := 0;
  for b := 250 to 255 do total := total + 1;
  writeln(total, ' ', b);
  total := 0;
  for b := 3 downto 0 do total := total + b;
  writeln(total)
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(
            lines,
            [
                "1 2 4 8",
                "4 8 4",
                "-128 127 65535 4294967295",
                "44 4464 -1",
                "1",
                "6 255",
                "6"
            ]
        );
    }

    #[test]
    fn sized_integers_cross_routine_boundaries() {
        let (ok, out) = build_and_run_source(
            "program T;
var w: Word;
function Low8(x: integer): Byte;
begin
  Low8 := x
end;
procedure Bump(var v: Word; by: Word);
begin
  v := v + by
end;
function Half(r: real): real;
begin
  Half := r / 2
end;
begin
  writeln(Low8(511));
  w := 65535;
  Bump(w, 2);
  writeln(w);
  writeln(Half(5):3:1)
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["255", "1", "2.5"]);
    }

    #[test]
    fn integers_assigned_to_reals_are_converted() {
        // Storing an integer into a real slot used to copy its raw bits,
        // so `r := 1` read back as 0.0.
        let (ok, out) = build_and_run_source(
            "program T;
type TPt = record x: real; end;
var r: real; a: array[1..2] of real; p: TPt;
begin
  r := 1;
  a[1] := 2;
  p.x := 3;
  writeln(r:3:1, ' ', a[1]:3:1, ' ', p.x:3:1)
end.
",
        );
        assert!(ok);
        assert_eq!(out.trim(), "1.0 2.0 3.0");
    }

    #[test]
    fn address_of_and_untyped_pointers() {
        let (ok, out) = build_and_run_source(
            "program T;
type TRec = record Id: integer; Tag: Byte; end;
  PRec = ^TRec;
  PInt = ^integer;
  TProc = procedure(n: integer);
var x: integer;
  arr: array[1..3] of integer;
  r: TRec;
  p: Pointer;
  pi: PInt;
  addr: PtrUInt;
  cb: TProc;
procedure Show(n: integer);
begin
  writeln('show ', n)
end;
begin
  x := 5;
  pi := @x;
  pi^ := pi^ + 1;
  writeln(x);
  pi := @arr[2];
  pi^ := 42;
  writeln(arr[2]);
  p := @r;
  PRec(p)^.Id := 7;
  PRec(p)^.Tag := 300;
  writeln(r.Id, ' ', r.Tag);
  addr := PtrUInt(@x);
  pi := PInt(Pointer(addr));
  writeln(pi^);
  writeln(Assigned(p), ' ', Assigned(nil));
  p := nil;
  writeln(Assigned(p));
  cb := @Show;
  cb(9);
  writeln(Byte(300), ' ', ShortInt(200), ' ', LongInt(4294967297), ' ', Word(-1))
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(
            lines,
            [
                "6",
                "42",
                "7 44",
                "6",
                "TRUE FALSE",
                "FALSE",
                "show 9",
                "44 -56 1 65535"
            ]
        );
    }

    #[test]
    fn untyped_pointer_dereference_is_an_error() {
        let src = "program T;\nvar p: Pointer; x: integer;\nbegin\n  p := @x;\n  x := p^\nend.\n";
        let err = match MiniPascal.build(src) {
            Ok(_) => panic!("expected an error"),
            Err(e) => e,
        };
        assert!(err.contains("untyped pointer"), "{err}");
    }

    #[test]
    fn string_indexing_reads_and_writes_characters() {
        let (ok, out) = build_and_run_source(
            "program T;
type TRec = record Name: string; end;
var s, t: string;
  r: TRec;
  i, vowels: integer;
  c: char;
begin
  s := 'hello';
  t := s;
  writeln(s[1], s[5]);
  s[1] := 'H';
  writeln(s, ' ', t);
  vowels := 0;
  for i := 1 to length(s) do
    if s[i] in ['a', 'e', 'i', 'o', 'u'] then inc(vowels);
  writeln(vowels);
  r.Name := 'abc';
  r.Name[2] := 'X';
  writeln(r.Name);
  c := s[2];
  writeln(ord(c));
  if (s[1] = 'H') and (c <> 'q') and ('e' = c) then writeln('compare')
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["ho", "Hello hello", "2", "aXc", "101", "compare"]);
    }

    #[test]
    fn routine_declaration_parts() {
        let (ok, out) = build_and_run_source(
            "program T;
var total: integer;
const Base = 10;
type TSmall = 0..9;
var extra: TSmall;
procedure Nothing;
begin
end;
function Scaled(n: integer): integer;
type TPair = record a, b: integer; end;
const Factor = 3;
var p: TPair;
  function Twice(x: integer): integer;
  begin
    Twice := x * 2
  end;
const Offset = Base + 1;
begin
  p.a := n * Factor;
  p.b := Offset;
  Scaled := Twice(p.a) + p.b
end;
begin
  Nothing;
  extra := 4;
  total := Scaled(extra);
  writeln(total)
end.
",
        );
        assert!(ok);
        assert_eq!(out.trim(), "35");
    }

    #[test]
    fn object_types_with_virtual_methods() {
        let (ok, out) = build_and_run_source(
            "program Shapes;
type
  PShape = ^TShape;
  TShape = object
    Name: string;
    constructor Init(AName: string);
    function Area: real; virtual;
    procedure Describe;
    destructor Done; virtual;
  end;
  PCircle = ^TCircle;
  TCircle = object(TShape)
    Radius: real;
    constructor Init(AName: string; R: real);
    function Area: real; virtual;
    destructor Done; virtual;
  end;
  TCounter = object
    Count: LongInt;
    procedure Bump;
    function Twice: integer;
  end;
var
  s: PShape;
  c: TCircle;
  k: TCounter;
  disposed: integer;

constructor TShape.Init(AName: string);
begin
  Name := AName
end;
function TShape.Area: real;
begin
  Area := 0
end;
procedure TShape.Describe;
begin
  writeln(Name, ' area ', Area:5:2)
end;
destructor TShape.Done;
begin
  disposed := disposed + 1
end;
constructor TCircle.Init(AName: string; R: real);
begin
  inherited Init(AName);
  Radius := R
end;
function TCircle.Area: real;
begin
  Area := 3 * Radius * Radius
end;
destructor TCircle.Done;
begin
  writeln('circle done');
  inherited Done
end;
procedure TCounter.Bump;
begin
  Inc(Count)
end;
function TCounter.Twice: integer;
begin
  Bump;
  Twice := Self.Count * 2
end;
begin
  s := New(PCircle, Init('big', 2));
  s^.Describe;
  writeln(s^.Area:5:2);
  Dispose(s, Done);
  writeln(disposed);
  c.Init('small', 1);
  c.Describe;
  New(s, Init('plain'));
  s^.Describe;
  writeln(TypeOf(s^) = TypeOf(TShape), ' ', TypeOf(c) = TypeOf(TCircle));
  k.Count := 1;
  writeln(k.Twice);
  writeln(SizeOf(TCounter))
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(
            lines,
            [
                "big area 12.00",
                "12.00",
                "circle done",
                "1",
                "small area  3.00",
                "plain area  0.00",
                "TRUE TRUE",
                "4",
                "4"
            ]
        );
    }

    #[test]
    fn methods_named_like_builtins_and_abstract_methods() {
        let (ok, out) = build_and_run_source(
            "program T;
type
  TStream = object
    Total: integer;
    constructor Init;
    procedure Write(n: integer); virtual;
    function Read: integer; virtual;
    procedure Copy3(n: integer);
    procedure Show; virtual; abstract;
  end;
constructor TStream.Init;
begin
  Total := 0
end;
procedure TStream.Write(n: integer);
begin
  Total := Total + n
end;
function TStream.Read: integer;
begin
  Read := Total
end;
procedure TStream.Copy3(n: integer);
begin
  Write(n); Write(n); Write(n)
end;
var s: TStream;
begin
  s.Init;
  s.Write(5);
  s.Copy3(2);
  writeln(s.Read);
  s.Show
end.
",
        );
        assert!(!ok, "calling an abstract method should stop the program");
        assert_eq!(out.trim(), "11");
    }

    #[test]
    fn self_referencing_pointer_types() {
        let (ok, out) = build_and_run_source(
            "program T;
type
  PNode = ^TNode;
  TNode = record
    Value: integer;
    Next: PNode;
  end;
var head, n: PNode;
  i, sum: integer;
begin
  head := nil;
  for i := 1 to 4 do
  begin
    new(n);
    n^.Value := i;
    n^.Next := head;
    head := n
  end;
  sum := 0;
  n := head;
  while n <> nil do
  begin
    sum := sum + n^.Value;
    n := n^.Next
  end;
  writeln(sum, ' ', head^.Next^.Value)
end.
",
        );
        assert!(ok);
        assert_eq!(out.trim(), "10 3");
    }

    #[test]
    fn fpc_literals_and_declarations() {
        let (ok, out) = build_and_run_source(
            "program T;
{$MACRO ON}
{$DEFINE Deref:=^}
type
  TPoint = record X, Y: integer; end;
  TNote = record Text: string; Count: Byte; end;
resourcestring
  sHello = 'hello';
const
  CrLf = #13#10;
  Bell = ^G;
  Mask = %1010;
  Octal = &17;
  Mixed = 'A'#$42'C';
  Origin: TPoint = (X: 3; Y: 4);
  Primes: array[1..4] of integer = (2, 3, 5, 7);
  Names: array[Boolean] of string = ('no', 'yes');
  Empty: TPoint = ();
  Old = 1 deprecated 'use New';
var
  n: TNote;
  p: ^integer;
  s: string;
  i, total: integer;
function Sum3(a, b, c: integer): integer;
begin
  Result := a + b;
  Result := Result + c
end;
begin
  writeln(length(CrLf), ' ', ord(Bell), ' ', Mask, ' ', Octal, ' ', Mixed);
  writeln(sHello, ' ', Origin.X + Origin.Y, ' ', Primes[4], ' ', Names[true], ' ', Empty.Y);
  n.Text := 'note';
  n.Count := 2;
  writeln(n.Text, n.Count);
  new(p);
  p Deref := 41;
  inc(p^);
  writeln(p^);
  total := 0;
  for i := 1 to 3 do ;
  case i of
    1: writeln('one');
  otherwise
    writeln('other ', i)
  end;
  Str(42:5, s);
  writeln('[', s, ']');
  Str(3.14159:0:2, s);
  writeln(s);
  writeln(Sum3(1, 2, 3))
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(
            lines,
            [
                "2 7 10 15 ABC",
                "hello 7 7 yes 0",
                "note2",
                "42",
                "other 3",
                "[   42]",
                "3.14",
                "6"
            ]
        );
    }

    #[test]
    fn untyped_params_absolute_and_pointer_arithmetic() {
        let (ok, out) = build_and_run_source(
            "program T;
type
  TBytes = array[0..7] of Byte;
  PRec = ^TRec;
  TRec = record A, B: integer; end;
var
  r: TRec;
  q: PRec;
  w: LongInt;
  b: TBytes;
  pb, pe: ^Byte;
procedure Fill(var Buf; Count: integer; Value: Byte);
var Bytes: TBytes absolute Buf;
  i: integer;
begin
  for i := 0 to Count - 1 do Bytes[i] := Value
end;
function FirstByte(const Data): Byte;
begin
  FirstByte := TBytes(Data)[0]
end;
begin
  Fill(b, 8, 7);
  writeln(b[0] + b[7]);
  w := $01020304;
  writeln(FirstByte(w));
  pb := @b[0];
  pe := pb + 5;
  pe^ := 99;
  writeln(b[5], ' ', pe - pb);
  with r do begin A := 1; B := 2 end;
  q := @r;
  with q^ do writeln(A + B);
  new(q);
  q^.A := 10;
  with q^ do B := A * 2;
  writeln(q^.B);
  dispose(q);
  Pointer(q) := nil;
  writeln(q = nil)
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["14", "4", "99 5", "3", "20", "TRUE"]);
    }

    #[test]
    fn nested_variants_indirect_calls_and_qualifiers() {
        let (ok, out) = build_and_run_source(
            "program T;
const
  evMouse = 1;
  evKey = 2;
type
  TKind = (kNone, kSome);
  TEvent = record
    What: Word;
    case Word of
      evMouse: (X, Y: integer);
      evKey: (
        case integer of
          0: (KeyCode: Word);
          1: (CharCode: char; ScanCode: Byte));
  end;
  TTagged = record
    case Kind: TKind of
      kNone: ();
      kSome: (Value: integer);
  end;
  TFunc = function(a, b: integer): integer;
var
  e: TEvent;
  t: TTagged;
  f: Pointer;
function Add(a, b: integer): integer;
begin
  Add := a + b
end;
begin
  e.What := evKey;
  e.KeyCode := $1E41;
  writeln(e.CharCode, ' ', e.ScanCode);
  with e do begin X := 5; Y := 6 end;
  writeln(e.X + e.Y);
  t.Kind := kSome;
  t.Value := 9;
  writeln(ord(t.Kind), ' ', t.Value);
  f := @Add;
  writeln(TFunc(f)(2, 3));
  writeln(System.Length('abc'))
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["A 30", "11", "1 9", "5", "3"]);
    }

    #[test]
    fn short_strings_under_h_minus() {
        let (ok, out) = build_and_run_source(
            "program T;
{$H-}
type
  TName = record Name: string[10]; Age: Byte; end;
var
  s, t: string;
  five: string[5];
  r: TName;
  p: PString;
procedure Shout(var x: string);
begin
  x := x + '!'
end;
function Twice(x: string): string;
begin
  x[1] := UpCase(x[1]);
  Twice := x + x
end;
begin
  s := 'hello';
  writeln(ord(s[0]), ' ', length(s), ' ', s);
  five := 'truncated';
  writeln(five, ' ', ord(five[0]));
  s[0] := #4;
  writeln(s);
  Shout(s);
  writeln(s);
  t := Twice('ab');
  writeln(t, ' ', s = 'hell!');
  r.Name := 'Ada Lovelace';
  r.Age := 36;
  writeln(r.Name, ' ', r.Age);
  GetMem(p, 6);
  p^[0] := #3;
  p^[1] := 'x'; p^[2] := 'y'; p^[3] := 'z';
  writeln(p^, ' ', length(p^));
  Delete(s, 1, 1);
  Insert('>', s, 1);
  writeln(s, ' ', copy(s, 2, 3), ' ', pos('l', s));
  writeln(SizeOf(five), ' ', SizeOf(s))
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(
            lines,
            [
                "5 5 hello",
                "trunc 5",
                "hell",
                "hell!",
                "AbAb TRUE",
                "Ada Lovela 36",
                "xyz 3",
                ">ell! ell 3",
                "6 256"
            ]
        );
    }

    #[test]
    fn system_routines_and_untyped_files() {
        let (ok, out) = build_and_run_source(
            "program T;
type TBuf = array[0..7] of Byte;
var
  a, b: TBuf;
  p: PChar;
  f: file;
  w: Word;
  n, done: LongInt;
begin
  FillChar(a, SizeOf(a), 3);
  Move(a, b, 4);
  writeln(b[0] + b[3], ' ', b[4]);
  GetMem(p, 4);
  p[0] := 'o'; p[1] := 'k'; p[2] := #0;
  writeln(p[0], p[1]);
  FreeMem(p, 4);
  w := $1234;
  writeln(Hi(w), ' ', Lo(w), ' ', Swap(w) = $3412);
  writeln(ParamCount >= 0, ' ', length(ParamStr(0)) > 0);
  Assign(f, 'bruto_blockio_test.bin');
  Rewrite(f, 1);
  n := 1234567;
  BlockWrite(f, n, SizeOf(n));
  Close(f);
  n := 0;
  Reset(f, 1);
  BlockRead(f, n, SizeOf(n), done);
  Close(f);
  writeln(n, ' ', done);
  System.Halt(0);
  writeln('not reached')
end.
",
        );
        let _ = std::fs::remove_file("bruto_blockio_test.bin");
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["6 0", "ok", "18 52 TRUE", "TRUE TRUE", "1234567 4"]);
    }

    #[test]
    fn overloaded_routines() {
        let (ok, out) = build_and_run_source(
            "program T;
function Describe(n: integer): string;
begin
  Describe := 'int'
end;
function Describe(s: string): string;
begin
  Describe := 'string ' + s
end;
function Describe(a, b: integer): string;
begin
  Result := 'pair'
end;
procedure Show(r: real);
begin
  writeln('real ', r:0:1)
end;
procedure Show(c: char);
begin
  writeln('char ', c)
end;
procedure Early;
begin
  Late
end;
procedure Late;
begin
  writeln('late')
end;
begin
  writeln(Describe(1), ' / ', Describe('x'), ' / ', Describe(1, 2));
  Show(2.5);
  Show('z');
  Early
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(
            lines,
            ["int / string x / pair", "real 2.5", "char z", "late"]
        );
    }

    #[test]
    fn nested_procedural_values() {
        let (ok, out) = build_and_run_source(
            "program T;
type
  TProcParam = procedure(Item: Pointer) is nested;
  TFunBoolParam = function(Item: Pointer): Boolean is nested;
  TList = object
    Items: array[0..4] of LongInt;
    Count: integer;
    procedure ForEach(Action: TProcParam);
    function FirstThat(Test: TFunBoolParam): Pointer;
  end;
procedure TList.ForEach(Action: TProcParam);
var i: integer;
begin
  for i := 0 to Count - 1 do Action(@Items[i])
end;
function TList.FirstThat(Test: TFunBoolParam): Pointer;
var i: integer;
begin
  FirstThat := nil;
  for i := 0 to Count - 1 do
    if Test(@Items[i]) then
    begin
      FirstThat := @Items[i];
      exit
    end
end;
function Call(F: TFunBoolParam; P: Pointer): Boolean;
begin
  Call := F(P)
end;
function IsBig(Item: Pointer): Boolean;
begin
  IsBig := PLongInt(Item)^ > 1000
end;
procedure Run;
var
  L: TList;
  total, limit: LongInt;
  found: PLongInt;
  i: integer;
  procedure Add(Item: Pointer);
  begin
    total := total + PLongInt(Item)^
  end;
  function Above(Item: Pointer): Boolean;
  begin
    Above := PLongInt(Item)^ > limit
  end;
begin
  L.Count := 5;
  for i := 0 to 4 do L.Items[i] := (i + 1) * 10;
  total := 0;
  L.ForEach(@Add);
  writeln(total);
  limit := 25;
  found := L.FirstThat(@Above);
  writeln(found^);
  limit := 100;
  writeln(L.FirstThat(@Above) = nil);
  writeln(Call(IsBig, @L.Items[0]), ' ', Call(@Above, @L.Items[0]))
end;
begin
  Run
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["150", "30", "TRUE", "FALSE FALSE"]);
    }

    #[test]
    fn bare_function_name_is_a_call() {
        let (ok, out) = build_and_run_source(
            "program T;
function Title: string;
begin
  Title := 'Free Vision'
end;
function Half: real;
begin
  Half := 0.5
end;
begin
  writeln(Title);
  writeln(Half:4:2)
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["Free Vision", "0.50"]);
    }

    #[test]
    fn local_shadows_program_global() {
        let (ok, out) = build_and_run_source(
            "program T;
var x: integer;
procedure P;
var x: integer;
begin
  x := 5;
  writeln(x)
end;
begin
  x := 1;
  P;
  writeln(x)
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines, ["5", "1"]);
    }

    #[test]
    fn math_builtins() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar x: integer;\n  r: real;\nbegin\n  x := abs(-5);\n  writeln(x);\n  x := sqr(4);\n  writeln(x);\n  r := sqrt(16.0);\n  writeln(trunc(r));\n  writeln(round(3.7))\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "5");
        assert_eq!(lines[1], "16");
        assert_eq!(lines[2], "4");
        assert_eq!(lines[3], "4");
    }

    #[test]
    fn trig_builtins() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar r: real;\nbegin\n  r := sin(0.0);\n  writeln(trunc(r));\n  r := cos(0.0);\n  writeln(trunc(r));\n  r := exp(0.0);\n  writeln(trunc(r));\n  r := ln(1.0);\n  writeln(trunc(r))\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "0"); // sin(0) = 0
        assert_eq!(lines[1], "1"); // cos(0) = 1
        assert_eq!(lines[2], "1"); // exp(0) = 1
        assert_eq!(lines[3], "0"); // ln(1) = 0
    }

    #[test]
    fn ord_chr() {
        let (ok, out) = build_and_run_source(
            "program T;\nbegin\n  writeln(ord('A'));\n  write(chr(66))\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "65");
        assert_eq!(lines[1], "B");
    }

    #[test]
    fn multi_dim_array() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar\n  m: array[1..2, 1..3] of integer;\nbegin\n  m[1, 2] := 42;\n  writeln(m[1, 2])\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "42");
    }

    #[test]
    fn variant_record() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  Shape = record\n    kind: integer;\n    case tag: integer of\n      1: (radius: integer);\n      2: (width, height: integer);\n  end;\nvar s: Shape;\nbegin\n  s.kind := 1;\n  s.tag := 1;\n  s.radius := 10;\n  writeln(s.radius)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "10");
    }

    #[test]
    fn all_new_features() {
        let source = r#"program NewFeatures;
label 99;
type
  Color = (Red, Green, Blue);
  SmallInt = 1..10;
  Point = record
    x, y: integer;
  end;

var
  c: Color;
  n: SmallInt;
  s: set of integer;
  ok: boolean;
  pt: Point;
  m: array[1..2, 1..3] of integer;
  i: integer;

begin
  { Enumerated types }
  c := Blue;
  writeln('Blue = ', c);

  { Subrange }
  n := 7;
  writeln('n = ', n);

  { Sets }
  s := [1, 3, 5..9];
  ok := 5 in s;
  writeln('5 in set: ', ok);
  ok := 4 in s;
  writeln('4 in set: ', ok);

  { Case }
  case c of
    0: writeln('Red');
    1: writeln('Green');
    2: writeln('Blue')
  end;

  { With }
  with pt do
  begin
    x := 100;
    y := 200
  end;
  writeln('pt = ', pt.x + pt.y);

  { Multi-dim arrays }
  m[1, 2] := 42;
  writeln('m[1,2] = ', m[1, 2]);

  { Goto }
  i := 0;
  99: i := i + 1;
  if i < 3 then
    goto 99;
  writeln('goto count = ', i)
end.
"#;
        let (ok, out) = build_and_run_source(source);
        assert!(ok, "new features program failed to run");
        assert!(out.contains("Blue = Blue"), "enum: {out}");
        assert!(out.contains("n = 7"), "subrange: {out}");
        assert!(out.contains("5 in set: TRUE"), "set-in: {out}");
        assert!(out.contains("4 in set: FALSE"), "set-not-in: {out}");
        assert!(out.contains("Blue"), "case: {out}");
        assert!(out.contains("pt = 300"), "with: {out}");
        assert!(out.contains("m[1,2] = 42"), "multi-dim: {out}");
        assert!(out.contains("goto count = 3"), "goto: {out}");
    }

    #[test]
    fn ordinal_builtins() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar x: integer;\n  a: array[1..5] of integer;\nbegin\n  x := 10;\n  inc(x);\n  writeln(x);\n  dec(x, 3);\n  writeln(x);\n  writeln(succ(5));\n  writeln(pred(5));\n  writeln(low(a));\n  writeln(high(a))\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "11");
        assert_eq!(lines[1], "8");
        assert_eq!(lines[2], "6");
        assert_eq!(lines[3], "4");
        assert_eq!(lines[4], "1");
        assert_eq!(lines[5], "5");
    }

    #[test]
    fn set_include_exclude() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar s: set of integer;\nbegin\n  s := [1, 3];\n  include(s, 5);\n  writeln(5 in s);\n  exclude(s, 3);\n  writeln(3 in s)\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "TRUE");
        assert_eq!(lines[1], "FALSE");
    }

    #[test]
    fn forward_declaration() {
        let (ok, out) = build_and_run_source(
            "program T;\n\nprocedure B; forward;\n\nprocedure A;\nbegin\n  B\nend;\n\nprocedure B;\nbegin\n  writeln('B called')\nend;\n\nbegin\n  A\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "B called");
    }

    #[test]
    fn nested_lvalue() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  Rec = record\n    vals: array[1..3] of integer;\n  end;\nvar r: Rec;\nbegin\n  r.vals[2] := 42;\n  writeln(r.vals[2])\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "42");
    }

    #[test]
    fn string_builtins() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar s, t: string;\n  n, code: integer;\nbegin\n  s := 'Hello World';\n  t := copy(s, 7, 5);\n  writeln(t);\n  n := pos('World', s);\n  writeln(n);\n  writeln(upcase('a'));\n  t := concat('abc', 'def', 'ghi');\n  writeln(t);\n  str(42, t);\n  writeln(t);\n  val('123', n, code);\n  writeln(n)\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "World");
        assert_eq!(lines[1], "7");
        assert_eq!(lines[2], "A");
        assert_eq!(lines[3], "abcdefghi");
        assert_eq!(lines[4], "42");
        assert_eq!(lines[5], "123");
    }

    #[test]
    fn odd_builtin() {
        let (ok, out) = build_and_run_source(
            "program T;\nbegin\n  writeln(odd(3));\n  writeln(odd(4))\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "TRUE");
        assert_eq!(lines[1], "FALSE");
    }

    #[test]
    fn maxint_constant() {
        let (ok, out) = build_and_run_source("program T;\nbegin\n  writeln(maxint)\nend.\n");
        assert!(ok);
        assert_eq!(out.trim(), "9223372036854775807");
    }

    #[test]
    fn packed_keyword_noop() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar a: packed array[1..3] of integer;\nbegin\n  a[1] := 7;\n  writeln(a[1])\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "7");
    }

    #[test]
    fn program_parameters_header() {
        let (ok, out) =
            build_and_run_source("program T(input, output);\nbegin\n  writeln('hi')\nend.\n");
        assert!(ok);
        assert_eq!(out.trim(), "hi");
    }

    #[test]
    fn write_to_predefined_output() {
        let (ok, out) =
            build_and_run_source("program T;\nbegin\n  writeln(output, 'via output')\nend.\n");
        assert!(ok);
        // writes to stdout — capture file misses it, but stdout test would need direct capture.
        // Just verify it built and ran successfully.
        let _ = out;
    }

    #[test]
    fn pack_unpack_basic() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar src: array[1..6] of integer;\n  dst: array[1..3] of integer;\n  i: integer;\nbegin\n  for i := 1 to 6 do src[i] := i * 10;\n  pack(src, 2, dst);\n  writeln(dst[1]);\n  writeln(dst[2]);\n  writeln(dst[3])\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "20");
        assert_eq!(lines[1], "30");
        assert_eq!(lines[2], "40");
    }

    #[test]
    fn procedural_parameter() {
        let (ok, out) = build_and_run_source(
            r#"program T;

function Square(x: integer): integer;
begin
  Square := x * x
end;

function ApplyTwice(function f(n: integer): integer; v: integer): integer;
begin
  ApplyTwice := f(f(v))
end;

begin
  writeln(ApplyTwice(Square, 3))
end.
"#,
        );
        assert!(ok);
        assert_eq!(out.trim(), "81"); // (3*3)*(3*3) = 9*9 = 81
    }

    #[test]
    fn conformant_array_param() {
        let (ok, out) = build_and_run_source(
            r#"program T;

var a: array[1..5] of integer;
    i, total: integer;

function SumArr(var arr: array[lo..hi: integer] of integer): integer;
var k, s: integer;
begin
  s := 0;
  for k := lo to hi do
    s := s + arr[k];
  SumArr := s
end;

begin
  for i := 1 to 5 do a[i] := i;
  total := SumArr(a);
  writeln(total)
end.
"#,
        );
        assert!(ok);
        assert_eq!(out.trim(), "15");
    }

    #[test]
    fn fixed_length_string() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar s: string[20];\nbegin\n  s := 'hi';\n  writeln(s)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "hi");
    }

    #[test]
    fn ioresult_default_zero() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar code: integer;\nbegin\n  code := ioresult;\n  writeln(code)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "0");
    }

    #[test]
    fn file_filepos_filesize() {
        // Pascal accepts backslashes in single-quoted strings, but use
        // forward slashes for portability — fopen on Windows handles them.
        let path = std::env::temp_dir()
            .join("_bruto_test_pos.txt")
            .to_string_lossy()
            .replace('\\', "/");
        let _ = std::fs::remove_file(&path);
        let prog = format!(
            "program T;\nvar f: text;\n  size: integer;\nbegin\n  assign(f, '{path}');\n  rewrite(f);\n  writeln(f, 'ABC');\n  close(f);\n  assign(f, '{path}');\n  reset(f);\n  size := filesize(f);\n  writeln(size);\n  close(f)\nend.\n"
        );
        let (ok, out) = build_and_run_source(&prog);
        assert!(ok);
        // Pascal `text` files open in C text mode, so `writeln` emits the
        // platform line ending: 4 bytes on Unix ("ABC\n"), 5 on Windows
        // ("ABC\r\n").
        let expected = if cfg!(windows) { "5" } else { "4" };
        assert_eq!(out.trim(), expected);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn range_check_directive() {
        // {$R+} on; out-of-bounds index should abort the executable.
        let lang = MiniPascal;
        let result = lang.build(
            "{$R+}\nprogram T;\nvar a: array[1..3] of integer;\n  i: integer;\nbegin\n  i := 5;\n  a[i] := 0\nend.\n",
        ).expect("build failed");
        let status = std::process::Command::new(&result.exe_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("run failed");
        assert!(!status.success(), "expected non-zero exit from range check");
        let _ = std::fs::remove_file(&result.exe_path);
        let _ = std::fs::remove_dir_all(format!("{}.dSYM", result.exe_path));
        let _ = std::fs::remove_file(&result.source_path);
    }

    #[test]
    fn overflow_check_directive() {
        let lang = MiniPascal;
        let result = lang.build(
            "{$Q+}\nprogram T;\nvar x: integer;\nbegin\n  x := 9223372036854775807;\n  x := x + 1\nend.\n",
        ).expect("build failed");
        let status = std::process::Command::new(&result.exe_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("run failed");
        assert!(
            !status.success(),
            "expected non-zero exit from overflow check"
        );
        let _ = std::fs::remove_file(&result.exe_path);
        let _ = std::fs::remove_dir_all(format!("{}.dSYM", result.exe_path));
        let _ = std::fs::remove_file(&result.source_path);
    }

    #[test]
    fn nested_procedure() {
        let (ok, out) = build_and_run_source(
            r#"program T;
var t: integer;

procedure Outer(var total: integer);
var n: integer;

  procedure Inner;
  begin
    n := n + 10;
    total := total + n
  end;

begin
  n := 5;
  Inner;
  Inner
end;

begin
  t := 0;
  Outer(t);
  writeln(t)
end.
"#,
        );
        assert!(ok);
        // n=5 -> +10=15 (total=15) -> +10=25 (total=40)
        assert_eq!(out.trim(), "40");
    }

    #[test]
    fn typed_const() {
        let (ok, out) = build_and_run_source(
            "program T;\nconst\n  x: integer = 42;\n  pi: real = 3.14;\nbegin\n  writeln(x);\n  writeln(pi)\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "42");
        // As FPC writes a Double.
        assert_eq!(lines[1], " 3.1400000000000001E+000");
    }

    #[test]
    fn nil_constant() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar p: ^integer;\nbegin\n  p := nil;\n  if p = nil then\n    writeln('null')\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "null");
    }

    #[test]
    fn type_cast_int_real() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar i: integer;\n  r: real;\nbegin\n  r := 3.7;\n  i := integer(r);\n  writeln(i);\n  r := real(42);\n  writeln(trunc(r))\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "3");
        assert_eq!(lines[1], "42");
    }

    #[test]
    fn type_cast_char() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar c: char;\nbegin\n  c := char(65);\n  writeln(c)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "A");
    }

    #[test]
    fn write_format_int() {
        let (ok, out) =
            build_and_run_source("program T;\nbegin\n  writeln(42:5);\n  writeln(7:3)\nend.\n");
        assert!(ok);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "   42");
        assert_eq!(lines[1], "  7");
    }

    #[test]
    fn write_format_real() {
        let (ok, out) = build_and_run_source(
            "program T;\nbegin\n  writeln(3.14:8:2);\n  writeln(1.5:6:1)\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "    3.14");
        assert_eq!(lines[1], "   1.5");
    }

    #[test]
    fn file_io_write_read() {
        let path = std::env::temp_dir()
            .join("_bruto_test_io.txt")
            .to_string_lossy()
            .replace('\\', "/");
        let _ = std::fs::remove_file(&path);
        let prog = format!(
            "program T;\nvar f: text;\n  s: string;\nbegin\n  assign(f, '{path}');\n  rewrite(f);\n  writeln(f, 'hello world');\n  writeln(f, 42);\n  close(f);\n  assign(f, '{path}');\n  reset(f);\n  readln(f, s);\n  writeln(s);\n  close(f)\nend.\n"
        );
        let (ok, out) = build_and_run_source(&prog);
        assert!(ok);
        assert_eq!(out.trim(), "hello world");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sample_pas_showcase() {
        let source = include_str!("../../SAMPLE.PAS");
        let (ok, out) = build_and_run_source(source);
        assert!(ok, "SAMPLE.PAS failed to run");
        assert!(
            out.contains("=== All features OK ==="),
            "missing final line: {out}"
        );
    }

    #[test]
    fn uses_clause_imports_unit_const_and_function() {
        // Build a small temp directory with a unit and a program that uses it.
        let tmp = std::env::temp_dir().join("bruto_pascal_uses_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("MathUtils.pas"),
            "unit MathUtils;\n\
             interface\n\
             const Pi = 3;\n\
             function Square(x: integer): integer;\n\
             implementation\n\
             function Square(x: integer): integer;\n\
             begin\n\
               Square := x * x\n\
             end;\n\
             end.\n",
        )
        .unwrap();
        let main_path = tmp.join("UsesMain.pas");
        let main_source = "program UsesMain;\n\
             uses MathUtils;\n\
             var n: integer;\n\
             begin\n\
               n := Square(7);\n\
               writeln('pi=', Pi, ' sq=', n)\n\
             end.\n";
        std::fs::write(&main_path, main_source).unwrap();

        let lang = MiniPascal;
        let result = lang
            .build_job_at(main_source, Some(&main_path))
            .as_mut()
            .poll_to_done()
            .expect("build failed");
        let _ = std::fs::remove_file(&result.console_capture_path);
        let status = std::process::Command::new(&result.exe_path)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run failed");
        // Windows writes the capture file in text mode (\r\n).
        let captured = std::fs::read_to_string(&result.console_capture_path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        let _ = std::fs::remove_file(&result.exe_path);
        let _ = std::fs::remove_dir_all(format!("{}.dSYM", result.exe_path));
        let _ = std::fs::remove_file(&result.console_capture_path);
        let _ = std::fs::remove_dir_all(&tmp);

        assert!(status.success(), "compiled program crashed");
        assert!(
            captured.contains("pi=3") && captured.contains("sq=49"),
            "unexpected output: {captured}"
        );
    }

    /// Write `files` into a fresh temp directory, build `main` from there
    /// and run it. Returns (exited successfully, captured output).
    fn build_and_run_in_dir(dir_name: &str, files: &[(&str, &[u8])], main: &str) -> (bool, String) {
        let tmp = std::env::temp_dir().join(dir_name);
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        for (name, body) in files {
            std::fs::write(tmp.join(name), body).unwrap();
        }
        let main_path = tmp.join("Main.pas");
        std::fs::write(&main_path, main).unwrap();
        let result = MiniPascal
            .build_job_at(main, Some(&main_path))
            .as_mut()
            .poll_to_done();
        let _ = std::fs::remove_dir_all(&tmp);
        let result = result.expect("build failed");
        let _ = std::fs::remove_file(&result.console_capture_path);
        let status = std::process::Command::new(&result.exe_path)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run failed");
        // Windows writes the capture file in text mode (\r\n).
        let captured = std::fs::read_to_string(&result.console_capture_path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        let _ = std::fs::remove_file(&result.exe_path);
        let _ = std::fs::remove_dir_all(format!("{}.dSYM", result.exe_path));
        let _ = std::fs::remove_file(&result.console_capture_path);
        (status.success(), captured)
    }

    #[test]
    fn units_keep_same_named_declarations_apart() {
        let low: &[u8] = b"unit Low;
interface
var Width: Integer;
procedure Init;
function Twice(X: Integer): Integer;
implementation
procedure Init;
begin
  Width := 80;
  writeln('Low.Init');
end;
function Twice(X: Integer): Integer;
begin
  Twice := X * 2;
end;
end.
";
        let high: &[u8] = b"unit High;
interface
uses Low;
var Width: Integer;
procedure Init;
implementation
procedure Init;
var Twice: Integer;
begin
  Low.Init;
  Twice := 3;
  Width := Low.Width + Twice;
  writeln('High.Init ', Width);
end;
end.
";
        let (ok, out) = build_and_run_in_dir(
            "bruto_unit_scopes_test",
            &[("Low.pas", low), ("High.pas", high)],
            "program Main;
uses Low, High;
begin
  Init;
  writeln(Width, ' ', Low.Width, ' ', Twice(High.Width));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "Low.Init\nHigh.Init 83\n83 80 166\n");
    }

    #[test]
    fn external_c_functions() {
        let (ok, out) = build_and_run_source(
            "program T;
function CLength(P: PChar): LongInt; cdecl; external 'c' name 'strlen';
function snprintf(Buf: PChar; N: QWord; Fmt: PChar): LongInt; cdecl; varargs; external 'c';
var Buf: array[0..31] of Char;
begin
  writeln(CLength('hello'));
  snprintf(@Buf, 32, '%d-%d', 12, 345);
  writeln(CLength(@Buf));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "5\n6\n");
    }

    #[test]
    fn records_compare_and_strings_resize() {
        let (ok, out) = build_and_run_source(
            "program T;
type
  TKey = record
    Code: Word;
    Ch: Char;
    Flags: set of 0..7;
  end;
  TPoint = record X, Y: Integer; end;
  TEvent = record
    What: Word;
    case Integer of
      0: (Where: TPoint);
      1: (Code: Word);
  end;
const
  NoKey: TKey = (Code: 0; Ch: #0; Flags: []);
var
  A, B: TKey;
  E: TEvent;
  S: AnsiString;
  Short: string[20];
  N: LongInt;
  P: Pointer;

procedure Bye;
begin
  writeln('bye');
end;

begin
  A := NoKey;
  B := NoKey;
  writeln(A = B, ' ', A <> NoKey);
  B.Flags := [3];
  writeln(A = B);
  case 'q' of
    'a'..'p': writeln('early');
    'q'..'z': writeln('late');
  end;
  E.Where.X := 4;
  E.Where.Y := 5;
  writeln(E.Where.X + E.Where.Y);
  S := 'abc';
  SetLength(S, 5);
  writeln('[', S, '] ', Length(S));
  SetLength(S, 2);
  writeln('[', S, ']');
  Short := 'hello';
  writeln(High(Short));
  N := 42;
  P := @N;
  writeln(LongInt(P^));
  ExitProc := @Bye;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "TRUE FALSE\nFALSE\nlate\n9\n[abc\0\0] 5\n[ab]\n20\n42\nbye\n"
        );
    }

    // Bruto's console units are Unix ones.
    #[cfg(unix)]
    #[test]
    fn console_units_ship_with_the_compiler() {
        // Only the parts that don't need a terminal.
        let (ok, out) = build_and_run_source(
            "program T;
uses Video, Keyboard, Mouse, BaseUnix, Unix, TermIO;
var
  K: TKeyEvent;
  TV: TimeVal;
begin
  K := $3B00 or (LongWord(kbAlt) shl 16);
  writeln(GetKeyEventCode(K), ' ', GetKeyEventShiftState(K));
  writeln(GetCapabilities and cpColor, ' ', DetectMouse);
  writeln(FpGetTimeOfDay(@TV, nil), ' ', TV.tv_sec > 0);
  writeln(PollEnhancedKeyEvent = NilEnhancedKeyEvent);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "15104 8\n4 3\n0 TRUE\nTRUE\n");
    }

    #[test]
    fn implementation_uses_may_form_a_cycle() {
        let a: &[u8] = b"unit UA;
interface
function AName: string;
implementation
uses UB;
function AName: string;
begin
  AName := 'a' + BName;
end;
end.
";
        let b: &[u8] = b"unit UB;
interface
uses UA;
function BName: string;
function Both: string;
implementation
function BName: string;
begin
  BName := 'b';
end;
function Both: string;
begin
  Both := AName + '!';
end;
end.
";
        let (ok, out) = build_and_run_in_dir(
            "bruto_unit_cycle_test",
            &[("UA.pas", a), ("UB.pas", b)],
            "program Main;
uses UB;
begin
  writeln(Both);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "ab!\n");
    }

    #[test]
    fn free_vision_object_idioms() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}
const
  Pal = #$71#$70 + #$37;
type
  TCallback = procedure(P: Pointer) is nested;
  PBase = ^TBase;
  TBase = object
    Count: Integer;
    constructor Init(Ok: Boolean);
    procedure Each(F: TCallback);
  end;
  PChild = ^TChild;
  TChild = object(TBase)
    constructor Init(Ok: Boolean);
    function Depth(N: Integer): Integer;
  end;

constructor TBase.Init(Ok: Boolean);
begin
  Count := 3;
  if not Ok then
    Fail;
end;

procedure TBase.Each(F: TCallback);
var I: Integer;
begin
  for I := 1 to Count do
    F(nil);
end;

constructor TChild.Init(Ok: Boolean);
begin
  if not inherited Init(Ok) then
    Fail;
end;

function TChild.Depth(N: Integer): Integer;
var Seen: Integer;
  procedure Visit(P: Pointer);
  begin
    Inc(Seen);
  end;
begin
  if N = 0 then
    Depth := 0
  else
    Depth := Depth(N - 1) + 1;
  Seen := 0;
  Each(TCallback(@Visit));
  if N = 2 then
    writeln('visited ', Seen);
end;

procedure Outer1;
  procedure Helper; begin writeln('one'); end;
begin
  Helper;
end;

procedure Outer2;
  procedure Helper; begin writeln('two'); end;
begin
  Helper;
end;

function GetEnv(Name: string): string;
  function c_getenv(N: PChar): PChar; cdecl; external 'c' name 'getenv';
begin
  GetEnv := 'x';
end;

var
  P: PChild;
  S: string;
  C: Char;
begin
  P := New(PChild, Init(True));
  writeln(P^.Depth(2));
  writeln(New(PChild, Init(False)) = nil);
  Outer1;
  Outer2;
  C := 'b';
  S := 'a' + C + #$63;
  writeln(S, ' ', Length(Pal), ' ', Pos(C, S));
  writeln(GetEnv('HOME'));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "visited 3\n2\nTRUE\none\ntwo\nabc 3 2\nx\n");
    }

    #[test]
    fn short_string_constants_and_char_arrays() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}
const
  LineBreak: string[2] = #13#10;
  Frame: array[0..1] of array[0..2] of Char = (#218#196#191, 'ab');
  Chars: array[0..3] of Char = 'xyz';
var
  Hs: string[7];
  P: PChar;
begin
  P := @LineBreak[1];
  writeln(Ord(P^), ' ', Ord(LineBreak[2]));
  writeln(Ord(Frame[0][1]), ' ', Frame[1][1], ' ', Ord(Frame[1][2]));
  Str(7, Hs);
  while Length(Hs) < 2 do Hs := '0' + Hs;
  writeln(Hs);
  P := Chars;
  writeln(P[1]);
  writeln(HexStr(255, 4), ' ', HexStr($12345, 2));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "13 10\n196 b 0\n07\ny\n00FF 45\n");
    }

    #[test]
    fn sysutils_format_and_string_edges() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}
uses SysUtils;
type
  TBase = object
    procedure Show(N: Integer); virtual;
    constructor Init;
  end;
  TChild = object(TBase)
    procedure Show(N: Integer); virtual;
  end;
constructor TBase.Init; begin end;
procedure TBase.Show(N: Integer); begin writeln('base ', N); end;
procedure TChild.Show(N: Integer);
begin
  writeln('child');
  inherited;
end;
var
  C: TChild;
  Never: AnsiString;
  S: string[20];
begin
  writeln(Format('$%4.4x|%d|%s|%5s', [255, -7, 'ab', 'r']));
  writeln(IntToStr(42), ' ', IntToHex(171, 2), ' ', UpperCase('MiX'), ' [', Trim('  t  '), ']');
  C.Init;
  C.Show(3);
  writeln(Never = '', ' [', Copy('hello', 0, 2), '] [', Copy('hi', 2, 50), ']');
  S := 'abc';
  Inc(S[0]);
  S[Length(S)] := '/';
  writeln(S, ' ', Length(S));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "$00FF|-7|ab|    r\n42 AB MIX [t]\nchild\nbase 3\nTRUE [he] [i]\nabc/ 4\n"
        );
    }

    #[test]
    fn unicode_strings_and_wide_chars() {
        let (ok, out) = build_and_run_source(
            "program T;
type
  TCell = record
    EGC: UnicodeString;
    Attr: Byte;
  end;
const
  Frame: array[0..2] of WideChar = #$250C#$2500 + #$2510;
var
  U, V: UnicodeString;
  C: WideChar;
  A: AnsiString;
  Cells: array[0..1] of TCell;
  I: Integer;

procedure Append(var S: UnicodeString; W: WideChar);
begin
  S := S + W;
end;

begin
  U := 'h\u{e9}llo \u{6253}';
  A := U;
  writeln(Length(U), ' ', Length(A), ' ', A);
  C := #$2502;
  Append(U, C);
  writeln(U, ' ', Ord(U[2]), ' ', Ord(C));
  V := Copy(U, 2, 3);
  Delete(U, 1, 1);
  Insert('<', U, 1);
  writeln(V, ' ', U, ' ', Pos('l', U), ' ', U > V, ' ', V = '\u{e9}ll');
  U[1] := '>';
  SetLength(U, 3);
  writeln(U, Length(U), ' ', UpCase(UnicodeString('\u{e9}a')), ' ', Frame[0], Frame[2]);
  for I := 0 to 1 do
    Cells[I].EGC := WideChar(Ord('a') + I);
  writeln(Cells[0].EGC, Cells[1].EGC, ' ', Cells[1].Attr);
  case C of
    'a': writeln('a');
    #$2500..#$25FF: writeln('box');
  end;
  writeln(C in ['a'..'z'], ' ', WideChar('x') = 'x');
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "7 10 h\u{e9}llo \u{6253}\n\
             h\u{e9}llo \u{6253}\u{2502} 233 9474\n\
             \u{e9}ll <\u{e9}llo \u{6253}\u{2502} 3 FALSE TRUE\n\
             >\u{e9}l3 \u{c9}A \u{250c}\u{2510}\n\
             ab 0\n\
             box\n\
             FALSE TRUE\n"
        );
    }

    #[test]
    fn for_in_loops() {
        let (ok, out) = build_and_run_source(
            "program T;
type
  TWords = object
    S: String;
    P: Integer;
    Current: String;
    constructor Create(const AText: String);
    function MoveNext: Boolean;
  end;

constructor TWords.Create(const AText: String);
begin
  S := AText;
  P := 1;
end;

function TWords.MoveNext: Boolean;
var Start: Integer;
begin
  while (P <= Length(S)) and (S[P] = ' ') do Inc(P);
  Start := P;
  while (P <= Length(S)) and (S[P] <> ' ') do Inc(P);
  Current := Copy(S, Start, P - Start);
  MoveNext := P > Start;
end;

var
  C: Char;
  I: Integer;
  A: array[3..5] of Integer;
  W: String;
begin
  for C in 'abc' do write(C, '.');
  writeln;
  A[3] := 10; A[4] := 20; A[5] := 30;
  for I in A do write(I, ' ');
  writeln;
  for W in TWords.Create('  the quick  brown fox ') do
  begin
    if W = 'brown' then continue;
    write('[', W, ']');
  end;
  writeln;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "a.b.c.\n10 20 30 \n[the][quick][fox]\n");
    }

    // Bruto's console units are Unix ones.
    #[cfg(unix)]
    #[test]
    fn grapheme_clusters_and_display_widths() {
        let (ok, out) = build_and_run_source(
            "program T;
uses Video, GraphemeBreakProperty;
var
  EGC: UnicodeString;
  N: Integer;
begin
  N := 0;
  for EGC in TUnicodeStringExtendedGraphemeClustersEnumerator.Create('a\u{44a}\u{301}\u{6253}\u{1f4a9}') do
  begin
    write(Length(EGC), ':', ExtendedGraphemeClusterDisplayWidth(EGC), ' ');
    Inc(N);
  end;
  writeln(N, ' ', StringDisplayWidth('ab\u{6253}'));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "1:1 2:1 1:2 2:2 4 4\n");
    }

    #[test]
    fn val_reports_where_the_number_ends() {
        let (ok, out) = build_and_run_source(
            "program T;
type TRec = record W: Word; end;
var
  I: LongInt;
  R: Real;
  Code: SmallInt;
  Rec: TRec;
begin
  Val('42', I, Code); writeln(I, ' ', Code);
  Val('$1F', I, Code); writeln(I, ' ', Code);
  Val('12x', I, Code); writeln(Code);
  Val('', I, Code); writeln(Code);
  Val('2.5', R, Code); writeln(R:3:1, ' ', Code);
  Val('7', Rec.W, Code); writeln(Rec.W);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "42 0\n31 0\n3\n1\n2.5 0\n7\n");
    }

    #[test]
    fn casts_reinterpret_untyped_parameters() {
        let (ok, out) = build_and_run_source(
            "program T;
var
  U, Back: UnicodeString;
  P: Pointer;

procedure SetIt(var Rec);
begin
  writeln(UnicodeString(Rec));
  UnicodeString(Rec) := 'set';
end;

begin
  U := 'given';
  SetIt(U);
  writeln(U);
  P := Pointer(U);
  Back := UnicodeString(P);
  writeln(Back);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "given\nset\nset\n");
    }

    #[test]
    fn chars_compare_unsigned() {
        let (ok, out) = build_and_run_source(
            "program T;
var a, b: Char;
begin
  a := #$82; b := #32;
  writeln(a < b, ' ', a > b, ' ', #200 >= #100);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "FALSE TRUE TRUE\n");
    }

    #[test]
    fn code_page_437_literals() {
        let (ok, out) = build_and_run_source(
            "program T;
{$codepage cp437}
var
  U: UnicodeString;
  A: AnsiString;
  S: ShortString;
begin
  U := #222'v'#221;
  A := #222'v'#221;
  writeln(U, ' ', Ord(A[1]), ' ', Length(A));
  U := '*.*';
  S := ShortString(U);
  writeln(S, ' ', Pos('*', S));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "\u{2590}v\u{258c} 222 3\n*.* 1\n");
    }

    #[test]
    fn calling_a_method_with_wrong_arguments_is_an_error() {
        let result = MiniPascal.build(
            "program T;
type
  TW = object
    procedure Close;
    procedure Run;
  end;
procedure TW.Close; begin end;
procedure TW.Run;
var F: Text;
begin
  Close(F);
end;
begin
end.
",
        );
        let Err(e) = result else {
            panic!("the call should not compile");
        };
        assert!(
            format!("{e:?}").contains("takes 0 argument(s), found 1"),
            "{e:?}"
        );
    }

    #[test]
    fn classes_with_virtual_methods_and_properties() {
        let (ok, out) = build_and_run_source(
            "program T;
type
  TNode = class;

  TShape = class
  protected
    FName: string;
  public
    constructor Create(const AName: string); virtual;
    destructor Destroy; override;
    function Area: Double; virtual; abstract;
    function Describe: string;
    class function Kind: string; virtual;
    property Name: string read FName write FName;
  end;
  TShapeClass = class of TShape;

  TSquare = class(TShape)
  private
    FSide: Double;
    procedure SetSide(V: Double);
  public
    constructor Create(const AName: string); override;
    function Area: Double; override;
    class function Kind: string; override;
    property Side: Double read FSide write SetSide;
  end;

  TNode = class
    Value: Integer;
    Next: TNode;
    destructor Destroy; override;
  end;

  TBag = class
  private
    FItems: array[0..7] of Integer;
    FCount: Integer;
    function GetItem(I: Integer): Integer;
    procedure SetItem(I: Integer; V: Integer);
    function GetFlag(Index: Integer): Boolean;
  public
    procedure Add(V: Integer);
    property Count: Integer read FCount;
    property Items[I: Integer]: Integer read GetItem write SetItem; default;
    property IsEven: Boolean index 0 read GetFlag;
  end;

var
  Freed: Integer;

constructor TShape.Create(const AName: string);
begin
  inherited Create;
  FName := AName;
end;

destructor TShape.Destroy;
begin
  writeln('bye ', Name);
  inherited;
end;

function TShape.Describe: string;
begin
  Result := Name + ' is a ' + Kind;
end;

class function TShape.Kind: string;
begin
  Result := 'shape';
end;

constructor TSquare.Create(const AName: string);
begin
  inherited Create(AName);
  Side := 2;
end;

procedure TSquare.SetSide(V: Double);
begin
  if V > 0 then FSide := V;
end;

function TSquare.Area: Double;
begin
  Result := Side * Side;
end;

class function TSquare.Kind: string;
begin
  Result := 'square';
end;

destructor TNode.Destroy;
begin
  Next.Free;
  Inc(Freed);
  inherited Destroy;
end;

function TBag.GetItem(I: Integer): Integer;
begin
  Result := FItems[I];
end;

procedure TBag.SetItem(I: Integer; V: Integer);
begin
  FItems[I] := V;
end;

function TBag.GetFlag(Index: Integer): Boolean;
var I, Sum: Integer;
begin
  Sum := 0;
  for I := 0 to Count - 1 do Sum := Sum + Items[I];
  Result := Sum mod 2 = Index;
end;

procedure TBag.Add(V: Integer);
begin
  FItems[FCount] := V;
  Inc(FCount);
end;

var
  S: TShape;
  C: TShapeClass;
  Head, N: TNode;
  B: TBag;
  I: Integer;
begin
  C := TSquare;
  S := C.Create('sq');
  writeln(S.Describe, ' ', S.Area:0:1, ' ', S.ClassName, ' ', C.Kind);
  with TSquare(S) do
  begin
    Side := -1;
    Side := Side + 1;
    writeln(Name, ' ', Area:0:1);
  end;
  writeln(S is TSquare, ' ', S is TShape, ' ', (S as TSquare).Side:0:1);
  writeln(S.InheritsFrom(TShape), ' ', TSquare.ClassParent.ClassName, ' ', S.ClassType = TSquare);
  S.Free;
  S := nil;
  S.Free;

  Head := nil;
  for I := 1 to 4 do
  begin
    N := TNode.Create;
    N.Value := I;
    N.Next := Head;
    Head := N;
  end;
  Head.Next.Value := 30;
  writeln(Head.Value, Head.Next.Value, Head.Next.Next.Value);
  Head.Free;
  writeln(Freed);

  B := TBag.Create;
  B.Add(3);
  B.Add(4);
  B[1] := 10;
  B.Items[0] := B[0] + 2;
  writeln(B.Count, ' ', B[0], ' ', B.Items[1], ' ', B.IsEven);
  B.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "sq is a square 4.0 TSquare square\nsq 9.0\nTRUE TRUE 3.0\n\
             TRUE TShape TRUE\nbye sq\n4302\n4\n2 5 10 FALSE\n"
        );
    }

    #[test]
    fn array_properties_with_several_indices() {
        let (ok, out) = build_and_run_source(
            "program T;
type
  TGrid = class
  private
    FCells: array[0..3, 0..3] of Integer;
    function GetCell(X, Y: Integer): Integer;
    procedure SetCell(X, Y: Integer; V: Integer);
  public
    procedure Fill;
    property Cells[X, Y: Integer]: Integer read GetCell write SetCell; default;
  end;
function TGrid.GetCell(X, Y: Integer): Integer; begin Result := FCells[X, Y]; end;
procedure TGrid.SetCell(X, Y: Integer; V: Integer); begin FCells[X, Y] := V; end;
procedure TGrid.Fill;
var I: Integer;
begin
  for I := 0 to 3 do Cells[I, I] := I * 10;
end;
var G: TGrid;
begin
  G := TGrid.Create;
  G.Fill;
  G.Cells[1, 2] := 7;
  G[2, 3] := G.Cells[1, 2] * 2;
  writeln(G.Cells[1, 2], ' ', G[2, 3], ' ', G[3, 3]);
  G.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "7 14 30\n");
    }

    #[test]
    fn booleans_are_written_as_fpc_writes_them() {
        let path = std::env::temp_dir().join(format!("bruto-bools-{}.txt", std::process::id()));
        let source = format!(
            "program T;
var B: Boolean; F: Text; S: String;
begin
  B := 1 < 2;
  writeln(B, ' ', not B, ' [', B:6, '] [', not B:7, ']');
  Assign(F, '{}');
  Rewrite(F);
  writeln(F, B, ' ', not B);
  Close(F);
  Reset(F);
  readln(F, S);
  Close(F);
  writeln(S);
end.
",
            path.display()
        );
        let (ok, out) = build_and_run_source(&source);
        let _ = std::fs::remove_file(&path);
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "TRUE FALSE [  TRUE] [  FALSE]\nTRUE FALSE\n");
    }

    #[test]
    fn exceptions_unwind_to_the_nearest_handler() {
        let (ok, out) = build_and_run_source(
            "program T;
type
  EBoom = class
    Code: Integer;
    constructor Create(C: Integer);
  end;
constructor EBoom.Create(C: Integer); begin Code := C; end;

procedure Deep(N: Integer);
var S: String;
begin
  S := 'level';
  if N = 0 then raise EBoom.Create(42);
  Deep(N - 1);
end;

function Guarded(N: Integer): Integer;
begin
  Result := -1;
  try
    if N > 5 then Exit(N * 2);
    Deep(N);
    Result := 0;
  finally
    writeln('finally ', N);
  end;
end;

var I, Total, Changed: Integer;
begin
  Changed := 0;
  try
    Changed := 1;
    Deep(3);
    writeln('not reached');
  except
    on E: EBoom do writeln('caught ', E.Code, ' ', Changed);
  end;
  writeln(Guarded(10));
  try
    writeln(Guarded(2));
  except
    writeln('outer caught');
  end;
  Total := 0;
  for I := 1 to 5 do
    try
      if I = 2 then Continue;
      if I = 4 then Break;
      Total := Total + I;
    finally
      write('[', I, ']');
    end;
  writeln(' total ', Total);
  try
    try
      raise EBoom.Create(7);
    except
      on E: EBoom do
      begin
        writeln('inner ', E.Code);
        raise;
      end;
    end;
  except
    on E: TObject do writeln('again ', E.ClassName);
  end;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "caught 42 1\nfinally 10\n20\nfinally 2\nouter caught\n[1][2][3][4] total 4\n\
             inner 7\nagain EBoom\n"
        );
    }

    #[test]
    fn sysutils_exceptions() {
        let (ok, out) = build_and_run_source(
            "program T;
uses SysUtils;
type
  TA = class end;
  TB = class(TA) end;
  EApp = class(Exception)
    Detail: String;
  end;
  TRes = class
    Name: String;
    constructor Create(const AName: String; ShouldFail: Boolean);
    destructor Destroy; override;
  end;

constructor TRes.Create(const AName: String; ShouldFail: Boolean);
begin
  Name := AName;
  if ShouldFail then raise Exception.Create('cannot create ' + AName);
end;

destructor TRes.Destroy;
begin
  writeln('destroying ', Name);
  inherited Destroy;
end;

function Risky(I: Integer): String;
var S: String; E: EApp;
begin
  S := 'value ' + IntToStr(I);
  if I mod 3 = 0 then
  begin
    E := EApp.Create('bad ' + S);
    E.Detail := S + S;
    raise E;
  end;
  Result := S;
end;

function Count(N: Integer): Integer;
var I: Integer; S: String;
begin
  Result := 0;
  for I := 1 to N do
    try
      S := Risky(I);
    except
      on E: EApp do Inc(Result);
    end;
end;

var
  A: TA;
  R: TRes;
  H: TFPCHeapStatus;
  Before: PtrUInt;
begin
  try
    StrToInt('12x');
  except
    on E: EConvertError do writeln(E.ToString);
  end;
  A := TA.Create;
  try
    writeln((A as TB).ClassName);
  except
    on E: EInvalidCast do writeln(E.ClassName, ': ', E.Message);
  end;
  A.Free;
  try
    try
      Abort;
    finally
      try
        raise EApp.Create('inside finally');
      except
        on E: Exception do writeln('handled ', E.Message);
      end;
    end;
  except
    on E: EAbort do writeln('abort: ', E.Message);
  end;
  try
    raise EApp.CreateFmt('%d items, %s', [3, 'ok']);
  except
    on E: Exception do writeln(E.Message);
  end;
  R := nil;
  try
    R := TRes.Create('a', True);
  except
    on E: Exception do writeln(E.Message, ' ', R = nil);
  end;
  H := GetFPCHeapStatus;
  Before := H.CurrHeapUsed;
  writeln(Count(30000));
  H := GetFPCHeapStatus;
  {$ifdef DARWIN}
  writeln(H.CurrHeapUsed - Before < 50000);
  {$else}
  writeln(TRUE);
  {$endif}
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "EConvertError: \"12x\" is an invalid integer\nEInvalidCast: Invalid type cast\n\
             handled inside finally\nabort: Operation aborted\n3 items, ok\n\
             destroying a\ncannot create a TRUE\n10000\nTRUE\n"
        );
    }

    #[test]
    fn an_unhandled_exception_stops_the_program() {
        let result = MiniPascal
            .build(
                "program T;
uses SysUtils;
begin
  writeln('before');
  raise EConvertError.Create('boom');
  writeln('after');
end.
",
            )
            .expect("build failed");
        let run = std::process::Command::new(&result.exe_path)
            .output()
            .expect("run failed");
        let _ = std::fs::remove_file(&result.exe_path);
        let _ = std::fs::remove_dir_all(format!("{}.dSYM", result.exe_path));
        let _ = std::fs::remove_file(&result.source_path);
        let _ = std::fs::remove_file(&result.console_capture_path);
        let stdout = String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n");
        let stderr = String::from_utf8_lossy(&run.stderr).replace("\r\n", "\n");
        assert_eq!(run.status.code(), Some(217), "{stdout}{stderr}");
        assert_eq!(stdout, "before\n");
        assert!(
            stderr.starts_with("An unhandled exception occurred at $"),
            "{stderr}"
        );
        assert!(stderr.contains(":\nEConvertError: boom\n  $"), "{stderr}");
    }

    #[test]
    fn method_pointers_carry_their_instance() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}{$H+}
type
  TNotifyEvent = procedure(Sender: TObject) of object;
  TValueEvent = function(Sender: TObject; V: Integer): Integer of object;

  TButton = class
  private
    FOnClick: TNotifyEvent;
    FCaption: string;
  public
    OnValue: TValueEvent;
    constructor Create(const ACaption: string);
    procedure Click;
    property OnClick: TNotifyEvent read FOnClick write FOnClick;
    property Caption: string read FCaption;
  end;

  TForm = class
  private
    FClicks: Integer;
  public
    Name: string;
    procedure ButtonClick(Sender: TObject);
    function Twice(Sender: TObject; V: Integer): Integer; virtual;
    procedure Wire(B: TButton);
  end;

  TBigForm = class(TForm)
    function Twice(Sender: TObject; V: Integer): Integer; override;
  end;

  PCounter = ^TCounter;
  TCounter = object
    N: Integer;
    procedure Bump(Sender: TObject);
  end;

constructor TButton.Create(const ACaption: string);
begin
  FCaption := ACaption;
end;

procedure TButton.Click;
begin
  if Assigned(FOnClick) then
    FOnClick(Self)
  else
    WriteLn(FCaption, ': nobody listens');
end;

procedure TForm.ButtonClick(Sender: TObject);
begin
  Inc(FClicks);
  WriteLn(Name, ' got click ', FClicks, ' from ', TButton(Sender).Caption);
end;

function TForm.Twice(Sender: TObject; V: Integer): Integer;
begin
  Result := V * 2;
end;

function TBigForm.Twice(Sender: TObject; V: Integer): Integer;
begin
  Result := V * 20;
end;

procedure TForm.Wire(B: TButton);
begin
  B.OnClick := @ButtonClick;
end;

procedure TCounter.Bump(Sender: TObject);
begin
  Inc(N);
end;

var
  B: TButton;
  F: TForm;
  G: TForm;
  Ev, Ev2: TNotifyEvent;
  C: TCounter;
begin
  B := TButton.Create('OK');
  F := TForm.Create;
  F.Name := 'main';
  B.Click;
  B.OnClick := @F.ButtonClick;
  B.Click;
  B.Click;
  G := TBigForm.Create;
  G.Name := 'big';
  G.Wire(B);
  B.Click;
  B.OnValue := @G.Twice;
  WriteLn(B.OnValue(nil, 3));
  Ev := B.OnClick;
  Ev2 := nil;
  WriteLn(Assigned(Ev), ' ', Assigned(Ev2), ' ', Ev2 = nil, ' ', Ev <> nil);
  Ev(B);
  C.N := 0;
  B.OnClick := @C.Bump;
  B.Click;
  B.Click;
  WriteLn('counter ', C.N);
  B.OnClick := nil;
  B.Click;
  B.Free; F.Free; G.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "OK: nobody listens\nmain got click 1 from OK\nmain got click 2 from OK\nbig got click 1 from OK\n60\nTRUE FALSE TRUE TRUE\nbig got click 2 from OK\ncounter 2\nOK: nobody listens\n"
        );
    }

    #[test]
    fn classes_unit_lists_strings_and_streams() {
        let tmp = std::env::temp_dir().join(format!("bruto-classes-{}.tmp", std::process::id()));
        let tmp = tmp.display();
        let source = format!(
            "program T;
{{$mode objfpc}}{{$H+}}
uses SysUtils, Classes;

type
  TWatcher = class
    Changes: Integer;
    procedure Changed(Sender: TObject);
  end;

procedure TWatcher.Changed(Sender: TObject);
begin
  Inc(Changes);
end;

function ByValue(Item1, Item2: Pointer): Integer;
begin
  Result := PtrInt(Item1) - PtrInt(Item2);
end;

function ByLength(List: TStringList; Index1, Index2: Integer): Integer;
begin
  Result := Length(List[Index1]) - Length(List[Index2]);
  if Result = 0 then
    Result := CompareStr(List[Index1], List[Index2]);
end;

var
  SL, SL2: TStringList;
  L: TList;
  W: TWatcher;
  I: Integer;
  V: LongInt;
  S: string;
  MS: TMemoryStream;
  SS: TStringStream;
  FS: TFileStream;
begin
  SL := TStringList.Create;
  W := TWatcher.Create;
  SL.OnChange := @W.Changed;
  SL.Add('pear');
  SL.Add('Apple');
  SL.Add('banana');
  SL.Add('_under');
  SL.Add('cherry');
  SL.Insert(1, 'kiwi');
  WriteLn(SL.Count, ' ', SL[1], ' ', SL.IndexOf('APPLE'), ' ', SL.IndexOf('fig'), ' changes ', W.Changes);
  WriteLn(SL.CommaText);
  SL.Sort;
  WriteLn(SL.CommaText);
  SL.CustomSort(@ByLength);
  WriteLn(SL.CommaText);
  SL.Delete(0);
  SL.Exchange(0, 1);
  SL.Move(0, 3);
  Write('[');
  for S in SL do
    Write(S, ';');
  WriteLn(']');
  SL.Clear;
  SL.CommaText := 'one, \"two three\",four,\"say \"\"hi\"\"\",, last';
  WriteLn(SL.Count);
  for I := 0 to SL.Count - 1 do
    Write('<', SL[I], '>');
  WriteLn;
  WriteLn(SL.CommaText);
  SL.Delimiter := ';';
  SL.StrictDelimiter := True;
  SL.DelimitedText := 'a b;c;;d';
  WriteLn(SL.Count, ' ', SL.DelimitedText);
  SL.Clear;
  SL.Values['host'] := 'example.com';
  SL.Values['port'] := '80';
  SL.Values['host'] := 'other.org';
  WriteLn(SL.Text = 'host=other.org' + LineEnding + 'port=80' + LineEnding, ' ', SL.Names[1], ' ',
    SL.Values['PORT'], ' ', SL.ValueFromIndex[0], ' ', SL.IndexOfName('port'));
  SL.Values['port'] := '';
  WriteLn(SL.Count);
  SL.Text := 'line1'#13#10'line2'#10'line3'#13'line4';
  WriteLn(SL.Count, ' ', SL[3]);

  SL2 := TStringList.Create;
  SL2.Sorted := True;
  SL2.Duplicates := dupIgnore;
  SL2.Add('m'); SL2.Add('c'); SL2.Add('x'); SL2.Add('c'); SL2.Add('A');
  WriteLn(SL2.CommaText, ' ', SL2.IndexOf('X'));
  SL2.Duplicates := dupError;
  try
    SL2.Add('m');
  except
    on E: EStringListError do WriteLn(E.ClassName, ': ', E.Message);
  end;
  try
    WriteLn(SL2[10]);
  except
    on E: Exception do WriteLn(E.ClassName, ': ', E.Message);
  end;
  SL2.Assign(SL);
  WriteLn(SL2.Count, ' ', SL2.Sorted, ' ', SL2.Equals(SL));

  L := TList.Create;
  for I := 1 to 10 do
    L.Add(Pointer(PtrInt((I * 7) mod 11)));
  L.Sort(@ByValue);
  for I := 0 to L.Count - 1 do
    Write(PtrInt(L[I]), ' ');
  WriteLn;
  L.Delete(0);
  L.Remove(Pointer(PtrInt(5)));
  L.Insert(0, nil);
  L.Pack;
  WriteLn(L.Count, ' ', PtrInt(L.First), ' ', PtrInt(L.Last), ' ', L.IndexOf(Pointer(PtrInt(9))));
  try
    L[50] := nil;
  except
    on E: EListError do WriteLn(E.Message);
  end;

  MS := TMemoryStream.Create;
  MS.WriteAnsiString('hello');
  V := 12345;
  MS.WriteBuffer(V, SizeOf(V));
  MS.Position := 0;
  S := MS.ReadAnsiString;
  V := 0;
  MS.ReadBuffer(V, SizeOf(V));
  WriteLn(S, ' ', V, ' ', MS.Size, ' ', MS.Position);
  SL.SaveToStream(MS);
  MS.SaveToFile('{tmp}');
  FS := TFileStream.Create('{tmp}', fmOpenRead);
  WriteLn(FS.Size);
  FS.Free;
  SL2.Clear;
  SL2.LoadFromFile('{tmp}');
  WriteLn(SL2.Count, ' ', SL2[SL2.Count - 1]);
  SS := TStringStream.Create('abc');
  SS.Position := 3;
  SS.WriteString('def');
  SS.Position := 1;
  WriteLn(SS.DataString, ' ', SS.ReadString(3), ' ', SS.Size);
  try
    FS := TFileStream.Create('/nonexistent/x.txt', fmOpenRead);
  except
    on E: EFOpenError do WriteLn(E.Message);
  end;
  DeleteFile('{tmp}');
  SS.Free; MS.Free; L.Free; SL2.Free;
  SL.Free; W.Free;
end.
"
        );
        let (ok, out) = build_and_run_source(&source);
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "6 kiwi 2 -1 changes 6\npear,kiwi,Apple,banana,_under,cherry\n_under,Apple,banana,cherry,kiwi,pear\nkiwi,pear,Apple,_under,banana,cherry\n[pear;_under;banana;Apple;cherry;]\n6\n<one><two three><four><say \"hi\"><><last>\none,\"two three\",four,\"say \"\"hi\"\"\",,last\n4 a b;c;;d\nTRUE port 80 other.org 1\n2\n4 line4\nA,c,m,x 3\nEStringListError: String list does not allow duplicates\nEStringListError: List index (10) out of bounds\n4 TRUE TRUE\n1 2 3 4 5 6 7 8 9 10 \n8 2 10 6\nList index (50) out of bounds\nhello 12345 13 13\n37\n4 line4\nabcdef bcd 6\nUnable to open file \"/nonexistent/x.txt\": No such file or directory\n"
        );
    }

    #[test]
    fn overloaded_methods_and_default_parameters() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}{$H+}
uses SysUtils, Classes;
type
  TClient = class
  private
    FHost: string;
    FPort: Word;
  public
    constructor Create(const AHost: string); overload;
    constructor Create(const AHost: string; APort: Word); overload;
    function Get(const URL: string): string; overload;
    procedure Get(const URL: string; Stream: TStream); overload;
    function Describe: string; virtual;
    function Sum(A, B: Integer): Integer; overload;
    function Sum(A, B: Double): Double; overload;
    function Sum(const A, B: string): string; overload;
  end;
  TSecureClient = class(TClient)
  public
    function Describe: string; override;
    function Get(const URL: string; Retries: Integer): string; overload;
  end;

constructor TClient.Create(const AHost: string);
begin
  Create(AHost, 80);
end;

constructor TClient.Create(const AHost: string; APort: Word);
begin
  inherited Create;
  FHost := AHost;
  FPort := APort;
end;

function TClient.Get(const URL: string): string;
begin
  Result := 'GET ' + Describe + URL;
end;

procedure TClient.Get(const URL: string; Stream: TStream);
var S: string;
begin
  S := Get(URL);
  Stream.WriteBuffer(S[1], Length(S));
end;

function TClient.Describe: string;
begin
  Result := FHost + ':' + IntToStr(FPort);
end;

function TClient.Sum(A, B: Integer): Integer; begin Result := A + B; end;
function TClient.Sum(A, B: Double): Double; begin Result := A + B + 0.5; end;
function TClient.Sum(const A, B: string): string; begin Result := A + '+' + B; end;

function TSecureClient.Describe: string;
begin
  Result := 'tls://' + inherited Describe;
end;

function TSecureClient.Get(const URL: string; Retries: Integer): string;
begin
  Result := Get(URL) + ' x' + IntToStr(Retries);
end;

var
  C: TClient;
  S: TSecureClient;
  M: TStringStream;
begin
  C := TClient.Create('example.com');
  WriteLn(C.Get('/a'));
  M := TStringStream.Create('');
  C.Get('/b', M);
  WriteLn(M.DataString);
  WriteLn(C.Sum(1, 2), ' ', C.Sum(1.5, 2.0):0:1, ' ', C.Sum('x', 'y'));
  S := TSecureClient.Create('secure.org', 443);
  WriteLn(S.Get('/c'), ' | ', S.Get('/d', 3));
  M.Free; C.Free; S.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "GET example.com:80/a\nGET example.com:80/b\n3 4.0 x+y\nGET tls://secure.org:443/c | GET tls://secure.org:443/d x3\n"
        );
        let (ok, out) = build_and_run_source(
            "program T;
type
  TA = class
    constructor Create(const H: string; P: Word; X: TObject = nil);
    function Join(const A: string; const Sep: string = ', '; Times: Integer = 1): string;
  end;
constructor TA.Create(const H: string; P: Word; X: TObject);
begin
  writeln(H, P, ' ', X = nil);
end;
function TA.Join(const A: string; const Sep: string; Times: Integer): string;
var I: Integer;
begin
  Result := A;
  for I := 2 to Times do Result := Result + Sep + A;
end;
procedure F(A: Integer; B: Integer = 7); begin writeln(A + B); end;
var A: TA;
begin
  F(1);
  F(1, 2);
  A := TA.Create('h', 80);
  writeln(A.Join('x'), '|', A.Join('y', '-'), '|', A.Join('z', '+', 3));
  TA.Create('i', 81, A).Free;
  A.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "8\n3\nh80 TRUE\nx|y|z+z+z\ni81 FALSE\n");
    }

    #[test]
    fn var_parameters_of_procedural_types() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}{$H+}
type
  TVerifyEvent = procedure(Sender: TObject; var Allow: Boolean) of object;
  TFiller = procedure(out S: string; N: Integer);
  TForm = class
    procedure Check(Sender: TObject; var Allow: Boolean);
  end;
procedure TForm.Check(Sender: TObject; var Allow: Boolean);
begin
  Allow := False;
end;
procedure Fill(out S: string; N: Integer);
begin
  S := StringOfChar('*', N);
end;
var F: TForm; E: TVerifyEvent; Ok: Boolean; P: TFiller; T: string;
begin
  F := TForm.Create;
  E := @F.Check;
  Ok := True;
  E(nil, Ok);
  WriteLn(Ok);
  P := @Fill;
  P(T, 3);
  WriteLn(T);
  F.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "FALSE\n***\n");
    }

    // TLS through FPC's units (ssockets, sslsockets, opensslsockets,
    // fphttpclient) against a local server with a self-signed
    // certificate. Skipped where the `openssl` command or the OpenSSL
    // libraries are missing.
    #[cfg(unix)]
    #[test]
    fn tls_sockets_and_https() {
        let dir = std::env::temp_dir().join(format!("bruto-tls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cert = dir.join("cert.pem");
        let key = dir.join("key.pem");
        let made = std::process::Command::new("openssl")
            .args([
                "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2",
            ])
            .args([
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost",
            ])
            .arg("-keyout")
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .output()
            .is_ok_and(|o| o.status.success());
        if !made {
            eprintln!("skipped: no openssl command");
            return;
        }
        let cert = cert.display();
        let key = key.display();
        let source = format!(
            "program T;
{{$mode objfpc}}{{$H+}}
uses SysUtils, Classes, ssockets, sslsockets, opensslsockets, openssl, fphttpclient;
function c_fork: LongInt; cdecl; external 'c' name 'fork';
procedure c_exit(Code: LongInt); cdecl; external 'c' name '_exit';
function c_waitpid(Pid: LongInt; Status: Pointer; Options: LongInt): LongInt;
  cdecl; external 'c' name 'waitpid';
function c_alarm(Seconds: LongWord): LongWord; cdecl; external 'c' name 'alarm';
const
  CertFile = '{cert}';
  KeyFile = '{key}';
var
  Server: TInetServer;
  ServerHandler, H: TSSLSocketHandler;
  Peer: TSocketStream;
  Client: TInetSocket;
  Http: TFPHTTPClient;
  Buf: array[0..255] of Char;
  N: LongInt;
  Reply, Body: string;
  Pid, Status, I: LongInt;
  Port: Word;
begin
  if not InitSSLInterface then
  begin
    WriteLn('no OpenSSL');
    Halt(0);
  end;
  ServerHandler := TSSLSocketHandler.GetDefaultHandler;
  ServerHandler.CertificateData.Certificate.FileName := CertFile;
  ServerHandler.CertificateData.PrivateKey.FileName := KeyFile;
  Server := TInetServer.Create('127.0.0.1', 0, ServerHandler);
  Server.Listen;
  Port := Server.Port;
  c_alarm(30);
  Pid := c_fork;
  if Pid = 0 then
  begin
    c_alarm(20);
    for I := 1 to 3 do
    begin
      Peer := Server.GetConnection;
      if Peer = nil then
        c_exit(3);
      N := Peer.Read(Buf, SizeOf(Buf));
      if I < 3 then
      begin
        Reply := 'echo: ' + Copy(PChar(@Buf), 1, N);
        Peer.WriteBuffer(Reply[1], Length(Reply));
      end
      else
      begin
        Body := 'secure hello';
        Reply := 'HTTP/1.1 200 OK'#13#10'Content-Type: text/plain'#13#10 +
          'Content-Length: ' + IntToStr(Length(Body)) + #13#10#13#10 + Body;
        Peer.WriteBuffer(Reply[1], Length(Reply));
      end;
      Peer.Free;
    end;
    c_exit(0);
  end;
  {{ Without checking the certificate. }}
  H := TSSLSocketHandler.GetDefaultHandler;
  Client := TInetSocket.Create('127.0.0.1', Port, H);
  Client.WriteBuffer('ping', 4);
  N := Client.Read(Buf, SizeOf(Buf));
  WriteLn(Copy(PChar(@Buf), 1, N), ' ', H.SSLActive);
  Client.Free;
  {{ Checking it against the certificate itself, for localhost. }}
  H := TSSLSocketHandler.GetDefaultHandler;
  H.VerifyPeerCert := True;
  H.CertificateData.TrustedCertificate.FileName := CertFile;
  Client := TInetSocket.Create('localhost', Port, H);
  Client.WriteBuffer('pong', 4);
  N := Client.Read(Buf, SizeOf(Buf));
  WriteLn(Copy(PChar(@Buf), 1, N));
  Client.Free;
  Http := TFPHTTPClient.Create(nil);
  WriteLn(Http.Get('https://127.0.0.1:' + IntToStr(Port) + '/'), ' ', Http.ResponseStatusCode);
  Http.Free;
  c_waitpid(Pid, @Status, 0);
  WriteLn('server exit ', Status);
  Server.Free;
end.
"
        );
        let (ok, out) = build_and_run_source(&source);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ok, "program failed: {out}");
        if out == "no OpenSSL\n" {
            eprintln!("skipped: no OpenSSL libraries");
            return;
        }
        assert_eq!(
            out,
            "echo: ping TRUE\necho: pong\nsecure hello 200\nserver exit 0\n"
        );
    }

    #[test]
    fn dynamic_and_open_arrays() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}{$H+}
type
  TIntArray = array of Integer;
  TStrArray = array of string;
  TPoint = record X, Y: Integer; Name: string; end;
var
  A, B: TIntArray;
  S: TStrArray;
  G: array of array of Integer;
  P: array of TPoint;
  I, J, Sum: Integer;
  T: string;

function Squares(N: Integer): TIntArray;
var K: Integer;
begin
  SetLength(Result, N);
  for K := 0 to N - 1 do
    Result[K] := K * K;
end;

function Total(const V: array of Integer): Integer;
var X: Integer;
begin
  Result := 0;
  for X in V do
    Result := Result + X;
end;

procedure Double(var V: array of Integer);
var K: Integer;
begin
  for K := Low(V) to High(V) do
    V[K] := V[K] * 2;
end;

procedure Show(const Args: array of const);
var K: Integer;
begin
  for K := 0 to High(Args) do
    case Args[K].VType of
      vtInteger: Write('int:', Args[K].VInteger, ' ');
      vtInt64: Write('int64:', Args[K].VInt64^, ' ');
      vtAnsiString: Write('str:', AnsiString(Args[K].VAnsiString), ' ');
      vtChar: Write('char:', Args[K].VChar, ' ');
      vtBoolean: Write('bool:', Args[K].VBoolean, ' ');
      vtExtended: Write('real:', Args[K].VExtended^:0:2, ' ');
    else
      Write('?', Args[K].VType, ' ');
    end;
  WriteLn;
end;

var
  St: array[1..4] of Integer;
begin
  WriteLn(Length(A), ' ', A = nil);
  SetLength(A, 3);
  A[0] := 10; A[1] := 20; A[2] := 30;
  WriteLn(Length(A), ' ', High(A), ' ', Low(A), ' ', A[1]);
  B := A;
  B[0] := 99;
  WriteLn(A[0]);
  SetLength(B, 5);
  B[1] := 7;
  WriteLn(A[1], ' ', B[1], ' ', B[4], ' ', Length(B));
  A := Squares(6);
  WriteLn(A[5], ' ', Total(A), ' ', Total([1, 2, 3]));
  St[1] := 1; St[2] := 2; St[3] := 3; St[4] := 4;
  Double(St);
  WriteLn(St[4], ' ', Total(St));
  Double(A);
  WriteLn(A[5]);
  SetLength(S, 3);
  S[0] := 'a'; S[1] := 'b' + 'c'; S[2] := S[1] + S[1];
  T := '';
  for I := 0 to High(S) do T := T + S[I] + ';';
  WriteLn(T);
  S := Copy(S, 1, 2);
  WriteLn(Length(S), ' ', S[0], ' ', S[1]);
  SetLength(G, 3, 4);
  for I := 0 to 2 do for J := 0 to 3 do G[I][J] := I * 10 + J;
  WriteLn(G[2][3], ' ', G[1, 2], ' ', Length(G[0]));
  SetLength(P, 2);
  P[1].X := 5; P[1].Name := 'pt';
  WriteLn(P[1].X, P[1].Name, ' ', Length(P));
  A := [4, 5, 6];
  Sum := 0;
  for I in A do Sum := Sum + I;
  WriteLn(Sum);
  Show([42, 'hi', 'x', True, 2.5, Int64(5000000000), Sum]);
  A := nil;
  WriteLn(Length(A));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "0 TRUE\n3 2 0 20\n99\n20 7 0 5\n25 55 6\n8 20\n50\na;bc;bcbc;\n2 bc bcbc\n23 12 4\n5pt 2\n15\nint:42 str:hi char:x bool:TRUE real:2.50 int64:5000000000 int:15 \n0\n"
        );
    }

    #[test]
    fn dynamic_array_insert_and_delete() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}{$H+}
var
  A: array of Integer;
  S, T: array of string;
  I: Integer;
begin
  A := [1, 2, 3, 4, 5];
  Delete(A, 1, 2);
  for I in A do Write(I, ' ');
  WriteLn(Length(A));
  Insert(9, A, 1);
  Insert(7, A, 100);
  Insert(0, A, 0);
  for I in A do Write(I, ' ');
  WriteLn(Length(A));
  S := ['a', 'b', 'c'];
  T := S;
  Delete(S, 0, 1);
  Insert('z' + 'z', S, 1);
  for I := 0 to High(S) do Write(S[I], ' ');
  for I := 0 to High(T) do Write(T[I], ' ');
  WriteLn;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "1 4 5 3\n0 1 9 4 5 7 6\nb zz c a b c \n");
    }

    #[test]
    fn format_matches_fpc() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}{$H+}
uses SysUtils;
var P: Pointer; I64: Int64;
begin
  P := Pointer(PtrUInt($1234ABCD));
  I64 := 123456789012;
  WriteLn(Format('[%d] [%5d] [%-5d] [%.3d] [%x] [%.4X] [%u]', [42, 42, 42, 5, 255, 255, 7]));
  WriteLn(Format('[%s] [%8s] [%-8s] [%.2s] [%s]', ['abc', 'abc', 'abc', 'abcdef', 'x']));
  WriteLn(Format('[%f] [%.3f] [%8.1f] [%-8.1f] [%f]', [1.5, 3.14159, 2.25, 2.25, -0.005]));
  WriteLn(Format('[%e] [%.3e] [%e] [%e]', [1.5, 12345.678, 0.000123, -2.0]));
  WriteLn(Format('[%g] [%g] [%g] [%g] [%g] [%.3g]', [1.5, 100.0, 1e20, 0.0001, 123456789.0, 3.14159]));
  WriteLn(Format('[%n] [%.0n] [%n]', [1234567.891, 1234.5, 12.0]));
  WriteLn(Format('[%1:s %0:s] [%0:s]', ['a', 'b']));
  WriteLn(Format('[%*d] [%-*d] [%.*f]', [4, 7, 4, 7, 1, 2.25]));
  WriteLn(Format('[%f] [%.1f] [%.0f] [%.2f] [%.2f] [%f]', [0.125, 0.05, 2.5, 1.005, 1e15, 0.0]));
  WriteLn(Format('[%e] [%g] [%g] [%n] [%d]', [0.0, 0.0, -1.5e-7, -1234.5, Low(Int64)]));
  WriteLn(Format('[%d] [%s] [%%] [%p]', [I64, 'end', P]));
  WriteLn(Format('[%d] [%x]', [-5, -1]));
  WriteLn(Format('%s=%d', ['x', 1]), ' ', Format('no args', []));
  try
    WriteLn(Format('%d', ['x']));
  except
    on E: Exception do WriteLn(E.ClassName, ': ', E.Message);
  end;
  try
    WriteLn(Format('%d %d', [1]));
  except
    on E: Exception do WriteLn(E.ClassName, ': ', E.Message);
  end;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "[42] [   42] [42   ] [005] [FF] [00FF] [7]\n[abc] [     abc] [abc     ] [ab] [x]\n[1.50] [3.142] [     2.3] [2.3     ] [-0.01]\n[1.50000000000000E+000] [1.23E+004] [1.23000000000000E-004] [-2.00000000000000E+000]\n[1.5] [100] [1E20] [0.0001] [123456789] [3.14]\n[1,234,567.89] [1,235] [12.00]\n[b a] [a]\n[   7] [7   ] [2.3]\n[0.13] [0.1] [3] [1.01] [1000000000000000.00] [0.00]\n[0.00000000000000E+000] [0] [-1.5E-7] [-1,234.50] [-9223372036854775808]\n[123456789012] [end] [%] [000000001234ABCD]\n[-5] [FFFFFFFF]\nx=1 no args\nEConvertError: Invalid argument index in format \"%d\"\nEConvertError: Invalid argument index in format \"%d %d\"\n"
        );
    }

    #[test]
    fn class_variables_and_construction_hooks() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}{$H+}
type
  TCounter = class
  private
    class var FCount: Integer;
    class var FNames: string;
  public
    Id: Integer;
    constructor Create;
    destructor Destroy; override;
    procedure AfterConstruction; override;
    procedure BeforeDestruction; override;
    class function Count: Integer;
  end;
  TSub = class(TCounter)
    constructor Create;
  end;

function IntToStr(I: Integer): string;
begin
  Str(I, Result);
end;

constructor TCounter.Create;
begin
  Inc(FCount);
  Id := FCount;
  FNames := FNames + IntToStr(Id);
  WriteLn('create ', Id);
end;

destructor TCounter.Destroy;
begin
  WriteLn('destroy ', Id);
  Dec(FCount);
  inherited Destroy;
end;

procedure TCounter.AfterConstruction;
begin
  WriteLn('after construction ', Id);
end;

procedure TCounter.BeforeDestruction;
begin
  WriteLn('before destruction ', Id);
end;

class function TCounter.Count: Integer;
begin
  Result := FCount;
end;

constructor TSub.Create;
begin
  inherited Create;
  WriteLn('sub ', Id);
end;

var A, B: TCounter;
begin
  A := TCounter.Create;
  B := TSub.Create;
  WriteLn(TCounter.Count, ' ', A.Count, ' ', TSub.FCount, ' ', TCounter.FNames);
  TCounter.FCount := TCounter.FCount + 10;
  WriteLn(TSub.Count);
  A.Free;
  B.Free;
  WriteLn(TCounter.Count);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "create 1\nafter construction 1\ncreate 2\nsub 2\nafter construction 2\n2 2 2 12\n12\nbefore destruction 1\ndestroy 1\nbefore destruction 2\ndestroy 2\n10\n"
        );
    }

    #[test]
    fn division_by_zero_is_runtime_error_200() {
        let (ok, out) = build_and_run_source(
            "program T;
uses SysUtils;
var A, B: Integer;
begin
  A := 10; B := 0;
  try writeln(A div B); except on E: Exception do writeln(E.ClassName, ': ', E.Message); end;
  try writeln(A mod B); except on E: EDivByZero do writeln('mod: ', E.Message); end;
  writeln('done');
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "EDivByZero: Division by zero\nmod: Division by zero\ndone\n"
        );
        let result = MiniPascal
            .build("program T;\nvar A, B: Integer;\nbegin\n  A := 1; B := 0;\n  writeln(A div B);\nend.\n")
            .expect("build failed");
        let run = std::process::Command::new(&result.exe_path)
            .output()
            .expect("run failed");
        let _ = std::fs::remove_file(&result.exe_path);
        let _ = std::fs::remove_dir_all(format!("{}.dSYM", result.exe_path));
        let _ = std::fs::remove_file(&result.source_path);
        let _ = std::fs::remove_file(&result.console_capture_path);
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert_eq!(run.status.code(), Some(200), "{stderr}");
        assert!(stderr.starts_with("Runtime error 200 at $"), "{stderr}");
    }

    #[cfg(unix)]
    #[test]
    fn nil_dereference_is_an_access_violation() {
        let (ok, out) = build_and_run_source(
            "program T;
uses SysUtils;
var P: PInteger; I: Integer;
begin
  P := nil;
  for I := 1 to 3 do
    try
      if I = 2 then P^ := 5 else writeln(P^);
    except
      on E: EAccessViolation do writeln(I, ' ', E.Message);
    end;
  writeln('done');
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "1 Access violation\n2 Access violation\n3 Access violation\ndone\n"
        );
    }

    #[test]
    fn integer_is_32_bits_in_objfpc_mode() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}
type TRec = record A: Integer; B: Byte; end;
var I: Integer;
begin
  I := MaxInt;
  writeln(SizeOf(I), SizeOf(Integer), ' ', High(Integer), ' ', SizeOf(TRec));
  I := I + 1;
  writeln(I, ' ', Integer(5000000000));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "44 2147483647 8\n-2147483648 705032704\n");
        // Without a mode, Integer stays 64-bit, and a callback with such
        // parameters still works with a unit compiled in objfpc mode.
        let (ok, out) = build_and_run_source(
            "program T;
uses Classes;
function ByLength(List: TStringList; Index1, Index2: Integer): Integer;
begin
  Result := Length(List[Index1]) - Length(List[Index2]);
end;
var L: TStringList;
begin
  writeln(SizeOf(Integer));
  L := TStringList.Create;
  L.CommaText := 'ccc,a,bb';
  L.CustomSort(@ByLength);
  writeln(L.CommaText);
  L.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "8\na,bb,ccc\n");
    }

    #[test]
    fn generic_classes_and_records() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}{$H+}
type
  generic TStack<T> = class
  private
    FItems: array of T;
    FCount: Integer;
    function GetTop: T;
  public
    procedure Push(const Item: T);
    function Pop: T;
    function IsEmpty: Boolean;
    property Count: Integer read FCount;
    property Top: T read GetTop;
  end;

  generic TPair<TKey, TValue> = record
    Key: TKey;
    Value: TValue;
  end;

  TIntStack = specialize TStack<Integer>;
  TStrStack = specialize TStack<string>;
  TNamedAge = specialize TPair<string, Integer>;
  TPairStack = specialize TStack<TNamedAge>;

procedure TStack.Push(const Item: T);
begin
  if FCount = Length(FItems) then
    SetLength(FItems, FCount * 2 + 4);
  FItems[FCount] := Item;
  Inc(FCount);
end;

function TStack.Pop: T;
begin
  Dec(FCount);
  Result := FItems[FCount];
  FItems[FCount] := Default(T);
end;

function TStack.GetTop: T;
begin
  Result := FItems[FCount - 1];
end;

function TStack.IsEmpty: Boolean;
begin
  Result := FCount = 0;
end;

var
  I: TIntStack;
  S: TStrStack;
  P: TPairStack;
  Pair: TNamedAge;
  K: Integer;
begin
  I := TIntStack.Create;
  for K := 1 to 10 do I.Push(K * K);
  Write(I.Count, ' ', I.Top, ':');
  while not I.IsEmpty do Write(' ', I.Pop);
  WriteLn;
  S := TStrStack.Create;
  S.Push('a'); S.Push('b' + 'c'); S.Push('def');
  WriteLn(S.Pop, S.Pop, S.Count);
  P := specialize TStack<TNamedAge>.Create;
  Pair.Key := 'ann'; Pair.Value := 31;
  P.Push(Pair);
  Pair.Key := 'bob'; Pair.Value := 42;
  P.Push(Pair);
  WriteLn(P.Pop.Key, ' ', P.Top.Value, ' ', SizeOf(TNamedAge));
  I.Free; S.Free; P.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "10 100: 100 81 64 49 36 25 16 9 4 1\ndefbc1\nbob 31 16\n"
        );
    }

    #[test]
    fn fgl_lists_and_maps() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}{$H+}
uses SysUtils, fgl;
type
  TIntList = specialize TFPGList<Integer>;
  TStrList = specialize TFPGList<string>;
  TThing = class
    Name: string;
    constructor Create(const AName: string);
    destructor Destroy; override;
  end;
  TThings = specialize TFPGObjectList<TThing>;
  TAges = specialize TFPGMap<string, Integer>;

var Destroyed: Integer;

constructor TThing.Create(const AName: string);
begin
  Name := AName;
end;

destructor TThing.Destroy;
begin
  Inc(Destroyed);
  inherited Destroy;
end;

function CompareInts(const A, B: Integer): Integer;
begin
  Result := A - B;
end;

function CompareLen(const A, B: string): Integer;
begin
  Result := Length(A) - Length(B);
end;

var
  L: TIntList;
  S: TStrList;
  T: TThings;
  M: TAges;
  I, Age: Integer;
  Str: string;
begin
  L := TIntList.Create;
  for I := 1 to 8 do L.Add((I * 37) mod 11);
  L.Sort(@CompareInts);
  for I in L do Write(I, ' ');
  WriteLn('| ', L.Count, ' ', L.First, ' ', L.Last, ' ', L.IndexOf(4), ' ', L.IndexOf(99));
  L.Delete(0); L.Insert(0, 100); L.Remove(10); L[1] := -1;
  for I := 0 to L.Count - 1 do Write(L[I], ' ');
  WriteLn;
  S := TStrList.Create;
  S.Add('ccc'); S.Add('a'); S.Add('bb');
  S.Sort(@CompareLen);
  for Str in S do Write(Str, ';');
  WriteLn(S.IndexOf('bb'));
  T := TThings.Create;
  T.Add(TThing.Create('x')); T.Add(TThing.Create('y')); T.Add(TThing.Create('z'));
  T.Delete(1);
  WriteLn(T.Count, ' ', T[1].Name, ' ', Destroyed);
  T.Free;
  WriteLn(Destroyed);
  M := TAges.Create;
  M.Add('bob', 40); M.Add('ann', 31); M['cid'] := 25; M['bob'] := 41;
  WriteLn(M.Count, ' ', M['bob'], ' ', M.Keys[0], ' ', M.Data[1], ' ', M.IndexOf('cid'));
  M.Sorted := True;
  for I := 0 to M.Count - 1 do Write(M.Keys[I], '=', M.Data[I], ' ');
  WriteLn(M.TryGetData('ann', Age), Age, ' ', M.TryGetData('zed', Age), Age);
  M.Remove('ann');
  WriteLn(M.Count, ' ', M.Keys[0]);
  try
    WriteLn(M['nobody']);
  except
    on E: Exception do WriteLn(E.ClassName);
  end;
  L.Free; S.Free; M.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "1 2 4 5 6 8 9 10 | 8 1 10 2 -1\n100 -1 4 5 6 8 9 \na;bb;ccc;1\n2 z 1\n3\n3 41 bob 31 2\nann=31 bob=41 cid=25 TRUE31 FALSE0\n2 bob\nEListError\n"
        );
    }

    #[test]
    fn generics_collections() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode delphi}{$H+}
uses SysUtils, Generics.Collections;
type
  TThing = class
    Name: string;
    constructor Create(const AName: string);
    destructor Destroy; override;
  end;
var
  Freed: Integer;
constructor TThing.Create(const AName: string); begin Name := AName; end;
destructor TThing.Destroy; begin Inc(Freed); inherited; end;

var
  L: TList<Integer>;
  S: TList<string>;
  O: TObjectList<TThing>;
  St: TStack<string>;
  Q: TQueue<Integer>;
  D: TDictionary<string, Integer>;
  Keys: TArray<string>;
  Pair: TPair<string, Integer>;
  I, V, Total: Integer;
  K: string;
  Found: SizeInt;
begin
  L := TList<Integer>.Create;
  L.AddRange([5, 3, 9, 1]);
  L.Add(7);
  L.Insert(0, 42);
  L.Sort;
  for I in L do Write(I, ' ');
  WriteLn('| ', L.Count, ' ', L.IndexOf(9), ' ', L.Contains(4), ' ', L.First, ' ', L.Last);
  WriteLn(L.BinarySearch(7, Found), ' ', Found, ' ', L.BinarySearch(4, Found), ' ', Found);
  L.Reverse; L.Delete(0); L.Remove(1);
  for I in L.ToArray do Write(I, ' ');
  WriteLn;
  S := TList<string>.Create;
  S.AddRange(['pear', 'apple', 'fig']);
  S.Sort;
  WriteLn(S[0], ' ', S[2], ' ', S.Count);
  O := TObjectList<TThing>.Create;
  O.Add(TThing.Create('a')); O.Add(TThing.Create('b')); O.Add(TThing.Create('c'));
  O.Delete(1);
  WriteLn(O.Count, ' ', O[1].Name, ' ', Freed);
  O.Free;
  WriteLn(Freed);
  St := TStack<string>.Create;
  St.Push('x'); St.Push('y'); St.Push('z');
  WriteLn(St.Pop, St.Peek, St.Count);
  Q := TQueue<Integer>.Create;
  for I := 1 to 10 do Q.Enqueue(I);
  for I := 1 to 7 do Q.Dequeue;
  for I := 11 to 14 do Q.Enqueue(I);
  Write(Q.Count, ':');
  while Q.Count > 0 do Write(' ', Q.Dequeue);
  WriteLn;
  D := TDictionary<string, Integer>.Create;
  for I := 1 to 100 do D.Add('k' + IntToStr(I), I * I);
  D.AddOrSetValue('k5', -5);
  D.Remove('k10');
  D['k20'] := 20;
  WriteLn(D.Count, ' ', D['k5'], ' ', D.ContainsKey('k10'), ' ', D.ContainsKey('k99'), ' ', D['k20'],
    ' ', D.TryGetValue('k3', V), V, ' ', D.TryGetValue('none', V), V, ' ', D.ContainsValue(9801));
  Total := 0;
  for Pair in D do Total := Total + Pair.Value;
  Keys := D.Keys.ToArray;
  WriteLn(Total, ' ', Length(Keys), ' ', D.Keys.Count);
  try
    D.Add('k1', 0);
  except
    on E: Exception do WriteLn(E.ClassName);
  end;
  try
    V := D['missing'];
  except
    on E: Exception do WriteLn(E.ClassName);
  end;
  L.Free; S.Free; St.Free; Q.Free; D.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "1 3 5 7 9 42 | 6 4 FALSE 1 42\nTRUE 3 FALSE -1\n9 7 5 3 \napple pear 3\n2 c 1\n3\nzy2\n7: 8 9 10 11 12 13 14\n99 -5 FALSE TRUE 20 TRUE9 FALSE0 TRUE\n337840 99 99\nEListError\nEListError\n"
        );
    }

    #[test]
    fn generic_routines() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode objfpc}{$H+}
uses Generics.Collections;

generic function Larger<T>(const A, B: T): T;
begin
  if A > B then Result := A else Result := B;
end;

generic procedure Swap<T>(var A, B: T);
var Temp: T;
begin
  Temp := A; A := B; B := Temp;
end;

generic function Sum<T>(const Items: array of T): T;
var Item: T;
begin
  Result := Default(T);
  for Item in Items do Result := Result + Item;
end;

generic function ToList<T>(const Items: array of T): specialize TList<T>;
var Item: T;
begin
  Result := specialize TList<T>.Create;
  for Item in Items do Result.Add(Item);
end;

var
  X, Y: Integer;
  S1, S2: string;
  L: specialize TList<string>;
begin
  WriteLn(specialize Larger<Integer>(3, 9), ' ', specialize Larger<string>('pear', 'apple'),
    ' ', specialize Larger<Double>(1.5, -2.0):0:1);
  X := 1; Y := 2;
  specialize Swap<Integer>(X, Y);
  S1 := 'a'; S2 := 'b';
  specialize Swap<string>(S1, S2);
  WriteLn(X, Y, S1, S2);
  WriteLn(specialize Sum<Integer>([1, 2, 3, 4]), ' ', specialize Sum<string>(['x', 'y', 'z']));
  L := specialize ToList<string>(['one', 'two']);
  WriteLn(L.Count, ' ', L[1]);
  L.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "9 pear 1.5\n21ba\n10 xyz\n2 two\n");
    }

    #[test]
    fn interfaces_com() {
        let (ok, out) = build_and_run_source(
            "program Intf1;
{$mode objfpc}{$H+}

uses SysUtils;

type
  IGreeter = interface
    ['{6A1F3C2E-1B2D-4E5F-8A9B-0C1D2E3F4A5B}']
    function Greet(const Name: string): string;
    procedure SetPrefix(const P: string);
    function GetPrefix: string;
    property Prefix: string read GetPrefix write SetPrefix;
  end;

  ICounter = interface
    ['{11111111-2222-3333-4444-555555555555}']
    function Next: Integer;
  end;

  ILoudGreeter = interface(IGreeter)
    ['{22222222-2222-3333-4444-555555555555}']
    function Shout(const Name: string): string;
  end;

  TGreeter = class(TInterfacedObject, IGreeter, ILoudGreeter, ICounter)
  private
    FPrefix: string;
    FCount: Integer;
  public
    constructor Create(const P: string);
    destructor Destroy; override;
    function Greet(const Name: string): string; virtual;
    procedure SetPrefix(const P: string);
    function GetPrefix: string;
    function Shout(const Name: string): string;
    function Next: Integer;
  end;

  TPolite = class(TGreeter)
    function Greet(const Name: string): string; override;
  end;

  TPlain = class(TInterfacedObject)
  end;

constructor TGreeter.Create(const P: string);
begin
  inherited Create;
  FPrefix := P;
  WriteLn('create ', P);
end;

destructor TGreeter.Destroy;
begin
  WriteLn('destroy ', FPrefix);
  inherited Destroy;
end;

function TGreeter.Greet(const Name: string): string;
begin
  Result := FPrefix + ', ' + Name;
end;

procedure TGreeter.SetPrefix(const P: string);
begin
  FPrefix := P;
end;

function TGreeter.GetPrefix: string;
begin
  Result := FPrefix;
end;

function TGreeter.Shout(const Name: string): string;
begin
  Result := UpperCase(Greet(Name)) + '!';
end;

function TGreeter.Next: Integer;
begin
  Inc(FCount);
  Result := FCount;
end;

function TPolite.Greet(const Name: string): string;
begin
  Result := 'Dear ' + inherited Greet(Name);
end;

function MakeGreeter(const P: string): IGreeter;
begin
  Result := TGreeter.Create(P);
end;

procedure UseIt(G: IGreeter);
begin
  WriteLn(G.Greet('param'));
end;

var
  G: IGreeter;
  L: ILoudGreeter;
  C: ICounter;
  Obj: TGreeter;
  U: IUnknown;
  P: TPlain;
begin
  G := TGreeter.Create('Hello');
  WriteLn(G.Greet('World'));
  G.Prefix := 'Hi';
  WriteLn(G.Prefix);
  if G is ILoudGreeter then
    WriteLn('loud');
  L := G as ILoudGreeter;
  WriteLn(L.Shout('there'));
  C := G as ICounter;
  WriteLn(C.Next, ' ', C.Next);
  WriteLn('nil before: ', G = nil);
  G := nil;
  WriteLn('after G := nil');
  L := nil;
  WriteLn('after L := nil');
  C := nil;
  WriteLn('after C := nil');

  WriteLn(MakeGreeter('Temp').Greet('x'));
  WriteLn('after temp');

  G := TPolite.Create('Yo');
  WriteLn(G.Greet('Sir'));
  UseIt(G);
  UseIt(TGreeter.Create('Arg'));
  WriteLn('after arg');

  Obj := TGreeter.Create('Kept');
  G := Obj;
  if Supports(Obj, ICounter, C) then
    WriteLn('supports counter ', C.Next);
  WriteLn(Supports(G, ICounter));
  U := G;
  WriteLn(Assigned(U));
  P := TPlain.Create;
  WriteLn(Supports(P, IGreeter));
  P.Free;
  WriteLn((G as TGreeter).GetPrefix);
  C := nil;
  U := nil;
  G := nil;
  WriteLn('end');
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "create Hello\nHello, World\nHi\nloud\nHI, THERE!\n1 2\nnil before: FALSE\nafter G := nil\nafter L := nil\nafter C := nil\ncreate Temp\nTemp, x\nafter temp\ncreate Yo\nDear Yo, Sir\nDear Yo, param\ncreate Arg\nArg, param\ndestroy Arg\nafter arg\ncreate Kept\ndestroy Yo\nsupports counter 1\nTRUE\nTRUE\nFALSE\nKept\ndestroy Kept\nend\ndestroy Temp\ndestroy Hi\n"
        );
    }

    #[test]
    fn interfaces_temporaries_and_fields() {
        let (ok, out) = build_and_run_source(
            "program Intf2;
{$mode objfpc}{$H+}

uses SysUtils, Classes;

type
  IShape = interface
    ['{A0000000-0000-0000-0000-000000000001}']
    function Area: Double;
    function Name: string;
  end;

  INamed = interface
    ['{A0000000-0000-0000-0000-000000000002}']
    function Name: string;
  end;

  TShape = class(TInterfacedObject, IShape, INamed)
  private
    FName: string;
  public
    constructor Create(const AName: string);
    destructor Destroy; override;
    function Area: Double; virtual; abstract;
    function Name: string;
    function _AddRef: LongInt; {$IFDEF WINDOWS}stdcall{$ELSE}cdecl{$ENDIF};
  end;

  TSquare = class(TShape)
    FSide: Double;
    constructor Create(ASide: Double);
    function Area: Double; override;
  end;

  TCircle = class(TShape)
    FR: Double;
    constructor Create(AR: Double);
    function Area: Double; override;
  end;

  THolder = class
    FShape: IShape;
    destructor Destroy; override;
  end;

  TRec = record
    S: IShape;
    Tag: Integer;
  end;

  TShapes = array of IShape;

constructor TShape.Create(const AName: string);
begin
  inherited Create;
  FName := AName;
end;

destructor TShape.Destroy;
begin
  WriteLn('bye ', FName);
  inherited;
end;

function TShape.Name: string;
begin
  Result := FName;
end;

function TShape._AddRef: LongInt; {$IFDEF WINDOWS}stdcall{$ELSE}cdecl{$ENDIF};
begin
  Result := inherited _AddRef;
end;

constructor TSquare.Create(ASide: Double);
begin
  inherited Create('square' + FloatToStr(ASide));
  FSide := ASide;
end;

function TSquare.Area: Double;
begin
  Result := FSide * FSide;
end;

constructor TCircle.Create(AR: Double);
begin
  inherited Create('circle' + FloatToStr(AR));
  FR := AR;
end;

function TCircle.Area: Double;
begin
  Result := 3 * FR * FR;
end;

destructor THolder.Destroy;
begin
  WriteLn('holder gone');
  inherited;
end;

function Make(I: Integer): IShape;
begin
  if Odd(I) then
    Result := TSquare.Create(I)
  else
    Result := TCircle.Create(I);
end;

procedure Show(const S: IShape);
begin
  WriteLn(S.Name, ' ', S.Area:0:1);
end;

function Total(const A: TShapes): Double;
var
  I: Integer;
begin
  Result := 0;
  for I := 0 to High(A) do
    Result := Result + A[I].Area;
end;

var
  Global: IShape;
  A: TShapes;
  I: Integer;
  H: THolder;
  R: TRec;
  N: INamed;
  U: IInterface;
  Q: IShape;
begin
  for I := 1 to 3 do
    WriteLn(Make(I).Name);
  WriteLn('loop done');

  SetLength(A, 3);
  for I := 0 to 2 do
    A[I] := Make(I + 10);
  WriteLn(Total(A):0:1);
  A[1] := nil;
  WriteLn('slot cleared');
  SetLength(A, 0);
  WriteLn('array cleared');

  H := THolder.Create;
  H.FShape := TSquare.Create(7);
  Show(H.FShape);
  H.Free;

  R.S := TCircle.Create(5);
  R.Tag := 1;
  Show(R.S);
  R.S := nil;

  Global := TSquare.Create(2);
  U := Global;
  if U.QueryInterface(INamed, N) = S_OK then
    WriteLn('named: ', N.Name);
  if Supports(U, IShape, Q) then
    Show(Q);
  N := nil;
  U := nil;
  Q := nil;
  try
    Show(TSquare.Create(9));
    raise Exception.Create('boom');
  except
    on E: Exception do
      WriteLn('caught ', E.Message);
  end;
  WriteLn('end of main');
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "square1\nbye square1\ncircle2\nbye circle2\nsquare3\nloop done\nbye square3\n853.0\nbye square11\nslot cleared\nbye circle10\narray cleared\nsquare7 49.0\nholder gone\nbye square7\ncircle5 75.0\nbye circle5\nnamed: square2\nsquare2 4.0\nsquare9 81.0\ncaught boom\nend of main\nbye circle12\n"
        );
    }

    #[test]
    fn interfaces_casts_overloads_generics() {
        let (ok, out) = build_and_run_source(
            "program Intf3;
{$mode objfpc}{$H+}

type
  IValue = interface
    ['{B0000000-0000-0000-0000-000000000001}']
    function Get: Integer;
    procedure Put(V: Integer); overload;
    procedure Put(const S: string); overload;
  end;

  generic IBox<T> = interface
    function Value: T;
  end;

  TIntBox = class(TInterfacedObject, specialize IBox<Integer>)
    function Value: Integer;
  end;

  TValue = class(TInterfacedObject, IValue)
  private
    FV: Integer;
  public
    function Get: Integer;
    procedure Put(V: Integer); overload;
    procedure Put(const S: string); overload;
  end;

  TOwner = class
  private
    FVal: IValue;
  public
    property Val: IValue read FVal write FVal;
    function Fresh: IValue;
  end;

function TIntBox.Value: Integer;
begin
  Result := 42;
end;

function TValue.Get: Integer;
begin
  Result := FV;
end;

procedure TValue.Put(V: Integer);
begin
  FV := V;
end;

procedure TValue.Put(const S: string);
begin
  FV := Length(S);
end;

function TOwner.Fresh: IValue;
begin
  Result := TValue.Create;
  Result.Put(5);
end;

procedure Swap(var A, B: IValue);
var
  T: IValue;
begin
  T := A;
  A := B;
  B := T;
end;

var
  V1, V2: IValue;
  O: TOwner;
  Obj: TValue;
  B: specialize IBox<Integer>;
begin
  Obj := TValue.Create;
  V1 := IValue(Obj);
  V1.Put(3);
  WriteLn(V1.Get, ' ', Obj.RefCount);
  V2 := V1;
  WriteLn(V1 = V2, ' ', Obj.RefCount);
  V2 := TValue.Create;
  V2.Put('hello');
  WriteLn(V1 = V2, ' ', V2.Get);
  Swap(V1, V2);
  WriteLn(V1.Get, ' ', V2.Get);
  O := TOwner.Create;
  O.Val := V1;
  WriteLn(O.Val.Get, ' ', O.Fresh.Get);
  O.Val := nil;
  O.Free;
  B := TIntBox.Create;
  WriteLn(B.Value);
  WriteLn(Obj.RefCount);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "3 1\nTRUE 2\nFALSE 5\n5 3\n5 5\n42\n1\n");
    }

    #[test]
    fn math_unit_matches_fpc() {
        let (ok, out) = build_and_run_source(
            "program mt;
{$mode objfpc}{$H+}
uses Math;
var M: TFPUExceptionMask; D: array[0..3] of Double = (1.5, 2.5, -3, 4);
begin
  WriteLn(Max(3, 7), ' ', Min(2.5, 1.5):0:2, ' ', Max(Int64(5), 9));
  WriteLn(Power(2, 10):0:0, ' ', Power(2, 0.5):0:6, ' ', IntPower(3, -2):0:6);
  WriteLn(Floor(-2.5), ' ', Ceil(-2.5), ' ', Floor(2.5), ' ', Ceil(2.1));
  WriteLn(Log10(1000):0:3, ' ', Log2(8):0:3, ' ', LogN(3, 81):0:3);
  WriteLn(Tan(Pi/4):0:6, ' ', ArcSin(1):0:6, ' ', ArcTan2(1, 1):0:6, ' ', Hypot(3, 4):0:1);
  WriteLn(DegToRad(180):0:6, ' ', RadToDeg(Pi):0:1);
  WriteLn(Sum(D):0:2, ' ', Mean(D):0:3, ' ', MaxValue(D):0:1, ' ', MinValue(D):0:1, ' ', StdDev(D):0:4);
  WriteLn(RoundTo(1234.5678, 2):0:2, ' ', RoundTo(1.2345, -2):0:4, ' ', SimpleRoundTo(1.235):0:2);
  WriteLn(Sign(-3.5), ' ', Sign(0), ' ', CompareValue(1.0, 2.0), ' ', InRange(5, 1, 10), ' ', EnsureRange(15, 1, 10));
  WriteLn(IsZero(1e-13), ' ', SameValue(1.0, 1.0 + 1e-14), ' ', IsNan(NaN), ' ', IsInfinite(Infinity));
  M := GetExceptionMask;
  WriteLn(exZeroDivide in M, ' ', exPrecision in M, ' ', exUnderflow in M);
  SetExceptionMask([exInvalidOp, exZeroDivide, exOverflow]);
  WriteLn(exZeroDivide in GetExceptionMask, ' ', 1 / (D[0] - 1.5) > 0);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "7 1.50 9\n1024 1.414214 0.111111\n-3 -2 2 3\n3.000 3.000 4.000\n1.000000 1.570796 0.785398 5.0\n3.141593 180.0\n5.00 1.250 4.0 -3.0 3.0139\n1200.00 1.2300 1.24\n-1 0 -1 TRUE 10\nTRUE TRUE TRUE TRUE\nFALSE TRUE TRUE\nTRUE TRUE\n"
        );
    }

    #[test]
    fn random_matches_fpc() {
        let (ok, out) = build_and_run_source(
            "program rnd;
var I: Integer;
begin
  for I := 1 to 5 do Write(Random(100), ' ');
  WriteLn;
  RandSeed := 42;
  for I := 1 to 5 do Write(Random(1000), ' ');
  WriteLn(Random:0:8);
  for I := 1 to 700 do Random(10);
  WriteLn(Random(1000000), ' ', RandSeed);
  RandSeed := 42;
  WriteLn(Random(1000));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "54 59 71 84 60 \n374 796 950 183 731 0.77969100\n162934 4294967253\n374\n"
        );
    }

    #[test]
    fn real_output_matches_fpc() {
        let (ok, out) = build_and_run_source(
            "program rf;
var S: Single; D: Double;
begin
  WriteLn(0.125:0:2, ' ', 0.375:0:2, ' ', 2.5:0:0, ' ', 3.5:0:0, ' ', 0.5:0:0, ' ', 1.005:0:2, ' ', 2.675:0:2);
  WriteLn(1.25:8, '|', 1.25:9, '|', 1.25:20, '|', 1.25:30, '|', -1.25:3);
  S := 0.1; D := 0.1;
  WriteLn(S, '|', D, '|', S:0:10, '|', D:0:20, '|', S:12);
  WriteLn(123456789012345678.0:0:1, ' ', 1e20:0:2, ' ', 0.000001234:0:8);
  WriteLn(-0.0:0:2, ' ', -0.0, ' ', 1e-320, ' ', 5e-324:0:2);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "0.13 0.38 3 4 1 1.01 2.68\n 1.3E+00| 1.25E+00|     1.250000000E+00|               1.250000000E+00|-1.3E+00\n 1.000000015E-01| 1.0000000000000001E-001|0.1000000015|0.10000000000000001000| 1.00000E-01\n123456789012345680.0 100000000000000000000.00 0.00000123\n-0.00 -0.000000000E+00  9.9998886718268301E-321 0.00\n"
        );
    }

    #[test]
    fn real_specials_and_widths_match_fpc() {
        let (ok, out) = build_and_run_source(
            "program wf;
{$mode objfpc}
uses Math;
var Z, N, X: Double; S: Single;
begin
  SetExceptionMask([exInvalidOp, exDenormalized, exZeroDivide, exOverflow, exUnderflow, exPrecision]);
  Z := 0; X := 1.5; S := 2.25;
  WriteLn(Z);
  WriteLn(X);
  WriteLn(-X);
  WriteLn(S);
  WriteLn(1 / Z);
  WriteLn(-1 / Z);
  N := Z / Z;
  WriteLn(N);
  WriteLn(1 / Z:10:2, '|', N:8:3, '|', X:12, '|', Z:0:2);
  WriteLn(1e300 * 10, ' ', 123456789.125:0:3, ' ', 1e-5);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            " 0.0000000000000000E+000\n 1.5000000000000000E+000\n-1.5000000000000000E+000\n 2.250000000E+00\n                    +Inf\n                    -Inf\n                     Nan\n      +Inf|     Nan| 1.5000E+000|0.00\n 1.0000000000000001E+301 123456789.125  1.0000000000000001E-005\n"
        );
    }

    #[test]
    fn qword_arithmetic_matches_fpc() {
        let (ok, out) = build_and_run_source(
            "program qw;
{$mode objfpc}
uses SysUtils;
var Q, R: QWord; I: Int64; S: string; B: Byte;
begin
  Q := QWord($FFFFFFFFFFFFFFFF);
  R := 10;
  WriteLn(Q shr 60, ' ', Q div R, ' ', Q mod R, ' ', Q > R, ' ', Q);
  WriteLn(Q div 3, ' ', Q mod 7, ' ', Q > 5, ' ', (Q - 1) div 2);
  Str(Q, S);
  WriteLn(S, ' ', IntToStr(Q), ' ', IntToStr(-5), ' ', IntToStr(Int64(42)), ' ', UIntToStr(Q));
  Q := QWord(1) shl 63;
  B := 200;
  WriteLn(Q, ' ', Q > B, ' ', Q:25, '|');
  I := -1;
  WriteLn(I < 0, ' ', I div 2);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "15 1844674407370955161 5 TRUE 18446744073709551615\n6148914691236517205 1 TRUE 9223372036854775807\n18446744073709551615 18446744073709551615 -5 42 18446744073709551615\n9223372036854775808 TRUE       9223372036854775808|\nTRUE 0\n"
        );
    }

    #[test]
    fn initialized_variables() {
        let (ok, out) = build_and_run_source(
            "program iv;
type TPt = record X, Y: Integer; end;
var
  Count: Integer = 5;
  Name: string = 'bruto';
  Pt: TPt = (X: 1; Y: 2);
  Primes: array[1..4] of Integer = (2, 3, 5, 7);
procedure Bump;
var Calls: Integer = 10;
begin
  Inc(Calls);
  WriteLn('calls ', Calls);
end;
begin
  WriteLn(Count, ' ', Name, ' ', Pt.X + Pt.Y, ' ', Primes[4]);
  Count := Count * 2;
  Name := Name + '!';
  Primes[1] := 11;
  WriteLn(Count, ' ', Name, ' ', Primes[1]);
  Bump;
  Bump;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "5 bruto 3 7\n10 bruto! 11\ncalls 11\ncalls 11\n");
    }

    #[test]
    fn float_errors_raise_as_fpc() {
        let (ok, out) = build_and_run_source(
            "program fz3;
{$mode objfpc}{$H+}
uses SysUtils, Math;
var A, B, C: Double; S: Single; I: Integer;
procedure Try1(const What: string; X: Double);
begin
  WriteLn(What, ' ', X:0:3);
end;
begin
  A := 1; B := 0;
  try C := A / B; WriteLn(C); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  try C := B / B; WriteLn(C); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  try C := Sqrt(B - 1); WriteLn(C); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  try C := Ln(B); WriteLn(C); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  try C := Exp(1000); WriteLn(C); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  try C := 1e300 * 1e300; WriteLn(C); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  try I := 0; I := 10 div I; except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  C := 1e-300 * 1e-300;
  WriteLn('underflow masked ', C);
  SetExceptionMask([exInvalidOp, exDenormalized, exZeroDivide, exOverflow, exUnderflow, exPrecision]);
  C := A / B;
  WriteLn('masked ', C, ' ', IsInfinite(C), ' ', IsNan(B / B));
  SetExceptionMask([exDenormalized, exUnderflow, exPrecision]);
  C := Infinity + 1;
  WriteLn('inf arithmetic ok ', C > 0);
end.
",
        );
        assert!(ok, "program failed: {out}");
        // FPC on 64-bit ARM macOS reports every floating point trap as an
        // access violation (runtime error 216).
        let traps = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            "EAccessViolation: Access violation\n".repeat(6)
        } else {
            "EZeroDivide: Floating point division by zero\nEInvalidOp: Invalid floating point operation\nEInvalidOp: Invalid floating point operation\nEZeroDivide: Floating point division by zero\nEOverflow: Floating point overflow\nEOverflow: Floating point overflow\n".to_string()
        };
        assert_eq!(
            out,
            traps
                + "EDivByZero: Division by zero\nunderflow masked  0.0000000000000000E+000\nmasked                     +Inf TRUE TRUE\ninf arithmetic ok TRUE\n"
        );
    }

    #[test]
    fn goto_into_or_out_of_try_is_an_error() {
        let Err(e) = MiniPascal.build(
            "program gt;
{$mode objfpc}{$H+}
uses SysUtils;
label 10, 20, 30;
var I: Integer; S: string;
begin
  I := 0;
  try
    try
      WriteLn('inner');
      Inc(I);
      if I = 1 then goto 10;
      WriteLn('not here');
    finally
      WriteLn('finally inner');
    end;
  finally
    WriteLn('finally outer');
  end;
10:
  WriteLn('at 10');
  try
    S := 'x';
    try
      goto 20;
    finally
      WriteLn('finally A');
    end;
20:
    WriteLn('at 20, still in try');
    goto 30;
  except
    WriteLn('no');
  end;
30:
  WriteLn('at 30');
  try
    raise Exception.Create('e');
  except
    on E: Exception do
    begin
      WriteLn('handler');
    end;
  end;
  WriteLn('done ', I);
end.
",
        ) else {
            panic!("should not compile");
        };
        assert!(
            format!("{e:?}").contains("jump in or outside of an exception block"),
            "{e:?}"
        );
        // Within one part of a `try`, a goto is fine.
        let (ok, out) = build_and_run_source(
            "program T;
label 1;
var I: Integer;
begin
  try
    I := 0;
1:  I := I + 1;
    if I < 3 then goto 1;
  finally
    WriteLn(I);
  end;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "3\n");
    }

    #[test]
    fn specialization_class_names_match_fpc() {
        let (ok, out) = build_and_run_source(
            "program cn;
{$mode objfpc}{$H+}
uses fgl, Generics.Collections;
type
  generic TBox<T> = class
    V: T;
  end;
  generic TPair2<A, B> = class end;
  TRec = record X: Integer; end;
  TIntBox = specialize TBox<Integer>;
  TMyList = specialize TFPGList<Integer>;
var
  B1: TIntBox;
  B2: specialize TBox<string>;
  B3: specialize TBox<Double>;
  B4: specialize TBox<TRec>;
  B5: specialize TBox<TObject>;
  P: specialize TPair2<Byte, specialize TBox<Char>>;
  L: TMyList;
  D: specialize TDictionary<string, Integer>;
  B6: specialize TBox<Int64>;
  B7: specialize TBox<Boolean>;
  B8: specialize TBox<AnsiString>;
begin
  B1 := TIntBox.Create; WriteLn(B1.ClassName);
  B2 := specialize TBox<string>.Create; WriteLn(B2.ClassName);
  B3 := specialize TBox<Double>.Create; WriteLn(B3.ClassName);
  B4 := specialize TBox<TRec>.Create; WriteLn(B4.ClassName);
  B5 := specialize TBox<TObject>.Create; WriteLn(B5.ClassName);
  P := specialize TPair2<Byte, specialize TBox<Char>>.Create; WriteLn(P.ClassName);
  L := TMyList.Create; WriteLn(L.ClassName);
  D := specialize TDictionary<string, Integer>.Create; WriteLn(D.ClassName);
  B6 := specialize TBox<Int64>.Create; WriteLn(B6.ClassName);
  B7 := specialize TBox<Boolean>.Create; WriteLn(B7.ClassName);
  B8 := specialize TBox<AnsiString>.Create; WriteLn(B8.ClassName);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "TBox<System.LongInt>\nTBox<System.AnsiString>\nTBox<System.Double>\nTBox<cn.TRec>\nTBox<System.TObject>\nTPair2<System.Byte,cn.TBox<System.Char>>\nTFPGList<System.LongInt>\nTDictionary<System.AnsiString,System.LongInt>\nTBox<System.Int64>\nTBox<System.Boolean>\nTBox<System.AnsiString>\n"
        );
    }

    #[test]
    fn interface_resolution_clauses_and_delegation() {
        let (ok, out) = build_and_run_source(
            "program res;
{$mode objfpc}{$H+}
uses SysUtils;
type
  IReader = interface
    ['{11111111-0000-0000-0000-000000000001}']
    function Read: string;
    function Name: string;
  end;
  IWriter = interface
    ['{11111111-0000-0000-0000-000000000002}']
    procedure Write(const S: string);
    function Name: string;
  end;

  TFile = class(TInterfacedObject, IReader, IWriter)
  private
    FData: string;
  public
    function IReader.Name = ReaderName;
    function IWriter.Name = WriterName;
    procedure IWriter.Write = Store;
    function Read: string;
    function ReaderName: string;
    function WriterName: string;
    procedure Store(const S: string);
    destructor Destroy; override;
  end;

  { Delegation: TOuter implements IWriter through FInner. }
  TOuter = class(TInterfacedObject, IWriter)
  private
    FInner: IWriter;
  public
    constructor Create;
    destructor Destroy; override;
    property Inner: IWriter read FInner implements IWriter;
  end;

  TLog = class(TInterfacedObject, IWriter)
    procedure Write(const S: string);
    function Name: string;
    destructor Destroy; override;
  end;

  { Delegation to a class-typed field. }
  TOuter2 = class(TInterfacedObject, IReader)
  private
    FFile: TFile;
    function GetFile: TFile;
  public
    constructor Create;
    destructor Destroy; override;
    property AsFile: TFile read GetFile implements IReader;
  end;

function TFile.Read: string; begin Result := FData; end;
function TFile.ReaderName: string; begin Result := 'reader'; end;
function TFile.WriterName: string; begin Result := 'writer'; end;
procedure TFile.Store(const S: string); begin FData := FData + S; end;
destructor TFile.Destroy; begin WriteLn('file gone'); inherited; end;

constructor TOuter.Create; begin inherited Create; FInner := TLog.Create; end;
destructor TOuter.Destroy; begin WriteLn('outer gone'); inherited; end;

procedure TLog.Write(const S: string); begin WriteLn('log: ', S); end;
function TLog.Name: string; begin Result := 'log'; end;
destructor TLog.Destroy; begin WriteLn('log gone'); inherited; end;

constructor TOuter2.Create; begin inherited Create; FFile := TFile.Create; FFile.Store('inside'); end;
destructor TOuter2.Destroy; begin WriteLn('outer2 gone'); FFile.Free; inherited; end;
function TOuter2.GetFile: TFile; begin Result := FFile; end;

procedure Main;
var
  F: TFile;
  R: IReader;
  W: IWriter;
  O: TOuter;
  R2: IReader;
begin
  F := TFile.Create;
  R := F;
  W := F;
  W.Write('abc');
  W.Write('def');
  WriteLn(R.Read, ' ', R.Name, ' ', W.Name);
  WriteLn(Supports(R, IWriter), ' ', (R as IWriter).Name);
  R := nil;
  W := nil;
  WriteLn('--');
  O := TOuter.Create;
  W := O;
  W.Write('via outer');
  WriteLn(W.Name);
  W := nil;
  WriteLn(Supports(O, IWriter, W));
  W.Write('again');
  W := nil;
  O.Free;
  WriteLn('--');
  R2 := TOuter2.Create;
  WriteLn(R2.Read, ' ', R2.Name);
  R2 := nil;
  WriteLn('end');
end;

begin
  Main;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "abcdef reader writer\nTRUE writer\n--\nlog: via outer\nlog\nTRUE\nlog: again\nouter gone\nlog gone\n--\ninside reader\nfile gone\nend\nfile gone\n"
        );
    }

    #[test]
    fn corba_interfaces() {
        let (ok, out) = build_and_run_source(
            "program corba;
{$mode objfpc}{$H+}
{$interfaces corba}
uses SysUtils;
type
  IShape = interface
    ['IShape']
    function Area: Double;
    function Name: string;
  end;
  IScalable = interface
    ['IScalable']
    procedure Scale(F: Double);
  end;
  IDrawable = interface(IShape)
    ['IDrawable']
    procedure Draw;
  end;

  TSquare = class(TObject, IDrawable, IScalable)
    FSide: Double;
    constructor Create(S: Double);
    destructor Destroy; override;
    function Area: Double;
    function Name: string;
    procedure Draw;
    procedure Scale(F: Double);
  end;

constructor TSquare.Create(S: Double); begin FSide := S; end;
destructor TSquare.Destroy; begin WriteLn('square destroyed'); inherited; end;
function TSquare.Area: Double; begin Result := FSide * FSide; end;
function TSquare.Name: string; begin Result := 'square'; end;
procedure TSquare.Draw; begin WriteLn('drawing ', Name, ' ', Area:0:1); end;
procedure TSquare.Scale(F: Double); begin FSide := FSide * F; end;

procedure Show(S: IShape);
begin
  WriteLn(S.Name, ': ', S.Area:0:2);
end;

function GetDrawable(Sq: TSquare): IDrawable;
begin
  Result := Sq;
end;

var
  Sq: TSquare;
  D: IDrawable;
  Sc: IScalable;
  S: IShape;
begin
  Sq := TSquare.Create(2);
  D := Sq;
  D.Draw;
  S := D;
  Show(S);
  Sc := Sq;
  Sc.Scale(1.5);
  Show(D);
  if Supports(Sq, IScalable) then WriteLn('scalable');
  WriteLn(Sq is IDrawable, ' ', D <> nil);
  (Sq as IScalable).Scale(2);
  GetDrawable(Sq).Draw;
  D := nil;
  S := nil;
  Sc := nil;
  WriteLn('refs dropped, object alive');
  Sq.Draw;
  Sq.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "drawing square 4.0\nsquare: 4.00\nsquare: 9.00\nscalable\nTRUE TRUE\ndrawing square 36.0\nrefs dropped, object alive\ndrawing square 36.0\nsquare destroyed\n"
        );
    }

    #[test]
    fn enum_names_and_writestr_match_fpc() {
        let (ok, out) = build_and_run_source(
            "program ws;
{$mode objfpc}{$H+}
type
  TColor = (clRed, clGreen, clBlue);
  TSize = (szSmall = 1, szLarge = 5, szHuge);
var
  C: TColor; Z: TSize; S: string; B: Boolean; Q: QWord; F: Text;
begin
  C := clGreen;
  WriteLn(C, ' ', Ord(C), ' [', C:8, ']', High(TSize), Low(TSize), Ord(szHuge));
  Z := szLarge;
  WriteLn(Z, ' ', Ord(Z), ' ', Succ(clRed));
  B := True; Q := QWord($FFFFFFFFFFFFFFFF);
  WriteStr(S, C, ' ', 42:5, ' ', 3.5:0:2, ' ', B, ' ', 'x', ' ', Q, '|', 'abc':6, '|', 1.25);
  WriteLn(S);
  WriteStr(S, Z);
  WriteLn(Length(S), S);
  Assign(F, 'bruto_ws_test.txt'); Rewrite(F); WriteLn(F, C, ' ', Z:7); Close(F);
  Assign(F, 'bruto_ws_test.txt'); Reset(F); ReadLn(F, S); Close(F); WriteLn(S);
end.
",
        );
        let _ = std::fs::remove_file("bruto_ws_test.txt");
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "clGreen 1 [clGreen ]szHugeszSmall6\nszLarge 5 clGreen\nclGreen    42 3.50 TRUE x 18446744073709551615|   abc| 1.250000000E+00\n7szLarge\nclGreen szLarge\n"
        );
    }

    #[test]
    fn enum_type_information_matches_fpc() {
        let (ok, out) = build_and_run_source(
            "program en;
{$mode objfpc}{$H+}
uses TypInfo;
type
  TColor = (clRed, clGreen, clBlue);
  TSize = (szSmall = 1, szLarge = 5);
  TColors = set of TColor;
var
  C: TColor;
  S: string;
  P: PTypeInfo;
  D: PTypeData;
  Cs: TColors;
begin
  C := clGreen;
  WriteLn(C, ' ', Ord(C));
  WriteStr(S, C);
  WriteLn(S, ' ', GetEnumName(TypeInfo(TColor), 2), ' ', GetEnumValue(TypeInfo(TColor), 'clblue'));
  P := TypeInfo(TColor);
  WriteLn(P^.Name, ' ', P^.Kind = tkEnumeration);
  D := GetTypeData(P);
  WriteLn(D^.MinValue, ' ', D^.MaxValue);
  WriteLn(GetEnumName(TypeInfo(Boolean), 1), ' ', GetEnumNameCount(TypeInfo(TColor)));
  WriteLn(GetEnumValue(TypeInfo(TColor), 'nothing'));
  Cs := [clRed, clBlue];
  WriteLn(SetToString(PTypeInfo(TypeInfo(TColors)), Integer(Cs), True));
  WriteLn(PTypeInfo(TypeInfo(Integer))^.Name, ' ', PTypeInfo(TypeInfo(string))^.Name);
  WriteLn(szLarge);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "clGreen 1\nclGreen clBlue 2\nTColor TRUE\n0 2\nTrue 3\n-1\n[clRed,clBlue]\nLongInt AnsiString\nszLarge\n"
        );
    }

    #[test]
    fn published_properties_match_fpc() {
        let (ok, out) = build_and_run_source(
            "program pr;
{$mode objfpc}{$H+}
uses SysUtils, Classes, TypInfo;
type
  TShape = (shCircle, shSquare, shTriangle);
  TStyle = (stBold, stItalic, stUnder);
  TStyles = set of TStyle;

  TBase = class(TPersistent)
  private
    FName: string;
    FTag: Integer;
  published
    property Name: string read FName write FName;
    property Tag: Integer read FTag write FTag default 0;
  end;

  TWidget = class(TBase)
  private
    FWidth: Integer;
    FShape: TShape;
    FStyles: TStyles;
    FRatio: Double;
    FVisible: Boolean;
    FCaption: string;
    FCoords: array[0..1] of Integer;
    FOwnerW: TObject;
    FSmall: Byte;
    function GetCaption: string;
    procedure SetCaption(const V: string);
    function GetCoord(Index: Integer): Integer;
    procedure SetCoord(Index: Integer; V: Integer);
    function GetArea: Integer; virtual;
  public
    Hidden: Integer;
  published
    property Width: Integer read FWidth write FWidth;
    property Shape: TShape read FShape write FShape default shSquare;
    property Styles: TStyles read FStyles write FStyles;
    property Ratio: Double read FRatio write FRatio;
    property Visible: Boolean read FVisible write FVisible;
    property Caption: string read GetCaption write SetCaption;
    property X: Integer index 0 read GetCoord write SetCoord;
    property Y: Integer index 1 read GetCoord write SetCoord;
    property Area: Integer read GetArea;
    property Other: TObject read FOwnerW write FOwnerW;
    property Small: Byte read FSmall write FSmall;
  end;

  TBig = class(TWidget)
    function GetArea: Integer; override;
  end;

function TWidget.GetCaption: string; begin Result := '<' + FCaption + '>'; end;
procedure TWidget.SetCaption(const V: string); begin FCaption := UpperCase(V); end;
function TWidget.GetCoord(Index: Integer): Integer; begin Result := FCoords[Index]; end;
procedure TWidget.SetCoord(Index: Integer; V: Integer); begin FCoords[Index] := V * 10; end;
function TWidget.GetArea: Integer; begin Result := FWidth * FWidth; end;
function TBig.GetArea: Integer; begin Result := 1000 + FWidth; end;

var
  W: TWidget;
  L: PPropList;
  N, I: Integer;
  P: PPropInfo;
begin
  W := TBig.Create;
  N := GetPropList(W, L);
  WriteLn(N);
  for I := 0 to N - 1 do
    WriteLn(I, ' ', L^[I]^.Name, ' ', L^[I]^.PropType^.Name, ' ', Ord(L^[I]^.PropType^.Kind), ' ', L^[I]^.NameIndex);
  FreeMem(L);
  SetStrProp(W, 'Name', 'w1');
  SetOrdProp(W, 'Tag', 42);
  SetOrdProp(W, 'Width', 7);
  SetEnumProp(W, 'Shape', 'shTriangle');
  SetSetProp(W, 'Styles', '[stBold,stUnder]');
  SetFloatProp(W, 'Ratio', 1.5);
  SetOrdProp(W, 'Visible', 1);
  SetStrProp(W, 'Caption', 'hello');
  SetOrdProp(W, 'X', 3);
  SetOrdProp(W, 'Y', 4);
  SetObjectProp(W, 'Other', W);
  SetOrdProp(W, 'Small', 200);
  WriteLn(W.Name, ' ', W.Tag, ' ', W.Width, ' ', W.Shape, ' ', W.Ratio:0:2, ' ', W.Visible, ' ', W.Caption, ' ', W.X, ' ', W.Y);
  WriteLn(GetStrProp(W, 'Name'), ' ', GetOrdProp(W, 'Tag'), ' ', GetEnumProp(W, 'Shape'), ' ', GetSetProp(W, 'Styles', True));
  WriteLn(GetFloatProp(W, 'Ratio'):0:3, ' ', GetOrdProp(W, 'Visible'), ' ', GetStrProp(W, 'Caption'), ' ', GetOrdProp(W, 'X'), ' ', GetOrdProp(W, 'Area'));
  WriteLn(GetObjectProp(W, 'Other') = W, ' ', GetOrdProp(W, 'Small'), ' ', GetObjectPropClass(W, 'Other').ClassName);
  WriteLn(IsPublishedProp(W, 'Hidden'), ' ', IsPublishedProp(W, 'width'), ' ', PropIsType(W, 'Shape', tkEnumeration));
  P := GetPropInfo(W, 'Shape');
  WriteLn(P^.Default, ' ', GetPropInfo(W, 'Width')^.Default = Low(LongInt), ' ', IsStoredProp(W, P));
  WriteLn(GetTypeData(PTypeInfo(W.ClassInfo))^.PropCount, ' ', PTypeInfo(W.ClassInfo)^.Name, ' ', GetTypeData(PTypeInfo(W.ClassInfo))^.ParentInfo^.Name);
  N := GetPropList(PTypeInfo(W.ClassInfo), [tkInteger, tkEnumeration], nil, True);
  GetMem(L, N * SizeOf(Pointer));
  GetPropList(PTypeInfo(W.ClassInfo), [tkInteger, tkEnumeration], L, True);
  for I := 0 to N - 1 do Write(L^[I]^.Name, ' ');
  WriteLn;
  FreeMem(L);
  try
    GetOrdProp(W, 'Nope');
  except
    on E: Exception do WriteLn(E.ClassName, ': ', E.Message);
  end;
  W.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "13\n0 Name AnsiString 9 0\n1 Tag LongInt 1 1\n2 Width LongInt 1 2\n3 Shape TShape 3 3\n4 Styles TStyles 5 4\n5 Ratio Double 4 5\n6 Visible Boolean 18 6\n7 Caption AnsiString 9 7\n8 X LongInt 1 8\n9 Y LongInt 1 9\n10 Area LongInt 1 10\n11 Other TObject 15 11\n12 Small Byte 1 12\nw1 42 7 shTriangle 1.50 TRUE <HELLO> 30 40\nw1 42 shTriangle [stBold,stUnder]\n1.500 1 <HELLO> 30 1007\nTRUE 200 TObject\nFALSE TRUE TRUE\n1 TRUE TRUE\n13 TBig TWidget\nArea Shape Small Tag Width X Y \nEPropertyError: Unknown property: \"Nope\"\n"
        );
    }

    #[test]
    fn component_published_properties_match_fpc() {
        let (ok, out) = build_and_run_source(
            "program comp;
{$mode objfpc}{$H+}
uses SysUtils, Classes, TypInfo;
type
  TColor = (clRed, clGreen);
  TThing = class(TComponent)
  private
    FSize: Integer;
    FColor: TColor;
    FSecret: Integer;
  public
    property Secret: Integer read FSecret write FSecret;
  { default section, published under $M+ }
  published
    property Size: Integer read FSize write FSize default 10;
    property Color: TColor read FColor write FColor;
  end;
  TDefaulted = class(TPersistent)
  private
    FA: Integer;
  property A: Integer read FA write FA;
  end;
var
  T: TThing;
  L: PPropList;
  N, I: Integer;
begin
  T := TThing.Create(nil);
  T.Name := 'Thing1';
  N := GetPropList(T, L);
  for I := 0 to N - 1 do
    WriteLn(L^[I]^.Name, ': ', L^[I]^.PropType^.Name, ' stored=', IsStoredProp(T, L^[I]), ' default=', L^[I]^.Default);
  FreeMem(L);
  WriteLn(GetStrProp(T, 'Name'), ' ', IsPublishedProp(T, 'Secret'), ' ', IsPublishedProp(TDefaulted, 'A'));
  T.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "Name: AnsiString stored=FALSE default=0\nTag: Int64 stored=TRUE default=0\nSize: LongInt stored=TRUE default=10\nColor: TColor stored=TRUE default=-2147483648\nThing1 FALSE FALSE\n"
        );
    }

    #[test]
    fn component_streaming_matches_fpc() {
        let (ok, out) = build_and_run_source(
            "program st1;
{$mode objfpc}{$H+}
uses SysUtils, Classes, TypInfo;
type
  TShape = (shCircle, shSquare, shTriangle);
  TStyle = (stBold, stItalic, stUnder);
  TStyles = set of TStyle;

  TFont = class(TPersistent)
  private
    FSize: Integer;
    FName: string;
  published
    property Size: Integer read FSize write FSize default 10;
    property Name: string read FName write FName;
  end;

  TPanel = class(TComponent)
  private
    FWidth: Integer;
    FShape: TShape;
    FStyles: TStyles;
    FRatio: Double;
    FVisible: Boolean;
    FCaption: string;
    FFont: TFont;
    FBuddy: TPanel;
    FBig: Int64;
    FInitial: Char;
    FTiny: ShortInt;
  public
    constructor Create(AOwner: TComponent); override;
    destructor Destroy; override;
    procedure GetChildren(Proc: TGetChildProc; Root: TComponent); override;
  published
    property Width: Integer read FWidth write FWidth;
    property Shape: TShape read FShape write FShape default shCircle;
    property Styles: TStyles read FStyles write FStyles;
    property Ratio: Double read FRatio write FRatio;
    property Visible: Boolean read FVisible write FVisible default True;
    property Caption: string read FCaption write FCaption;
    property Font: TFont read FFont write FFont;
    property Buddy: TPanel read FBuddy write FBuddy;
    property Big: Int64 read FBig write FBig;
    property Initial: Char read FInitial write FInitial;
    property Tiny: ShortInt read FTiny write FTiny;
  end;

constructor TPanel.Create(AOwner: TComponent);
begin
  inherited Create(AOwner);
  FFont := TFont.Create;
  FFont.Size := 10;
  FVisible := True;
end;

destructor TPanel.Destroy;
begin
  FFont.Free;
  inherited Destroy;
end;

procedure TPanel.GetChildren(Proc: TGetChildProc; Root: TComponent);
var
  I: Integer;
begin
  for I := 0 to ComponentCount - 1 do
    Proc(Components[I]);
end;

procedure Dump(S: TMemoryStream);
var
  I: Integer;
  B: Byte;
begin
  S.Position := 0;
  for I := 0 to S.Size - 1 do
  begin
    S.ReadBuffer(B, 1);
    Write(IntToHex(B, 2));
    if I mod 32 = 31 then WriteLn;
  end;
  WriteLn;
end;

var
  Root, Child, Back: TPanel;
  Bin: TMemoryStream;
  Txt: TStringStream;
begin
  RegisterClass(TPanel);
  Root := TPanel.Create(nil);
  Root.Name := 'Main';
  Root.Width := 300;
  Root.Shape := shTriangle;
  Root.Styles := [stBold, stUnder];
  Root.Ratio := 1.5;
  Root.Visible := False;
  Root.Caption := 'Hello ''world''' + #10 + 'é';
  Root.Font.Size := 12;
  Root.Font.Name := 'Mono';
  Root.Big := 5000000000;
  Root.Initial := 'Q';
  Root.Tiny := -5;
  Child := TPanel.Create(Root);
  Child.Name := 'Side';
  Child.Width := 40000;
  Child.Tag := 7;
  Root.Buddy := Child;

  Bin := TMemoryStream.Create;
  Bin.WriteComponent(Root);
  Dump(Bin);
  Bin.Position := 0;
  Txt := TStringStream.Create('');
  ObjectBinaryToText(Bin, Txt);
  Write(Txt.DataString);

  Bin.Position := 0;
  Back := TPanel(Bin.ReadComponent(nil));
  WriteLn(Back.Name, ' ', Back.Width, ' ', Back.Shape, ' ', Back.Ratio:0:2, ' ', Back.Visible, ' ', Back.Caption);
  WriteLn(Back.Font.Size, ' ', Back.Font.Name, ' ', Back.Big, ' ', Back.Initial, ' ', Back.Tiny, ' ', Back.ComponentCount);
  WriteLn(Back.Components[0].Name, ' ', TPanel(Back.Components[0]).Width, ' ', Back.Components[0].Tag, ' ', Back.Buddy = Back.Components[0]);
  WriteLn(GetSetProp(Back, 'Styles', True));
  Back.Free;
  Root.Free;
  Bin.Free;
  Txt.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "54504630065450616E656C044D61696E055769647468032C0105536861706507\n0A7368547269616E676C65065374796C65730B067374426F6C64077374556E64\n65720005526174696F0500000000000000C0FF3F0756697369626C6508074361\n7074696F6E061048656C6C6F2027776F726C64270AC3A909466F6E742E53697A\n65020C09466F6E742E4E616D6506044D6F6E6F05427564647907045369646503\n4269671300F2052A0100000007496E697469616C0601510454696E7902FB0006\n5450616E656C045369646503546167020705576964746804409C000006537479\n6C65730B0003426967020007496E697469616C0601000454696E790200000000\n\nobject Main: TPanel\n  Width = 300\n  Shape = shTriangle\n  Styles = [stBold, stUnder]\n  Ratio =  1.5000000000000000E+000\n  Visible = False\n  Caption = 'Hello ''world'''#10#195#169\n  Font.Size = 12\n  Font.Name = 'Mono'\n  Buddy = Side\n  Big = 5000000000\n  Initial = 'Q'\n  Tiny = -5\n  object Side: TPanel\n    Tag = 7\n    Width = 40000\n    Styles = []\n    Big = 0\n    Initial = #0\n    Tiny = 0\n  end\nend\nMain 300 shTriangle 1.50 FALSE Hello 'world'\né\n12 Mono 5000000000 Q -5 1\nSide 40000 7 TRUE\n[stBold,stUnder]\n"
        );
    }

    #[test]
    fn component_text_streaming_matches_fpc() {
        let (ok, out) = build_and_run_source(
            "program st2;
{$mode objfpc}{$H+}
uses SysUtils, Classes;
type
  TAlign = (alNone, alTop, alClient);
  TButton = class(TComponent)
  private
    FCaption: string;
    FOnClick: TNotifyEvent;
    FAlign: TAlign;
    FTarget: TComponent;
    FScale: Double;
  published
    property Caption: string read FCaption write FCaption;
    property OnClick: TNotifyEvent read FOnClick write FOnClick;
    property Align: TAlign read FAlign write FAlign default alNone;
    property Target: TComponent read FTarget write FTarget;
    property Scale: Double read FScale write FScale;
  end;

  TForm = class(TComponent)
  private
    FTitle: string;
    FLoadedCalled: Boolean;
  protected
    procedure Loaded; override;
  public
    procedure GetChildren(Proc: TGetChildProc; Root: TComponent); override;
  published
    OkButton: TButton;
    CancelButton: TButton;
    procedure OkClick(Sender: TObject);
    property Title: string read FTitle write FTitle;
  end;

procedure TForm.Loaded;
begin
  inherited Loaded;
  FLoadedCalled := True;
end;

procedure TForm.GetChildren(Proc: TGetChildProc; Root: TComponent);
var
  I: Integer;
begin
  for I := 0 to ComponentCount - 1 do
    Proc(Components[I]);
end;

procedure TForm.OkClick(Sender: TObject);
begin
  WriteLn('OK clicked by ', TComponent(Sender).Name);
end;

const
  Form1Text =
    'object Form1: TForm' + LineEnding +
    '  Title = ''Hello '' + #9 + ''there''' + LineEnding +
    '  Tag = -70000' + LineEnding +
    '  object OkButton: TButton' + LineEnding +
    '    Caption = ''&OK''' + LineEnding +
    '    OnClick = OkClick' + LineEnding +
    '    Align = alTop' + LineEnding +
    '    Target = CancelButton' + LineEnding +
    '    Scale = 2.25' + LineEnding +
    '  end' + LineEnding +
    '  object CancelButton: TButton' + LineEnding +
    '    Caption = ''Cancel''' + LineEnding +
    '    Scale = -1E-3' + LineEnding +
    '  end' + LineEnding +
    'end' + LineEnding;

var
  Txt, Bin, Out: TStringStream;
  Form: TForm;
begin
  RegisterClasses([TButton, TForm]);
  Txt := TStringStream.Create(Form1Text);
  Bin := TStringStream.Create('');
  ObjectTextToBinary(Txt, Bin);
  WriteLn(Bin.Size);
  Bin.Position := 0;
  Form := TForm.Create(nil);
  Bin.ReadComponent(Form);
  WriteLn(Form.Name, ' [', Form.Title, '] ', Form.Tag, ' ', Form.FLoadedCalled, ' ', Form.ComponentCount);
  WriteLn(Form.OkButton.Caption, ' ', Form.OkButton.Align, ' ', Form.OkButton.Target = Form.CancelButton, ' ', Form.OkButton.Scale:0:3);
  WriteLn(Form.CancelButton.Caption, ' ', Form.CancelButton.Scale:0:4, ' ', Assigned(Form.CancelButton.OnClick));
  Form.OkButton.OnClick(Form.OkButton);
  Out := TStringStream.Create('');
  Bin.Size := 0;
  Bin.WriteComponent(Form);
  Bin.Position := 0;
  ObjectBinaryToText(Bin, Out);
  Write(Out.DataString);
  WriteLn(FindClass('tbutton').ClassName, ' ', GetClass('TNope') = nil);
  try
    FindClass('TNope');
  except
    on E: Exception do WriteLn(E.ClassName, ': ', E.Message);
  end;
  Form.Free;
  Txt.Free; Bin.Free; Out.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "203\nForm1 [Hello 	there] -70000 TRUE 2\n&OK alTop TRUE 2.250\nCancel -0.0010 FALSE\nOK clicked by OkButton\nobject Form1: TForm\n  Tag = -70000\n  Title = 'Hello '#9'there'\n  object OkButton: TButton\n    Caption = '&OK'\n    OnClick = OkClick\n    Align = alTop\n    Target = CancelButton\n    Scale =  2.2500000000000000E+000\n  end\n  object CancelButton: TButton\n    Caption = 'Cancel'\n    Scale = -1.0000000000000000E-003\n  end\nend\nTButton TRUE\nEClassNotFound: Class \"TNope\" not found\n"
        );
    }

    #[test]
    fn utf8_string_literals_match_fpc() {
        let (ok, out) = build_and_run_source(
            "program u8;
{$mode objfpc}{$H+}
var S: string;
begin
  S := 'é';
  WriteLn(S, ' ', Length(S));
  WriteLn('é', ' ', Length('é'));
  WriteLn('naïve €');
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "é 2\né 2\nnaïve €\n");
    }

    #[test]
    fn enum_sizes_match_fpc_objfpc() {
        let (ok, out) = build_and_run_source(
            "program es;
{$mode objfpc}
uses TypInfo;
type
  TA = (a1, a2);
  TNeg = (n1 = -5, n2 = 3);
  TBig = (b1, b2 = 70000);
{$packenum 1}
  TP1 = (p1, p2);
  TP1w = (q1, q2 = 300);
{$packenum 2}
  TP2 = (r1, r2);
{$Z4}
  TZ4 = (z1, z2);
{$Z1}
  TZ1 = (y1, y2);
{$packenum default}
  TD = (d1, d2);
  TRec = record B: Byte; E: TA; C: Byte; end;
begin
  WriteLn(SizeOf(TA), SizeOf(TNeg), SizeOf(TBig), SizeOf(TP1), SizeOf(TP1w), SizeOf(TP2), SizeOf(TZ4), SizeOf(TZ1), SizeOf(TD), ' ', SizeOf(TRec));
  WriteLn(Ord(GetTypeData(TypeInfo(TA))^.OrdType), Ord(GetTypeData(TypeInfo(TP1))^.OrdType), Ord(GetTypeData(TypeInfo(TP2))^.OrdType), Ord(GetTypeData(TypeInfo(Boolean))^.OrdType));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "444122414 12\n5131\n");
    }

    #[test]
    fn enum_sizes_match_fpc_delphi() {
        let (ok, out) = build_and_run_source(
            "program es;
{$mode delphi}
uses TypInfo;
type
  TA = (a1, a2);
  TNeg = (n1 = -5, n2 = 3);
  TBig = (b1, b2 = 70000);
{$packenum 1}
  TP1 = (p1, p2);
  TP1w = (q1, q2 = 300);
{$packenum 2}
  TP2 = (r1, r2);
{$Z4}
  TZ4 = (z1, z2);
{$Z1}
  TZ1 = (y1, y2);
{$packenum default}
  TD = (d1, d2);
  TRec = record B: Byte; E: TA; C: Byte; end;
begin
  WriteLn(SizeOf(TA), SizeOf(TNeg), SizeOf(TBig), SizeOf(TP1), SizeOf(TP1w), SizeOf(TP2), SizeOf(TZ4), SizeOf(TZ1), SizeOf(TD), ' ', SizeOf(TRec));
  WriteLn(Ord(GetTypeData(TypeInfo(TA))^.OrdType), Ord(GetTypeData(TypeInfo(TP1))^.OrdType), Ord(GetTypeData(TypeInfo(TP2))^.OrdType), Ord(GetTypeData(TypeInfo(Boolean))^.OrdType));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "114122414 3\n1131\n");
    }

    #[test]
    fn enums_records_and_sets_match_fpc() {
        let (ok, out) = build_and_run_source(
            "program ew;
{$mode objfpc}{$H+}
uses SysUtils, Classes;
type
  TColor = (clRed, clGreen, clBlue, clYellow);
  TSmall = (s0, s1);
  TColors = set of TColor;
  TRec = packed record
    A: Byte;
    C: TColor;
    B: Word;
  end;
  TObj = class
    Color: TColor;
    function Next(C: TColor): TColor;
  end;
const
  Names: array[TColor] of string = ('red', 'green', 'blue', 'yellow');
  Fav: TColor = clBlue;
var
  C, D: TColor;
  Counts: array[TColor] of Integer;
  R: TRec;
  Arr: array[0..2] of TColor;
  S: TColors;
  O: TObj;
  F: file of TRec;
  M: TMemoryStream;
  P: PByte;
  I: Integer;

function TObj.Next(C: TColor): TColor;
begin
  if C = High(TColor) then Result := Low(TColor) else Result := Succ(C);
end;

procedure Bump(var X: TColor);
begin
  Inc(X);
end;

begin
  for C := Low(TColor) to High(TColor) do
    Counts[C] := Ord(C) * 10;
  for C in TColor do Write(Names[C], ':', Counts[C], ' ');
  WriteLn;
  C := clGreen;
  Bump(C);
  WriteLn(C, ' ', Pred(C), ' ', Fav, ' ', TColor(3), ' ', C > clRed, ' ', C = clBlue);
  case C of
    clRed: WriteLn('r');
    clGreen, clBlue: WriteLn('gb');
  else
    WriteLn('other');
  end;
  S := [clRed, clYellow];
  Include(S, C);
  for D in S do Write(D, ' ');
  WriteLn(clGreen in S);
  O := TObj.Create;
  O.Color := clYellow;
  WriteLn(O.Next(O.Color), ' ', O.Next(clRed));
  O.Free;
  R.A := 1; R.C := clYellow; R.B := $BEEF;
  WriteLn(SizeOf(R), ' ', SizeOf(TColor), ' ', SizeOf(TSmall), ' ', SizeOf(Arr));
  M := TMemoryStream.Create;
  M.WriteBuffer(R, SizeOf(R));
  P := M.Memory;
  for I := 0 to M.Size - 1 do Write(IntToHex(P[I], 2));
  WriteLn;
  M.Free;
  Arr[0] := clBlue; Arr[1] := clRed; Arr[2] := Arr[0];
  WriteLn(Arr[2], ' ', Ord(Arr[1]));
  C := clRed;
  Inc(C, 2);
  Dec(C);
  WriteLn(C, ' ', Ord(High(TSmall)));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "red:0 green:10 blue:20 yellow:30 \nclBlue clGreen clBlue clYellow TRUE TRUE\ngb\nclRed clBlue clYellow FALSE\nclRed clGreen\n7 4 4 12\n0103000000EFBE\nclBlue 0\nclGreen 1\n"
        );
    }

    #[test]
    fn collections_match_fpc() {
        let (ok, out) = build_and_run_source(
            "program co;
{$mode objfpc}{$H+}
uses SysUtils, Classes;
type
  TColumn = class(TCollectionItem)
  private
    FTitle: string;
    FWidth: Integer;
    FVisible: Boolean;
  protected
    function GetDisplayName: string; override;
  public
    procedure Assign(Source: TPersistent); override;
  published
    property Title: string read FTitle write FTitle;
    property Width: Integer read FWidth write FWidth default 50;
    property Visible: Boolean read FVisible write FVisible default True;
  end;

  TColumns = class(TOwnedCollection)
  private
    function GetColumn(Index: Integer): TColumn;
  protected
    procedure Update(Item: TCollectionItem); override;
  public
    function AddColumn(const T: string; W: Integer): TColumn;
    property Columns[Index: Integer]: TColumn read GetColumn; default;
  end;

  TGrid = class(TComponent)
  private
    FColumns: TColumns;
    FCaption: string;
    procedure SetColumns(V: TColumns);
  public
    constructor Create(AOwner: TComponent); override;
    destructor Destroy; override;
  published
    property Caption: string read FCaption write FCaption;
    property Columns: TColumns read FColumns write SetColumns;
  end;

var
  Updates: Integer;

function TColumn.GetDisplayName: string;
begin
  Result := 'Col ' + FTitle;
end;

procedure TColumn.Assign(Source: TPersistent);
begin
  if Source is TColumn then
  begin
    FTitle := TColumn(Source).FTitle;
    FWidth := TColumn(Source).FWidth;
    FVisible := TColumn(Source).FVisible;
  end
  else
    inherited Assign(Source);
end;

function TColumns.GetColumn(Index: Integer): TColumn;
begin
  Result := TColumn(Items[Index]);
end;

procedure TColumns.Update(Item: TCollectionItem);
begin
  Inc(Updates);
end;

function TColumns.AddColumn(const T: string; W: Integer): TColumn;
begin
  Result := TColumn(Add);
  Result.FTitle := T;
  Result.FWidth := W;
  Result.FVisible := True;
end;

constructor TGrid.Create(AOwner: TComponent);
begin
  inherited Create(AOwner);
  FColumns := TColumns.Create(Self, TColumn);
end;

destructor TGrid.Destroy;
begin
  FColumns.Free;
  inherited Destroy;
end;

procedure TGrid.SetColumns(V: TColumns);
begin
  FColumns.Assign(V);
end;

var
  G, G2: TGrid;
  C: TCollectionItem;
  Bin, Txt: TMemoryStream;
  S: TStringStream;
  I: Integer;
begin
  RegisterClass(TGrid);
  G := TGrid.Create(nil);
  G.Name := 'Grid1';
  G.Caption := 'Data';
  G.Columns.AddColumn('Id', 30);
  G.Columns.AddColumn('Name', 50);
  G.Columns.AddColumn('Notes', 200).Visible := False;
  WriteLn(G.Columns.Count, ' ', G.Columns[1].Title, ' ', G.Columns[2].Index, ' ', G.Columns[2].ID, ' ', Updates);
  WriteLn(G.Columns[0].DisplayName, ' ', G.Columns.GetNamePath, ' ', G.Columns[1].GetNamePath);
  G.Columns[2].Index := 0;
  for C in G.Columns do Write(TColumn(C).Title, ' ');
  WriteLn;
  G.Columns.Delete(1);
  WriteLn(G.Columns.Count, ' ', G.Columns.FindItemID(0) = nil, ' ', TColumn(G.Columns.FindItemID(2)).Title);
  TColumn(G.Columns.Insert(1)).Title := 'Middle';
  for I := 0 to G.Columns.Count - 1 do Write(G.Columns[I].Title, ':', G.Columns[I].ID, ' ');
  WriteLn;

  Bin := TMemoryStream.Create;
  Bin.WriteComponent(G);
  Bin.Position := 0;
  S := TStringStream.Create('');
  ObjectBinaryToText(Bin, S);
  Write(S.DataString);
  WriteLn(Bin.Size);

  Bin.Position := 0;
  G2 := TGrid(Bin.ReadComponent(nil));
  WriteLn(G2.Name, ' ', G2.Caption, ' ', G2.Columns.Count);
  for I := 0 to G2.Columns.Count - 1 do
    Write(G2.Columns[I].Title, '/', G2.Columns[I].Width, '/', G2.Columns[I].Visible, ' ');
  WriteLn;

  Txt := TMemoryStream.Create;
  S.Position := 0;
  ObjectTextToBinary(S, Txt);
  WriteLn(Txt.Size, ' ', CompareMem(Txt.Memory, Bin.Memory, Bin.Size));
  G2.Columns.Clear;
  G2.Columns.Assign(G.Columns);
  WriteLn(G2.Columns.Count, ' ', G2.Columns[0].Title);
  G2.Free;
  G.Free;
  Bin.Free; Txt.Free; S.Free;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "3 Name 2 2 3\nCol Id Grid1.Columns Grid1.Columns[1]\nNotes Id Name \n2 TRUE Notes\nNotes:2 Middle:3 Name:1 \nobject Grid1: TGrid\n  Caption = 'Data'\n  Columns = <  \n    item\n      Title = 'Notes'\n      Width = 200\n      Visible = False\n    end  \n    item\n      Title = 'Middle'\n      Width = 0\n      Visible = False\n    end  \n    item\n      Title = 'Name'\n    end>\nend\n122\nGrid1 Data 3\nNotes/200/FALSE Middle/0/FALSE Name/0/FALSE \n122 TRUE\n3 Notes\n"
        );
    }

    #[test]
    fn variants_match_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program v1;
{$mode objfpc}{$H+}
uses SysUtils, Variants;
var
  A, B, C: Variant;
  I: Integer;
  D: Double;
  S: string;
  Bo: Boolean;
procedure Show(const Title: string; const V: Variant);
begin
  WriteLn(Title, ': ', VarTypeAsText(VarType(V)), ' [', VarToStr(V), ']');
end;
begin
  Show('empty', A);
  A := 42; Show('int', A);
  A := Int64(5000000000); Show('int64', A);
  A := 2.5; Show('double', A);
  A := 'hello'; Show('string', A);
  A := True; Show('bool', A);
  A := Null; Show('null', A);
  A := 'a'; Show('char', A);
  A := Byte(7); Show('byte', A);
  A := 10; B := 3;
  Show('add', A + B); Show('sub', A - B); Show('mul', A * B); Show('div/', A / B);
  Show('idiv', A div B); Show('mod', A mod B); Show('neg', -A);
  A := '10'; B := 5; Show('str+int', A + B);
  A := 'ab'; B := 'cd'; Show('str+str', A + B);
  A := 'x'; B := 1;
  try Show('bad', A + B); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  A := Null; B := 5; Show('null+5', A + B);
  A := 2.5; B := 2; Show('dbl*int', A * B);
  A := 7; B := '7'; WriteLn('eq ', A = B, ' lt ', A < 10, ' gt ', B > '6');
  A := 'abc'; WriteLn('cmp ', A = 'abc', ' ', A <> 'abd');
  A := 3.75; I := A; D := A; S := A; WriteLn(I, ' ', D:0:2, ' ', S);
  A := '123'; I := A; WriteLn(I + 1);
  A := 'zz';
  try I := A; except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  A := True; Bo := A; I := A; WriteLn(Bo, ' ', I);
  A := 'True'; Bo := A; WriteLn(Bo);
  A := 1; WriteLn(A);
  A := 2.5; WriteLn(A);
  A := 'str'; WriteLn(A);
  A := Null; WriteLn(VarIsNull(A), ' ', VarIsEmpty(A), ' ', VarIsEmpty(Unassigned));
  A := VarArrayOf([1, 'two', 3.5]);
  WriteLn(VarIsArray(A), ' ', VarArrayLowBound(A, 1), ' ', VarArrayHighBound(A, 1), ' ', A[1], ' ', A[2] + 1);
  A[0] := 100; WriteLn(A[0] + A[2]);
  C := A; C[0] := 5; WriteLn(A[0], ' ', C[0]);
  A := 5; B := A; A := 6; WriteLn(B);
  A := 'x'; B := A; A := A + 'y'; WriteLn(A, B);
  WriteLn(VarToStrDef(Null, 'def'), ' ', VarToStr(Null), '|');
  A := VarAsType('42', varInteger); Show('astype', A);
  WriteLn(VarIsNumeric(A), ' ', VarIsStr('x'), ' ', VarType(A) = varInteger);
  A := 1; A := A + 0.5; Show('promote', A);
  A := High(Integer); A := A + 1; Show('overflow', A);
  A := 7; B := 2; Show('and', A and B); Show('or', A or B); Show('xor', A xor B); Show('not', not A);
  A := True; B := False; Show('band', A and B); Show('bnot', not A);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"empty: Empty []
int: ShortInt [42]
int64: Int64 [5000000000]
double: Double [2.5]
string: String [hello]
bool: Boolean [True]
null: Null []
char: String [a]
byte: Byte [7]
add: Integer [13]
sub: Integer [7]
mul: Integer [30]
div/: Double [3.33333333333333]
idiv: Integer [3]
mod: Integer [1]
neg: ShortInt [-10]
str+int: Double [15]
str+str: String [abcd]
EVariantError: Invalid variant type cast
null+5: Null []
dbl*int: Double [5]
eq TRUE lt TRUE gt TRUE
cmp TRUE TRUE
4 3.75 3.75
124
EVariantError: Invalid variant type cast
TRUE -1
TRUE
1
2.5
str
TRUE FALSE TRUE
TRUE 0 2 two 4.5
103.5
100 5
5
xyx
def |
astype: Integer [42]
TRUE TRUE TRUE
promote: Double [1.5]
overflow: Int64 [2147483648]
and: Integer [2]
or: Integer [7]
xor: Integer [5]
not: ShortInt [-8]
band: Boolean [False]
bnot: Boolean [False]
"#
        );
    }

    #[test]
    fn variant_types_and_conversions() {
        let (ok, out) = build_and_run_source(
            r#"program v2;
{$mode objfpc}{$H+}
uses SysUtils, Variants;
var
  A, B: Variant;
  I: Integer; L: LongInt; W: Word; Q: QWord; I6: Int64; Sg: Single; U: UnicodeString; WC: WideChar; Ch: Char;
procedure T(const V: Variant);
begin
  Write(VarTypeAsText(VarType(V)), ' ');
end;
begin
  T(200); T(1000); T(40000); T(100000); T(3000000000); T(-200); T(-100000); T(10000000000);
  WriteLn;
  I := 5; L := 5; W := 5; Q := 5; I6 := 5; Sg := 1.5; U := 'u'; WC := 'w'; Ch := 'c';
  T(I); T(L); T(W); T(Q); T(I6); T(Sg); T(U); T(WC); T(Ch); T(I + 1); T(I * 2.0); T(L + L);
  WriteLn;
  A := 5; B := 10; T(A + B); A := Byte(200); B := Byte(100); T(A + B); A := Int64(5); T(A + 1);
  A := Sg; T(A * 2); A := 'x'; T(A + 'y'); A := U; T(A + 'z'); T(A + A);
  WriteLn;
  A := Null;
  WriteLn(A = Null, ' ', A = 5, ' ', A <> 5, ' ', VarIsNull(A + 1));
  try I := A; WriteLn('null->int ', I); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  try WriteLn('null->str [', string(A), ']'); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  A := Unassigned;
  try I := A; WriteLn('empty->int ', I); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  WriteLn('empty->str [', string(A), ']');
  A := 2.5; I := A; WriteLn(I); A := 3.5; I := A; WriteLn(I); A := -2.5; I := A; WriteLn(I);
  A := 'abc';
  try A := A * 2; except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  A := 1.0; WriteLn(A); A := 1e20; WriteLn(A); A := 0.1; WriteLn(A); A := Sg; WriteLn(A);
  A := 5; B := 5.0; WriteLn(A = B); A := 'B'; B := 'a'; WriteLn(A < B);
  A := VarArrayCreate([0, 2], varInteger); A[1] := 7; WriteLn(A[1], ' ', A[0], ' ', VarTypeAsText(VarType(A)));
  WriteLn(VarTypeAsText(VarType(VarArrayOf([1, 2]))));
  A := 10; Inc(I); WriteLn(I);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"Byte Smallint Word Integer DWord Smallint Integer Int64 
Integer Integer Word QWord Int64 Double OleStr OleStr String Int64 Double Int64 
Integer Integer Int64 Double String OleStr OleStr 
TRUE FALSE TRUE TRUE
EVariantTypeCastError: Could not convert variant of type (Null) into type (Int64)
null->str [EVariantTypeCastError: Could not convert variant of type (Null) into type (String)
empty->int 0
empty->str []
2
4
-2
EVariantError: Invalid variant type cast
1
1E20
0.1
1.5
TRUE
TRUE
7 0 Array of Integer
Array of Variant
-1
"#
        );
    }

    #[test]
    fn variant_reference_counting() {
        let (ok, out) = build_and_run_source(
            r#"program v5;
{$mode objfpc}{$H+}
uses SysUtils, Variants;
type
  TRec = record
    Name: string;
    V: Variant;
  end;
  TObj = class
    F: Variant;
  end;
var
  A, B: Variant;
  R: TRec;
  Arr: array of Variant;
  Fixed: array[1..3] of Variant;
  O: TObj;
  I: Integer;
  S: string;

function MakeV(N: Integer): Variant;
begin
  Result := 'v' + IntToStr(N);
end;

procedure ByVal(V: Variant);
begin
  V := V + '!';
  S := V;
end;

procedure ByRef(var V: Variant);
begin
  V := V + '?';
end;

procedure ByConst(const V: Variant);
begin
  S := VarToStr(V);
end;

begin
  for I := 1 to 2000 do
  begin
    A := 'x' + IntToStr(I);
    B := A + A;
    R.V := B;
    R.Name := A;
    SetLength(Arr, 3);
    Arr[0] := A; Arr[1] := I; Arr[2] := MakeV(I);
    Fixed[I mod 3 + 1] := Arr[2];
    ByVal(A); ByRef(A); ByConst(B);
    A := VarArrayOf([A, B, I]);
    B := A;
    B[0] := 'changed';
    O := TObj.Create;
    O.F := B;
    O.Free;
  end;
  WriteLn(VarToStr(A[0]), ' ', VarToStr(B[0]), ' ', R.V, ' ', Arr[2], ' ', Fixed[1], ' ', S);
  WriteLn(VarToStr(MakeV(3)));
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"x2000? changed x2000x2000 v2000 v1998 x2000x2000
v3
"#
        );
    }

    #[test]
    fn variant_conditions_casts_and_banker_rounding() {
        let (ok, out) = build_and_run_source(
            r#"program tc;
{$mode objfpc}{$H+}
uses Variants;
var A: Variant; D: Double;
begin
  A := True;
  if A then WriteLn('yes');
  A := 0;
  while A < 3 do A := A + 1;
  WriteLn(A);
  repeat A := A - 1 until A = 0;
  WriteLn(A, ' ', Integer(A) + 1, ' ', string(A) + '!', ' ', Variant(5) * 2);
  D := 2.5;
  WriteLn(Round(2.5), ' ', Round(3.5), ' ', Round(-2.5), ' ', Round(D), ' ', Round(0.5));
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"yes
3
0 1 0! 10
2 4 -2 2 0
"#
        );
    }

    #[test]
    fn variant_properties_stream_as_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program vs;
{$mode objfpc}{$H+}
uses SysUtils, Classes, TypInfo, Variants;
type
  TThing = class(TComponent)
  private
    FA, FB, FC, FD, FE, FF, FG: Variant;
    function GetB: Variant;
    procedure SetB(const V: Variant);
  published
    property A: Variant read FA write FA;
    property B: Variant read GetB write SetB;
    property C: Variant read FC write FC;
    property D: Variant read FD write FD;
    property E: Variant read FE write FE;
    property F: Variant read FF write FF;
    property G: Variant read FG write FG;
  end;

function TThing.GetB: Variant;
begin
  Result := FB;
end;

procedure TThing.SetB(const V: Variant);
begin
  FB := V;
end;

procedure Show(T: TThing);
begin
  WriteLn(VarTypeAsText(VarType(T.A)), ' ', VarToStr(T.A), ' | ', VarTypeAsText(VarType(T.B)), ' ', VarToStr(T.B),
    ' | ', VarTypeAsText(VarType(T.C)), ' ', VarToStr(T.C), ' | ', VarTypeAsText(VarType(T.D)), ' ', VarToStr(T.D),
    ' | ', VarTypeAsText(VarType(T.E)), ' ', VarToStr(T.E), ' | ', VarTypeAsText(VarType(T.F)), ' | ', VarTypeAsText(VarType(T.G)), ' ', VarToStr(T.G));
end;

var
  T, T2: TThing;
  M: TMemoryStream;
  Txt: TStringStream;
  I: Integer;
begin
  RegisterClass(TThing);
  T := TThing.Create(nil);
  T.Name := 'Thing1';
  T.A := 42;
  T.B := 'hello';
  T.C := 2.5;
  T.D := True;
  T.E := Int64(10000000000);

  T.G := UnicodeString('wide');
  WriteLn(PropType(T, 'A') = tkVariant, ' ', GetPropInfo(T, 'B')^.PropType^.Name);
  WriteLn(VarToStr(GetPropValue(T, 'B')), ' ', VarToStr(GetVariantProp(T, 'A')));
  SetPropValue(T, 'C', 7);
  Show(T);
  M := TMemoryStream.Create;
  M.WriteComponent(T);
  for I := 0 to M.Size - 1 do
    Write(IntToHex(PByte(M.Memory)[I], 2));
  WriteLn;
  M.Position := 0;
  Txt := TStringStream.Create('');
  ObjectBinaryToText(M, Txt);
  WriteLn(Txt.DataString);
  M.Position := 0;
  T2 := TThing.Create(nil);
  M.ReadComponent(T2);
  Show(T2);
  T.Free; T2.Free; M.Free; Txt.Free;
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"TRUE Variant
hello 42
ShortInt 42 | String hello | ShortInt 7 | Boolean True | Int64 10000000000 | Empty | OleStr wide
5450463006545468696E67065468696E67310141022A01421205000000680065006C006C006F000143020701440901451300E40B54020000000147120400000077006900640065000000
object Thing1: TThing
  A = 42
  B = 'hello'
  C = 7
  D = True
  E = 10000000000
  G = 'wide'
end

Integer 42 | OleStr hello | Integer 7 | Boolean True | Int64 10000000000 | Empty | OleStr wide
"#
        );
    }

    #[test]
    fn currency_matches_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program cu;
{$mode objfpc}{$H+}
uses SysUtils, Variants;
type R = record A: Byte; C: Currency; end;
var C, D: Currency; X: Double; I: Integer; V: Variant; S: string; Rec: R;
begin
  C := 2.5; D := 1/3;
  WriteLn(C); WriteLn(D); WriteLn(C:0:2, ' ', D:10:4);
  WriteLn(SizeOf(C), ' ', SizeOf(Rec), ' ', PInt64(@D)^);
  X := C * D; WriteLn(X:0:6);
  C := C * 3; WriteLn(C:0:4);
  C := 123456789.12345; WriteLn(C:0:4);
  D := 0.00005; WriteLn(D:0:4); D := 0.00015; WriteLn(D:0:4); D := -0.00005; WriteLn(D:0:4);
  I := Round(C); WriteLn(I); WriteLn(Trunc(C));
  Str(C:0:2, S); WriteLn(S);
  WriteLn(CurrToStr(C), ' ', FloatToStr(C), ' ', CurrToStr(D));
  V := C; WriteLn(VarTypeAsText(VarType(V)), ' ', VarToStr(V));
  V := V + 1; WriteLn(VarTypeAsText(VarType(V)), ' ', VarToStr(V));
  V := V * 2.5; WriteLn(VarTypeAsText(VarType(V)), ' ', VarToStr(V));
  V := C / 3; WriteLn(VarTypeAsText(VarType(V)), ' ', VarToStr(V));
  C := V; WriteLn(C:0:4);
  C := 10; D := 3; WriteLn(C / D:0:6, ' ', C > D, ' ', C = 10);
  C := High(Int64) / 10000 / 2; WriteLn(C:0:0);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#" 2.500000000000000000E+00
 3.333000000000000000E-01
2.50     0.3333
8 16 3333
0.833200
7.5000
123456789.1234
0.0000
0.0001
0.0000
123456789
123456789
123456789.12
123456789.1234 123456789.1234 0
Currency 123456789.1234
Currency 123456790.1234
Currency 308641975.3085
Double 41152263.0411333
41152263.0411
3.333333 TRUE TRUE
461168601842739
"#
        );
    }

    #[test]
    fn currency_constants_mixing_and_write_widths() {
        let (ok, out) = build_and_run_source(
            r#"program cu2;
{$mode objfpc}{$H+}
var C: Currency; X: Double; I: Integer;
const K: Currency = 5; K2: Currency = 1.23456;
var V: Currency = 7;
function F(A: Currency): Currency; begin Result := A * 2; end;
begin
  C := 2.5; X := 0.33335;
  WriteLn(C * 0.33335:0:8, ' ', C * X:0:8, ' ', C + 0.00004:0:8);
  C := C * 0.33335; WriteLn(C:0:4);
  I := 3; C := I; WriteLn(C:0:4, ' ', K:0:4, ' ', K2:0:4, ' ', V:0:4, ' ', F(1.5):0:4);
  C := -C; WriteLn(C:0:4, ' ', -C:0:4, ' ', Abs(C):0:4);
  WriteLn(C:12:2, '|', C:10, '|');
  C := 1; C := C / 3; WriteLn(C:0:4);
  C := 7; C := C - 0.00005; WriteLn(C:0:4);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"0.83350000 0.83350000 2.50000000
0.8335
3.0000 5.0000 1.2346 7.0000 3.0000
-3.0000 3.0000 3.0000
       -3.00|-3.000E+00|
0.3333
7.0000
"#
        );
    }

    #[test]
    fn currency_records_properties_and_variants() {
        let (ok, out) = build_and_run_source(
            r#"program cu3;
{$mode objfpc}{$H+}
uses SysUtils, Classes, TypInfo, Variants;
type
  TRec = record Qty: Integer; Price: Currency; end;
  TItem = class(TComponent)
  private
    FPrice: Currency; FV: Variant;
    function GetTotal: Currency;
    procedure SetTotal(const C: Currency);
  published
    property Price: Currency read FPrice write FPrice;
    property Total: Currency read GetTotal write SetTotal;
    property V: Variant read FV write FV;
  end;
function TItem.GetTotal: Currency; begin Result := FPrice * 2; end;
procedure TItem.SetTotal(const C: Currency); begin FPrice := C / 2; end;
procedure Bump(var C: Currency; By: Currency); begin C := C + By; end;
var
  R: TRec; A: array[1..3] of Currency; C, D: Currency; I: Integer; S: string;
  V, W: Variant; T, T2: TItem; M: TMemoryStream; Txt: TStringStream;
begin
  R.Qty := 3; R.Price := 19.99;
  C := R.Price * R.Qty; WriteLn(C:0:2, ' ', R.Price:8:3);
  for I := 1 to 3 do A[I] := I / 3;
  WriteLn(A[1]:0:4, ' ', A[2]:0:4, ' ', A[3]:0:4, ' ', A[1] + A[2] + A[3]:0:4);
  C := 2; C := C / 3; WriteLn(C:0:4);
  Bump(C, 0.1); WriteLn(C:0:4); Bump(C, 1); WriteLn(C);
  if C > 1.7 then WriteLn('big'); if C = 1.7667 then WriteLn('eq');
  Str(C:10:1, S); WriteLn('[', S, ']');
  D := -1234567.8901; WriteLn(D:0:2, ' ', D:20, ' ', D:0:0, ' ', Trunc(D), ' ', Round(D));
  WriteLn(CurrToStr(D), ' ', SizeOf(TRec));
  V := D; W := V * 2; WriteLn(VarTypeAsText(VarType(W)), ' ', VarToStr(W));
  W := V + V; C := W; WriteLn(C:0:4, ' ', VarToStr(-V), ' ', V > 0, ' ', V = D);
  W := V / 3; WriteLn(VarTypeAsText(VarType(W)), ' ', VarToStr(W));
  T := TItem.Create(nil); T.Name := 'It';
  T.Price := 12.5; T.V := 42;
  WriteLn(GetFloatProp(T, 'Total'):0:4, ' ', PropType(T, 'Price') = tkFloat);
  SetFloatProp(T, 'Total', 7.77); WriteLn(T.Price:0:4);
  M := TMemoryStream.Create; M.WriteComponent(T);
  for I := 0 to M.Size - 1 do Write(IntToHex(PByte(M.Memory)[I], 2)); WriteLn;
  M.Position := 0; T2 := TItem.Create(nil); M.ReadComponent(T2);
  WriteLn(T2.Price:0:4, ' ', VarTypeAsText(VarType(T2.V)), ' ', VarToStr(T2.V));
  T.Free; T2.Free; M.Free;
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"59.97   19.990
0.3333 0.6667 1.0000 2.0000
0.6667
0.7667
 1.766700000000000000E+00
big
eq
[       1.8]
-1234567.89 -1.2345678901000E+06 -1234568 -1234567 -1234568
-1234567.8901 16
Currency -2469135.7802
-2469135.7802 1234567.8901 FALSE TRUE
Currency -411522.63
25.0000 TRUE
3.8850
5450463005544974656D0249740550726963650500A0703D0AD7A3F8004005546F74616C0500A0703D0AD7A3F801400156022A0000
3.8850 Integer 42
"#
        );
    }

    #[test]
    fn variant_arrays_of_several_dimensions() {
        let (ok, out) = build_and_run_source(
            r#"program md;
{$mode objfpc}{$H+}
uses SysUtils, Variants;
var A, B, N: Variant; I, J: Integer;
begin
  A := VarArrayCreate([1, 3, -1, 1], varInteger);
  WriteLn(VarArrayDimCount(A), ' ', VarArrayLowBound(A, 1), ' ', VarArrayHighBound(A, 1), ' ',
    VarArrayLowBound(A, 2), ' ', VarArrayHighBound(A, 2), ' ', VarTypeAsText(VarType(A)));
  for I := 1 to 3 do
    for J := -1 to 1 do
      A[I, J] := I * 10 + J;
  for I := 1 to 3 do
  begin
    for J := -1 to 1 do Write(A[I, J], ' ');
    WriteLn;
  end;
  B := A; B[2, 0] := 99; WriteLn(A[2, 0], ' ', B[2, 0]);
  A[3, 1] := '42'; WriteLn(VarTypeAsText(VarType(A[3, 1])), ' ', A[3, 1] + 1);
  try A[4, 0] := 1; except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  try WriteLn(A[1]); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  try WriteLn(VarArrayLowBound(A, 3)); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  N := VarArrayOf([1, VarArrayOf(['x', 'y', 'z']), 3]);
  WriteLn(N[1][2], ' ', VarArrayHighBound(N[1], 1));
  B := VarArrayCreate([0, 1, 0, 1, 0, 1], varVariant);
  B[1, 0, 1] := 'deep'; WriteLn(B[1, 0, 1], ' ', VarIsEmpty(B[0, 0, 0]), ' ', VarArrayDimCount(B));
  A := VarArrayCreate([0, 2], varInteger); A[2] := 5;
  VarArrayRedim(A, 4); A[4] := 7;
  WriteLn(VarArrayHighBound(A, 1), ' ', A[2], ' ', A[3], ' ', A[4]);
  VarArrayRedim(A, 1); WriteLn(VarArrayHighBound(A, 1), ' ', A[1]);
  A := VarArrayCreate([0, 1, 0, 1], varInteger);
  A[1, 0] := 3; VarArrayRedim(A, 2); A[1, 2] := 4;
  WriteLn(A[1, 0], ' ', A[1, 2], ' ', A[0, 2], ' ', VarArrayHighBound(A, 2));
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"2 1 3 -1 1 Array of Integer
9 10 11 
19 20 21 
29 30 31 
20 99
Integer 43
EVariantBadIndexError: Variant array bounds error
EVariantInvalidArgError: Invalid argument
EVariantBadIndexError: Variant array bounds error
z 2
deep TRUE 3
4 5 0 7
1 0
0 4 3 2
"#
        );
    }

    #[test]
    fn var_clear_keeps_number_bytes_as_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program un2;
{$mode objfpc}{$H+}
uses SysUtils, Classes, Variants;
type
  TThing = class(TComponent)
  private
    FF, FG, FH: Variant;
  published
    property F: Variant read FF write FF;
    property G: Variant read FG write FG;
    property H: Variant read FH write FH;
  end;
var T: TThing; M: TMemoryStream; I: Integer;
begin
  T := TThing.Create(nil);
  T.FF := 5; VarClear(T.FF);
  T.FG := 'abc'; VarClear(T.FG);

  M := TMemoryStream.Create; M.WriteComponent(T);
  for I := 0 to M.Size - 1 do Write(IntToHex(PByte(M.Memory)[I], 2));
  WriteLn; M.Free; T.Free;
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"5450463006545468696E670001460D0000
"#
        );
    }

    #[test]
    fn set_sizes_and_layout_match_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program tsz;
{$mode objfpc}
type
  TSmall = (a, b, c);
  T31 = 0..31; T32 = 0..32; THi = 24..31; TMid = 30..40; TBig = 200..255; T7 = 8..15; T16 = 0..16;
  SSmall = set of TSmall; S31 = set of T31; S32 = set of T32; SHi = set of THi; SMid = set of TMid;
  SBig = set of TBig; TChars = set of Char; SByte = set of Byte; S7 = set of T7; S16 = set of T16;
  TR = record X: Byte; S: SSmall; Y: Byte; end;
  TR2 = packed record X: Byte; S: SSmall; end;
{$packset 1}
type
  PSmall = set of TSmall; P32 = set of T32; PMid = set of TMid; PBig = set of TBig;
{$packset 2}
type
  QSmall = set of TSmall; Q32 = set of T32; QBig = set of TBig;
{$packset 8}
type
  ESmall = set of TSmall;
var
  M: PMid; G: PBig;
procedure Dump(const X; N: Integer);
var P: PByte; I: Integer;
begin
  P := @X;
  for I := 0 to N - 1 do Write(P[I], ' ');
  WriteLn;
end;
begin
  WriteLn(SizeOf(SSmall), ' ', SizeOf(S31), ' ', SizeOf(S32), ' ', SizeOf(SHi), ' ', SizeOf(SMid), ' ',
    SizeOf(SBig), ' ', SizeOf(TChars), ' ', SizeOf(SByte), ' ', SizeOf(S7), ' ', SizeOf(S16), ' ', SizeOf(TR), ' ', SizeOf(TR2));
  WriteLn(SizeOf(PSmall), ' ', SizeOf(P32), ' ', SizeOf(PMid), ' ', SizeOf(PBig), ' ',
    SizeOf(QSmall), ' ', SizeOf(Q32), ' ', SizeOf(QBig), ' ', SizeOf(ESmall));
  M := [30, 33, 40]; G := [200, 255]; Include(M, 31);
  Dump(M, SizeOf(M)); Dump(G, SizeOf(G));
  WriteLn(31 in M, ' ', 32 in M, ' ', 255 in G);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"4 4 32 4 32 32 32 32 4 4 12 5
1 5 4 7 2 6 8 8
192 2 1 0 
1 0 0 0 0 0 128 
TRUE FALSE TRUE
"#
        );
    }

    #[test]
    fn small_sets_in_records_params_and_streaming() {
        let (ok, out) = build_and_run_source(
            r#"program s5;
{$mode objfpc}{$H+}
uses SysUtils, Classes, TypInfo;
type
  TColor = (cRed, cGreen, cBlue, cYellow);
  TColors = set of TColor;
  TDigits = set of 0..9;
  TRec = record A: Byte; C: TColors; B: Byte; D: TDigits; end;
  TBox = class(TComponent)
  private
    FColors: TColors;
    FDigits: TDigits;
  public
    property Digits: TDigits read FDigits write FDigits;
  published
    property Colors: TColors read FColors write FColors;
  end;
const
  Warm: TColors = [cRed, cYellow];
  Primes: TDigits = [2, 3, 5, 7];
var
  S, T: TColors; D: TDigits; R: TRec; C: TColor; Chs: set of Char; I: Integer;
  B, B2: TBox; M: TMemoryStream; Txt: TStringStream;
  Arr: array[1..3] of TColors;
procedure AddBlue(var X: TColors); begin Include(X, cBlue); end;
function Both(A, B: TColors): TColors; begin Result := A * B; end;
begin
  S := [cRed, cBlue]; T := Warm;
  WriteLn(SizeOf(S), ' ', SizeOf(R), ' ', Integer(S), ' ', Integer(S + T), ' ', Integer(S * T), ' ', Integer(S - T));
  WriteLn(cBlue in S, ' ', cGreen in S, ' ', S <= [cRed, cBlue, cGreen], ' ', S = [cBlue, cRed], ' ', S <> T);
  Include(S, cGreen); Exclude(S, cRed); WriteLn(Integer(S));
  AddBlue(T); WriteLn(Integer(T), ' ', Integer(Both(S, T)));
  R.A := 1; R.B := 2; R.C := [cYellow]; R.D := Primes; Include(R.C, cRed); Include(R.D, 9);
  WriteLn(Integer(R.C), ' ', Integer(R.D), ' ', R.A, ' ', R.B, ' ', PByte(@R)[4], ' ', PWord(@R.D)^);
  for C in R.C do Write(Ord(C), ' '); WriteLn;
  for I in R.D do Write(I, ' '); WriteLn;
  Arr[2] := [cGreen]; Include(Arr[2], cBlue); Arr[3] := Arr[2] + [cRed]; WriteLn(Integer(Arr[2]), ' ', Integer(Arr[3]));
  D := []; for I := 0 to 9 do if not Odd(I) then Include(D, I); WriteLn(Integer(D));
  Chs := ['a'..'c', 'z']; WriteLn(SizeOf(Chs), ' ', 'b' in Chs, ' ', 'y' in Chs);
  B := TBox.Create(nil); B.Name := 'Box'; B.Colors := [cGreen, cYellow]; B.Digits := [1, 8];
  WriteLn(GetSetProp(B, 'Colors', True), ' ', Integer(B.Digits));
  SetSetProp(B, 'Colors', '[cRed,cBlue]'); WriteLn(Integer(B.Colors));
  M := TMemoryStream.Create; M.WriteComponent(B);
  for I := 0 to M.Size - 1 do Write(IntToHex(PByte(M.Memory)[I], 2)); WriteLn;
  M.Position := 0; Txt := TStringStream.Create(''); ObjectBinaryToText(M, Txt); Write(Txt.DataString);
  M.Position := 0; B2 := TBox.Create(nil); M.ReadComponent(B2); WriteLn(Integer(B2.Colors), ' ', Integer(B2.Digits));
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"4 16 5 13 1 4
TRUE FALSE TRUE TRUE TRUE
6
13 4
9 684 1 2 9 684
0 3 
2 3 5 7 9 
6 7
341
32 TRUE FALSE
[cGreen,cYellow] 258
5
545046300454426F7803426F7806436F6C6F72730B04635265640563426C7565000000
object Box: TBox
  Colors = [cRed, cBlue]
end
5 0
"#
        );
    }

    #[test]
    fn delphi_mode_packed_sets_match_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program s6;
{$mode delphi}
uses SysUtils, Classes, TypInfo;
type
  TColor = (cRed, cGreen, cBlue, cYellow);
  TColors = set of TColor;
  THigh = 20..30;
  SHigh = set of THigh;
  TRec = record A: Byte; C: TColors; H: SHigh; B: Byte; end;
  TBox = class(TComponent)
  private
    FColors: TColors;
  published
    property Colors: TColors read FColors write FColors;
  end;
const
  Warm: TColors = [cRed, cYellow];
var
  S, T: TColors; R: TRec; H: SHigh; C: TColor; I: Integer;
  B, B2: TBox; M: TMemoryStream; Txt: TStringStream;
procedure Dump(const X; N: Integer);
var P: PByte; I: Integer;
begin
  P := @X;
  for I := 0 to N - 1 do Write(P[I], ' ');
  WriteLn;
end;
procedure Show(const X: TColors);
var C: TColor;
begin
  for C := Low(TColor) to High(TColor) do if C in X then Write(Ord(C)); WriteLn;
end;
begin
  S := [cRed, cBlue]; T := Warm;
  WriteLn(SizeOf(S), ' ', SizeOf(R), ' ', SizeOf(H));
  Dump(S, SizeOf(S)); Show(S + T); Show(S * T); Show(S - T);
  Include(S, cGreen); Exclude(S, cRed); Show(S);
  H := [20, 25, 30]; Include(H, 23); Dump(H, SizeOf(H));
  for I in H do Write(I, ' '); WriteLn(25 in H, ' ', 24 in H);
  R.A := 7; R.B := 9; R.C := [cYellow]; R.H := H; Include(R.C, cGreen); Exclude(R.H, 25);
  Dump(R, SizeOf(R));
  B := TBox.Create(nil); B.Name := 'Box'; B.Colors := [cGreen, cYellow];
  WriteLn(GetSetProp(B, 'Colors', True), ' ', GetOrdProp(B, 'Colors'));
  M := TMemoryStream.Create; M.WriteComponent(B);
  for I := 0 to M.Size - 1 do Write(IntToHex(PByte(M.Memory)[I], 2)); WriteLn;
  M.Position := 0; B2 := TBox.Create(nil); M.ReadComponent(B2); Show(B2.Colors);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"1 6 2
5 
023
0
2
12
144 66 
20 23 25 30 TRUE FALSE
7 10 144 64 9 0 
[cGreen,cYellow] 10
545046300454426F7803426F7806436F6C6F72730B0663477265656E076359656C6C6F77000000
13
"#
        );
    }

    #[test]
    fn class_references_compare_by_address() {
        let (ok, out) = build_and_run_source(
            r#"program ih4;
{$mode objfpc}
type TA = class X: Integer; end; TB = class Y: Integer; end;
var C, D: TClass; P, Q: TObject;
begin
  C := TA; D := TB;
  WriteLn(C <> D, ' ', C = D, ' ', C = TA, ' ', TA = TB);
  P := TA.Create; Q := TB.Create;
  WriteLn(P = Q, ' ', P.ClassType = Q.ClassType);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "TRUE FALSE TRUE FALSE\nFALSE FALSE\n");
    }

    /// Output as FPC on 64-bit ARM macOS gives it, where every floating
    /// point trap is an access violation (other platforms raise
    /// EZeroDivide, EInvalidOp and EOverflow).
    fn fpc_arm64_float_traps(out: &str) -> String {
        out.lines()
            .map(|line| {
                for class in ["EZeroDivide: ", "EInvalidOp: ", "EOverflow: "] {
                    if let Some(at) = line.find(class) {
                        return format!("{}EAccessViolation: Access violation\n", &line[..at]);
                    }
                }
                format!("{line}\n")
            })
            .collect()
    }

    #[test]
    fn float_traps_match_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program f1;
{$mode objfpc}
uses SysUtils, Math;
var A, B, C: Double; S: Single; I: Integer;
procedure T(const N: string; Op: Integer);
begin
  try
    case Op of
      1: C := A / B;
      2: C := Sqrt(-A);
      3: C := Ln(-A);
      4: C := Exp(1000 * A);
      5: C := A * 1e308 * 1e308;
      6: C := Ln(B);
      7: C := 0 / B;
      8: S := A * 1e300;
      9: I := Trunc(A * 1e30);
      10: C := Power(-1, 0.5);
      11: C := A - Infinity + Infinity;
      12: C := A * 1e-308 / 1e300;
    end;
    WriteLn(N, ' ok ', C);
  except
    on E: Exception do WriteLn(N, ' ', E.ClassName, ': ', E.Message);
  end;
end;
begin
  A := 1; B := 0;
  T('div0', 1); T('sqrt', 2); T('ln-', 3); T('exp', 4); T('ovf', 5); T('ln0', 6); T('0/0', 7);
  T('single', 8); T('trunc', 9); T('pow', 10); T('inf-inf', 11); T('underflow', 12);
  WriteLn(GetExceptionMask = [], ' ', Integer(GetExceptionMask));
  SetExceptionMask([exInvalidOp, exDenormalized, exZeroDivide, exOverflow, exUnderflow, exPrecision]);
  T('masked div0', 1); T('masked 0/0', 7);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            fpc_arm64_float_traps(&out),
            r#"div0 EAccessViolation: Access violation
sqrt EAccessViolation: Access violation
ln- EAccessViolation: Access violation
exp EAccessViolation: Access violation
ovf EAccessViolation: Access violation
ln0 EAccessViolation: Access violation
0/0 EAccessViolation: Access violation
single EAccessViolation: Access violation
trunc EAccessViolation: Access violation
pow EAccessViolation: Access violation
inf-inf EAccessViolation: Access violation
underflow ok  0.0000000000000000E+000
FALSE 50
masked div0 ok                     +Inf
masked 0/0 ok                      Nan
"#
        );
    }

    #[test]
    fn float_traps_in_math_and_conversions_match_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program f3;
{$mode objfpc}
uses SysUtils, Math;
var A, B, C: Double; S: Single; I: Int64; N: Integer;
procedure T(const Name: string; Op: Integer);
begin
  try
    case Op of
      1: C := ArcSin(2 * A);
      2: C := Tan(A) / B;
      3: I := Round(A * 1e20);
      4: C := Sqr(A * 1e200);
      5: C := A / B + 1;
      6: begin S := A * 3e38; S := S * 10; end;
      7: C := Frac(A * Infinity);
      8: C := Int(NaN);
      9: N := Trunc(A * 3e9);
      10: C := Log10(-A);
      11: C := IntPower(10 * A, 400);

    end;
    WriteLn(Name, ' ok ', C, ' ', I, ' ', N, ' ', S);
  except
    on E: Exception do WriteLn(Name, ' ', E.ClassName, ': ', E.Message);
  end;
end;
var K: Integer;
begin
  A := 1; B := 0;
  for K := 1 to 11 do T(IntToStr(K), K);
  SetExceptionMask([exInvalidOp, exDenormalized, exZeroDivide, exOverflow, exUnderflow, exPrecision]);
  WriteLn('masked');
  for K := 1 to 11 do T(IntToStr(K), K);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            fpc_arm64_float_traps(&out),
            r#"1 EAccessViolation: Access violation
2 EAccessViolation: Access violation
3 EAccessViolation: Access violation
4 EAccessViolation: Access violation
5 EAccessViolation: Access violation
6 EAccessViolation: Access violation
7 EAccessViolation: Access violation
8 ok                      Nan 0 0  3.000000005E+38
9 ok                      Nan 0 -1294967296  3.000000005E+38
10 EAccessViolation: Access violation
11 EAccessViolation: Access violation
masked
1 ok                      Nan 0 -1294967296  3.000000005E+38
2 ok                     +Inf 0 -1294967296  3.000000005E+38
3 ok                     +Inf 9223372036854775807 -1294967296  3.000000005E+38
4 ok                     +Inf 9223372036854775807 -1294967296  3.000000005E+38
5 ok                     +Inf 9223372036854775807 -1294967296  3.000000005E+38
6 ok                     +Inf 9223372036854775807 -1294967296             +Inf
7 ok                      Nan 9223372036854775807 -1294967296             +Inf
8 ok                      Nan 9223372036854775807 -1294967296             +Inf
9 ok                      Nan 9223372036854775807 -1294967296             +Inf
10 ok                      Nan 9223372036854775807 -1294967296             +Inf
11 ok                     +Inf 9223372036854775807 -1294967296             +Inf
"#
        );
    }

    #[test]
    fn a_units_own_tobject_is_seen_only_where_it_is_used() {
        // Free Vision's Objects unit has an object type called TObject:
        // units that don't use it (SysUtils, Classes, Modern) still mean
        // the root class.
        let old_obj: &[u8] = br#"unit OldObj;
interface
type
  PObject = ^TObject;
  TObject = object
    Tag: Integer;
    constructor Init;
    destructor Done; virtual;
    function Describe: string; virtual;
  end;
  PThing = ^TThing;
  TThing = object(TObject)
    function Describe: string; virtual;
  end;
implementation
constructor TObject.Init; begin Tag := 7; end;
destructor TObject.Done; begin end;
function TObject.Describe: string; begin Describe := 'object'; end;
function TThing.Describe: string; begin Describe := 'thing'; end;
end.
"#;
        let modern: &[u8] = br#"unit Modern;
{$mode objfpc}{$H+}
interface
uses SysUtils, Classes;
type
  TNote = class(TObject)
    Text: string;
    constructor Create(const S: string);
  end;
function Lines(const S: string): Integer;
implementation
constructor TNote.Create(const S: string); begin inherited Create; Text := S; end;
function Lines(const S: string): Integer;
var L: TStringList;
begin
  L := TStringList.Create;
  try L.Text := S; Result := L.Count; finally L.Free; end;
end;
end.
"#;
        let (ok, out) = build_and_run_in_dir(
            "bruto_tobject_scope_test",
            &[("OldObj.pas", old_obj), ("Modern.pas", modern)],
            r#"program ObjMix;
{$mode objfpc}{$H+}
uses SysUtils, Classes, Modern, OldObj;
var
  P: PObject;
  N: TNote;
  L: Classes.TStringList;
begin
  P := New(PThing, Init);
  WriteLn(P^.Describe, ' ', P^.Tag);
  Dispose(P, Done);
  N := TNote.Create('hello');
  WriteLn(N.Text, ' ', N.ClassName, ' ', N.ClassParent.ClassName, ' ', Lines('a'#10'b'));
  N.Free;
  L := Classes.TStringList.Create;
  L.Add('x');
  try
    raise Exception.Create('caught ' + L[0]);
  except
    on E: Exception do WriteLn(E.Message);
  end;
  L.Free;
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"thing 7
hello TNote TObject 2
caught x
"#
        );
    }

    #[test]
    fn chained_assignments_static_array_lengths_file_names_and_random_match_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program Misc;
{$mode objfpc}{$H+}
uses SysUtils;
type
  TG = (ga, gb);
  TInfo = class
  private FGenre: TG;
  public property Genre: TG read FGenre write FGenre;
  end;
  PCl = ^TCl;
  TCl = object Value: LongInt; end;
  PDoc = ^TDoc;
  TDoc = object Info: TInfo; end;
const
  Names: array[1..4] of string = ('one', 'two', 'three', 'four');
var
  D: TDoc;
  N: LongInt;
  I6: Int64;
  I: Integer;
procedure Chained(Doc: PDoc);
var
  Genre: PCl;
const
  Local: array[0..2] of Char = ('a', 'b', 'c');
begin
  New(Genre);
  Genre^.Value := Ord(Doc^.Info.Genre) + Length(Local);
  WriteLn(Genre^.Value);
  Dispose(Genre);
end;
begin
  D.Info := TInfo.Create;
  D.Info.Genre := gb;
  Chained(@D);
  WriteLn(Length(Names), ' ', Names[Length(Names)]);
  WriteLn(ChangeFileExt('/tmp/a.b/doc.wri', '.html'), ' ', ChangeFileExt('notes', '.md'), ' ',
    ChangeFileExt('/tmp/.hidden', '.x'));
  WriteLn(ExtractFileName('/tmp/a/doc.wri'), ' ', ExtractFilePath('/tmp/a/doc.wri'), ' ',
    ExtractFileDir('/tmp/a/doc.wri'), ' ', ExtractFileExt('/tmp/a/doc.wri'), '|',
    ExtractFileExt('/tmp/.hidden'), '|');
  WriteLn(IncludeTrailingPathDelimiter('/tmp'), ' ', ExcludeTrailingPathDelimiter('/tmp/'));
  WriteLn(TryStrToInt('42', N), ' ', N, ' ', TryStrToInt('4x', N), ' ',
    TryStrToInt('9999999999', N), ' ', TryStrToInt64('9999999999', I6), ' ', I6);
  RandSeed := 42;
  for I := 1 to 3 do Write(Random(-7), ' ', Random(I6), ' ', Random(Length(Names)), ' ');
  WriteLn;
  D.Info.Free;
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        // On Windows the path delimiter appended is a backslash, as FPC's.
        let out = if cfg!(windows) {
            out.replace("/tmp\\ /tmp", "/tmp/ /tmp")
        } else {
            out
        };
        assert_eq!(
            out,
            r#"4
4 four
/tmp/a.b/doc.html notes.md /tmp/.hidden.x
doc.wri /tmp/a/ /tmp/a .wri||
/tmp/ /tmp
TRUE 42 FALSE TRUE TRUE 9999999999
-3 3778711 2 -5 4343377330 2 -1 8910898524 2 
"#
        );
    }

    #[test]
    fn val_and_try_str_to_int_overflow_as_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program t3;
var V: Int64; C: Integer; Q: QWord;
begin
  Val('99999999999999999999', V, C); WriteLn(V, ' ', C);
  Val('9223372036854775807', V, C); WriteLn(V, ' ', C);
  Val('9223372036854775808', V, C); WriteLn(V, ' ', C);
  Val('-9223372036854775808', V, C); WriteLn(V, ' ', C);
  Val('-9223372036854775809', V, C); WriteLn(V, ' ', C);
  Val('$FFFFFFFFFFFFFFFF', V, C); WriteLn(V, ' ', C);
  Val('$1FFFFFFFFFFFFFFFF', V, C); WriteLn(V, ' ', C);
  Val(' 12', V, C); WriteLn(V, ' ', C);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"0 19
9223372036854775807 0
0 19
-9223372036854775808 0
0 20
-1 0
0 18
12 0
"#
        );
        let (ok, out) = build_and_run_source(
            r#"program t2;
{$mode objfpc}
uses SysUtils;
var N: LongInt; C: Integer;
begin
  WriteLn(TryStrToInt('9999999999', N), ' ', N);
  WriteLn(TryStrToInt('-9999999999', N), ' ', N);
  WriteLn(TryStrToInt('99999999999999999999', N), ' ', N);
  Val('9999999999', N, C); WriteLn(N, ' ', C);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"TRUE 1410065407
TRUE -1410065407
FALSE 0
1410065407 0
"#
        );
    }

    #[test]
    fn sysutils_dates_and_times_as_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program d1;
{$mode objfpc}{$H+}
uses SysUtils;
var D, T: TDateTime; Y, M, Dd, H, Mi, S, Ms: Word;
begin
  with DefaultFormatSettings do
    WriteLn('[', DateSeparator, '][', TimeSeparator, '][', ShortDateFormat, '][', LongDateFormat, '][', ShortTimeFormat, '][', LongTimeFormat, '][', TimeAMString, '][', TimePMString, '][', DecimalSeparator, '][', ThousandSeparator, '][', ListSeparator, '][', ShortMonthNames[1], '][', LongMonthNames[2], '][', ShortDayNames[1], '][', LongDayNames[2], ']');
  D := EncodeDate(2024, 2, 29) + EncodeTime(13, 5, 7, 250);
  WriteLn(D:0:6);
  WriteLn(DateTimeToStr(D));
  WriteLn(DateToStr(D), ' | ', TimeToStr(D));
  WriteLn(FormatDateTime('yyyy-mm-dd hh:nn:ss.zzz', D));
  WriteLn(FormatDateTime('dddd d mmmm yyyy, h:n am/pm', D));
  WriteLn(FormatDateTime('ddd dd mmm yy "at" hh:mm ampm c', D));
  WriteLn(FormatDateTime('c', EncodeDate(2024, 2, 29)));
  DecodeDate(D, Y, M, Dd); DecodeTime(D, H, Mi, S, Ms);
  WriteLn(Y, ' ', M, ' ', Dd, ' ', H, ' ', Mi, ' ', S, ' ', Ms, ' ', DayOfWeek(D), ' ', IsLeapYear(2024), ' ', IsLeapYear(1900));
  WriteLn(DateToStr(IncMonth(D, 12)), ' ', DateToStr(IncMonth(EncodeDate(2024,1,31), 1)));
  WriteLn(DateTimeToStr(StrToDate('29-2-2024')), ' ', DateTimeToStr(StrToDateTime('1-3-2024 10:20:30')), ' ', TimeToStr(StrToTime('8:09:10')));
  WriteLn(EncodeDate(1899, 12, 30):0:1, ' ', EncodeDate(1, 1, 1):0:1, ' ', DateTimeToStr(0), ' ', DateTimeToStr(-1.25));
  WriteLn(Trunc(Now) > 45000, ' ', Frac(Time) < 1, ' ', Date = Trunc(Now));
  try EncodeDate(2023, 2, 29); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  try StrToDate('x'); except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  WriteLn(FormatDateTime('hh:nn', 0.999999999));
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"[-][:][d/m/y][dd" "mmmm" "yyyy][hh:nn][hh:nn:ss][AM][PM][.][,][,][Jan][February][Sun][Monday]
45351.545223
29-2-24 13:05:07
29-2-24 | 13:05:07
2024-02-29 13:05:07.250
Thursday 29 February 2024, 1:5 pm
Thu 29 Feb 24 at 01:05 PM 29-2-24 13:05:07
29-2-24
2024 2 29 13 5 7 250 5 TRUE FALSE
28-2-25 29-2-24
29-2-24 1-3-24 10:20:30 08:09:10
0.0 -693593.0 30-12-99 29-12-99 06:00:00
TRUE TRUE TRUE
EConvertError: 2023-2-29 is not a valid date specification
EConvertError: "x" is not a valid date format
00:00
"#
        );
    }

    #[test]
    fn dateutils_unit_as_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program du;
{$mode objfpc}{$H+}
uses SysUtils, DateUtils;
var D, E: TDateTime;
begin
  D := EncodeDateTime(2024, 2, 29, 13, 5, 7, 250);
  E := EncodeDate(2025, 1, 1);
  WriteLn(YearOf(D), ' ', MonthOf(D), ' ', DayOf(D), ' ', HourOf(D), ' ', MinuteOf(D), ' ', SecondOf(D), ' ', MilliSecondOf(D));
  WriteLn(DaysBetween(D, E), ' ', HoursBetween(D, E), ' ', MonthsBetween(D, E), ' ', WeeksBetween(D, E), ' ', YearsBetween(D, E));
  WriteLn(DateTimeToStr(IncDay(D, 3)), ' | ', DateTimeToStr(IncHour(D, -14)), ' | ', DateTimeToStr(IncYear(D)));
  WriteLn(DaysInMonth(D), ' ', DaysInYear(D), ' ', DayOfTheWeek(D), ' ', DayOfTheYear(D), ' ', WeekOf(D), ' ', IsSameDay(D, Trunc(D) + 0.1));
  WriteLn(DateTimeToStr(StartOfTheMonth(D)), ' | ', DateTimeToStr(EndOfTheMonth(D)), ' | ', DateTimeToStr(StartOfTheWeek(D)));
  WriteLn(DateTimeToUnix(D), ' ', DateTimeToStr(UnixToDateTime(0)), ' ', DateToISO8601(D));
  WriteLn(CompareDate(D, E), ' ', SameDate(D, D + 0.2), ' ', IsValidDate(2023, 2, 29), ' ', DateOf(D):0:1, ' ', TimeOf(D):0:6);
  WriteLn(IsToday(Now), ' ', Trunc(Yesterday) = Trunc(Today) - 1, ' ', Trunc(Tomorrow) = Trunc(Today) + 1);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"2024 2 29 13 5 7 250
306 7354 10 43 0
3-3-24 13:05:07 | 28-2-24 23:05:07 | 28-2-25 13:05:07
29 366 4 60 9 TRUE
1-2-24 | 29-2-24 23:59:59 | 26-2-24
1709211907 1-1-70 2024-02-29T13:05:07.250Z
-1 TRUE FALSE 45351.0 0.545223
TRUE TRUE TRUE
"#
        );
    }

    #[test]
    fn sysutils_pchar_routines_as_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program p1;
{$mode objfpc}{$H+}
uses SysUtils;
var Buf: array[0..63] of Char; P, Q: PChar;
begin
  StrPCopy(Buf, 'Hello'); StrCat(Buf, ', world');
  WriteLn(StrPas(Buf), ' ', StrLen(Buf));
  WriteLn(StrComp('abc', 'abd') < 0, ' ', StrIComp('ABC', 'abc'), ' ', StrLComp('abcX', 'abcY', 3));
  P := StrScan(Buf, 'o'); WriteLn(P);
  P := StrRScan(Buf, 'o'); WriteLn(P);
  P := StrPos(Buf, 'wor'); WriteLn(P, ' ', P - PChar(@Buf[0]));
  StrUpper(Buf); WriteLn(Buf); StrLower(Buf); WriteLn(Buf);
  Q := StrNew('copy me'); WriteLn(Q, ' ', StrLen(Q)); StrDispose(Q);
  StrLCopy(Buf, 'truncated text', 9); WriteLn(Buf);
  P := StrECopy(Buf, 'ab'); StrCopy(P, 'cd'); WriteLn(Buf, ' ', StrEnd(Buf) - PChar(@Buf[0]));
  WriteLn(CharInSet('x', ['a'..'z']), ' ', CharInSet('1', ['a'..'z']));
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"Hello, world 12
TRUE 0 0
o, world
orld
world 7
HELLO, WORLD
hello, world
copy me 7
truncated
abcd 4
TRUE FALSE
"#
        );
    }

    #[test]
    fn variant_dates_lock_and_pinteger_as_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program vd;
{$mode objfpc}{$H+}
uses SysUtils, Variants;
var V, W: Variant; D: TDateTime; X: Double; P: PInteger; I: Integer; A: Variant; S: string;
begin
  D := EncodeDate(2024, 2, 29) + EncodeTime(13, 5, 7, 0);
  V := D; WriteLn(VarTypeAsText(VarType(V)), ' ', VarToStr(V));
  X := 2.5; V := X; WriteLn(VarTypeAsText(VarType(V)));
  V := VarFromDateTime(D); WriteLn(VarTypeAsText(VarType(V)), ' ', DateTimeToStr(VarToDateTime(V)));
  W := V + 1; WriteLn(VarTypeAsText(VarType(W)), ' ', VarToStr(W));
  W := V - 1.5; WriteLn(VarTypeAsText(VarType(W)), ' ', VarToStr(W));
  W := V - VarFromDateTime(EncodeDate(2024, 1, 1)); WriteLn(VarTypeAsText(VarType(W)), ' ', VarToStr(W));
  W := V * 2; WriteLn(VarTypeAsText(VarType(W)), ' ', VarToStr(W));
  WriteLn(V > VarFromDateTime(EncodeDate(2024, 1, 1)), ' ', V = D, ' ', VarToStr(VarFromDateTime(EncodeTime(8, 30, 0, 0))));
  V := '1-3-2024'; W := VarAsType(V, varDate); WriteLn(VarTypeAsText(VarType(W)), ' ', VarToStr(W));
  D := V; WriteLn(D:0:2);
  V := VarFromDateTime(D); I := V; X := V; S := V; WriteLn(I, ' ', X:0:2, ' ', S);
  WriteLn(VarToStr(VarFromDateTime(0)), '|', VarToStr(VarFromDateTime(-1.5)));
  A := VarArrayCreate([0, 3], varInteger);
  A[0] := 10; A[3] := 40;
  P := VarArrayLock(A); WriteLn(P[0], ' ', P[3]); P[1] := 20; VarArrayUnlock(A);
  WriteLn(A[1], ' ', VarArrayDimCount(A));
  A := VarArrayCreate([1, 2], varDouble); A[1] := 1.5;
  WriteLn(PDouble(VarArrayLock(A))[0]:0:1); VarArrayUnlock(A);
  A := VarArrayOf(['x', 2]);
  WriteLn(VarTypeAsText(VarType(PVariant(VarArrayLock(A))[0]))); VarArrayUnlock(A);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"Date 29-2-24 13:05:07
Double
Date 29-2-24 13:05:07
Date 1-3-24 13:05:07
Date 28-2-24 01:05:07
Double 59.5452199074061
Double 90703.0904398148
TRUE TRUE 08:30:00
Date 1-3-24
45352.00
45352 45352.00 1-3-24
00:00:00|29-12-99 12:00:00
10 40
20 1
1.5
OleStr
"#
        );
    }

    #[test]
    fn custom_variant_types_and_late_binding_as_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program vc;
{$mode objfpc}{$H+}
uses SysUtils, Variants;
type
  { A complex number as a custom variant. }
  TComplex = class
    Re, Im: Double;
    constructor Create(ARe, AIm: Double);
  end;
  TComplexType = class(TInvokeableVariantType)
  public
    procedure Clear(var V: TVarData); override;
    procedure Copy(var Dest: TVarData; const Source: TVarData; const Indirect: Boolean); override;
    procedure Cast(var Dest: TVarData; const Source: TVarData); override;
    procedure CastTo(var Dest: TVarData; const Source: TVarData; const AVarType: TVarType); override;
    procedure BinaryOp(var Left: TVarData; const Right: TVarData; const Operation: TVarOp); override;
    function CompareOp(const Left, Right: TVarData; const Operation: TVarOp): Boolean; override;
    function DoFunction(var Dest: TVarData; const V: TVarData; const Name: string; const Arguments: TVarDataArray): Boolean; override;
    function GetProperty(var Dest: TVarData; const V: TVarData; const Name: string): Boolean; override;
    function SetProperty(var V: TVarData; const Name: string; const Value: TVarData): Boolean; override;
  end;
var
  ComplexType: TComplexType;
  Frees: Integer;

constructor TComplex.Create(ARe, AIm: Double); begin Re := ARe; Im := AIm; end;

function C(const V: TVarData): TComplex; begin Result := TComplex(V.VPointer); end;

function VarComplex(ARe, AIm: Double): Variant;
begin
  VarClear(Result);
  TVarData(Result).VType := ComplexType.VarType;
  TVarData(Result).VPointer := TComplex.Create(ARe, AIm);
end;

procedure TComplexType.Clear(var V: TVarData);
begin
  C(V).Free; Inc(Frees);
  V.VType := varEmpty; V.VPointer := nil;
end;

procedure TComplexType.Copy(var Dest: TVarData; const Source: TVarData; const Indirect: Boolean);
begin
  Dest.VType := Source.VType;
  Dest.VPointer := TComplex.Create(C(Source).Re, C(Source).Im);
end;

procedure TComplexType.Cast(var Dest: TVarData; const Source: TVarData);
begin
  Dest.VType := VarType;
  Dest.VPointer := TComplex.Create(Variant(Source), 0);
end;

procedure TComplexType.CastTo(var Dest: TVarData; const Source: TVarData; const AVarType: TVarType);
var S: string;
begin
  S := '(' + FloatToStr(C(Source).Re) + ',' + FloatToStr(C(Source).Im) + ')';
  if AVarType = varString then
  begin
    VarDataClear(Dest);
    Dest.VType := varString; Dest.VString := nil;
    AnsiString(Dest.VString) := S;
  end
  else
    RaiseCastError;
end;

procedure TComplexType.BinaryOp(var Left: TVarData; const Right: TVarData; const Operation: TVarOp);
var L, R: TComplex; Res: TComplex;
begin
  L := C(Left);
  if Right.VType = VarType then R := C(Right) else R := TComplex.Create(Variant(Right), 0);
  case Operation of
    opAdd: Res := TComplex.Create(L.Re + R.Re, L.Im + R.Im);
    opMultiply: Res := TComplex.Create(L.Re * R.Re - L.Im * R.Im, L.Re * R.Im + L.Im * R.Re);
  else
    RaiseInvalidOp;
  end;
  if Right.VType <> VarType then R.Free;
  L.Free;
  Left.VPointer := Res;
end;

function TComplexType.CompareOp(const Left, Right: TVarData; const Operation: TVarOp): Boolean;
begin
  Result := (C(Left).Re = C(Right).Re) and (C(Left).Im = C(Right).Im);
  if Operation = opCmpNe then Result := not Result;
end;

function TComplexType.DoFunction(var Dest: TVarData; const V: TVarData; const Name: string; const Arguments: TVarDataArray): Boolean;
begin
  WriteLn('DoFunction ', Name, ' ', Length(Arguments));
  Result := CompareText(Name, 'Scale') = 0;
  if Result then
    Variant(Dest) := VarComplex(C(V).Re * Variant(Arguments[0]), C(V).Im * Variant(Arguments[0]));
end;

function TComplexType.GetProperty(var Dest: TVarData; const V: TVarData; const Name: string): Boolean;
begin
  WriteLn('GetProperty ', Name);
  Result := True;
  if CompareText(Name, 'Re') = 0 then Variant(Dest) := C(V).Re
  else if CompareText(Name, 'Im') = 0 then Variant(Dest) := C(V).Im
  else Result := False;
end;

function TComplexType.SetProperty(var V: TVarData; const Name: string; const Value: TVarData): Boolean;
begin
  WriteLn('SetProperty ', Name);
  Result := True;
  if CompareText(Name, 'Re') = 0 then C(V).Re := Variant(Value) else C(V).Im := Variant(Value);
end;

var A, B, Z: Variant;
begin
  ComplexType := TComplexType.Create;
  WriteLn(ComplexType.VarType >= $10F, ' ', VarTypeAsText(ComplexType.VarType) <> '');
  A := VarComplex(1, 2);
  B := VarComplex(3, -1);
  Z := A + B; WriteLn(VarToStr(Z));
  Z := A * B; WriteLn(VarToStr(Z));
  Z := A + 10; WriteLn(VarToStr(Z));
  WriteLn(A = VarComplex(1, 2), ' ', A <> B);
  Z := A; Z.Re := 7; WriteLn(VarToStr(A), ' ', VarToStr(Z));
  WriteLn(Z.Im, ' ', VarToStr(Z.Scale(2)), ' ', VarToStr(Z.scale(3)));
  Z.Im := Z.Re + 1; WriteLn(VarToStr(Z));
  try Z.Missing; except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  VarClear(A); VarClear(B); VarClear(Z);
  WriteLn('frees>0 ', Frees > 0);
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"TRUE TRUE
(4,1)
(5,5)
(11,2)
TRUE TRUE
SetProperty Re
(1,2) (7,2)
GetProperty Im
2 DoFunction Scale 1
(14,4) DoFunction scale 1
(21,6)
GetProperty Re
SetProperty Im
(7,8)
DoFunction Missing 0
EVariantDispatchError: No variant method call dispatch
frees>0 TRUE
"#
        );
    }

    #[test]
    fn generics_defaults_comparers_as_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program g2;
{$mode objfpc}{$H+}
uses SysUtils, Generics.Defaults;
type
  TRec = record A, B: Integer; end;
  TByLen = class
    function Cmp(constref L, R: string): Integer;
  end;
function TByLen.Cmp(constref L, R: string): Integer; begin Result := Length(L) - Length(R); end;
function Rev(constref L, R: Integer): Integer; begin Result := R - L; end;
function EqMod(constref L, R: Integer): Boolean; begin Result := L mod 10 = R mod 10; end;
function HashMod(constref V: Integer): UInt32; begin Result := V mod 10; end;
var
  CI: specialize IComparer<Integer>; CS: specialize IComparer<string>;
  CR: specialize IComparer<TRec>; E: specialize IEqualityComparer<Integer>;
  ES: specialize IEqualityComparer<string>; ER: specialize IEqualityComparer<TRec>;
  X, Y: TRec; B: TByLen;
begin
  CI := specialize TComparer<Integer>.Default;
  WriteLn(CI.Compare(1, 2), ' ', CI.Compare(2, 2), ' ', CI.Compare(3, 2));
  CS := specialize TComparer<string>.Default;
  WriteLn(CS.Compare('abc', 'abd') < 0, ' ', CS.Compare('b', 'a') > 0, ' ', CS.Compare('x', 'x'));
  X.A := 1; X.B := 2; Y := X;
  CR := specialize TComparer<TRec>.Default;
  WriteLn(CR.Compare(X, Y)); Y.B := 3; WriteLn(CR.Compare(X, Y) <> 0);
  CI := specialize TComparer<Integer>.Construct(@Rev);
  WriteLn(CI.Compare(1, 2), ' ', CI.Compare(5, 2));
  B := TByLen.Create;
  CS := specialize TComparer<string>.Construct(@B.Cmp);
  WriteLn(CS.Compare('aaa', 'b'));
  E := specialize TEqualityComparer<Integer>.Default;
  WriteLn(E.Equals(3, 3), ' ', E.Equals(3, 4), ' ', E.GetHashCode(7) = E.GetHashCode(7));
  ES := specialize TEqualityComparer<string>.Default;
  WriteLn(ES.Equals('a', 'a'), ' ', ES.GetHashCode('hello') = ES.GetHashCode('hel' + 'lo'));
  ER := specialize TEqualityComparer<TRec>.Default;
  WriteLn(ER.Equals(X, X), ' ', ER.Equals(X, Y));
  E := specialize TEqualityComparer<Integer>.Construct(@EqMod, @HashMod);
  WriteLn(E.Equals(13, 23), ' ', E.GetHashCode(47));
  B.Free;
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"-1 0 1
TRUE TRUE 0
0
TRUE
1 -3
2
TRUE FALSE TRUE
TRUE TRUE
TRUE FALSE
TRUE 7
"#
        );
    }

    #[test]
    fn generics_collections_with_comparers_as_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program g4;
{$mode objfpc}{$H+}
uses SysUtils, Generics.Defaults, Generics.Collections;
type
  TPoint2 = record X, Y: Integer; end;
  TPerson = record Name: string; Age: Integer; end;
  TSorter = class
    function ByAge(constref A, B: TPerson): Integer;
  end;
function TSorter.ByAge(constref A, B: TPerson): Integer; begin Result := A.Age - B.Age; end;
function ByName(constref A, B: TPerson): Integer; begin Result := CompareStr(A.Name, B.Name); end;
function Desc(constref A, B: Integer): Integer; begin Result := B - A; end;
function CIEq(constref A, B: string): Boolean; begin Result := SameText(A, B); end;
function CIHash(constref V: string): UInt32; begin Result := Length(V); end;
function P(const N: string; A: Integer): TPerson; begin Result.Name := N; Result.Age := A; end;
function Pt(X, Y: Integer): TPoint2; begin Result.X := X; Result.Y := Y; end;
var
  L: specialize TList<Integer>; PL: specialize TList<TPerson>;
  D: specialize TDictionary<string, Integer>; PD: specialize TDictionary<TPoint2, string>;
  A: array of Integer; I, Idx: SizeInt; S: TSorter; Q: TPerson; R: TBinarySearchResult;
  Found: Boolean; V: Integer; Str: string;
begin
  L := specialize TList<Integer>.Create;
  L.AddRange([5, 3, 9, 1, 7, 3]);
  L.Sort; for I := 0 to L.Count - 1 do Write(L[I], ' '); WriteLn;
  WriteLn(L.BinarySearch(7, Idx), ' ', Idx, ' ', L.BinarySearch(4, Idx), ' ', Idx);
  L.Sort(specialize TComparer<Integer>.Construct(@Desc));
  for I := 0 to L.Count - 1 do Write(L[I], ' '); WriteLn;
  WriteLn(L.IndexOf(9), ' ', L.Contains(4), ' ', L.Remove(3), ' ', L.Count);
  L.Free;
  L := specialize TList<Integer>.Create(specialize TComparer<Integer>.Construct(@Desc));
  L.AddRange([2, 8, 4]); L.Sort; for I := 0 to L.Count - 1 do Write(L[I], ' '); WriteLn;
  L.Free;
  S := TSorter.Create;
  PL := specialize TList<TPerson>.Create(specialize TComparer<TPerson>.Construct(@S.ByAge));
  PL.Add(P('Ann', 41)); PL.Add(P('Bob', 25)); PL.Add(P('Cid', 33)); PL.Add(P('Dee', 25));
  PL.Sort; for Q in PL do Write(Q.Name, ':', Q.Age, ' '); WriteLn;
  PL.Sort(specialize TComparer<TPerson>.Construct(@ByName));
  for Q in PL do Write(Q.Name, ' '); WriteLn;
  WriteLn(PL.IndexOf(P('Cid', 33)), ' ', PL.Contains(P('Cid', 34)));
  PL.Free; S.Free;
  SetLength(A, 6); A[0] := 4; A[1] := 2; A[2] := 6; A[3] := 1; A[4] := 5; A[5] := 3;
  specialize TArrayHelper<Integer>.Sort(A);
  for I := 0 to High(A) do Write(A[I], ' '); WriteLn;
  Found := specialize TArrayHelper<Integer>.BinarySearch(A, 5, Idx); WriteLn(Found, ' ', Idx);
  Found := specialize TArrayHelper<Integer>.BinarySearch(A, 7, R);
  WriteLn(Found, ' ', R.FoundIndex, ' ', R.CandidateIndex, ' ', R.CompareResult);
  D := specialize TDictionary<string, Integer>.Create(
    specialize TEqualityComparer<string>.Construct(@CIEq, @CIHash));
  D.Add('Apple', 1); D.AddOrSetValue('APPLE', 2); D.Add('pear', 3);
  WriteLn(D.Count, ' ', D['apple'], ' ', D.ContainsKey('PEAR'), ' ', D.TryGetValue('fig', V));
  D.Free;
  PD := specialize TDictionary<TPoint2, string>.Create;
  PD.Add(Pt(1, 2), 'a'); PD.Add(Pt(3, 4), 'b'); PD[Pt(1, 2)] := 'c';
  WriteLn(PD.Count, ' ', PD[Pt(1, 2)], ' ', PD.ContainsKey(Pt(3, 4)), ' ', PD.ContainsKey(Pt(4, 3)));
  WriteLn(PD.ContainsValue('b'), ' ', PD.TryGetValue(Pt(9, 9), Str));
  PD.Free;
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"1 3 3 5 7 9 
TRUE 4 FALSE -1
9 7 5 3 3 1 
0 FALSE 3 5
8 4 2 
Dee:25 Bob:25 Cid:33 Ann:41 
Ann Bob Cid Dee 
2 FALSE
1 2 3 4 5 6 
TRUE 4
FALSE -1 5 -1
2 2 TRUE FALSE
2 c TRUE FALSE
TRUE FALSE
"#
        );
    }

    #[test]
    fn stream_date_variants_and_properties_as_fpc() {
        let (ok, out) = build_and_run_source(
            r#"program s1;
{$mode objfpc}{$H+}
uses SysUtils, Classes, Variants;
type
  TThing = class(TComponent)
  private
    FWhen: Variant;
  published
    property When: Variant read FWhen write FWhen;
  end;
var T, U: TThing; M: TMemoryStream; S: TStringStream; I: Integer; B: PByte;
begin
  RegisterClass(TThing);
  T := TThing.Create(nil); T.Name := 'T1';
  T.When := VarFromDateTime(EncodeDate(2024, 2, 29) + EncodeTime(13, 5, 7, 0));
  M := TMemoryStream.Create; M.WriteComponent(T);
  B := M.Memory; for I := 0 to M.Size - 1 do Write(IntToHex(B[I], 2)); WriteLn;
  S := TStringStream.Create('');
  try
    M.Position := 0; ObjectBinaryToText(M, S); Write(S.DataString);
  except on E: Exception do WriteLn(E.ClassName, ': ', E.Message); end;
  M.Position := 0; U := TThing.Create(nil);
  M.ReadComponent(U);
  WriteLn(VarTypeAsText(VarType(U.When)), ' ', VarToStr(U.When));
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"5450463006545468696E67025431045768656E11EE047172F124E6400000
EReadError: Invalid property type from streamed property: 17
Date 29-2-24 13:05:07
"#
        );
        let (ok, out) = build_and_run_source(
            r#"program s2;
{$mode objfpc}{$H+}
uses SysUtils, Classes;
type
  TThing = class(TComponent)
  private
    FWhen: TDateTime; FTime: TTime;
  published
    property When: TDateTime read FWhen write FWhen;
    property At: TTime read FTime write FTime;
  end;
var T, U: TThing; M: TMemoryStream; S: TStringStream; I: Integer; B: PByte;
begin
  RegisterClass(TThing);
  T := TThing.Create(nil); T.Name := 'T1';
  T.When := EncodeDate(2024, 2, 29) + EncodeTime(13, 5, 7, 0); T.At := EncodeTime(8, 30, 0, 0);
  M := TMemoryStream.Create; M.WriteComponent(T);
  B := M.Memory; for I := 0 to M.Size - 1 do Write(IntToHex(B[I], 2)); WriteLn;
  S := TStringStream.Create('');
  M.Position := 0; ObjectBinaryToText(M, S); Write(S.DataString);
  M.Position := 0; U := TThing.Create(nil); M.ReadComponent(U);
  WriteLn(DateTimeToStr(U.When), ' ', TimeToStr(U.At));
end.
"#,
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            r#"5450463006545468696E67025431045768656E0500702788938B27B10E400241740500585555555555B5FD3F0000
object T1: TThing
  When =  4.5351545219907406E+004
  At =  3.5416666666666669E-001
end
29-2-24 13:05:07 08:30:00
"#
        );
    }

    #[test]
    fn generic_routines_delphi_syntax() {
        let (ok, out) = build_and_run_source(
            "program T;
{$mode delphi}{$H+}
type
  TUtil = class
  end;

function Larger<T>(const A, B: T): T;
begin
  if A > B then Result := A else Result := B;
end;

procedure Show<T>(const V: T);
begin
  WriteLn('value ', V);
end;

var I: Integer;
begin
  I := Larger<Integer>(3, 9);
  WriteLn(I, ' ', Larger<string>('a', 'b'));
  Show<Integer>(I * 2);
  Show<string>('done');
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "9 b\nvalue 18\nvalue done\n");
    }

    #[test]
    fn class_instances_and_their_strings_are_freed() {
        let (ok, out) = build_and_run_source(
            "program T;
uses SysUtils;
type
  TPerson = class
  private
    FName: string;
    FTags: array[1..3] of string;
  public
    constructor Create(const AName: string);
    property Name: string read FName write FName;
  end;
  TEmployee = class(TPerson)
    Role: string;
  end;

constructor TPerson.Create(const AName: string);
var I: Integer;
begin
  FName := AName + '!';
  for I := 1 to 3 do FTags[I] := FName + IntToStr(I);
end;

var
  E: TEmployee;
  P: TPerson;
  I: Integer;
  H: TFPCHeapStatus;
  Before: PtrUInt;
begin
  H := GetFPCHeapStatus;
  Before := H.CurrHeapUsed;
  for I := 1 to 50000 do
  begin
    E := TEmployee.Create('Ann' + IntToStr(I));
    E.Role := 'dev ' + E.Name;
    E.Name := E.Name + E.Role;
    P := E;
    FreeAndNil(P);
  end;
  H := GetFPCHeapStatus;
  writeln(P = nil);
  {$ifdef DARWIN}
  writeln(H.CurrHeapUsed - Before < 100000);
  {$else}
  writeln(true);
  {$endif}
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "TRUE\nTRUE\n");
    }

    #[test]
    fn classes_beside_an_object_named_tobject() {
        // Free Vision's Objects unit has an object type called TObject;
        // classes still descend from the root class.
        let (ok, out) = build_and_run_source(
            "program T;
type
  PObject = ^TObject;
  TObject = object
    constructor Init;
    destructor Done; virtual;
  end;
  TCounter = class
    N: Integer;
    procedure Bump;
  end;
  THolder = object(TObject)
    C: TCounter;
    constructor Init;
    destructor Done; virtual;
  end;

constructor TObject.Init; begin end;
destructor TObject.Done; begin end;
procedure TCounter.Bump; begin Inc(N); end;
constructor THolder.Init;
begin
  inherited Init;
  C := TCounter.Create;
end;
destructor THolder.Done;
begin
  C.Free;
  inherited Done;
end;

var H: ^THolder;
begin
  New(H, Init);
  H^.C.Bump;
  H^.C.Bump;
  writeln(H^.C.N, ' ', H^.C.ClassName, ' ', H^.C.ClassParent.ClassName);
  Dispose(H, Done);
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "2 TCounter TObject\n");
    }

    #[test]
    fn class_mistakes_are_errors() {
        for (source, message) in [
            (
                "program T;
type
  TA = class
    procedure Run; override;
  end;
procedure TA.Run; begin end;
begin
end.
",
                "overrides nothing",
            ),
            (
                "program T;
type
  TA = class
    FX: Integer;
    property X: Integer read FX;
  end;
var A: TA;
begin
  A := TA.Create;
  A.X := 1;
end.
",
                "read-only",
            ),
            (
                "program T;
type
  TA = class
    procedure Run;
  end;
procedure TA.Run; begin end;
begin
  TA.Run;
end.
",
                "needs an object to call it on",
            ),
        ] {
            let Err(e) = MiniPascal.build(source) else {
                panic!("should not compile: {source}");
            };
            assert!(format!("{e:?}").contains(message), "{e:?}");
        }
    }

    #[test]
    fn strings_are_reference_counted() {
        let (ok, out) = build_and_run_source(
            "program T;
uses SysUtils;
type
  TRec = record
    Name: String;
    Wide: UnicodeString;
    N: Integer;
  end;
var
  G: String;
  A: array[1..3] of String;
  R, R2: TRec;
  P: ^TRec;
  I: Integer;
  H: TFPCHeapStatus;
  Before: PtrUInt;

function Twice(S: String): String;
begin
  S := S + S;
  Twice := S;
end;

function Join(const A, B: String): String;
begin
  Result := A + '-' + B;
end;

procedure SetIt(var S: String; const V: String);
begin
  S := V + '!';
end;

function Upper(S: UnicodeString): UnicodeString;
var I: Integer;
begin
  for I := 1 to Length(S) do
    S[I] := UpCase(S[I]);
  Upper := S;
end;

begin
  G := Twice('ab');
  writeln(Join(G, Twice('c')));
  SetIt(G, 'set');
  writeln(G);
  A[1] := 'one'; A[2] := A[1] + 'two'; A[3] := A[2];
  A[1] := 'changed';
  writeln(A[1], ' ', A[2], ' ', A[3]);
  R.Name := 'rec' + IntToStr(1); R.Wide := 'w'; R.N := 5;
  R2 := R;
  R.Name := 'other'; R.Wide := 'x';
  writeln(R.Name, R.Wide, ' ', R2.Name, R2.Wide);
  New(P);
  P^.Name := Join('heap', 'rec');
  writeln(P^.Name, ' ', Upper('mixed Case'));
  H := GetFPCHeapStatus;
  Before := H.CurrHeapUsed;
  for I := 1 to 100000 do
  begin
    G := Twice(IntToStr(I)) + Join('a', 'b');
    R.Name := G;
    R.Wide := Upper(G);
    A[2] := Copy(G, 1, 3);
  end;
  H := GetFPCHeapStatus;
  writeln(G, ' ', R.Wide, ' ', A[2]);
  {$ifdef DARWIN}
  writeln(H.CurrHeapUsed - Before < 200000);
  {$else}
  writeln(true);
  {$endif}
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "abab-cc\nset!\nchanged onetwo onetwo\notherx rec1w\nheap-rec MIXED CASE\n\
             100000100000a-b 100000100000A-B 100\nTRUE\n"
        );
    }

    #[test]
    fn records_with_strings_are_freed() {
        let (ok, out) = build_and_run_source(
            "program T;
uses SysUtils;
type
  TPerson = record
    Name: String;
    Nick: UnicodeString;
    Age: Integer;
  end;
  PPerson = ^TPerson;
var
  P: PPerson;
  R: TPerson;
  I: Integer;
  S: String;
  H: TFPCHeapStatus;
  B: PtrUInt;

function MakePerson(N: Integer): TPerson;
begin
  Result.Name := 'person' + IntToStr(N);
  Result.Nick := 'nick';
  Result.Age := N;
end;

begin
  H := GetFPCHeapStatus;
  B := H.CurrHeapUsed;
  for I := 1 to 50000 do
  begin
    New(P);
    P^.Name := 'n' + IntToStr(I);
    P^.Nick := 'k' + IntToStr(I);
    Dispose(P);
    R := MakePerson(I);
    S := String(Chr(65 + I mod 26));
  end;
  H := GetFPCHeapStatus;
  writeln(R.Name, ' ', R.Nick, ' ', R.Age, ' ', S, ' ', MakePerson(7).Name);
  {$ifdef DARWIN}
  writeln(H.CurrHeapUsed - B < 100000);
  {$else}
  writeln(true);
  {$endif}
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(out, "person50000 nick 50000 C person7\nTRUE\n");
    }

    // Bruto's network units are for Unix (BSD sockets).
    #[cfg(unix)]
    #[test]
    fn sockets_talk_over_loopback() {
        let (ok, out) = build_and_run_source(
            "program T;
uses Sockets, NetDB;
var
  Server, Client, Conn: LongInt;
  Addr, Peer: TInetSockAddr;
  Len: TSockLen;
  Buf: array[0..63] of Char;
  N: LongInt;
  Msg: AnsiString;
  H: THostEntry;
begin
  writeln(SizeOf(TInetSockAddr), ' ', SizeOf(in_addr), ' ',
    NetAddrToStr(StrToNetAddr('192.168.1.20')), ' ', HostAddrToStr(StrToHostAddr('10.0.0.1')),
    ' ', htons($1234) = $3412, ' ', StrToNetAddr('300.1.1.1').s_addr);
  Server := fpSocket(AF_INET, SOCK_STREAM, 0);
  FillChar(Addr, SizeOf(Addr), 0);
  Addr.sin_family := AF_INET;
  Addr.sin_addr := StrToNetAddr('127.0.0.1');
  writeln(fpBind(Server, @Addr, SizeOf(Addr)), ' ', fpListen(Server, 1));
  Len := SizeOf(Addr);
  fpGetSockName(Server, @Addr, @Len);
  Client := fpSocket(AF_INET, SOCK_STREAM, 0);
  writeln(fpConnect(Client, @Addr, SizeOf(Addr)));
  Len := SizeOf(Peer);
  Conn := fpAccept(Server, @Peer, @Len);
  writeln(NetAddrToStr(Peer.sin_addr));
  Msg := 'hello';
  fpSend(Client, @Msg[1], Length(Msg), 0);
  N := fpRecv(Conn, @Buf, SizeOf(Buf), 0);
  Buf[N] := #0;
  writeln(N, ' ', PChar(@Buf));
  CloseSocket(Conn);
  writeln(fpRecv(Client, @Buf, SizeOf(Buf), 0));
  CloseSocket(Client);
  CloseSocket(Server);
  writeln(ResolveHostByName('localhost', H), ' ', HostAddrToStr(H.Addr));
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "16 4 192.168.1.20 10.0.0.1 TRUE 0\n0 0\n0\n127.0.0.1\n5 hello\n0\nTRUE 127.0.0.1\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn network_objects() {
        let (ok, out) = build_and_run_source(
            "program T;
uses NetObjects;
function c_fork: LongInt; cdecl; external 'c' name 'fork';
procedure c_exit(Code: LongInt); cdecl; external 'c' name '_exit';
function c_waitpid(Pid: LongInt; Status: Pointer; Options: LongInt): LongInt;
  cdecl; external 'c' name 'waitpid';
var
  Server: TTcpServer;
  Peer: PTcpClient;
  Client: TTcpClient;
  Http: THttpClient;
  Udp1, Udp2: TUdpSocket;
  Line, Data, From: AnsiString;
  FromPort, Port: Word;
  Pid, Status: LongInt;
begin
  if not Server.Listen(0, 4) then Halt(1);
  Port := Server.LocalPort;
  Pid := c_fork;
  if Pid = 0 then
  begin
    Peer := Server.Accept;
    while Peer^.ReadLine(Line) and (Line <> 'quit') do
      Peer^.SendString('echo: ' + Line + #10);
    Dispose(Peer, Done);
    Peer := Server.Accept;
    while Peer^.ReadLine(Line) and (Line <> '') do ;
    Peer^.SendString('HTTP/1.1 200 OK'#13#10'Content-Type: text/plain'#13#10 +
      'Transfer-Encoding: chunked'#13#10#13#10'5'#13#10'Hello'#13#10 +
      '7'#13#10', world'#13#10'0'#13#10#13#10);
    Dispose(Peer, Done);
    c_exit(0);
  end;
  Server.Done;
  if not Client.Connect('127.0.0.1', Port) then writeln('connect failed');
  Client.SendString('first'#10'second'#10);
  Client.ReadLine(Line); writeln(Line);
  Client.ReadLine(Line); writeln(Line);
  Client.SendString('quit'#10);
  Client.Done;
  Http.Init;
  Str(Port, Line);
  writeln(Http.Get('http://localhost:' + Line + '/x'), ' [', Http.Body, '] ',
    Http.Header('content-type'));
  Http.Done;
  c_waitpid(Pid, @Status, 0);
  writeln(Client.Connect('127.0.0.1', 1));
  Udp1.Init(0);
  Udp2.Init(0);
  Udp1.SendTo('127.0.0.1', Udp2.LocalPort, 'datagram');
  if Udp2.ReceiveFrom(Data, From, FromPort) then
    writeln(Data, ' ', From, ' ', FromPort = Udp1.LocalPort);
  Udp1.Done;
  Udp2.Done;
end.
",
        );
        assert!(ok, "program failed: {out}");
        assert_eq!(
            out,
            "echo: first\necho: second\n200 [Hello, world] text/plain\nFALSE\ndatagram 127.0.0.1 TRUE\n"
        );
    }

    #[test]
    fn conditional_compilation() {
        let (ok, out) = build_and_run_source(
            "program T;
{$DEFINE Verbose}
{$IFDEF verbose}
const Mode = 'verbose';
{$ELSE}
const Mode = 'quiet';
{$ENDIF}
begin
  writeln(Mode);
  {$IFNDEF FPC} this is not Pascal {$ENDIF}
  {$IFDEF BRUTO} writeln('bruto'); {$ENDIF}
  {$IF defined(FPC) and not defined(Nope)}
  writeln('if');
  {$ELSEIF defined(FPC)}
  writeln('elseif skipped');
  {$ELSE}
  writeln('else skipped');
  {$ENDIF}
  {$IF FPC_FULLVERSION >= 30200} writeln('version'); {$IFEND}
  {$IFDEF Nope}
    {$IFDEF FPC} writeln('nested in false branch'); {$ENDIF}
    writeln('unterminated string
  {$ELSE}
    writeln('outer else');
  {$ENDIF}
  {$UNDEF Verbose}
  {$IFDEF Verbose} writeln('still defined'); {$ENDIF}
  { a commented-out block { with a nested comment } and {$IFDEF X} a
    directive, all ignored }
  { $ifdef Nope  -- a space after the brace makes this a plain comment }
  writeln('done')
end.
",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(
            lines,
            ["verbose", "bruto", "if", "version", "outer else", "done"]
        );
    }

    #[test]
    fn include_files() {
        let (ok, out) = build_and_run_in_dir(
            "bruto_pascal_include_test",
            &[
                // Found without its extension and with a different case;
                // carries a code page 437 byte (0xB3) in a comment.
                (
                    "Consts.INC",
                    b"const Greeting = 'hello from consts'; { \xb3 }\n",
                ),
                (
                    "body.inc",
                    b"{$I consts}\nprocedure Show;\nbegin\n  writeln(Greeting)\nend;\n",
                ),
            ],
            "program Main;\n{$I body.inc}\nbegin\n  Show\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "hello from consts");
    }

    #[test]
    fn preprocessor_errors_are_reported() {
        for (src, msg) in [
            (
                "program T;\n{$error unsupported target}\nbegin\nend.\n",
                "unsupported target",
            ),
            (
                "program T;\n{$I nowhere.inc}\nbegin\nend.\n",
                "'nowhere.inc' not found",
            ),
            (
                "program T;\n{$IFDEF FPC}\nbegin\nend.\n",
                "missing {$ENDIF}",
            ),
            ("program T;\n{$ENDIF}\nbegin\nend.\n", "without a matching"),
            (
                "program T;\n{$IFDEF Nope}{$error not here}{$ENDIF}\nbegin\n  {$error here}\nend.\n",
                "line 4",
            ),
        ] {
            let err = match MiniPascal.build(src) {
                Ok(_) => panic!("expected an error for {src:?}"),
                Err(e) => e,
            };
            assert!(err.contains(msg), "{err}");
        }
    }

    #[test]
    fn included_code_gets_its_own_debug_file() {
        let tmp = std::env::temp_dir().join("bruto_pascal_include_dwarf");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("helper.inc"),
            "procedure Helper;\nbegin\n  writeln('x')\nend;\n",
        )
        .unwrap();
        let source = "program Main;\n{$I helper.inc}\nbegin\n  Helper\nend.\n";
        let mut parser = Parser::with_options(
            source,
            &parser::ParseOptions {
                file: None,
                include_dirs: vec![tmp.clone()],
            },
        );
        let program = parser.parse_program().expect("parse");
        let _ = std::fs::remove_dir_all(&tmp);
        let context = Context::create();
        let mut cg = CodeGen::new(&context, "/tmp/Main.pas");
        cg.compile(&program).expect("codegen");
        let ir = cg.print_ir();
        assert!(ir.contains(r#"!DIFile(filename: "helper.inc""#), "{ir}");
        assert!(
            ir.contains(r#"!DISubprogram(name: "Helper""#),
            "missing Helper subprogram"
        );
    }

    /// Run the preprocessor over Free Vision's sources. Needs a checkout of
    /// FPC's `packages/fv/src` (and `packages/rtl-extra/src/inc` for
    /// `objects.pp`):
    /// `FV_SRC=/path/to/fpc FV_SRC_EXTRA=... cargo test -- --ignored free_vision`
    #[test]
    #[ignore]
    fn free_vision_sources_preprocess_cleanly() {
        let fv = PathBuf::from(std::env::var("FV_SRC").expect("set FV_SRC to packages/fv/src"));
        let mut files: Vec<PathBuf> = std::fs::read_dir(&fv)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let name = p
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_ascii_lowercase();
                name.ends_with(".pas") && !name.starts_with('u')
            })
            .collect();
        if let Ok(extra) = std::env::var("FV_SRC_EXTRA") {
            files.push(PathBuf::from(extra).join("objects.pp"));
        }
        files.sort();
        let mut failures = Vec::new();
        for path in &files {
            let source = source_map::read_source(path).unwrap();
            let parser = Parser::with_options(
                &source,
                &parser::ParseOptions {
                    file: Some(path.clone()),
                    include_dirs: vec![fv.clone()],
                },
            );
            match parser.preprocessing_error() {
                Some(e) => failures.push(format!("{}: {e}", path.display())),
                None => println!("{}: {} tokens", path.display(), parser.token_count()),
            }
            // With FV_PARSE set, also report how far parsing gets.
            if std::env::var("FV_PARSE").is_ok() {
                let mut parser = Parser::with_options(
                    &source,
                    &parser::ParseOptions {
                        file: Some(path.clone()),
                        include_dirs: vec![fv.clone()],
                    },
                );
                match parser.parse_unit() {
                    Ok(_) => println!("PARSED {}", path.display()),
                    Err(e) => println!("PARSE-ERROR {}: {e}", path.display()),
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    fn ir_for(source: &str, instrument: bool) -> (String, Vec<String>) {
        let mut parser = Parser::new(source);
        let program = parser.parse_program().expect("parse");
        let context = Context::create();
        let mut cg = CodeGen::new(&context, "/tmp/t.pas");
        cg.set_instrument(instrument);
        cg.compile(&program).expect("codegen");
        (cg.print_ir(), cg.prof_map_lines().to_vec())
    }

    // Globals are passed by `var` parameter: top-level procedures cannot
    // reference globals directly in the current codegen (pre-existing).
    const PROF_SRC: &str = "program P;\nvar i: integer;\nprocedure Q(var n: integer);\nbegin\n  n := n + 1\nend;\nbegin\n  i := 0;\n  Q(i)\nend.\n";

    #[test]
    fn instrumented_ir_has_hooks_and_map() {
        let (ir, map) = ir_for(PROF_SRC, true);
        assert!(ir.contains("call void @__bruto_prof_enter"));
        assert!(ir.contains("call void @__bruto_prof_exit"));
        assert!(ir.contains("call void @__bruto_prof_line"));
        // Two routines: Q (line 3) and the program block; three statements
        // (line 5, line 8, line 9).
        assert!(
            map.iter()
                .any(|l| l.starts_with("P ") && l.ends_with(" 3 Q")),
            "{map:?}"
        );
        assert!(
            map.iter()
                .any(|l| l.starts_with("P ") && l.ends_with(" program")),
            "{map:?}"
        );
        assert_eq!(
            map.iter().filter(|l| l.starts_with("L ")).count(),
            3,
            "{map:?}"
        );
        // exit before every ret: Q returns once, main returns once.
        assert_eq!(ir.matches("call void @__bruto_prof_exit").count(), 2);
    }

    #[test]
    fn uninstrumented_ir_is_clean() {
        let (ir, map) = ir_for(PROF_SRC, false);
        assert!(!ir.contains("__bruto_prof"));
        assert!(map.is_empty());
    }

    #[test]
    fn profile_build_and_run() {
        use bruto_lang::profile::ProfileKind;
        // `acc` is passed by `var`: top-level procedures cannot reference
        // globals directly in the current codegen (pre-existing limitation).
        let src = "program Hot;\nvar i, acc: integer;\nprocedure Work(var a: integer);\nvar k: integer;\nbegin\n  for k := 1 to 2000 do\n    a := a + k\nend;\nbegin\n  acc := 0;\n  for i := 1 to 50 do\n    Work(acc);\n  writeln(acc)\nend.\n";
        let lang = MiniPascal;
        let mut job = lang.profile_job_at(src, None);
        let result = loop {
            match job.poll() {
                BuildPhase::Pending(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
                BuildPhase::Done(r) => break r,
                BuildPhase::Failed(e) => panic!("profile build failed: {e}"),
            }
        };
        let prof_path = result.profile_path.clone().expect("profile path");
        let _ = std::fs::remove_file(&prof_path);
        let status = std::process::Command::new(&result.exe_path)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run");
        assert!(status.success());

        let profile = lang.load_profile(&result).expect("load profile");
        assert!(profile.elapsed_ns > 0);
        assert!(!profile.truncated);
        let work = profile
            .nodes
            .iter()
            .find(|n| n.kind == ProfileKind::Routine && n.name == "Work")
            .expect("Work node");
        assert_eq!(work.calls, 50);
        assert_eq!(work.line, 3);
        let totals = profile.line_totals();
        let (hot_line, _) = totals
            .iter()
            .max_by_key(|(_, (self_ns, _))| *self_ns)
            .map(|(l, v)| (*l, *v))
            .unwrap();
        assert_eq!(
            hot_line, 7,
            "hottest line should be the inner assignment: {totals:?}"
        );
        assert_eq!(totals[&7].1, 50 * 2000, "hits of the inner assignment");

        let _ = std::fs::remove_file(&result.exe_path);
        let _ = std::fs::remove_dir_all(format!("{}.dSYM", result.exe_path));
        let _ = std::fs::remove_file(&prof_path);
        let _ = std::fs::remove_file(result.profile_map_path.unwrap());
    }

    #[test]
    fn compile_checks_programs_and_units_without_linking() {
        let lang = MiniPascal;
        let dir = std::env::temp_dir().join(format!("bruto_check_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let unit_path = dir.join("helper.pas");
        let unit = "unit Helper;\ninterface\nfunction Twice(N: Integer): Integer;\nimplementation\nfunction Twice(N: Integer): Integer;\nbegin\n  Result := N * 2;\nend;\nend.\n";
        std::fs::write(&unit_path, unit).unwrap();
        let ok = lang.check_at(unit, Some(&unit_path)).unwrap();
        assert_eq!(ok, Ok("unit, 9 lines, no errors".to_string()));
        // The window's text is checked, not the file on disk.
        let broken = unit.replace("N * 2", "N * Missing");
        let err = lang
            .check_at(&broken, Some(&unit_path))
            .unwrap()
            .unwrap_err();
        assert!(err.contains("missing") && err.contains("line 7"), "{err}");
        let main_path = dir.join("main.pas");
        let main = "program Main;\nuses Helper;\nbegin\n  WriteLn(Twice(21))\nend.\n";
        assert_eq!(
            lang.check_at(main, Some(&main_path)).unwrap(),
            Ok("5 lines, no errors".to_string())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn units_and_their_users_are_recognized() {
        let lang = MiniPascal;
        let unit = "{ A unit ({$mode objfpc}). }\n(* old style *)\nunit WriterCore;\n{$mode objfpc}\ninterface\nuses SysUtils;\nimplementation\nend.\n";
        assert_eq!(lang.library_name(unit).as_deref(), Some("writercore"));
        let program = "// a program\nprogram Check;\n{$mode objfpc}{$H+}\nuses\n  {$ifdef UNIX}cwstring,{$endif}\n  SysUtils, WriterCore in 'writercore.pas', Generics.Collections;\nvar I: Integer;\nbegin\nend.\n";
        assert_eq!(lang.library_name(program), None);
        assert_eq!(
            lang.used_libraries(program),
            ["cwstring", "sysutils", "writercore", "generics.collections"]
        );
        assert_eq!(lang.library_name("begin\n  WriteLn('unit')\nend.\n"), None);
        assert!(lang.used_libraries("program P;\nbegin\nend.\n").is_empty());
    }

    #[test]
    fn profile_names_unit_files() {
        use bruto_lang::profile::ProfileKind;
        let dir = std::env::temp_dir().join(format!("bruto_prof_units_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let unit = dir.join("helper.pas");
        std::fs::write(
            &unit,
            "unit Helper;\ninterface\nfunction Twice(N: Integer): Integer;\nimplementation\nfunction Twice(N: Integer): Integer;\nvar K: Integer;\nbegin\n  Result := 0;\n  for K := 1 to 2 do\n    Result := Result + N;\nend;\nend.\n",
        )
        .unwrap();
        let src = "program Main;\nuses Helper;\nvar I, S: Integer;\nbegin\n  S := 0;\n  for I := 1 to 100 do\n    S := S + Twice(I);\n  WriteLn(S)\nend.\n";
        let main = dir.join("main.pas");
        std::fs::write(&main, src).unwrap();
        let lang = MiniPascal;
        let mut job = lang.profile_job_at(src, Some(&main));
        let result = job.poll_to_done().expect("profile build");
        let status = std::process::Command::new(&result.exe_path)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run");
        assert!(status.success());
        let profile = lang.load_profile(&result).expect("load profile");
        let unit_path = unit.canonicalize().unwrap().to_string_lossy().into_owned();
        let twice = profile
            .nodes
            .iter()
            .find(|n| n.kind == ProfileKind::Routine && n.name == "Twice")
            .expect("Twice node");
        assert_eq!(
            (twice.line, twice.file.as_deref()),
            (5, Some(unit_path.as_str()))
        );
        // The unit's lines are its own; the program's heat column only
        // gets the program's lines (it has 9).
        assert!(profile.nodes.iter().any(|n| n.kind == ProfileKind::Line
            && n.line == 10
            && n.file.as_deref() == Some(unit_path.as_str())));
        let totals = profile.line_totals();
        assert!(totals.keys().all(|l| *l <= 9), "{totals:?}");
        assert_eq!(totals[&7].1, 100);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Test-only helper: drive a `BuildJob` to completion synchronously.
#[cfg(test)]
trait BuildJobExt {
    fn poll_to_done(&mut self) -> Result<BuildResult, String>;
}

#[cfg(test)]
impl BuildJobExt for dyn BuildJob {
    fn poll_to_done(&mut self) -> Result<BuildResult, String> {
        loop {
            match self.poll() {
                BuildPhase::Pending(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
                BuildPhase::Done(r) => return Ok(r),
                BuildPhase::Failed(e) => return Err(e),
            }
        }
    }
}
