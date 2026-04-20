/// LLVM code generation for Mini-Pascal with DWARF debug info.
///
/// Uses inkwell to generate LLVM IR, emit object files, and link with the system linker.
/// Every statement is annotated with source locations for breakpoint support.

use crate::ast::*;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::AddressSpace;
use inkwell::OptimizationLevel;
use inkwell::debug_info::{
    AsDIScope, DICompileUnit, DIFile, DIFlags, DIFlagsConstants, DIScope, DISubprogram,
    DIType, DWARFEmissionKind, DWARFSourceLanguage, DebugInfoBuilder,
};
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
        Self { message: message.into(), span }
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
    saved_scopes: Vec<(HashMap<String, PointerValue<'ctx>>, HashMap<String, PascalType>)>,

    // Type aliases (from `type` section)
    type_defs: HashMap<String, PascalType>,

    // Enum value → ordinal mapping
    enum_values: HashMap<String, i64>,
    // Enum type name → value names
    enum_type_values: HashMap<String, Vec<String>>,

    // Procedure/function metadata
    proc_return_types: HashMap<String, Option<PascalType>>,
    proc_param_modes: HashMap<String, Vec<(String, ParamMode, PascalType)>>,

    // Current function being compiled
    current_fn: Option<FunctionValue<'ctx>>,
    current_scope: Option<DIScope<'ctx>>,

    // Goto/label support
    label_blocks: HashMap<i64, inkwell::basic_block::BasicBlock<'ctx>>,

    source_path: String,
}

