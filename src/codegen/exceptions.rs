//! Exceptions: `try ... except`, `try ... finally` and `raise`.
//!
//! As in FPC on most targets, they are built on `setjmp`/`longjmp`. Each
//! `try` pushes a frame (a `jmp_buf` and a link to the enclosing frame)
//! onto the chain `BrutoExcTop` points to and calls `setjmp`; `raise`
//! (`BrutoRaise` in `rtl/objpas.pp`) records the exception object in
//! `BrutoExcObject`, unlinks the innermost frame and `longjmp`s to it,
//! or reports the exception and stops the program when there is none.
//!
//! A routine with a `try` is never optimized (`optnone`): its locals stay
//! in memory, so the values a `longjmp` comes back to are the current
//! ones. Leaving a `try` block by `exit`, `break` or `continue` unlinks
//! its frame and runs its `finally` statements first.
//!
//! The routines an exception leaves early still release their strings:
//! while it runs, a routine with string locals links a record of their
//! addresses onto the chain `BrutoCleanupTop` points to, and `raise`
//! releases the strings of every record newer than the catching frame's.

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::types::StructType;
use inkwell::values::{FunctionValue, PointerValue};
use std::collections::HashMap;

/// Words in a frame's `jmp_buf`: more than any supported system needs.
const JMP_BUF_WORDS: u32 = 64;

/// A `try` statement whose block is being compiled.
#[derive(Clone)]
pub(super) struct TryContext<'ctx> {
    /// Its exception frame.
    frame: PointerValue<'ctx>,
    /// Its `finally` statements, for a `try ... finally`.
    finally: Option<Block>,
    /// How many loops enclosed it, so `break` and `continue` know which
    /// `try` statements they leave.
    loop_depth: usize,
}

