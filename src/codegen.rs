/// LLVM code generation for Mini-Pascal with DWARF debug info.
///
/// Uses inkwell to generate LLVM IR, emit object files, and link with the system linker.
/// Every statement is annotated with source locations for breakpoint support.
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::OptimizationLevel;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::debug_info::{
    AsDIScope, DICompileUnit, DIFile, DIFlags, DIFlagsConstants, DIScope, DISubprogram, DIType,
    DWARFEmissionKind, DWARFSourceLanguage, DebugInfoBuilder,
};
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

#[derive(Debug)]
pub struct CodeGenError {
    pub message: String,
    pub span: Option<Span>,
}

impl fmt::Display for CodeGenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(span) = self.span {
            write!(f, "line {}:{}: {}", span.line, span.column, self.message)
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

    source_path: String,

    // Compiler directives ({$R+}, {$Q+}, {$I+}).
    directives: crate::parser::Directives,

    // Per-variable metadata for the watch window. Populated during compile.
    // Format lines: `name|kind|extra` (kind = "enum", "set", "vrec", "real", "char", "bool")
    metadata_lines: Vec<String>,
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
            source_path: source_path.to_string(),
            directives: crate::parser::Directives::default(),
            metadata_lines: Vec::new(),
        }
    }

    /// Write watch-window metadata to `<exe>.bruto-meta`.
    pub fn write_metadata(&self, exe_path: &str) -> Result<(), String> {
        let path = format!("{exe_path}.bruto-meta");
        let body = self.metadata_lines.join("\n");
        std::fs::write(&path, body).map_err(|e| format!("metadata write: {e}"))
    }

    /// Set compiler directives (parsed by the lexer from `{$X+/-}` comments).
    pub fn set_directives(&mut self, d: crate::parser::Directives) {
        self.directives = d;
    }

    /// Compile a Pascal program AST into LLVM IR.
    pub fn compile(&mut self, program: &Program) -> Result<(), CodeGenError> {
        self.emit_runtime_decls();

        // Create main function
        let main_fn_type = self.context.i64_type().fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_fn_type, None);

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
            &program.name,
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

        self.current_fn = Some(main_fn);
        self.current_scope = Some(di_main.as_debug_info_scope());

        // Pre-create basic blocks for all declared labels
        for &label in &program.labels {
            let bb = self
                .context
                .append_basic_block(main_fn, &format!("label_{label}"));
            self.label_blocks.insert(label, bb);
        }

        // Register type aliases
        for td in &program.type_decls {
            self.type_defs.insert(td.name.clone(), td.ty.clone());
            if let PascalType::Enum { values, .. } = &td.ty {
                for (i, val) in values.iter().enumerate() {
                    self.enum_values.insert(val.clone(), i as i64);
                }
                self.enum_type_values
                    .insert(td.name.clone(), values.clone());
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

        // Emit constants as variables initialized to their values
        for c in &program.consts {
            self.compile_const_decl(c, di_main)?;
        }

        // Emit variable allocations
        for var_decl in &program.vars {
            self.compile_var_decl(var_decl, di_main)?;
        }

        // Compile body (compile_block emits a synthetic alloca on the `end.`
        // line so users can drop a breakpoint there).
        self.compile_block(&program.body)?;

        // Return 0 — debug-loc on `end.` so the ret instruction is associated
        // with that source line. When the user steps past `end.`, lldb
        // returns from main into dyld; debugger.rs detects the no-source
        // stop frame and issues `continue`, so the process still exits
        // cleanly without the user pressing F8 over and over.
        self.set_debug_loc(program.body.end_span);
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

    /// Write object file and link to executable.
    pub fn emit_executable(&self, output_path: &str) -> Result<(), String> {
        Target::initialize_native(&InitializationConfig::default()).map_err(|e| e.to_string())?;

        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
        let machine = target
            .create_target_machine(
                &triple,
                "generic",
                "",
                OptimizationLevel::None,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or("could not create target machine")?;

        let obj_path = format!("{output_path}.o");
        machine
            .write_to_file(&self.module, FileType::Object, Path::new(&obj_path))
            .map_err(|e| e.to_string())?;

        // Link with -g to preserve debug info in the object file.
        // Redirect stdout/stderr so they don't corrupt the TUI.
        // -no-pie on Linux: most distros default to building Position
        // Independent Executables, but the LLVM TargetMachine is set to
        // the static reloc model, so absolute relocations in our object
        // file can't be folded into a PIE. Forcing a non-PIE binary
        // sidesteps that until/unless we switch the codegen to emit PIC.
        let link_args: Vec<&str> = {
            #[allow(unused_mut)]
            let mut a: Vec<&str> = vec![&obj_path, "-o", output_path, "-lm", "-g"];
            #[cfg(target_os = "linux")]
            a.push("-no-pie");
            a
        };
        let link_out = std::process::Command::new("cc")
            .args(&link_args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| e.to_string())?;

        if !link_out.status.success() {
            let stderr = String::from_utf8_lossy(&link_out.stderr);
            return Err(format!("linking failed: {stderr}"));
        }

        // On macOS, run dsymutil to create the .dSYM bundle that lldb needs.
        #[cfg(target_os = "macos")]
        {
            let dsym_out = std::process::Command::new("dsymutil")
                .arg(output_path)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .map_err(|e| e.to_string())?;
            if !dsym_out.status.success() {
                let stderr = String::from_utf8_lossy(&dsym_out.stderr);
                return Err(format!("dsymutil failed: {stderr}"));
            }
        }

        let _ = std::fs::remove_file(&obj_path);
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
        let path = self
            .builder
            .build_global_string_ptr("/tmp/turbo_pascal_console.txt", "capture_path")
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
                .builder
                .build_alloca(ptr_ty, var_name)
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

    fn set_debug_loc(&self, span: Span) {
        if let Some(scope) = self.current_scope {
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

        for name in &decl.names {
            let llvm_ty = self.llvm_type_for(&resolved_ty);
            let alloca = self
                .builder
                .build_alloca(llvm_ty, name)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;

            // Initialize scalar types to zero
            match &resolved_ty {
                PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => {
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
                | PascalType::ConformantArray { .. } => {
                    // Aggregate types: zero-initialized by alloca (no explicit store needed).
                }
            }

            // Debug variable info
            let di_type = self.create_debug_type(&resolved_ty);

            if let Some(di_type) = di_type {
                let di_var = self.di_builder.create_auto_variable(
                    di_sub.as_debug_info_scope(),
                    name,
                    self.di_file,
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

    fn record_metadata(&mut self, name: &str, ty: &PascalType) {
        let line = match ty {
            PascalType::Enum { values, .. } => {
                format!("{name}|enum|{}", values.join(","))
            }
            PascalType::Set { .. } => format!("{name}|set|"),
            PascalType::Record { fields, variant } => {
                let mut parts: Vec<String> = fields
                    .iter()
                    .map(|(n, t)| format!("{n}={}", type_short(t)))
                    .collect();
                if let Some(v) = variant {
                    parts.push(format!("__tag={}", v.tag_name));
                    for (vals, vfields) in &v.variants {
                        let val_str: Vec<String> = vals.iter().map(|v| v.to_string()).collect();
                        let f_str: Vec<String> = vfields
                            .iter()
                            .map(|(n, t)| format!("{n}={}", type_short(t)))
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

    fn compile_const_decl(
        &mut self,
        c: &ConstDecl,
        di_sub: DISubprogram<'ctx>,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(c.span);
        let val = self.compile_expr(&c.value)?;
        // Use declared type if provided, else infer from initializer.
        let ty = if let Some(t) = &c.ty {
            self.resolve_type(t)
        } else {
            self.infer_expr_type(&c.value)
        };
        let llvm_ty = self.llvm_type_for(&ty);
        let alloca = self
            .builder
            .build_alloca(llvm_ty, &c.name)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(c.span)))?;
        // If declared real but initializer is integer literal, promote.
        let store_val = if matches!(ty, PascalType::Real) && val.is_int_value() {
            self.builder
                .build_signed_int_to_float(
                    val.into_int_value(),
                    self.context.f64_type(),
                    "const_itof",
                )
                .map_err(|e| CodeGenError::new(e.to_string(), Some(c.span)))?
                .into()
        } else {
            val
        };
        self.builder
            .build_store(alloca, store_val)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(c.span)))?;
        self.variables.insert(c.name.clone(), alloca);
        self.var_types.insert(c.name.clone(), ty);
        let _ = di_sub;
        Ok(())
    }

    fn compile_proc_decl(&mut self, proc: &ProcDecl) -> Result<(), CodeGenError> {
        // Build parameter type list
        let mut param_llvm_types: Vec<inkwell::types::BasicMetadataTypeEnum> = Vec::new();
        let mut param_info: Vec<(String, ParamMode, PascalType)> = Vec::new();

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

        for group in &proc.params {
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
                    self.llvm_type_for(&resolved_group_ty).into()
                };
                param_llvm_types.push(ty);
                param_info.push((name.clone(), group.mode, resolved_group_ty.clone()));
            }
        }

        // Build function type
        let fn_type = if let Some(ref ret_ty) = proc.return_type {
            self.llvm_type_for(ret_ty).fn_type(&param_llvm_types, false)
        } else {
            self.context.void_type().fn_type(&param_llvm_types, false)
        };

        // Reuse existing function if this is the real definition after a forward decl
        let func = if let Some(existing) = self.module.get_function(&proc.name) {
            existing
        } else {
            self.module.add_function(&proc.name, fn_type, None)
        };

        // Store metadata
        self.proc_return_types
            .insert(proc.name.clone(), proc.return_type.clone());
        self.proc_param_modes
            .insert(proc.name.clone(), param_info.clone());

        // Forward declaration: empty body means just register the prototype
        if proc.body.statements.is_empty() {
            return Ok(());
        }

        // Debug info
        let di_i64 = self
            .di_builder
            .create_basic_type("integer", 64, 0x05, DIFlags::ZERO)
            .unwrap();
        let di_fn_type = self.di_builder.create_subroutine_type(
            self.di_file,
            proc.return_type.as_ref().map(|_| di_i64.as_type()),
            &[],
            DIFlags::ZERO,
        );
        let di_sub = self.di_builder.create_function(
            self.compile_unit.as_debug_info_scope(),
            &proc.name,
            None,
            self.di_file,
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
        self.saved_scopes.push((saved_vars, saved_types));

        self.current_fn = Some(func);
        self.current_scope = Some(di_sub.as_debug_info_scope());
        self.set_debug_loc(proc.span);

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
                    .builder
                    .build_alloca(llvm_ty, name)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
                self.builder
                    .build_store(alloca, param_val)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
                self.variables.insert(name.clone(), alloca);
                self.var_types.insert(name.clone(), resolved_ty);
            }
        }

        // Function result variable (Pascal assigns to function name)
        if let Some(ref ret_ty) = proc.return_type {
            let llvm_ty = self.llvm_type_for(ret_ty);
            let alloca = self
                .builder
                .build_alloca(llvm_ty, &proc.name)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
            // Initialize to zero
            let zero: BasicValueEnum = match ret_ty {
                PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => {
                    self.context.i64_type().const_int(0, false).into()
                }
                PascalType::Real => self.context.f64_type().const_float(0.0).into(),
                PascalType::Boolean => self.context.bool_type().const_int(0, false).into(),
                PascalType::Char => self.context.i8_type().const_int(0, false).into(),
                _ => self
                    .context
                    .ptr_type(AddressSpace::default())
                    .const_null()
                    .into(),
            };
            self.builder
                .build_store(alloca, zero)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
            self.variables.insert(proc.name.clone(), alloca);
            self.var_types.insert(proc.name.clone(), ret_ty.clone());
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
            collect_captures(
                &nested.body,
                &nested_locals,
                &self.var_types,
                &mut found,
                &mut seen,
            );
            self.proc_captures.insert(nested.name.clone(), found);
        }

        // Now compile each nested proc. Each call to compile_proc_decl saves and
        // restores self.variables/self.var_types, so we can call from within the parent.
        for nested in &proc.nested_procs {
            self.compile_proc_decl(nested)?;
        }
        // Restore parent's builder position
        if let Some(bb) = saved_block {
            self.builder.position_at_end(bb);
        }

        // Compile body
        self.compile_block(&proc.body)?;

        // Return
        if let Some(ref ret_ty) = proc.return_type {
            let llvm_ty = self.llvm_type_for(ret_ty);
            let alloca = *self.variables.get(&proc.name).unwrap();
            let ret_val = self
                .builder
                .build_load(llvm_ty, alloca, "retval")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
            self.builder
                .build_return(Some(&ret_val))
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
        } else {
            self.builder
                .build_return(None)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
        }

        // Restore scope
        let (restored_vars, restored_types) = self.saved_scopes.pop().unwrap();
        self.variables = restored_vars;
        self.var_types = restored_types;
        self.current_fn = saved_fn;
        self.current_scope = saved_scope;

        Ok(())
    }

    // ── block / statements ───────────────────────────────

    fn compile_block(&mut self, block: &Block) -> Result<(), CodeGenError> {
        for stmt in &block.statements {
            self.compile_statement(stmt)?;
        }
        // Emit a debug location on the `end` keyword so breakpoints can be
        // set there. We use a trivial load of 0 that the optimizer can remove
        // but that gives lldb a line to stop on.
        self.set_debug_loc(block.end_span);
        let _ = self
            .builder
            .build_alloca(self.context.i64_type(), "_end_bp")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(block.end_span)))?;
        Ok(())
    }

    fn compile_statement(&mut self, stmt: &Statement) -> Result<(), CodeGenError> {
        match stmt {
            Statement::Assignment { target, expr, span } => {
                self.set_debug_loc(*span);
                let val = self.compile_expr(expr)?;
                let alloca = *self.variables.get(target.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{target}'"), Some(*span))
                })?;
                self.builder
                    .build_store(alloca, val)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(())
            }
            Statement::DerefAssignment { target, expr, span } => {
                self.set_debug_loc(*span);
                let val = self.compile_expr(expr)?;
                let alloca = *self.variables.get(target.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{target}'"), Some(*span))
                })?;
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let ptr_val = self
                    .builder
                    .build_load(ptr_type, alloca, "ptr_val")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                // File buffer variable: `f^ := x`
                if let Some(t) = self.var_types.get(target.as_str()).cloned() {
                    if matches!(self.resolve_type(&t), PascalType::File { .. }) {
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
                }
                self.builder
                    .build_store(ptr_val.into_pointer_value(), val)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(())
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
                let alloca = *self.variables.get(target.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{target}'"), Some(*span))
                })?;
                let var_type = self
                    .var_types
                    .get(target.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodeGenError::new(format!("unknown type for '{target}'"), Some(*span))
                    })?;
                let resolved = self.resolve_type(&var_type);
                let (gep, _) =
                    self.gep_array_elem(alloca, &resolved, idx_val.into_int_value(), *span)?;
                self.builder
                    .build_store(gep, val)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(())
            }
            Statement::MultiIndexAssignment {
                target,
                indices,
                expr,
                span,
            } => {
                self.set_debug_loc(*span);
                let val = self.compile_expr(expr)?;
                let alloca = *self.variables.get(target.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{target}'"), Some(*span))
                })?;
                let var_type = self
                    .var_types
                    .get(target.as_str())
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
                        self.builder
                            .build_store(gep, val)
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
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
                let alloca = *self.variables.get(target.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{target}'"), Some(*span))
                })?;
                let var_type = self
                    .var_types
                    .get(target.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodeGenError::new(format!("unknown type for '{target}'"), Some(*span))
                    })?;
                let resolved = self.resolve_type(&var_type);
                match &resolved {
                    PascalType::Record { fields, variant } => {
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
                            self.builder
                                .build_store(gep, val)
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
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
                                self.builder
                                    .build_store(gep, val)
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                            } else {
                                // Search variant fields
                                let (byte_offset, _field_ty) =
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
                                self.builder
                                    .build_store(byte_gep, val)
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
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
            Statement::New { target, span } => self.compile_new(target, *span),
            Statement::Dispose { target, span } => self.compile_dispose(target, *span),
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
                if name == "reset" && args.len() == 1 {
                    return self.compile_file_open(&args[0], "r", *span);
                }
                if name == "rewrite" && args.len() == 1 {
                    return self.compile_file_open(&args[0], "w", *span);
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
                    let alloca = *self.variables.get(f_name.as_str()).unwrap();
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
                    let alloca = *self.variables.get(f_name.as_str()).unwrap();
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
            Statement::With {
                record_var,
                body,
                span,
            } => self.compile_with(record_var, body, *span),
            Statement::ChainedAssignment {
                target,
                chain,
                expr,
                span,
            } => self.compile_chained_assignment(target, chain, expr, *span),
        }
    }

    fn compile_new(&mut self, target: &str, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let alloca = *self.variables.get(target).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{target}'"), Some(span))
        })?;
        let var_type = self
            .var_types
            .get(target)
            .ok_or_else(|| CodeGenError::new(format!("unknown type for '{target}'"), Some(span)))?;

        let PascalType::Pointer(pointed) = var_type else {
            return Err(CodeGenError::new(
                format!("new() requires a pointer variable, got {target}"),
                Some(span),
            ));
        };

        let size = self.sizeof_type(pointed);
        let bruto_alloc = self.module.get_function("bruto_alloc").unwrap();
        let ptr = self
            .builder
            .build_call(
                bruto_alloc,
                &[self.context.i64_type().const_int(size, false).into()],
                "heap_ptr",
            )
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.builder
            .build_store(alloca, ptr)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_dispose(&mut self, target: &str, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let alloca = *self.variables.get(target).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{target}'"), Some(span))
        })?;
        let var_type = self
            .var_types
            .get(target)
            .ok_or_else(|| CodeGenError::new(format!("unknown type for '{target}'"), Some(span)))?;

        if !matches!(var_type, PascalType::Pointer(_)) {
            return Err(CodeGenError::new(
                format!("dispose() requires a pointer variable, got {target}"),
                Some(span),
            ));
        }

        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let ptr_val = self
            .builder
            .build_load(ptr_type, alloca, "ptr_val")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let bruto_free = self.module.get_function("bruto_free").unwrap();
        self.builder
            .build_call(bruto_free, &[ptr_val.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        // Null out the pointer
        self.builder
            .build_store(alloca, ptr_type.const_null())
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_with(
        &mut self,
        record_var: &str,
        body: &Block,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let alloca = *self.variables.get(record_var).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{record_var}'"), Some(span))
        })?;
        let var_type = self.var_types.get(record_var).cloned().ok_or_else(|| {
            CodeGenError::new(format!("unknown type for '{record_var}'"), Some(span))
        })?;
        let resolved = self.resolve_type(&var_type);
        let fields = match &resolved {
            PascalType::Record { fields, .. } => fields.clone(),
            _ => {
                return Err(CodeGenError::new(
                    format!("'{record_var}' is not a record"),
                    Some(span),
                ));
            }
        };

        // Save any existing variables that would be shadowed
        let mut saved: Vec<(String, Option<PointerValue<'ctx>>, Option<PascalType>)> = Vec::new();
        for (i, (name, ty)) in fields.iter().enumerate() {
            let old_var = self.variables.remove(name);
            let old_type = self.var_types.remove(name);
            saved.push((name.clone(), old_var, old_type));

            let gep = self
                .builder
                .build_struct_gep(
                    self.llvm_type_for(&resolved),
                    alloca,
                    i as u32,
                    &format!("with_{name}"),
                )
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.variables.insert(name.clone(), gep);
            self.var_types.insert(name.clone(), ty.clone());
        }

        self.compile_block(body)?;

        // Restore saved variables
        for (name, old_var, old_type) in saved {
            self.variables.remove(&name);
            self.var_types.remove(&name);
            if let Some(v) = old_var {
                self.variables.insert(name.clone(), v);
            }
            if let Some(t) = old_type {
                self.var_types.insert(name, t);
            }
        }

        Ok(())
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

        let mut ptr = *self.variables.get(target).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{target}'"), Some(span))
        })?;
        let mut cur_type =
            self.var_types.get(target).cloned().ok_or_else(|| {
                CodeGenError::new(format!("unknown type for '{target}'"), Some(span))
            })?;
        cur_type = self.resolve_type(&cur_type);

        for (i, access) in chain.iter().enumerate() {
            let is_last = i == chain.len() - 1;
            match access {
                LValueAccess::Field(field) => {
                    let (field_idx, field_ty) = match &cur_type {
                        PascalType::Record { fields, .. } => {
                            let idx =
                                fields.iter().position(|(n, _)| n == field).ok_or_else(|| {
                                    CodeGenError::new(
                                        format!("no field '{field}' in record"),
                                        Some(span),
                                    )
                                })?;
                            (idx, fields[idx].1.clone())
                        }
                        _ => {
                            return Err(CodeGenError::new(
                                "field access on non-record",
                                Some(span),
                            ));
                        }
                    };
                    let gep = self
                        .builder
                        .build_struct_gep(
                            self.llvm_type_for(&cur_type),
                            ptr,
                            field_idx as u32,
                            "chain_field",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    if is_last {
                        self.builder
                            .build_store(gep, val)
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                        return Ok(());
                    }
                    ptr = gep;
                    cur_type = self.resolve_type(&field_ty);
                }
                LValueAccess::Index(index_expr) => {
                    let idx_val = self.compile_expr(index_expr)?;
                    let (lo, hi) = match &cur_type {
                        PascalType::Array { lo, hi, .. } => (*lo, *hi),
                        _ => return Err(CodeGenError::new("indexing non-array", Some(span))),
                    };
                    let elem_ty = match &cur_type {
                        PascalType::Array { elem, .. } => elem.as_ref().clone(),
                        _ => unreachable!(),
                    };
                    self.emit_range_check(idx_val.into_int_value(), lo, hi, span)?;
                    let adj = self
                        .builder
                        .build_int_sub(
                            idx_val.into_int_value(),
                            self.context.i64_type().const_int(lo as u64, true),
                            "adj",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let gep = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.llvm_type_for(&cur_type),
                                ptr,
                                &[self.context.i64_type().const_int(0, false), adj],
                                "chain_idx",
                            )
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    };
                    if is_last {
                        self.builder
                            .build_store(gep, val)
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
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
                        self.builder
                            .build_store(loaded, val)
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
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
        let sel_val = self.compile_expr(expr)?;
        let func = self.current_fn.unwrap();
        let end_bb = self.context.append_basic_block(func, "case_end");

        for branch in branches {
            let match_bb = self.context.append_basic_block(func, "case_match");
            let next_bb = self.context.append_basic_block(func, "case_next");

            let mut any_match: Option<inkwell::values::IntValue> = None;
            for val in &branch.values {
                let cmp = match val {
                    CaseValue::Single(v) => {
                        let v_val = self.compile_expr(v)?;
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
                        let lo_val = self.compile_expr(lo)?;
                        let hi_val = self.compile_expr(hi)?;
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
            PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => 8,
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
            PascalType::Record { fields, variant } => {
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
                offset += self.sizeof_type(ty);
            }
        }
        None
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
        self.compile_block(body)?;
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
        self.set_debug_loc(span);
        let func = self.current_fn.unwrap();
        let alloca = *self
            .variables
            .get(var)
            .ok_or_else(|| CodeGenError::new(format!("undefined variable '{var}'"), Some(span)))?;

        // Initialize loop variable
        let from_val = self.compile_expr(from)?;
        self.builder
            .build_store(alloca, from_val)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        let to_val = self.compile_expr(to)?;

        let cond_bb = self.context.append_basic_block(func, "forcond");
        let body_bb = self.context.append_basic_block(func, "forbody");
        let after_bb = self.context.append_basic_block(func, "forend");

        self.builder
            .build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Condition: var <= to (or var >= to for downto)
        self.builder.position_at_end(cond_bb);
        self.set_debug_loc(span);
        let cur = self
            .builder
            .build_load(self.context.i64_type(), alloca, "for_cur")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let pred = if downto {
            inkwell::IntPredicate::SGE
        } else {
            inkwell::IntPredicate::SLE
        };
        let cond = self
            .builder
            .build_int_compare(
                pred,
                cur.into_int_value(),
                to_val.into_int_value(),
                "for_cmp",
            )
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder
            .build_conditional_branch(cond, body_bb, after_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Body
        self.builder.position_at_end(body_bb);
        self.compile_block(body)?;

        // Increment/decrement
        let cur2 = self
            .builder
            .build_load(self.context.i64_type(), alloca, "for_cur2")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let step = if downto {
            self.builder
                .build_int_sub(
                    cur2.into_int_value(),
                    self.context.i64_type().const_int(1, false),
                    "dec",
                )
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        } else {
            self.builder
                .build_int_add(
                    cur2.into_int_value(),
                    self.context.i64_type().const_int(1, false),
                    "inc",
                )
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        };
        self.builder
            .build_store(alloca, step)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder
            .build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        self.builder.position_at_end(after_bb);
        Ok(())
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

        self.builder.position_at_end(body_bb);
        for stmt in body {
            self.compile_statement(stmt)?;
        }

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
        // Indirect call through a procedural parameter
        if let Some(PascalType::Proc {
            params: ptypes,
            return_type,
        }) = self.var_types.get(name).cloned()
        {
            let alloca = *self.variables.get(name).unwrap();
            let ptr_ty = self.context.ptr_type(AddressSpace::default());
            let fp = self
                .builder
                .build_load(ptr_ty, alloca, "fp")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                .into_pointer_value();
            let mut llvm_params: Vec<inkwell::types::BasicMetadataTypeEnum> = Vec::new();
            for p in &ptypes {
                llvm_params.push(self.llvm_type_for(p).into());
            }
            let fn_type = if let Some(rt) = &return_type {
                self.llvm_type_for(rt).fn_type(&llvm_params, false)
            } else {
                self.context.void_type().fn_type(&llvm_params, false)
            };
            let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
            for arg in args {
                let v = self.compile_expr(arg)?;
                call_args.push(v.into());
            }
            self.builder
                .build_indirect_call(fn_type, fp, &call_args, "")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
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
        let var_name = match &args[0] {
            Expr::Var(name, _) => name.clone(),
            _ => return Err(CodeGenError::new("inc/dec requires a variable", Some(span))),
        };
        let alloca = *self.variables.get(var_name.as_str()).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{var_name}'"), Some(span))
        })?;
        let var_type = self
            .var_types
            .get(var_name.as_str())
            .cloned()
            .ok_or_else(|| {
                CodeGenError::new(format!("unknown type for '{var_name}'"), Some(span))
            })?;
        let llvm_ty = self.llvm_type_for(&var_type);

        let cur = self
            .builder
            .build_load(llvm_ty, alloca, "cur")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .into_int_value();

        let step = if args.len() == 2 {
            self.compile_expr(&args[1])?.into_int_value()
        } else {
            cur.get_type().const_int(1, false)
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

        self.builder
            .build_store(alloca, result)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_str_delete(&mut self, args: &[Expr], span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let s_name = match &args[0] {
            Expr::Var(name, _) => name.clone(),
            _ => {
                return Err(CodeGenError::new(
                    "delete requires a string variable",
                    Some(span),
                ));
            }
        };
        let alloca = *self.variables.get(s_name.as_str()).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{s_name}'"), Some(span))
        })?;
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let s_val = self
            .builder
            .build_load(ptr_ty, alloca, "s")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let idx = self.compile_expr(&args[1])?;
        let cnt = self.compile_expr(&args[2])?;
        let f = self.module.get_function("bruto_str_delete").unwrap();
        let result = self
            .builder
            .build_call(f, &[s_val.into(), idx.into(), cnt.into()], "deleted")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.builder
            .build_store(alloca, result)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_str_insert(&mut self, args: &[Expr], span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let source = self.compile_expr(&args[0])?;
        let s_name = match &args[1] {
            Expr::Var(name, _) => name.clone(),
            _ => {
                return Err(CodeGenError::new(
                    "insert requires a string variable as 2nd arg",
                    Some(span),
                ));
            }
        };
        let alloca = *self.variables.get(s_name.as_str()).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{s_name}'"), Some(span))
        })?;
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let s_val = self
            .builder
            .build_load(ptr_ty, alloca, "s")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let idx = self.compile_expr(&args[2])?;
        let f = self.module.get_function("bruto_str_insert").unwrap();
        let result = self
            .builder
            .build_call(f, &[source.into(), s_val.into(), idx.into()], "inserted")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.builder
            .build_store(alloca, result)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_str_proc(&mut self, args: &[Expr], span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let val = self.compile_expr(&args[0])?;
        let s_name = match &args[1] {
            Expr::Var(name, _) => name.clone(),
            _ => {
                return Err(CodeGenError::new(
                    "str requires a string variable as 2nd arg",
                    Some(span),
                ));
            }
        };
        let alloca = *self.variables.get(s_name.as_str()).ok_or_else(|| {
            CodeGenError::new(format!("undefined variable '{s_name}'"), Some(span))
        })?;
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
        self.builder
            .build_store(alloca, result)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
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
        let alloca = *self.variables.get(x_name.as_str()).ok_or_else(|| {
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
        let code_alloca = *self.variables.get(code_name.as_str()).ok_or_else(|| {
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
        let alloca = *self.variables.get(f_name.as_str()).ok_or_else(|| {
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
            .variables
            .get(name.as_str())
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
            .variables
            .get(name.as_str())
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
        let alloca = *self.variables.get(name.as_str()).unwrap();
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
        let alloca = *self.variables.get(name.as_str()).unwrap();
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
        let src_ptr = *self.variables.get(src_name.as_str()).unwrap();
        let dst_ptr = *self.variables.get(dst_name.as_str()).unwrap();
        let src_ty = self.var_types.get(src_name.as_str()).cloned().unwrap();
        let dst_ty = self.var_types.get(dst_name.as_str()).cloned().unwrap();
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
        let alloca = *self.variables.get(s_name.as_str()).ok_or_else(|| {
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
            let alloca = *self.variables.get(cname.as_str()).ok_or_else(|| {
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
                let alloca = *self.variables.get(vname.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{vname}'"), Some(span))
                })?;
                let var_ty = self.var_types.get(vname.as_str()).cloned().unwrap();
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
                if let Expr::Var(vname, _) = arg {
                    if let Some(PascalType::Proc { .. }) = self.var_types.get(vname.as_str()) {
                        let alloca = *self.variables.get(vname.as_str()).unwrap();
                        let ptr_ty = self.context.ptr_type(AddressSpace::default());
                        let fp = self
                            .builder
                            .build_load(ptr_ty, alloca, "argfp")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                        call_args.push(fp.into());
                    } else if let Some(target_fn) = self.module.get_function(vname) {
                        call_args.push(target_fn.as_global_value().as_pointer_value().into());
                    } else {
                        return Err(CodeGenError::new(
                            format!("'{vname}' is not a procedure or function"),
                            Some(span),
                        ));
                    }
                } else {
                    return Err(CodeGenError::new(
                        "procedural parameter requires a procedure or function name",
                        Some(span),
                    ));
                }
                pi += 1;
                continue;
            }

            if is_var_param {
                if let Expr::Var(vname, vspan) = arg {
                    let alloca = *self.variables.get(vname.as_str()).ok_or_else(|| {
                        CodeGenError::new(format!("undefined variable '{vname}'"), Some(*vspan))
                    })?;
                    call_args.push(alloca.into());
                } else {
                    return Err(CodeGenError::new(
                        "var parameter requires a variable",
                        Some(span),
                    ));
                }
            } else {
                let val = self.compile_expr(arg)?;
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
        if let Some(first) = args.first() {
            if first.width.is_none() && first.precision.is_none() {
                if let Expr::Var(name, _) = &first.expr {
                    if let Some(t) = self.var_types.get(name.as_str()) {
                        if matches!(self.resolve_type(t), PascalType::File { .. }) {
                            let alloca = *self.variables.get(name.as_str()).unwrap();
                            let ptr_ty = self.context.ptr_type(AddressSpace::default());
                            let fp = self
                                .builder
                                .build_load(ptr_ty, alloca, "fileptr")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                                .into_pointer_value();
                            file_target = Some(fp);
                            start_idx = 1;
                        }
                    }
                }
            }
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
            .var_types
            .get(targets[0].as_str())
            .cloned()
            .ok_or_else(|| {
                CodeGenError::new(format!("unknown type for '{}'", targets[0]), Some(span))
            })?;
        let resolved_first = self.resolve_type(&first_var_type);
        let mut start_idx = 0;
        let mut file_target: Option<PointerValue<'ctx>> = None;
        if matches!(resolved_first, PascalType::File { .. }) {
            let alloca = *self.variables.get(targets[0].as_str()).unwrap();
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
            let alloca = *self.variables.get(target.as_str()).ok_or_else(|| {
                CodeGenError::new(format!("undefined variable '{target}'"), Some(span))
            })?;
            let var_type = self
                .var_types
                .get(target.as_str())
                .cloned()
                .ok_or_else(|| {
                    CodeGenError::new(format!("unknown type for '{target}'"), Some(span))
                })?;
            let resolved = self.resolve_type(&var_type);

            let (read_fn, _is_file) = if let Some(_) = file_target {
                match resolved {
                    PascalType::Integer | PascalType::Subrange { .. } | PascalType::Enum { .. } => {
                        ("bruto_file_read_int", true)
                    }
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
                    PascalType::Integer | PascalType::Subrange { .. } | PascalType::Enum { .. } => {
                        ("bruto_read_int", false)
                    }
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
            self.builder
                .build_store(alloca, val)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
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
                let a = *self.variables.get(name.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{name}'"), Some(*vspan))
                })?;
                let t = self.var_types.get(name.as_str()).cloned().ok_or_else(|| {
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
                let (lo, hi) = match &base_type {
                    PascalType::Array { lo, hi, .. } => (*lo, *hi),
                    _ => return Err(CodeGenError::new("indexing non-array", Some(span))),
                };
                let elem_ty = match &base_type {
                    PascalType::Array { elem, .. } => elem.as_ref().clone(),
                    _ => unreachable!(),
                };
                self.emit_range_check(idx_val.into_int_value(), lo, hi, span)?;
                let adj = self
                    .builder
                    .build_int_sub(
                        idx_val.into_int_value(),
                        self.context.i64_type().const_int(lo as u64, true),
                        "adj_idx",
                    )
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                let gep = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.llvm_type_for(&base_type),
                            base_ptr,
                            &[self.context.i64_type().const_int(0, false), adj],
                            "arr_gep",
                        )
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                Ok((gep, elem_ty))
            }
            Expr::FieldAccess {
                record,
                field,
                span: fa_span,
            } => {
                let (rec_ptr, rec_type) = self.resolve_array_base(record, *fa_span)?;
                let resolved = self.resolve_type(&rec_type);
                let (field_idx, field_ty) = match &resolved {
                    PascalType::Record { fields, .. } => {
                        let idx = fields.iter().position(|(n, _)| n == field).ok_or_else(|| {
                            CodeGenError::new(format!("no field '{field}' in record"), Some(span))
                        })?;
                        (idx, fields[idx].1.clone())
                    }
                    _ => return Err(CodeGenError::new("field access on non-record", Some(span))),
                };
                let gep = self
                    .builder
                    .build_struct_gep(
                        self.llvm_type_for(&resolved),
                        rec_ptr,
                        field_idx as u32,
                        "fa_gep",
                    )
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                Ok((gep, field_ty))
            }
            Expr::Deref(inner, dspan) => {
                let (ptr, ptr_type) = self.resolve_array_base(inner, *dspan)?;
                let resolved = self.resolve_type(&ptr_type);
                let pointed = match &resolved {
                    PascalType::Pointer(inner) => inner.as_ref().clone(),
                    _ => return Err(CodeGenError::new("dereference of non-pointer", Some(span))),
                };
                let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
                let loaded = self
                    .builder
                    .build_load(ptr_ty, ptr, "deref_base")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into_pointer_value();
                Ok((loaded, pointed))
            }
            _ => Err(CodeGenError::new(
                "array indexing requires a variable",
                Some(span),
            )),
        }
    }

    // ── expressions ──────────────────────────────────────

    fn compile_expr(&mut self, expr: &Expr) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        match expr {
            Expr::IntLit(n, _) => Ok(self.context.i64_type().const_int(*n as u64, true).into()),
            Expr::RealLit(r, _) => Ok(self.context.f64_type().const_float(*r).into()),
            Expr::CharLit(c, _) => Ok(self.context.i8_type().const_int(*c as u64, false).into()),
            Expr::StrLit(s, span) => {
                let gs = self
                    .builder
                    .build_global_string_ptr(s, "str_lit")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(gs.as_pointer_value().into())
            }
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
                let alloca = *self.variables.get(name.as_str()).ok_or_else(|| {
                    CodeGenError::new(format!("undefined variable '{name}'"), Some(*span))
                })?;
                let var_type = self.var_types.get(name.as_str()).cloned().ok_or_else(|| {
                    CodeGenError::new(format!("unknown type for '{name}'"), Some(*span))
                })?;

                let ty = self.llvm_type_for(&var_type);
                let val = self
                    .builder
                    .build_load(ty, alloca, name)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(val)
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
                let PascalType::Pointer(pointed) = inner_type else {
                    return Err(CodeGenError::new(
                        "cannot dereference non-pointer",
                        Some(*span),
                    ));
                };
                let ty = self.llvm_type_for(&pointed);
                let val = self
                    .builder
                    .build_load(ty, ptr_val.into_pointer_value(), "deref")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(val)
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
                Ok(val)
            }
            Expr::FieldAccess {
                record,
                field,
                span,
            } => {
                let (alloca, var_type) = self.resolve_array_base(record, *span)?;
                let resolved = self.resolve_type(&var_type);
                match &resolved {
                    PascalType::Record { fields, variant } => {
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
                            Ok(val)
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
                                Ok(val)
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
                                Ok(val)
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
                if name == "low" && args.len() == 1 {
                    if let Expr::Var(vname, _) = &args[0] {
                        let var_type = self
                            .var_types
                            .get(vname.as_str())
                            .cloned()
                            .or_else(|| self.type_defs.get(vname.as_str()).cloned());
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
                        let var_type = self
                            .var_types
                            .get(vname.as_str())
                            .cloned()
                            .or_else(|| self.type_defs.get(vname.as_str()).cloned());
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
                                PascalType::Integer => {
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int(i64::MAX as u64, true)
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
                    let alloca = *self.variables.get(f_name.as_str()).ok_or_else(|| {
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
                    let alloca = *self.variables.get(f_name.as_str()).unwrap();
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
                    let alloca = *self.variables.get(f_name.as_str()).unwrap();
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
                // Indirect call through a procedural parameter.
                if let Some(PascalType::Proc {
                    params: ptypes,
                    return_type,
                }) = self.var_types.get(name.as_str()).cloned()
                {
                    let alloca = *self.variables.get(name.as_str()).unwrap();
                    let ptr_ty = self.context.ptr_type(AddressSpace::default());
                    let fp = self
                        .builder
                        .build_load(ptr_ty, alloca, "fp")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .into_pointer_value();
                    // Build the function type from the procedural signature.
                    let mut llvm_params: Vec<inkwell::types::BasicMetadataTypeEnum> = Vec::new();
                    for p in &ptypes {
                        llvm_params.push(self.llvm_type_for(p).into());
                    }
                    let fn_type = if let Some(rt) = &return_type {
                        self.llvm_type_for(rt).fn_type(&llvm_params, false)
                    } else {
                        self.context.void_type().fn_type(&llvm_params, false)
                    };
                    // Build call args
                    let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
                    for (i, arg) in args.iter().enumerate() {
                        let _ = i;
                        let v = self.compile_expr(arg)?;
                        call_args.push(v.into());
                    }
                    let ret = self
                        .builder
                        .build_indirect_call(fn_type, fp, &call_args, "icall")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return ret.try_as_basic_value().basic().ok_or_else(|| {
                        CodeGenError::new(
                            format!("procedural variable '{name}' does not return a value"),
                            Some(*span),
                        )
                    });
                }
                let func = self.module.get_function(name).ok_or_else(|| {
                    CodeGenError::new(format!("undefined function '{name}'"), Some(*span))
                })?;
                let call_args = self.compile_call_args(name, args, *span)?;
                let ret = self
                    .builder
                    .build_call(func, &call_args, "call")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                ret.try_as_basic_value().basic().ok_or_else(|| {
                    CodeGenError::new(
                        format!("function '{name}' does not return a value"),
                        Some(*span),
                    )
                })
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

    fn compile_set_constructor(
        &mut self,
        elements: &[SetElement],
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let set_ty = self.context.i64_type().array_type(4);
        let alloca = self
            .builder
            .build_alloca(set_ty, "set_tmp")
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
                    let ord_val = self.compile_expr(expr)?;
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
                    let lo_val = self.compile_expr(lo_expr)?.into_int_value();
                    let hi_val = self.compile_expr(hi_expr)?.into_int_value();
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
                        .builder
                        .build_alloca(i64_ty, "set_iter")
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

    fn resolve_type(&self, ty: &PascalType) -> PascalType {
        match ty {
            PascalType::Named(name) => self
                .type_defs
                .get(name)
                .map(|t| self.resolve_type(t))
                .unwrap_or(PascalType::Integer),
            PascalType::Pointer(inner) => PascalType::Pointer(Box::new(self.resolve_type(inner))),
            PascalType::Array { lo, hi, elem } => PascalType::Array {
                lo: *lo,
                hi: *hi,
                elem: Box::new(self.resolve_type(elem)),
            },
            PascalType::Record { fields, variant } => PascalType::Record {
                fields: fields
                    .iter()
                    .map(|(n, t)| (n.clone(), self.resolve_type(t)))
                    .collect(),
                variant: variant.as_ref().map(|v| {
                    Box::new(RecordVariant {
                        tag_name: v.tag_name.clone(),
                        tag_type: self.resolve_type(&v.tag_type),
                        variants: v
                            .variants
                            .iter()
                            .map(|(vals, vf)| {
                                (
                                    vals.clone(),
                                    vf.iter()
                                        .map(|(n, t)| (n.clone(), self.resolve_type(t)))
                                        .collect(),
                                )
                            })
                            .collect(),
                    })
                }),
            },
            PascalType::Set { elem } => PascalType::Set {
                elem: Box::new(self.resolve_type(elem)),
            },
            PascalType::File { elem } => PascalType::File {
                elem: Box::new(self.resolve_type(elem)),
            },
            PascalType::Proc {
                params,
                return_type,
            } => PascalType::Proc {
                params: params.iter().map(|t| self.resolve_type(t)).collect(),
                return_type: return_type.as_ref().map(|t| Box::new(self.resolve_type(t))),
            },
            PascalType::ConformantArray {
                lo_name,
                hi_name,
                elem,
            } => PascalType::ConformantArray {
                lo_name: lo_name.clone(),
                hi_name: hi_name.clone(),
                elem: Box::new(self.resolve_type(elem)),
            },
            PascalType::Enum { .. } | PascalType::Subrange { .. } => ty.clone(),
            other => other.clone(),
        }
    }

    fn create_debug_type(&self, ty: &PascalType) -> Option<DIType<'ctx>> {
        match ty {
            PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => self
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
                let inner_di = self.create_debug_type(inner)?;
                Some(
                    self.di_builder
                        .create_pointer_type("ptr", inner_di, 64, 0, AddressSpace::default())
                        .as_type(),
                )
            }
            _ => {
                // Arrays, records — use a generic opaque type
                self.di_builder
                    .create_basic_type("aggregate", 64, 0x05, DIFlags::ZERO)
                    .ok()
                    .map(|t| t.as_type())
            }
        }
    }

    fn llvm_type_for(&self, ty: &PascalType) -> inkwell::types::BasicTypeEnum<'ctx> {
        match ty {
            PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => {
                self.context.i64_type().as_basic_type_enum()
            }
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
            PascalType::Record { fields, variant } => {
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

    fn compile_binop(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let lhs = self.compile_expr(left)?;
        let rhs = self.compile_expr(right)?;

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
                .builder
                .build_alloca(set_ty, "set_in_tmp")
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
                .builder
                .build_alloca(set_ty, "set_l")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.builder
                .build_store(l_alloca, lhs)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let r_alloca = self
                .builder
                .build_alloca(set_ty, "set_r")
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
                .builder
                .build_alloca(set_ty, "set_out")
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

        Err(CodeGenError::new(
            "unsupported operand types for binary operator",
            Some(span),
        ))
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
                } else {
                    Err(CodeGenError::new("cannot negate non-integer", Some(span)))
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
                let lo_alloca = *self.variables.get(lo_name.as_str()).ok_or_else(|| {
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
        .map(|v| {
            let _ = bool_ty;
            v
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

    fn infer_expr_type(&self, expr: &Expr) -> PascalType {
        match expr {
            Expr::IntLit(..) => PascalType::Integer,
            Expr::RealLit(..) => PascalType::Real,
            Expr::CharLit(..) => PascalType::Char,
            Expr::StrLit(..) => PascalType::String,
            Expr::BoolLit(..) => PascalType::Boolean,
            Expr::Var(name, _) => self
                .var_types
                .get(name.as_str())
                .cloned()
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
                    | BinOp::And
                    | BinOp::Or
                    | BinOp::In => PascalType::Boolean,
                    BinOp::RealDiv => PascalType::Real,
                    _ => {
                        let lt = self.infer_expr_type(left);
                        let rt = self.infer_expr_type(right);
                        // Set operations return a set
                        if matches!(lt, PascalType::Set { .. })
                            || matches!(rt, PascalType::Set { .. })
                        {
                            PascalType::Set {
                                elem: Box::new(PascalType::Integer),
                            }
                        } else if lt == PascalType::Real || rt == PascalType::Real {
                            PascalType::Real
                        } else {
                            PascalType::Integer
                        }
                    }
                }
            }
            Expr::UnaryOp { op, .. } => match op {
                UnaryOp::Neg => PascalType::Integer,
                UnaryOp::Not => PascalType::Boolean,
            },
            Expr::Call { name, args, .. } => {
                // Built-in functions
                match name.as_str() {
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
                    "low" | "high" => PascalType::Integer,
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
                match arr_ty {
                    PascalType::Array { elem, .. } => *elem,
                    _ => PascalType::Integer,
                }
            }
            Expr::FieldAccess { record, field, .. } => {
                let rec_ty = self.infer_expr_type(record);
                let resolved = self.resolve_type(&rec_ty);
                match &resolved {
                    PascalType::Record { fields, variant } => {
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
        }
    }
}

/// Short type tag for watch-window metadata.
fn type_short(t: &PascalType) -> &'static str {
    match t {
        PascalType::Integer | PascalType::Subrange { .. } | PascalType::Enum { .. } => "int",
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
        Statement::New { target, .. } | Statement::Dispose { target, .. } => {
            record(target, out, seen)
        }
        Statement::ProcCall { args, .. } => {
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
        Statement::With {
            record_var, body, ..
        } => {
            record(record_var, out, seen);
            collect_captures(body, locals, parent_types, out, seen);
        }
        Statement::Goto { .. } | Statement::Label { .. } => {}
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
        Expr::Deref(inner, _) => collect_captures_expr(inner, locals, parent_types, out, seen),
        Expr::Call { args, .. } => {
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
        let _ = std::fs::remove_file("/tmp/turbo_pascal_console.txt");
        let status = std::process::Command::new(exe_path)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run failed");
        assert!(status.success(), "non-zero exit");
        let captured =
            std::fs::read_to_string("/tmp/turbo_pascal_console.txt").expect("capture file missing");
        assert_eq!(captured.trim(), "42", "expected 42, got: {captured:?}");
        let _ = std::fs::remove_file("/tmp/turbo_pascal_console.txt");

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
    fn program_output_goes_to_capture_file() {
        let source = "program Cap;\nbegin\n  writeln('hello capture')\nend.\n";
        let source_path = "/tmp/test_capture.pas";
        std::fs::write(source_path, source).unwrap();

        let mut parser = Parser::new(source);
        let program = parser.parse_program().unwrap();
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, source_path);
        codegen.compile(&program).unwrap();

        let exe_path = "/tmp/test_capture_out";
        codegen.emit_executable(exe_path).unwrap();

        let _ = std::fs::remove_file("/tmp/turbo_pascal_console.txt");
        let status = std::process::Command::new(exe_path)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());

        let captured = std::fs::read_to_string("/tmp/turbo_pascal_console.txt").unwrap();
        assert_eq!(
            captured.trim(),
            "hello capture",
            "capture file: {captured:?}"
        );

        let _ = std::fs::remove_file(exe_path);
        let _ = std::fs::remove_dir_all(format!("{exe_path}.dSYM"));
        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file("/tmp/turbo_pascal_console.txt");
    }
}