impl<'ctx> CodeGen<'ctx> {
    pub fn new(context: &'ctx Context, source_path: &str) -> Self {
        let module = context.create_module("pascal_program");
        let builder = context.create_builder();

        let path = Path::new(source_path);
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("untitled.pas");
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
            current_fn: None,
            current_scope: None,
            label_blocks: HashMap::new(),
            source_path: source_path.to_string(),
        }
    }

    /// Compile a Pascal program AST into LLVM IR.
    pub fn compile(&mut self, program: &Program) -> Result<(), CodeGenError> {
        self.emit_runtime_decls();

        // Create main function
        let main_fn_type = self.context.i64_type().fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_fn_type, None);

        // Debug info for main
        let di_i64 = self.di_builder.create_basic_type(
            "integer", 64, 0x05, // DW_ATE_signed
            DIFlags::ZERO,
        ).unwrap();

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
            let bb = self.context.append_basic_block(main_fn, &format!("label_{label}"));
            self.label_blocks.insert(label, bb);
        }

        // Register type aliases
        for td in &program.type_decls {
            self.type_defs.insert(td.name.clone(), td.ty.clone());
            if let PascalType::Enum { values, .. } = &td.ty {
                for (i, val) in values.iter().enumerate() {
                    self.enum_values.insert(val.clone(), i as i64);
                }
                self.enum_type_values.insert(td.name.clone(), values.clone());
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

        // Open capture file for console output
        self.emit_capture_file_init(program.body.span)?;

        // Emit constants as variables initialized to their values
        for c in &program.consts {
            self.compile_const_decl(c, di_main)?;
        }

        // Emit variable allocations
        for var_decl in &program.vars {
            self.compile_var_decl(var_decl, di_main)?;
        }

        // Compile body
        self.compile_block(&program.body)?;

        // Return 0 — use the end_span so `end.` is breakpointable
        self.set_debug_loc(program.body.end_span);
        self.builder.build_return(Some(&self.context.i64_type().const_int(0, false)))
            .map_err(|e| CodeGenError::new(e.to_string(), None))?;

        self.di_builder.finalize();

        // Verify module
        if let Err(msg) = self.module.verify() {
            return Err(CodeGenError::new(format!("LLVM module verification failed: {}", msg.to_string()), None));
        }

        Ok(())
    }

    /// Write object file and link to executable.
    pub fn emit_executable(&self, output_path: &str) -> Result<(), String> {
        Target::initialize_native(&InitializationConfig::default())
            .map_err(|e| e.to_string())?;

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
        let link_out = std::process::Command::new("cc")
            .args([&obj_path, "-o", output_path, "-lm", "-g"])
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
        let path = self.builder.build_global_string_ptr(
            "/tmp/turbo_pascal_console.txt", "capture_path",
        ).map_err(|e| CodeGenError::new(e.to_string(), None))?;
        self.builder.build_call(func, &[path.as_pointer_value().into()], "")
            .map_err(|e| CodeGenError::new(e.to_string(), None))?;
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

    fn compile_var_decl(&mut self, decl: &VarDecl, di_sub: DISubprogram<'ctx>) -> Result<(), CodeGenError> {
        self.set_debug_loc(decl.span);
        let resolved_ty = self.resolve_type(&decl.ty);

        for name in &decl.names {
            let llvm_ty = self.llvm_type_for(&resolved_ty);
            let alloca = self.builder.build_alloca(llvm_ty, name)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;

            // Initialize scalar types to zero
            match &resolved_ty {
                PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => {
                    self.builder.build_store(alloca, self.context.i64_type().const_int(0, false))
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::Real => {
                    self.builder.build_store(alloca, self.context.f64_type().const_float(0.0))
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::Boolean => {
                    self.builder.build_store(alloca, self.context.bool_type().const_int(0, false))
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::Char => {
                    self.builder.build_store(alloca, self.context.i8_type().const_int(0, false))
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::String => {
                    let empty = self.builder.build_global_string_ptr("", "empty_str")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                    self.builder.build_store(alloca, empty.as_pointer_value())
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::Pointer(_) => {
                    let ptr_type = self.context.ptr_type(AddressSpace::default());
                    self.builder.build_store(alloca, ptr_type.const_null())
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                }
                PascalType::Array { .. } | PascalType::Record { .. } | PascalType::Set { .. } | PascalType::Named(_) => {
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
        }

        Ok(())
    }

    fn compile_const_decl(&mut self, c: &ConstDecl, di_sub: DISubprogram<'ctx>) -> Result<(), CodeGenError> {
        self.set_debug_loc(c.span);
        let val = self.compile_expr(&c.value)?;
        let ty = self.infer_expr_type(&c.value);
        let llvm_ty = self.llvm_type_for(&ty);
        let alloca = self.builder.build_alloca(llvm_ty, &c.name)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(c.span)))?;
        self.builder.build_store(alloca, val)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(c.span)))?;
        self.variables.insert(c.name.clone(), alloca);
        self.var_types.insert(c.name.clone(), ty);
        let _ = di_sub; // debug info for consts could be added later
        Ok(())
    }

    fn compile_proc_decl(&mut self, proc: &ProcDecl) -> Result<(), CodeGenError> {
        // Build parameter type list
        let mut param_llvm_types: Vec<inkwell::types::BasicMetadataTypeEnum> = Vec::new();
        let mut param_info: Vec<(String, ParamMode, PascalType)> = Vec::new();
        for group in &proc.params {
            let resolved_group_ty = self.resolve_type(&group.ty);
            for name in &group.names {
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

        let func = self.module.add_function(&proc.name, fn_type, None);

        // Store metadata
        self.proc_return_types.insert(proc.name.clone(), proc.return_type.clone());
        self.proc_param_modes.insert(proc.name.clone(), param_info.clone());

        // Debug info
        let di_i64 = self.di_builder.create_basic_type("integer", 64, 0x05, DIFlags::ZERO).unwrap();
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
                self.variables.insert(name.clone(), param_val.into_pointer_value());
                self.var_types.insert(name.clone(), resolved_ty);
            } else {
                // value param: copy into alloca
                let llvm_ty = self.llvm_type_for(&resolved_ty);
                let alloca = self.builder.build_alloca(llvm_ty, name)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
                self.builder.build_store(alloca, param_val)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
                self.variables.insert(name.clone(), alloca);
                self.var_types.insert(name.clone(), resolved_ty);
            }
        }

        // Function result variable (Pascal assigns to function name)
        if let Some(ref ret_ty) = proc.return_type {
            let llvm_ty = self.llvm_type_for(ret_ty);
            let alloca = self.builder.build_alloca(llvm_ty, &proc.name)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
            // Initialize to zero
            let zero: BasicValueEnum = match ret_ty {
                PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => self.context.i64_type().const_int(0, false).into(),
                PascalType::Real => self.context.f64_type().const_float(0.0).into(),
                PascalType::Boolean => self.context.bool_type().const_int(0, false).into(),
                PascalType::Char => self.context.i8_type().const_int(0, false).into(),
                _ => self.context.ptr_type(AddressSpace::default()).const_null().into(),
            };
            self.builder.build_store(alloca, zero)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
            self.variables.insert(proc.name.clone(), alloca);
            self.var_types.insert(proc.name.clone(), ret_ty.clone());
        }

        // Local variables
        for var_decl in &proc.vars {
            self.compile_var_decl(var_decl, di_sub)?;
        }

        // Compile body
        self.compile_block(&proc.body)?;

        // Return
        if let Some(ref ret_ty) = proc.return_type {
            let llvm_ty = self.llvm_type_for(ret_ty);
            let alloca = *self.variables.get(&proc.name).unwrap();
            let ret_val = self.builder.build_load(llvm_ty, alloca, "retval")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
            self.builder.build_return(Some(&ret_val))
                .map_err(|e| CodeGenError::new(e.to_string(), Some(proc.span)))?;
        } else {
            self.builder.build_return(None)
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
        let _ = self.builder.build_alloca(self.context.i64_type(), "_end_bp")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(block.end_span)))?;
        Ok(())
    }

    fn compile_statement(&mut self, stmt: &Statement) -> Result<(), CodeGenError> {
        match stmt {
            Statement::Assignment { target, expr, span } => {
                self.set_debug_loc(*span);
                let val = self.compile_expr(expr)?;
                let alloca = *self.variables.get(target.as_str())
                    .ok_or_else(|| CodeGenError::new(format!("undefined variable '{target}'"), Some(*span)))?;
                self.builder.build_store(alloca, val)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(())
            }
            Statement::DerefAssignment { target, expr, span } => {
                self.set_debug_loc(*span);
                let val = self.compile_expr(expr)?;
                let alloca = *self.variables.get(target.as_str())
                    .ok_or_else(|| CodeGenError::new(format!("undefined variable '{target}'"), Some(*span)))?;
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let ptr_val = self.builder.build_load(ptr_type, alloca, "ptr_val")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                self.builder.build_store(ptr_val.into_pointer_value(), val)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(())
            }
            Statement::If { condition, then_branch, else_branch, span } => {
                self.compile_if(condition, then_branch, else_branch.as_ref(), *span)
            }
            Statement::While { condition, body, span } => {
                self.compile_while(condition, body, *span)
            }
            Statement::WriteLn { args, span } => self.compile_write(args, true, *span),
            Statement::Write { args, span } => self.compile_write(args, false, *span),
            Statement::For { var, from, to, downto, body, span } => {
                self.compile_for(var, from, to, *downto, body, *span)
            }
            Statement::RepeatUntil { body, condition, span } => {
                self.compile_repeat_until(body, condition, *span)
            }
            Statement::ReadLn { target, span } => self.compile_readln(target, *span),
            Statement::Block(block) => self.compile_block(block),
            Statement::IndexAssignment { target, index, expr, span } => {
                self.set_debug_loc(*span);
                let idx_val = self.compile_expr(index)?;
                let val = self.compile_expr(expr)?;
                let alloca = *self.variables.get(target.as_str())
                    .ok_or_else(|| CodeGenError::new(format!("undefined variable '{target}'"), Some(*span)))?;
                let var_type = self.var_types.get(target.as_str()).cloned()
                    .ok_or_else(|| CodeGenError::new(format!("unknown type for '{target}'"), Some(*span)))?;
                let lo = match &var_type {
                    PascalType::Array { lo, .. } => *lo,
                    _ => return Err(CodeGenError::new(format!("'{target}' is not an array"), Some(*span))),
                };
                // Adjust index by lo (Pascal arrays can start at any value)
                let adj = self.builder.build_int_sub(
                    idx_val.into_int_value(),
                    self.context.i64_type().const_int(lo as u64, true),
                    "adj_idx",
                ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                let gep = unsafe {
                    self.builder.build_in_bounds_gep(
                        self.llvm_type_for(&var_type),
                        alloca,
                        &[self.context.i64_type().const_int(0, false), adj],
                        "arr_gep",
                    ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                };
                self.builder.build_store(gep, val)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(())
            }
            Statement::MultiIndexAssignment { target, indices, expr, span } => {
                self.set_debug_loc(*span);
                let val = self.compile_expr(expr)?;
                let alloca = *self.variables.get(target.as_str())
                    .ok_or_else(|| CodeGenError::new(format!("undefined variable '{target}'"), Some(*span)))?;
                let var_type = self.var_types.get(target.as_str()).cloned()
                    .ok_or_else(|| CodeGenError::new(format!("unknown type for '{target}'"), Some(*span)))?;

                let mut current_ptr = alloca;
                let mut current_type = var_type;
                for (dim, index_expr) in indices.iter().enumerate() {
                    let idx_val = self.compile_expr(index_expr)?;
                    let (lo, elem_ty) = match &current_type {
                        PascalType::Array { lo, elem, .. } => (*lo, elem.as_ref().clone()),
                        _ => return Err(CodeGenError::new(format!("too many indices for '{target}'"), Some(*span))),
                    };
                    let adj = self.builder.build_int_sub(
                        idx_val.into_int_value(),
                        self.context.i64_type().const_int(lo as u64, true), "adj_idx",
                    ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    let gep = unsafe {
                        self.builder.build_in_bounds_gep(
                            self.llvm_type_for(&current_type), current_ptr,
                            &[self.context.i64_type().const_int(0, false), adj], "multi_gep",
                        ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                    };
                    if dim == indices.len() - 1 {
                        self.builder.build_store(gep, val)
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    } else {
                        current_ptr = gep;
                        current_type = elem_ty;
                    }
                }
                Ok(())
            }
            Statement::FieldAssignment { target, field, expr, span } => {
                self.set_debug_loc(*span);
                let val = self.compile_expr(expr)?;
                let alloca = *self.variables.get(target.as_str())
                    .ok_or_else(|| CodeGenError::new(format!("undefined variable '{target}'"), Some(*span)))?;
                let var_type = self.var_types.get(target.as_str()).cloned()
                    .ok_or_else(|| CodeGenError::new(format!("unknown type for '{target}'"), Some(*span)))?;
                let resolved = self.resolve_type(&var_type);
                match &resolved {
                    PascalType::Record { fields, variant } => {
                        // Check fixed fields first
                        if let Some(idx) = fields.iter().position(|(n, _)| n == field) {
                            let gep = self.builder.build_struct_gep(
                                self.llvm_type_for(&resolved),
                                alloca,
                                idx as u32,
                                "field_gep",
                            ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                            self.builder.build_store(gep, val)
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                        } else if let Some(v) = variant {
                            if field == &v.tag_name {
                                // Tag field: index = fields.len()
                                let gep = self.builder.build_struct_gep(
                                    self.llvm_type_for(&resolved),
                                    alloca,
                                    fields.len() as u32,
                                    "tag_gep",
                                ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                                self.builder.build_store(gep, val)
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                            } else {
                                // Search variant fields
                                let (byte_offset, _field_ty) = self.find_variant_field(v, field)
                                    .ok_or_else(|| CodeGenError::new(format!("no field '{field}' in record"), Some(*span)))?;
                                // GEP to union array (index = fields.len() + 1)
                                let union_gep = self.builder.build_struct_gep(
                                    self.llvm_type_for(&resolved),
                                    alloca,
                                    (fields.len() + 1) as u32,
                                    "union_gep",
                                ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                                // Byte-offset into the union array
                                let byte_gep = unsafe {
                                    self.builder.build_in_bounds_gep(
                                        self.context.i8_type(),
                                        union_gep,
                                        &[self.context.i64_type().const_int(byte_offset, false)],
                                        "vfield_ptr",
                                    ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                                };
                                self.builder.build_store(byte_gep, val)
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                            }
                        } else {
                            return Err(CodeGenError::new(format!("no field '{field}' in record"), Some(*span)));
                        }
                    }
                    _ => return Err(CodeGenError::new(format!("'{target}' is not a record"), Some(*span))),
                };
                Ok(())
            }
            Statement::New { target, span } => self.compile_new(target, *span),
            Statement::Dispose { target, span } => self.compile_dispose(target, *span),
            Statement::ProcCall { name, args, span } => self.compile_proc_call(name, args, *span),
            Statement::Case { expr, branches, else_branch, span } => {
                self.compile_case(expr, branches, else_branch.as_deref(), *span)
            }
            Statement::Label { label, span } => {
                self.set_debug_loc(*span);
                let bb = *self.label_blocks.get(label)
                    .ok_or_else(|| CodeGenError::new(format!("undeclared label {label}"), Some(*span)))?;
                self.builder.build_unconditional_branch(bb)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                self.builder.position_at_end(bb);
                Ok(())
            }
            Statement::Goto { label, span } => {
                self.set_debug_loc(*span);
                let bb = *self.label_blocks.get(label)
                    .ok_or_else(|| CodeGenError::new(format!("undeclared label {label}"), Some(*span)))?;
                self.builder.build_unconditional_branch(bb)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                let func = self.current_fn.unwrap();
                let dead_bb = self.context.append_basic_block(func, "after_goto");
                self.builder.position_at_end(dead_bb);
                Ok(())
            }
            Statement::With { record_var, body, span } => {
                self.compile_with(record_var, body, *span)
            }
        }
    }

    fn compile_new(&mut self, target: &str, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let alloca = *self.variables.get(target)
            .ok_or_else(|| CodeGenError::new(format!("undefined variable '{target}'"), Some(span)))?;
        let var_type = self.var_types.get(target)
            .ok_or_else(|| CodeGenError::new(format!("unknown type for '{target}'"), Some(span)))?;

        let PascalType::Pointer(pointed) = var_type else {
            return Err(CodeGenError::new(format!("new() requires a pointer variable, got {target}"), Some(span)));
        };

        let size = self.sizeof_type(pointed);
        let bruto_alloc = self.module.get_function("bruto_alloc").unwrap();
        let ptr = self.builder.build_call(
            bruto_alloc,
            &[self.context.i64_type().const_int(size, false).into()],
            "heap_ptr",
        ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .try_as_basic_value().basic().unwrap();
        self.builder.build_store(alloca, ptr)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_dispose(&mut self, target: &str, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let alloca = *self.variables.get(target)
            .ok_or_else(|| CodeGenError::new(format!("undefined variable '{target}'"), Some(span)))?;
        let var_type = self.var_types.get(target)
            .ok_or_else(|| CodeGenError::new(format!("unknown type for '{target}'"), Some(span)))?;

        if !matches!(var_type, PascalType::Pointer(_)) {
            return Err(CodeGenError::new(format!("dispose() requires a pointer variable, got {target}"), Some(span)));
        }

        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let ptr_val = self.builder.build_load(ptr_type, alloca, "ptr_val")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let bruto_free = self.module.get_function("bruto_free").unwrap();
        self.builder.build_call(bruto_free, &[ptr_val.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        // Null out the pointer
        self.builder.build_store(alloca, ptr_type.const_null())
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_with(&mut self, record_var: &str, body: &Block, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let alloca = *self.variables.get(record_var)
            .ok_or_else(|| CodeGenError::new(format!("undefined variable '{record_var}'"), Some(span)))?;
        let var_type = self.var_types.get(record_var).cloned()
            .ok_or_else(|| CodeGenError::new(format!("unknown type for '{record_var}'"), Some(span)))?;
        let resolved = self.resolve_type(&var_type);
        let fields = match &resolved {
            PascalType::Record { fields, .. } => fields.clone(),
            _ => return Err(CodeGenError::new(format!("'{record_var}' is not a record"), Some(span))),
        };

        // Save any existing variables that would be shadowed
        let mut saved: Vec<(String, Option<PointerValue<'ctx>>, Option<PascalType>)> = Vec::new();
        for (i, (name, ty)) in fields.iter().enumerate() {
            let old_var = self.variables.remove(name);
            let old_type = self.var_types.remove(name);
            saved.push((name.clone(), old_var, old_type));

            let gep = self.builder.build_struct_gep(
                self.llvm_type_for(&resolved),
                alloca,
                i as u32,
                &format!("with_{name}"),
            ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
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

    fn compile_case(
        &mut self, expr: &Expr, branches: &[CaseBranch],
        else_branch: Option<&[Statement]>, span: Span,
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
                        self.builder.build_int_compare(
                            inkwell::IntPredicate::EQ,
                            sel_val.into_int_value(), v_val.into_int_value(), "case_eq",
                        ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                    CaseValue::Range(lo, hi) => {
                        let lo_val = self.compile_expr(lo)?;
                        let hi_val = self.compile_expr(hi)?;
                        let ge = self.builder.build_int_compare(
                            inkwell::IntPredicate::SGE,
                            sel_val.into_int_value(), lo_val.into_int_value(), "case_ge",
                        ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                        let le = self.builder.build_int_compare(
                            inkwell::IntPredicate::SLE,
                            sel_val.into_int_value(), hi_val.into_int_value(), "case_le",
                        ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                        self.builder.build_and(ge, le, "case_range")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                };
                any_match = Some(match any_match {
                    None => cmp,
                    Some(prev) => self.builder.build_or(prev, cmp, "case_or")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                });
            }

            self.builder.build_conditional_branch(any_match.unwrap(), match_bb, next_bb)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

            self.builder.position_at_end(match_bb);
            for stmt in &branch.body { self.compile_statement(stmt)?; }
            self.builder.build_unconditional_branch(end_bb)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

            self.builder.position_at_end(next_bb);
        }

        if let Some(stmts) = else_branch {
            for stmt in stmts { self.compile_statement(stmt)?; }
        }
        self.builder.build_unconditional_branch(end_bb)
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
            PascalType::Set { .. } => 32, // 4 x i64 = 256 bits
            PascalType::Array { lo, hi, elem } => {
                let count = (hi - lo + 1).max(0) as u64;
                count * self.sizeof_type(elem)
            }
            PascalType::Record { fields, variant } => {
                let fixed: u64 = fields.iter().map(|(_, t)| self.sizeof_type(t)).sum();
                let var_size = variant.as_ref().map(|v| {
                    let tag_size = self.sizeof_type(&v.tag_type);
                    let max_body = v.variants.iter()
                        .map(|(_, vf)| vf.iter().map(|(_, t)| self.sizeof_type(t)).sum::<u64>())
                        .max().unwrap_or(0);
                    tag_size + max_body
                }).unwrap_or(0);
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
                self.builder.build_int_compare(
                    inkwell::IntPredicate::NE,
                    iv,
                    iv.get_type().const_int(0, false),
                    "tobool",
                ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            }
        } else {
            return Err(CodeGenError::new("condition must be boolean or integer", Some(span)));
        };

        let func = self.current_fn.unwrap();
        let then_bb = self.context.append_basic_block(func, "then");
        let else_bb = self.context.append_basic_block(func, "else");
        let merge_bb = self.context.append_basic_block(func, "ifcont");

        self.builder.build_conditional_branch(cond_bool, then_bb, else_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Then branch
        self.builder.position_at_end(then_bb);
        self.compile_block(then_branch)?;
        self.builder.build_unconditional_branch(merge_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Else branch
        self.builder.position_at_end(else_bb);
        if let Some(else_block) = else_branch {
            self.compile_block(else_block)?;
        }
        self.builder.build_unconditional_branch(merge_bb)
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

        self.builder.build_unconditional_branch(cond_bb)
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
                self.builder.build_int_compare(
                    inkwell::IntPredicate::NE,
                    iv,
                    iv.get_type().const_int(0, false),
                    "tobool",
                ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            }
        } else {
            return Err(CodeGenError::new("while condition must be boolean or integer", Some(span)));
        };

        self.builder.build_conditional_branch(cond_bool, body_bb, after_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Body
        self.builder.position_at_end(body_bb);
        self.compile_block(body)?;
        self.builder.build_unconditional_branch(cond_bb)
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
        let alloca = *self.variables.get(var)
            .ok_or_else(|| CodeGenError::new(format!("undefined variable '{var}'"), Some(span)))?;

        // Initialize loop variable
        let from_val = self.compile_expr(from)?;
        self.builder.build_store(alloca, from_val)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        let to_val = self.compile_expr(to)?;

        let cond_bb = self.context.append_basic_block(func, "forcond");
        let body_bb = self.context.append_basic_block(func, "forbody");
        let after_bb = self.context.append_basic_block(func, "forend");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Condition: var <= to (or var >= to for downto)
        self.builder.position_at_end(cond_bb);
        self.set_debug_loc(span);
        let cur = self.builder.build_load(self.context.i64_type(), alloca, "for_cur")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let pred = if downto {
            inkwell::IntPredicate::SGE
        } else {
            inkwell::IntPredicate::SLE
        };
        let cond = self.builder.build_int_compare(pred, cur.into_int_value(), to_val.into_int_value(), "for_cmp")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder.build_conditional_branch(cond, body_bb, after_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Body
        self.builder.position_at_end(body_bb);
        self.compile_block(body)?;

        // Increment/decrement
        let cur2 = self.builder.build_load(self.context.i64_type(), alloca, "for_cur2")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        let step = if downto {
            self.builder.build_int_sub(cur2.into_int_value(), self.context.i64_type().const_int(1, false), "dec")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        } else {
            self.builder.build_int_add(cur2.into_int_value(), self.context.i64_type().const_int(1, false), "inc")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        };
        self.builder.build_store(alloca, step)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder.build_unconditional_branch(cond_bb)
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

        self.builder.build_unconditional_branch(body_bb)
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
                self.builder.build_int_compare(
                    inkwell::IntPredicate::NE, iv,
                    iv.get_type().const_int(0, false), "tobool",
                ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            }
        } else {
            return Err(CodeGenError::new("repeat-until condition must be boolean or integer", Some(span)));
        };
        // If condition true, exit; if false, loop back
        self.builder.build_conditional_branch(cond_bool, after_bb, body_bb)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        self.builder.position_at_end(after_bb);
        Ok(())
    }

    fn compile_proc_call(&mut self, name: &str, args: &[Expr], span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let func = self.module.get_function(name)
            .ok_or_else(|| CodeGenError::new(format!("undefined procedure '{name}'"), Some(span)))?;
        let call_args = self.compile_call_args(name, args, span)?;
        self.builder.build_call(func, &call_args, "")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(())
    }

    fn compile_call_args(&mut self, name: &str, args: &[Expr], span: Span) -> Result<Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>, CodeGenError> {
        let param_modes = self.proc_param_modes.get(name).cloned();
        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let is_var_param = param_modes.as_ref()
                .and_then(|p| p.get(i))
                .map(|(_, mode, _)| *mode == ParamMode::Var)
                .unwrap_or(false);
            if is_var_param {
                // Pass address of variable
                if let Expr::Var(vname, vspan) = arg {
                    let alloca = *self.variables.get(vname.as_str())
                        .ok_or_else(|| CodeGenError::new(format!("undefined variable '{vname}'"), Some(*vspan)))?;
                    call_args.push(alloca.into());
                } else {
                    return Err(CodeGenError::new("var parameter requires a variable", Some(span)));
                }
            } else {
                let val = self.compile_expr(arg)?;
                call_args.push(val.into());
            }
        }
        Ok(call_args)
    }

    fn compile_write(&mut self, args: &[Expr], newline: bool, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);

        for arg in args {
            let val = self.compile_expr(arg)?;
            let arg_type = self.infer_expr_type(arg);

            let (write_fn, capture_fn) = match arg_type {
                PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => ("bruto_write_int", "bruto_capture_write_int"),
                PascalType::Real => ("bruto_write_real", "bruto_capture_write_real"),
                PascalType::Boolean => ("bruto_write_bool", "bruto_capture_write_bool"),
                PascalType::Char => ("bruto_write_char", "bruto_capture_write_char"),
                PascalType::String | PascalType::Pointer(_) => ("bruto_write_str", "bruto_capture_write_str"),
                _ => return Err(CodeGenError::new("cannot write this type", Some(span))),
            };

            let f = self.module.get_function(write_fn).unwrap();
            self.builder.build_call(f, &[val.into()], "")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let cf = self.module.get_function(capture_fn).unwrap();
            self.builder.build_call(cf, &[val.into()], "")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        }

        if newline {
            let f = self.module.get_function("bruto_writeln").unwrap();
            self.builder.build_call(f, &[], "")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let cf = self.module.get_function("bruto_capture_writeln").unwrap();
            self.builder.build_call(cf, &[], "")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        }

        Ok(())
    }

    fn compile_readln(&mut self, target: &str, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let alloca = *self.variables.get(target)
            .ok_or_else(|| CodeGenError::new(format!("undefined variable '{target}'"), Some(span)))?;

        let var_type = self.var_types.get(target).cloned()
            .ok_or_else(|| CodeGenError::new(format!("unknown type for '{target}'"), Some(span)))?;

        match var_type {
            PascalType::Integer => {
                let f = self.module.get_function("bruto_read_int").unwrap();
                let val = self.builder.build_call(f, &[], "read_val")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .try_as_basic_value().basic().unwrap();
                self.builder.build_store(alloca, val)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            }
            _ => {
                return Err(CodeGenError::new(
                    format!("readln for {var_type:?} not yet supported"),
                    Some(span),
                ));
            }
        }

        Ok(())
    }

    // ── array base resolution (for multi-dimensional indexing) ──

    fn resolve_array_base(&mut self, expr: &Expr, span: Span) -> Result<(PointerValue<'ctx>, PascalType), CodeGenError> {
        match expr {
            Expr::Var(name, vspan) => {
                let a = *self.variables.get(name.as_str())
                    .ok_or_else(|| CodeGenError::new(format!("undefined variable '{name}'"), Some(*vspan)))?;
                let t = self.var_types.get(name.as_str()).cloned()
                    .ok_or_else(|| CodeGenError::new(format!("unknown type for '{name}'"), Some(*vspan)))?;
                Ok((a, t))
            }
            Expr::Index { array, index, span: idx_span } => {
                let (base_ptr, base_type) = self.resolve_array_base(array, *idx_span)?;
                let idx_val = self.compile_expr(index)?;
                let lo = match &base_type {
                    PascalType::Array { lo, .. } => *lo,
                    _ => return Err(CodeGenError::new("indexing non-array", Some(span))),
                };
                let elem_ty = match &base_type {
                    PascalType::Array { elem, .. } => elem.as_ref().clone(),
                    _ => unreachable!(),
                };
                let adj = self.builder.build_int_sub(
                    idx_val.into_int_value(),
                    self.context.i64_type().const_int(lo as u64, true), "adj_idx",
                ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                let gep = unsafe {
                    self.builder.build_in_bounds_gep(
                        self.llvm_type_for(&base_type), base_ptr,
                        &[self.context.i64_type().const_int(0, false), adj], "arr_gep",
                    ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                Ok((gep, elem_ty))
            }
            _ => Err(CodeGenError::new("array indexing requires a variable", Some(span))),
        }
    }

    // ── expressions ──────────────────────────────────────

    fn compile_expr(&mut self, expr: &Expr) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        match expr {
            Expr::IntLit(n, _) => {
                Ok(self.context.i64_type().const_int(*n as u64, true).into())
            }
            Expr::RealLit(r, _) => {
                Ok(self.context.f64_type().const_float(*r).into())
            }
            Expr::CharLit(c, _) => {
                Ok(self.context.i8_type().const_int(*c as u64, false).into())
            }
            Expr::StrLit(s, span) => {
                let gs = self.builder.build_global_string_ptr(s, "str_lit")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(gs.as_pointer_value().into())
            }
            Expr::BoolLit(b, _) => {
                Ok(self.context.bool_type().const_int(if *b { 1 } else { 0 }, false).into())
            }
            Expr::Var(name, span) => {
                // Check if this is an enum constant before looking up variables
                if let Some(&ordinal) = self.enum_values.get(name.as_str()) {
                    return Ok(self.context.i64_type().const_int(ordinal as u64, true).into());
                }
                let alloca = *self.variables.get(name.as_str())
                    .ok_or_else(|| CodeGenError::new(format!("undefined variable '{name}'"), Some(*span)))?;
                let var_type = self.var_types.get(name.as_str()).cloned()
                    .ok_or_else(|| CodeGenError::new(format!("unknown type for '{name}'"), Some(*span)))?;

                let ty = self.llvm_type_for(&var_type);
                let val = self.builder.build_load(ty, alloca, name)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(val)
            }
            Expr::Deref(inner, span) => {
                // inner must evaluate to a pointer; load the pointed-to value
                let ptr_val = self.compile_expr(inner)?;
                let inner_type = self.infer_expr_type(inner);
                let PascalType::Pointer(pointed) = inner_type else {
                    return Err(CodeGenError::new("cannot dereference non-pointer", Some(*span)));
                };
                let ty = self.llvm_type_for(&pointed);
                let val = self.builder.build_load(ty, ptr_val.into_pointer_value(), "deref")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(val)
            }
            Expr::Index { array, index, span } => {
                let (base_ptr, base_type) = self.resolve_array_base(array, *span)?;
                let idx_val = self.compile_expr(index)?;
                let lo = match &base_type {
                    PascalType::Array { lo, .. } => *lo,
                    _ => return Err(CodeGenError::new("indexing non-array", Some(*span))),
                };
                let elem_ty = match &base_type {
                    PascalType::Array { elem, .. } => elem.as_ref().clone(),
                    _ => unreachable!(),
                };
                let adj = self.builder.build_int_sub(
                    idx_val.into_int_value(),
                    self.context.i64_type().const_int(lo as u64, true), "adj_idx",
                ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                let gep = unsafe {
                    self.builder.build_in_bounds_gep(
                        self.llvm_type_for(&base_type), base_ptr,
                        &[self.context.i64_type().const_int(0, false), adj], "arr_gep",
                    ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                };
                let elem_llvm_ty = self.llvm_type_for(&elem_ty);
                let val = self.builder.build_load(elem_llvm_ty, gep, "arr_load")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(val)
            }
            Expr::FieldAccess { record, field, span } => {
                let (alloca, var_type) = match record.as_ref() {
                    Expr::Var(name, vspan) => {
                        let a = *self.variables.get(name.as_str())
                            .ok_or_else(|| CodeGenError::new(format!("undefined variable '{name}'"), Some(*vspan)))?;
                        let t = self.var_types.get(name.as_str()).cloned()
                            .ok_or_else(|| CodeGenError::new(format!("unknown type for '{name}'"), Some(*vspan)))?;
                        (a, t)
                    }
                    _ => return Err(CodeGenError::new("field access requires a variable", Some(*span))),
                };
                let resolved = self.resolve_type(&var_type);
                match &resolved {
                    PascalType::Record { fields, variant } => {
                        // Check fixed fields first
                        if let Some(idx) = fields.iter().position(|(n, _)| n == field) {
                            let field_ty = &fields[idx].1;
                            let gep = self.builder.build_struct_gep(
                                self.llvm_type_for(&resolved),
                                alloca,
                                idx as u32,
                                "field_gep",
                            ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                            let val = self.builder.build_load(self.llvm_type_for(field_ty), gep, "field_load")
                                .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                            Ok(val)
                        } else if let Some(v) = variant {
                            if field == &v.tag_name {
                                // Tag field: index = fields.len()
                                let gep = self.builder.build_struct_gep(
                                    self.llvm_type_for(&resolved),
                                    alloca,
                                    fields.len() as u32,
                                    "tag_gep",
                                ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                                let val = self.builder.build_load(self.llvm_type_for(&v.tag_type), gep, "tag_load")
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                                Ok(val)
                            } else {
                                // Variant field in union
                                let (byte_offset, field_ty) = self.find_variant_field(v, field)
                                    .ok_or_else(|| CodeGenError::new(format!("no field '{field}' in record"), Some(*span)))?;
                                let union_gep = self.builder.build_struct_gep(
                                    self.llvm_type_for(&resolved),
                                    alloca,
                                    (fields.len() + 1) as u32,
                                    "union_gep",
                                ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                                let byte_gep = unsafe {
                                    self.builder.build_in_bounds_gep(
                                        self.context.i8_type(),
                                        union_gep,
                                        &[self.context.i64_type().const_int(byte_offset, false)],
                                        "vfield_ptr",
                                    ).map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                                };
                                let val = self.builder.build_load(self.llvm_type_for(&field_ty), byte_gep, "vfield_load")
                                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                                Ok(val)
                            }
                        } else {
                            Err(CodeGenError::new(format!("no field '{field}' in record"), Some(*span)))
                        }
                    }
                    _ => Err(CodeGenError::new("field access on non-record", Some(*span))),
                }
            }
            Expr::Call { name, args, span } => {
                // Built-in functions
                if name == "length" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let f = self.module.get_function("bruto_str_length").unwrap();
                    let r = self.builder.build_call(f, &[val.into()], "len")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?
                        .try_as_basic_value().basic().unwrap();
                    return Ok(r);
                }
                if name == "ord" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    if val.is_pointer_value() {
                        // String: load first byte
                        let byte = self.builder.build_load(self.context.i8_type(), val.into_pointer_value(), "ord_byte")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                        let ext = self.builder.build_int_z_extend(byte.into_int_value(), self.context.i64_type(), "ord")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                        return Ok(ext.into());
                    }
                    let ext = self.builder.build_int_z_extend(val.into_int_value(), self.context.i64_type(), "ord")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(ext.into());
                }
                if name == "chr" && args.len() == 1 {
                    let val = self.compile_expr(&args[0])?;
                    let trunc = self.builder.build_int_truncate(val.into_int_value(), self.context.i8_type(), "chr")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                    return Ok(trunc.into());
                }
                let func = self.module.get_function(name)
                    .ok_or_else(|| CodeGenError::new(format!("undefined function '{name}'"), Some(*span)))?;
                let call_args = self.compile_call_args(name, args, *span)?;
                let ret = self.builder.build_call(func, &call_args, "call")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                ret.try_as_basic_value().basic()
                    .ok_or_else(|| CodeGenError::new(format!("function '{name}' does not return a value"), Some(*span)))
            }
            Expr::SetConstructor { elements, span } => {
                self.compile_set_constructor(elements, *span)
            }
            Expr::BinOp { op, left, right, span } => {
                self.compile_binop(*op, left, right, *span)
            }
            Expr::UnaryOp { op, operand, span } => {
                self.compile_unaryop(*op, operand, *span)
            }
        }
    }

    fn compile_set_constructor(
        &mut self,
        elements: &[SetElement],
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let set_ty = self.context.i64_type().array_type(4);
        let alloca = self.builder.build_alloca(set_ty, "set_tmp")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        // Zero-initialize with memset
        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let memset_fn = self.module.get_function("llvm.memset.p0.i64").unwrap_or_else(|| {
            let fn_type = self.context.void_type().fn_type(
                &[
                    i8_ptr.into(),
                    self.context.i8_type().into(),
                    self.context.i64_type().into(),
                    self.context.bool_type().into(),
                ],
                false,
            );
            self.module.add_function("llvm.memset.p0.i64", fn_type, None)
        });
        self.builder.build_call(
            memset_fn,
            &[
                alloca.into(),
                self.context.i8_type().const_int(0, false).into(),
                self.context.i64_type().const_int(32, false).into(),
                self.context.bool_type().const_int(0, false).into(),
            ],
            "",
        ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

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
                        self.builder.build_int_z_extend(ord, i64_ty, "ord_ext")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    } else {
                        ord
                    };
                    // word_idx = ord / 64, bit_idx = ord % 64
                    let word_idx = self.builder.build_int_unsigned_div(ord, sixty_four, "word_idx")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let bit_idx = self.builder.build_int_unsigned_rem(ord, sixty_four, "bit_idx")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let mask = self.builder.build_left_shift(one_i64, bit_idx, "mask")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    // GEP into the word
                    let gep = unsafe {
                        self.builder.build_in_bounds_gep(
                            set_ty, alloca,
                            &[zero, word_idx],
                            "set_word_ptr",
                        ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    };
                    let cur = self.builder.build_load(i64_ty, gep, "cur_word")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let new = self.builder.build_or(cur.into_int_value(), mask, "new_word")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder.build_store(gep, new)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                }
                SetElement::Range(lo_expr, hi_expr) => {
                    let lo_val = self.compile_expr(lo_expr)?.into_int_value();
                    let hi_val = self.compile_expr(hi_expr)?.into_int_value();
                    let lo_val = if lo_val.get_type().get_bit_width() < 64 {
                        self.builder.build_int_z_extend(lo_val, i64_ty, "lo_ext")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    } else {
                        lo_val
                    };
                    let hi_val = if hi_val.get_type().get_bit_width() < 64 {
                        self.builder.build_int_z_extend(hi_val, i64_ty, "hi_ext")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    } else {
                        hi_val
                    };

                    let current_fn = self.current_fn.unwrap();
                    let loop_bb = self.context.append_basic_block(current_fn, "set_range_loop");
                    let body_bb = self.context.append_basic_block(current_fn, "set_range_body");
                    let done_bb = self.context.append_basic_block(current_fn, "set_range_done");

                    // Store loop variable
                    let iter_alloca = self.builder.build_alloca(i64_ty, "set_iter")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder.build_store(iter_alloca, lo_val)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder.build_unconditional_branch(loop_bb)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

                    // Loop header: check iter <= hi
                    self.builder.position_at_end(loop_bb);
                    let iter_val = self.builder.build_load(i64_ty, iter_alloca, "iter")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .into_int_value();
                    let cond = self.builder.build_int_compare(
                        inkwell::IntPredicate::SLE, iter_val, hi_val, "range_cond",
                    ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder.build_conditional_branch(cond, body_bb, done_bb)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

                    // Loop body: set bit for iter_val
                    self.builder.position_at_end(body_bb);
                    let word_idx = self.builder.build_int_unsigned_div(iter_val, sixty_four, "word_idx")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let bit_idx = self.builder.build_int_unsigned_rem(iter_val, sixty_four, "bit_idx")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let mask = self.builder.build_left_shift(one_i64, bit_idx, "mask")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let gep = unsafe {
                        self.builder.build_in_bounds_gep(
                            set_ty, alloca,
                            &[zero, word_idx],
                            "set_word_ptr",
                        ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    };
                    let cur = self.builder.build_load(i64_ty, gep, "cur_word")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let new = self.builder.build_or(cur.into_int_value(), mask, "new_word")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder.build_store(gep, new)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

                    // Increment
                    let next = self.builder.build_int_add(iter_val, one_i64, "next_iter")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder.build_store(iter_alloca, next)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder.build_unconditional_branch(loop_bb)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

                    self.builder.position_at_end(done_bb);
                }
            }
        }

        // Load and return the set value
        let result = self.builder.build_load(set_ty, alloca, "set_val")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(result)
    }

    fn resolve_type(&self, ty: &PascalType) -> PascalType {
        match ty {
            PascalType::Named(name) => {
                self.type_defs.get(name).map(|t| self.resolve_type(t)).unwrap_or(PascalType::Integer)
            }
            PascalType::Pointer(inner) => PascalType::Pointer(Box::new(self.resolve_type(inner))),
            PascalType::Array { lo, hi, elem } => PascalType::Array {
                lo: *lo, hi: *hi, elem: Box::new(self.resolve_type(elem)),
            },
            PascalType::Record { fields, variant } => PascalType::Record {
                fields: fields.iter().map(|(n, t)| (n.clone(), self.resolve_type(t))).collect(),
                variant: variant.as_ref().map(|v| Box::new(RecordVariant {
                    tag_name: v.tag_name.clone(),
                    tag_type: self.resolve_type(&v.tag_type),
                    variants: v.variants.iter().map(|(vals, vf)| {
                        (vals.clone(), vf.iter().map(|(n, t)| (n.clone(), self.resolve_type(t))).collect())
                    }).collect(),
                })),
            },
            PascalType::Set { elem } => PascalType::Set { elem: Box::new(self.resolve_type(elem)) },
            PascalType::Enum { .. } | PascalType::Subrange { .. } => ty.clone(),
            other => other.clone(),
        }
    }

    fn create_debug_type(&self, ty: &PascalType) -> Option<DIType<'ctx>> {
        match ty {
            PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => {
                self.di_builder.create_basic_type("long", 64, 0x05, DIFlags::ZERO)
                    .ok().map(|t| t.as_type())
            }
            PascalType::Real => {
                self.di_builder.create_basic_type("double", 64, 0x04, DIFlags::ZERO)
                    .ok().map(|t| t.as_type())
            }
            PascalType::Boolean => {
                self.di_builder.create_basic_type("bool", 8, 0x02, DIFlags::ZERO)
                    .ok().map(|t| t.as_type())
            }
            PascalType::Char => {
                self.di_builder.create_basic_type("char", 8, 0x08, DIFlags::ZERO)
                    .ok().map(|t| t.as_type())
            }
            PascalType::String => {
                // char * — pointer to char, so lldb shows the string content
                let char_ty = self.di_builder.create_basic_type("char", 8, 0x08, DIFlags::ZERO).ok()?;
                Some(self.di_builder.create_pointer_type(
                    "char *",
                    char_ty.as_type(),
                    64,
                    0,
                    AddressSpace::default(),
                ).as_type())
            }
            PascalType::Pointer(inner) => {
                let inner_di = self.create_debug_type(inner)?;
                Some(self.di_builder.create_pointer_type(
                    "ptr",
                    inner_di,
                    64,
                    0,
                    AddressSpace::default(),
                ).as_type())
            }
            _ => {
                // Arrays, records — use a generic opaque type
                self.di_builder.create_basic_type("aggregate", 64, 0x05, DIFlags::ZERO)
                    .ok().map(|t| t.as_type())
            }
        }
    }

    fn llvm_type_for(&self, ty: &PascalType) -> inkwell::types::BasicTypeEnum<'ctx> {
        match ty {
            PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => self.context.i64_type().as_basic_type_enum(),
            PascalType::Real => self.context.f64_type().as_basic_type_enum(),
            PascalType::Boolean => self.context.bool_type().as_basic_type_enum(),
            PascalType::Char => self.context.i8_type().as_basic_type_enum(),
            PascalType::String | PascalType::Pointer(_) => {
                self.context.ptr_type(AddressSpace::default()).as_basic_type_enum()
            }
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
                    let max_size = v.variants.iter()
                        .map(|(_, vf)| vf.iter().map(|(_, t)| self.sizeof_type(t)).sum::<u64>())
                        .max().unwrap_or(0);
                    if max_size > 0 {
                        field_types.push(self.context.i8_type().array_type(max_size as u32).as_basic_type_enum());
                    }
                }
                self.context.struct_type(&field_types, false).as_basic_type_enum()
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
                self.builder.build_int_z_extend(ord, i64_ty, "ord_ext")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            } else {
                ord
            };

            // Store set to alloca so we can GEP into it
            let set_alloca = self.builder.build_alloca(set_ty, "set_in_tmp")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.builder.build_store(set_alloca, rhs)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

            let word_idx = self.builder.build_int_unsigned_div(ord, sixty_four, "word_idx")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let bit_idx = self.builder.build_int_unsigned_rem(ord, sixty_four, "bit_idx")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let mask = self.builder.build_left_shift(one_i64, bit_idx, "mask")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

            let gep = unsafe {
                self.builder.build_in_bounds_gep(
                    set_ty, set_alloca,
                    &[zero, word_idx],
                    "set_word_ptr",
                ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            };
            let word = self.builder.build_load(i64_ty, gep, "word")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let anded = self.builder.build_and(word.into_int_value(), mask, "anded")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let result = self.builder.build_int_compare(
                inkwell::IntPredicate::NE, anded, zero, "in_result",
            ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            return Ok(result.into());
        }

        // Set binary ops: +, -, * on [4 x i64]
        if lhs.is_array_value() && rhs.is_array_value() {
            let i64_ty = self.context.i64_type();
            let set_ty = i64_ty.array_type(4);
            let zero = i64_ty.const_int(0, false);

            let l_alloca = self.builder.build_alloca(set_ty, "set_l")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.builder.build_store(l_alloca, lhs)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let r_alloca = self.builder.build_alloca(set_ty, "set_r")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.builder.build_store(r_alloca, rhs)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            let out_alloca = self.builder.build_alloca(set_ty, "set_out")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

            for i in 0..4u64 {
                let idx = i64_ty.const_int(i, false);
                let l_gep = unsafe {
                    self.builder.build_in_bounds_gep(set_ty, l_alloca, &[zero, idx], "lg")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                let r_gep = unsafe {
                    self.builder.build_in_bounds_gep(set_ty, r_alloca, &[zero, idx], "rg")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                let o_gep = unsafe {
                    self.builder.build_in_bounds_gep(set_ty, out_alloca, &[zero, idx], "og")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                };
                let lw = self.builder.build_load(i64_ty, l_gep, "lw")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?.into_int_value();
                let rw = self.builder.build_load(i64_ty, r_gep, "rw")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?.into_int_value();

                let result_word = match op {
                    BinOp::Add => {
                        // Union: OR
                        self.builder.build_or(lw, rw, "union")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                    BinOp::Sub => {
                        // Difference: AND(l, NOT(r))
                        let not_r = self.builder.build_not(rw, "not_r")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                        self.builder.build_and(lw, not_r, "diff")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                    BinOp::Mul => {
                        // Intersection: AND
                        self.builder.build_and(lw, rw, "isect")
                            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    }
                    _ => return Err(CodeGenError::new("unsupported operator for set type", Some(span))),
                };
                self.builder.build_store(o_gep, result_word)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            }

            let result = self.builder.build_load(set_ty, out_alloca, "set_result")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            return Ok(result);
        }

        // Real division (/) always uses float even for integer operands
        if op == BinOp::RealDiv {
            let f64_ty = self.context.f64_type();
            let promote = |val: BasicValueEnum<'ctx>, b: &Builder<'ctx>| -> Result<inkwell::values::FloatValue<'ctx>, CodeGenError> {
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
            let result = self.builder.build_float_div(l, r, "rdiv")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            return Ok(result.into());
        }

        // Integer arithmetic
        if lhs.is_int_value() && rhs.is_int_value() {
            let l = lhs.into_int_value();
            let r = rhs.into_int_value();

            // For comparison operators on booleans, extend to i64 first
            let (l, r) = if l.get_type().get_bit_width() != r.get_type().get_bit_width() {
                let l64 = self.builder.build_int_z_extend(l, self.context.i64_type(), "zext_l")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                let r64 = self.builder.build_int_z_extend(r, self.context.i64_type(), "zext_r")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                (l64, r64)
            } else {
                (l, r)
            };

            let result = match op {
                BinOp::Add => self.builder.build_int_add(l, r, "add")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Sub => self.builder.build_int_sub(l, r, "sub")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Mul => self.builder.build_int_mul(l, r, "mul")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Div => self.builder.build_int_signed_div(l, r, "div")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Mod => self.builder.build_int_signed_rem(l, r, "mod_")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Eq => self.builder.build_int_compare(inkwell::IntPredicate::EQ, l, r, "eq")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Neq => self.builder.build_int_compare(inkwell::IntPredicate::NE, l, r, "neq")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Lt => self.builder.build_int_compare(inkwell::IntPredicate::SLT, l, r, "lt")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Gt => self.builder.build_int_compare(inkwell::IntPredicate::SGT, l, r, "gt")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Lte => self.builder.build_int_compare(inkwell::IntPredicate::SLE, l, r, "lte")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Gte => self.builder.build_int_compare(inkwell::IntPredicate::SGE, l, r, "gte")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::And => self.builder.build_and(l, r, "and")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::Or => self.builder.build_or(l, r, "or")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?,
                BinOp::RealDiv | BinOp::In => unreachable!("handled above"),
            };

            return Ok(result.into());
        }

        // Float arithmetic (including int-to-float promotion)
        if lhs.is_float_value() || rhs.is_float_value() {
            let f64_ty = self.context.f64_type();
            let promote = |val: BasicValueEnum<'ctx>, b: &Builder<'ctx>| -> Result<inkwell::values::FloatValue<'ctx>, CodeGenError> {
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
                BinOp::Add => self.builder.build_float_add(l, r, "fadd")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?.into(),
                BinOp::Sub => self.builder.build_float_sub(l, r, "fsub")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?.into(),
                BinOp::Mul => self.builder.build_float_mul(l, r, "fmul")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?.into(),
                BinOp::Div | BinOp::RealDiv => self.builder.build_float_div(l, r, "fdiv")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?.into(),
                BinOp::Eq => self.builder.build_float_compare(inkwell::FloatPredicate::OEQ, l, r, "feq")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?.into(),
                BinOp::Neq => self.builder.build_float_compare(inkwell::FloatPredicate::ONE, l, r, "fne")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?.into(),
                BinOp::Lt => self.builder.build_float_compare(inkwell::FloatPredicate::OLT, l, r, "flt")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?.into(),
                BinOp::Gt => self.builder.build_float_compare(inkwell::FloatPredicate::OGT, l, r, "fgt")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?.into(),
                BinOp::Lte => self.builder.build_float_compare(inkwell::FloatPredicate::OLE, l, r, "fle")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?.into(),
                BinOp::Gte => self.builder.build_float_compare(inkwell::FloatPredicate::OGE, l, r, "fge")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?.into(),
                _ => return Err(CodeGenError::new("unsupported operator for real type", Some(span))),
            };
            return Ok(result);
        }

        // String operations
        if lhs.is_pointer_value() && rhs.is_pointer_value() {
            let l = lhs.into_pointer_value();
            let r = rhs.into_pointer_value();
            match op {
                BinOp::Add => {
                    let concat = self.module.get_function("bruto_str_concat").unwrap();
                    let result = self.builder.build_call(concat, &[l.into(), r.into()], "concat")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .try_as_basic_value().basic().unwrap();
                    return Ok(result);
                }
                BinOp::Eq | BinOp::Neq | BinOp::Lt | BinOp::Gt | BinOp::Lte | BinOp::Gte => {
                    let cmp = self.module.get_function("bruto_str_compare").unwrap();
                    let cmp_result = self.builder.build_call(cmp, &[l.into(), r.into()], "strcmp")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                        .try_as_basic_value().basic().unwrap().into_int_value();
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
                    let result = self.builder.build_int_compare(pred, cmp_result, zero, "scmp")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    return Ok(result.into());
                }
                _ => {}
            }
        }

        Err(CodeGenError::new("unsupported operand types for binary operator", Some(span)))
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
                    let result = self.builder.build_int_neg(iv, "neg")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    Ok(result.into())
                } else {
                    Err(CodeGenError::new("cannot negate non-integer", Some(span)))
                }
            }
            UnaryOp::Not => {
                if val.is_int_value() {
                    let iv = val.into_int_value();
                    let result = self.builder.build_not(iv, "not")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    Ok(result.into())
                } else {
                    Err(CodeGenError::new("cannot apply 'not' to non-integer/boolean", Some(span)))
                }
            }
        }
    }

    // ── type inference (for write formatting) ────────────

    fn infer_expr_type(&self, expr: &Expr) -> PascalType {
        match expr {
            Expr::IntLit(..) => PascalType::Integer,
            Expr::RealLit(..) => PascalType::Real,
            Expr::CharLit(..) => PascalType::Char,
            Expr::StrLit(..) => PascalType::String,
            Expr::BoolLit(..) => PascalType::Boolean,
            Expr::Var(name, _) => self.var_types.get(name.as_str()).cloned().unwrap_or(PascalType::Integer),
            Expr::SetConstructor { .. } => PascalType::Set { elem: Box::new(PascalType::Integer) },
            Expr::BinOp { op, left, right, .. } => {
                match op {
                    BinOp::Eq | BinOp::Neq | BinOp::Lt | BinOp::Gt | BinOp::Lte | BinOp::Gte
                    | BinOp::And | BinOp::Or | BinOp::In => PascalType::Boolean,
                    BinOp::RealDiv => PascalType::Real,
                    _ => {
                        let lt = self.infer_expr_type(left);
                        let rt = self.infer_expr_type(right);
                        // Set operations return a set
                        if matches!(lt, PascalType::Set { .. }) || matches!(rt, PascalType::Set { .. }) {
                            PascalType::Set { elem: Box::new(PascalType::Integer) }
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
            Expr::Call { name, .. } => {
                // Built-in functions
                match name.as_str() {
                    "length" | "ord" => PascalType::Integer,
                    "chr" => PascalType::Char,
                    _ => self.proc_return_types.get(name.as_str())
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
                match inner_type {
                    PascalType::Pointer(pointed) => *pointed,
                    _ => PascalType::Integer, // fallback
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

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
        let captured = std::fs::read_to_string("/tmp/turbo_pascal_console.txt")
            .expect("capture file missing");
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
                "--no-use-colors", "--batch",
                "--one-line", "breakpoint set --file test_bp.pas --line 5",
                "--one-line", "run",
                "--one-line", "frame variable",
                "--one-line", "continue",
                "--one-line", "quit",
                "--", exe_path,
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
            .status().unwrap();
        assert!(status.success());

        let captured = std::fs::read_to_string("/tmp/turbo_pascal_console.txt").unwrap();
        assert_eq!(captured.trim(), "hello capture", "capture file: {captured:?}");

        let _ = std::fs::remove_file(exe_path);
        let _ = std::fs::remove_dir_all(format!("{exe_path}.dSYM"));
        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file("/tmp/turbo_pascal_console.txt");
    }
}