impl<'ctx> CodeGen<'ctx> {
    /// `{ jmp_buf, prev, cleanup }`: the cleanup chain's top when the
    /// frame was pushed.
    fn frame_type(&self) -> StructType<'ctx> {
        let i64_ty = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(
            &[
                i64_ty.array_type(JMP_BUF_WORDS).into(),
                ptr.into(),
                ptr.into(),
            ],
            false,
        )
    }

    /// When the program uses exceptions, link a record of the string
    /// slots `slots` onto the cleanup chain; the record, to unlink at the
    /// routine's end.
    pub(super) fn push_cleanup_record(
        &mut self,
        slots: &[PointerValue<'ctx>],
        span: Span,
    ) -> Result<Option<PointerValue<'ctx>>, CodeGenError> {
        let Some(&top) = self.global_vars.get("brutocleanuptop") else {
            return Ok(None);
        };
        if slots.is_empty() || !self.arc || self.in_runtime {
            return Ok(None);
        }
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        let rec_ty = self.context.struct_type(
            &[
                ptr.into(),
                i64_ty.into(),
                ptr.array_type(slots.len() as u32).into(),
            ],
            false,
        );
        let rec = self.entry_alloca(rec_ty, "cleanup_record").map_err(err)?;
        let prev = self
            .builder
            .build_load(ptr, top, "cleanup_prev")
            .map_err(err)?;
        let at = |i| {
            self.builder
                .build_struct_gep(rec_ty, rec, i, "cleanup_field")
        };
        self.builder
            .build_store(at(0).map_err(err)?, prev)
            .map_err(err)?;
        self.builder
            .build_store(
                at(1).map_err(err)?,
                i64_ty.const_int(slots.len() as u64, false),
            )
            .map_err(err)?;
        let array = at(2).map_err(err)?;
        for (i, slot) in slots.iter().enumerate() {
            let entry = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        ptr.array_type(slots.len() as u32),
                        array,
                        &[i64_ty.const_zero(), i64_ty.const_int(i as u64, false)],
                        "cleanup_slot",
                    )
                    .map_err(err)?
            };
            self.builder.build_store(entry, *slot).map_err(err)?;
        }
        self.builder.build_store(top, rec).map_err(err)?;
        Ok(Some(rec))
    }

    /// Unlink the record `push_cleanup_record` linked.
    pub(super) fn pop_cleanup_record(
        &mut self,
        rec: PointerValue<'ctx>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr = self.context.ptr_type(AddressSpace::default());
        let top = self.global_vars["brutocleanuptop"];
        let prev = self
            .builder
            .build_load(ptr, rec, "cleanup_prev")
            .map_err(err)?;
        self.builder.build_store(top, prev).map_err(err)?;
        Ok(())
    }

    /// A global of the exception runtime (`rtl/objpas.pp`).
    fn exception_global(&self, name: &str, span: Span) -> Result<PointerValue<'ctx>, CodeGenError> {
        self.global_vars
            .get(name)
            .copied()
            .ok_or_else(|| CodeGenError::new("the exception runtime is missing", Some(span)))
    }

    /// A routine of the exception runtime.
    fn exception_routine(
        &self,
        name: &str,
        span: Span,
    ) -> Result<FunctionValue<'ctx>, CodeGenError> {
        self.module
            .get_function(name)
            .ok_or_else(|| CodeGenError::new("the exception runtime is missing", Some(span)))
    }

    /// `_setjmp`, which returns twice: 0 when called, 1 when an exception
    /// comes back to it. (On Windows a nil frame argument makes `longjmp`
    /// skip unwinding, as C's own `setjmp` users need.)
    fn setjmp_fn(&self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function("_setjmp") {
            return f;
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i32_ty = self.context.i32_type();
        let fn_ty = if cfg!(windows) {
            i32_ty.fn_type(&[ptr.into(), ptr.into()], false)
        } else {
            i32_ty.fn_type(&[ptr.into()], false)
        };
        let f = self.module.add_function("_setjmp", fn_ty, None);
        f.add_attribute(AttributeLoc::Function, self.enum_attribute("returns_twice"));
        f
    }

    fn enum_attribute(&self, name: &str) -> Attribute {
        self.context
            .create_enum_attribute(Attribute::get_named_enum_kind_id(name), 0)
    }

    /// Keep the current routine unoptimized (see the module comment).
    fn keep_unoptimized(&self) {
        let func = self.current_fn.unwrap();
        for name in ["optnone", "noinline"] {
            let kind = Attribute::get_named_enum_kind_id(name);
            if func
                .get_enum_attribute(AttributeLoc::Function, kind)
                .is_none()
            {
                func.add_attribute(AttributeLoc::Function, self.enum_attribute(name));
            }
        }
        // `optnone` and `alwaysinline` can't go together.
        let always = Attribute::get_named_enum_kind_id("alwaysinline");
        func.remove_enum_attribute(AttributeLoc::Function, always);
    }

    /// Push a new frame and `setjmp` on it: the frame, and whether an
    /// exception came back (the `setjmp` result is not 0).
    fn push_frame(
        &mut self,
        span: Span,
    ) -> Result<(PointerValue<'ctx>, inkwell::values::IntValue<'ctx>), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        self.keep_unoptimized();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let frame_ty = self.frame_type();
        let frame = self.entry_alloca(frame_ty, "exc_frame").map_err(err)?;
        if let Some(inst) = frame.as_instruction() {
            let _ = inst.set_alignment(16);
        }
        let top = self.exception_global("brutoexctop", span)?;
        let outer = self
            .builder
            .build_load(ptr, top, "outer_frame")
            .map_err(err)?;
        let prev = self
            .builder
            .build_struct_gep(frame_ty, frame, 1, "prev")
            .map_err(err)?;
        self.builder.build_store(prev, outer).map_err(err)?;
        let cleanup_top = self.exception_global("brutocleanuptop", span)?;
        let cleanup = self
            .builder
            .build_load(ptr, cleanup_top, "cleanup_top")
            .map_err(err)?;
        let cleanup_field = self
            .builder
            .build_struct_gep(frame_ty, frame, 2, "cleanup")
            .map_err(err)?;
        self.builder
            .build_store(cleanup_field, cleanup)
            .map_err(err)?;
        self.builder.build_store(top, frame).map_err(err)?;
        let buf = self
            .builder
            .build_struct_gep(frame_ty, frame, 0, "jmp_buf")
            .map_err(err)?;
        let setjmp = self.setjmp_fn();
        let mut args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![buf.into()];
        if cfg!(windows) {
            args.push(ptr.const_null().into());
        }
        let call = self
            .builder
            .build_call(setjmp, &args, "caught")
            .map_err(err)?;
        call.add_attribute(AttributeLoc::Function, self.enum_attribute("returns_twice"));
        let result = call.try_as_basic_value().basic().unwrap().into_int_value();
        let caught = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                result,
                self.context.i32_type().const_zero(),
                "raised",
            )
            .map_err(err)?;
        Ok((frame, caught))
    }

    /// Unlink `frame`, the innermost one, from the chain.
    pub(super) fn pop_frame(
        &mut self,
        frame: PointerValue<'ctx>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr = self.context.ptr_type(AddressSpace::default());
        let prev = self
            .builder
            .build_struct_gep(self.frame_type(), frame, 1, "prev")
            .map_err(err)?;
        let outer = self
            .builder
            .build_load(ptr, prev, "outer_frame")
            .map_err(err)?;
        let top = self.exception_global("brutoexctop", span)?;
        self.builder.build_store(top, outer).map_err(err)?;
        Ok(())
    }

    /// Raise `obj` (a class instance): `BrutoRaise(Obj, Address)`, which
    /// doesn't return.
    fn raise_object(&mut self, obj: PointerValue<'ctx>, span: Span) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let raise = self.exception_routine("brutoraise", span)?;
        let here = self
            .current_fn
            .unwrap()
            .as_global_value()
            .as_pointer_value();
        self.builder
            .build_call(raise, &[obj.into(), here.into()], "")
            .map_err(err)?;
        self.builder.build_unreachable().map_err(err)?;
        let func = self.current_fn.unwrap();
        let dead = self.context.append_basic_block(func, "after_raise");
        self.builder.position_at_end(dead);
        Ok(())
    }

    /// `raise Obj` / `raise`.
    pub(super) fn compile_raise(
        &mut self,
        exception: Option<&Expr>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        let ptr = self.context.ptr_type(AddressSpace::default());
        let obj = match exception {
            Some(e) => {
                if self
                    .class_instance_of(&self.infer_declared_type(e))
                    .is_none()
                {
                    return Err(CodeGenError::new(
                        "'raise' needs an instance of a class",
                        Some(span),
                    ));
                }
                self.compile_expr(e)?.into_pointer_value()
            }
            None => {
                let slot = *self.handled_exceptions.last().ok_or_else(|| {
                    CodeGenError::new(
                        "a bare 'raise' is only allowed in an exception handler",
                        Some(span),
                    )
                })?;
                self.builder
                    .build_load(ptr, slot, "handled")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into_pointer_value()
            }
        };
        self.raise_object(obj, span)
    }

    /// `try ... except ... end` / `try ... finally ... end`.
    pub(super) fn compile_try(
        &mut self,
        body: &Block,
        handler: &TryHandler,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        self.set_debug_loc(span);
        let ptr = self.context.ptr_type(AddressSpace::default());
        let func = self.current_fn.unwrap();
        // The strings made in the block and not stored when an exception
        // leaves it are freed where it is caught.
        let pooled = self.arc && !self.in_runtime;
        let mark = if pooled {
            Some(self.pool_mark(span)?)
        } else {
            None
        };
        let (frame, caught) = self.push_frame(span)?;
        let body_bb = self.context.append_basic_block(func, "try_body");
        let landing_bb = self.context.append_basic_block(func, "try_caught");
        let done_bb = self.context.append_basic_block(func, "try_done");
        self.builder
            .build_conditional_branch(caught, landing_bb, body_bb)
            .map_err(err)?;

        // The block.
        self.builder.position_at_end(body_bb);
        let finally = match handler {
            TryHandler::Finally(b) => Some(b.clone()),
            TryHandler::Except { .. } => None,
        };
        self.try_stack.push(TryContext {
            frame,
            finally: finally.clone(),
            loop_depth: self.loop_targets.len(),
        });
        let result = self.compile_block(body);
        self.try_stack.pop();
        result?;
        if self.block_is_open() {
            self.pop_frame(frame, span)?;
            if let Some(f) = &finally {
                self.compile_block(f)?;
            }
            if self.block_is_open() {
                self.builder
                    .build_unconditional_branch(done_bb)
                    .map_err(err)?;
            }
        }

        // An exception came back: `BrutoRaise` has unlinked the frame.
        self.builder.position_at_end(landing_bb);
        self.set_debug_loc(span);
        if let Some(mark) = mark {
            self.pool_drain(mark, span)?;
        }
        let current = self.exception_global("brutoexcobject", span)?;
        let obj = self
            .builder
            .build_load(ptr, current, "exception")
            .map_err(err)?
            .into_pointer_value();
        match handler {
            TryHandler::Finally(f) => {
                // Run the statements, then pass the exception on.
                let saved = self.entry_alloca(ptr, "pending_exception").map_err(err)?;
                self.builder.build_store(saved, obj).map_err(err)?;
                self.compile_block(f)?;
                if self.block_is_open() {
                    let obj = self
                        .builder
                        .build_load(ptr, saved, "pending")
                        .map_err(err)?
                        .into_pointer_value();
                    self.raise_object(obj, span)?;
                }
            }
            TryHandler::Except {
                handlers,
                otherwise,
            } => {
                let slot = self.entry_alloca(ptr, "handled_exception").map_err(err)?;
                self.builder.build_store(slot, obj).map_err(err)?;
                for h in handlers {
                    if !self.is_class_name(&h.class) {
                        return Err(CodeGenError::new(
                            format!("'{}' is not a class", self.display(&h.class)),
                            Some(h.span),
                        ));
                    }
                    let vmt = self.vmt_of(&h.class);
                    let matches = self
                        .builder
                        .build_call(self.class_is_fn(), &[obj.into(), vmt.into()], "is")
                        .map_err(err)?
                        .try_as_basic_value()
                        .basic()
                        .unwrap()
                        .into_int_value();
                    let handle_bb = self.context.append_basic_block(func, "on_exception");
                    let next_bb = self.context.append_basic_block(func, "next_handler");
                    self.builder
                        .build_conditional_branch(matches, handle_bb, next_bb)
                        .map_err(err)?;
                    self.builder.position_at_end(handle_bb);
                    self.compile_handler(
                        h.var.as_deref(),
                        &h.class,
                        &h.body,
                        slot,
                        done_bb,
                        h.span,
                    )?;
                    self.builder.position_at_end(next_bb);
                }
                match otherwise {
                    Some(b) => self.compile_handler(None, "", b, slot, done_bb, span)?,
                    // Nothing handles it: on to the enclosing `try`.
                    None => self.raise_object(obj, span)?,
                }
            }
        }
        if self.block_is_open() {
            self.builder.build_unreachable().map_err(err)?;
        }
        self.builder.position_at_end(done_bb);
        Ok(())
    }

    /// Run handler `body` for the exception in `slot` (named `var`, of
    /// class `class`, if given), then free the exception and go on at
    /// `done`.
    fn compile_handler(
        &mut self,
        var: Option<&str>,
        class: &str,
        body: &Block,
        slot: PointerValue<'ctx>,
        done: inkwell::basic_block::BasicBlock<'ctx>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr = self.context.ptr_type(AddressSpace::default());
        let shadowed = var.map(|name| {
            let old = (
                self.variables.insert(name.to_string(), slot),
                self.var_types
                    .insert(name.to_string(), PascalType::Named(class.to_string())),
            );
            (name.to_string(), old)
        });
        self.handled_exceptions.push(slot);
        let result = self.compile_block(body);
        self.handled_exceptions.pop();
        if let Some((name, (old_var, old_type))) = shadowed {
            match old_var {
                Some(v) => self.variables.insert(name.clone(), v),
                None => self.variables.remove(&name),
            };
            match old_type {
                Some(t) => self.var_types.insert(name, t),
                None => self.var_types.remove(&name),
            };
        }
        result?;
        if self.block_is_open() {
            let obj = self.builder.build_load(ptr, slot, "handled").map_err(err)?;
            let free = self.exception_routine("brutofreeexception", span)?;
            self.builder
                .build_call(free, &[obj.into()], "")
                .map_err(err)?;
            self.builder.build_unconditional_branch(done).map_err(err)?;
        }
        Ok(())
    }

    /// Before `exit` (`loops` = `None`) or `break`/`continue` (`loops` =
    /// the loop depth they jump within): leave the `try` blocks in
    /// between, innermost first, running their `finally` statements.
    pub(super) fn leave_try_blocks(
        &mut self,
        loops: Option<usize>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let keep = match loops {
            None => 0,
            Some(depth) => self
                .try_stack
                .iter()
                .position(|t| t.loop_depth >= depth)
                .unwrap_or(self.try_stack.len()),
        };
        // What follows the jump is still inside these blocks, so the
        // stack is left as it was.
        let stack = self.try_stack.clone();
        let mut result = Ok(());
        for i in (keep..stack.len()).rev() {
            if !self.block_is_open() {
                break;
            }
            let ctx = &stack[i];
            if let Err(e) = self.pop_frame(ctx.frame, span) {
                result = Err(e);
                break;
            }
            // The `finally` statements run outside their own `try`.
            if let Some(f) = &ctx.finally {
                self.try_stack = stack[..i].to_vec();
                if let Err(e) = self.compile_block(f) {
                    result = Err(e);
                    break;
                }
            }
        }
        self.try_stack = stack;
        result?;
        Ok(())
    }

    /// Give `BrutoRunErrorHook` (set by SysUtils) the chance to turn
    /// runtime error `code` into an exception.
    pub(super) fn call_run_error_hook(
        &mut self,
        code: inkwell::values::IntValue<'ctx>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let Some(&hook) = self.global_vars.get("brutorunerrorhook") else {
            return Ok(());
        };
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i32_ty = self.context.i32_type();
        let code = if code.get_type().get_bit_width() == 32 {
            code
        } else {
            self.builder
                .build_int_truncate(code, i32_ty, "code32")
                .map_err(err)?
        };
        let func = self.current_fn.unwrap();
        let call_bb = self.context.append_basic_block(func, "run_error_hook");
        let after_bb = self.context.append_basic_block(func, "no_run_error_hook");
        let target = self
            .builder
            .build_load(ptr, hook, "hook")
            .map_err(err)?
            .into_pointer_value();
        let unset = self.builder.build_is_null(target, "unset").map_err(err)?;
        self.builder
            .build_conditional_branch(unset, after_bb, call_bb)
            .map_err(err)?;
        self.builder.position_at_end(call_bb);
        let hook_ty = self.context.void_type().fn_type(&[i32_ty.into()], false);
        self.builder
            .build_indirect_call(hook_ty, target, &[code.into()], "")
            .map_err(err)?;
        self.builder
            .build_unconditional_branch(after_bb)
            .map_err(err)?;
        self.builder.position_at_end(after_bb);
        Ok(())
    }

    /// In a class's constructor, when the program uses exceptions: if the
    /// body raises, destroy the instance (when this call made it, its
    /// `Self` argument being nil) and pass the exception on, as FPC does.
    /// The frame, to unlink when the constructor returns.
    pub(super) fn guard_constructor(
        &mut self,
        owner: &str,
        self_param: PointerValue<'ctx>,
        span: Span,
    ) -> Result<Option<PointerValue<'ctx>>, CodeGenError> {
        if !self.global_vars.contains_key("brutoexctop") {
            return Ok(None);
        }
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr = self.context.ptr_type(AddressSpace::default());
        let func = self.current_fn.unwrap();
        let self_ptr = self.method_context.unwrap().self_ptr;
        let (frame, caught) = self.push_frame(span)?;
        let body_bb = self.context.append_basic_block(func, "ctor_guarded");
        let landing_bb = self.context.append_basic_block(func, "ctor_raised");
        self.builder
            .build_conditional_branch(caught, landing_bb, body_bb)
            .map_err(err)?;
        self.builder.position_at_end(landing_bb);
        let destroy_bb = self.context.append_basic_block(func, "ctor_destroy");
        let pass_bb = self.context.append_basic_block(func, "ctor_pass_on");
        let made_here = self
            .builder
            .build_is_null(self_param, "made_here")
            .map_err(err)?;
        self.builder
            .build_conditional_branch(made_here, destroy_bb, pass_bb)
            .map_err(err)?;
        self.builder.position_at_end(destroy_bb);
        // Destroy may raise or handle exceptions of its own.
        let current = self.exception_global("brutoexcobject", span)?;
        let pending = self.entry_alloca(ptr, "ctor_exception").map_err(err)?;
        let obj = self
            .builder
            .build_load(ptr, current, "exception")
            .map_err(err)?;
        self.builder.build_store(pending, obj).map_err(err)?;
        self.call_method(
            super::objects::Receiver {
                object: owner,
                self_ptr,
                dispatch: super::objects::Dispatch::Virtual,
                vmt: None,
                class: None,
            },
            "destroy",
            &[],
            span,
        )?;
        let obj = self
            .builder
            .build_load(ptr, pending, "exception")
            .map_err(err)?;
        self.builder.build_store(current, obj).map_err(err)?;
        self.builder
            .build_unconditional_branch(pass_bb)
            .map_err(err)?;
        self.builder.position_at_end(pass_bb);
        let obj = self
            .builder
            .build_load(ptr, current, "exception")
            .map_err(err)?
            .into_pointer_value();
        self.raise_object(obj, span)?;
        if self.block_is_open() {
            self.builder.build_unreachable().map_err(err)?;
        }
        self.builder.position_at_end(body_bb);
        Ok(Some(frame))
    }
}

