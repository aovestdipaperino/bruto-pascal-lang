//! Routines of FPC's `System` unit that Bruto provides directly, mapped
//! onto libc: memory (`Move`, `FillChar`, `GetMem`, `FreeMem`,
//! `ReAllocMem`), program control (`RunError`, `Halt`), the command line
//! (`ParamCount`, `ParamStr`) and small helpers (`Hi`, `Lo`, `Swap`,
//! `Ofs`).
//!
//! A program's own routine of the same name always wins: these are only
//! used when the name isn't declared.

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};

/// Globals holding `main`'s `argc` and `argv`, for `ParamStr`.
pub(super) const ARGC_GLOBAL: &str = "pascal.argc";
pub(super) const ARGV_GLOBAL: &str = "pascal.argv";

impl<'ctx> CodeGen<'ctx> {
    /// Whether `name` is one of the program's own routines.
    fn is_user_routine(&self, name: &str) -> bool {
        !self.bypass_user && self.proc_return_types.contains_key(name)
    }

    /// A libc function, declared on first use.
    fn libc(&self, name: &str, ty: inkwell::types::FunctionType<'ctx>) -> FunctionValue<'ctx> {
        self.module
            .get_function(name)
            .unwrap_or_else(|| self.module.add_function(name, ty, None))
    }

    /// The address of an untyped `var` argument: the place itself, or a
    /// temporary holding the value.
    fn address_of_arg(
        &mut self,
        arg: &Expr,
        span: Span,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        if self.is_addressable(arg) {
            return Ok(self.resolve_array_base(arg, span)?.0);
        }
        let val = self.compile_expr(arg)?;
        let temp = self
            .entry_alloca(val.get_type(), "arg_tmp")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder
            .build_store(temp, val)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(temp)
    }

    /// An integer argument, widened to 64 bits.
    fn int_arg(&mut self, arg: &Expr, span: Span) -> Result<IntValue<'ctx>, CodeGenError> {
        let val = self.compile_expr(arg)?;
        let val = self.first_char_if_string(val, span)?;
        if !val.is_int_value() {
            return Err(CodeGenError::new("expected an integer", Some(span)));
        }
        self.to_i64(val, span)
    }

