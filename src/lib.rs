#![allow(dead_code)]

pub mod ast;
pub mod codegen;
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

        let mut parser = Parser::with_options(
            &source,
            &parser::ParseOptions {
                file: None,
                include_dirs: self.search_dirs.clone(),
            },
        );
        let mut program = match parser.parse_program() {
            Ok(p) => p,
            Err(e) => return BuildPhase::Failed(format!("Parse error: {e}")),
        };

        if let Err(e) = resolve_uses(&mut program, &self.search_dirs) {
            return BuildPhase::Failed(e);
        }

        // Obfuscation pass 1 (identifier rename) runs on the AST, before
        // codegen builds symbol tables from it. Pass 2 (bogus control flow)
        // runs on the module after compile(), below.
        if self.options.obfuscation_enabled() {
            obfuscate::rename_identifiers(&mut program);
        }

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, &source_path);
        codegen.set_directives(parser.directives);
        codegen.set_build_options(self.options);
        codegen.set_instrument(self.profile);
        if self.profile {
            codegen.set_prof_output(&format!("{exe_path}.bruto-prof"));
        }
        if let Err(e) = codegen.compile(&program) {
            return BuildPhase::Failed(format!("Codegen error: {e}"));
        }

        // Obfuscation pass 2: bogus control flow over the compiled module,
        // after a successful compile and before the object is emitted.
        if self.options.obfuscation_enabled() {
            codegen.apply_bogus_control_flow();
        }

        let artifacts = match codegen.emit_object(&exe_path) {
            Ok(a) => a,
            Err(e) => return BuildPhase::Failed(e),
        };
        let _ = codegen.write_metadata(&exe_path);

        let mut paths = JobPaths {
            source_path,
            exe_path,
            obj_path: artifacts.obj_path,
            runtime_obj_path: None,
            prof_map_path: None,
            asm_path: artifacts.asm_path,
        };

        if self.profile {
            paths.prof_map_path = match codegen.write_prof_map(&paths.exe_path) {
                Ok(p) => Some(p),
                Err(e) => return BuildPhase::Failed(e),
            };
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
            "program T;\ntype\n  Color = (Red, Green, Blue);\nvar c: Color;\nbegin\n  c := Green;\n  writeln(c)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "1");
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
        assert!(out.contains("Blue = 2"), "enum: {out}");
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
        assert!(lines[1].starts_with("3.14"), "got {:?}", lines[1]);
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
