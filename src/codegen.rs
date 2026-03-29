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
    DWARFEmissionKind, DWARFSourceLanguage, DebugInfoBuilder,
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

    // Symbol table
    variables: HashMap<String, PointerValue<'ctx>>,
    var_types: HashMap<String, PascalType>,

    // Current function being compiled
    current_fn: Option<FunctionValue<'ctx>>,
    current_scope: Option<DIScope<'ctx>>,

    /// Global pointer to the capture FILE* (opened at program start)
    capture_file: Option<PointerValue<'ctx>>,

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
            current_fn: None,
            current_scope: None,
            capture_file: None,
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

        // Open capture file for console output:
        //   FILE *_capture = fopen("/tmp/turbo_pascal_console.txt", "w");
        self.emit_capture_file_init(program.body.span)?;

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
        if self.module.verify().is_err() {
            return Err(CodeGenError::new("LLVM module verification failed", None));
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
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // int printf(const char *fmt, ...)
        let printf_type = i32_type.fn_type(&[ptr_type.into()], true);
        self.module.add_function("printf", printf_type, None);

        // int puts(const char *s)
        let puts_type = i32_type.fn_type(&[ptr_type.into()], false);
        self.module.add_function("puts", puts_type, None);

        // int scanf(const char *fmt, ...)
        let scanf_type = i32_type.fn_type(&[ptr_type.into()], true);
        self.module.add_function("scanf", scanf_type, None);

        // void exit(int)
        let exit_type = self.context.void_type().fn_type(&[i32_type.into()], false);
        self.module.add_function("exit", exit_type, None);

        // int putchar(int)
        let putchar_type = i32_type.fn_type(&[i32_type.into()], false);
        self.module.add_function("putchar", putchar_type, None);

        // FILE *freopen(const char *path, const char *mode, FILE *stream)
        let freopen_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
        self.module.add_function("freopen", freopen_type, None);

        // extern FILE *stdout  — declared as a global extern
        // On macOS/Linux, stdout is typically __stdoutp or a macro.
        // We use the POSIX fdopen(1, "w") approach instead — see emit_stdout_redirect.

        // FILE *fdopen(int fd, const char *mode)
        let fdopen_type = ptr_type.fn_type(&[i32_type.into(), ptr_type.into()], false);
        self.module.add_function("fdopen", fdopen_type, None);

        // int fprintf(FILE *stream, const char *fmt, ...)
        let fprintf_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
        self.module.add_function("fprintf", fprintf_type, None);

        // int fflush(FILE *stream)
        let fflush_type = i32_type.fn_type(&[ptr_type.into()], false);
        self.module.add_function("fflush", fflush_type, None);

        // FILE *fopen(const char *path, const char *mode)
        let fopen_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
        self.module.add_function("fopen", fopen_type, None);
    }

    // ── debug info helpers ───────────────────────────────

    /// Emit code to open the console capture file at program start.
    fn emit_capture_file_init(&mut self, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let fopen = self.module.get_function("fopen").unwrap();
        let path = self.builder.build_global_string_ptr(
            "/tmp/turbo_pascal_console.txt", "capture_path",
        ).map_err(|e| CodeGenError::new(e.to_string(), None))?;
        let mode = self.builder.build_global_string_ptr("w", "capture_mode")
            .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        let file_ptr = self.builder.build_call(
            fopen,
            &[path.as_pointer_value().into(), mode.as_pointer_value().into()],
            "capture_file",
        ).map_err(|e| CodeGenError::new(e.to_string(), None))?;

        // Store in a global alloca so compile_write can access it
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let alloca = self.builder.build_alloca(ptr_type, "_capture")
            .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        self.builder.build_store(alloca, file_ptr.try_as_basic_value().basic().unwrap())
            .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        self.capture_file = Some(alloca);
        Ok(())
    }

    /// Emit fprintf + fflush to the capture file (if open).
    fn emit_capture_write(&self, fmt_str: &str, args: &[inkwell::values::BasicMetadataValueEnum<'ctx>], span: Span) -> Result<(), CodeGenError> {
        let Some(capture_alloca) = self.capture_file else { return Ok(()) };
        let fprintf = self.module.get_function("fprintf").unwrap();
        let fflush = self.module.get_function("fflush").unwrap();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        let file_ptr = self.builder.build_load(ptr_type, capture_alloca, "cap_fp")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![file_ptr.into()];
        let fmt = self.builder.build_global_string_ptr(fmt_str, "cap_fmt")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        call_args.push(fmt.as_pointer_value().into());
        call_args.extend_from_slice(args);

        self.builder.build_call(fprintf, &call_args, "cap_fprintf")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder.build_call(fflush, &[file_ptr.into()], "cap_fflush")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
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

        for name in &decl.names {
            let alloca = match decl.ty {
                PascalType::Integer => {
                    let a = self.builder.build_alloca(self.context.i64_type(), name)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                    // Initialize to 0
                    self.builder.build_store(a, self.context.i64_type().const_int(0, false))
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                    a
                }
                PascalType::Boolean => {
                    let a = self.builder.build_alloca(self.context.bool_type(), name)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                    self.builder.build_store(a, self.context.bool_type().const_int(0, false))
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                    a
                }
                PascalType::String => {
                    let ptr_type = self.context.ptr_type(AddressSpace::default());
                    let a = self.builder.build_alloca(ptr_type, name)
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                    let empty = self.builder.build_global_string_ptr("", "empty_str")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                    self.builder.build_store(a, empty.as_pointer_value())
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(decl.span)))?;
                    a
                }
            };

            // Debug variable info
            let di_type = match decl.ty {
                PascalType::Integer => self.di_builder.create_basic_type("integer", 64, 0x05, DIFlags::ZERO),
                PascalType::Boolean => self.di_builder.create_basic_type("boolean", 8, 0x02, DIFlags::ZERO),
                PascalType::String => self.di_builder.create_basic_type("string", 64, 0x10, DIFlags::ZERO),
            };

            if let Ok(di_type) = di_type {
                let di_var = self.di_builder.create_auto_variable(
                    di_sub.as_debug_info_scope(),
                    name,
                    self.di_file,
                    decl.span.line,
                    di_type.as_type(),
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
            self.var_types.insert(name.clone(), decl.ty);
        }

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
            Statement::If { condition, then_branch, else_branch, span } => {
                self.compile_if(condition, then_branch, else_branch.as_ref(), *span)
            }
            Statement::While { condition, body, span } => {
                self.compile_while(condition, body, *span)
            }
            Statement::WriteLn { args, span } => self.compile_write(args, true, *span),
            Statement::Write { args, span } => self.compile_write(args, false, *span),
            Statement::ReadLn { target, span } => self.compile_readln(target, *span),
            Statement::Block(block) => self.compile_block(block),
        }
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

    fn compile_write(&mut self, args: &[Expr], newline: bool, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let printf = self.module.get_function("printf").unwrap();
        let putchar = self.module.get_function("putchar").unwrap();

        for arg in args {
            let val = self.compile_expr(arg)?;

            let arg_type = self.infer_expr_type(arg);
            match arg_type {
                PascalType::Integer => {
                    let fmt = self.builder.build_global_string_ptr("%lld", "fmt_int")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder.build_call(
                        printf,
                        &[fmt.as_pointer_value().into(), val.into()],
                        "printf_call",
                    ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.emit_capture_write("%lld", &[val.into()], span)?;
                }
                PascalType::Boolean => {
                    let true_str = self.builder.build_global_string_ptr("true", "true_str")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    let false_str = self.builder.build_global_string_ptr("false", "false_str")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

                    let bool_val = val.into_int_value();

                    let str_ptr = self.builder.build_select(
                        bool_val,
                        true_str.as_pointer_value(),
                        false_str.as_pointer_value(),
                        "bool_str",
                    ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;

                    let fmt = self.builder.build_global_string_ptr("%s", "fmt_str")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder.build_call(
                        printf,
                        &[fmt.as_pointer_value().into(), str_ptr.into()],
                        "printf_call",
                    ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.emit_capture_write("%s", &[str_ptr.into()], span)?;
                }
                PascalType::String => {
                    let fmt = self.builder.build_global_string_ptr("%s", "fmt_str")
                        .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.builder.build_call(
                        printf,
                        &[fmt.as_pointer_value().into(), val.into()],
                        "printf_call",
                    ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                    self.emit_capture_write("%s", &[val.into()], span)?;
                }
            }
        }

        if newline {
            self.builder.build_call(
                putchar,
                &[self.context.i32_type().const_int(10, false).into()],
                "nl",
            ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.emit_capture_write("\n", &[], span)?;
        }

        Ok(())
    }

    fn compile_readln(&mut self, target: &str, span: Span) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let alloca = *self.variables.get(target)
            .ok_or_else(|| CodeGenError::new(format!("undefined variable '{target}'"), Some(span)))?;

        let var_type = self.var_types.get(target).copied()
            .ok_or_else(|| CodeGenError::new(format!("unknown type for '{target}'"), Some(span)))?;

        let scanf = self.module.get_function("scanf").unwrap();

        match var_type {
            PascalType::Integer => {
                let fmt = self.builder.build_global_string_ptr("%lld", "fmt_scanf_int")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                self.builder.build_call(
                    scanf,
                    &[fmt.as_pointer_value().into(), alloca.into()],
                    "scanf_call",
                ).map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
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

    // ── expressions ──────────────────────────────────────

    fn compile_expr(&mut self, expr: &Expr) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        match expr {
            Expr::IntLit(n, _) => {
                Ok(self.context.i64_type().const_int(*n as u64, true).into())
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
                let alloca = *self.variables.get(name.as_str())
                    .ok_or_else(|| CodeGenError::new(format!("undefined variable '{name}'"), Some(*span)))?;
                let var_type = self.var_types.get(name.as_str()).copied()
                    .ok_or_else(|| CodeGenError::new(format!("unknown type for '{name}'"), Some(*span)))?;

                let ty = match var_type {
                    PascalType::Integer => self.context.i64_type().as_basic_type_enum(),
                    PascalType::Boolean => self.context.bool_type().as_basic_type_enum(),
                    PascalType::String => self.context.ptr_type(AddressSpace::default()).as_basic_type_enum(),
                };

                let val = self.builder.build_load(ty, alloca, name)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(*span)))?;
                Ok(val)
            }
            Expr::BinOp { op, left, right, span } => {
                self.compile_binop(*op, left, right, *span)
            }
            Expr::UnaryOp { op, operand, span } => {
                self.compile_unaryop(*op, operand, *span)
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
            };

            return Ok(result.into());
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
            Expr::StrLit(..) => PascalType::String,
            Expr::BoolLit(..) => PascalType::Boolean,
            Expr::Var(name, _) => self.var_types.get(name.as_str()).copied().unwrap_or(PascalType::Integer),
            Expr::BinOp { op, .. } => {
                match op {
                    BinOp::Eq | BinOp::Neq | BinOp::Lt | BinOp::Gt | BinOp::Lte | BinOp::Gte
                    | BinOp::And | BinOp::Or => PascalType::Boolean,
                    _ => PascalType::Integer,
                }
            }
            Expr::UnaryOp { op, .. } => match op {
                UnaryOp::Neg => PascalType::Integer,
                UnaryOp::Not => PascalType::Boolean,
            },
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