/// As FPC, reject a `goto` into or out of a `try` block (its `except` and
/// `finally` parts included): it would leave the exception frames wrong.
pub(super) fn check_goto_targets(body: &Block) -> Result<(), CodeGenError> {
    let mut walk = GotoWalk::default();
    walk.block(body, &mut Vec::new());
    for (label, path, span) in &walk.gotos {
        if let Some(target) = walk.labels.get(label)
            && target != path
        {
            return Err(CodeGenError::new(
                "jump in or outside of an exception block",
                Some(*span),
            ));
        }
    }
    Ok(())
}

/// Where each label and `goto` is: the `try` parts enclosing it.
#[derive(Default)]
struct GotoWalk {
    next_part: usize,
    labels: HashMap<i64, Vec<usize>>,
    gotos: Vec<(i64, Vec<usize>, Span)>,
}

impl GotoWalk {
    fn block(&mut self, block: &Block, path: &mut Vec<usize>) {
        self.list(&block.statements, path);
    }

    fn list(&mut self, stmts: &[Statement], path: &mut Vec<usize>) {
        for s in stmts {
            self.stmt(s, path);
        }
    }

    /// Walk `block` as a new `try` part.
    fn part(&mut self, block: &Block, path: &mut Vec<usize>) {
        self.next_part += 1;
        path.push(self.next_part);
        self.block(block, path);
        path.pop();
    }

