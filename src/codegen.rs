/// LLVM code generation for Mini-Pascal with DWARF debug info.
///
/// Uses inkwell to generate LLVM IR, emit object files, and link with the system linker.
/// Every statement is annotated with source locations for breakpoint support.
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::OptimizationLevel;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::debug_info::{
    AsDIScope, DICompileUnit, DIFile, DIFlags, DIFlagsConstants, DIScope, DISubprogram, DIType,
    DWARFEmissionKind, DWARFSourceLanguage, DebugInfoBuilder,
};
use inkwell::module::{Linkage, Module};
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::BasicType;
use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, PointerValue};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

mod closures;
mod objects;
mod strings;
mod system;

const UNTYPED_DEREF: &str =
    "an untyped pointer can't be dereferenced; cast it to a typed pointer first";

#[derive(Debug)]
pub struct CodeGenError {
    pub message: String,
    pub span: Option<Span>,
}

impl fmt::Display for CodeGenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(span) = self.span {
            write!(f, "{}: {}", span.describe(), self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl CodeGenError {
    fn new(message: impl Into<String>, span: Option<Span>) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

/// Result of [`CodeGen::emit_object`]: the object file ready for linking,
/// plus a text assembly listing for the IDE's Disassembly window.
pub struct ObjectArtifacts {
    /// Path to the `.o` file. Caller links it, then removes it.
    pub obj_path: String,
    /// Path to the `.s` file, or `None` if assembly emission failed for
    /// this target. Debug builds keep `.loc` directives mapping each
    /// block back to a Pascal source line; Retail builds strip debug
    /// info first, so their listing has no source mapping.
    pub asm_path: Option<String>,
}

pub struct CodeGen<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,

    // Debug info
    di_builder: DebugInfoBuilder<'ctx>,
    compile_unit: DICompileUnit<'ctx>,
    di_file: DIFile<'ctx>,

    // Symbol table (scope stack — globals are always at index 0)
    variables: HashMap<String, PointerValue<'ctx>>,
    var_types: HashMap<String, PascalType>,

    // Program-level variables and constants. They live in LLVM globals so
    // every procedure can reach them; lookups fall back here after the
    // current routine's locals (see `lookup_var`).
    global_vars: HashMap<String, PointerValue<'ctx>>,
    global_types: HashMap<String, PascalType>,

    // Debug-info files other than the main one (units, `{$I}` includes),
    // by source-map id, and lexical-block scopes used when a routine's
    // code comes from a different file than the routine itself.
    di_files: std::cell::RefCell<HashMap<u32, DIFile<'ctx>>>,
    file_scopes: std::cell::RefCell<HashMap<(String, u32), DIScope<'ctx>>>,
    // The file the current routine's DISubprogram belongs to.
    scope_file: u32,

    // Identifiers are lower-cased by the semantic pass; this maps each back
    // to the spelling the user wrote, for debug info and watch metadata.
    display_names: HashMap<String, String>,

    // Saved global scope (restored after compiling a procedure)
    saved_scopes: Vec<(
        HashMap<String, PointerValue<'ctx>>,
        HashMap<String, PascalType>,
    )>,

    // Type aliases (from `type` section)
    type_defs: HashMap<String, PascalType>,

    // Enum value → ordinal mapping
    enum_values: HashMap<String, i64>,
    // Enum type name → value names
    enum_type_values: HashMap<String, Vec<String>>,

    // Procedure/function metadata
    proc_return_types: HashMap<String, Option<PascalType>>,
    proc_param_modes: HashMap<String, Vec<(String, ParamMode, PascalType)>>,

    // For nested procs, the captured variables (name + resolved type) from
    // enclosing scope. Captures are passed as implicit `var` parameters.
    proc_captures: HashMap<String, Vec<(String, PascalType)>>,

    // Current function being compiled
    current_fn: Option<FunctionValue<'ctx>>,
    current_scope: Option<DIScope<'ctx>>,

    // Goto/label support
    label_blocks: HashMap<i64, inkwell::basic_block::BasicBlock<'ctx>>,

    // `continue` and `break` targets of the enclosing loops, innermost last.
    // Reset on entry to each routine: a nested procedure can't break out
    // of its parent's loop.
    loop_targets: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>)>,
    // How many pointers `create_debug_type` is currently inside.
    debug_pointer_depth: std::cell::Cell<u32>,
    // Overloaded routines: base name -> one symbol per signature.
    overloads: HashMap<String, Vec<String>>,
    // External routines: Pascal name -> the C function implementing it.
    external_symbols: HashMap<String, String>,
    // Compiling a `System.X` call: built-ins win over methods and the
    // program's own routines.
    bypass_user: bool,
    // Objects named by enclosing `with` statements, innermost last.
    with_objects: Vec<(String, PointerValue<'ctx>)>,
    // Object types (lowered to records) and the method being compiled.
    objects: HashMap<String, objects::ObjectInfo>,
    current_object: Option<String>,
    method_context: Option<objects::MethodContext<'ctx>>,
    // Each routine's dummy slot for `end`-line breakpoint markers.
    end_bp_slots: HashMap<String, PointerValue<'ctx>>,
    // Where `exit` jumps: the current routine's single return block.
    exit_block: Option<BasicBlock<'ctx>>,
    // Name of the function being compiled (its result variable), for
    // `exit(value)`. `None` in procedures and the main program.
    current_function: Option<String>,

    source_path: String,

    // Compiler directives ({$R+}, {$Q+}, {$I+}).
    directives: crate::parser::Directives,

    // Debug vs Retail + optimization goal; applied at object emit / link.
    build_options: bruto_lang::language::BuildOptions,

    // Per-variable metadata for the watch window. Populated during compile.
    // Format lines: `name|kind|extra` (kind = "enum", "set", "vrec", "real", "char", "bool")
    metadata_lines: Vec<String>,

    // Profiling instrumentation (see prof_runtime.rs). Off by default.
    instrument: bool,
    prof_next_id: u32,
    // `P <id> <line> <name>` / `L <id> <line> <col>` lines for <exe>.bruto-prof-map
    prof_map_lines: Vec<String>,
    // Default output path baked into the program; `BRUTO_PROF_OUT` overrides
    // it at runtime. Set via `set_prof_output` before `compile`.
    prof_output: Option<String>,
}

impl<'ctx> CodeGen<'ctx> {
    pub fn new(context: &'ctx Context, source_path: &str) -> Self {
        let module = context.create_module("pascal_program");
        let builder = context.create_builder();

        let path = Path::new(source_path);
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("untitled.pas");
        let directory = path.parent().and_then(|p| p.to_str()).unwrap_or(".");

        let (di_builder, compile_unit) = module.create_debug_info_builder(
            true,
            DWARFSourceLanguage::Pascal83,
            filename,
            directory,
            "turbo-pascal-ide",
            false,
            "",
            0,
            "",
            DWARFEmissionKind::Full,
            0,
            false,
            false,
            "",
            "",
        );

        let di_file = di_builder.create_file(filename, directory);

        Self {
            context,
            module,
            builder,
            di_builder,
            compile_unit,
            di_file,
            variables: HashMap::new(),
            var_types: HashMap::new(),
            global_vars: HashMap::new(),
            global_types: HashMap::new(),
            display_names: HashMap::new(),
            di_files: std::cell::RefCell::new(HashMap::new()),
            file_scopes: std::cell::RefCell::new(HashMap::new()),
            scope_file: crate::source_map::MAIN_FILE,
            saved_scopes: Vec::new(),
            type_defs: HashMap::new(),
            enum_values: HashMap::new(),
            enum_type_values: HashMap::new(),
            proc_return_types: HashMap::new(),
            proc_param_modes: HashMap::new(),
            proc_captures: HashMap::new(),
            current_fn: None,
            current_scope: None,
            label_blocks: HashMap::new(),
            loop_targets: Vec::new(),
            exit_block: None,
            end_bp_slots: HashMap::new(),
            debug_pointer_depth: std::cell::Cell::new(0),
            objects: HashMap::new(),
            with_objects: Vec::new(),
            bypass_user: false,
            overloads: HashMap::new(),
            external_symbols: HashMap::new(),
            current_object: None,
            method_context: None,
            current_function: None,
            source_path: source_path.to_string(),
            directives: crate::parser::Directives::default(),
            build_options: bruto_lang::language::BuildOptions::default(),
            metadata_lines: Vec::new(),
            instrument: false,
            prof_next_id: 1,
            prof_map_lines: Vec::new(),
            prof_output: None,
        }
    }

    /// Write watch-window metadata to `<exe>.bruto-meta`.
    pub fn write_metadata(&self, exe_path: &str) -> Result<(), String> {
        let path = format!("{exe_path}.bruto-meta");
        let body = self.metadata_lines.join("\n");
        std::fs::write(&path, body).map_err(|e| format!("metadata write: {e}"))
    }

    /// Turn profiler instrumentation on or off. Must be called before
    /// `compile`.
    pub fn set_instrument(&mut self, on: bool) {
        self.instrument = on;
    }

    /// Bake `path` into the program as the default profile output, written
    /// via `__bruto_prof_set_output` right after instrumentation starts.
    /// `BRUTO_PROF_OUT` still overrides it at runtime. Must be called before
    /// `compile`.
    pub fn set_prof_output(&mut self, path: &str) {
        self.prof_output = Some(path.to_string());
    }

    /// The id map accumulated so far (for tests and `write_prof_map`).
    pub fn prof_map_lines(&self) -> &[String] {
        &self.prof_map_lines
    }

    /// Write `<exe>.bruto-prof-map` and return its path.
    pub fn write_prof_map(&self, exe_path: &str) -> Result<String, String> {
        let path = format!("{exe_path}.bruto-prof-map");
        let mut body = self.prof_map_lines.join("\n");
        body.push('\n');
        std::fs::write(&path, body).map_err(|e| format!("prof map write: {e}"))?;
        Ok(path)
    }

    // ── profiler hooks ───────────────────────────────────

    /// Declare the three runtime hooks. Called once from `compile` when
    /// instrumentation is on.
    fn emit_prof_decls(&self) {
        let void_ty = self.context.void_type();
        let i32_ty = self.context.i32_type();
        self.module.add_function(
            "__bruto_prof_enter",
            void_ty.fn_type(&[i32_ty.into()], false),
            None,
        );
        self.module
            .add_function("__bruto_prof_exit", void_ty.fn_type(&[], false), None);
        self.module.add_function(
            "__bruto_prof_line",
            void_ty.fn_type(&[i32_ty.into()], false),
            None,
        );
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        self.module.add_function(
            "__bruto_prof_set_output",
            void_ty.fn_type(&[ptr_ty.into()], false),
            None,
        );
    }

    fn prof_alloc_id(&mut self) -> u32 {
        let id = self.prof_next_id;
        self.prof_next_id += 1;
        id
    }

    /// `__bruto_prof_enter(id)` at the current insertion point, registering
    /// `name` at `line` as a routine.
    fn emit_prof_enter(&mut self, name: &str, line: u32) -> Result<(), CodeGenError> {
        if !self.instrument {
            return Ok(());
        }
        let id = self.prof_alloc_id();
        let name = self.display(name);
        self.prof_map_lines.push(format!("P {id} {line} {name}"));
        let f = self.module.get_function("__bruto_prof_enter").unwrap();
        self.builder
            .build_call(
                f,
                &[self.context.i32_type().const_int(id as u64, false).into()],
                "",
            )
            .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        Ok(())
    }

    /// `__bruto_prof_exit()` at the current insertion point (call right
    /// before every `ret`).
    fn emit_prof_exit(&mut self) -> Result<(), CodeGenError> {
        if !self.instrument {
            return Ok(());
        }
        let f = self.module.get_function("__bruto_prof_exit").unwrap();
        self.builder
            .build_call(f, &[], "")
            .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        Ok(())
    }

    /// `__bruto_prof_line(id)` for the statement at `span`.
    fn emit_prof_line(&mut self, span: Span) -> Result<(), CodeGenError> {
        if !self.instrument {
            return Ok(());
        }
        let id = self.prof_alloc_id();
        self.prof_map_lines
            .push(format!("L {id} {} {}", span.line, span.column));
        let f = self.module.get_function("__bruto_prof_line").unwrap();
        self.builder
            .build_call(
                f,
                &[self.context.i32_type().const_int(id as u64, false).into()],
                "",
            )
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    /// Set compiler directives (parsed by the lexer from `{$X+/-}` comments).
    pub fn set_directives(&mut self, d: crate::parser::Directives) {
        self.directives = d;
    }

    /// Set the build profile (Debug/Retail) and optimization goal.
    pub fn set_build_options(&mut self, options: bruto_lang::language::BuildOptions) {
        self.build_options = options;
    }

    /// The build options this codegen emits with.
    pub fn build_options(&self) -> bruto_lang::language::BuildOptions {
        self.build_options
    }

    /// Run the bogus-control-flow obfuscation pass over the compiled module.
    /// Call after [`compile`](Self::compile) and before
    /// [`emit_object`](Self::emit_object). The identifier rename runs
    /// earlier, on the AST (see [`crate::obfuscate::rename_identifiers`]).
    /// The synthetic instructions get a line-0 debug location so a Debug
    /// build's disassembly stays aligned to the real Pascal lines.
    pub fn apply_bogus_control_flow(&self) {
        crate::obfuscate::apply_bogus_control_flow_with_debug(
            &self.module,
            self.context,
            &self.di_builder,
        );
    }

    /// Whether the current module passes LLVM's verifier. Mainly useful
    /// after running a transform (e.g. obfuscation) to assert validity.
    pub fn module_is_valid(&self) -> bool {
        self.module.verify().is_ok()
    }

    /// Compile a Pascal program AST into LLVM IR.
    pub fn compile(&mut self, program: &Program) -> Result<(), CodeGenError> {
        let resolved = crate::sema::resolve_program(program)
            .map_err(|e| CodeGenError::new(e.message, Some(e.span)))?;
        self.display_names = resolved.display_names;
        let program = &resolved.program;
        self.overloads = program.overloads.clone();
        self.emit_runtime_decls();
        if self.instrument {
            self.emit_prof_decls();
        }

        // Create main function
        // `main(argc, argv)`, kept for ParamCount / ParamStr.
        let main_fn_type = self.context.i64_type().fn_type(
            &[
                self.context.i32_type().into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ],
            false,
        );
        let main_fn = self.module.add_function("main", main_fn_type, None);
        self.declare_command_line_globals();

        // Debug info for main
        let di_i64 = self
            .di_builder
            .create_basic_type(
                "integer",
                64,
                0x05, // DW_ATE_signed
                DIFlags::ZERO,
            )
            .unwrap();

        let di_main_type = self.di_builder.create_subroutine_type(
            self.di_file,
            Some(di_i64.as_type()),
            &[],
            DIFlags::ZERO,
        );

        let di_main = self.di_builder.create_function(
            self.compile_unit.as_debug_info_scope(),
            &self.display(&program.name),
            None,
            self.di_file,
            program.span.line,
            di_main_type,
            true,
            true,
            program.body.span.line,
            DIFlags::ZERO,
            false,
        );
        main_fn.set_subprogram(di_main);

        let entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(entry);
        for (i, global) in [system::ARGC_GLOBAL, system::ARGV_GLOBAL]
            .iter()
            .enumerate()
        {
            let slot = self.module.get_global(global).unwrap().as_pointer_value();
            let value = main_fn.get_nth_param(i as u32).unwrap();
            self.builder
                .build_store(slot, value)
                .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        }

        self.current_fn = Some(main_fn);
        self.current_scope = Some(di_main.as_debug_info_scope());

        // Pre-create basic blocks for all declared labels
        for &label in &program.labels {
            let bb = self
                .context
                .append_basic_block(main_fn, &format!("label_{label}"));
            self.label_blocks.insert(label, bb);
        }

        // Register type aliases. Object types are lowered afterwards, in
        // declaration order, so their fields can use any other type and
        // each one's parent is ready before it.
        for td in &program.type_decls {
            if matches!(td.ty, PascalType::Object(_)) {
                continue;
            }
            self.type_defs.insert(td.name.clone(), td.ty.clone());
            if let PascalType::Enum { values, .. } = &td.ty {
                self.enum_type_values
                    .insert(td.name.clone(), values.clone());
            }
        }
        // Enumeration values, wherever their type is written.
        let enum_sources = program
            .type_decls
            .iter()
            .map(|td| &td.ty)
            .chain(program.vars.iter().map(|v| &v.ty));
        for ty in enum_sources {
            self.register_enum_values(ty);
        }

        for td in &program.type_decls {
            if let PascalType::Object(decl) = &td.ty {
                self.register_object(&td.name, decl, td.span)?;
            }
        }
        self.emit_vmts();

        self.declare_system_variables();
        self.register_exit_procs();

        // Program-level constants and variables become LLVM globals before
        // any procedure is compiled, so every routine can reach them.
        for c in &program.consts {
            let ty = self.const_decl_type(c);
            self.declare_global(&c.name, &ty, c.span);
        }
        for var_decl in &program.vars {
            for name in &var_decl.names {
                match &var_decl.absolute {
                    // A global `absolute` variable shares its target's global.
                    Some(target) => {
                        let address = *self.global_vars.get(target).ok_or_else(|| {
                            CodeGenError::new(
                                format!("undefined variable '{target}'"),
                                Some(var_decl.span),
                            )
                        })?;
                        let ty = self.resolve_type(&var_decl.ty);
                        self.global_vars.insert(name.clone(), address);
                        self.global_types.insert(name.clone(), ty);
                    }
                    None => self.declare_global(name, &var_decl.ty, var_decl.span),
                }
            }
        }

        // Declare every routine's prototype first, so any body can call
        // any routine (a unit's implementation calls routines implemented
        // further down; calls to overloads are resolved by signature).
        for proc in &program.procedures {
            if proc.owner.is_none() && (!proc.is_forward || proc.external.is_some()) {
                let mut header = proc.clone();
                header.is_forward = true;
                self.compile_proc_decl(&header)?;
            }
        }

        // Compile procedures/functions first (they are separate LLVM functions)
        for proc in &program.procedures {
            self.compile_proc_decl(proc)?;
        }

        // Position back in main's entry block
        self.builder.position_at_end(entry);
        self.current_fn = Some(main_fn);
        self.current_scope = Some(di_main.as_debug_info_scope());

        // Install signal handler to catch stack overflow / segfaults.
        self.set_debug_loc(program.span);
        self.emit_prof_enter("program", program.body.span.line)?;
        if self.instrument
            && let Some(path) = self.prof_output.clone()
        {
            let ptr = self
                .builder
                .build_global_string_ptr(&path, "prof_output_path")
                .map_err(|e| CodeGenError::new(e.to_string(), None))?;
            let f = self.module.get_function("__bruto_prof_set_output").unwrap();
            self.builder
                .build_call(f, &[ptr.as_pointer_value().into()], "")
                .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        }
        {
            let f = self
                .module
                .get_function("bruto_install_stack_guard")
                .unwrap();
            self.builder
                .build_call(f, &[], "")
                .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        }

        // Open capture file for console output
        self.emit_capture_file_init(program.body.span)?;

        // Predefined `input` and `output` text files (Wirth's spec).
        self.emit_predefined_files(program.span)?;

        // Store each constant's value into its global before the body runs.
        for c in &program.consts {
            self.compile_const_decl(c)?;
        }

        // Compile body (compile_block emits a synthetic alloca on the `end.`
        // line so users can drop a breakpoint there).
        self.begin_exit_block(main_fn);
        self.compile_block(&program.body)?;
        self.enter_exit_block(program.body.end_span)?;

        // Return 0 — debug-loc on `end.` so the ret instruction is associated
        // with that source line. When the user steps past `end.`, lldb
        // returns from main into dyld; debugger.rs detects the no-source
        // stop frame and issues `continue`, so the process still exits
        // cleanly without the user pressing F8 over and over.
        self.set_debug_loc(program.body.end_span);
        self.emit_prof_exit()?;
        self.builder
            .build_return(Some(&self.context.i64_type().const_int(0, false)))
            .map_err(|e| CodeGenError::new(e.to_string(), None))?;

        self.di_builder.finalize();

        // Verify module
        if let Err(msg) = self.module.verify() {
            return Err(CodeGenError::new(
                format!("LLVM module verification failed: {}", msg.to_string()),
                None,
            ));
        }

        Ok(())
    }

    /// Write the LLVM module to a `.o` file and return its path.
    /// Caller is responsible for linking + cleanup.
    ///
    /// Retail builds strip the DWARF metadata and run the LLVM
    /// optimization pipeline selected by the build options first.
    /// Build the `TargetMachine` for this build's options, applying its
    /// side effects to `self.module` first: strip DWARF for Retail builds,
    /// then run the optimization pipeline. Shared by [`emit_object`]
    /// (writes `.o` and `.s` from the *same* prepared module, so the two
    /// never drift apart) — don't call this a second time on the same
    /// `CodeGen`, since stripping/optimizing again would double-apply.
    ///
    /// [`emit_object`]: Self::emit_object
    fn make_target_machine(&self) -> Result<TargetMachine, String> {
        use bruto_lang::language::{BuildProfile, OptimizeFor};

        Target::initialize_native(&InitializationConfig::default()).map_err(|e| e.to_string())?;

        let opt_level = match (self.build_options.profile, self.build_options.optimize) {
            (BuildProfile::Debug, _) => OptimizationLevel::None,
            (BuildProfile::Retail, OptimizeFor::Size) => OptimizationLevel::Less,
            (BuildProfile::Retail, OptimizeFor::Both) => OptimizationLevel::Default,
            (BuildProfile::Retail, OptimizeFor::Speed) => OptimizationLevel::Aggressive,
        };

        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
        let machine = target
            .create_target_machine(
                &triple,
                "generic",
                "",
                opt_level,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or("could not create target machine")?;

        if !self.build_options.debug_info() {
            self.module.strip_debug_info();
        }
        if let Some(pipeline) = self.build_options.pass_pipeline() {
            self.module
                .run_passes(
                    pipeline,
                    &machine,
                    inkwell::passes::PassBuilderOptions::create(),
                )
                .map_err(|e| format!("optimization failed: {e}"))?;
        }

        Ok(machine)
    }

    /// Write the module to an `.o` object file (for linking) and a `.s`
    /// text listing (for the IDE's Disassembly window), both from the
    /// same prepared module so the listing matches the linked binary
    /// exactly. `asm_path` is `None` if the target machine couldn't emit
    /// textual assembly — rare, and the object file is unaffected either
    /// way, so this only disables the Disassembly window for that build.
    pub fn emit_object(&self, output_path: &str) -> Result<ObjectArtifacts, String> {
        let machine = self.make_target_machine()?;

        let obj_path = format!("{output_path}.o");
        machine
            .write_to_file(&self.module, FileType::Object, Path::new(&obj_path))
            .map_err(|e| e.to_string())?;

        let asm_path = format!("{output_path}.s");
        let asm_path = machine
            .write_to_file(&self.module, FileType::Assembly, Path::new(&asm_path))
            .is_ok()
            .then_some(asm_path);

        Ok(ObjectArtifacts { obj_path, asm_path })
    }

    /// Spawn the platform linker on `obj_path → output_path` and return
    /// the running child. The caller drives it via `try_wait` so the
    /// IDE can surface progress / handle cancel.
    /// `debug_info` adds `-g` so the linker keeps the debug map that
    /// dsymutil needs; Retail builds leave it off.
    pub fn spawn_linker(
        obj_path: &str,
        output_path: &str,
        debug_info: bool,
    ) -> Result<std::process::Child, String> {
        Self::spawn_linker_objs(&[obj_path], output_path, debug_info)
    }

    /// Same as [`spawn_linker`] for several object files (the profile build
    /// links the program object plus the profiler runtime object).
    pub fn spawn_linker_objs(
        obj_paths: &[&str],
        output_path: &str,
        debug_info: bool,
    ) -> Result<std::process::Child, String> {
        // Linker selection per platform:
        //   macOS:   `cc` resolves to Apple's clang; supports -lm/-g.
        //   Linux:   `cc` resolves to gcc/clang; need -no-pie because the
        //            LLVM TargetMachine is set to the static reloc model
        //            and modern distros default to PIE binaries.
        //   Windows: `cc` doesn't exist on PATH; LLVM ships `clang`. The
        //            MSVC CRT provides math intrinsics so -lm is not
        //            needed (and would error). PIE is a non-issue.
        let linker = if cfg!(target_os = "windows") {
            "clang"
        } else {
            "cc"
        };
        let link_args: Vec<&str> = {
            #[allow(unused_mut)]
            let mut a: Vec<&str> = obj_paths.to_vec();
            a.extend(["-o", output_path]);
            if debug_info {
                a.push("-g");
            }
            #[cfg(not(target_os = "windows"))]
            a.push("-lm");
            #[cfg(target_os = "linux")]
            a.push("-no-pie");
            // On Windows-MSVC, force LLVM's lld-link as the linker so we
            // don't depend on link.exe finding the right .lib paths from
            // a GitHub-runner-shaped environment, and explicitly pull in
            // the UCRT/legacy stdio libs the runtime helpers reference.
            #[cfg(all(target_os = "windows", target_env = "msvc"))]
            {
                a.push("-fuse-ld=lld");
                a.push("-lmsvcrt");
                a.push("-llegacy_stdio_definitions");
            }
            a
        };
        std::process::Command::new(linker)
            .args(&link_args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn linker: {e}"))
    }

    /// On macOS, spawn `dsymutil` to build the .dSYM bundle alongside
    /// `output_path`. No-op (returns `Ok(None)`) on other platforms.
    pub fn spawn_dsymutil(output_path: &str) -> Result<Option<std::process::Child>, String> {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("dsymutil")
                .arg(output_path)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map(Some)
                .map_err(|e| format!("failed to spawn dsymutil: {e}"))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = output_path;
            Ok(None)
        }
    }

    /// Remove a `.dSYM` bundle left behind by an earlier Debug build of
    /// the same output path, so a Retail binary isn't paired with debug
    /// info that no longer matches it.
    pub fn remove_stale_dsym(output_path: &str) {
        let _ = std::fs::remove_dir_all(format!("{output_path}.dSYM"));
    }

    /// One-shot synchronous build (used by callers that don't need
    /// progress info). Drives object emit + linker + dsymutil to
    /// completion blocking on each child.
    pub fn emit_executable(&self, output_path: &str) -> Result<(), String> {
        let debug_info = self.build_options.debug_info();
        let artifacts = self.emit_object(output_path)?;
        let linker = Self::spawn_linker(&artifacts.obj_path, output_path, debug_info)?;
        let link_out = linker
            .wait_with_output()
            .map_err(|e| format!("waiting on linker: {e}"))?;
        if !link_out.status.success() {
            // Surface stdout too — link.exe writes "unresolved external"
            // diagnostics there rather than to stderr.
            let stderr = String::from_utf8_lossy(&link_out.stderr);
            let stdout = String::from_utf8_lossy(&link_out.stdout);
            return Err(format!("linking failed: {stderr}{stdout}"));
        }

        let dsym = if debug_info {
            Self::spawn_dsymutil(output_path)?
        } else {
            Self::remove_stale_dsym(output_path);
            None
        };
        if let Some(dsym) = dsym {
            let dsym_out = dsym
                .wait_with_output()
                .map_err(|e| format!("waiting on dsymutil: {e}"))?;
            if !dsym_out.status.success() {
                let stderr = String::from_utf8_lossy(&dsym_out.stderr);
                return Err(format!("dsymutil failed: {stderr}"));
            }
        }

        let _ = std::fs::remove_file(&artifacts.obj_path);
        // The CLI one-shot path has nowhere to show the listing — drop it
        // rather than littering the output directory with a stray `.s`.
        if let Some(asm_path) = &artifacts.asm_path {
            let _ = std::fs::remove_file(asm_path);
        }
        Ok(())
    }

    /// Print the generated LLVM IR (for diagnostics).
    pub fn print_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    // ── runtime declarations ─────────────────────────────

    fn emit_runtime_decls(&self) {
        bruto_lang::runtime::emit_runtime(self.context, &self.module);
    }

    // ── debug info helpers ───────────────────────────────

    /// Open the console capture file at program start via the bruto runtime.
    fn emit_capture_file_init(&mut self, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let func = self.module.get_function("bruto_capture_open").unwrap();
        let capture = bruto_lang::target::console_capture_path();
        let path = self
            .builder
            .build_global_string_ptr(&capture, "capture_path")
            .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        self.builder
            .build_call(func, &[path.as_pointer_value().into()], "")
            .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        Ok(())
    }

    /// Set up `input` and `output` as predefined text files mapped to stdin/stdout.
    fn emit_predefined_files(&mut self, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let make_fn = self.module.get_function("bruto_make_predef_file").unwrap();

        for (var_name, which) in &[
            ("input", bruto_lang::target::Stdio::Stdin),
            ("output", bruto_lang::target::Stdio::Stdout),
        ] {
            let fp = bruto_lang::target::emit_load_stdio(
                &self.builder,
                &self.module,
                self.context,
                *which,
            );
            let s = self
                .builder
                .build_call(make_fn, &[fp.into()], "predef_s")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let alloca = self
                .entry_alloca(ptr_ty, var_name)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.builder
                .build_store(alloca, s)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.variables.insert(var_name.to_string(), alloca);
            self.var_types.insert(
                var_name.to_string(),
                PascalType::File {
                    elem: Box::new(PascalType::Char),
                },
            );
        }
        Ok(())
    }

    /// The debug-info file for a source-map id.
    fn di_file_for(&self, file: u32) -> DIFile<'ctx> {
        let Some(path) = crate::source_map::path_of(file) else {
            return self.di_file;
        };
        *self.di_files.borrow_mut().entry(file).or_insert_with(|| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let dir = path
                .parent()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.di_builder.create_file(&name, &dir)
        })
    }

    /// The scope for a location in `file` inside the current routine: the
    /// routine itself, or a lexical block naming `file` when the code came
    /// from somewhere else (an include file inside a routine body, say).
    fn scope_for_file(&self, scope: DIScope<'ctx>, file: u32) -> DIScope<'ctx> {
        if file == self.scope_file {
            return scope;
        }
        let key = (
            self.current_fn
                .map(|f| f.get_name().to_string_lossy().into_owned())
                .unwrap_or_default(),
            file,
        );
        *self.file_scopes.borrow_mut().entry(key).or_insert_with(|| {
            self.di_builder
                .create_lexical_block(scope, self.di_file_for(file), 0, 0)
                .as_debug_info_scope()
        })
    }

    fn set_debug_loc(&self, span: Span) {
        if let Some(scope) = self.current_scope {
            let scope = self.scope_for_file(scope, span.file);
            let loc = self.di_builder.create_debug_location(
                self.context,
                span.line,
                span.column,
                scope,
                None,
            );
            self.builder.set_current_debug_location(loc);
        }
    }

    // ── variable declarations ────────────────────────────

    fn compile_var_decl(
        &mut self,
        decl: &VarDecl,
        di_sub: DISubprogram<'ctx>,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(decl.span);
        let resolved_ty = self.resolve_type(&decl.ty);

        // `absolute`: no storage of its own, just another view of the
        // target's.
        if let Some(target) = &decl.absolute {
            let address = *self.lookup_var(target).ok_or_else(|| {
                CodeGenError::new(format!("undefined variable '{target}'"), Some(decl.span))
            })?;
            for name in &decl.names {
                self.variables.insert(name.clone(), address);
                self.var_types.insert(name.clone(), resolved_ty.clone());
            }
            return Ok(());
        }

        for name in &decl.names {
            let llvm_ty = self.llvm_type_for(&resolved_ty);
            let alloca = self
                .entry_alloca(llvm_ty, name)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;

            // Initialize scalar types to zero
            match &resolved_ty {
                PascalType::SizedInt { .. }
                | PascalType::Object(_)
                | PascalType::ShortString { .. } => {
                    self.builder
                        .build_store(alloca, llvm_ty.const_zero())
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::Integer
                | PascalType::Enum { .. }
                | PascalType::Subrange { .. }
                | PascalType::SubrangeExpr { .. } => {
                    self.builder
                        .build_store(alloca, self.context.i64_type().const_int(0, false))
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::Real => {
                    self.builder
                        .build_store(alloca, self.context.f64_type().const_float(0.0))
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::Boolean => {
                    self.builder
                        .build_store(alloca, self.context.bool_type().const_int(0, false))
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::Char => {
                    self.builder
                        .build_store(alloca, self.context.i8_type().const_int(0, false))
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::String => {
                    let empty = self
                        .builder
                        .build_global_string_ptr("", "empty_str")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                    self.builder
                        .build_store(alloca, empty.as_pointer_value())
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::Pointer(_) | PascalType::File { .. } | PascalType::Proc { .. } => {
                    let ptr_type = self.context.ptr_type(AddressSpace::default());
                    self.builder
                        .build_store(alloca, ptr_type.const_null())
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::Array { .. }
                | PascalType::Record { .. }
                | PascalType::Set { .. }
                | PascalType::Named(_)
                | PascalType::Untyped
                | PascalType::ArrayExpr { .. }
                | PascalType::ConformantArray { .. } => {
                    // Aggregate types: zero-initialized by alloca (no explicit store needed).
                }
            }

            // Debug variable info
            let di_type = self.create_debug_type(&resolved_ty);

            if let Some(di_type) = di_type {
                let di_var = self.di_builder.create_auto_variable(
                    di_sub.as_debug_info_scope(),
                    &self.display(name),
                    self.di_file_for(decl.span.file),
                    decl.span.line,
                    di_type,
                    true,
                    DIFlags::ZERO,
                    0,
                );

                let loc = self.di_builder.create_debug_location(
                    self.context,
                    decl.span.line,
                    decl.span.column,
                    di_sub.as_debug_info_scope(),
                    None,
                );

                self.di_builder.insert_declare_at_end(
                    alloca,
                    Some(di_var),
                    None,
                    loc,
                    self.builder.get_insert_block().unwrap(),
                );
            }

            self.variables.insert(name.clone(), alloca);
            self.var_types.insert(name.clone(), resolved_ty.clone());

            // Record watch-window metadata for top-level kinds.
            self.record_metadata(name, &resolved_ty);
        }

        Ok(())
    }

    /// The spelling the user wrote for a (lower-cased) identifier, for
    /// debug info, watch metadata and the profiler map.
    /// Make the values of the enumerations written in `ty` constants.
    fn register_enum_values(&mut self, ty: &PascalType) {
        let mut found = Vec::new();
        ty.for_each_enum(&mut |values| found.push(values.to_vec()));
        for values in found {
            for (i, val) in values.into_iter().enumerate() {
                self.enum_values.insert(val, i as i64);
            }
        }
    }

    fn display(&self, name: &str) -> String {
        // A nested routine (`outer/inner`) or a name split between units
        // (`initvideo@video`) shows as written.
        let base = name.rsplit('/').next().unwrap_or(name);
        let base = base.split_once('@').map_or(base, |(base, _)| base);
        self.display_names
            .get(base)
            .cloned()
            .unwrap_or_else(|| base.to_string())
    }

    /// The type a name denotes in `low`, `high` or `sizeof`: a variable's
    /// type, a declared type, or a built-in integer type.
    fn type_of_name(&self, name: &str) -> Option<PascalType> {
        self.lookup_var_type(name)
            .cloned()
            .or_else(|| self.type_defs.get(name).cloned())
            .or_else(|| crate::sema::builtin_type(name))
            .or(match name {
                "integer" => Some(PascalType::Integer),
                "real" => Some(PascalType::Real),
                "char" => Some(PascalType::Char),
                "boolean" => Some(PascalType::Boolean),
                "string" => Some(PascalType::String),
                "text" => Some(PascalType::File {
                    elem: Box::new(PascalType::Char),
                }),
                _ => None,
            })
    }

    /// Look a name up in the current routine's locals, then among the
    /// program-level globals.
    fn lookup_var(&self, name: &str) -> Option<&PointerValue<'ctx>> {
        self.variables
            .get(name)
            .or_else(|| self.global_vars.get(name))
    }

    /// Type of a local or, failing that, a program-level global.
    fn lookup_var_type(&self, name: &str) -> Option<&PascalType> {
        self.var_types
            .get(name)
            .or_else(|| self.global_types.get(name))
    }

    /// Declare a program-level variable or constant as an LLVM global,
    /// zero-initialised (strings start as `''`), with DWARF so the
    /// debugger can show it. Called before any procedure is compiled so
    /// every routine can reach it.
    fn declare_global(&mut self, name: &str, ty: &PascalType, span: Span) {
        let resolved = self.resolve_type(ty);
        let llvm_ty = self.llvm_type_for(&resolved);
        let symbol = format!("pascal.{name}");
        let global = self.module.add_global(llvm_ty, None, &symbol);
        global.set_linkage(Linkage::Internal);
        if matches!(resolved, PascalType::String) {
            global.set_initializer(&self.empty_string_constant());
        } else {
            global.set_initializer(&llvm_ty.const_zero());
        }

        if let Some(di_type) = self.create_debug_type(&resolved) {
            let gve = self.di_builder.create_global_variable_expression(
                self.compile_unit.as_debug_info_scope(),
                &self.display(name),
                "",
                self.di_file_for(span.file),
                span.line,
                di_type,
                true,
                None,
                None,
                0,
            );
            global.set_metadata(
                gve.as_metadata_value(self.context),
                self.context.get_kind_id("dbg"),
            );
        }

        self.global_vars
            .insert(name.to_string(), global.as_pointer_value());
        self.global_types.insert(name.to_string(), resolved.clone());
        self.record_metadata(name, &resolved);
    }

    /// A string literal as a private constant: one byte per character
    /// (Pascal strings are byte strings, so `#129` is the byte 129, not a
    /// UTF-8 sequence), NUL-terminated.
    fn string_literal(&self, text: &str) -> PointerValue<'ctx> {
        let bytes: Vec<u8> = text
            .chars()
            .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
            .collect();
        let value = self.context.const_string(&bytes, true);
        let global = self.module.add_global(value.get_type(), None, "str_lit");
        global.set_linkage(Linkage::Private);
        global.set_constant(true);
        global.set_unnamed_addr(true);
        global.set_initializer(&value);
        global.as_pointer_value()
    }

    /// Pointer to a shared, empty NUL-terminated string, usable as a
    /// constant initializer.
    fn empty_string_constant(&self) -> BasicValueEnum<'ctx> {
        let global = self
            .module
            .get_global("pascal.empty_str")
            .unwrap_or_else(|| {
                let bytes = self.context.const_string(b"", true);
                let g = self
                    .module
                    .add_global(bytes.get_type(), None, "pascal.empty_str");
                g.set_linkage(Linkage::Private);
                g.set_constant(true);
                g.set_initializer(&bytes);
                g
            });
        global.as_pointer_value().into()
    }

    fn record_metadata(&mut self, name: &str, ty: &PascalType) {
        // The IDE matches these names against lldb's output, which uses
        // the DWARF (display) spelling.
        let d = |n: &str| self.display(n);
        let name = d(name);
        let line = match ty {
            PascalType::Enum { values, .. } => {
                let values: Vec<String> = values.iter().map(|v| d(v)).collect();
                format!("{name}|enum|{}", values.join(","))
            }
            PascalType::Set { .. } => format!("{name}|set|"),
            // lldb has no Pascal support and prints 8-bit integers as C
            // chars; tell the IDE to show them as numbers.
            PascalType::SizedInt { bits: 8, signed } => {
                format!("{name}|int8|{}", if *signed { "s" } else { "u" })
            }
            PascalType::Record {
                fields, variant, ..
            } => {
                let mut parts: Vec<String> = fields
                    .iter()
                    .map(|(n, t)| format!("{}={}", d(n), type_short(t)))
                    .collect();
                if let Some(v) = variant {
                    parts.push(format!("__tag={}", d(&v.tag_name)));
                    for (vals, vfields) in &v.variants {
                        let val_str: Vec<String> = vals
                            .iter()
                            .map(|v| v.as_int().unwrap_or(0).to_string())
                            .collect();
                        let f_str: Vec<String> = vfields
                            .iter()
                            .map(|(n, t)| format!("{}={}", d(n), type_short(t)))
                            .collect();
                        parts.push(format!("__case[{}]={}", val_str.join(","), f_str.join(",")));
                    }
                    return self
                        .metadata_lines
                        .push(format!("{name}|vrec|{}", parts.join(";")));
                }
                format!("{name}|rec|{}", parts.join(";"))
            }
            _ => return,
        };
        self.metadata_lines.push(line);
    }

    /// Type of a constant: its annotation if it has one, else inferred
    /// from the initializer.
    fn const_decl_type(&self, c: &ConstDecl) -> PascalType {
        match &c.ty {
            Some(t) => self.resolve_type(t),
            None => self.infer_expr_type(&c.value),
        }
    }

    /// Store a typed constant's value into `slot`: a plain expression, or a
    /// record / array initializer filled in field by field.
    fn store_initializer(
        &mut self,
        slot: PointerValue<'ctx>,
        value: &Expr,
        ty: &PascalType,
    ) -> Result<(), CodeGenError> {
        let resolved = self.resolve_type(ty);
        let err = |e: inkwell::builder::BuilderError| {
            CodeGenError::new(e.to_string(), Some(value.span()))
        };
        match (value, &resolved) {
            (
                Expr::RecordInit {
                    fields: inits,
                    span,
                },
                PascalType::Record { fields, .. },
            ) => {
                let record_ty = self.llvm_type_for(&resolved);
                for (name, init) in inits {
                    let index = fields.iter().position(|(n, _)| n == name).ok_or_else(|| {
                        CodeGenError::new(
                            format!("no field '{}' in this record", self.display(name)),
                            Some(*span),
                        )
                    })?;
                    let field_ty = fields[index].1.clone();
                    let addr = self
                        .builder
                        .build_struct_gep(record_ty, slot, index as u32, "init_field")
                        .map_err(err)?;
                    self.store_initializer(addr, init, &field_ty)?;
                }
                Ok(())
            }
            (Expr::ArrayInit { elements, span }, PascalType::Array { lo, hi, elem }) => {
                if elements.len() as i64 != hi - lo + 1 {
                    return Err(CodeGenError::new(
                        format!(
                            "the array has {} elements but {} values are given",
                            hi - lo + 1,
                            elements.len()
                        ),
                        Some(*span),
                    ));
                }
                let array_ty = self.llvm_type_for(&resolved);
                let i64_ty = self.context.i64_type();
                for (i, init) in elements.iter().enumerate() {
                    let addr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                array_ty,
                                slot,
                                &[i64_ty.const_zero(), i64_ty.const_int(i as u64, false)],
                                "init_elem",
                            )
                            .map_err(err)?
                    };
                    self.store_initializer(addr, init, elem)?;
                }
                Ok(())
            }
            // `array[0..2] of Char = #218#196#191`: the characters, then
            // zeros.
            (Expr::StrLit(text, span), PascalType::Array { lo, hi, elem })
                if self.resolve_type(elem) == PascalType::Char =>
            {
                let bytes: Vec<u8> = text.chars().map(|c| c as u32 as u8).collect();
                let len = (hi - lo + 1) as usize;
                if bytes.len() > len {
                    return Err(CodeGenError::new(
                        format!(
                            "the string has {} characters but the array holds {len}",
                            bytes.len()
                        ),
                        Some(*span),
                    ));
                }
                let i8_ty = self.context.i8_type();
                let i64_ty = self.context.i64_type();
                for i in 0..len {
                    let addr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                i8_ty,
                                slot,
                                &[i64_ty.const_int(i as u64, false)],
                                "init_char",
                            )
                            .map_err(err)?
                    };
                    let byte = bytes.get(i).copied().unwrap_or(0);
                    self.builder
                        .build_store(addr, i8_ty.const_int(byte as u64, false))
                        .map_err(err)?;
                }
                Ok(())
            }
            (Expr::RecordInit { span, .. } | Expr::ArrayInit { span, .. }, _) => Err(
                CodeGenError::new("this value doesn't match the constant's type", Some(*span)),
            ),
            _ => {
                let val = self.compile_expr(value)?;
                self.store_value(slot, val, &resolved, value.span())
            }
        }
    }

    /// A constant declared inside a routine: a local slot holding its value.
    fn compile_local_const(&mut self, c: &ConstDecl) -> Result<(), CodeGenError> {
        self.set_debug_loc(c.span);
        let ty = self.const_decl_type(c);
        let slot = self
            .entry_alloca(self.llvm_type_for(&ty), &c.name)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(c.span)))?;
        self.store_initializer(slot, &c.value, &ty)?;
        self.variables.insert(c.name.clone(), slot);
        self.var_types.insert(c.name.clone(), ty);
        Ok(())
    }

    /// Store a program-level constant's value into the global that
    /// `declare_global` created for it.
    fn compile_const_decl(&mut self, c: &ConstDecl) -> Result<(), CodeGenError> {
        self.set_debug_loc(c.span);
        if matches!(c.value, Expr::RecordInit { .. } | Expr::ArrayInit { .. })
            || matches!(self.const_decl_type(c), PascalType::Array { .. })
        {
            let ty = self.const_decl_type(c);
            let slot = *self.global_vars.get(&c.name).ok_or_else(|| {
                CodeGenError::new(
                    format!("constant '{}' was not declared", c.name),
                    Some(c.span),
                )
            })?;
            return self.store_initializer(slot, &c.value, &ty);
        }
        let val = self.compile_expr(&c.value)?;
        let ty = self.const_decl_type(c);
        let alloca = *self.global_vars.get(&c.name).ok_or_else(|| {
            CodeGenError::new(
                format!("constant '{}' was not declared", c.name),
                Some(c.span),
            )
        })?;
        // Converted as any store: an integer into a real, a string into
        // a short string's buffer.
        self.store_value(alloca, val, &ty, c.span)
    }

    fn compile_proc_decl(&mut self, proc: &ProcDecl) -> Result<(), CodeGenError> {
        // A method implementation, `procedure TFoo.Bar`, belongs to a
        // method declared in TFoo's type.
        let method = match &proc.owner {
            Some(owner) => {
                let m = self
                    .objects
                    .get(owner)
                    .and_then(|o| o.methods.get(&proc.name))
                    .filter(|m| m.symbol == format!("{owner}.{}", proc.name))
                    .cloned()
                    .ok_or_else(|| {
                        CodeGenError::new(
                            format!(
                                "'{}' is not a method declared in '{}'",
                                self.display(&proc.name),
                                self.display(owner)
                            ),
                            Some(proc.span),
                        )
                    })?;
                Some((owner.clone(), m))
            }
            None => None,
        };
        let symbol = method
            .as_ref()
            .map(|(_, m)| m.symbol.clone())
            .unwrap_or_else(|| proc.name.clone());
        // The implementation may leave out the parameters and result type
        // given in the object declaration.
        let params: Vec<ParamGroup> = match &method {
            Some((_, m)) if proc.params.is_empty() => m.params.clone(),
            _ => proc.params.clone(),
        };
        let return_type = proc
            .return_type
            .clone()
            .or_else(|| method.as_ref().and_then(|(_, m)| m.return_type.clone()));
        let is_constructor = matches!(&method, Some((_, m)) if m.kind == RoutineKind::Constructor);

        // Build parameter type list
        let mut param_llvm_types: Vec<inkwell::types::BasicMetadataTypeEnum> = Vec::new();
        let mut param_info: Vec<(String, ParamMode, PascalType)> = Vec::new();

        // A method's hidden parameters: Self (by reference), and the VMT
        // for a constructor.
        if let Some((owner, _)) = &method {
            let record = self.objects[owner].record.clone();
            param_llvm_types.push(self.context.ptr_type(AddressSpace::default()).into());
            param_info.push(("self".to_string(), ParamMode::Var, record));
            if is_constructor {
                param_llvm_types.push(self.context.ptr_type(AddressSpace::default()).into());
                param_info.push((
                    "__vmt_arg".to_string(),
                    ParamMode::Value,
                    PascalType::Pointer(Box::new(PascalType::Untyped)),
                ));
            }
        }

        // Captures (only set if this proc is currently registered as nested)
        let captures = self
            .proc_captures
            .get(&proc.name)
            .cloned()
            .unwrap_or_default();
        for (cname, cty) in &captures {
            // Each capture is passed as a pointer (var-by-ref).
            param_llvm_types.push(self.context.ptr_type(AddressSpace::default()).into());
            param_info.push((cname.clone(), ParamMode::Var, cty.clone()));
        }

        for group in &params {
            let resolved_group_ty = self.resolve_type(&group.ty);
            for name in &group.names {
                // Conformant array parameter: emit ptr + i64 lo + i64 hi.
                if let PascalType::ConformantArray {
                    lo_name, hi_name, ..
                } = &resolved_group_ty
                {
                    param_llvm_types.push(self.context.ptr_type(AddressSpace::default()).into());
                    param_llvm_types.push(self.context.i64_type().into());
                    param_llvm_types.push(self.context.i64_type().into());
                    param_info.push((name.clone(), ParamMode::Var, resolved_group_ty.clone()));
                    param_info.push((lo_name.clone(), ParamMode::Value, PascalType::Integer));
                    param_info.push((hi_name.clone(), ParamMode::Value, PascalType::Integer));
                    continue;
                }
                let ty = if group.mode == ParamMode::Var {
                    self.context.ptr_type(AddressSpace::default()).into()
                } else {
                    self.abi_type_for(&resolved_group_ty).into()
                };
                param_llvm_types.push(ty);
                param_info.push((name.clone(), group.mode, resolved_group_ty.clone()));
            }
        }

        // Build function type
        let varargs = proc.varargs && proc.external.is_some();
        let fn_type = match &method {
            Some((_, m)) => self.method_fn_type(m),
            None => match &return_type {
                Some(ret_ty) => self
                    .abi_type_for(ret_ty)
                    .fn_type(&param_llvm_types, varargs),
                None => self.context.void_type().fn_type(&param_llvm_types, varargs),
            },
        };

        // An external routine is the C function itself, under its own
        // symbol; calls by the Pascal name are redirected to it.
        if let Some(c_symbol) = &proc.external {
            if self.module.get_function(c_symbol).is_none() {
                self.module
                    .add_function(c_symbol, fn_type, Some(Linkage::External));
            }
            self.external_symbols
                .insert(proc.name.clone(), c_symbol.clone());
            self.proc_return_types
                .insert(c_symbol.clone(), return_type.clone());
            self.proc_param_modes.insert(c_symbol.clone(), param_info);
            return Ok(());
        }

        // Reuse existing function if this is the real definition after a
        // forward declaration (or a method's prototype)
        let func = if let Some(existing) = self.module.get_function(&symbol) {
            existing
        } else {
            self.module.add_function(&symbol, fn_type, None)
        };

        // Store metadata (a method's was registered with its object type)
        if method.is_none() {
            self.proc_return_types
                .insert(proc.name.clone(), return_type.clone());
            self.proc_param_modes
                .insert(proc.name.clone(), param_info.clone());
        }

        // Forward declaration: empty body means just register the prototype
        if proc.is_forward {
            return Ok(());
        }

        // Debug info
        let di_i64 = self
            .di_builder
            .create_basic_type("integer", 64, 0x05, DIFlags::ZERO)
            .unwrap();
        let proc_file = self.di_file_for(proc.span.file);
        let di_fn_type = self.di_builder.create_subroutine_type(
            proc_file,
            return_type.as_ref().map(|_| di_i64.as_type()),
            &[],
            DIFlags::ZERO,
        );
        let display_name = match &method {
            Some((owner, _)) => format!("{}.{}", self.display(owner), self.display(&proc.name)),
            None => self.display(&proc.name),
        };
        let di_sub = self.di_builder.create_function(
            self.compile_unit.as_debug_info_scope(),
            &display_name,
            None,
            proc_file,
            proc.span.line,
            di_fn_type,
            true,
            true,
            proc.body.span.line,
            DIFlags::ZERO,
            false,
        );
        func.set_subprogram(di_sub);

        let bb = self.context.append_basic_block(func, "entry");
        self.builder.position_at_end(bb);

        // Save current scope
        let saved_vars = std::mem::take(&mut self.variables);
        let saved_types = std::mem::take(&mut self.var_types);
        let saved_fn = self.current_fn;
        let saved_scope = self.current_scope;
        let saved_scope_file = self.scope_file;
        self.saved_scopes.push((saved_vars, saved_types));

        self.current_fn = Some(func);
        self.current_scope = Some(di_sub.as_debug_info_scope());
        self.scope_file = proc.span.file;
        self.set_debug_loc(proc.span);
        let proc_name = proc.name.clone();
        self.emit_prof_enter(&proc_name, proc.span.line)?;

        // Create allocas for parameters
        for (i, (name, mode, ty)) in param_info.iter().enumerate() {
            let param_val = func.get_nth_param(i as u32).unwrap();
            let resolved_ty = self.resolve_type(ty);
            if *mode == ParamMode::Var {
                // var param: the parameter IS a pointer to the caller's variable.
                // Use it directly as the alloca — loads/stores go through to the caller.
                self.variables
                    .insert(name.clone(), param_val.into_pointer_value());
                self.var_types.insert(name.clone(), resolved_ty);
            } else {
                // value param: copy into alloca
                let llvm_ty = self.llvm_type_for(&resolved_ty);
                let alloca = self
                    .entry_alloca(llvm_ty, name)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
                // (A short string arrives as a C string and is copied in.)
                self.store_value(alloca, param_val, &resolved_ty, proc.span)?;
                self.variables.insert(name.clone(), alloca);
                self.var_types.insert(name.clone(), resolved_ty);
            }
        }

        // A constructor allocates and/or stamps the VMT before its body;
        // `Self` is whatever that produced.
        let saved_object = self.current_object.clone();
        let saved_method_context = self.method_context;
        // A routine nested in a method is its own function: it reaches the
        // object's fields through captures, and its `Self` is captured too
        // (set up below, once the captures are bound).
        let enclosing_object = self.current_object.take();
        self.method_context = None;
        if let Some((owner, _)) = &method {
            let self_param = func.get_nth_param(0).unwrap().into_pointer_value();
            let self_ptr = if is_constructor {
                let vmt_param = func.get_nth_param(1).unwrap().into_pointer_value();
                self.constructor_prologue(owner, self_param, vmt_param, proc.span)?
            } else {
                self_param
            };
            self.variables.insert("self".to_string(), self_ptr);
            self.current_object = Some(owner.clone());
            self.method_context = Some(objects::MethodContext {
                self_ptr,
                in_constructor: is_constructor,
            });
            self.bind_object_fields(owner, self_ptr, proc.span)?;
        }

        // Nested in a method: the object's methods are reachable through
        // the captured `Self`.
        if method.is_none()
            && let (Some(object), Some(self_ptr)) =
                (enclosing_object, self.variables.get("self").copied())
        {
            self.current_object = Some(object);
            self.method_context = Some(objects::MethodContext {
                self_ptr,
                in_constructor: false,
            });
        }

        // Function result variable (Pascal assigns to function name)
        if let Some(ref ret_ty) = return_type {
            let resolved = self.resolve_type(ret_ty);
            let llvm_ty = self.llvm_type_for(&resolved);
            let alloca = self
                .entry_alloca(llvm_ty, &proc.name)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
            let zero = if resolved == PascalType::String {
                self.empty_string_constant()
            } else {
                llvm_ty.const_zero()
            };
            self.builder
                .build_store(alloca, zero)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
            self.variables.insert(proc.name.clone(), alloca);
            self.var_types.insert(proc.name.clone(), ret_ty.clone());
            // An overload (`name$2`) assigns its result by its plain name.
            if let Some((base, _)) = proc.name.split_once('$') {
                self.variables.insert(base.to_string(), alloca);
                self.var_types.insert(base.to_string(), ret_ty.clone());
            }
            // `Result` names the same variable (FPC's objfpc and Delphi
            // modes), unless a parameter is called that.
            if !self.variables.contains_key("result") {
                self.variables.insert("result".to_string(), alloca);
                self.var_types.insert("result".to_string(), ret_ty.clone());
            }
        }

        // Local types, visible until the routine ends.
        let mut shadowed_types = Vec::new();
        for td in &proc.type_decls {
            let previous = self.type_defs.insert(td.name.clone(), td.ty.clone());
            shadowed_types.push((td.name.clone(), previous));
            self.register_enum_values(&td.ty);
        }
        for v in &proc.vars {
            self.register_enum_values(&v.ty);
        }

        // Local constants: a slot set when the routine starts.
        for c in &proc.consts {
            self.compile_local_const(c)?;
        }

        // Local variables
        for var_decl in &proc.vars {
            self.compile_var_decl(var_decl, di_sub)?;
        }

        // Nested procedures/functions
        // For each nested proc, compute its captures (names referenced in its body
        // that exist in this enclosing scope but not as locals/params of the nested proc).
        // Save current builder position so we can restore after compiling nested procs.
        let saved_block = self.builder.get_insert_block();
        for nested in &proc.nested_procs {
            let mut nested_locals: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            for grp in &nested.params {
                for n in &grp.names {
                    nested_locals.insert(n.clone());
                }
            }
            for vd in &nested.vars {
                for n in &vd.names {
                    nested_locals.insert(n.clone());
                }
            }
            // The nested proc's name is its own result variable (if function).
            nested_locals.insert(nested.name.clone());

            let mut found: Vec<(String, PascalType)> = Vec::new();
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            // Routines nested inside it reach this scope through it, so
            // what they use is captured too.
            let mut bodies = vec![&nested.body];
            let mut stack: Vec<&ProcDecl> = nested.nested_procs.iter().collect();
            while let Some(inner) = stack.pop() {
                bodies.push(&inner.body);
                stack.extend(inner.nested_procs.iter());
            }
            for body in bodies {
                collect_captures(body, &nested_locals, &self.var_types, &mut found, &mut seen);
            }
            // Inside a method, a nested routine can call the object's
            // methods by bare name, so it always gets `Self`.
            if self.current_object.is_some()
                && !seen.contains("self")
                && let Some(self_ty) = self.var_types.get("self").cloned()
            {
                found.push(("self".to_string(), self_ty));
            }
            self.proc_captures.insert(nested.name.clone(), found);
        }
        // A nested routine that calls a sibling, or a routine it is
        // nested in, must also capture what that one captures, to pass it
        // on.
        let siblings: HashMap<String, PascalType> = self
            .proc_captures
            .keys()
            .map(|n| (n.clone(), PascalType::Integer))
            .collect();
        let calls: Vec<(String, Vec<String>)> = proc
            .nested_procs
            .iter()
            .map(|n| {
                let mut called = Vec::new();
                let mut seen = std::collections::HashSet::new();
                let mut bodies = vec![&n.body];
                let mut stack: Vec<&ProcDecl> = n.nested_procs.iter().collect();
                while let Some(inner) = stack.pop() {
                    bodies.push(&inner.body);
                    stack.extend(inner.nested_procs.iter());
                }
                for body in bodies {
                    collect_captures(
                        body,
                        &std::collections::HashSet::new(),
                        &siblings,
                        &mut called,
                        &mut seen,
                    );
                }
                (n.name.clone(), called.into_iter().map(|(c, _)| c).collect())
            })
            .collect();
        loop {
            let mut changed = false;
            for (caller, callees) in &calls {
                for callee in callees {
                    let needed = self.proc_captures.get(callee).cloned().unwrap_or_default();
                    let have = self.proc_captures.entry(caller.clone()).or_default();
                    for capture in needed {
                        if self.var_types.contains_key(&capture.0)
                            && !have.iter().any(|(n, _)| *n == capture.0)
                        {
                            have.push(capture);
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        // Now compile each nested proc. Each call to compile_proc_decl saves and
        // restores self.variables/self.var_types, so we can call from within the parent.
        for nested in &proc.nested_procs {
            self.compile_proc_decl(nested)?;
        }
        // Restore parent's builder position, and its debug location (the
        // nested routines left theirs set).
        if let Some(bb) = saved_block {
            self.builder.position_at_end(bb);
        }
        if !proc.nested_procs.is_empty() {
            self.set_debug_loc(proc.body.span);
        }

        // Compile body. Its exit block, loop state and result name are set
        // only now, after the nested procedures, which install their own.
        let saved_exit = self.begin_exit_block(func);
        let saved_loops = std::mem::take(&mut self.loop_targets);
        let saved_function = std::mem::replace(
            &mut self.current_function,
            return_type.as_ref().map(|_| proc.name.clone()),
        );
        self.compile_block(&proc.body)?;
        self.enter_exit_block(proc.body.end_span)?;

        // Return (profiler exit first so the routine's time closes)
        self.emit_prof_exit()?;
        if is_constructor {
            let self_ptr = self.method_context.unwrap().self_ptr;
            self.builder
                .build_return(Some(&self_ptr))
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
        } else if let Some(ref ret_ty) = return_type {
            let llvm_ty = self.llvm_type_for(ret_ty);
            let alloca = *self.lookup_var(&proc.name).unwrap();
            // A short string result is returned as a C string.
            let ret_val = if let PascalType::ShortString { .. } = self.resolve_type(ret_ty) {
                self.short_to_c_string(alloca, proc.span)?
            } else {
                self.builder
                    .build_load(llvm_ty, alloca, "retval")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?
            };
            self.builder
                .build_return(Some(&ret_val))
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
        } else {
            self.builder
                .build_return(None)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
        }

        // Restore scope
        for (name, previous) in shadowed_types.into_iter().rev() {
            match previous {
                Some(ty) => self.type_defs.insert(name, ty),
                None => self.type_defs.remove(&name),
            };
        }
        self.exit_block = saved_exit;
        self.loop_targets = saved_loops;
        self.current_function = saved_function;
        self.current_object = saved_object;
        self.method_context = saved_method_context;
        let (restored_vars, restored_types) = self.saved_scopes.pop().unwrap();
        self.variables = restored_vars;
        self.var_types = restored_types;
        self.current_fn = saved_fn;
        self.current_scope = saved_scope;
        self.scope_file = saved_scope_file;

        Ok(())
    }

    // ── block / statements ───────────────────────────────

    /// The LLVM type a value of type `ty` has when passed by value or
    /// returned: a short string travels as an ordinary (C) string.
    fn abi_type_for(&self, ty: &PascalType) -> inkwell::types::BasicTypeEnum<'ctx> {
        match self.resolve_type(ty) {
            PascalType::ShortString { .. } => self
                .context
                .ptr_type(AddressSpace::default())
                .as_basic_type_enum(),
            resolved => self.llvm_type_for(&resolved),
        }
    }

    /// Turn a value just loaded from a slot of type `ty` into its
    /// expression form: integers narrower than 64 bits are sign- or
    /// zero-extended, so arithmetic only ever sees 64-bit integers.
    fn widen_loaded(
        &self,
        val: BasicValueEnum<'ctx>,
        ty: &PascalType,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let resolved = self.resolve_type(ty);
        // A short string's bytes become an ordinary (C) string.
        if let PascalType::ShortString { .. } = resolved
            && val.is_array_value()
        {
            let temp = self
                .entry_alloca(val.get_type(), "short_tmp")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.builder
                .build_store(temp, val)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            return self.short_to_c_string(temp, span);
        }
        let PascalType::SizedInt { bits, signed } = resolved else {
            return Ok(val);
        };
        if bits >= 64 || !val.is_int_value() {
            return Ok(val);
        }
        let iv = val.into_int_value();
        let i64_ty = self.context.i64_type();
        let wide = if signed {
            self.builder.build_int_s_extend(iv, i64_ty, "sext")
        } else {
            self.builder.build_int_z_extend(iv, i64_ty, "zext")
        };
        Ok(wide
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .into())
    }

    /// Convert an expression value so it can be stored into a slot of
    /// type `ty`: integers are truncated or extended to the slot's width,
    /// and integers stored into a `real` are converted.
    fn convert_for_store(
        &self,
        val: BasicValueEnum<'ctx>,
        ty: &PascalType,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let resolved = self.resolve_type(ty);
        let target = self.llvm_type_for(&resolved);
        // A one-character string stored into a char.
        let val = if resolved == PascalType::Char {
            self.first_char_if_string(val, span)?
        } else {
            val
        };
        // A char stored into (or passed as) a string.
        if matches!(
            resolved,
            PascalType::String | PascalType::ShortString { .. }
        ) {
            return self.as_string_value(val, span);
        }
        let BasicValueEnum::IntValue(iv) = val else {
            return Ok(val);
        };
        Ok(match target {
            inkwell::types::BasicTypeEnum::IntType(it) => {
                let (from, to) = (iv.get_type().get_bit_width(), it.get_bit_width());
                if from > to {
                    self.builder
                        .build_int_truncate(iv, it, "trunc")
                        .map_err(err)?
                        .into()
                } else if from < to && from <= 8 {
                    // Booleans and chars are unsigned.
                    self.builder
                        .build_int_z_extend(iv, it, "zext")
                        .map_err(err)?
                        .into()
                } else if from < to {
                    self.builder
                        .build_int_s_extend(iv, it, "sext")
                        .map_err(err)?
                        .into()
                } else {
                    val
                }
            }
            inkwell::types::BasicTypeEnum::FloatType(ft) => self
                .builder
                .build_signed_int_to_float(iv, ft, "itof")
                .map_err(err)?
                .into(),
            _ => val,
        })
    }

    /// Compile `expr` as a value of type `target`. An array assigned to a
    /// pointer (`P := CharArray`, as FPC allows for `PChar`) is its
    /// address.
    fn compile_expr_for(
        &mut self,
        expr: &Expr,
        target: &PascalType,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        if matches!(self.resolve_type(target), PascalType::Pointer(_))
            && self.is_addressable(expr)
            && matches!(
                self.resolve_type(&self.infer_declared_type(expr)),
                PascalType::Array { .. }
            )
        {
            return Ok(self.resolve_array_base(expr, expr.span())?.0.into());
        }
        self.compile_expr(expr)
    }

    /// `val` as a string: a char becomes a one-character string, anything
    /// else is returned unchanged.
    fn as_string_value(
        &self,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let BasicValueEnum::IntValue(ch) = val else {
            return Ok(val);
        };
        if ch.get_type().get_bit_width() != 8 {
            return Ok(val);
        }
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i8_ty = self.context.i8_type();
        let i64_ty = self.context.i64_type();
        let malloc = self.module.get_function("malloc").unwrap();
        let buf = self
            .builder
            .build_call(malloc, &[i64_ty.const_int(2, false).into()], "char_str")
            .map_err(err)?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        self.builder.build_store(buf, ch).map_err(err)?;
        let end = unsafe {
            self.builder
                .build_in_bounds_gep(i8_ty, buf, &[i64_ty.const_int(1, false)], "char_end")
                .map_err(err)?
        };
        self.builder
            .build_store(end, i8_ty.const_zero())
            .map_err(err)?;
        Ok(buf.into())
    }

    /// Store an expression value into a slot of type `ty`, converting it
    /// first (see [`Self::convert_for_store`]).
    fn store_value(
        &self,
        ptr: PointerValue<'ctx>,
        val: BasicValueEnum<'ctx>,
        ty: &PascalType,
        span: Span,
    ) -> Result<(), CodeGenError> {
        // Into a short string: copy (and truncate) the characters.
        if let PascalType::ShortString { max } = self.resolve_type(ty) {
            return self.store_short_string(ptr, val, max, span);
        }
        let val = self.convert_for_store(val, ty, span)?;
        self.builder
            .build_store(ptr, val)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    /// Allocate a stack slot in the current routine's entry block, so a
    /// slot used inside a loop is reserved once per call rather than once
    /// per iteration. Returns the same type as `Builder::build_alloca` so
    /// call sites keep their error handling.
    fn entry_alloca<T: BasicType<'ctx>>(
        &self,
        ty: T,
        name: &str,
    ) -> Result<PointerValue<'ctx>, inkwell::builder::BuilderError> {
        let entry = self
            .current_fn
            .and_then(|f| f.get_first_basic_block())
            .expect("alloca outside a function");
        let builder = self.context.create_builder();
        match entry.get_first_instruction() {
            Some(first) => builder.position_before(&first),
            None => builder.position_at_end(entry),
        }
        builder.build_alloca(ty, name)
    }

    /// `write` or `writeln`, for a write statement.
    fn write_name(stmt: &Statement) -> &'static str {
        if matches!(stmt, Statement::WriteLn { .. }) {
            "writeln"
        } else {
            "write"
        }
    }

    /// Branch to `target`, then continue emitting into a fresh block that
    /// nothing jumps to, so statements after a `break`, `exit` and so on
    /// still have somewhere valid to go.
    fn jump_to(
        &mut self,
        target: BasicBlock<'ctx>,
        dead_name: &str,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.builder
            .build_unconditional_branch(target)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let dead = self
            .context
            .append_basic_block(self.current_fn.unwrap(), dead_name);
        self.builder.position_at_end(dead);
        Ok(())
    }

    /// Start the current routine's return block, which `exit` jumps to.
    /// Returns the previous one so the caller can restore it.
    fn begin_exit_block(&mut self, func: FunctionValue<'ctx>) -> Option<BasicBlock<'ctx>> {
        let exit = self.context.append_basic_block(func, "exit");
        self.exit_block.replace(exit)
    }

    /// Fall through into the return block, moved to the end of the function
    /// so the listing reads top to bottom, and position the builder there.
    fn enter_exit_block(&mut self, span: Span) -> Result<(), CodeGenError> {
        let exit = self.exit_block.expect("exit block not started");
        self.builder
            .build_unconditional_branch(exit)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        if let Some(last) = self.current_fn.and_then(|f| f.get_last_basic_block())
            && last != exit
        {
            let _ = exit.move_after(last);
        }
        self.builder.position_at_end(exit);
        Ok(())
    }

    /// The current routine's `_end_bp` slot, created in its entry block on
    /// first use.
    fn end_breakpoint_slot(&mut self) -> Result<PointerValue<'ctx>, CodeGenError> {
        let func = self.current_fn.expect("block outside a function");
        let key = func.get_name().to_string_lossy().into_owned();
        if let Some(slot) = self.end_bp_slots.get(&key) {
            return Ok(*slot);
        }
        let i64_ty = self.context.i64_type();
        let slot = self
            .entry_alloca(i64_ty, "_end_bp")
            .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        // Initialise it right after the allocas so the load reads a defined
        // value.
        let init = self.context.create_builder();
        let entry = func.get_first_basic_block().unwrap();
        let mut after_allocas = entry.get_first_instruction();
        while let Some(i) = after_allocas {
            if i.get_opcode() != inkwell::values::InstructionOpcode::Alloca {
                break;
            }
            after_allocas = i.get_next_instruction();
        }
        match after_allocas {
            Some(i) => init.position_before(&i),
            None => init.position_at_end(entry),
        }
        init.build_store(slot, i64_ty.const_zero())
            .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        self.end_bp_slots.insert(key, slot);
        Ok(slot)
    }

    fn compile_block(&mut self, block: &Block) -> Result<(), CodeGenError> {
        for stmt in &block.statements {
            self.compile_statement(stmt)?;
        }
        // Give the `end` keyword an instruction of its own so breakpoints
        // can be set there: a volatile load from a per-routine dummy slot.
        // Retail builds carry no line info, so they skip it.
        if self.build_options.debug_info() {
            self.set_debug_loc(block.end_span);
            let slot = self.end_breakpoint_slot()?;
            let load = self
                .builder
                .build_load(self.context.i64_type(), slot, "_end_bp")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(block.end_span)))?;
            if let Some(inst) = load.as_instruction_value() {
                let _ = inst.set_volatile(true);
            }
        }
        Ok(())
    }

    fn compile_statement(&mut self, stmt: &Statement) -> Result<(), CodeGenError> {
        if self.instrument && !matches!(stmt, Statement::Label { .. }) {
            // The hook carries the statement's own debug location so the
            // call is attributed to the line it measures.
            self.set_debug_loc(stmt.span());
            self.emit_prof_line(stmt.span())?;
        }
        match stmt {
            // `System.X(...)`: the built-in X, whatever else X names here.
            Statement::ProcCall { name, args, span } if name.starts_with("system.") => {
                let bare = Statement::ProcCall {
                    name: name["system.".len()..].to_string(),
                    args: args.clone(),
                    span: *span,
                };
                let saved = std::mem::replace(&mut self.bypass_user, true);
                let result = self.compile_statement(&bare);
                self.bypass_user = saved;
                result
            }
            Statement::Assignment { target, expr, span } => {
                self.set_debug_loc(*span);
                let ty = self
                    .lookup_var_type(target.as_str())
                    .cloned()
                    .unwrap_or(PascalType::Integer);
                let val = self.compile_expr_for(expr, &ty)?;
                let alloca = *self.lookup_var(target.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{target}'"), Some(*span))
                })?;
                self.store_value(alloca, val, &ty, *span)
            }
            Statement::DerefAssignment { target, expr, span } => {
                self.set_debug_loc(*span);
                let val = self.compile_expr(expr)?;
                let alloca = *self.lookup_var(target.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{target}'"), Some(*span))
                })?;
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let ptr_val = self
                    .builder
                    .build_load(ptr_type, alloca, "ptr_val")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                // File buffer variable: `f^ := x`
                if let Some(t) = self.lookup_var_type(target.as_str()).cloned()
                    && matches!(self.resolve_type(&t), PascalType::File { .. })
                {
                    let ch = if val.is_int_value() {
                        let iv = val.into_int_value();
                        if iv.get_type().get_bit_width() == 8 {
                            iv
                        } else {
                            self.builder
                                .build_int_truncate(iv, self.context.i8_type(), "fbufc")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        }
                    } else {
                        return Err(CodeGenError::new(
                            "file buffer assignment expects char/int value",
                            Some(*span),
                        ));
                    };
                    let f = self.module.get_function("bruto_file_buf_store").unwrap();
                    self.builder
                        .build_call(f, &[ptr_val.into(), ch.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(());
                }
                let pointed = match self
                    .lookup_var_type(target.as_str())
                    .map(|t| self.resolve_type(t))
                {
                    Some(PascalType::Pointer(inner)) => *inner,
                    _ => PascalType::Integer,
                };
                self.store_value(ptr_val.into_pointer_value(), val, &pointed, *span)
            }
            Statement::If {
                condition,
                then_branch,
                else_branch,
                span,
            } => self.compile_if(condition, then_branch, else_branch.as_ref(), *span),
            Statement::While {
                condition,
                body,
                span,
            } => self.compile_while(condition, body, *span),
            Statement::WriteLn { args, span } | Statement::Write { args, span }
                if self.self_method(Self::write_name(stmt)).is_some() =>
            {
                let name = Self::write_name(stmt);
                let args: Vec<Expr> = args.iter().map(|a| a.expr.clone()).collect();
                self.compile_proc_call(name, &args, *span)
            }
            Statement::WriteLn { args, span } => self.compile_write(args, true, *span),
            Statement::Write { args, span } => self.compile_write(args, false, *span),
            Statement::For {
                var,
                from,
                to,
                downto,
                body,
                span,
            } => self.compile_for(var, from, to, *downto, body, *span),
            Statement::RepeatUntil {
                body,
                condition,
                span,
            } => self.compile_repeat_until(body, condition, *span),
            Statement::ReadLn { targets, span } => self.compile_readln(targets, *span),
            Statement::Block(block) => self.compile_block(block),
            Statement::IndexAssignment {
                target,
                index,
                expr,
                span,
            } => {
                self.set_debug_loc(*span);
                let idx_val = self.compile_expr(index)?;
                let val = self.compile_expr(expr)?;
                let alloca = *self.lookup_var(target.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{target}'"), Some(*span))
                })?;
                let var_type = self
                    .lookup_var_type(target.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodeGenError::new(format!("unknown type for '{target}'"), Some(*span))
                    })?;
                let resolved = self.resolve_type(&var_type);
                if resolved == PascalType::String {
                    return self.set_string_char(alloca, idx_val, val, *span);
                }
                let (gep, elem_ty) =
                    self.gep_array_elem(alloca, &resolved, idx_val.into_int_value(), *span)?;
                self.store_value(gep, val, &elem_ty, *span)
            }
            Statement::MultiIndexAssignment {
                target,
                indices,
                expr,
                span,
            } => {
                self.set_debug_loc(*span);
                let val = self.compile_expr(expr)?;
                let alloca = *self.lookup_var(target.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{target}'"), Some(*span))
                })?;
                let var_type = self
                    .lookup_var_type(target.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodeGenError::new(format!("unknown type for '{target}'"), Some(*span))
                    })?;