    /// A System procedure call, if `name` is one and isn't the program's
    /// own. Returns whether it was handled.
    pub(super) fn compile_system_proc(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<bool, CodeGenError> {
        if self.is_user_routine(name) {
            return Ok(false);
        }
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();
        let void = self.context.void_type();
        match (name, args.len()) {
            // Move(Source, Dest, Count)
            ("move", 3) => {
                let src = self.address_of_arg(&args[0], span)?;
                let dst = self.address_of_arg(&args[1], span)?;
                let count = self.int_arg(&args[2], span)?;
                let memmove = self.libc(
                    "memmove",
                    ptr.fn_type(&[ptr.into(), ptr.into(), i64_ty.into()], false),
                );
                self.builder
                    .build_call(memmove, &[dst.into(), src.into(), count.into()], "")
                    .map_err(err)?;
            }
            // FillChar(Dest, Count, Value)
            ("fillchar", 3) => {
                let dst = self.address_of_arg(&args[0], span)?;
                let count = self.int_arg(&args[1], span)?;
                let value = self.int_arg(&args[2], span)?;
                let value = self
                    .builder
                    .build_int_truncate(value, i32_ty, "fill")
                    .map_err(err)?;
                let memset = self.libc(
                    "memset",
                    ptr.fn_type(&[ptr.into(), i32_ty.into(), i64_ty.into()], false),
                );
                self.builder
                    .build_call(memset, &[dst.into(), value.into(), count.into()], "")
                    .map_err(err)?;
            }
            // GetMem(P, Size)
            ("getmem", 2) => {
                let (slot, _) = self.resolve_array_base(&args[0], span)?;
                let size = self.int_arg(&args[1], span)?;
                let malloc = self.libc("malloc", ptr.fn_type(&[i64_ty.into()], false));
                let block = self
                    .builder
                    .build_call(malloc, &[size.into()], "getmem")
                    .map_err(err)?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                self.builder.build_store(slot, block).map_err(err)?;
            }
            // FreeMem(P [, Size])
            ("freemem", 1 | 2) => {
                let block = self.compile_expr(&args[0])?;
                let free = self.libc("free", void.fn_type(&[ptr.into()], false));
                self.builder
                    .build_call(free, &[block.into()], "")
                    .map_err(err)?;
            }
            // ReAllocMem(P, Size)
            ("reallocmem", 2) => {
                let (slot, _) = self.resolve_array_base(&args[0], span)?;
                let old = self.builder.build_load(ptr, slot, "old").map_err(err)?;
                let size = self.int_arg(&args[1], span)?;
                let realloc =
                    self.libc("realloc", ptr.fn_type(&[ptr.into(), i64_ty.into()], false));
                let block = self
                    .builder
                    .build_call(realloc, &[old.into(), size.into()], "realloc")
                    .map_err(err)?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                self.builder.build_store(slot, block).map_err(err)?;
            }
            // BlockRead(F, Buf, Count [, Done]) / BlockWrite(...): raw bytes
            // to or from an untyped file.
            ("blockread" | "blockwrite", 3 | 4) => {
                let fp = self.file_handle(&args[0], span)?;
                let buf = self.address_of_arg(&args[1], span)?;
                let count = self.int_arg(&args[2], span)?;
                let io = self.libc(
                    if name == "blockread" {
                        "fread"
                    } else {
                        "fwrite"
                    },
                    i64_ty.fn_type(
                        &[ptr.into(), i64_ty.into(), i64_ty.into(), ptr.into()],
                        false,
                    ),
                );
                let done = self
                    .builder
                    .build_call(
                        io,
                        &[
                            buf.into(),
                            i64_ty.const_int(1, false).into(),
                            count.into(),
                            fp.into(),
                        ],
                        "blockio",
                    )
                    .map_err(err)?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                if let Some(result) = args.get(3) {
                    let (slot, ty) = self.resolve_array_base(result, span)?;
                    self.store_value(slot, done, &ty, span)?;
                }
            }
            // Truncate(F): cut the file at the current position.
            ("truncate", 1) => {
                let fp = self.file_handle(&args[0], span)?;
                let ftell = self.libc("ftell", i64_ty.fn_type(&[ptr.into()], false));
                let fflush = self.libc("fflush", i32_ty.fn_type(&[ptr.into()], false));
                let fileno = self.libc("fileno", i32_ty.fn_type(&[ptr.into()], false));
                let ftruncate = self.libc(
                    "ftruncate",
                    i32_ty.fn_type(&[i32_ty.into(), i64_ty.into()], false),
                );
                self.builder
                    .build_call(fflush, &[fp.into()], "")
                    .map_err(err)?;
                let at = self
                    .builder
                    .build_call(ftell, &[fp.into()], "at")
                    .map_err(err)?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                let fd = self
                    .builder
                    .build_call(fileno, &[fp.into()], "fd")
                    .map_err(err)?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                self.builder
                    .build_call(ftruncate, &[fd.into(), at.into()], "")
                    .map_err(err)?;
            }
            // RunError(Code): report and stop, as FPC does.
            ("runerror", 0 | 1) => {
                let code = match args.first() {
                    Some(a) => self.int_arg(a, span)?,
                    None => i64_ty.const_zero(),
                };
                self.emit_runtime_error(code, span)?;
            }
            // Halt([Code])
            ("halt", 0 | 1) => {
                let code = match args.first() {
                    Some(a) => self.int_arg(a, span)?,
                    None => i64_ty.const_zero(),
                };
                let code = self
                    .builder
                    .build_int_truncate(code, i32_ty, "code")
                    .map_err(err)?;
                let exit = self.libc("exit", void.fn_type(&[i32_ty.into()], false));
                self.builder
                    .build_call(exit, &[code.into()], "")
                    .map_err(err)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The C `FILE*` of a file variable (the first field of the runtime's
    /// file record).
    fn file_handle(&mut self, file: &Expr, span: Span) -> Result<PointerValue<'ctx>, CodeGenError> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let record = self.compile_expr(file)?;
        if !record.is_pointer_value() {
            return Err(CodeGenError::new("expected a file variable", Some(span)));
        }
        Ok(self
            .builder
            .build_load(ptr, record.into_pointer_value(), "fp")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .into_pointer_value())
    }

    /// Print `Runtime error N` on stderr and exit with code N.
    pub(super) fn emit_runtime_error(
        &mut self,
        code: IntValue<'ctx>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i32_ty = self.context.i32_type();
        let void = self.context.void_type();
        let dprintf = self.libc(
            "dprintf",
            i32_ty.fn_type(&[i32_ty.into(), ptr.into()], true),
        );
        let format = self.string_literal("Runtime error %lld\n");
        self.builder
            .build_call(
                dprintf,
                &[
                    i32_ty.const_int(2, false).into(),
                    format.into(),
                    code.into(),
                ],
                "",
            )
            .map_err(err)?;
        let code32 = self
            .builder
            .build_int_truncate(code, i32_ty, "code")
            .map_err(err)?;
        let exit = self.libc("exit", void.fn_type(&[i32_ty.into()], false));
        self.builder
            .build_call(exit, &[code32.into()], "")
            .map_err(err)?;
        Ok(())
    }

    /// A System function call, if `name` is one and isn't the program's
    /// own.
    pub(super) fn compile_system_func(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        if self.is_user_routine(name) {
            return Ok(None);
        }
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i64_ty = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let value: BasicValueEnum<'ctx> = match (name, args.len()) {
            ("hi", 1) => {
                let v = self.int_arg(&args[0], span)?;
                let shifted = self
                    .builder
                    .build_right_shift(v, i64_ty.const_int(8, false), false, "hi")
                    .map_err(err)?;
                self.builder
                    .build_and(shifted, i64_ty.const_int(0xFF, false), "hi")
                    .map_err(err)?
                    .into()
            }
            ("lo", 1) => {
                let v = self.int_arg(&args[0], span)?;
                self.builder
                    .build_and(v, i64_ty.const_int(0xFF, false), "lo")
                    .map_err(err)?
                    .into()
            }
            // Swap(W): exchange the two bytes of a 16-bit value.
            ("swap", 1) => {
                let v = self.int_arg(&args[0], span)?;
                let high = self
                    .builder
                    .build_right_shift(v, i64_ty.const_int(8, false), false, "sh")
                    .map_err(err)?;
                let high = self
                    .builder
                    .build_and(high, i64_ty.const_int(0xFF, false), "sh")
                    .map_err(err)?;
                let low = self
                    .builder
                    .build_and(v, i64_ty.const_int(0xFF, false), "sl")
                    .map_err(err)?;
                let low = self
                    .builder
                    .build_left_shift(low, i64_ty.const_int(8, false), "sl")
                    .map_err(err)?;
                self.builder
                    .build_or(high, low, "swap")
                    .map_err(err)?
                    .into()
            }
            // Ofs(X): the address of X as a number. `Ofs(P^)` is P's value.
            ("ofs", 1) => {
                let address = match &args[0] {
                    Expr::Deref(inner, _) => self.compile_expr(inner)?.into_pointer_value(),
                    other => self.address_of_arg(other, span)?,
                };
                self.builder
                    .build_ptr_to_int(address, i64_ty, "ofs")
                    .map_err(err)?
                    .into()
            }
            ("paramcount", 0) => {
                let argc = self.argc_value(span)?;
                self.builder
                    .build_int_sub(argc, i64_ty.const_int(1, false), "paramcount")
                    .map_err(err)?
                    .into()
            }
            // ParamStr(I): argv[I], or '' when out of range.
            ("paramstr", 1) => {
                let index = self.int_arg(&args[0], span)?;
                let argc = self.argc_value(span)?;
                let argv = self
                    .builder
                    .build_load(
                        ptr,
                        self.module
                            .get_global(ARGV_GLOBAL)
                            .unwrap()
                            .as_pointer_value(),
                        "argv",
                    )
                    .map_err(err)?
                    .into_pointer_value();
                let in_range = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::ULT, index, argc, "in_range")
                    .map_err(err)?;
                let slot = unsafe {
                    self.builder
                        .build_in_bounds_gep(ptr, argv, &[index], "arg_slot")
                        .map_err(err)?
                };
                let func = self.current_fn.unwrap();
                let load_bb = self.context.append_basic_block(func, "paramstr_load");
                let done_bb = self.context.append_basic_block(func, "paramstr_done");
                let start_bb = self.builder.get_insert_block().unwrap();
                self.builder
                    .build_conditional_branch(in_range, load_bb, done_bb)
                    .map_err(err)?;
                self.builder.position_at_end(load_bb);
                let arg = self.builder.build_load(ptr, slot, "arg").map_err(err)?;
                self.builder
                    .build_unconditional_branch(done_bb)
                    .map_err(err)?;
                self.builder.position_at_end(done_bb);
                let phi = self.builder.build_phi(ptr, "paramstr").map_err(err)?;
                let empty = self.empty_string_constant();
                phi.add_incoming(&[(&empty, start_bb), (&arg, load_bb)]);
                phi.as_basic_value()
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    /// `argc` as a 64-bit integer.
    fn argc_value(&mut self, span: Span) -> Result<IntValue<'ctx>, CodeGenError> {
        let global = self
            .module
            .get_global(ARGC_GLOBAL)
            .unwrap()
            .as_pointer_value();
        let argc = self
            .builder
            .build_load(self.context.i32_type(), global, "argc")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        // `to_i64` zero-extends, which is right: argc is never negative.
        self.to_i64(argc, span)
    }

    /// The type of a System function's result, for type inference.
    pub(super) fn system_func_type(&self, name: &str, arg_count: usize) -> Option<PascalType> {
        if self.is_user_routine(name) {
            return None;
        }
        Some(match (name, arg_count) {
            ("hi" | "lo" | "swap" | "ofs", 1) | ("paramcount", 0) => PascalType::Integer,
            ("paramstr", 1) => PascalType::String,
            _ => return None,
        })
    }

    /// System variables: `FileMode` (2, read/write) and `ExitCode`.
    /// Declared before the program's globals, which can hide them.
    pub(super) fn declare_system_variables(&mut self) {
        let byte = PascalType::SizedInt {
            bits: 8,
            signed: false,
        };
        let longint = PascalType::SizedInt {
            bits: 32,
            signed: true,
        };
        for (name, ty, initial) in [("filemode", byte, 2u64), ("exitcode", longint, 0)] {
            let llvm_ty = self.llvm_type_for(&ty).into_int_type();
            let global = self
                .module
                .add_global(llvm_ty, None, &format!("pascal.{name}"));
            global.set_linkage(inkwell::module::Linkage::Internal);
            global.set_initializer(&llvm_ty.const_int(initial, false));
            self.global_vars
                .insert(name.to_string(), global.as_pointer_value());
            self.global_types.insert(name.to_string(), ty);
        }
    }

    /// Declare the `argc` / `argv` globals `main` fills in.
    pub(super) fn declare_command_line_globals(&self) {
        let i32_ty = self.context.i32_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let argc = self.module.add_global(i32_ty, None, ARGC_GLOBAL);
        argc.set_linkage(inkwell::module::Linkage::Internal);
        argc.set_initializer(&i32_ty.const_zero());
        let argv = self.module.add_global(ptr, None, ARGV_GLOBAL);
        argv.set_linkage(inkwell::module::Linkage::Internal);
        argv.set_initializer(&ptr.const_null());
    }
}