    fn stmt(&mut self, stmt: &Statement, path: &mut Vec<usize>) {
        match stmt {
            Statement::Label { label, .. } => {
                self.labels.insert(*label, path.clone());
            }
            Statement::Goto { label, span } => self.gotos.push((*label, path.clone(), *span)),
            Statement::Block(b) => self.block(b, path),
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                self.block(then_branch, path);
                if let Some(e) = else_branch {
                    self.block(e, path);
                }
            }
            Statement::While { body, .. }
            | Statement::For { body, .. }
            | Statement::ForIn { body, .. }
            | Statement::With { body, .. } => self.block(body, path),
            Statement::RepeatUntil { body, .. } => self.list(body, path),
            Statement::Case {
                branches,
                else_branch,
                ..
            } => {
                for b in branches {
                    self.list(&b.body, path);
                }
                if let Some(e) = else_branch {
                    self.list(e, path);
                }
            }
            Statement::Try { body, handler, .. } => {
                self.part(body, path);
                match handler {
                    TryHandler::Except {
                        handlers,
                        otherwise,
                    } => {
                        for h in handlers {
                            self.part(&h.body, path);
                        }
                        if let Some(o) = otherwise {
                            self.part(o, path);
                        }
                    }
                    TryHandler::Finally(b) => self.part(b, path),
                }
            }
            _ => {}
        }
    }
}