                let mut current_ptr = alloca;
                let mut current_type = var_type;
                for (dim, index_expr) in indices.iter().enumerate() {
                    let idx_val = self.compile_expr(index_expr)?;
                    let (lo, hi, elem_ty) = match &current_type {
                        PascalType::Array { lo, hi, elem, .. } => (*lo, *hi, elem.as_ref().clone()),
                        _ => {
                            return Err(CodeGenError::new(
                                format!("too many indices for '{target}'"),
                                Some(*span),
                            ));
                        }
                    };
                    self.emit_range_check(idx_val.into_int_value(), lo, hi, *span)?;
                    let adj = self
                        .builder
                        .build_int_sub(
                            idx_val.into_int_value(),
                            self.context.i64_type().const_int(lo as u64, true),
                            "adj_idx",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    let gep = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.llvm_type_for(&current_type),
                                current_ptr,
                                &[self.context.i64_type().const_int(0, false), adj],
                                "multi_gep",
                            )
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                    };
                    if dim == indices.len() - 1 {
                        self.store_value(gep, val, &elem_ty, *span)?;
                    } else {
                        current_ptr = gep;
                        current_type = elem_ty;
                    }
                }
                Ok(())
            }
            Statement::FieldAssignment {
                target,
                field,
                expr,
                span,
            } => {
                self.set_debug_loc(*span);
                let val = self.compile_expr(expr)?;
                let alloca = *self.lookup_var(target.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{target}'"), Some(*span))
                })?;
                let var_type = self
                    .lookup_var_type(target.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodeGenError::new(format!("unknown type for '{target}'"), Some(*span))
                    })?;
                let resolved = self.resolve_type(&var_type);
                match &resolved {
                    PascalType::Record {
                        fields, variant, ..
                    } => {
                        // Check fixed fields first
                        if let Some(idx) = fields.iter().position(|(n, _)| n == field) {
                            let gep = self
                                .builder
                                .build_struct_gep(
                                    self.llvm_type_for(&resolved),
                                    alloca,
                                    idx as u32,
                                    "field_gep",
                                )
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                            self.store_value(gep, val, &fields[idx].1, *span)?;
                        } else if let Some(v) = variant {
                            if field == &v.tag_name {
                                // Tag field: index = fields.len()
                                let gep = self
                                    .builder
                                    .build_struct_gep(
                                        self.llvm_type_for(&resolved),
                                        alloca,
                                        fields.len() as u32,
                                        "tag_gep",
                                    )
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                                self.store_value(gep, val, &v.tag_type, *span)?;
                            } else {
                                // Search variant fields
                                let (byte_offset, field_ty) =
                                    self.find_variant_field(v, field).ok_or_else(|| {
                                        CodeGenError::new(
                                            format!("no field '{field}' in record"),
                                            Some(*span),
                                        )
                                    })?;
                                // GEP to union array (index = fields.len() + 1)
                                let union_gep = self
                                    .builder
                                    .build_struct_gep(
                                        self.llvm_type_for(&resolved),
                                        alloca,
                                        (fields.len() + 1) as u32,
                                        "union_gep",
                                    )
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                                // Byte-offset into the union array
                                let byte_gep = unsafe {
                                    self.builder
                                        .build_in_bounds_gep(
                                            self.context.i8_type(),
                                            union_gep,
                                            &[self
                                                .context
                                                .i64_type()
                                                .const_int(byte_offset, false)],
                                            "vfield_ptr",
                                        )
                                        .map_err(|e| {
                                            CodeGenError::new(e.to_string(), Some(*span))
                                        })?
                                };
                                self.store_value(byte_gep, val, &field_ty, *span)?;
                            }
                        } else {
                            return Err(CodeGenError::new(
                                format!("no field '{field}' in record"),
                                Some(*span),
                            ));
                        }
                    }
                    _ => {
                        return Err(CodeGenError::new(
                            format!("'{target}' is not a record"),
                            Some(*span),
                        ));
                    }
                };
                Ok(())
            }
            Statement::New {
                target,
                constructor: Some((ctor, args)),
                span,
            } => {
                self.set_debug_loc(*span);
                let (slot, ptr_ty) = self.resolve_array_base(target, *span)?;
                let obj = self.construct_new(&ptr_ty, ctor, args, *span)?;
                self.builder
                    .build_store(slot, obj)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(())
            }
            Statement::New { target, span, .. } => self.compile_new(target, *span),
            Statement::Dispose {
                target,
                destructor: Some((dtor, args)),
                span,
            } => {
                // Run the destructor (virtually) on p^, then free p.
                self.set_debug_loc(*span);
                let span = *span;
                let deref = Expr::Deref(Box::new(target.clone()), span);
                self.compile_method_call(&deref, dtor, args, span)?;
                self.compile_dispose(target, span)
            }
            Statement::Dispose { target, span, .. } => self.compile_dispose(target, *span),
            Statement::MethodCall {
                object,
                method,
                args,
                span,
            } => {
                self.set_debug_loc(*span);
                self.compile_method_call(object, method, args, *span)?;
                Ok(())
            }
            Statement::Inherited { method, args, span } => {
                self.set_debug_loc(*span);
                self.compile_inherited(method, args, *span)?;
                Ok(())
            }
            Statement::Str {
                value,
                target,
                span,
            } => self.compile_str_statement(value, target, *span),
            Statement::IndirectCall { call, span } => {
                self.set_debug_loc(*span);
                if let Expr::IndirectCall { callee, args, .. } = call {
                    self.compile_indirect_call(callee, args, *span)?;
                } else {
                    self.compile_expr(call)?;
                }
                Ok(())
            }
            // Inside a method, the object's own `Read`, `Inc` and so on hide
            // the built-in ones.
            Statement::ProcCall { name, args, span } if self.self_method(name).is_some() => {
                self.compile_proc_call(name, args, *span)
            }
            Statement::ProcCall { name, args, span } => {
                if name == "inc" && (args.len() == 1 || args.len() == 2) {
                    return self.compile_inc_dec(args, true, *span);
                }
                if name == "dec" && (args.len() == 1 || args.len() == 2) {
                    return self.compile_inc_dec(args, false, *span);
                }
                if name == "delete" && args.len() == 3 {
                    return self.compile_str_delete(args, *span);
                }
                if name == "insert" && args.len() == 3 {
                    return self.compile_str_insert(args, *span);
                }
                if name == "str" && args.len() == 2 {
                    return self.compile_str_proc(args, *span);
                }
                if name == "val" && args.len() == 3 {
                    return self.compile_val(args, *span);
                }
                if name == "include" && args.len() == 2 {
                    return self.compile_set_include_exclude(args, true, *span);
                }
                if name == "exclude" && args.len() == 2 {
                    return self.compile_set_include_exclude(args, false, *span);
                }
                if name == "assign" && args.len() == 2 {
                    return self.compile_file_assign(args, *span);
                }
                // An untyped `file` (or a record size, `Reset(F, 1)`) is
                // opened in binary read/write mode, FPC's default.
                if name == "reset" && (args.len() == 1 || args.len() == 2) {
                    let mode = if args.len() == 2 || self.is_untyped_file(&args[0]) {
                        "r+b"
                    } else {
                        "r"
                    };
                    return self.compile_file_open(&args[0], mode, *span);
                }
                if name == "rewrite" && (args.len() == 1 || args.len() == 2) {
                    let mode = if args.len() == 2 || self.is_untyped_file(&args[0]) {
                        "w+b"
                    } else {
                        "w"
                    };
                    return self.compile_file_open(&args[0], mode, *span);
                }
                if name == "append" && args.len() == 1 {
                    return self.compile_file_open(&args[0], "a", *span);
                }
                if name == "close" && args.len() == 1 {
                    return self.compile_file_close(&args[0], *span);
                }
                if name == "pack" && args.len() == 3 {
                    return self.compile_pack_unpack(args, true, *span);
                }
                if name == "unpack" && args.len() == 3 {
                    return self.compile_pack_unpack(args, false, *span);
                }
                if name == "get" && args.len() == 1 {
                    return self.compile_file_get(&args[0], *span);
                }
                if name == "put" && args.len() == 1 {
                    return self.compile_file_put(&args[0], *span);
                }
                if name == "page" && args.len() == 1 {
                    self.set_debug_loc(*span);
                    let f_name = match &args[0] {
                        Expr::Var(n, _) => n.clone(),
                        _ => {
                            return Err(CodeGenError::new(
                                "page requires a file variable",
                                Some(*span),
                            ));
                        }
                    };
                    let alloca = *self.lookup_var(f_name.as_str()).unwrap();
                    let ptr_ty = self.context.ptr_type(AddressSpace::default());
                    let s = self
                        .builder
                        .build_load(ptr_ty, alloca, "fs")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .into_pointer_value();
                    // Write form-feed (0x0C)
                    let f = self.module.get_function("bruto_file_write_char").unwrap();
                    self.builder
                        .build_call(
                            f,
                            &[
                                s.into(),
                                self.context.i8_type().const_int(0x0C, false).into(),
                            ],
                            "",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(());
                }
                if name == "seek" && args.len() == 2 {
                    self.set_debug_loc(*span);
                    let f_name = match &args[0] {
                        Expr::Var(n, _) => n.clone(),
                        _ => {
                            return Err(CodeGenError::new(
                                "seek requires a file variable",
                                Some(*span),
                            ));
                        }
                    };
                    let alloca = *self.lookup_var(f_name.as_str()).unwrap();
                    let ptr_ty = self.context.ptr_type(AddressSpace::default());
                    let s = self
                        .builder
                        .build_load(ptr_ty, alloca, "fs")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .into_pointer_value();
                    let pos = self.compile_expr(&args[1])?;
                    let f = self.module.get_function("bruto_file_seek").unwrap();
                    self.builder
                        .build_call(f, &[s.into(), pos.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(());
                }
                if name == "read" && !args.is_empty() {
                    return self.compile_file_read(args, *span);
                }
                if name == "write" && !args.is_empty() {
                    return self.compile_file_write_proc(args, false, *span);
                }
                if name == "writeln" && !args.is_empty() {
                    return self.compile_file_write_proc(args, true, *span);
                }
                if name == "readln" {
                    let names: Vec<String> = args
                        .iter()
                        .filter_map(|a| match a {
                            Expr::Var(n, _) => Some(n.clone()),
                            _ => None,
                        })
                        .collect();
                    if names.len() == args.len() {
                        return self.compile_readln(&names, *span);
                    }
                }
                self.compile_proc_call(name, args, *span)
            }
            Statement::Case {
                expr,
                branches,
                else_branch,
                span,
            } => self.compile_case(expr, branches, else_branch.as_deref(), *span),
            Statement::Label { label, span } => {
                self.set_debug_loc(*span);
                let bb = *self.label_blocks.get(label).ok_or_else(|| {
                    CodeGenError::new(format!("undeclared label {label}"), Some(*span))
                })?;
                self.builder
                    .build_unconditional_branch(bb)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                self.builder.position_at_end(bb);
                Ok(())
            }
            Statement::PlaceAssignment { target, expr, span } => {
                self.set_debug_loc(*span);
                let val = self.compile_expr(expr)?;
                if let Expr::Index { array, index, .. } = target
                    && self.is_string_expr(array)
                {
                    let (slot, _) = self.resolve_array_base(array, *span)?;
                    let idx = self.compile_expr(index)?;
                    return self.set_string_char(slot, idx, val, *span);
                }
                let (ptr, ty) = self.resolve_array_base(target, *span)?;
                self.store_value(ptr, val, &ty, *span)
            }
            Statement::Break { span } => {
                self.set_debug_loc(*span);
                let (_, after) = *self.loop_targets.last().ok_or_else(|| {
                    CodeGenError::new("'break' is only allowed inside a loop", Some(*span))
                })?;
                self.jump_to(after, "after_break", *span)
            }
            Statement::Continue { span } => {
                self.set_debug_loc(*span);
                let (next, _) = *self.loop_targets.last().ok_or_else(|| {
                    CodeGenError::new("'continue' is only allowed inside a loop", Some(*span))
                })?;
                self.jump_to(next, "after_continue", *span)
            }
            Statement::Exit { value, span } => {
                if let Some(value) = value {
                    let target = self.current_function.clone().ok_or_else(|| {
                        CodeGenError::new(
                            "'exit' with a value is only allowed in a function",
                            Some(*span),
                        )
                    })?;
                    self.compile_statement(&Statement::Assignment {
                        target,
                        expr: value.clone(),
                        span: *span,
                    })?;
                }
                self.set_debug_loc(*span);
                let exit = self
                    .exit_block
                    .ok_or_else(|| CodeGenError::new("'exit' outside a routine", Some(*span)))?;
                self.jump_to(exit, "after_exit", *span)
            }
            Statement::Goto { label, span } => {
                self.set_debug_loc(*span);
                let bb = *self.label_blocks.get(label).ok_or_else(|| {
                    CodeGenError::new(format!("undeclared label {label}"), Some(*span))
                })?;
                self.builder
                    .build_unconditional_branch(bb)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                let func = self.current_fn.unwrap();
                let dead_bb = self.context.append_basic_block(func, "after_goto");
                self.builder.position_at_end(dead_bb);
                Ok(())
            }
            Statement::With { target, body, span } => self.compile_with(target, body, *span),
            Statement::ChainedAssignment {
                target,
                chain,
                expr,
                span,
            } => self.compile_chained_assignment(target, chain, expr, *span),
        }
    }

    fn compile_new(&mut self, target: &Expr, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let (slot, var_type) = self.resolve_array_base(target, span)?;
        let PascalType::Pointer(pointed) = self.resolve_type(&var_type) else {
            return Err(CodeGenError::new(
                "new() needs a pointer variable",
                Some(span),
            ));
        };
        // LLVM's own size, so records get their padding.
        let size = self
            .llvm_type_for(&pointed)
            .size_of()
            .unwrap_or_else(|| self.context.i64_type().const_int(8, false));
        let bruto_alloc = self.module.get_function("bruto_alloc").unwrap();
        let ptr = self
            .builder
            .build_call(bruto_alloc, &[size.into()], "heap_ptr")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.builder
            .build_store(slot, ptr)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_dispose(&mut self, target: &Expr, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let ptr_val = self.compile_expr(target)?;
        if !ptr_val.is_pointer_value() {
            return Err(CodeGenError::new("dispose() needs a pointer", Some(span)));
        }
        let bruto_free = self.module.get_function("bruto_free").unwrap();
        self.builder
            .build_call(bruto_free, &[ptr_val.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        // Null out a pointer variable (not, say, a cast of one).
        if Self::is_place(target) && self.is_addressable(target) {
            let (slot, _) = self.resolve_array_base(target, span)?;
            let null = self.context.ptr_type(AddressSpace::default()).const_null();
            self.builder
                .build_store(slot, null)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        }
        Ok(())
    }

    fn compile_with(
        &mut self,
        target: &Expr,
        body: &Block,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let (address, ty) = self.resolve_array_base(target, span)?;
        let resolved = self.resolve_type(&ty);
        let PascalType::Record {
            fields,
            variant,
            object,
        } = &resolved
        else {
            return Err(CodeGenError::new(
                "'with' needs a record or object",
                Some(span),
            ));
        };

        // Every field name, including the variant tag and variant fields.
        let mut names: Vec<String> = fields
            .iter()
            .map(|(n, _)| n.clone())
            .filter(|n| n != objects::VMT_FIELD)
            .collect();
        if let Some(v) = variant {
            if !v.tag_name.is_empty() {
                names.push(v.tag_name.clone());
            }
            names.extend(Self::variant_field_names(v));
        }

        // Bind them, remembering what they hide.
        let mut saved: Vec<(String, Option<PointerValue<'ctx>>, Option<PascalType>)> = Vec::new();
        for name in names {
            let (field_ptr, field_ty) = self.field_address(address, &resolved, &name, span)?;
            let old_var = self.variables.insert(name.clone(), field_ptr);
            let old_type = self.var_types.insert(name.clone(), field_ty);
            saved.push((name, old_var, old_type));
        }
        // An object's methods are callable by bare name too.
        if let Some(object) = object {
            self.with_objects.push((object.clone(), address));
        }

        let result = self.compile_block(body);

        if object.is_some() {
            self.with_objects.pop();
        }
        for (name, old_var, old_type) in saved.into_iter().rev() {
            match old_var {
                Some(v) => self.variables.insert(name.clone(), v),
                None => self.variables.remove(&name),
            };
            match old_type {
                Some(t) => self.var_types.insert(name, t),
                None => self.var_types.remove(&name),
            };
        }
        result
    }

    fn compile_chained_assignment(
        &mut self,
        target: &str,
        chain: &[LValueAccess],
        expr: &Expr,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let val = self.compile_expr(expr)?;

        let mut ptr = *self.lookup_var(target).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{target}'"), Some(span))
        })?;
        let mut cur_type = self
            .lookup_var_type(target)
            .cloned()
            .ok_or_else(|| CodeGenError::new(format!("unknown type for '{target}'"), Some(span)))?;
        cur_type = self.resolve_type(&cur_type);

        for (i, access) in chain.iter().enumerate() {
            let is_last = i == chain.len() - 1;
            match access {
                LValueAccess::Field(field) => {
                    let (gep, field_ty) = self.field_address(ptr, &cur_type, field, span)?;
                    if is_last {
                        self.store_value(gep, val, &field_ty, span)?;
                        return Ok(());
                    }
                    ptr = gep;
                    cur_type = self.resolve_type(&field_ty);
                }
                LValueAccess::Index(index_expr) => {
                    let idx_val = self.compile_expr(index_expr)?;
                    if is_last && self.resolve_type(&cur_type) == PascalType::String {
                        return self.set_string_char(ptr, idx_val, val, span);
                    }
                    let idx_val = self.first_char_if_string(idx_val, span)?;
                    let resolved = self.resolve_type(&cur_type);
                    let (gep, elem_ty) =
                        self.gep_array_elem(ptr, &resolved, idx_val.into_int_value(), span)?;
                    if is_last {
                        self.store_value(gep, val, &elem_ty, span)?;
                        return Ok(());
                    }
                    ptr = gep;
                    cur_type = self.resolve_type(&elem_ty);
                }
                LValueAccess::Deref => {
                    let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
                    let loaded = self
                        .builder
                        .build_load(ptr_ty, ptr, "deref_ptr")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .into_pointer_value();
                    let pointed = match &cur_type {
                        PascalType::Pointer(inner) => inner.as_ref().clone(),
                        _ => {
                            return Err(CodeGenError::new(
                                "dereference of non-pointer",
                                Some(span),
                            ));
                        }
                    };
                    if is_last {
                        self.store_value(loaded, val, &pointed, span)?;
                        return Ok(());
                    }
                    ptr = loaded;
                    cur_type = self.resolve_type(&pointed);
                }
            }
        }
        Ok(())
    }

    fn compile_case(
        &mut self,
        expr: &Expr,
        branches: &[CaseBranch],
        else_branch: Option<&[Statement]>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let sel_val = self.case_label(expr, span)?;
        let func = self.current_fn.unwrap();
        let end_bb = self.context.append_basic_block(func, "case_end");

        for branch in branches {
            let match_bb = self.context.append_basic_block(func, "case_match");
            let next_bb = self.context.append_basic_block(func, "case_next");

            let mut any_match: Option<inkwell::values::IntValue> = None;
            for val in &branch.values {
                let cmp = match val {
                    CaseValue::Single(v) => {
                        let v_val = self.case_label(v, span)?;
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                sel_val.into_int_value(),
                                v_val.into_int_value(),
                                "case_eq",
                            )
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                    CaseValue::Range(lo, hi) => {
                        let lo_val = self.case_label(lo, span)?;
                        let hi_val = self.case_label(hi, span)?;
                        let ge = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::SGE,
                                sel_val.into_int_value(),
                                lo_val.into_int_value(),
                                "case_ge",
                            )
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                        let le = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::SLE,
                                sel_val.into_int_value(),
                                hi_val.into_int_value(),
                                "case_le",
                            )
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                        self.builder
                            .build_and(ge, le, "case_range")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                };
                any_match = Some(match any_match {
                    None => cmp,
                    Some(prev) => self
                        .builder
                        .build_or(prev, cmp, "case_or")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                });
            }

            self.builder
                .build_conditional_branch(any_match.unwrap(), match_bb, next_bb)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

            self.builder.position_at_end(match_bb);
            for stmt in &branch.body {
                self.compile_statement(stmt)?;
            }
            self.builder
                .build_unconditional_branch(end_bb)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

            self.builder.position_at_end(next_bb);
        }

        if let Some(stmts) = else_branch {
            for stmt in stmts {
                self.compile_statement(stmt)?;
            }
        }
        self.builder
            .build_unconditional_branch(end_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        self.builder.position_at_end(end_bb);
        Ok(())
    }

    fn sizeof_type(&self, ty: &PascalType) -> u64 {
        match ty {
            // Object declarations are lowered to records when registered.
            PascalType::Object(_) => 0,
            PascalType::ShortString { max } => *max as u64 + 1,
            PascalType::Integer
            | PascalType::Enum { .. }
            | PascalType::Subrange { .. }
            | PascalType::SubrangeExpr { .. } => 8,
            PascalType::SizedInt { bits, .. } => (*bits / 8) as u64,
            PascalType::Untyped => 1,
            PascalType::Real => 8,
            PascalType::Boolean => 1,
            PascalType::Char => 1,
            PascalType::String | PascalType::Pointer(_) => 8,
            PascalType::File { .. } => 8, // pointer to opaque struct
            PascalType::Proc { .. } => 8, // function pointer
            PascalType::ConformantArray { .. } => 8, // base pointer
            PascalType::Set { .. } => 32, // 4 x i64 = 256 bits
            PascalType::Array { lo, hi, elem } => {
                let count = (hi - lo + 1).max(0) as u64;
                count * self.sizeof_type(elem)
            }
            PascalType::Record {
                fields, variant, ..
            } => {
                let fixed: u64 = fields.iter().map(|(_, t)| self.sizeof_type(t)).sum();
                let var_size = variant
                    .as_ref()
                    .map(|v| {
                        let tag_size = self.sizeof_type(&v.tag_type);
                        let max_body = v
                            .variants
                            .iter()
                            .map(|(_, vf)| vf.iter().map(|(_, t)| self.sizeof_type(t)).sum::<u64>())
                            .max()
                            .unwrap_or(0);
                        tag_size + max_body
                    })
                    .unwrap_or(0);
                fixed + var_size
            }
            PascalType::Named(_) => {
                let resolved = self.resolve_type(ty);
                self.sizeof_type(&resolved)
            }
            // Folded by the semantic pass before code generation.
            PascalType::ArrayExpr { .. } => 0,
        }
    }

    /// Find a field in the variant part of a record. Returns (byte_offset, field_type).
    /// Each variant's fields are laid out at byte offset 0 within the union (they overlap).
    /// Within a single variant, fields are sequential.
    fn find_variant_field(&self, v: &RecordVariant, field: &str) -> Option<(u64, PascalType)> {
        for (_values, vfields) in &v.variants {
            let mut offset = 0u64;
            for (name, ty) in vfields {
                if name == field {
                    return Some((offset, ty.clone()));
                }
                // A nested variant part: an unnamed record laid out as its
                // tag followed by its own union.
                if name.is_empty()
                    && let PascalType::Record {
                        variant: Some(nested),
                        ..
                    } = ty
                    && let Some((inner, inner_ty)) = self.find_variant_field(nested, field)
                {
                    let tag = self.sizeof_type(&nested.tag_type);
                    return Some((offset + tag + inner, inner_ty));
                }
                offset += self.sizeof_type(ty);
            }
        }
        None
    }

    /// The names of all fields in a variant part, nested ones included.
    fn variant_field_names(v: &RecordVariant) -> Vec<String> {
        let mut names = Vec::new();
        for (_, vfields) in &v.variants {
            for (name, ty) in vfields {
                if !name.is_empty() {
                    names.push(name.clone());
                } else if let PascalType::Record {
                    variant: Some(nested),
                    ..
                } = ty
                {
                    names.extend(Self::variant_field_names(nested));
                }
            }
        }
        names
    }

    fn compile_if(
        &mut self,
        condition: &Expr,
        then_branch: &Block,
        else_branch: Option<&Block>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let cond_val = self.compile_expr(condition)?;

        // Ensure condition is an i1 (bool)
        let cond_bool = if cond_val.is_int_value() {
            let iv = cond_val.into_int_value();
            if iv.get_type().get_bit_width() == 1 {
                iv
            } else {
                self.builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        iv,
                        iv.get_type().const_int(0, false),
                        "tobool",
                    )
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            }
        } else {
            return Err(CodeGenError::new(
                "condition must be boolean or integer",
                Some(span),
            ));
        };

        let func = self.current_fn.unwrap();
        let then_bb = self.context.append_basic_block(func, "then");
        let else_bb = self.context.append_basic_block(func, "else");
        let merge_bb = self.context.append_basic_block(func, "ifcont");

        self.builder
            .build_conditional_branch(cond_bool, then_bb, else_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Then branch
        self.builder.position_at_end(then_bb);
        self.compile_block(then_branch)?;
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Else branch
        self.builder.position_at_end(else_bb);
        if let Some(else_block) = else_branch {
            self.compile_block(else_block)?;
        }
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        self.builder.position_at_end(merge_bb);
        Ok(())
    }

    fn compile_while(
        &mut self,
        condition: &Expr,
        body: &Block,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let func = self.current_fn.unwrap();
        let cond_bb = self.context.append_basic_block(func, "whilecond");
        let body_bb = self.context.append_basic_block(func, "whilebody");
        let after_bb = self.context.append_basic_block(func, "whileend");

        self.builder
            .build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Condition
        self.builder.position_at_end(cond_bb);
        self.set_debug_loc(span);
        let cond_val = self.compile_expr(condition)?;

        let cond_bool = if cond_val.is_int_value() {
            let iv = cond_val.into_int_value();
            if iv.get_type().get_bit_width() == 1 {
                iv
            } else {
                self.builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        iv,
                        iv.get_type().const_int(0, false),
                        "tobool",
                    )
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            }
        } else {
            return Err(CodeGenError::new(
                "while condition must be boolean or integer",
                Some(span),
            ));
        };

        self.builder
            .build_conditional_branch(cond_bool, body_bb, after_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Body
        self.builder.position_at_end(body_bb);
        self.loop_targets.push((cond_bb, after_bb));
        self.compile_block(body)?;
        self.loop_targets.pop();
        self.builder
            .build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        self.builder.position_at_end(after_bb);
        Ok(())
    }

    fn compile_for(
        &mut self,
        var: &str,
        from: &Expr,
        to: &Expr,
        downto: bool,
        body: &Block,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        self.set_debug_loc(span);
        let func = self.current_fn.unwrap();
        let alloca = *self
            .lookup_var(var)
            .ok_or_else(|| CodeGenError::new(format!("undefined variable '{var}'"), Some(span)))?;
        let var_ty = self
            .lookup_var_type(var)
            .cloned()
            .unwrap_or(PascalType::Integer);
        let var_llvm_ty = self.llvm_type_for(&var_ty);

        // Both bounds are evaluated once, before the first iteration. The
        // loop variable is compared in 64 bits.
        let from_val = self.compile_expr(from)?;
        let to_val = self.compile_expr(to)?;
        self.store_value(alloca, from_val, &var_ty, span)?;
        let to_val = self.to_i64(to_val, span)?;
        let load_cur = |cg: &mut Self, name: &str| -> Result<_, CodeGenError> {
            let raw = cg
                .builder
                .build_load(var_llvm_ty, alloca, name)
                .map_err(err)?;
            let wide = cg.widen_loaded(raw, &var_ty, span)?;
            cg.to_i64(wide, span)
        };

        let body_bb = self.context.append_basic_block(func, "forbody");
        let step_bb = self.context.append_basic_block(func, "forstep");
        let after_bb = self.context.append_basic_block(func, "forend");

        // Skip the loop entirely when the range is empty.
        let first = load_cur(self, "for_first")?;
        let enter = self
            .builder
            .build_int_compare(
                if downto {
                    inkwell::IntPredicate::SGE
                } else {
                    inkwell::IntPredicate::SLE
                },
                first,
                to_val,
                "for_enter",
            )
            .map_err(err)?;
        self.builder
            .build_conditional_branch(enter, body_bb, after_bb)
            .map_err(err)?;

        // Body. `continue` goes to the step block.
        self.builder.position_at_end(body_bb);
        self.loop_targets.push((step_bb, after_bb));
        self.compile_block(body)?;
        self.loop_targets.pop();
        self.builder
            .build_unconditional_branch(step_bb)
            .map_err(err)?;

        // Step: stop once the variable has reached the final value, so a
        // `byte` loop up to 255 (or an integer loop up to maxint) ends
        // instead of wrapping around.
        let _ = step_bb.move_after(self.builder.get_insert_block().unwrap());
        let _ = after_bb.move_after(step_bb);
        self.builder.position_at_end(step_bb);
        self.set_debug_loc(span);
        let cur = load_cur(self, "for_cur")?;
        let done = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, cur, to_val, "for_done")
            .map_err(err)?;
        let next_bb = self.context.append_basic_block(func, "fornext");
        let _ = next_bb.move_after(step_bb);
        self.builder
            .build_conditional_branch(done, after_bb, next_bb)
            .map_err(err)?;
        self.builder.position_at_end(next_bb);
        let one = self.context.i64_type().const_int(1, false);
        let next = if downto {
            self.builder.build_int_sub(cur, one, "dec").map_err(err)?
        } else {
            self.builder.build_int_add(cur, one, "inc").map_err(err)?
        };
        self.store_value(alloca, next.into(), &var_ty, span)?;
        self.builder
            .build_unconditional_branch(body_bb)
            .map_err(err)?;

        self.builder.position_at_end(after_bb);
        Ok(())
    }

    /// A case label as a 64-bit ordinal; a one-character string literal
    /// is that character.
    fn case_label(
        &mut self,
        label: &Expr,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        if let Expr::StrLit(s, _) = label
            && s.chars().count() == 1
        {
            let code = s.chars().next().unwrap() as u64;
            return Ok(self.context.i64_type().const_int(code, false).into());
        }
        let val = self.compile_expr(label)?;
        Ok(self.to_i64(val, span)?.into())
    }

    /// Extend an integer value (boolean, char or narrower integer) to
    /// 64 bits, zero-extending since those are all unsigned.
    fn to_i64(
        &self,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodeGenError> {
        let iv = val.into_int_value();
        if iv.get_type().get_bit_width() >= 64 {
            return Ok(iv);
        }
        self.builder
            .build_int_z_extend(iv, self.context.i64_type(), "to_i64")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))
    }

    fn compile_repeat_until(
        &mut self,
        body: &[Statement],
        condition: &Expr,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let func = self.current_fn.unwrap();
        let body_bb = self.context.append_basic_block(func, "repeat");
        let after_bb = self.context.append_basic_block(func, "repeatend");

        self.builder
            .build_unconditional_branch(body_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // `continue` jumps to the condition check.
        let cond_bb = self.context.append_basic_block(func, "repeatcond");
        self.builder.position_at_end(body_bb);
        self.loop_targets.push((cond_bb, after_bb));
        for stmt in body {
            self.compile_statement(stmt)?;
        }
        self.loop_targets.pop();
        self.builder
            .build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let _ = cond_bb.move_after(self.builder.get_insert_block().unwrap());
        self.builder.position_at_end(cond_bb);

        // Check condition — repeat exits when condition is true
        self.set_debug_loc(span);
        let cond_val = self.compile_expr(condition)?;
        let cond_bool = if cond_val.is_int_value() {
            let iv = cond_val.into_int_value();
            if iv.get_type().get_bit_width() == 1 {
                iv
            } else {
                self.builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        iv,
                        iv.get_type().const_int(0, false),
                        "tobool",
                    )
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            }
        } else {
            return Err(CodeGenError::new(
                "repeat-until condition must be boolean or integer",
                Some(span),
            ));
        };
        // If condition true, exit; if false, loop back
        self.builder
            .build_conditional_branch(cond_bool, after_bb, body_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        self.builder.position_at_end(after_bb);
        Ok(())
    }

    fn compile_proc_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let resolved_name = self.resolve_overload(name, args);
        let name = resolved_name.as_str();
        // Inside a method, the object's methods hide routines of the same
        // name.
        if let Some((object, self_ptr)) = self
            .implicit_method(name)
            .or_else(|| self.recursive_method(name))
        {
            self.call_method(
                objects::Receiver {
                    object: &object,
                    self_ptr,
                    dispatch: objects::Dispatch::Virtual,
                    vmt: None,
                },
                name,
                args,
                span,
            )?;
            return Ok(());
        }
        // `Fail` in a constructor: give up and return nil.
        if name == "fail"
            && args.is_empty()
            && self.method_context.is_some_and(|c| c.in_constructor)
        {
            let err =
                |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
            self.emit_prof_exit()?;
            let null = self.context.ptr_type(AddressSpace::default()).const_null();
            self.builder.build_return(Some(&null)).map_err(err)?;
            let after = self
                .context
                .append_basic_block(self.current_fn.unwrap(), "after_fail");
            self.builder.position_at_end(after);
            return Ok(());
        }
        // `Abstract`: the body of a method that must be overridden.
        if name == "abstract" && args.is_empty() && self.module.get_function(name).is_none() {
            let f = self.abstract_error_fn();
            self.builder
                .build_call(f, &[], "")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            return Ok(());
        }
        // A call through a procedural variable or parameter.
        if let Some(proc_ty) = self.procedural_var_type(name) {
            let slot = *self.lookup_var(name).unwrap();
            let target = self
                .builder
                .build_load(self.context.ptr_type(AddressSpace::default()), slot, "proc")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                .into_pointer_value();
            self.call_procedural(target, &proc_ty, args, span)?;
            return Ok(());
        }
        if self.module.get_function(name).is_none() && self.compile_system_proc(name, args, span)? {
            return Ok(());
        }
        let func = self.module.get_function(name).ok_or_else(|| {
            CodeGenError::new(format!("undefined procedure '{name}'"), Some(span))
        })?;
        let call_args = self.compile_call_args(name, args, span)?;
        self.builder
            .build_call(func, &call_args, "")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_inc_dec(
        &mut self,
        args: &[Expr],
        is_inc: bool,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        if !self.is_addressable(&args[0]) {
            return Err(CodeGenError::new("inc/dec requires a variable", Some(span)));
        }
        let (alloca, var_type) = self.resolve_array_base(&args[0], span)?;
        // A pointer steps by whole elements (bytes for an untyped one).
        if let PascalType::Pointer(pointed) = self.resolve_type(&var_type) {
            let err =
                |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
            let ptr_ty = self.context.ptr_type(AddressSpace::default());
            let elem = if *pointed == PascalType::Untyped {
                self.context.i8_type().as_basic_type_enum()
            } else {
                self.llvm_type_for(&self.resolve_type(&pointed))
            };
            let mut step = match args.get(1) {
                Some(n) => {
                    let n = self.compile_expr(n)?;
                    self.to_i64(n, span)?
                }
                None => self.context.i64_type().const_int(1, false),
            };
            if !is_inc {
                step = self.builder.build_int_neg(step, "neg").map_err(err)?;
            }
            let current = self
                .builder
                .build_load(ptr_ty, alloca, "p")
                .map_err(err)?
                .into_pointer_value();
            let moved = unsafe {
                self.builder
                    .build_gep(elem, current, &[step], "p_step")
                    .map_err(err)?
            };
            self.builder.build_store(alloca, moved).map_err(err)?;
            return Ok(());
        }
        let llvm_ty = self.llvm_type_for(&var_type);

        // Work in 64 bits whatever the variable's width (chars and narrow
        // integers included); `store_value` narrows the result again.
        let i64_ty = self.context.i64_type();
        let loaded = self
            .builder
            .build_load(llvm_ty, alloca, "cur")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let cur = self.widen_loaded(loaded, &var_type, span)?.into_int_value();
        let cur = if cur.get_type().get_bit_width() < 64 {
            self.builder
                .build_int_z_extend(cur, i64_ty, "cur64")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        } else {
            cur
        };

        let step = if args.len() == 2 {
            let step = self.compile_expr(&args[1])?.into_int_value();
            if step.get_type().get_bit_width() < 64 {
                self.builder
                    .build_int_z_extend(step, i64_ty, "step64")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            } else {
                step
            }
        } else {
            i64_ty.const_int(1, false)
        };

        let result = if is_inc {
            self.builder
                .build_int_add(cur, step, "inc")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        } else {
            self.builder
                .build_int_sub(cur, step, "dec")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        };

        self.store_value(alloca, result.into(), &var_type, span)
    }

    /// `delete(s, index, count)`: rebuild the string without the range
    /// and store it back (into a dynamic or a short string).
    fn compile_str_delete(&mut self, args: &[Expr], span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        if !self.is_addressable(&args[0]) {
            return Err(CodeGenError::new(
                "delete requires a string variable",
                Some(span),
            ));
        }
        let (slot, ty) = self.resolve_array_base(&args[0], span)?;
        let s_val = self.compile_expr(&args[0])?;
        let idx = self.compile_expr(&args[1])?;
        let idx = self.to_i64(idx, span)?;
        let cnt = self.compile_expr(&args[2])?;
        let cnt = self.to_i64(cnt, span)?;
        let f = self.module.get_function("bruto_str_delete").unwrap();
        let result = self
            .builder
            .build_call(f, &[s_val.into(), idx.into(), cnt.into()], "deleted")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.store_value(slot, result, &ty, span)
    }

    /// `insert(source, s, index)`: rebuild the string with `source` inserted
    /// and store it back.
    fn compile_str_insert(&mut self, args: &[Expr], span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let source = self.compile_expr(&args[0])?;
        let source = if source.is_int_value() {
            // A single character: make it a one-character string.
            self.compile_expr(&Expr::Call {
                name: "string".to_string(),
                args: vec![args[0].clone()],
                span,
            })?
        } else {
            source
        };
        if !self.is_addressable(&args[1]) {
            return Err(CodeGenError::new(
                "insert requires a string variable as 2nd arg",
                Some(span),
            ));
        }
        let (slot, ty) = self.resolve_array_base(&args[1], span)?;
        let s_val = self.compile_expr(&args[1])?;
        let idx = self.compile_expr(&args[2])?;
        let idx = self.to_i64(idx, span)?;
        let f = self.module.get_function("bruto_str_insert").unwrap();
        let result = self
            .builder
            .build_call(f, &[source.into(), s_val.into(), idx.into()], "inserted")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.store_value(slot, result, &ty, span)
    }

    fn compile_str_proc(&mut self, args: &[Expr], span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let val = self.compile_expr(&args[0])?;
        let val = if val.is_float_value() {
            val
        } else {
            self.to_i64(val, span)?.into()
        };
        let (alloca, target_ty) = self.resolve_array_base(&args[1], span)?;
        let fn_name = if val.is_float_value() {
            "bruto_real_to_str"
        } else {
            "bruto_int_to_str"
        };
        let f = self.module.get_function(fn_name).unwrap();
        let result = self
            .builder
            .build_call(f, &[val.into()], "str_result")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.store_value(alloca, result, &target_ty, span)
    }

    /// Call the procedural value `callee` computes (`TFunc(p)(args)`),
    /// using its procedural type's signature.
    fn compile_indirect_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let proc_ty = self.infer_declared_type(callee);
        let target = self.compile_expr(callee)?;
        if !target.is_pointer_value() {
            return Err(CodeGenError::new(
                "this isn't a procedural value",
                Some(span),
            ));
        }
        self.call_procedural(target.into_pointer_value(), &proc_ty, args, span)
    }

    /// The procedural type of variable `name`, if it is one.
    fn procedural_var_type(&self, name: &str) -> Option<PascalType> {
        let ty = self.resolve_type(self.lookup_var_type(name)?);
        matches!(ty, PascalType::Proc { .. }).then_some(ty)
    }

    /// `str(value[:width[:precision]], target)`. A precision formats the
    /// value as a fixed-point real; a width pads it on the left.
    fn compile_str_statement(
        &mut self,
        value: &WriteArg,
        target: &Expr,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        self.set_debug_loc(span);
        let (slot, target_ty) = self.resolve_array_base(target, span)?;
        let val = self.compile_expr(&value.expr)?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();
        let snprintf = self.module.get_function("snprintf").unwrap_or_else(|| {
            self.module.add_function(
                "snprintf",
                i32_ty.fn_type(&[ptr.into(), i64_ty.into(), ptr.into()], true),
                None,
            )
        });
        let alloc = self.module.get_function("bruto_alloc").unwrap();
        let buffer = |cg: &Self| -> Result<PointerValue<'ctx>, CodeGenError> {
            Ok(cg
                .builder
                .build_call(alloc, &[i64_ty.const_int(512, false).into()], "str_buf")
                .map_err(err)?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_pointer_value())
        };

        let mut text = if let Some(precision) = &value.precision {
            let real = self.promote_to_f64(val, span)?;
            let digits = self.compile_expr(precision)?;
            let digits = self
                .builder
                .build_int_truncate(self.to_i64(digits, span)?, i32_ty, "prec")
                .map_err(err)?;
            let out = buffer(self)?;
            let fmt = self.string_literal("%.*f");
            self.builder
                .build_call(
                    snprintf,
                    &[
                        out.into(),
                        i64_ty.const_int(512, false).into(),
                        fmt.into(),
                        digits.into(),
                        real.into(),
                    ],
                    "",
                )
                .map_err(err)?;
            out
        } else {
            let fn_name = if val.is_float_value() {
                "bruto_real_to_str"
            } else {
                "bruto_int_to_str"
            };
            let arg = if val.is_float_value() {
                val
            } else {
                self.to_i64(val, span)?.into()
            };
            let f = self.module.get_function(fn_name).unwrap();
            self.builder
                .build_call(f, &[arg.into()], "str_result")
                .map_err(err)?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_pointer_value()
        };
        if let Some(width) = &value.width {
            let width = self.compile_expr(width)?;
            let width = self
                .builder
                .build_int_truncate(self.to_i64(width, span)?, i32_ty, "width")
                .map_err(err)?;
            let out = buffer(self)?;
            let fmt = self.string_literal("%*s");
            self.builder
                .build_call(
                    snprintf,
                    &[
                        out.into(),
                        i64_ty.const_int(512, false).into(),
                        fmt.into(),
                        width.into(),
                        text.into(),
                    ],
                    "",
                )
                .map_err(err)?;
            text = out;
        }
        self.store_value(slot, text.into(), &target_ty, span)
    }

    fn compile_val(&mut self, args: &[Expr], span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let s_val = self.compile_expr(&args[0])?;
        let x_name = match &args[1] {
            Expr::Var(name, _) => name.clone(),
            _ => {
                return Err(CodeGenError::new(
                    "val requires a variable as 2nd arg",
                    Some(span),
                ));
            }
        };
        let alloca = *self.lookup_var(x_name.as_str()).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{x_name}'"), Some(span))
        })?;
        let f = self.module.get_function("bruto_str_to_int").unwrap();
        let result = self
            .builder
            .build_call(f, &[s_val.into()], "val_result")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.builder
            .build_store(alloca, result)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        // Set code to 0 (success)
        let code_name = match &args[2] {
            Expr::Var(name, _) => name.clone(),
            _ => {
                return Err(CodeGenError::new(
                    "val requires a variable as 3rd arg",
                    Some(span),
                ));
            }
        };
        let code_alloca = *self.lookup_var(code_name.as_str()).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{code_name}'"), Some(span))
        })?;
        self.builder
            .build_store(code_alloca, self.context.i64_type().const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_file_assign(&mut self, args: &[Expr], span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let f_name = match &args[0] {
            Expr::Var(name, _) => name.clone(),
            _ => {
                return Err(CodeGenError::new(
                    "assign requires a file variable",
                    Some(span),
                ));
            }
        };
        let alloca = *self.lookup_var(f_name.as_str()).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{f_name}'"), Some(span))
        })?;
        let path_val = self.compile_expr(&args[1])?;
        let new_fn = self.module.get_function("bruto_file_new").unwrap();
        let new_struct = self
            .builder
            .build_call(new_fn, &[path_val.into()], "fnew")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.builder
            .build_store(alloca, new_struct)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    /// Whether `expr` is an untyped `file` variable.
    fn is_untyped_file(&self, expr: &Expr) -> bool {
        matches!(
            self.resolve_type(&self.infer_declared_type(expr)),
            PascalType::File { elem } if *elem == PascalType::Untyped
        )
    }

    fn compile_file_open(
        &mut self,
        arg: &Expr,
        mode: &str,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let name = match arg {
            Expr::Var(n, _) => n.clone(),
            _ => return Err(CodeGenError::new("file op requires a variable", Some(span))),
        };
        let alloca = *self
            .lookup_var(name.as_str())
            .ok_or_else(|| CodeGenError::new(format!("undefined variable '{name}'"), Some(span)))?;
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let s = self
            .builder
            .build_load(ptr_ty, alloca, "fs")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .into_pointer_value();
        let mode_str = self
            .builder
            .build_global_string_ptr(mode, "mode")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let f = self.module.get_function("bruto_file_open").unwrap();
        self.builder
            .build_call(f, &[s.into(), mode_str.as_pointer_value().into()], "")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_file_close(&mut self, arg: &Expr, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let name = match arg {
            Expr::Var(n, _) => n.clone(),
            _ => return Err(CodeGenError::new("close requires a variable", Some(span))),
        };
        let alloca = *self
            .lookup_var(name.as_str())
            .ok_or_else(|| CodeGenError::new(format!("undefined variable '{name}'"), Some(span)))?;
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let s = self
            .builder
            .build_load(ptr_ty, alloca, "fs")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .into_pointer_value();
        let f = self.module.get_function("bruto_file_close").unwrap();
        self.builder
            .build_call(f, &[s.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_file_read(&mut self, args: &[Expr], span: Span) -> Result<(), CodeGenError> {
        // First arg is the file; remaining args are targets.
        let mut names: Vec<String> = Vec::new();
        for a in args {
            match a {
                Expr::Var(n, _) => names.push(n.clone()),
                _ => return Err(CodeGenError::new("read requires variables", Some(span))),
            }
        }
        // Forward to compile_readln (it auto-detects file).
        self.compile_readln(&names, span)
    }

    fn compile_file_write_proc(
        &mut self,
        args: &[Expr],
        newline: bool,
        span: Span,
    ) -> Result<(), CodeGenError> {
        // Wrap each Expr in a WriteArg with no formatting and forward.
        let wargs: Vec<WriteArg> = args
            .iter()
            .map(|e| WriteArg {
                expr: e.clone(),
                width: None,
                precision: None,
            })
            .collect();
        self.compile_write(&wargs, newline, span)
    }

    /// Compile `pack(src, i, dst)` (true) or `unpack(z, dst, i)` (false).
    /// pack: copies from src[i..i+n-1] into dst[low..hi].
    /// unpack: copies from src[low..hi] into dst[i..i+n-1].
    fn compile_file_get(&mut self, arg: &Expr, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let name = match arg {
            Expr::Var(n, _) => n.clone(),
            _ => {
                return Err(CodeGenError::new(
                    "get requires a file variable",
                    Some(span),
                ));
            }
        };
        let alloca = *self.lookup_var(name.as_str()).unwrap();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let s = self
            .builder
            .build_load(ptr_ty, alloca, "fs")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .into_pointer_value();
        let f = self.module.get_function("bruto_file_get").unwrap();
        self.builder
            .build_call(f, &[s.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_file_put(&mut self, arg: &Expr, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let name = match arg {
            Expr::Var(n, _) => n.clone(),
            _ => {
                return Err(CodeGenError::new(
                    "put requires a file variable",
                    Some(span),
                ));
            }
        };
        let alloca = *self.lookup_var(name.as_str()).unwrap();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let s = self
            .builder
            .build_load(ptr_ty, alloca, "fs")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .into_pointer_value();
        let f = self.module.get_function("bruto_file_put").unwrap();
        self.builder
            .build_call(f, &[s.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_pack_unpack(
        &mut self,
        args: &[Expr],
        is_pack: bool,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let (src_idx, dst_idx, idx_arg) = if is_pack {
            (0usize, 2usize, &args[1])
        } else {
            (0usize, 1usize, &args[2])
        };
        let src_name = match &args[src_idx] {
            Expr::Var(n, _) => n.clone(),
            _ => {
                return Err(CodeGenError::new(
                    "pack/unpack expects array variables",
                    Some(span),
                ));
            }
        };
        let dst_name = match &args[dst_idx] {
            Expr::Var(n, _) => n.clone(),
            _ => {
                return Err(CodeGenError::new(
                    "pack/unpack expects array variables",
                    Some(span),
                ));
            }
        };
        let src_ptr = *self.lookup_var(src_name.as_str()).unwrap();
        let dst_ptr = *self.lookup_var(dst_name.as_str()).unwrap();
        let src_ty = self.lookup_var_type(src_name.as_str()).cloned().unwrap();
        let dst_ty = self.lookup_var_type(dst_name.as_str()).cloned().unwrap();
        let src_resolved = self.resolve_type(&src_ty);
        let dst_resolved = self.resolve_type(&dst_ty);

        let (src_lo, src_hi, src_elem) = match &src_resolved {
            PascalType::Array { lo, hi, elem } => (*lo, *hi, elem.as_ref().clone()),
            _ => {
                return Err(CodeGenError::new(
                    "pack/unpack source not an array",
                    Some(span),
                ));
            }
        };
        let (dst_lo, dst_hi, _dst_elem) = match &dst_resolved {
            PascalType::Array { lo, hi, elem } => (*lo, *hi, elem.as_ref().clone()),
            _ => {
                return Err(CodeGenError::new(
                    "pack/unpack dest not an array",
                    Some(span),
                ));
            }
        };
        let _ = (src_lo, src_hi);

        let elem_size = self.sizeof_type(&src_elem);
        let count = (dst_hi - dst_lo + 1).max(0) as u64;
        let total_bytes = count * elem_size;

        // Compute starting index argument (i)
        let i_val = self.compile_expr(idx_arg)?.into_int_value();
        let i64_ty = self.context.i64_type();
        let i_val = if i_val.get_type().get_bit_width() < 64 {
            self.builder
                .build_int_s_extend(i_val, i64_ty, "ix")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        } else {
            i_val
        };

        // For pack: copy src[i .. i+count-1] -> dst[0 .. count-1].
        // For unpack: copy src[0 .. count-1] -> dst[i .. i+count-1].
        let memcpy = self.module.get_function("memcpy").unwrap();
        if is_pack {
            // src offset = (i - src_lo) * elem_size
            let src_lo_v = i64_ty.const_int(src_lo as u64, true);
            let off_idx = self
                .builder
                .build_int_sub(i_val, src_lo_v, "soff")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let off = self
                .builder
                .build_int_mul(off_idx, i64_ty.const_int(elem_size, false), "soffb")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let src_gep = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), src_ptr, &[off], "sgep")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            };
            self.builder
                .build_call(
                    memcpy,
                    &[
                        dst_ptr.into(),
                        src_gep.into(),
                        i64_ty.const_int(total_bytes, false).into(),
                    ],
                    "",
                )
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        } else {
            // dst offset = (i - dst_lo) * elem_size; src is at offset 0
            let dst_lo_v = i64_ty.const_int(dst_lo as u64, true);
            let off_idx = self
                .builder
                .build_int_sub(i_val, dst_lo_v, "doff")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let off = self
                .builder
                .build_int_mul(off_idx, i64_ty.const_int(elem_size, false), "doffb")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let dst_gep = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), dst_ptr, &[off], "dgep")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            };
            // count = src array length
            let (_slo, _shi, _) = match &src_resolved {
                PascalType::Array { lo, hi, elem } => (*lo, *hi, elem),
                _ => unreachable!(),
            };
            let src_count = (_shi - _slo + 1).max(0) as u64;
            let src_total = src_count * elem_size;
            self.builder
                .build_call(
                    memcpy,
                    &[
                        dst_gep.into(),
                        src_ptr.into(),
                        i64_ty.const_int(src_total, false).into(),
                    ],
                    "",
                )
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        }
        Ok(())
    }

    fn compile_set_include_exclude(
        &mut self,
        args: &[Expr],
        is_include: bool,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let s_name = match &args[0] {
            Expr::Var(name, _) => name.clone(),
            _ => {
                return Err(CodeGenError::new(
                    "include/exclude requires a set variable",
                    Some(span),
                ));
            }
        };
        let alloca = *self.lookup_var(s_name.as_str()).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{s_name}'"), Some(span))
        })?;
        let elem_val = self.compile_expr(&args[1])?.into_int_value();

        let i64_ty = self.context.i64_type();
        let set_ty = i64_ty.array_type(4);
        let elem_val = if elem_val.get_type().get_bit_width() < 64 {
            self.builder
                .build_int_z_extend(elem_val, i64_ty, "elem_ext")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        } else {
            elem_val
        };
        let word_idx = self
            .builder
            .build_int_unsigned_div(elem_val, i64_ty.const_int(64, false), "word_idx")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let bit_idx = self
            .builder
            .build_int_unsigned_rem(elem_val, i64_ty.const_int(64, false), "bit_idx")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let bit = self
            .builder
            .build_left_shift(i64_ty.const_int(1, false), bit_idx, "bit")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let gep = unsafe {
            self.builder
                .build_in_bounds_gep(
                    set_ty,
                    alloca,
                    &[i64_ty.const_int(0, false), word_idx],
                    "set_gep",
                )
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        };
        let cur = self
            .builder
            .build_load(i64_ty, gep, "cur_word")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .into_int_value();

        let new_word = if is_include {
            self.builder
                .build_or(cur, bit, "include")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        } else {
            let not_bit = self
                .builder
                .build_not(bit, "not_bit")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.builder
                .build_and(cur, not_bit, "exclude")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        };
        self.builder
            .build_store(gep, new_word)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_call_args(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>, CodeGenError> {
        let param_modes = self.proc_param_modes.get(name).cloned();
        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();

        // Capture-arg prefix for nested procs.
        let captures = self.proc_captures.get(name).cloned().unwrap_or_default();
        let cap_count = captures.len();
        for (cname, _) in &captures {
            let alloca = *self.lookup_var(cname.as_str()).ok_or_else(|| {
                CodeGenError::new(
                    format!("nested proc '{name}' captures '{cname}' which is not in scope"),
                    Some(span),
                )
            })?;
            call_args.push(alloca.into());
        }

        // Walk param_modes alongside args. Conformant array params consume 3
        // entries in param_modes (array, lo, hi) but only one source arg.
        let mut pi = cap_count;
        for arg in args.iter() {
            let pinfo = param_modes.as_ref().and_then(|p| p.get(pi));
            let is_var_param = pinfo
                .map(|(_, mode, _)| *mode == ParamMode::Var)
                .unwrap_or(false);
            let is_proc_param = matches!(pinfo.map(|(_, _, t)| t), Some(PascalType::Proc { .. }));
            let is_conformant = matches!(
                pinfo.map(|(_, _, t)| t),
                Some(PascalType::ConformantArray { .. })
            );

            if is_conformant {
                // Expect an array variable; pass &arr, lo, hi.
                let vname = match arg {
                    Expr::Var(n, _) => n.clone(),
                    _ => {
                        return Err(CodeGenError::new(
                            "conformant array arg requires a variable",
                            Some(span),
                        ));
                    }
                };
                let alloca = *self.lookup_var(vname.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{vname}'"), Some(span))
                })?;
                let var_ty = self.lookup_var_type(vname.as_str()).cloned().unwrap();
                let resolved = self.resolve_type(&var_ty);
                let (lo, hi) = match resolved {
                    PascalType::Array { lo, hi, .. } => (lo, hi),
                    PascalType::ConformantArray { .. } => {
                        // forwarding: load runtime lo/hi from sibling allocas
                        // For simplicity we don't support pass-through of conformant arrays
                        // (rare), so reject.
                        return Err(CodeGenError::new(
                            "forwarding a conformant array is not supported",
                            Some(span),
                        ));
                    }
                    _ => {
                        return Err(CodeGenError::new(
                            format!("'{vname}' is not an array"),
                            Some(span),
                        ));
                    }
                };
                let i64_ty = self.context.i64_type();
                call_args.push(alloca.into());
                call_args.push(i64_ty.const_int(lo as u64, true).into());
                call_args.push(i64_ty.const_int(hi as u64, true).into());
                pi += 3;
                continue;
            }

            if is_proc_param {
                // A routine name (or `@Routine`) gives its code, or for an
                // `is nested` parameter a closure over its environment; any
                // other expression is already a procedural value.
                let param_ty = pinfo.map(|(_, _, t)| self.resolve_type(t)).unwrap();
                let routine = match arg {
                    Expr::Var(n, _) => Some(n),
                    Expr::AddrOf(inner, _) => match inner.as_ref() {
                        Expr::Var(n, _) => Some(n),
                        _ => None,
                    },
                    _ => None,
                }
                .filter(|n| self.lookup_var(n).is_none())
                .cloned();
                let value: BasicValueEnum = match routine {
                    Some(r) if matches!(param_ty, PascalType::Proc { nested: true, .. }) => {
                        self.make_closure(&r, &param_ty, span)?.into()
                    }
                    Some(r) => self
                        .module
                        .get_function(&r)
                        .ok_or_else(|| {
                            CodeGenError::new(
                                format!("'{r}' is not a procedure or function"),
                                Some(span),
                            )
                        })?
                        .as_global_value()
                        .as_pointer_value()
                        .into(),
                    None => self.compile_expr(arg)?,
                };
                call_args.push(value.into());
                pi += 1;
                continue;
            }

            if is_var_param {
                // By reference: the address of the variable, element, field
                // or dereferenced pointer. Anything else (a constant passed
                // to an untyped `const` parameter) goes through a temporary.
                let address = if self.is_addressable(arg) {
                    self.resolve_array_base(arg, span)?.0
                } else {
                    let val = self.compile_expr(arg)?;
                    let temp = self
                        .entry_alloca(val.get_type(), "arg_tmp")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder
                        .build_store(temp, val)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    temp
                };
                call_args.push(address.into());
            } else {
                let val = match pinfo {
                    Some((_, _, param_ty)) => self.compile_expr_for(arg, param_ty)?,
                    None => self.compile_expr(arg)?,
                };
                let val = match pinfo {
                    Some((_, _, param_ty)) => self.convert_for_store(val, param_ty, span)?,
                    None => val,
                };
                call_args.push(val.into());
            }
            pi += 1;
        }
        Ok(call_args)
    }

    fn compile_write(
        &mut self,
        args: &[WriteArg],
        newline: bool,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);

        // If first arg is a file variable, switch to file-write mode
        let mut start_idx = 0;
        let mut file_target: Option<PointerValue<'ctx>> = None;
        if let Some(first) = args.first()
            && first.width.is_none()
            && first.precision.is_none()
            && let Expr::Var(name, _) = &first.expr
            && let Some(t) = self.lookup_var_type(name.as_str())
            && matches!(self.resolve_type(t), PascalType::File { .. })
        {
            let alloca = *self.lookup_var(name.as_str()).unwrap();
            let ptr_ty = self.context.ptr_type(AddressSpace::default());
            let fp = self
                .builder
                .build_load(ptr_ty, alloca, "fileptr")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                .into_pointer_value();
            file_target = Some(fp);
            start_idx = 1;
        }

        for arg in &args[start_idx..] {
            let val = self.compile_expr(&arg.expr)?;
            let arg_type = self.infer_expr_type(&arg.expr);

            // File output path
            if let Some(fp) = file_target {
                let fname = match arg_type {
                    PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => {
                        "bruto_file_write_int"
                    }
                    PascalType::Real => "bruto_file_write_real",
                    PascalType::Boolean => "bruto_file_write_int",
                    PascalType::Char => "bruto_file_write_char",
                    PascalType::String | PascalType::Pointer(_) => "bruto_file_write_str",
                    _ => {
                        return Err(CodeGenError::new(
                            "cannot write this type to file",
                            Some(span),
                        ));
                    }
                };
                let mut call_val = val;
                if matches!(arg_type, PascalType::Boolean) {
                    let iv = val.into_int_value();
                    call_val = self
                        .builder
                        .build_int_z_extend(iv, self.context.i64_type(), "z")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .into();
                }
                let f = self.module.get_function(fname).unwrap();
                self.builder
                    .build_call(f, &[fp.into(), call_val.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                continue;
            }

            // Stdout path with optional formatting
            let has_fmt = arg.width.is_some();
            if has_fmt {
                let width = self.compile_expr(arg.width.as_ref().unwrap())?;
                let width_i = if width.is_int_value() {
                    width.into_int_value()
                } else {
                    return Err(CodeGenError::new("width must be integer", Some(span)));
                };
                let width_i = if width_i.get_type().get_bit_width() < 64 {
                    self.builder
                        .build_int_s_extend(width_i, self.context.i64_type(), "we")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                } else {
                    width_i
                };

                match arg_type {
                    PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => {
                        let f = self.module.get_function("bruto_write_int_fmt").unwrap();
                        self.builder
                            .build_call(f, &[val.into(), width_i.into()], "")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                        let cf = self
                            .module
                            .get_function("bruto_capture_write_int_fmt")
                            .unwrap();
                        self.builder
                            .build_call(cf, &[val.into(), width_i.into()], "")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    }
                    PascalType::Real => {
                        if let Some(prec_expr) = &arg.precision {
                            let p = self.compile_expr(prec_expr)?;
                            let p_i = if p.is_int_value() {
                                p.into_int_value()
                            } else {
                                return Err(CodeGenError::new(
                                    "precision must be integer",
                                    Some(span),
                                ));
                            };
                            let p_i = if p_i.get_type().get_bit_width() < 64 {
                                self.builder
                                    .build_int_s_extend(p_i, self.context.i64_type(), "pe")
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                            } else {
                                p_i
                            };
                            let f = self.module.get_function("bruto_write_real_fmt").unwrap();
                            self.builder
                                .build_call(f, &[val.into(), width_i.into(), p_i.into()], "")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                            let cf = self
                                .module
                                .get_function("bruto_capture_write_real_fmt")
                                .unwrap();
                            self.builder
                                .build_call(cf, &[val.into(), width_i.into(), p_i.into()], "")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                        } else {
                            let f = self.module.get_function("bruto_write_real_fmt_w").unwrap();
                            self.builder
                                .build_call(f, &[val.into(), width_i.into()], "")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                            let cf = self
                                .module
                                .get_function("bruto_capture_write_real_fmt_w")
                                .unwrap();
                            self.builder
                                .build_call(cf, &[val.into(), width_i.into()], "")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                        }
                    }
                    PascalType::String | PascalType::Pointer(_) => {
                        let f = self.module.get_function("bruto_write_str_fmt").unwrap();
                        self.builder
                            .build_call(f, &[val.into(), width_i.into()], "")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                        let cf = self
                            .module
                            .get_function("bruto_capture_write_str_fmt")
                            .unwrap();
                        self.builder
                            .build_call(cf, &[val.into(), width_i.into()], "")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    }
                    _ => {
                        return Err(CodeGenError::new(
                            "formatted write not supported for this type",
                            Some(span),
                        ));
                    }
                }
                continue;
            }

            let (write_fn, capture_fn) = match arg_type {
                PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => {
                    ("bruto_write_int", "bruto_capture_write_int")
                }
                PascalType::Real => ("bruto_write_real", "bruto_capture_write_real"),
                PascalType::Boolean => ("bruto_write_bool", "bruto_capture_write_bool"),
                PascalType::Char => ("bruto_write_char", "bruto_capture_write_char"),
                PascalType::String | PascalType::Pointer(_) => {
                    ("bruto_write_str", "bruto_capture_write_str")
                }
                _ => return Err(CodeGenError::new("cannot write this type", Some(span))),
            };

            let f = self.module.get_function(write_fn).unwrap();
            self.builder
                .build_call(f, &[val.into()], "")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let cf = self.module.get_function(capture_fn).unwrap();
            self.builder
                .build_call(cf, &[val.into()], "")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        }

        if newline {
            if let Some(fp) = file_target {
                let f = self.module.get_function("bruto_file_writeln").unwrap();
                self.builder
                    .build_call(f, &[fp.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            } else {
                let f = self.module.get_function("bruto_writeln").unwrap();
                self.builder
                    .build_call(f, &[], "")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                let cf = self.module.get_function("bruto_capture_writeln").unwrap();
                self.builder
                    .build_call(cf, &[], "")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            }
        }

        Ok(())
    }

    fn compile_readln(&mut self, targets: &[String], span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        if targets.is_empty() {
            // Skip a line on stdin
            let f = self.module.get_function("bruto_read_str").unwrap();
            self.builder
                .build_call(f, &[], "")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            return Ok(());
        }

        // Check whether first target is a file variable
        let first_var_type = self
            .lookup_var_type(targets[0].as_str())
            .cloned()
            .ok_or_else(|| {
                CodeGenError::new(format!("unknown type for '{}'", targets[0]), Some(span))
            })?;
        let resolved_first = self.resolve_type(&first_var_type);
        let mut start_idx = 0;
        let mut file_target: Option<PointerValue<'ctx>> = None;
        if matches!(resolved_first, PascalType::File { .. }) {
            let alloca = *self.lookup_var(targets[0].as_str()).unwrap();
            let ptr_ty = self.context.ptr_type(AddressSpace::default());
            let fp = self
                .builder
                .build_load(ptr_ty, alloca, "fileptr")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                .into_pointer_value();
            file_target = Some(fp);
            start_idx = 1;
        }

        for target in &targets[start_idx..] {
            let alloca = *self.lookup_var(target.as_str()).ok_or_else(|| {
                CodeGenError::new(format!("undefined variable '{target}'"), Some(span))
            })?;
            let var_type = self
                .lookup_var_type(target.as_str())
                .cloned()
                .ok_or_else(|| {
                    CodeGenError::new(format!("unknown type for '{target}'"), Some(span))
                })?;
            let resolved = self.resolve_type(&var_type);

            let (read_fn, _is_file) = if file_target.is_some() {
                match resolved {
                    PascalType::Integer
                    | PascalType::SizedInt { .. }
                    | PascalType::Subrange { .. }
                    | PascalType::Enum { .. } => ("bruto_file_read_int", true),
                    PascalType::Real => ("bruto_file_read_real", true),
                    PascalType::Char => ("bruto_file_read_char", true),
                    PascalType::String => ("bruto_file_read_str", true),
                    _ => {
                        return Err(CodeGenError::new(
                            format!("readln from file for {resolved:?} not supported"),
                            Some(span),
                        ));
                    }
                }
            } else {
                match resolved {
                    PascalType::Integer
                    | PascalType::SizedInt { .. }
                    | PascalType::Subrange { .. }
                    | PascalType::Enum { .. } => ("bruto_read_int", false),
                    PascalType::Real => ("bruto_read_real", false),
                    PascalType::Char => ("bruto_read_char", false),
                    PascalType::String => ("bruto_read_str", false),
                    _ => {
                        return Err(CodeGenError::new(
                            format!("readln for {resolved:?} not supported"),
                            Some(span),
                        ));
                    }
                }
            };

            let f = self.module.get_function(read_fn).unwrap();
            let val = if let Some(fp) = file_target {
                self.builder
                    .build_call(f, &[fp.into()], "rval")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
            } else {
                self.builder
                    .build_call(f, &[], "rval")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
            };
            self.store_value(alloca, val, &var_type, span)?;
        }

        Ok(())
    }

    // ── array base resolution (for multi-dimensional indexing) ──

    fn resolve_array_base(
        &mut self,
        expr: &Expr,
        span: Span,
    ) -> Result<(PointerValue<'ctx>, PascalType), CodeGenError> {
        match expr {
            Expr::Var(name, vspan) => {
                let a = *self.lookup_var(name.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{name}'"), Some(*vspan))
                })?;
                let t = self
                    .lookup_var_type(name.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodeGenError::new(format!("unknown type for '{name}'"), Some(*vspan))
                    })?;
                Ok((a, t))
            }
            Expr::Index {
                array,
                index,
                span: idx_span,
            } => {
                let (base_ptr, base_type) = self.resolve_array_base(array, *idx_span)?;
                let idx_val = self.compile_expr(index)?;
                let idx_val = self.first_char_if_string(idx_val, span)?;
                let resolved = self.resolve_type(&base_type);
                self.gep_array_elem(base_ptr, &resolved, idx_val.into_int_value(), span)
            }
            Expr::FieldAccess {
                record,
                field,
                span: fa_span,
            } => {
                let (rec_ptr, rec_type) = self.resolve_array_base(record, *fa_span)?;
                self.field_address(rec_ptr, &rec_type, field, span)
            }
            Expr::Deref(inner, dspan) => {
                // The pointer itself: loaded from a variable, element or
                // field, or computed (a cast such as `PView(P)^`, a call).
                let (loaded, ptr_type) = if Self::is_place(inner)
                    && self.routine_name(inner).is_none()
                {
                    let (ptr, ptr_type) = self.resolve_array_base(inner, *dspan)?;
                    let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
                    let loaded = self
                        .builder
                        .build_load(ptr_ty, ptr, "deref_base")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .into_pointer_value();
                    (loaded, ptr_type)
                } else {
                    let ptr_type = self.infer_declared_type(inner);
                    let value = self.compile_expr(inner)?;
                    if !value.is_pointer_value() {
                        return Err(CodeGenError::new("dereference of non-pointer", Some(span)));
                    }
                    (value.into_pointer_value(), ptr_type)
                };
                let pointed = match self.resolve_type(&ptr_type) {
                    PascalType::Pointer(inner) => *inner,
                    _ => return Err(CodeGenError::new("dereference of non-pointer", Some(span))),
                };
                // `P^` of an untyped pointer is a place without a type: it
                // can be passed to an untyped `var` parameter or cast.
                Ok((loaded, pointed))
            }
            // `TFoo(Buf)`, `TByteArray(P^)`: the same memory seen as
            // another type.
            Expr::Call { name, args, .. }
                if args.len() == 1
                    && self.cast_target(name).is_some()
                    && self.is_addressable(&args[0]) =>
            {
                let target = self.cast_target(name).unwrap();
                let (address, _) = self.resolve_array_base(&args[0], span)?;
                Ok((address, target))
            }
            _ => Err(CodeGenError::new(
                "array indexing requires a variable",
                Some(span),
            )),
        }
    }

    /// `s[i]`: the `i`-th character (1-based) of a string value.
    fn string_char_at(
        &mut self,
        s: PointerValue<'ctx>,
        index: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i8_ty = self.context.i8_type();
        let idx = self.to_i64(index, span)?;
        let offset = self
            .builder
            .build_int_sub(idx, self.context.i64_type().const_int(1, false), "str_off")
            .map_err(err)?;
        let at = unsafe {
            self.builder
                .build_in_bounds_gep(i8_ty, s, &[offset], "str_at")
                .map_err(err)?
        };
        self.builder.build_load(i8_ty, at, "str_ch").map_err(err)
    }

    /// `s[i] := ch` for the string held in `slot`. Strings can share
    /// memory with literals and other variables, so the string is copied
    /// before the character is written and the copy stored back.
    fn set_string_char(
        &mut self,
        slot: PointerValue<'ctx>,
        index: BasicValueEnum<'ctx>,
        ch: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let strdup = self.module.get_function("strdup").unwrap_or_else(|| {
            self.module
                .add_function("strdup", ptr_ty.fn_type(&[ptr_ty.into()], false), None)
        });
        let old = self
            .builder
            .build_load(ptr_ty, slot, "str_old")
            .map_err(err)?;
        let copy = self
            .builder
            .build_call(strdup, &[old.into()], "str_copy")
            .map_err(err)?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        let idx = self.to_i64(index, span)?;
        let offset = self
            .builder
            .build_int_sub(idx, self.context.i64_type().const_int(1, false), "str_off")
            .map_err(err)?;
        let at = unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), copy, &[offset], "str_at")
                .map_err(err)?
        };
        self.store_value(at, ch, &PascalType::Char, span)?;
        self.builder.build_store(slot, copy).map_err(err)?;
        Ok(())
    }

    /// Whether `expr` is a `string` value (so `[i]` picks a character).
    fn is_string_expr(&self, expr: &Expr) -> bool {
        self.resolve_type(&self.infer_declared_type(expr)) == PascalType::String
    }

    /// The address and type of `field` in the record at `rec_ptr`: a fixed
    /// field, the variant tag, or a field of the variant part (which all
    /// share the union region after the tag).
    fn field_address(
        &self,
        rec_ptr: PointerValue<'ctx>,
        rec_type: &PascalType,
        field: &str,
        span: Span,
    ) -> Result<(PointerValue<'ctx>, PascalType), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let resolved = self.resolve_type(rec_type);
        let PascalType::Record {
            fields, variant, ..
        } = &resolved
        else {
            return Err(CodeGenError::new("field access on non-record", Some(span)));
        };
        let record_ty = self.llvm_type_for(&resolved);
        if let Some(index) = fields.iter().position(|(n, _)| n == field) {
            let addr = self
                .builder
                .build_struct_gep(record_ty, rec_ptr, index as u32, "fa_gep")
                .map_err(err)?;
            return Ok((addr, fields[index].1.clone()));
        }
        let missing = || {
            CodeGenError::new(
                format!("no field '{}' in this record", self.display(field)),
                Some(span),
            )
        };
        let v = variant.as_ref().ok_or_else(missing)?;
        if field == v.tag_name {
            let addr = self
                .builder
                .build_struct_gep(record_ty, rec_ptr, fields.len() as u32, "tag_gep")
                .map_err(err)?;
            return Ok((addr, v.tag_type.clone()));
        }
        let (offset, field_ty) = self.find_variant_field(v, field).ok_or_else(missing)?;
        let union = self
            .builder
            .build_struct_gep(record_ty, rec_ptr, (fields.len() + 1) as u32, "union_gep")
            .map_err(err)?;
        let addr = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    union,
                    &[self.context.i64_type().const_int(offset, false)],
                    "vfield_ptr",
                )
                .map_err(err)?
        };
        Ok((addr, field_ty))
    }

    /// Whether `resolve_array_base` can give `expr`'s address: a place
    /// (see `is_place`) that isn't a routine name, or a type cast of one.
    fn is_addressable(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Var(name, _) => self.lookup_var(name).is_some(),
            Expr::Call { name, args, .. } => {
                args.len() == 1 && self.cast_target(name).is_some() && self.is_addressable(&args[0])
            }
            other => Self::is_place(other),
        }
    }

    /// Whether `expr` names storage (a variable, element, field or
    /// dereference) whose address `resolve_array_base` can compute.
    fn is_place(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::Var(..) | Expr::Index { .. } | Expr::FieldAccess { .. } | Expr::Deref(..)
        )
    }

    // ── expressions ──────────────────────────────────────

    fn compile_expr(&mut self, expr: &Expr) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        match expr {
            // `System.X`: the built-in X, whatever else X names here.
            Expr::Call { name, args, span } if name.starts_with("system.") => {
                let bare = Expr::Call {
                    name: name["system.".len()..].to_string(),
                    args: args.clone(),
                    span: *span,
                };
                let saved = std::mem::replace(&mut self.bypass_user, true);
                let result = self.compile_expr(&bare);
                self.bypass_user = saved;
                result
            }
            Expr::Var(name, span) if name.starts_with("system.") => {
                let bare = Expr::Var(name["system.".len()..].to_string(), *span);
                let saved = std::mem::replace(&mut self.bypass_user, true);
                let result = self.compile_expr(&bare);
                self.bypass_user = saved;
                result
            }
            Expr::IndirectCall { callee, args, span } => self
                .compile_indirect_call(callee, args, *span)?
                .ok_or_else(|| {
                    CodeGenError::new("this procedural value doesn't return a value", Some(*span))
                }),
            Expr::RecordInit { span, .. } | Expr::ArrayInit { span, .. } => Err(CodeGenError::new(
                "a parenthesised record or array value is only allowed for a typed constant",
                Some(*span),
            )),
            Expr::MethodCall {
                object,
                method,
                args,
                span,
            } => {
                let value = self
                    .compile_method_call(object, method, args, *span)?
                    .ok_or_else(|| {
                        CodeGenError::new(
                            format!("method '{method}' doesn't return a value"),
                            Some(*span),
                        )
                    })?;
                // `if not TAncestor.Init(...) then Fail`.
                if self.ancestor_constructor(object, method) {
                    return Ok(self
                        .builder
                        .build_is_not_null(value.into_pointer_value(), "constructed")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .into());
                }
                Ok(value)
            }
            Expr::Inherited { method, args, span } => {
                let value = self
                    .compile_inherited(method, args, *span)?
                    .ok_or_else(|| {
                        CodeGenError::new(
                            format!("method '{method}' doesn't return a value"),
                            Some(*span),
                        )
                    })?;
                // `if not inherited Init(...) then Fail`: a constructor as
                // a value says whether it succeeded.
                if self.inherited_is_constructor(method) {
                    return Ok(self
                        .builder
                        .build_is_not_null(value.into_pointer_value(), "constructed")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .into());
                }
                Ok(value)
            }
            Expr::AddrOf(inner, span) => {
                // `@Proc` is the routine's code address.
                if let Some(f) = self.routine_name(inner) {
                    return Ok(f.as_global_value().as_pointer_value().into());
                }
                // `@TType.Method`: the code of that type's implementation.
                if let Expr::FieldAccess { record, field, .. } = inner.as_ref()
                    && let Expr::Var(type_name, _) = record.as_ref()
                    && self.lookup_var(type_name).is_none()
                    && let Some(info) = self.objects.get(type_name)
                {
                    let method = info.methods.get(field).ok_or_else(|| {
                        CodeGenError::new(
                            format!("'{}' has no method '{field}'", self.display(type_name)),
                            Some(*span),
                        )
                    })?;
                    return Ok(match self.module.get_function(&method.symbol) {
                        Some(f) => f.as_global_value().as_pointer_value().into(),
                        None => self
                            .abstract_error_fn()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                    });
                }
                let (ptr, _) = self.resolve_array_base(inner, *span)?;
                Ok(ptr.into())
            }
            Expr::IntLit(n, _) => Ok(self.context.i64_type().const_int(*n as u64, true).into()),
            Expr::RealLit(r, _) => Ok(self.context.f64_type().const_float(*r).into()),
            Expr::CharLit(c, _) => Ok(self.context.i8_type().const_int(*c as u64, false).into()),
            Expr::StrLit(s, _) => Ok(self.string_literal(s).into()),
            Expr::BoolLit(b, _) => Ok(self
                .context
                .bool_type()
                .const_int(if *b { 1 } else { 0 }, false)
                .into()),
            Expr::Nil(_) => Ok(self
                .context
                .ptr_type(AddressSpace::default())
                .const_null()
                .into()),
            Expr::Var(name, span) if self.self_method(name).is_some() => {
                let (object, self_ptr) = self.implicit_method(name).unwrap();
                self.call_method(
                    objects::Receiver {
                        object: &object,
                        self_ptr,
                        dispatch: objects::Dispatch::Virtual,
                        vmt: None,
                    },
                    name,
                    &[],
                    *span,
                )?
                .ok_or_else(|| {
                    CodeGenError::new(
                        format!("method '{name}' doesn't return a value"),
                        Some(*span),
                    )
                })
            }
            Expr::Var(name, span) => {
                // Check if this is an enum constant before looking up variables
                if let Some(&ordinal) = self.enum_values.get(name.as_str()) {
                    return Ok(self
                        .context
                        .i64_type()
                        .const_int(ordinal as u64, true)
                        .into());
                }
                // Pascal allows calling no-arg functions by bare name.
                if name == "ioresult" {
                    return self.compile_expr(&Expr::Call {
                        name: name.clone(),
                        args: Vec::new(),
                        span: *span,
                    });
                }
                // Predefined constants
                if name == "maxint" {
                    return Ok(self
                        .context
                        .i64_type()
                        .const_int(i64::MAX as u64, true)
                        .into());
                }
                // A bare function name that isn't shadowed by a variable is a
                // call with no arguments.
                if self.lookup_var(name.as_str()).is_none()
                    && matches!(self.proc_return_types.get(name.as_str()), Some(Some(_)))
                {
                    return self.compile_expr(&Expr::Call {
                        name: name.clone(),
                        args: Vec::new(),
                        span: *span,
                    });
                }
                if self.lookup_var(name.as_str()).is_none()
                    && self.system_func_type(name, 0).is_some()
                    && let Some(value) = self.compile_system_func(name, &[], *span)?
                {
                    return Ok(value);
                }
                let alloca = *self.lookup_var(name.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{name}'"), Some(*span))
                })?;
                let var_type = self
                    .lookup_var_type(name.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodeGenError::new(format!("unknown type for '{name}'"), Some(*span))
                    })?;

                let ty = self.llvm_type_for(&var_type);
                let val = self
                    .builder
                    .build_load(ty, alloca, name)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                self.widen_loaded(val, &var_type, *span)
            }
            Expr::Deref(inner, span) => {
                let inner_type = self.infer_expr_type(inner);
                // File buffer variable: f^ — load the buffer byte from the file struct.
                if matches!(self.resolve_type(&inner_type), PascalType::File { .. }) {
                    let ptr_val = self.compile_expr(inner)?;
                    let f = self.module.get_function("bruto_file_buf_load").unwrap();
                    let r = self
                        .builder
                        .build_call(f, &[ptr_val.into()], "fbuf")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                // inner must evaluate to a pointer; load the pointed-to value
                let ptr_val = self.compile_expr(inner)?;
                let PascalType::Pointer(pointed) = self.resolve_type(&inner_type) else {
                    return Err(CodeGenError::new(
                        "cannot dereference non-pointer",
                        Some(*span),
                    ));
                };
                if *pointed == PascalType::Untyped {
                    return Err(CodeGenError::new(UNTYPED_DEREF, Some(*span)));
                }
                let ty = self.llvm_type_for(&pointed);
                let val = self
                    .builder
                    .build_load(ty, ptr_val.into_pointer_value(), "deref")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                self.widen_loaded(val, &pointed, *span)
            }
            Expr::Index { array, index, span } if self.is_string_expr(array) => {
                let s = self.compile_expr(array)?.into_pointer_value();
                let idx = self.compile_expr(index)?;
                self.string_char_at(s, idx, *span)
            }
            Expr::Index { array, index, span } => {
                let (base_ptr, base_type) = self.resolve_array_base(array, *span)?;
                let idx_val = self.compile_expr(index)?;
                let resolved = self.resolve_type(&base_type);
                let (gep, elem_ty) =
                    self.gep_array_elem(base_ptr, &resolved, idx_val.into_int_value(), *span)?;
                let elem_llvm_ty = self.llvm_type_for(&elem_ty);
                let val = self
                    .builder
                    .build_load(elem_llvm_ty, gep, "arr_load")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                self.widen_loaded(val, &elem_ty, *span)
            }
            Expr::FieldAccess {
                record,
                field,
                span,
            } if self.is_parameterless_method(record, field) => self
                .compile_method_call(record, field, &[], *span)?
                .ok_or_else(|| {
                    CodeGenError::new(
                        format!("method '{field}' doesn't return a value"),
                        Some(*span),
                    )
                }),
            Expr::FieldAccess {
                record,
                field,
                span,
            } => {
                let (alloca, var_type) = self.resolve_array_base(record, *span)?;
                let resolved = self.resolve_type(&var_type);
                match &resolved {
                    PascalType::Record {
                        fields, variant, ..
                    } => {
                        // Check fixed fields first
                        if let Some(idx) = fields.iter().position(|(n, _)| n == field) {
                            let field_ty = &fields[idx].1;
                            let gep = self
                                .builder
                                .build_struct_gep(
                                    self.llvm_type_for(&resolved),
                                    alloca,
                                    idx as u32,
                                    "field_gep",
                                )
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                            let val = self
                                .builder
                                .build_load(self.llvm_type_for(field_ty), gep, "field_load")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                            self.widen_loaded(val, field_ty, *span)
                        } else if let Some(v) = variant {
                            if field == &v.tag_name {
                                // Tag field: index = fields.len()
                                let gep = self
                                    .builder
                                    .build_struct_gep(
                                        self.llvm_type_for(&resolved),
                                        alloca,
                                        fields.len() as u32,
                                        "tag_gep",
                                    )
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                                let val = self
                                    .builder
                                    .build_load(self.llvm_type_for(&v.tag_type), gep, "tag_load")
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                                self.widen_loaded(val, &v.tag_type, *span)
                            } else {
                                // Variant field in union
                                let (byte_offset, field_ty) =
                                    self.find_variant_field(v, field).ok_or_else(|| {
                                        CodeGenError::new(
                                            format!("no field '{field}' in record"),
                                            Some(*span),
                                        )
                                    })?;
                                let union_gep = self
                                    .builder
                                    .build_struct_gep(
                                        self.llvm_type_for(&resolved),
                                        alloca,
                                        (fields.len() + 1) as u32,
                                        "union_gep",
                                    )
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                                let byte_gep = unsafe {
                                    self.builder
                                        .build_in_bounds_gep(
                                            self.context.i8_type(),
                                            union_gep,
                                            &[self
                                                .context
                                                .i64_type()
                                                .const_int(byte_offset, false)],
                                            "vfield_ptr",
                                        )
                                        .map_err(|e| {
                                            CodeGenError::new(e.to_string(), Some(*span))
                                        })?
                                };
                                let val = self
                                    .builder
                                    .build_load(
                                        self.llvm_type_for(&field_ty),
                                        byte_gep,
                                        "vfield_load",
                                    )
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                                self.widen_loaded(val, &field_ty, *span)
                            }
                        } else {
                            Err(CodeGenError::new(
                                format!("no field '{field}' in record"),
                                Some(*span),
                            ))
                        }
                    }
                    _ => Err(CodeGenError::new("field access on non-record", Some(*span))),
                }
            }
            Expr::Call { name, args, span } => {
                // Type casts: integer(x), real(x), char(x), boolean(x)
                if matches!(name.as_str(), "integer" | "real" | "char" | "boolean")
                    && args.len() == 1
                {
                    let val = self.compile_expr(&args[0])?;
                    return self.compile_type_cast(name, val, *span);
                }
                // Casts to a declared or built-in type: `PView(P)`,
                // `Byte(x)`, `Pointer(n)`, `PtrUInt(p)`.
                if args.len() == 1
                    && let Some(target) = self.cast_target(name)
                {
                    // `TCallback(@Local)` with a nested procedural type: a
                    // closure over the routine's environment.
                    if let PascalType::Proc { nested: true, .. } = target {
                        let routine = match &args[0] {
                            Expr::AddrOf(inner, _) => match inner.as_ref() {
                                Expr::Var(n, _) => Some(n),
                                _ => None,
                            },
                            Expr::Var(n, _) => Some(n),
                            _ => None,
                        }
                        .filter(|n| self.lookup_var(n).is_none())
                        .cloned();
                        if let Some(r) = routine {
                            return Ok(self.make_closure(&r, &target, *span)?.into());
                        }
                    }
                    // `T(P^)` with an untyped pointer `P`: the memory it
                    // points at, read as a `T`.
                    if let Expr::Deref(inner, _) = &args[0]
                        && let PascalType::Pointer(pointed) =
                            self.resolve_type(&self.infer_declared_type(inner))
                        && *pointed == PascalType::Untyped
                    {
                        let address = self.compile_expr(inner)?.into_pointer_value();
                        if let PascalType::ShortString { .. } = target {
                            return self.short_to_c_string(address, *span);
                        }
                        let loaded = self
                            .builder
                            .build_load(self.llvm_type_for(&target), address, "reinterpret")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                        return self.widen_loaded(loaded, &target, *span);
                    }
                    // `ShortString(Buf)` with an untyped `Buf`: the short
                    // string stored there.
                    if let PascalType::ShortString { .. } = target
                        && self.is_addressable(&args[0])
                        && !matches!(
                            self.resolve_type(&self.infer_declared_type(&args[0])),
                            PascalType::String | PascalType::ShortString { .. }
                        )
                    {
                        let (address, _) = self.resolve_array_base(&args[0], *span)?;
                        return self.short_to_c_string(address, *span);
                    }
                    // A record, array or set seen through a variable of
                    // another type: load it from that variable's memory.
                    if matches!(
                        target,
                        PascalType::Record { .. }
                            | PascalType::Array { .. }
                            | PascalType::Set { .. }
                    ) && self.is_addressable(&args[0])
                    {
                        let (address, _) = self.resolve_array_base(&args[0], *span)?;
                        let ty = self.llvm_type_for(&target);
                        return self
                            .builder
                            .build_load(ty, address, "reinterpret")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)));
                    }
                    let val = self.compile_expr(&args[0])?;
                    return self.compile_cast_to(&target, val, *span);
                }
                // Inside a method, the object's methods hide routines of the
                // same name.
                if let Some((object, self_ptr)) = self
                    .implicit_method(name)
                    .or_else(|| self.recursive_method(name))
                {
                    return self
                        .call_method(
                            objects::Receiver {
                                object: &object,
                                self_ptr,
                                dispatch: objects::Dispatch::Virtual,
                                vmt: None,
                            },
                            name,
                            args,
                            *span,
                        )?
                        .ok_or_else(|| {
                            CodeGenError::new(
                                format!("method '{name}' doesn't return a value"),
                                Some(*span),
                            )
                        });
                }
                // `New(PType, Init(args))`: construct and return an object.
                if name == "new"
                    && args.len() == 2
                    && let (
                        Expr::Var(ptype, _),
                        Expr::Call {
                            name: ctor,
                            args: ctor_args,
                            ..
                        },
                    ) = (&args[0], &args[1])
                    && let Some(target) = self.cast_target(ptype)
                {
                    return self.construct_new(&target, ctor, ctor_args, *span);
                }
                // `String(x)`: a string stays as it is; a char becomes a
                // one-character string.
                if name == "string" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    if val.is_pointer_value() {
                        return Ok(val);
                    }
                    let ch = self.to_i64(val, *span)?;
                    let ch = self
                        .builder
                        .build_int_truncate(ch, self.context.i8_type(), "ch")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    let alloc = self.module.get_function("bruto_alloc").unwrap();
                    let buf = self
                        .builder
                        .build_call(
                            alloc,
                            &[self.context.i64_type().const_int(2, false).into()],
                            "chstr",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap()
                        .into_pointer_value();
                    self.builder
                        .build_store(buf, ch)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    let nul = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i8_type(),
                                buf,
                                &[self.context.i64_type().const_int(1, false)],
                                "nul",
                            )
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                    };
                    self.builder
                        .build_store(nul, self.context.i8_type().const_zero())
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(buf.into());
                }
                if name == "typeof" && args.len() == 1 {
                    return self.compile_typeof(&args[0], *span);
                }
                if name == "assigned" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    if !val.is_pointer_value() {
                        return Err(CodeGenError::new(
                            "assigned() needs a pointer or procedural value",
                            Some(*span),
                        ));
                    }
                    return Ok(self
                        .builder
                        .build_is_not_null(val.into_pointer_value(), "assigned")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .into());
                }
                // Built-in functions
                if name == "length" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let f = self.module.get_function("bruto_str_length").unwrap();
                    let r = self
                        .builder
                        .build_call(f, &[val.into()], "len")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                if name == "ord" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    if val.is_pointer_value() {
                        // String: load first byte
                        let byte = self
                            .builder
                            .build_load(
                                self.context.i8_type(),
                                val.into_pointer_value(),
                                "ord_byte",
                            )
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                        let ext = self
                            .builder
                            .build_int_z_extend(
                                byte.into_int_value(),
                                self.context.i64_type(),
                                "ord",
                            )
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                        return Ok(ext.into());
                    }
                    let ext = self
                        .builder
                        .build_int_z_extend(val.into_int_value(), self.context.i64_type(), "ord")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(ext.into());
                }
                if name == "chr" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let trunc = self
                        .builder
                        .build_int_truncate(val.into_int_value(), self.context.i8_type(), "chr")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(trunc.into());
                }
                if name == "abs" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    if val.is_float_value() {
                        let intrinsic = self.get_or_declare_f64_intrinsic("llvm.fabs.f64");
                        let r = self
                            .builder
                            .build_call(intrinsic, &[val.into()], "fabs")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                            .try_as_basic_value()
                            .basic()
                            .unwrap();
                        return Ok(r);
                    }
                    let iv = val.into_int_value();
                    let is_neg = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::SLT,
                            iv,
                            self.context.i64_type().const_int(0, false),
                            "is_neg",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    let neg = self
                        .builder
                        .build_int_neg(iv, "neg")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    let r = self
                        .builder
                        .build_select(is_neg, neg, iv, "abs")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(r);
                }
                if name == "sqr" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    if val.is_float_value() {
                        let fv = val.into_float_value();
                        let r = self
                            .builder
                            .build_float_mul(fv, fv, "sqr")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                        return Ok(r.into());
                    }
                    let iv = val.into_int_value();
                    let r = self
                        .builder
                        .build_int_mul(iv, iv, "sqr")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(r.into());
                }
                if name == "sqrt" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let fval = self.promote_to_f64(val, *span)?;
                    let intrinsic = self.get_or_declare_f64_intrinsic("llvm.sqrt.f64");
                    let r = self
                        .builder
                        .build_call(intrinsic, &[fval.into()], "sqrt")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                if name == "sin" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let fval = self.promote_to_f64(val, *span)?;
                    let intrinsic = self.get_or_declare_f64_intrinsic("llvm.sin.f64");
                    let r = self
                        .builder
                        .build_call(intrinsic, &[fval.into()], "sin")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                if name == "cos" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let fval = self.promote_to_f64(val, *span)?;
                    let intrinsic = self.get_or_declare_f64_intrinsic("llvm.cos.f64");
                    let r = self
                        .builder
                        .build_call(intrinsic, &[fval.into()], "cos")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                if name == "exp" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let fval = self.promote_to_f64(val, *span)?;
                    let intrinsic = self.get_or_declare_f64_intrinsic("llvm.exp.f64");
                    let r = self
                        .builder
                        .build_call(intrinsic, &[fval.into()], "exp")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                if name == "ln" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let fval = self.promote_to_f64(val, *span)?;
                    let intrinsic = self.get_or_declare_f64_intrinsic("llvm.log.f64");
                    let r = self
                        .builder
                        .build_call(intrinsic, &[fval.into()], "ln")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                if name == "arctan" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let fval = self.promote_to_f64(val, *span)?;
                    let atan_fn = self.module.get_function("atan").unwrap_or_else(|| {
                        let ft = self
                            .context
                            .f64_type()
                            .fn_type(&[self.context.f64_type().into()], false);
                        self.module.add_function("atan", ft, None)
                    });
                    let r = self
                        .builder
                        .build_call(atan_fn, &[fval.into()], "arctan")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                if name == "trunc" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let fval = self.promote_to_f64(val, *span)?;
                    let r = self
                        .builder
                        .build_float_to_signed_int(fval, self.context.i64_type(), "trunc")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(r.into());
                }
                if name == "round" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let fval = self.promote_to_f64(val, *span)?;
                    let round_fn = self.get_or_declare_f64_intrinsic("llvm.round.f64");
                    let rounded = self
                        .builder
                        .build_call(round_fn, &[fval.into()], "rounded")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    let r = self
                        .builder
                        .build_float_to_signed_int(
                            rounded.into_float_value(),
                            self.context.i64_type(),
                            "round",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(r.into());
                }
                if name == "odd" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let iv = val.into_int_value();
                    let i64_ty = self.context.i64_type();
                    let iv = if iv.get_type().get_bit_width() < 64 {
                        self.builder
                            .build_int_s_extend(iv, i64_ty, "ze")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                    } else {
                        iv
                    };
                    let one = i64_ty.const_int(1, false);
                    let masked = self
                        .builder
                        .build_and(iv, one, "odd_mask")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    let r = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::EQ, masked, one, "odd")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(r.into());
                }
                if name == "succ" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let iv = val.into_int_value();
                    let r = self
                        .builder
                        .build_int_add(iv, iv.get_type().const_int(1, false), "succ")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(r.into());
                }
                if name == "pred" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let iv = val.into_int_value();
                    let r = self
                        .builder
                        .build_int_sub(iv, iv.get_type().const_int(1, false), "pred")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(r.into());
                }
                if name == "sizeof" && args.len() == 1 {
                    let ty = match &args[0] {
                        Expr::Var(vname, _) => self.type_of_name(vname),
                        other => Some(self.infer_declared_type(other)),
                    }
                    .ok_or_else(|| {
                        CodeGenError::new("sizeof() needs a variable or a type", Some(*span))
                    })?;
                    let resolved = self.resolve_type(&ty);
                    // LLVM's own size, so it includes alignment padding and
                    // matches the real memory layout.
                    let size = self
                        .llvm_type_for(&resolved)
                        .size_of()
                        .ok_or_else(|| CodeGenError::new("type has no size", Some(*span)))?;
                    return Ok(size.into());
                }
                if name == "low" && args.len() == 1 {
                    if let Expr::Var(vname, _) = &args[0] {
                        let var_type = self.type_of_name(vname);
                        if let Some(ty) = var_type {
                            let resolved = self.resolve_type(&ty);
                            match &resolved {
                                PascalType::Array { lo, .. } => {
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int(*lo as u64, true)
                                        .into());
                                }
                                PascalType::Subrange { lo, .. } => {
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int(*lo as u64, true)
                                        .into());
                                }
                                PascalType::Enum { .. } => {
                                    return Ok(self.context.i64_type().const_int(0, false).into());
                                }
                                PascalType::Char => {
                                    return Ok(self.context.i64_type().const_int(0, false).into());
                                }
                                PascalType::Boolean => {
                                    return Ok(self.context.i64_type().const_int(0, false).into());
                                }
                                PascalType::Integer => {
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int(i64::MIN as u64, true)
                                        .into());
                                }
                                PascalType::SizedInt { bits, signed } => {
                                    let lo = if *signed { -(1i128 << (bits - 1)) } else { 0 };
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int(lo as i64 as u64, true)
                                        .into());
                                }
                                _ => {}
                            }
                        }
                    }
                    return Err(CodeGenError::new(
                        "low() requires an array, subrange, or enum variable",
                        Some(*span),
                    ));
                }
                if name == "high" && args.len() == 1 {
                    if let Expr::Var(vname, _) = &args[0] {
                        let var_type = self.type_of_name(vname);
                        if let Some(ty) = var_type {
                            let resolved = self.resolve_type(&ty);
                            match &resolved {
                                PascalType::Array { hi, .. } => {
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int(*hi as u64, true)
                                        .into());
                                }
                                PascalType::Subrange { hi, .. } => {
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int(*hi as u64, true)
                                        .into());
                                }
                                PascalType::Enum { values, .. } => {
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int((values.len() - 1) as u64, false)
                                        .into());
                                }
                                PascalType::Char => {
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int(255, false)
                                        .into());
                                }
                                PascalType::Boolean => {
                                    return Ok(self.context.i64_type().const_int(1, false).into());
                                }
                                // A dynamic string's last index: its length.
                                PascalType::String => {
                                    return self.compile_expr(&Expr::Call {
                                        name: "length".to_string(),
                                        args: args.clone(),
                                        span: *span,
                                    });
                                }
                                // A short string's capacity.
                                PascalType::ShortString { max } => {
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int(*max as u64, false)
                                        .into());
                                }
                                PascalType::Integer => {
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int(i64::MAX as u64, true)
                                        .into());
                                }
                                PascalType::SizedInt { bits, signed } => {
                                    let hi = if *signed {
                                        (1i128 << (bits - 1)) - 1
                                    } else {
                                        (1i128 << bits) - 1
                                    };
                                    // A qword's maximum doesn't fit in the
                                    // signed 64-bit expression type; it
                                    // wraps, as it would in FPC's int64.
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int(hi as u64, false)
                                        .into());
                                }
                                _ => {}
                            }
                        }
                    }
                    return Err(CodeGenError::new(
                        "high() requires an array, subrange, or enum variable",
                        Some(*span),
                    ));
                }
                if name == "copy" && args.len() == 3 {
                    let s = self.compile_expr(&args[0])?;
                    let idx = self.compile_expr(&args[1])?;
                    let cnt = self.compile_expr(&args[2])?;
                    let f = self.module.get_function("bruto_str_copy").unwrap();
                    let r = self
                        .builder
                        .build_call(f, &[s.into(), idx.into(), cnt.into()], "copy")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                if name == "pos" && args.len() == 2 {
                    let substr = self.compile_expr(&args[0])?;
                    let substr = self.as_string_value(substr, *span)?;
                    let s = self.compile_expr(&args[1])?;
                    let f = self.module.get_function("bruto_str_pos").unwrap();
                    let r = self
                        .builder
                        .build_call(f, &[substr.into(), s.into()], "pos")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                if name == "upcase" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    // Handle both char (i8) and string (ptr) — for strings, load first byte
                    let ch = if val.is_pointer_value() {
                        self.builder
                            .build_load(
                                self.context.i8_type(),
                                val.into_pointer_value(),
                                "first_ch",
                            )
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                            .into_int_value()
                    } else {
                        val.into_int_value()
                    };
                    let is_lower_a = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::UGE,
                            ch,
                            self.context.i8_type().const_int(b'a' as u64, false),
                            "ge_a",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    let is_lower_z = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::ULE,
                            ch,
                            self.context.i8_type().const_int(b'z' as u64, false),
                            "le_z",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    let is_lower = self
                        .builder
                        .build_and(is_lower_a, is_lower_z, "is_lower")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    let upper = self
                        .builder
                        .build_int_sub(ch, self.context.i8_type().const_int(32, false), "upper")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    let r = self
                        .builder
                        .build_select(is_lower, upper, ch, "upcase")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(r);
                }
                if (name == "eof" || name == "eoln") && args.len() == 1 {
                    let f_name = match &args[0] {
                        Expr::Var(n, _) => n.clone(),
                        _ => {
                            return Err(CodeGenError::new(
                                format!("{name} requires a file variable"),
                                Some(*span),
                            ));
                        }
                    };
                    let alloca = *self.lookup_var(f_name.as_str()).ok_or_else(|| {
                        CodeGenError::new(format!("undefined variable '{f_name}'"), Some(*span))
                    })?;
                    let ptr_ty = self.context.ptr_type(AddressSpace::default());
                    let s = self
                        .builder
                        .build_load(ptr_ty, alloca, "fs")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .into_pointer_value();
                    let fname = if name == "eof" {
                        "bruto_file_eof"
                    } else {
                        "bruto_file_eoln"
                    };
                    let f = self.module.get_function(fname).unwrap();
                    let r = self
                        .builder
                        .build_call(f, &[s.into()], name.as_str())
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                if name == "ioresult" && args.is_empty() {
                    let f = self.module.get_function("bruto_ioresult").unwrap();
                    let r = self
                        .builder
                        .build_call(f, &[], "ior")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    // i32 → i64
                    let ext = self
                        .builder
                        .build_int_z_extend(r.into_int_value(), self.context.i64_type(), "iorx")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(ext.into());
                }
                if name == "filepos" && args.len() == 1 {
                    let f_name = match &args[0] {
                        Expr::Var(n, _) => n.clone(),
                        _ => {
                            return Err(CodeGenError::new(
                                "filepos requires a file variable",
                                Some(*span),
                            ));
                        }
                    };
                    let alloca = *self.lookup_var(f_name.as_str()).unwrap();
                    let ptr_ty = self.context.ptr_type(AddressSpace::default());
                    let s = self
                        .builder
                        .build_load(ptr_ty, alloca, "fs")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .into_pointer_value();
                    let f = self.module.get_function("bruto_file_filepos").unwrap();
                    let r = self
                        .builder
                        .build_call(f, &[s.into()], "fp")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                if name == "filesize" && args.len() == 1 {
                    let f_name = match &args[0] {
                        Expr::Var(n, _) => n.clone(),
                        _ => {
                            return Err(CodeGenError::new(
                                "filesize requires a file variable",
                                Some(*span),
                            ));
                        }
                    };
                    let alloca = *self.lookup_var(f_name.as_str()).unwrap();
                    let ptr_ty = self.context.ptr_type(AddressSpace::default());
                    let s = self
                        .builder
                        .build_load(ptr_ty, alloca, "fs")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .into_pointer_value();
                    let f = self.module.get_function("bruto_file_filesize").unwrap();
                    let r = self
                        .builder
                        .build_call(f, &[s.into()], "fs2")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(r);
                }
                if name == "concat" && args.len() >= 2 {
                    let concat_fn = self.module.get_function("bruto_str_concat").unwrap();
                    let mut result = self.compile_expr(&args[0])?;
                    for arg in &args[1..] {
                        let next = self.compile_expr(arg)?;
                        result = self
                            .builder
                            .build_call(concat_fn, &[result.into(), next.into()], "concat")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                            .try_as_basic_value()
                            .basic()
                            .unwrap();
                    }
                    return Ok(result);
                }
                // A call through a procedural variable or parameter.
                if let Some(proc_ty) = self.procedural_var_type(name) {
                    let slot = *self.lookup_var(name.as_str()).unwrap();
                    let target = self
                        .builder
                        .build_load(self.context.ptr_type(AddressSpace::default()), slot, "proc")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .into_pointer_value();
                    return self
                        .call_procedural(target, &proc_ty, args, *span)?
                        .ok_or_else(|| {
                            CodeGenError::new(
                                format!("procedural variable '{name}' does not return a value"),
                                Some(*span),
                            )
                        });
                }
                let name = &self.resolve_overload(name, args);
                if !self.proc_return_types.contains_key(name.as_str())
                    && let Some(value) = self.compile_system_func(name, args, *span)?
                {
                    return Ok(value);
                }
                let func = self.module.get_function(name).ok_or_else(|| {
                    CodeGenError::new(format!("undefined function '{name}'"), Some(*span))
                })?;
                let call_args = self.compile_call_args(name, args, *span)?;
                let ret = self
                    .builder
                    .build_call(func, &call_args, "call")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                let val = ret.try_as_basic_value().basic().ok_or_else(|| {
                    CodeGenError::new(
                        format!("function '{name}' does not return a value"),
                        Some(*span),
                    )
                })?;
                match self.proc_return_types.get(name.as_str()).cloned().flatten() {
                    Some(ret_ty) => self.widen_loaded(val, &ret_ty, *span),
                    None => Ok(val),
                }
            }
            Expr::SetConstructor { elements, span } => {
                self.compile_set_constructor(elements, *span)
            }
            Expr::BinOp {
                op,
                left,
                right,
                span,
            } => self.compile_binop(*op, left, right, *span),
            Expr::UnaryOp { op, operand, span } => self.compile_unaryop(*op, operand, *span),
        }
    }

    /// Compile an expression used as an ordinal (a set element, say). A
    /// one-character string such as `'a'` is the character itself.
    fn compile_ordinal(&mut self, expr: &Expr) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        if let Expr::StrLit(text, _) = expr
            && text.len() == 1
        {
            return Ok(self
                .context
                .i8_type()
                .const_int(text.as_bytes()[0] as u64, false)
                .into());
        }
        let val = self.compile_expr(expr)?;
        self.first_char_if_string(val, expr.span())
    }

    /// A string value used where a `char` is expected (a one-character
    /// literal, typically) becomes its first character.
    fn first_char_if_string(
        &self,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        if !val.is_pointer_value() {
            return Ok(val);
        }
        self.builder
            .build_load(self.context.i8_type(), val.into_pointer_value(), "first_ch")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))
    }

    fn compile_set_constructor(
        &mut self,
        elements: &[SetElement],
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let set_ty = self.context.i64_type().array_type(4);
        let alloca = self
            .entry_alloca(set_ty, "set_tmp")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Zero-initialize with memset
        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let memset_fn = self
            .module
            .get_function("llvm.memset.p0.i64")
            .unwrap_or_else(|| {
                let fn_type = self.context.void_type().fn_type(
                    &[
                        i8_ptr.into(),
                        self.context.i8_type().into(),
                        self.context.i64_type().into(),
                        self.context.bool_type().into(),
                    ],
                    false,
                );
                self.module
                    .add_function("llvm.memset.p0.i64", fn_type, None)
            });
        self.builder
            .build_call(
                memset_fn,
                &[
                    alloca.into(),
                    self.context.i8_type().const_int(0, false).into(),
                    self.context.i64_type().const_int(32, false).into(),
                    self.context.bool_type().const_int(0, false).into(),
                ],
                "",
            )
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        let i64_ty = self.context.i64_type();
        let zero = i64_ty.const_int(0, false);
        let sixty_four = i64_ty.const_int(64, false);
        let one_i64 = i64_ty.const_int(1, false);

        for elem in elements {
            match elem {
                SetElement::Single(expr) => {
                    let ord_val = self.compile_ordinal(expr)?;
                    let ord = ord_val.into_int_value();
                    // Extend to i64 if needed (e.g., from i8 for char)
                    let ord = if ord.get_type().get_bit_width() < 64 {
                        self.builder
                            .build_int_z_extend(ord, i64_ty, "ord_ext")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    } else {
                        ord
                    };
                    // word_idx = ord / 64, bit_idx = ord % 64
                    let word_idx = self
                        .builder
                        .build_int_unsigned_div(ord, sixty_four, "word_idx")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let bit_idx = self
                        .builder
                        .build_int_unsigned_rem(ord, sixty_four, "bit_idx")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let mask = self
                        .builder
                        .build_left_shift(one_i64, bit_idx, "mask")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    // GEP into the word
                    let gep = unsafe {
                        self.builder
                            .build_in_bounds_gep(set_ty, alloca, &[zero, word_idx], "set_word_ptr")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    };
                    let cur = self
                        .builder
                        .build_load(i64_ty, gep, "cur_word")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let new = self
                        .builder
                        .build_or(cur.into_int_value(), mask, "new_word")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder
                        .build_store(gep, new)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                }
                SetElement::Range(lo_expr, hi_expr) => {
                    let lo_val = self.compile_ordinal(lo_expr)?.into_int_value();
                    let hi_val = self.compile_ordinal(hi_expr)?.into_int_value();
                    let lo_val = if lo_val.get_type().get_bit_width() < 64 {
                        self.builder
                            .build_int_z_extend(lo_val, i64_ty, "lo_ext")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    } else {
                        lo_val
                    };
                    let hi_val = if hi_val.get_type().get_bit_width() < 64 {
                        self.builder
                            .build_int_z_extend(hi_val, i64_ty, "hi_ext")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    } else {
                        hi_val
                    };

                    let current_fn = self.current_fn.unwrap();
                    let loop_bb = self
                        .context
                        .append_basic_block(current_fn, "set_range_loop");
                    let body_bb = self
                        .context
                        .append_basic_block(current_fn, "set_range_body");
                    let done_bb = self
                        .context
                        .append_basic_block(current_fn, "set_range_done");

                    // Store loop variable
                    let iter_alloca = self
                        .entry_alloca(i64_ty, "set_iter")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder
                        .build_store(iter_alloca, lo_val)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder
                        .build_unconditional_branch(loop_bb)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

                    // Loop header: check iter <= hi
                    self.builder.position_at_end(loop_bb);
                    let iter_val = self
                        .builder
                        .build_load(i64_ty, iter_alloca, "iter")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .into_int_value();
                    let cond = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::SLE,
                            iter_val,
                            hi_val,
                            "range_cond",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder
                        .build_conditional_branch(cond, body_bb, done_bb)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

                    // Loop body: set bit for iter_val
                    self.builder.position_at_end(body_bb);
                    let word_idx = self
                        .builder
                        .build_int_unsigned_div(iter_val, sixty_four, "word_idx")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let bit_idx = self
                        .builder
                        .build_int_unsigned_rem(iter_val, sixty_four, "bit_idx")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let mask = self
                        .builder
                        .build_left_shift(one_i64, bit_idx, "mask")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let gep = unsafe {
                        self.builder
                            .build_in_bounds_gep(set_ty, alloca, &[zero, word_idx], "set_word_ptr")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    };
                    let cur = self
                        .builder
                        .build_load(i64_ty, gep, "cur_word")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let new = self
                        .builder
                        .build_or(cur.into_int_value(), mask, "new_word")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder
                        .build_store(gep, new)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

                    // Increment
                    let next = self
                        .builder
                        .build_int_add(iter_val, one_i64, "next_iter")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder
                        .build_store(iter_alloca, next)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder
                        .build_unconditional_branch(loop_bb)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

                    self.builder.position_at_end(done_bb);
                }
            }
        }

        // Load and return the set value
        let result = self
            .builder
            .build_load(set_ty, alloca, "set_val")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(result)
    }

    /// Replace type names with what they stand for. Pointer targets are
    /// resolved through one pointer only, so a self-referencing type
    /// (`PNode = ^TNode` with a `Next: PNode` field) stays finite; deeper
    /// targets are resolved when they are dereferenced.
    fn resolve_type(&self, ty: &PascalType) -> PascalType {
        self.resolve_type_at(ty, false)
    }

    fn resolve_type_at(&self, ty: &PascalType, behind_pointer: bool) -> PascalType {
        match ty {
            PascalType::Named(name) => self
                .type_defs
                .get(name)
                .map(|t| self.resolve_type_at(t, behind_pointer))
                .unwrap_or(PascalType::Integer),
            PascalType::Pointer(inner) if behind_pointer => PascalType::Pointer(inner.clone()),
            PascalType::Pointer(inner) => {
                PascalType::Pointer(Box::new(self.resolve_type_at(inner, true)))
            }
            PascalType::Array { lo, hi, elem } => PascalType::Array {
                lo: *lo,
                hi: *hi,
                elem: Box::new(self.resolve_type_at(elem, behind_pointer)),
            },
            PascalType::Record {
                fields,
                variant,
                object,
            } => PascalType::Record {
                object: object.clone(),
                fields: fields
                    .iter()
                    .map(|(n, t)| (n.clone(), self.resolve_type_at(t, behind_pointer)))
                    .collect(),
                variant: variant.as_ref().map(|v| {
                    Box::new(RecordVariant {
                        tag_name: v.tag_name.clone(),
                        tag_type: self.resolve_type_at(&v.tag_type, behind_pointer),
                        variants: v
                            .variants
                            .iter()
                            .map(|(vals, vf)| {
                                (
                                    vals.clone(),
                                    vf.iter()
                                        .map(|(n, t)| {
                                            (n.clone(), self.resolve_type_at(t, behind_pointer))
                                        })
                                        .collect(),
                                )
                            })
                            .collect(),
                    })
                }),
            },
            PascalType::Set { elem } => PascalType::Set {
                elem: Box::new(self.resolve_type_at(elem, behind_pointer)),
            },
            PascalType::File { elem } => PascalType::File {
                elem: Box::new(self.resolve_type_at(elem, behind_pointer)),
            },
            PascalType::Proc {
                params,
                return_type,
                nested,
            } => PascalType::Proc {
                params: params
                    .iter()
                    .map(|t| self.resolve_type_at(t, behind_pointer))
                    .collect(),
                return_type: return_type
                    .as_ref()
                    .map(|t| Box::new(self.resolve_type_at(t, behind_pointer))),
                nested: *nested,
            },
            PascalType::ConformantArray {
                lo_name,
                hi_name,
                elem,
            } => PascalType::ConformantArray {
                lo_name: lo_name.clone(),
                hi_name: hi_name.clone(),
                elem: Box::new(self.resolve_type_at(elem, behind_pointer)),
            },
            PascalType::Enum { .. } | PascalType::Subrange { .. } => ty.clone(),
            other => other.clone(),
        }
    }

    fn create_debug_type(&self, ty: &PascalType) -> Option<DIType<'ctx>> {
        match ty {
            PascalType::Enum { name, values } => {
                // Emit DW_TAG_enumeration_type so lldb prints names rather
                // than ordinals. Falls back to plain `long` if the inner
                // basic type can't be created.
                let inner = self
                    .di_builder
                    .create_basic_type("long", 64, 0x05, DIFlags::ZERO)
                    .ok()?;
                let enumerators: Vec<_> = values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        self.di_builder
                            .create_enumerator(&self.display(v), i as i64, false)
                    })
                    .collect();
                let display_name = if name.is_empty() {
                    "Enum".to_string()
                } else {
                    self.display(name)
                };
                Some(
                    self.di_builder
                        .create_enumeration_type(
                            self.compile_unit.as_debug_info_scope(),
                            &display_name,
                            self.compile_unit.get_file(),
                            0,
                            64,
                            64,
                            &enumerators,
                            inner.as_type(),
                        )
                        .as_type(),
                )
            }
            PascalType::SizedInt { bits, signed } => {
                let name = match (bits, signed) {
                    (8, true) => "ShortInt",
                    (8, false) => "Byte",
                    (16, true) => "SmallInt",
                    (16, false) => "Word",
                    (32, true) => "LongInt",
                    (32, false) => "LongWord",
                    _ => "QWord",
                };
                // DW_ATE_signed / DW_ATE_unsigned (not unsigned_char, so
                // a Byte prints as a number).
                let encoding = if *signed { 0x05 } else { 0x07 };
                self.di_builder
                    .create_basic_type(name, *bits as u64, encoding, DIFlags::ZERO)
                    .ok()
                    .map(|t| t.as_type())
            }
            PascalType::Integer | PascalType::Subrange { .. } => self
                .di_builder
                .create_basic_type("long", 64, 0x05, DIFlags::ZERO)
                .ok()
                .map(|t| t.as_type()),
            PascalType::Real => self
                .di_builder
                .create_basic_type("double", 64, 0x04, DIFlags::ZERO)
                .ok()
                .map(|t| t.as_type()),
            PascalType::Boolean => self
                .di_builder
                .create_basic_type("bool", 8, 0x02, DIFlags::ZERO)
                .ok()
                .map(|t| t.as_type()),
            PascalType::Char => self
                .di_builder
                .create_basic_type("char", 8, 0x08, DIFlags::ZERO)
                .ok()
                .map(|t| t.as_type()),
            PascalType::String => {
                // char * — pointer to char, so lldb shows the string content
                let char_ty = self
                    .di_builder
                    .create_basic_type("char", 8, 0x08, DIFlags::ZERO)
                    .ok()?;
                Some(
                    self.di_builder
                        .create_pointer_type(
                            "char *",
                            char_ty.as_type(),
                            64,
                            0,
                            AddressSpace::default(),
                        )
                        .as_type(),
                )
            }
            PascalType::Pointer(inner) => {
                // Follow at most two pointers, so a self-referencing type
                // (`Next: PNode` inside TNode) has a finite description.
                let depth = self.debug_pointer_depth.get();
                let inner_di = if depth >= 2 {
                    self.di_builder
                        .create_basic_type("opaque", 8, 0x08, DIFlags::ZERO)
                        .ok()?
                        .as_type()
                } else {
                    self.debug_pointer_depth.set(depth + 1);
                    let inner_di = self.create_debug_type(inner);
                    self.debug_pointer_depth.set(depth);
                    inner_di?
                };
                Some(
                    self.di_builder
                        .create_pointer_type("ptr", inner_di, 64, 0, AddressSpace::default())
                        .as_type(),
                )
            }
            PascalType::Record {
                fields, variant, ..
            } => self.create_record_debug_type(fields, variant.as_deref()),
            _ => {
                // Arrays, sets, etc. — opaque aggregate.
                self.di_builder
                    .create_basic_type("aggregate", 64, 0x05, DIFlags::ZERO)
                    .ok()
                    .map(|t| t.as_type())
            }
        }
    }

    /// Build a DICompositeType (struct) describing a record's layout. Fixed
    /// fields go at sequential 8-byte offsets; the variant part — if any —
    /// places every variant's fields starting at the same offset (the union
    /// region). lldb will print all of them; the IDE's `format_with_meta`
    /// then filters to the active variant based on the tag value.
    fn create_record_debug_type(
        &self,
        fields: &[(String, PascalType)],
        variant: Option<&RecordVariant>,
    ) -> Option<DIType<'ctx>> {
        let scope = self.compile_unit.as_debug_info_scope();
        let file = self.compile_unit.get_file();
        let mut members: Vec<DIType<'ctx>> = Vec::new();
        let mut offset: u64 = 0;

        for (name, ty) in fields {
            let resolved = self.resolve_type(ty);
            let mty = self.create_debug_type(&resolved)?;
            let size = self.sizeof_type(&resolved) * 8;
            members.push(
                self.di_builder
                    .create_member_type(
                        scope,
                        &self.display(name),
                        file,
                        0,
                        size,
                        64,
                        offset,
                        DIFlags::ZERO,
                        mty,
                    )
                    .as_type(),
            );
            offset += self.sizeof_type(&resolved) * 8;
        }

        if let Some(v) = variant {
            // Tag field (named member, sequential).
            let tag_resolved = self.resolve_type(&v.tag_type);
            let tag_dt = self.create_debug_type(&tag_resolved)?;
            let tag_size = self.sizeof_type(&tag_resolved) * 8;
            members.push(
                self.di_builder
                    .create_member_type(
                        scope,
                        &self.display(&v.tag_name),
                        file,
                        0,
                        tag_size,
                        64,
                        offset,
                        DIFlags::ZERO,
                        tag_dt,
                    )
                    .as_type(),
            );
            offset += tag_size;

            // Variant fields: all variants share the same starting offset
            // (the union region). Sequential within a single variant.
            for (_vals, vfields) in &v.variants {
                let mut voffset = offset;
                for (vname, vty) in vfields {
                    let vresolved = self.resolve_type(vty);
                    let vdt = self.create_debug_type(&vresolved)?;
                    let vsize = self.sizeof_type(&vresolved) * 8;
                    members.push(
                        self.di_builder
                            .create_member_type(
                                scope,
                                &self.display(vname),
                                file,
                                0,
                                vsize,
                                64,
                                voffset,
                                DIFlags::ZERO,
                                vdt,
                            )
                            .as_type(),
                    );
                    voffset += vsize;
                }
            }

            // Total record size = fixed + tag + max variant size.
            let max_variant_bits: u64 = v
                .variants
                .iter()
                .map(|(_, vf)| {
                    vf.iter()
                        .map(|(_, t)| self.sizeof_type(&self.resolve_type(t)) * 8)
                        .sum::<u64>()
                })
                .max()
                .unwrap_or(0);
            let total = offset + max_variant_bits;
            Some(
                self.di_builder
                    .create_struct_type(
                        scope,
                        "record",
                        file,
                        0,
                        total,
                        64,
                        DIFlags::ZERO,
                        None,
                        &members,
                        0,
                        None,
                        "",
                    )
                    .as_type(),
            )
        } else {
            Some(
                self.di_builder
                    .create_struct_type(
                        scope,
                        "record",
                        file,
                        0,
                        offset,
                        64,
                        DIFlags::ZERO,
                        None,
                        &members,
                        0,
                        None,
                        "",
                    )
                    .as_type(),
            )
        }
    }

    fn llvm_type_for(&self, ty: &PascalType) -> inkwell::types::BasicTypeEnum<'ctx> {
        match ty {
            PascalType::ShortString { max } => self
                .context
                .i8_type()
                .array_type(max + 1)
                .as_basic_type_enum(),
            PascalType::SizedInt { bits, .. } => self
                .context
                .custom_width_int_type(*bits)
                .as_basic_type_enum(),
            // Only reachable through an untyped pointer; treat it as bytes.
            // Object declarations are lowered to records when registered.
            PascalType::Untyped | PascalType::Object(_) => {
                self.context.i8_type().as_basic_type_enum()
            }
            // Constant-expression types are folded by the semantic pass;
            // `ArrayExpr` can't reach here.
            PascalType::Integer
            | PascalType::Enum { .. }
            | PascalType::Subrange { .. }
            | PascalType::SubrangeExpr { .. }
            | PascalType::ArrayExpr { .. } => self.context.i64_type().as_basic_type_enum(),
            PascalType::Real => self.context.f64_type().as_basic_type_enum(),
            PascalType::Boolean => self.context.bool_type().as_basic_type_enum(),
            PascalType::Char => self.context.i8_type().as_basic_type_enum(),
            PascalType::String | PascalType::Pointer(_) => self
                .context
                .ptr_type(AddressSpace::default())
                .as_basic_type_enum(),
            PascalType::File { .. } => self
                .context
                .ptr_type(AddressSpace::default())
                .as_basic_type_enum(),
            PascalType::Proc { .. } => self
                .context
                .ptr_type(AddressSpace::default())
                .as_basic_type_enum(),
            PascalType::ConformantArray { .. } => self
                .context
                .ptr_type(AddressSpace::default())
                .as_basic_type_enum(),
            PascalType::Set { .. } => self.context.i64_type().array_type(4).as_basic_type_enum(),
            PascalType::Array { lo, hi, elem } => {
                let count = (hi - lo + 1).max(0) as u32;
                let elem_ty = self.llvm_type_for(elem);
                elem_ty.array_type(count).as_basic_type_enum()
            }
            PascalType::Record {
                fields, variant, ..
            } => {
                let mut field_types: Vec<inkwell::types::BasicTypeEnum> =
                    fields.iter().map(|(_, t)| self.llvm_type_for(t)).collect();
                if let Some(v) = variant {
                    field_types.push(self.llvm_type_for(&v.tag_type));
                    let max_size = v
                        .variants
                        .iter()
                        .map(|(_, vf)| vf.iter().map(|(_, t)| self.sizeof_type(t)).sum::<u64>())
                        .max()
                        .unwrap_or(0);
                    if max_size > 0 {
                        field_types.push(
                            self.context
                                .i8_type()
                                .array_type(max_size as u32)
                                .as_basic_type_enum(),
                        );
                    }
                }
                self.context
                    .struct_type(&field_types, false)
                    .as_basic_type_enum()
            }
            PascalType::Named(_) => {
                let resolved = self.resolve_type(ty);
                self.llvm_type_for(&resolved)
            }
        }
    }

    /// `lhs and right` / `lhs or right` for booleans, evaluating `right`
    /// only when `lhs` doesn't already decide the result.
    fn compile_short_circuit(
        &mut self,
        op: BinOp,
        lhs: inkwell::values::IntValue<'ctx>,
        right: &Expr,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let func = self.current_fn.unwrap();
        let lhs_end = self.builder.get_insert_block().unwrap();
        let rhs_bb = self.context.append_basic_block(func, "sc_rhs");
        let done_bb = self.context.append_basic_block(func, "sc_done");
        let is_and = op == BinOp::And;
        if is_and {
            self.builder
                .build_conditional_branch(lhs, rhs_bb, done_bb)
                .map_err(err)?;
        } else {
            self.builder
                .build_conditional_branch(lhs, done_bb, rhs_bb)
                .map_err(err)?;
        }

        self.builder.position_at_end(rhs_bb);
        let rhs = self.compile_expr(right)?;
        if !rhs.is_int_value() {
            return Err(CodeGenError::new(
                "operands of 'and' / 'or' must both be boolean or both integer",
                Some(span),
            ));
        }
        let rhs = rhs.into_int_value();
        let rhs = if rhs.get_type().get_bit_width() == 1 {
            rhs
        } else {
            self.builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    rhs,
                    rhs.get_type().const_zero(),
                    "tobool",
                )
                .map_err(err)?
        };
        let rhs_end = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(done_bb)
            .map_err(err)?;

        self.builder.position_at_end(done_bb);
        let bool_ty = self.context.bool_type();
        let phi = self.builder.build_phi(bool_ty, "sc").map_err(err)?;
        let decided = bool_ty.const_int(if is_and { 0 } else { 1 }, false);
        phi.add_incoming(&[(&decided, lhs_end), (&rhs, rhs_end)]);
        Ok(phi.as_basic_value())
    }

    /// `p + n` / `p - n` (scaled by the size of what `p` points to) and
    /// `p - q` (their distance in elements), as FPC allows for typed
    /// pointers. `None` when this isn't pointer arithmetic.
    fn pointer_arithmetic(
        &mut self,
        op: BinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        left: &Expr,
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        if !lhs.is_pointer_value() || !matches!(op, BinOp::Add | BinOp::Sub) {
            return Ok(None);
        }
        let elem = match self.resolve_type(&self.infer_declared_type(left)) {
            PascalType::Pointer(inner) if *inner != PascalType::Untyped => {
                self.llvm_type_for(&self.resolve_type(&inner))
            }
            // Untyped pointers step in bytes. (Strings are pointers too,
            // but `+` on them is concatenation, handled elsewhere.)
            PascalType::Pointer(_) => self.context.i8_type().as_basic_type_enum(),
            _ => return Ok(None),
        };
        let i64_ty = self.context.i64_type();
        let base = lhs.into_pointer_value();
        if rhs.is_int_value() {
            let mut n = self.to_i64(rhs, span)?;
            if op == BinOp::Sub {
                n = self.builder.build_int_neg(n, "neg").map_err(err)?;
            }
            let moved = unsafe {
                self.builder
                    .build_gep(elem, base, &[n], "ptr_add")
                    .map_err(err)?
            };
            return Ok(Some(moved.into()));
        }
        if rhs.is_pointer_value() && op == BinOp::Sub {
            let a = self
                .builder
                .build_ptr_to_int(base, i64_ty, "pa")
                .map_err(err)?;
            let b = self
                .builder
                .build_ptr_to_int(rhs.into_pointer_value(), i64_ty, "pb")
                .map_err(err)?;
            let bytes = self.builder.build_int_sub(a, b, "pdiff").map_err(err)?;
            let size = elem.size_of().unwrap_or_else(|| i64_ty.const_int(1, false));
            let count = self
                .builder
                .build_int_signed_div(bytes, size, "pcount")
                .map_err(err)?;
            return Ok(Some(count.into()));
        }
        Ok(None)
    }

    fn compile_binop(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let lhs = self.compile_expr(left)?;
        // Boolean `and` / `or` short-circuit, as in Turbo Pascal and FPC's
        // default `{$B-}`: `(p <> nil) and (p^.x > 0)` must not touch `p^`.
        if matches!(op, BinOp::And | BinOp::Or)
            && lhs.is_int_value()
            && lhs.into_int_value().get_type().get_bit_width() == 1
        {
            return self.compile_short_circuit(op, lhs.into_int_value(), right, span);
        }
        let rhs = self.compile_expr(right)?;
        if let Some(result) = self.pointer_arithmetic(op, lhs, rhs, left, span)? {
            return Ok(result);
        }
        // `c + 'x'`, `#3 + Title`: adding characters builds a string.
        let is_char_value = |v: &BasicValueEnum| {
            v.is_int_value() && v.into_int_value().get_type().get_bit_width() == 8
        };
        let (lhs, rhs) = if op == BinOp::Add
            && (is_char_value(&lhs) || is_char_value(&rhs))
            && (is_char_value(&lhs) || lhs.is_pointer_value())
            && (is_char_value(&rhs) || rhs.is_pointer_value())
        {
            (
                self.as_string_value(lhs, span)?,
                self.as_string_value(rhs, span)?,
            )
        } else {
            (lhs, rhs)
        };
        // A char against a one-character string literal (`c = 'x'`)
        // compares characters.
        let is_char = |v: &BasicValueEnum| {
            v.is_int_value() && v.into_int_value().get_type().get_bit_width() == 8
        };
        let (lhs, rhs) = if is_char(&lhs) && rhs.is_pointer_value() {
            (lhs, self.first_char_if_string(rhs, span)?)
        } else if lhs.is_pointer_value() && is_char(&rhs) && op != BinOp::In {
            (self.first_char_if_string(lhs, span)?, rhs)
        } else {
            (lhs, rhs)
        };

        // Set membership: x in S
        if op == BinOp::In {
            let i64_ty = self.context.i64_type();
            let set_ty = i64_ty.array_type(4);
            let zero = i64_ty.const_int(0, false);
            let sixty_four = i64_ty.const_int(64, false);
            let one_i64 = i64_ty.const_int(1, false);

            // lhs is the ordinal, rhs is the set ([4 x i64])
            let ord = lhs.into_int_value();
            let ord = if ord.get_type().get_bit_width() < 64 {
                self.builder
                    .build_int_z_extend(ord, i64_ty, "ord_ext")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            } else {
                ord
            };

            // Store set to alloca so we can GEP into it
            let set_alloca = self
                .entry_alloca(set_ty, "set_in_tmp")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.builder
                .build_store(set_alloca, rhs)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

            let word_idx = self
                .builder
                .build_int_unsigned_div(ord, sixty_four, "word_idx")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let bit_idx = self
                .builder
                .build_int_unsigned_rem(ord, sixty_four, "bit_idx")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let mask = self
                .builder
                .build_left_shift(one_i64, bit_idx, "mask")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

            let gep = unsafe {
                self.builder
                    .build_in_bounds_gep(set_ty, set_alloca, &[zero, word_idx], "set_word_ptr")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            };
            let word = self
                .builder
                .build_load(i64_ty, gep, "word")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let anded = self
                .builder
                .build_and(word.into_int_value(), mask, "anded")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let result = self
                .builder
                .build_int_compare(inkwell::IntPredicate::NE, anded, zero, "in_result")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            return Ok(result.into());
        }

        // Set binary ops: +, -, *, =, <>, <=, >= on [4 x i64]
        if lhs.is_array_value() && rhs.is_array_value() {
            let i64_ty = self.context.i64_type();
            let set_ty = i64_ty.array_type(4);
            let zero = i64_ty.const_int(0, false);

            let l_alloca = self
                .entry_alloca(set_ty, "set_l")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.builder
                .build_store(l_alloca, lhs)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let r_alloca = self
                .entry_alloca(set_ty, "set_r")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.builder
                .build_store(r_alloca, rhs)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

            // Set comparison operators return a boolean, not a set
            if matches!(op, BinOp::Eq | BinOp::Neq | BinOp::Lte | BinOp::Gte) {
                // Accumulate per-word results: start with true (1)
                let bool_ty = self.context.bool_type();
                let mut acc = bool_ty.const_int(1, false);
                for i in 0..4u64 {
                    let idx = i64_ty.const_int(i, false);
                    let l_gep = unsafe {
                        self.builder
                            .build_in_bounds_gep(set_ty, l_alloca, &[zero, idx], "lg")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    };
                    let r_gep = unsafe {
                        self.builder
                            .build_in_bounds_gep(set_ty, r_alloca, &[zero, idx], "rg")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    };
                    let lw = self
                        .builder
                        .build_load(i64_ty, l_gep, "lw")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .into_int_value();
                    let rw = self
                        .builder
                        .build_load(i64_ty, r_gep, "rw")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .into_int_value();

                    let word_ok = match op {
                        BinOp::Eq | BinOp::Neq => {
                            // Per-word equality
                            self.builder
                                .build_int_compare(inkwell::IntPredicate::EQ, lw, rw, "weq")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        }
                        BinOp::Lte => {
                            // Subset: (l & ~r) == 0
                            let not_r = self
                                .builder
                                .build_not(rw, "not_r")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                            let diff = self
                                .builder
                                .build_and(lw, not_r, "diff")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                            self.builder
                                .build_int_compare(inkwell::IntPredicate::EQ, diff, zero, "sub_ok")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        }
                        BinOp::Gte => {
                            // Superset: (r & ~l) == 0
                            let not_l = self
                                .builder
                                .build_not(lw, "not_l")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                            let diff = self
                                .builder
                                .build_and(rw, not_l, "diff")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                            self.builder
                                .build_int_compare(inkwell::IntPredicate::EQ, diff, zero, "sup_ok")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        }
                        _ => unreachable!(),
                    };
                    acc = self
                        .builder
                        .build_and(acc, word_ok, "acc")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                }
                // For <>, negate the final result
                if op == BinOp::Neq {
                    acc = self
                        .builder
                        .build_not(acc, "neq_result")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                }
                return Ok(acc.into());
            }

            // Set arithmetic ops (+, -, *) return a set
            let out_alloca = self
                .entry_alloca(set_ty, "set_out")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

            for i in 0..4u64 {
                let idx = i64_ty.const_int(i, false);
                let l_gep = unsafe {
                    self.builder
                        .build_in_bounds_gep(set_ty, l_alloca, &[zero, idx], "lg")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                let r_gep = unsafe {
                    self.builder
                        .build_in_bounds_gep(set_ty, r_alloca, &[zero, idx], "rg")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                let o_gep = unsafe {
                    self.builder
                        .build_in_bounds_gep(set_ty, out_alloca, &[zero, idx], "og")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                let lw = self
                    .builder
                    .build_load(i64_ty, l_gep, "lw")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into_int_value();
                let rw = self
                    .builder
                    .build_load(i64_ty, r_gep, "rw")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into_int_value();

                let result_word = match op {
                    BinOp::Add => {
                        // Union: OR
                        self.builder
                            .build_or(lw, rw, "union")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                    BinOp::Sub => {
                        // Difference: AND(l, NOT(r))
                        let not_r = self
                            .builder
                            .build_not(rw, "not_r")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                        self.builder
                            .build_and(lw, not_r, "diff")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                    BinOp::Mul => {
                        // Intersection: AND
                        self.builder
                            .build_and(lw, rw, "isect")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                    _ => {
                        return Err(CodeGenError::new(
                            "unsupported operator for set type",
                            Some(span),
                        ));
                    }
                };
                self.builder
                    .build_store(o_gep, result_word)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            }

            let result = self
                .builder
                .build_load(set_ty, out_alloca, "set_result")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            return Ok(result);
        }

        // Real division (/) always uses float even for integer operands
        if op == BinOp::RealDiv {
            let f64_ty = self.context.f64_type();
            let promote = |val: BasicValueEnum<'ctx>,
                           b: &Builder<'ctx>|
             -> Result<inkwell::values::FloatValue<'ctx>, CodeGenError> {
                if val.is_float_value() {
                    Ok(val.into_float_value())
                } else if val.is_int_value() {
                    b.build_signed_int_to_float(val.into_int_value(), f64_ty, "itof")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))
                } else {
                    Err(CodeGenError::new("cannot convert to real", Some(span)))
                }
            };
            let l = promote(lhs, &self.builder)?;
            let r = promote(rhs, &self.builder)?;
            let result = self
                .builder
                .build_float_div(l, r, "rdiv")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            return Ok(result.into());
        }

        // Integer arithmetic
        if lhs.is_int_value() && rhs.is_int_value() {
            let l = lhs.into_int_value();
            let r = rhs.into_int_value();

            // For comparison operators on booleans, extend to i64 first
            let (l, r) = if l.get_type().get_bit_width() != r.get_type().get_bit_width() {
                let l64 = self
                    .builder
                    .build_int_z_extend(l, self.context.i64_type(), "zext_l")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                let r64 = self
                    .builder
                    .build_int_z_extend(r, self.context.i64_type(), "zext_r")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                (l64, r64)
            } else {
                (l, r)
            };

            let result = match op {
                BinOp::Add => {
                    if self.directives.overflow_check && l.get_type().get_bit_width() == 64 {
                        self.emit_checked_arith("llvm.sadd.with.overflow.i64", l, r, span)?
                    } else {
                        self.builder
                            .build_int_add(l, r, "add")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                }
                BinOp::Sub => {
                    if self.directives.overflow_check && l.get_type().get_bit_width() == 64 {
                        self.emit_checked_arith("llvm.ssub.with.overflow.i64", l, r, span)?
                    } else {
                        self.builder
                            .build_int_sub(l, r, "sub")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                }
                BinOp::Mul => {
                    if self.directives.overflow_check && l.get_type().get_bit_width() == 64 {
                        self.emit_checked_arith("llvm.smul.with.overflow.i64", l, r, span)?
                    } else {
                        self.builder
                            .build_int_mul(l, r, "mul")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                }
                BinOp::Div => self
                    .builder
                    .build_int_signed_div(l, r, "div")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Mod => self
                    .builder
                    .build_int_signed_rem(l, r, "mod_")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Eq => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::EQ, l, r, "eq")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Neq => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, l, r, "neq")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Lt => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::SLT, l, r, "lt")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Gt => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::SGT, l, r, "gt")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Lte => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::SLE, l, r, "lte")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Gte => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::SGE, l, r, "gte")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::And => self
                    .builder
                    .build_and(l, r, "and")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Or => self
                    .builder
                    .build_or(l, r, "or")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Xor => self
                    .builder
                    .build_xor(l, r, "xor")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Shl => self
                    .builder
                    .build_left_shift(l, r, "shl")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Shr => self
                    .builder
                    .build_right_shift(l, r, false, "shr")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::RealDiv | BinOp::In => unreachable!("handled above"),
            };

            return Ok(result.into());
        }

        // Float arithmetic (including int-to-float promotion)
        if lhs.is_float_value() || rhs.is_float_value() {
            let f64_ty = self.context.f64_type();
            let promote = |val: BasicValueEnum<'ctx>,
                           b: &Builder<'ctx>|
             -> Result<inkwell::values::FloatValue<'ctx>, CodeGenError> {
                if val.is_float_value() {
                    Ok(val.into_float_value())
                } else if val.is_int_value() {
                    b.build_signed_int_to_float(val.into_int_value(), f64_ty, "itof")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))
                } else {
                    Err(CodeGenError::new("cannot convert to real", Some(span)))
                }
            };
            let l = promote(lhs, &self.builder)?;
            let r = promote(rhs, &self.builder)?;

            let result: BasicValueEnum = match op {
                BinOp::Add => self
                    .builder
                    .build_float_add(l, r, "fadd")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into(),
                BinOp::Sub => self
                    .builder
                    .build_float_sub(l, r, "fsub")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into(),
                BinOp::Mul => self
                    .builder
                    .build_float_mul(l, r, "fmul")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into(),
                BinOp::Div | BinOp::RealDiv => self
                    .builder
                    .build_float_div(l, r, "fdiv")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into(),
                BinOp::Eq => self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OEQ, l, r, "feq")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into(),
                BinOp::Neq => self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::ONE, l, r, "fne")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into(),
                BinOp::Lt => self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OLT, l, r, "flt")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into(),
                BinOp::Gt => self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OGT, l, r, "fgt")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into(),
                BinOp::Lte => self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OLE, l, r, "fle")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into(),
                BinOp::Gte => self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OGE, l, r, "fge")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into(),
                _ => {
                    return Err(CodeGenError::new(
                        "unsupported operator for real type",
                        Some(span),
                    ));
                }
            };
            return Ok(result);
        }

        // String operations
        if lhs.is_pointer_value() && rhs.is_pointer_value() {
            let l = lhs.into_pointer_value();
            let r = rhs.into_pointer_value();
            // If either side is nil or a pointer (not string), compare addresses directly.
            let lt = self.infer_expr_type(left);
            let rt = self.infer_expr_type(right);
            let is_ptr_cmp = matches!(op, BinOp::Eq | BinOp::Neq)
                && (matches!(lt, PascalType::Pointer(_)) || matches!(rt, PascalType::Pointer(_)));
            if is_ptr_cmp {
                let i64_ty = self.context.i64_type();
                let li = self
                    .builder
                    .build_ptr_to_int(l, i64_ty, "li")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                let ri = self
                    .builder
                    .build_ptr_to_int(r, i64_ty, "ri")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                let pred = if op == BinOp::Eq {
                    inkwell::IntPredicate::EQ
                } else {
                    inkwell::IntPredicate::NE
                };
                let result = self
                    .builder
                    .build_int_compare(pred, li, ri, "pcmp")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                return Ok(result.into());
            }
            match op {
                BinOp::Add => {
                    let concat = self.module.get_function("bruto_str_concat").unwrap();
                    let result = self
                        .builder
                        .build_call(concat, &[l.into(), r.into()], "concat")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    return Ok(result);
                }
                BinOp::Eq | BinOp::Neq | BinOp::Lt | BinOp::Gt | BinOp::Lte | BinOp::Gte => {
                    let cmp = self.module.get_function("bruto_str_compare").unwrap();
                    let cmp_result = self
                        .builder
                        .build_call(cmp, &[l.into(), r.into()], "strcmp")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .try_as_basic_value()
                        .basic()
                        .unwrap()
                        .into_int_value();
                    let zero = self.context.i32_type().const_int(0, false);
                    let pred = match op {
                        BinOp::Eq => inkwell::IntPredicate::EQ,
                        BinOp::Neq => inkwell::IntPredicate::NE,
                        BinOp::Lt => inkwell::IntPredicate::SLT,
                        BinOp::Gt => inkwell::IntPredicate::SGT,
                        BinOp::Lte => inkwell::IntPredicate::SLE,
                        BinOp::Gte => inkwell::IntPredicate::SGE,
                        _ => unreachable!(),
                    };
                    let result = self
                        .builder
                        .build_int_compare(pred, cmp_result, zero, "scmp")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    return Ok(result.into());
                }
                _ => {}
            }
        }

        // Records compare field by field.
        if matches!(op, BinOp::Eq | BinOp::Neq) && lhs.is_struct_value() && rhs.is_struct_value() {
            let err =
                |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
            let equal = self.aggregate_equal(lhs, rhs).map_err(err)?;
            return Ok(if op == BinOp::Eq {
                equal.into()
            } else {
                self.builder.build_not(equal, "ne").map_err(err)?.into()
            });
        }

        Err(CodeGenError::new(
            "unsupported operand types for binary operator",
            Some(span),
        ))
    }

    /// Whether two values of the same record (or array) type are equal,
    /// element by element.
    fn aggregate_equal(
        &self,
        a: BasicValueEnum<'ctx>,
        b: BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, inkwell::builder::BuilderError> {
        let bld = &self.builder;
        if a.is_int_value() {
            return bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                a.into_int_value(),
                b.into_int_value(),
                "field_eq",
            );
        }
        if a.is_float_value() {
            return bld.build_float_compare(
                inkwell::FloatPredicate::OEQ,
                a.into_float_value(),
                b.into_float_value(),
                "field_eq",
            );
        }
        if a.is_pointer_value() {
            let i64_ty = self.context.i64_type();
            let x = bld.build_ptr_to_int(a.into_pointer_value(), i64_ty, "pa")?;
            let y = bld.build_ptr_to_int(b.into_pointer_value(), i64_ty, "pb")?;
            return bld.build_int_compare(inkwell::IntPredicate::EQ, x, y, "field_eq");
        }
        let count = if a.is_struct_value() {
            a.into_struct_value().get_type().count_fields()
        } else if a.is_array_value() {
            a.into_array_value().get_type().len()
        } else {
            0
        };
        let mut all = self.context.bool_type().const_int(1, false);
        for i in 0..count {
            let (x, y) = if a.is_struct_value() {
                (
                    bld.build_extract_value(a.into_struct_value(), i, "fa")?,
                    bld.build_extract_value(b.into_struct_value(), i, "fb")?,
                )
            } else {
                (
                    bld.build_extract_value(a.into_array_value(), i, "ea")?,
                    bld.build_extract_value(b.into_array_value(), i, "eb")?,
                )
            };
            let eq = self.aggregate_equal(x, y)?;
            all = bld.build_and(all, eq, "all_eq")?;
        }
        Ok(all)
    }

    fn compile_unaryop(
        &mut self,
        op: UnaryOp,
        operand: &Expr,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let val = self.compile_expr(operand)?;

        match op {
            UnaryOp::Neg => {
                if val.is_int_value() {
                    let iv = val.into_int_value();
                    let result = self
                        .builder
                        .build_int_neg(iv, "neg")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    Ok(result.into())
                } else if val.is_float_value() {
                    let result = self
                        .builder
                        .build_float_neg(val.into_float_value(), "fneg")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    Ok(result.into())
                } else {
                    Err(CodeGenError::new("cannot negate a non-number", Some(span)))
                }
            }
            UnaryOp::Not => {
                if val.is_int_value() {
                    let iv = val.into_int_value();
                    let result = self
                        .builder
                        .build_not(iv, "not")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    Ok(result.into())
                } else {
                    Err(CodeGenError::new(
                        "cannot apply 'not' to non-integer/boolean",
                        Some(span),
                    ))
                }
            }
        }
    }

    // ── type inference (for write formatting) ────────────

    /// Compute the address of `base[idx]` for either a fixed-size Array or a
    /// ConformantArray. Returns the element pointer.
    fn gep_array_elem(
        &mut self,
        base_ptr: PointerValue<'ctx>,
        base_type: &PascalType,
        idx: inkwell::values::IntValue<'ctx>,
        span: Span,
    ) -> Result<(PointerValue<'ctx>, PascalType), CodeGenError> {
        // A char or boolean index is compared and offset in 64 bits.
        let idx = if idx.get_type().get_bit_width() < 64 {
            self.builder
                .build_int_z_extend(idx, self.context.i64_type(), "idx64")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        } else {
            idx
        };
        match base_type {
            PascalType::Array { lo, hi, elem } => {
                self.emit_range_check(idx, *lo, *hi, span)?;
                let adj = self
                    .builder
                    .build_int_sub(
                        idx,
                        self.context.i64_type().const_int(*lo as u64, true),
                        "adj_idx",
                    )
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                let gep = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.llvm_type_for(base_type),
                            base_ptr,
                            &[self.context.i64_type().const_int(0, false), adj],
                            "arr_gep",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                Ok((gep, elem.as_ref().clone()))
            }
            PascalType::ConformantArray { lo_name, elem, .. } => {
                // Load runtime lo from the sibling alloca.
                let lo_alloca = *self.lookup_var(lo_name.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("conformant lo '{lo_name}' missing"), Some(span))
                })?;
                let lo = self
                    .builder
                    .build_load(self.context.i64_type(), lo_alloca, "clo")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into_int_value();
                let adj = self
                    .builder
                    .build_int_sub(idx, lo, "cadj")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                let elem_size = self.sizeof_type(elem);
                let off = self
                    .builder
                    .build_int_mul(
                        adj,
                        self.context.i64_type().const_int(elem_size, false),
                        "coff",
                    )
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                let gep = unsafe {
                    self.builder
                        .build_in_bounds_gep(self.context.i8_type(), base_ptr, &[off], "carr_gep")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                Ok((gep, elem.as_ref().clone()))
            }
            // `s[i]` on a short string: byte i of its buffer (byte 0 is the
            // length).
            PascalType::ShortString { .. } => {
                let gep = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.llvm_type_for(base_type),
                            base_ptr,
                            &[self.context.i64_type().const_zero(), idx],
                            "short_char",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                Ok((gep, PascalType::Char))
            }
            // `p[i]` on a typed pointer (`PChar`, `PByte`...): the i-th
            // element after where it points. `base_ptr` holds the pointer.
            PascalType::Pointer(pointed) if **pointed != PascalType::Untyped => {
                let pointed = self.resolve_type(pointed);
                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                let target = self
                    .builder
                    .build_load(ptr_ty, base_ptr, "ptr_base")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into_pointer_value();
                let gep = unsafe {
                    self.builder
                        .build_gep(self.llvm_type_for(&pointed), target, &[idx], "ptr_index")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                Ok((gep, pointed))
            }
            // `s[i]` on a dynamic string, as a place (`Move(s[1], ...)`):
            // character i of the buffer the string points at.
            PascalType::String => {
                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                let chars = self
                    .builder
                    .build_load(ptr_ty, base_ptr, "str_base")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into_pointer_value();
                let offset = self
                    .builder
                    .build_int_sub(idx, self.context.i64_type().const_int(1, false), "str_off")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                let gep = unsafe {
                    self.builder
                        .build_gep(self.context.i8_type(), chars, &[offset], "str_char")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                Ok((gep, PascalType::Char))
            }
            _ => Err(CodeGenError::new("indexing non-array", Some(span))),
        }
    }

    /// Emit `llvm.sadd/ssub/smul.with.overflow.i64` and abort on overflow.
    fn emit_checked_arith(
        &mut self,
        intrinsic: &str,
        l: inkwell::values::IntValue<'ctx>,
        r: inkwell::values::IntValue<'ctx>,
        span: Span,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodeGenError> {
        let i64_ty = self.context.i64_type();
        let i1_ty = self.context.bool_type();
        let result_ty = self
            .context
            .struct_type(&[i64_ty.into(), i1_ty.into()], false);
        let func = self.module.get_function(intrinsic).unwrap_or_else(|| {
            let ft = result_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false);
            self.module.add_function(intrinsic, ft, None)
        });
        let call = self
            .builder
            .build_call(func, &[l.into(), r.into()], "ovf_pair")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let agg = call
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let val = self
            .builder
            .build_extract_value(agg, 0, "ovf_val")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .into_int_value();
        let flag = self
            .builder
            .build_extract_value(agg, 1, "ovf_flag")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .into_int_value();
        let cur_fn = self.current_fn.unwrap();
        let fail_bb = self.context.append_basic_block(cur_fn, "ovf_fail");
        let ok_bb = self.context.append_basic_block(cur_fn, "ovf_ok");
        self.builder
            .build_conditional_branch(flag, fail_bb, ok_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder.position_at_end(fail_bb);
        let f = self.module.get_function("bruto_overflow_fail").unwrap();
        self.builder
            .build_call(f, &[i64_ty.const_int(span.line as u64, false).into()], "")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder
            .build_unreachable()
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder.position_at_end(ok_bb);
        Ok(val)
    }

    /// If `{$R+}` is enabled, emit a check that `idx` is in `[lo, hi]` and abort otherwise.
    fn emit_range_check(
        &mut self,
        idx: inkwell::values::IntValue<'ctx>,
        lo: i64,
        hi: i64,
        span: Span,
    ) -> Result<(), CodeGenError> {
        if !self.directives.range_check {
            return Ok(());
        }
        let i64_ty = self.context.i64_type();
        let lo_v = i64_ty.const_int(lo as u64, true);
        let hi_v = i64_ty.const_int(hi as u64, true);
        // Promote idx to i64 if needed.
        let idx = if idx.get_type().get_bit_width() < 64 {
            self.builder
                .build_int_s_extend(idx, i64_ty, "rngx")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        } else {
            idx
        };
        let lt = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SLT, idx, lo_v, "rng_lt")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let gt = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SGT, idx, hi_v, "rng_gt")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let bad = self
            .builder
            .build_or(lt, gt, "rng_bad")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let func = self.current_fn.unwrap();
        let fail_bb = self.context.append_basic_block(func, "rng_fail");
        let ok_bb = self.context.append_basic_block(func, "rng_ok");
        self.builder
            .build_conditional_branch(bad, fail_bb, ok_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder.position_at_end(fail_bb);
        let f = self.module.get_function("bruto_range_check_fail").unwrap();
        self.builder
            .build_call(
                f,
                &[
                    i64_ty.const_int(span.line as u64, false).into(),
                    lo_v.into(),
                    hi_v.into(),
                    idx.into(),
                ],
                "",
            )
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder
            .build_unreachable()
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder.position_at_end(ok_bb);
        Ok(())
    }

    /// The type `name` denotes when called like a function, `T(x)`, unless
    /// a variable or routine of that name hides it. The four basic types
    /// keep their own cast handling in `compile_type_cast`.
    fn cast_target(&self, name: &str) -> Option<PascalType> {
        if self.lookup_var(name).is_some() || self.module.get_function(name).is_some() {
            return None;
        }
        self.type_defs
            .get(name)
            .map(|t| self.resolve_type(t))
            .or_else(|| crate::sema::builtin_type(name))
    }

    /// Convert `val` to `target` as a Pascal value cast: integers and
    /// pointers convert into one another, integers narrow to the target's
    /// width (`Byte(300)` is 44), pointers are reinterpreted.
    fn compile_cast_to(
        &mut self,
        target: &PascalType,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i64_ty = self.context.i64_type();
        match target {
            PascalType::Pointer(_) | PascalType::Proc { .. } => {
                if val.is_pointer_value() {
                    Ok(val)
                } else if val.is_int_value() {
                    let iv = self.to_i64(val, span)?;
                    let ptr_ty = self.context.ptr_type(AddressSpace::default());
                    Ok(self
                        .builder
                        .build_int_to_ptr(iv, ptr_ty, "to_ptr")
                        .map_err(err)?
                        .into())
                } else {
                    Err(CodeGenError::new(
                        "cannot cast this value to a pointer",
                        Some(span),
                    ))
                }
            }
            PascalType::Integer
            | PascalType::SizedInt { .. }
            | PascalType::Enum { .. }
            | PascalType::Subrange { .. } => {
                let wide: BasicValueEnum = if val.is_pointer_value() {
                    self.builder
                        .build_ptr_to_int(val.into_pointer_value(), i64_ty, "from_ptr")
                        .map_err(err)?
                        .into()
                } else if val.is_float_value() {
                    self.builder
                        .build_float_to_signed_int(val.into_float_value(), i64_ty, "from_real")
                        .map_err(err)?
                        .into()
                } else if val.is_int_value() {
                    self.to_i64(val, span)?.into()
                } else {
                    return Err(CodeGenError::new(
                        "cannot cast this value to an integer",
                        Some(span),
                    ));
                };
                // Narrow to the target's width and widen back, so the value
                // is what a variable of that type would hold.
                let narrow = self.convert_for_store(wide, target, span)?;
                self.widen_loaded(narrow, target, span)
            }
            PascalType::Char => self.compile_type_cast("char", val, span),
            PascalType::Boolean => self.compile_type_cast("boolean", val, span),
            PascalType::Real => self.compile_type_cast("real", val, span),
            // `AnsiString(s)`, `ShortString(s)`: strings of every kind are
            // C strings in expressions.
            PascalType::String | PascalType::ShortString { .. } if val.is_pointer_value() => {
                Ok(val)
            }
            _ => Err(CodeGenError::new(
                "this kind of type cast is not supported yet",
                Some(span),
            )),
        }
    }

    fn compile_type_cast(
        &self,
        name: &str,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let i64_ty = self.context.i64_type();
        let i8_ty = self.context.i8_type();
        let f64_ty = self.context.f64_type();
        let bool_ty = self.context.bool_type();
        match name {
            "integer" => {
                if val.is_int_value() {
                    let iv = val.into_int_value();
                    if iv.get_type().get_bit_width() < 64 {
                        Ok(self
                            .builder
                            .build_int_z_extend(iv, i64_ty, "to_int")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                            .into())
                    } else {
                        Ok(iv.into())
                    }
                } else if val.is_float_value() {
                    Ok(self
                        .builder
                        .build_float_to_signed_int(val.into_float_value(), i64_ty, "to_int")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .into())
                } else {
                    Err(CodeGenError::new("cannot cast to integer", Some(span)))
                }
            }
            "real" => {
                if val.is_float_value() {
                    Ok(val)
                } else if val.is_int_value() {
                    Ok(self
                        .builder
                        .build_signed_int_to_float(val.into_int_value(), f64_ty, "to_real")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .into())
                } else {
                    Err(CodeGenError::new("cannot cast to real", Some(span)))
                }
            }
            "char" => {
                if val.is_int_value() {
                    let iv = val.into_int_value();
                    if iv.get_type().get_bit_width() == 8 {
                        Ok(iv.into())
                    } else {
                        Ok(self
                            .builder
                            .build_int_truncate(iv, i8_ty, "to_char")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                            .into())
                    }
                } else {
                    Err(CodeGenError::new("cannot cast to char", Some(span)))
                }
            }
            "boolean" => {
                if val.is_int_value() {
                    let iv = val.into_int_value();
                    if iv.get_type().get_bit_width() == 1 {
                        Ok(iv.into())
                    } else {
                        Ok(self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                iv,
                                iv.get_type().const_int(0, false),
                                "to_bool",
                            )
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                            .into())
                    }
                } else {
                    Err(CodeGenError::new("cannot cast to boolean", Some(span)))
                }
            }
            _ => Err(CodeGenError::new(
                format!("unknown cast: {name}"),
                Some(span),
            )),
        }
        .inspect(|_| {
            let _ = bool_ty;
        })
    }

    fn promote_to_f64(
        &self,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<inkwell::values::FloatValue<'ctx>, CodeGenError> {
        if val.is_float_value() {
            Ok(val.into_float_value())
        } else if val.is_int_value() {
            self.builder
                .build_signed_int_to_float(val.into_int_value(), self.context.f64_type(), "itof")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))
        } else {
            Err(CodeGenError::new(
                "expected numeric value".to_string(),
                Some(span),
            ))
        }
    }

    fn get_or_declare_f64_intrinsic(&self, name: &str) -> FunctionValue<'ctx> {
        self.module.get_function(name).unwrap_or_else(|| {
            let ft = self
                .context
                .f64_type()
                .fn_type(&[self.context.f64_type().into()], false);
            self.module.add_function(name, ft, None)
        })
    }

    /// The type of an expression's *value*. Narrow integers are widened to
    /// 64 bits when loaded, so they report as `integer` here.
    fn infer_expr_type(&self, expr: &Expr) -> PascalType {
        match self.infer_declared_type(expr) {
            PascalType::SizedInt { .. } => PascalType::Integer,
            PascalType::ShortString { .. } => PascalType::String,
            PascalType::Named(n) => match self.resolve_type(&PascalType::Named(n.clone())) {
                PascalType::SizedInt { .. } => PascalType::Integer,
                PascalType::ShortString { .. } => PascalType::String,
                _ => PascalType::Named(n),
            },
            other => other,
        }
    }

    /// The declared type of what an expression names, before widening.
    fn infer_declared_type(&self, expr: &Expr) -> PascalType {
        match expr {
            Expr::Call { name, args, span } if name.starts_with("system.") => self
                .infer_declared_type(&Expr::Call {
                    name: name["system.".len()..].to_string(),
                    args: args.clone(),
                    span: *span,
                }),
            Expr::Var(name, span) if name.starts_with("system.") => {
                self.infer_declared_type(&Expr::Var(name["system.".len()..].to_string(), *span))
            }
            Expr::IndirectCall { callee, .. } => {
                match self.resolve_type(&self.infer_declared_type(callee)) {
                    PascalType::Proc {
                        return_type: Some(rt),
                        ..
                    } => *rt,
                    _ => PascalType::Integer,
                }
            }
            Expr::RecordInit { .. } | Expr::ArrayInit { .. } => PascalType::Integer,
            Expr::MethodCall { object, method, .. }
                if self.ancestor_constructor(object, method) =>
            {
                PascalType::Boolean
            }
            Expr::MethodCall { object, method, .. } => {
                let ty = self.infer_declared_type(object);
                self.object_name_of(&ty)
                    .and_then(|o| self.method_result_type(&o, method))
                    .unwrap_or(PascalType::Integer)
            }
            Expr::Inherited { method, .. } if self.inherited_is_constructor(method) => {
                PascalType::Boolean
            }
            Expr::Inherited { method, .. } => self
                .current_object
                .as_ref()
                .and_then(|o| self.objects[o].parent.clone())
                .and_then(|p| self.method_result_type(&p, method))
                .unwrap_or(PascalType::Integer),
            Expr::IntLit(..) => PascalType::Integer,
            Expr::RealLit(..) => PascalType::Real,
            Expr::CharLit(..) => PascalType::Char,
            Expr::StrLit(..) => PascalType::String,
            Expr::BoolLit(..) => PascalType::Boolean,
            Expr::Var(name, _) => self
                .lookup_var_type(name.as_str())
                .cloned()
                .or_else(|| {
                    self.self_method(name)
                        .and_then(|o| self.method_result_type(&o, name))
                })
                .or_else(|| self.proc_return_types.get(name.as_str()).cloned().flatten())
                .or_else(|| self.system_func_type(name, 0))
                .unwrap_or(PascalType::Integer),
            Expr::Nil(_) => PascalType::Pointer(Box::new(PascalType::Integer)),
            Expr::SetConstructor { .. } => PascalType::Set {
                elem: Box::new(PascalType::Integer),
            },
            Expr::BinOp {
                op, left, right, ..
            } => {
                match op {
                    BinOp::Eq
                    | BinOp::Neq
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::Lte
                    | BinOp::Gte
                    | BinOp::In => PascalType::Boolean,
                    // Logical on booleans, bitwise on integers.
                    BinOp::And | BinOp::Or | BinOp::Xor => {
                        if self.resolve_type(&self.infer_expr_type(left)) == PascalType::Boolean {
                            PascalType::Boolean
                        } else {
                            PascalType::Integer
                        }
                    }
                    BinOp::Shl | BinOp::Shr => PascalType::Integer,
                    BinOp::Add | BinOp::Sub
                        if matches!(
                            self.resolve_type(&self.infer_declared_type(left)),
                            PascalType::Pointer(_)
                        ) =>
                    {
                        let lt = self.infer_declared_type(left);
                        let rt = self.resolve_type(&self.infer_declared_type(right));
                        if matches!(rt, PascalType::Pointer(_)) {
                            PascalType::Integer
                        } else {
                            lt
                        }
                    }
                    BinOp::RealDiv => PascalType::Real,
                    _ => {
                        let lt = self.infer_expr_type(left);
                        let rt = self.infer_expr_type(right);
                        // Set operations return a set
                        let (lr, rr) = (self.resolve_type(&lt), self.resolve_type(&rt));
                        let textual =
                            |t: &PascalType| matches!(t, PascalType::String | PascalType::Char);
                        if matches!(lt, PascalType::Set { .. })
                            || matches!(rt, PascalType::Set { .. })
                        {
                            PascalType::Set {
                                elem: Box::new(PascalType::Integer),
                            }
                        } else if *op == BinOp::Add && textual(&lr) && textual(&rr) {
                            // Concatenation, characters included.
                            PascalType::String
                        } else if lt == PascalType::Real || rt == PascalType::Real {
                            PascalType::Real
                        } else {
                            PascalType::Integer
                        }
                    }
                }
            }
            Expr::UnaryOp { op, operand, .. } => {
                let operand = self.resolve_type(&self.infer_expr_type(operand));
                match op {
                    UnaryOp::Neg if operand == PascalType::Real => PascalType::Real,
                    UnaryOp::Neg => PascalType::Integer,
                    UnaryOp::Not if operand == PascalType::Boolean => PascalType::Boolean,
                    UnaryOp::Not => PascalType::Integer,
                }
            }
            Expr::Call { name, args, .. } => {
                let name = &self.resolve_overload(name, args);
                if let Some(object) = self.self_method(name) {
                    return self
                        .method_result_type(&object, name)
                        .unwrap_or(PascalType::Integer);
                }
                if name == "new"
                    && args.len() == 2
                    && let Expr::Var(ptype, _) = &args[0]
                    && let Some(target) = self.cast_target(ptype)
                {
                    return target;
                }
                if name == "typeof" {
                    return PascalType::Pointer(Box::new(PascalType::Untyped));
                }
                if name == "string" && args.len() == 1 {
                    return PascalType::String;
                }
                if let Some(ty) = self.system_func_type(name, args.len()) {
                    return ty;
                }
                if args.len() == 1
                    && let Some(target) = self.cast_target(name)
                {
                    return target;
                }
                // Built-in functions
                match name.as_str() {
                    "assigned" => PascalType::Boolean,
                    "integer" => PascalType::Integer,
                    "real" => PascalType::Real,
                    "char" => PascalType::Char,
                    "boolean" => PascalType::Boolean,
                    "length" | "ord" | "trunc" | "round" | "pos" | "ioresult" | "filepos"
                    | "filesize" => PascalType::Integer,
                    "eof" | "eoln" | "odd" => PascalType::Boolean,
                    "chr" | "upcase" => PascalType::Char,
                    "copy" | "concat" => PascalType::String,
                    "abs" | "sqr" => {
                        if !args.is_empty() {
                            self.infer_expr_type(&args[0])
                        } else {
                            PascalType::Integer
                        }
                    }
                    "succ" | "pred" => {
                        if !args.is_empty() {
                            self.infer_expr_type(&args[0])
                        } else {
                            PascalType::Integer
                        }
                    }
                    "low" | "high" | "sizeof" => PascalType::Integer,
                    "sqrt" | "sin" | "cos" | "arctan" | "exp" | "ln" => PascalType::Real,
                    _ => self
                        .proc_return_types
                        .get(name.as_str())
                        .and_then(|rt| rt.clone())
                        .unwrap_or(PascalType::Integer),
                }
            }
            Expr::Index { array, .. } => {
                let arr_ty = self.infer_expr_type(array);
                match self.resolve_type(&arr_ty) {
                    PascalType::Array { elem, .. } | PascalType::ConformantArray { elem, .. } => {
                        *elem
                    }
                    PascalType::String | PascalType::ShortString { .. } => PascalType::Char,
                    PascalType::Pointer(elem) if *elem != PascalType::Untyped => *elem,
                    _ => PascalType::Integer,
                }
            }
            Expr::FieldAccess { record, field, .. }
                if self.is_parameterless_method(record, field) =>
            {
                let ty = self.infer_declared_type(record);
                self.object_name_of(&ty)
                    .and_then(|o| self.method_result_type(&o, field))
                    .unwrap_or(PascalType::Integer)
            }
            Expr::FieldAccess { record, field, .. } => {
                let rec_ty = self.infer_expr_type(record);
                let resolved = self.resolve_type(&rec_ty);
                match &resolved {
                    PascalType::Record {
                        fields, variant, ..
                    } => {
                        if let Some((_, t)) = fields.iter().find(|(n, _)| n == field) {
                            t.clone()
                        } else if let Some(v) = variant {
                            if field == &v.tag_name {
                                v.tag_type.clone()
                            } else {
                                self.find_variant_field(v, field)
                                    .map(|(_, t)| t)
                                    .unwrap_or(PascalType::Integer)
                            }
                        } else {
                            PascalType::Integer
                        }
                    }
                    _ => PascalType::Integer,
                }
            }
            Expr::Deref(inner, _) => {
                let inner_type = self.infer_expr_type(inner);
                let resolved = self.resolve_type(&inner_type);
                match resolved {
                    PascalType::Pointer(pointed) => *pointed,
                    PascalType::File { elem } => *elem,
                    _ => PascalType::Integer,
                }
            }
            Expr::AddrOf(inner, _) => {
                if self.routine_name(inner).is_some() {
                    PascalType::Pointer(Box::new(PascalType::Untyped))
                } else {
                    PascalType::Pointer(Box::new(self.infer_declared_type(inner)))
                }
            }
        }
    }

    /// What calling method `method` of object type `object` yields: its
    /// result type, or a pointer to the object for a constructor.
    fn method_result_type(&self, object: &str, method: &str) -> Option<PascalType> {
        let m = self.objects.get(object)?.methods.get(method)?;
        match m.kind {
            RoutineKind::Constructor => Some(PascalType::Pointer(Box::new(PascalType::Named(
                object.to_string(),
            )))),
            _ => m.return_type.clone(),
        }
    }

    /// Whether `record.field` names a method of an object rather than a
    /// field, so it is a call with no arguments.
    fn is_parameterless_method(&self, record: &Expr, field: &str) -> bool {
        let ty = self.infer_declared_type(record);
        let Some(object) = self.object_name_of(&ty) else {
            return false;
        };
        let info = &self.objects[&object];
        let is_field = matches!(&info.record, PascalType::Record { fields, .. }
            if fields.iter().any(|(n, _)| n == field));
        !is_field && info.methods.contains_key(field)
    }

    /// The routine a call to `name` with `args` means: `name` itself, or
    /// for an overloaded name, the overload whose parameters best fit the
    /// argument types.
    fn resolve_overload(&self, name: &str, args: &[Expr]) -> String {
        if let Some(c_symbol) = self.external_symbols.get(name) {
            return c_symbol.clone();
        }
        let Some(candidates) = self.overloads.get(name) else {
            return name.to_string();
        };
        let score = |symbol: &String| -> Option<i32> {
            let params = self.proc_param_modes.get(symbol)?;
            if params.len() != args.len() {
                return None;
            }
            let mut total = 0;
            for ((_, mode, param), arg) in params.iter().zip(args) {
                let param = self.resolve_type(param);
                let arg_ty = self.resolve_type(&self.infer_declared_type(arg));
                total += if param == arg_ty {
                    3
                } else if *mode == ParamMode::Var && param == PascalType::Untyped {
                    1
                } else {
                    match (&param, &arg_ty) {
                        (
                            PascalType::Integer | PascalType::SizedInt { .. },
                            PascalType::Integer | PascalType::SizedInt { .. },
                        )
                        | (PascalType::Real, PascalType::Integer | PascalType::SizedInt { .. })
                        | (
                            PascalType::String | PascalType::ShortString { .. },
                            PascalType::String | PascalType::ShortString { .. },
                        ) => 2,
                        (
                            PascalType::Pointer(_) | PascalType::Proc { .. },
                            PascalType::Pointer(_),
                        ) => 1,
                        (PascalType::Char, PascalType::String) => 1,
                        _ => return None,
                    }
                };
            }
            Some(total)
        };
        candidates
            .iter()
            .filter_map(|s| score(s).map(|n| (n, s)))
            .max_by_key(|(n, _)| *n)
            .map(|(_, s)| s.clone())
            .unwrap_or_else(|| name.to_string())
    }

    /// If `expr` is the bare name of a procedure or function (not shadowed
    /// by a variable), that routine.
    fn routine_name(&self, expr: &Expr) -> Option<FunctionValue<'ctx>> {
        match expr {
            Expr::Var(name, _) if self.lookup_var(name).is_none() => self.module.get_function(name),
            _ => None,
        }
    }
}

