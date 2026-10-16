//! Freeing AnsiStrings: reference counts and a pool of temporaries.
//!
//! Every AnsiString Bruto makes carries a reference count (see
//! `rtl/brutostrings.pp`). Storing a string into a variable, field or
//! element counts it, and the string it replaces is released; a routine
//! releases its string locals and value parameters when it returns. A
//! string no one has stored yet (the result of `A + B`, say) waits in the
//! pool, and each statement frees, when it ends, the pooled strings it made
//! that were never stored. A function's result goes back to the pool for
//! its caller's statement. Literals and C strings are never freed.
//!
//! UnicodeStrings are counted the same way (their header has the same
//! layout); in expressions they are `{ ptr }` structs.

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// Whether a value of type `ty` holds strings to count: an AnsiString
    /// or UnicodeString, or a record or array with one inside.
    pub(super) fn needs_arc(&self, ty: &PascalType) -> bool {
        if !self.arc || self.in_runtime {
            return false;
        }
        self.holds_counted_string(ty)
    }

    /// Whether a value of type `ty` holds strings or dynamic arrays
    /// (whether or not counting is on).
    pub(super) fn holds_counted(&self, ty: &PascalType) -> bool {
        self.holds_counted_string(ty)
    }

    fn holds_counted_string(&self, ty: &PascalType) -> bool {
        match self.resolve_type(ty) {
            PascalType::String | PascalType::UnicodeString | PascalType::DynArray(_) => true,
            PascalType::Interface(name) => self.interface_counted(&name),
            PascalType::Array { elem, .. } => self.holds_counted_string(&elem),
            PascalType::Record { fields, .. } => {
                fields.iter().any(|(_, t)| self.holds_counted_string(t))
            }
            _ => false,
        }
    }

    /// Call the one-pointer runtime routine `name` on `s`.
    fn arc_call(&self, name: &str, s: PointerValue<'ctx>, span: Span) -> Result<(), CodeGenError> {
        self.call_ustr_runtime(name, &[s.into()], span)?;
        Ok(())
    }

    /// Count the strings in `val` (of type `ty`), about to be stored.
    pub(super) fn arc_addref_value(
        &mut self,
        val: BasicValueEnum<'ctx>,
        ty: &PascalType,
        span: Span,
    ) -> Result<(), CodeGenError> {
        match self.resolve_type(ty) {
            PascalType::String | PascalType::DynArray(_) if val.is_pointer_value() => {
                self.arc_call("bruto_astr_addref", val.into_pointer_value(), span)
            }
            PascalType::String | PascalType::DynArray(_) => Ok(()),
            PascalType::UnicodeString if self.is_ustr(val) => {
                let p = self.ustr_ptr(val, span)?;
                self.arc_call("bruto_astr_addref", p, span)
            }
            PascalType::UnicodeString => Ok(()),
            PascalType::Interface(_) => {
                let (obj, itable) = self.interface_parts(val, span)?;
                self.call_ustr_runtime("bruto_intf_addref", &[obj.into(), itable.into()], span)?;
                Ok(())
            }
            resolved => {
                // An aggregate: from a temporary copy.
                let temp = self
                    .entry_alloca(val.get_type(), "arc_tmp")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                self.builder
                    .build_store(temp, val)
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
                self.arc_walk(temp, &resolved, "bruto_astr_addref", span)
            }
        }
    }

    /// Release the strings held in the slot at `slot` (of type `ty`).
    pub(super) fn arc_release_slot(
        &mut self,
        slot: PointerValue<'ctx>,
        ty: &PascalType,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let resolved = self.resolve_type(ty);
        self.arc_walk(slot, &resolved, "bruto_astr_release", span)
    }

    /// Hand the strings in the function result at `slot` (a record or
    /// array) back to the pool, for the caller's statement.
    pub(super) fn arc_return_slot(
        &mut self,
        slot: PointerValue<'ctx>,
        ty: &PascalType,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let resolved = self.resolve_type(ty);
        self.arc_walk(slot, &resolved, "bruto_astr_return", span)
    }

    /// Apply runtime routine `op` to every AnsiString in the memory at
    /// `slot`, of type `ty`.
    pub(super) fn arc_walk(
        &mut self,
        slot: PointerValue<'ctx>,
        ty: &PascalType,
        op: &str,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr = self.context.ptr_type(AddressSpace::default());
        match self.resolve_type(ty) {
            // A UnicodeString slot holds the pointer too, and so does a
            // dynamic array's.
            PascalType::String | PascalType::UnicodeString | PascalType::DynArray(_) => {
                let s = self
                    .builder
                    .build_load(ptr, slot, "arc_str")
                    .map_err(err)?
                    .into_pointer_value();
                self.arc_call(op, s, span)
            }
            // A record result's interfaces pass to the caller as they are.
            PascalType::Interface(_) if op == "bruto_astr_return" => Ok(()),
            PascalType::Interface(_) => {
                let value = self
                    .builder
                    .build_load(self.method_pointer_type(), slot, "arc_intf")
                    .map_err(err)?;
                let (obj, itable) = self.interface_parts(value, span)?;
                let op = op.replace("bruto_astr_", "bruto_intf_");
                self.call_ustr_runtime(&op, &[obj.into(), itable.into()], span)?;
                Ok(())
            }
            PascalType::Record { ref fields, .. } => {
                let record_ty = self.llvm_type_for(&self.resolve_type(ty));
                for (i, (_, field_ty)) in fields.iter().enumerate() {
                    if !self.holds_counted_string(field_ty) {
                        continue;
                    }
                    let field = self
                        .builder
                        .build_struct_gep(record_ty, slot, i as u32, "arc_field")
                        .map_err(err)?;
                    self.arc_walk(field, field_ty, op, span)?;
                }
                Ok(())
            }
            PascalType::Array { lo, hi, ref elem } => {
                if !self.holds_counted_string(elem) || hi < lo {
                    return Ok(());
                }
                // for i := 0 to count - 1: walk element i
                let i64_ty = self.context.i64_type();
                let array_ty = self.llvm_type_for(&self.resolve_type(ty));
                let func = self.current_fn.unwrap();
                let index = self.entry_alloca(i64_ty, "arc_i").map_err(err)?;
                self.builder
                    .build_store(index, i64_ty.const_zero())
                    .map_err(err)?;
                let head = self.context.append_basic_block(func, "arc_loop");
                let body = self.context.append_basic_block(func, "arc_elem");
                let done = self.context.append_basic_block(func, "arc_done");
                self.builder.build_unconditional_branch(head).map_err(err)?;
                self.builder.position_at_end(head);
                let i = self
                    .builder
                    .build_load(i64_ty, index, "i")
                    .map_err(err)?
                    .into_int_value();
                let count = i64_ty.const_int((hi - lo + 1) as u64, false);
                let more = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::ULT, i, count, "more")
                    .map_err(err)?;
                self.builder
                    .build_conditional_branch(more, body, done)
                    .map_err(err)?;
                self.builder.position_at_end(body);
                let element = unsafe {
                    self.builder
                        .build_in_bounds_gep(array_ty, slot, &[i64_ty.const_zero(), i], "arc_at")
                        .map_err(err)?
                };
                let elem = (**elem).clone();
                self.arc_walk(element, &elem, op, span)?;
                let next = self
                    .builder
                    .build_int_add(i, i64_ty.const_int(1, false), "next")
                    .map_err(err)?;
                self.builder.build_store(index, next).map_err(err)?;
                self.builder.build_unconditional_branch(head).map_err(err)?;
                self.builder.position_at_end(done);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The pool's size now, for [`Self::pool_drain`].
    pub(super) fn pool_mark(&self, span: Span) -> Result<IntValue<'ctx>, CodeGenError> {
        Ok(self
            .call_ustr_runtime("bruto_pool_mark", &[], span)?
            .unwrap()
            .into_int_value())
    }

    /// Free the strings pooled since `mark` that nothing stored.
    pub(super) fn pool_drain(&self, mark: IntValue<'ctx>, span: Span) -> Result<(), CodeGenError> {
        self.call_ustr_runtime("bruto_pool_drain", &[mark.into()], span)?;
        Ok(())
    }

    /// Whether the builder's current block still needs a terminator (code
    /// placed there runs).
    pub(super) fn block_is_open(&self) -> bool {
        self.builder
            .get_insert_block()
            .is_some_and(|b| b.get_terminator().is_none())
    }

    /// At a routine's return: release `locals` (names of string-holding
    /// locals and value parameters).
    pub(super) fn arc_release_locals(
        &mut self,
        locals: &[(String, PascalType)],
        span: Span,
    ) -> Result<(), CodeGenError> {
        for (name, ty) in locals {
            if let Some(&slot) = self.variables.get(name) {
                self.arc_release_slot(slot, ty, span)?;
            }
        }
        Ok(())
    }
}

