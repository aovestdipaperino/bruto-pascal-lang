//! Turbo Pascal short strings (`ShortString`, `string[N]`): a length byte
//! followed by up to `N` characters, stored in place.
//!
//! In expressions a short string is read as an ordinary (C) string, so all
//! the dynamic-string operations apply unchanged; storing a string into a
//! short one copies and truncates it. Two small runtime helpers, generated
//! on first use, do the conversions.

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// The short string in the buffer at `buffer`, as a fresh AnsiString
    /// (#0 characters included).
    pub(super) fn short_to_c_string(
        &self,
        buffer: PointerValue<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i8_ty = self.context.i8_type();
        let i64_ty = self.context.i64_type();
        let len = self
            .builder
            .build_load(i8_ty, buffer, "short_len")
            .map_err(err)?
            .into_int_value();
        let len = self
            .builder
            .build_int_z_extend(len, i64_ty, "len")
            .map_err(err)?;
        let chars = unsafe {
            self.builder
                .build_in_bounds_gep(i8_ty, buffer, &[i64_ty.const_int(1, false)], "chars")
                .map_err(err)?
        };
        Ok(self
            .call_ustr_runtime("bruto_astr_from_buf", &[chars.into(), len.into()], span)?
            .unwrap())
    }

    /// Store `val` (a string, or a char) into the short string buffer at
    /// `dest`, keeping at most `max` characters.
    pub(super) fn store_short_string(
        &self,
        dest: PointerValue<'ctx>,
        val: BasicValueEnum<'ctx>,
        max: u32,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i8_ty = self.context.i8_type();
        let i64_ty = self.context.i64_type();
        if val.is_int_value() {
            // A single character.
            let ch = val.into_int_value();
            let ch = if ch.get_type().get_bit_width() == 8 {
                ch
            } else {
                self.builder
                    .build_int_truncate(ch, i8_ty, "ch")
                    .map_err(err)?
            };
            self.builder
                .build_store(dest, i8_ty.const_int(1, false))
                .map_err(err)?;
            let first = unsafe {
                self.builder
                    .build_in_bounds_gep(i8_ty, dest, &[i64_ty.const_int(1, false)], "first")
                    .map_err(err)?
            };
            self.builder.build_store(first, ch).map_err(err)?;
            return Ok(());
        }
        if !val.is_pointer_value() {
            return Err(CodeGenError::new(
                "only a string or a char can be stored in a short string",
                Some(span),
            ));
        }
        let f = self.c_string_to_short_fn();
        self.builder
            .build_call(
                f,
                &[
                    dest.into(),
                    val.into(),
                    i64_ty.const_int(max as u64, false).into(),
                ],
                "",
            )
            .map_err(err)?;
        Ok(())
    }

    /// `bruto_cstr_to_short(dest, src, max)`: store a string (nil means
    /// empty) into a short string buffer, keeping at most `max` characters.
    fn c_string_to_short_fn(&self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function("bruto_cstr_to_short") {
            return f;
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i8_ty = self.context.i8_type();
        let i64_ty = self.context.i64_type();
        let void = self.context.void_type();
        let f = self.module.add_function(
            "bruto_cstr_to_short",
            void.fn_type(&[ptr.into(), ptr.into(), i64_ty.into()], false),
            None,
        );
        f.set_linkage(Linkage::Internal);
        let b = self.context.create_builder();
        let entry = self.context.append_basic_block(f, "entry");
        let measure = self.context.append_basic_block(f, "measure");
        let copy = self.context.append_basic_block(f, "copy");
        b.position_at_end(entry);
        let dest = f.get_nth_param(0).unwrap().into_pointer_value();
        let src = f.get_nth_param(1).unwrap().into_pointer_value();
        let max = f.get_nth_param(2).unwrap().into_int_value();
        let is_nil = b.build_is_null(src, "nil").unwrap();
        b.build_conditional_branch(is_nil, copy, measure).unwrap();

        b.position_at_end(measure);
        // The AnsiString's length (#0 characters included), or strlen of
        // a C string.
        let measure_fn = self
            .module
            .get_function("bruto_astr_len")
            .expect("the string runtime is part of every program");
        let full = b
            .build_call(measure_fn, &[src.into()], "full")
            .unwrap()
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let too_long = b
            .build_int_compare(inkwell::IntPredicate::UGT, full, max, "too_long")
            .unwrap();
        let kept = b.build_select(too_long, max, full, "kept").unwrap();
        b.build_unconditional_branch(copy).unwrap();

        b.position_at_end(copy);
        let n = b.build_phi(i64_ty, "n").unwrap();
        n.add_incoming(&[(&i64_ty.const_zero(), entry), (&kept, measure)]);
        let n = n.as_basic_value().into_int_value();
        let n8 = b.build_int_truncate(n, i8_ty, "n8").unwrap();
        b.build_store(dest, n8).unwrap();
        let chars = unsafe {
            b.build_in_bounds_gep(i8_ty, dest, &[i64_ty.const_int(1, false)], "chars")
                .unwrap()
        };
        // `n` is zero when `src` is nil, so nothing is read from it then.
        b.build_memmove(chars, 1, src, 1, n).unwrap();
        b.build_return(None).unwrap();
        f
    }
}