/// Short type tag for watch-window metadata.
fn type_short(t: &PascalType) -> &'static str {
    match t {
        PascalType::Integer
        | PascalType::SizedInt { .. }
        | PascalType::Subrange { .. }
        | PascalType::Enum { .. } => "int",
        PascalType::Real => "real",
        PascalType::Boolean => "bool",
        PascalType::Char => "char",
        PascalType::String => "str",
        PascalType::Pointer(_) => "ptr",
        PascalType::Array { .. } => "arr",
        PascalType::Record { .. } => "rec",
        PascalType::Set { .. } => "set",
        PascalType::File { .. } => "file",
        _ => "?",
    }
}

/// Walk a block looking for variable references that escape `locals` and exist
/// in the enclosing `parent_types` map. Each capture is recorded once.
fn collect_captures(
    block: &Block,
    locals: &std::collections::HashSet<String>,
    parent_types: &HashMap<String, PascalType>,
    out: &mut Vec<(String, PascalType)>,
    seen: &mut std::collections::HashSet<String>,
) {
    for stmt in &block.statements {
        collect_captures_stmt(stmt, locals, parent_types, out, seen);
    }
}

fn collect_captures_stmt(
    stmt: &Statement,
    locals: &std::collections::HashSet<String>,
    parent_types: &HashMap<String, PascalType>,
    out: &mut Vec<(String, PascalType)>,
    seen: &mut std::collections::HashSet<String>,
) {
    let record = |name: &str,
                  out: &mut Vec<(String, PascalType)>,
                  seen: &mut std::collections::HashSet<String>| {
        if !locals.contains(name) && parent_types.contains_key(name) && !seen.contains(name) {
            seen.insert(name.to_string());
            out.push((
                name.to_string(),
                parent_types
                    .get(name)
                    .cloned()
                    .unwrap_or(PascalType::Integer),
            ));
        }
    };
    match stmt {
        Statement::Assignment { target, expr, .. }
        | Statement::DerefAssignment { target, expr, .. } => {
            record(target, out, seen);
            collect_captures_expr(expr, locals, parent_types, out, seen);
        }
        Statement::IndexAssignment {
            target,
            index,
            expr,
            ..
        } => {
            record(target, out, seen);
            collect_captures_expr(index, locals, parent_types, out, seen);
            collect_captures_expr(expr, locals, parent_types, out, seen);
        }
        Statement::MultiIndexAssignment {
            target,
            indices,
            expr,
            ..
        } => {
            record(target, out, seen);
            for i in indices {
                collect_captures_expr(i, locals, parent_types, out, seen);
            }
            collect_captures_expr(expr, locals, parent_types, out, seen);
        }
        Statement::FieldAssignment { target, expr, .. } => {
            record(target, out, seen);
            collect_captures_expr(expr, locals, parent_types, out, seen);
        }
        Statement::ChainedAssignment {
            target,
            chain,
            expr,
            ..
        } => {
            record(target, out, seen);
            for a in chain {
                if let LValueAccess::Index(e) = a {
                    collect_captures_expr(e, locals, parent_types, out, seen);
                }
            }
            collect_captures_expr(expr, locals, parent_types, out, seen);
        }
        Statement::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            collect_captures_expr(condition, locals, parent_types, out, seen);
            collect_captures(then_branch, locals, parent_types, out, seen);
            if let Some(eb) = else_branch {
                collect_captures(eb, locals, parent_types, out, seen);
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            collect_captures_expr(condition, locals, parent_types, out, seen);
            collect_captures(body, locals, parent_types, out, seen);
        }
        Statement::For {
            var,
            from,
            to,
            body,
            ..
        } => {
            record(var, out, seen);
            collect_captures_expr(from, locals, parent_types, out, seen);
            collect_captures_expr(to, locals, parent_types, out, seen);
            collect_captures(body, locals, parent_types, out, seen);
        }
        Statement::RepeatUntil {
            body, condition, ..
        } => {
            for s in body {
                collect_captures_stmt(s, locals, parent_types, out, seen);
            }
            collect_captures_expr(condition, locals, parent_types, out, seen);
        }
        Statement::WriteLn { args, .. } | Statement::Write { args, .. } => {
            for a in args {
                collect_captures_expr(&a.expr, locals, parent_types, out, seen);
                if let Some(w) = &a.width {
                    collect_captures_expr(w, locals, parent_types, out, seen);
                }
                if let Some(p) = &a.precision {
                    collect_captures_expr(p, locals, parent_types, out, seen);
                }
            }
        }
        Statement::ReadLn { targets, .. } => {
            for t in targets {
                record(t, out, seen);
            }
        }
        Statement::Block(b) => collect_captures(b, locals, parent_types, out, seen),
        Statement::New {
            target,
            constructor: method,
            ..
        }
        | Statement::Dispose {
            target,
            destructor: method,
            ..
        } => {
            collect_captures_expr(target, locals, parent_types, out, seen);
            if let Some((_, args)) = method {
                for a in args {
                    collect_captures_expr(a, locals, parent_types, out, seen);
                }
            }
        }
        Statement::MethodCall { object, args, .. } => {
            collect_captures_expr(object, locals, parent_types, out, seen);
            for a in args {
                collect_captures_expr(a, locals, parent_types, out, seen);
            }
        }
        Statement::Inherited { args, .. } => {
            for a in args {
                collect_captures_expr(a, locals, parent_types, out, seen);
            }
        }
        Statement::IndirectCall { call, .. } => {
            collect_captures_expr(call, locals, parent_types, out, seen)
        }
        Statement::Str { value, target, .. } => {
            collect_captures_expr(&value.expr, locals, parent_types, out, seen);
            for e in value.width.iter().chain(value.precision.iter()) {
                collect_captures_expr(e, locals, parent_types, out, seen);
            }
            collect_captures_expr(target, locals, parent_types, out, seen);
        }
        Statement::ProcCall { name, args, span } => {
            // The callee may be a procedural variable of the parent.
            collect_captures_expr(
                &Expr::Var(name.clone(), *span),
                locals,
                parent_types,
                out,
                seen,
            );
            for a in args {
                collect_captures_expr(a, locals, parent_types, out, seen);
            }
        }
        Statement::Case {
            expr,
            branches,
            else_branch,
            ..
        } => {
            collect_captures_expr(expr, locals, parent_types, out, seen);
            for br in branches {
                for v in &br.values {
                    match v {
                        CaseValue::Single(e) => {
                            collect_captures_expr(e, locals, parent_types, out, seen)
                        }
                        CaseValue::Range(a, b) => {
                            collect_captures_expr(a, locals, parent_types, out, seen);
                            collect_captures_expr(b, locals, parent_types, out, seen);
                        }
                    }
                }
                for s in &br.body {
                    collect_captures_stmt(s, locals, parent_types, out, seen);
                }
            }
            if let Some(stmts) = else_branch {
                for s in stmts {
                    collect_captures_stmt(s, locals, parent_types, out, seen);
                }
            }
        }
        Statement::With { target, body, .. } => {
            collect_captures_expr(target, locals, parent_types, out, seen);
            collect_captures(body, locals, parent_types, out, seen);
        }
        Statement::Exit {
            value: Some(value), ..
        } => collect_captures_expr(value, locals, parent_types, out, seen),
        Statement::PlaceAssignment { target, expr, .. } => {
            collect_captures_expr(target, locals, parent_types, out, seen);
            collect_captures_expr(expr, locals, parent_types, out, seen);
        }
        Statement::Exit { value: None, .. }
        | Statement::Break { .. }
        | Statement::Continue { .. }
        | Statement::Goto { .. }
        | Statement::Label { .. } => {}
    }
}