/// Whether `stmt` contains a label, which a `goto` from outside could jump
/// to (so a value computed at its start wouldn't dominate its end).
pub(super) fn contains_label(stmt: &Statement) -> bool {
    match stmt {
        Statement::Label { .. } => true,
        Statement::Block(b) => block_has_label(b),
        Statement::If {
            then_branch,
            else_branch,
            ..
        } => block_has_label(then_branch) || else_branch.as_ref().is_some_and(block_has_label),
        Statement::While { body, .. }
        | Statement::For { body, .. }
        | Statement::ForIn { body, .. }
        | Statement::With { body, .. } => block_has_label(body),
        Statement::RepeatUntil { body, .. } => body.iter().any(contains_label),
        Statement::Try { body, handler, .. } => {
            block_has_label(body)
                || match handler {
                    TryHandler::Except {
                        handlers,
                        otherwise,
                    } => {
                        handlers.iter().any(|h| block_has_label(&h.body))
                            || otherwise.as_ref().is_some_and(block_has_label)
                    }
                    TryHandler::Finally(b) => block_has_label(b),
                }
        }
        Statement::Case {
            branches,
            else_branch,
            ..
        } => {
            branches.iter().any(|b| b.body.iter().any(contains_label))
                || else_branch
                    .as_ref()
                    .is_some_and(|e| e.iter().any(contains_label))
        }
        _ => false,
    }
}

fn block_has_label(b: &Block) -> bool {
    b.statements.iter().any(contains_label)
}