fn collect_captures_expr(
    expr: &Expr,
    locals: &std::collections::HashSet<String>,
    parent_types: &HashMap<String, PascalType>,
    out: &mut Vec<(String, PascalType)>,
    seen: &mut std::collections::HashSet<String>,
) {
    match expr {
        Expr::IndirectCall { callee, args, .. } => {
            collect_captures_expr(callee, locals, parent_types, out, seen);
            for a in args {
                collect_captures_expr(a, locals, parent_types, out, seen);
            }
        }
        Expr::RecordInit { fields, .. } => {
            for (_, value) in fields {
                collect_captures_expr(value, locals, parent_types, out, seen);
            }
        }
        Expr::ArrayInit { elements, .. } => {
            for e in elements {
                collect_captures_expr(e, locals, parent_types, out, seen);
            }
        }
        Expr::MethodCall { object, args, .. } => {
            collect_captures_expr(object, locals, parent_types, out, seen);
            for a in args {
                collect_captures_expr(a, locals, parent_types, out, seen);
            }
        }
        Expr::Inherited { args, .. } => {
            for a in args {
                collect_captures_expr(a, locals, parent_types, out, seen);
            }
        }
        Expr::Var(name, _) => {
            if !locals.contains(name) && parent_types.contains_key(name) && !seen.contains(name) {
                seen.insert(name.clone());
                out.push((
                    name.clone(),
                    parent_types
                        .get(name)
                        .cloned()
                        .unwrap_or(PascalType::Integer),
                ));
            }
        }
        Expr::BinOp { left, right, .. } => {
            collect_captures_expr(left, locals, parent_types, out, seen);
            collect_captures_expr(right, locals, parent_types, out, seen);
        }
        Expr::UnaryOp { operand, .. } => {
            collect_captures_expr(operand, locals, parent_types, out, seen)
        }
        Expr::Deref(inner, _) | Expr::AddrOf(inner, _) => {
            collect_captures_expr(inner, locals, parent_types, out, seen)
        }
        Expr::Call { name, args, span } => {
            collect_captures_expr(
                &Expr::Var(name.clone(), *span),
                locals,
                parent_types,
                out,
                seen,
            );
            for a in args {
                collect_captures_expr(a, locals, parent_types, out, seen);
            }
        }
        Expr::Index { array, index, .. } => {
            collect_captures_expr(array, locals, parent_types, out, seen);
            collect_captures_expr(index, locals, parent_types, out, seen);
        }
        Expr::FieldAccess { record, .. } => {
            collect_captures_expr(record, locals, parent_types, out, seen)
        }
        Expr::SetConstructor { elements, .. } => {
            for e in elements {
                match e {
                    SetElement::Single(e) => {
                        collect_captures_expr(e, locals, parent_types, out, seen)
                    }
                    SetElement::Range(a, b) => {
                        collect_captures_expr(a, locals, parent_types, out, seen);
                        collect_captures_expr(b, locals, parent_types, out, seen);
                    }
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    // dSYM bundle + Apple-format DWARF: macOS-only.
    #[cfg(target_os = "macos")]
    #[test]
    fn debug_info_keeps_the_users_spelling() {
        let source = "program Demo;\ntype Color = (Red, Green);\n  TPoint = record X, Y: integer; end;\nvar Counter: integer;\n  Pt: TPoint;\n  Hue: Color;\nprocedure Bump;\nvar Step: integer;\nbegin\n  step := 1;\n  COUNTER := counter + STEP\nend;\nbegin\n  bump;\n  pt.x := 1;\n  hue := green\nend.\n";
        let program = Parser::new(source).parse_program().expect("parse failed");
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "/tmp/test_spelling.pas");
        codegen.compile(&program).expect("codegen failed");
        let ir = codegen.print_ir();
        for expected in [
            r#"!DIGlobalVariable(name: "Counter""#,
            r#"!DISubprogram(name: "Bump""#,
            r#"!DILocalVariable(name: "Step""#,
            r#"!DIDerivedType(tag: DW_TAG_member, name: "X""#,
            r#"!DIEnumerator(name: "Green""#,
        ] {
            assert!(ir.contains(expected), "missing {expected} in IR");
        }
    }

    #[test]
    fn compile_and_run_simple_program() {
        let source = "program Test;\nvar\n  x: integer;\nbegin\n  x := 42;\n  writeln(x)\nend.\n";
        let source_path = "/tmp/test_codegen.pas";
        std::fs::write(source_path, source).unwrap();

        let mut parser = Parser::new(source);
        let program = parser.parse_program().expect("parse failed");

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, source_path);
        codegen.compile(&program).expect("codegen failed");

        let exe_path = "/tmp/test_codegen_out";
        codegen.emit_executable(exe_path).expect("emit failed");

        // Verify the executable runs and outputs 42 (to capture file)
        let _ = std::fs::remove_file(&bruto_lang::target::console_capture_path());
        let status = std::process::Command::new(exe_path)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run failed");
        assert!(status.success(), "non-zero exit");
        let captured = std::fs::read_to_string(&bruto_lang::target::console_capture_path())
            .expect("capture file missing");
        assert_eq!(captured.trim(), "42", "expected 42, got: {captured:?}");
        let _ = std::fs::remove_file(&bruto_lang::target::console_capture_path());

        // Verify debug info exists (dwarfdump should find compilation unit)
        let dwarf = std::process::Command::new("dwarfdump")
            .arg("--debug-info")
            .arg(format!("{exe_path}.dSYM"))
            .output()
            .expect("dwarfdump failed");
        let dwarf_out = String::from_utf8_lossy(&dwarf.stdout);
        assert!(
            dwarf_out.contains("DW_TAG_compile_unit"),
            "no DWARF compile unit found:\n{dwarf_out}"
        );
        assert!(
            dwarf_out.contains("test_codegen.pas"),
            "source file not in DWARF:\n{dwarf_out}"
        );

        // Clean up
        let _ = std::fs::remove_file(exe_path);
        let _ = std::fs::remove_dir_all(format!("{exe_path}.dSYM"));
        let _ = std::fs::remove_file(source_path);
    }

    // lldb-script driven breakpoint stop check is wired against the
    // macOS lldb output format; on Linux the equivalent flow is gdb.
    #[cfg(target_os = "macos")]
    #[test]
    fn lldb_stops_at_breakpoint() {
        // Full integration: compile → set breakpoint → run → verify stop
        let source = "program BP;\nvar\n  x: integer;\nbegin\n  x := 10;\n  x := x + 1;\n  writeln(x)\nend.\n";
        let source_path = "/tmp/test_bp.pas";
        std::fs::write(source_path, source).unwrap();

        let mut parser = Parser::new(source);
        let program = parser.parse_program().expect("parse failed");

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, source_path);
        codegen.compile(&program).expect("codegen failed");

        let exe_path = "/tmp/test_bp_out";
        codegen.emit_executable(exe_path).expect("emit failed");

        // Run lldb in batch mode: set breakpoint on line 5, run, check output
        let lldb_out = std::process::Command::new("lldb")
            .args([
                "--no-use-colors",
                "--batch",
                "--one-line",
                "breakpoint set --file test_bp.pas --line 5",
                "--one-line",
                "run",
                "--one-line",
                "frame variable",
                "--one-line",
                "continue",
                "--one-line",
                "quit",
                "--",
                exe_path,
            ])
            .output()
            .expect("lldb failed");
        let stdout = String::from_utf8_lossy(&lldb_out.stdout);
        let stderr = String::from_utf8_lossy(&lldb_out.stderr);

        eprintln!("=== lldb stdout ===\n{stdout}");
        eprintln!("=== lldb stderr ===\n{stderr}");

        // Should see "stop reason = breakpoint" proving the breakpoint fired
        assert!(
            stdout.contains("stop reason = breakpoint"),
            "lldb did not stop at breakpoint:\nstdout: {stdout}\nstderr: {stderr}"
        );

        let _ = std::fs::remove_file(exe_path);
        let _ = std::fs::remove_dir_all(format!("{exe_path}.dSYM"));
        let _ = std::fs::remove_file(source_path);
    }

    #[test]
    fn retail_build_is_optimized_and_has_no_debug_info() {
        use bruto_lang::language::{BuildOptions, BuildProfile, OptimizeFor};

        let source = "program Opt;\nvar\n  i, s: integer;\nbegin\n  s := 0;\n  for i := 1 to 10 do\n    s := s + i;\n  writeln(s)\nend.\n";
        let tmp = std::env::temp_dir();
        let source_path_buf = tmp.join("test_retail.pas");
        let source_path = source_path_buf.to_string_lossy();
        std::fs::write(source_path.as_ref(), source).unwrap();

        for optimize in [OptimizeFor::Size, OptimizeFor::Both, OptimizeFor::Speed] {
            let mut parser = Parser::new(source);
            let program = parser.parse_program().unwrap();
            let context = Context::create();
            let mut codegen = CodeGen::new(&context, source_path.as_ref());
            codegen.set_build_options(BuildOptions {
                profile: BuildProfile::Retail,
                optimize,
                obfuscate: false,
            });
            codegen.compile(&program).unwrap();

            let exe_path_buf = tmp.join(format!(
                "test_retail_{}_out{}",
                optimize.as_str(),
                if cfg!(windows) { ".exe" } else { "" }
            ));
            let exe_path = exe_path_buf.to_string_lossy();
            // A leftover bundle from a Debug build must not survive a Retail one.
            let _ = std::fs::create_dir_all(format!("{exe_path}.dSYM"));
            codegen.emit_executable(exe_path.as_ref()).unwrap();

            let ir = codegen.print_ir();
            assert!(
                !ir.contains("DICompileUnit") && !ir.contains("!dbg"),
                "{optimize:?}: debug info survived a Retail build"
            );
            assert!(
                !std::path::Path::new(&format!("{exe_path}.dSYM")).exists(),
                "{optimize:?}: stale .dSYM left next to Retail binary"
            );

            let output = std::process::Command::new(exe_path.as_ref())
                .output()
                .expect("run failed");
            assert!(output.status.success(), "{optimize:?}: non-zero exit");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let captured = std::fs::read_to_string(bruto_lang::target::console_capture_path())
                .unwrap_or_default();
            assert!(
                stdout.contains("55") || captured.contains("55"),
                "{optimize:?}: expected 55, got stdout={stdout:?} capture={captured:?}"
            );

            let _ = std::fs::remove_file(exe_path.as_ref());
        }
        let _ = std::fs::remove_file(source_path.as_ref());
    }

    #[test]
    fn debug_build_assembly_maps_back_to_pascal_lines() {
        // End-to-end: Debug build -> emit_object's .s listing -> bruto_lang's
        // .loc parser -> at least one instruction tagged with the real
        // Pascal line the IDE's Disassembly window would navigate to.
        let source = "program Asm;\nvar\n  x: integer;\nbegin\n  x := 42;\n  writeln(x)\nend.\n";
        let tmp = std::env::temp_dir();
        let source_path_buf = tmp.join("test_asm_map.pas");
        let source_path = source_path_buf.to_string_lossy();
        std::fs::write(source_path.as_ref(), source).unwrap();

        let mut parser = Parser::new(source);
        let program = parser.parse_program().unwrap();
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, source_path.as_ref());
        codegen.compile(&program).unwrap();

        let out_path_buf = tmp.join("test_asm_map_out");
        let out_path = out_path_buf.to_string_lossy();
        let artifacts = codegen.emit_object(out_path.as_ref()).unwrap();
        let asm_path = artifacts
            .asm_path
            .as_ref()
            .expect("Debug build should emit a .s listing");
        let asm_text = std::fs::read_to_string(asm_path).unwrap();
        assert!(asm_text.contains(".loc"), "no .loc directives in listing");

        let lines = bruto_lang::disasm::parse(&asm_text);
        assert!(!lines.is_empty(), "parser produced no lines");
        assert!(
            lines.iter().any(|l| l.source_line == Some(5)),
            "no instruction mapped to line 5 (`x := 42;`)"
        );
        assert!(
            lines.iter().any(|l| l.source_line == Some(6)),
            "no instruction mapped to line 6 (`writeln(x)`)"
        );

        let _ = std::fs::remove_file(&artifacts.obj_path);
        let _ = std::fs::remove_file(asm_path);
        let _ = std::fs::remove_file(source_path.as_ref());
    }

    #[test]
    fn program_output_goes_to_capture_file() {
        let source = "program Cap;\nbegin\n  writeln('hello capture')\nend.\n";
        let tmp = std::env::temp_dir();
        let source_path_buf = tmp.join("test_capture.pas");
        let source_path = source_path_buf.to_string_lossy();
        std::fs::write(source_path.as_ref(), source).unwrap();

        let mut parser = Parser::new(source);
        let program = parser.parse_program().unwrap();
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, source_path.as_ref());
        codegen.compile(&program).unwrap();

        let exe_path_buf = tmp.join(if cfg!(windows) {
            "test_capture_out.exe"
        } else {
            "test_capture_out"
        });
        let exe_path = exe_path_buf.to_string_lossy();
        codegen.emit_executable(exe_path.as_ref()).unwrap();

        let _ = std::fs::remove_file(&bruto_lang::target::console_capture_path());
        let status = std::process::Command::new(exe_path.as_ref())
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());

        let captured =
            std::fs::read_to_string(&bruto_lang::target::console_capture_path()).unwrap();
        assert_eq!(
            captured.trim(),
            "hello capture",
            "capture file: {captured:?}"
        );

        let _ = std::fs::remove_file(exe_path.as_ref());
        let _ = std::fs::remove_dir_all(format!("{exe_path}.dSYM"));
        let _ = std::fs::remove_file(source_path.as_ref());
        let _ = std::fs::remove_file(&bruto_lang::target::console_capture_path());
    }
}
