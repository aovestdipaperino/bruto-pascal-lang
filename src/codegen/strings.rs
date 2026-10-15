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
    /// The short string in the buffer at `buffer`, as a fresh C string.
    pub(super) fn short_to_c_string(
        &self,
        buffer: PointerValue<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let f = self.short_to_c_string_fn();
        Ok(self
            .builder
            .build_call(f, &[buffer.into()], "short_str")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .try_as_basic_value()
            .basic()
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

    /// `bruto_latin1_to_utf8(dest, max, src, n) -> i64`: FPC's
    /// `UnicodeToUtf8` for 8-bit characters. Writes at most `max - 1`
    /// bytes and a NUL to `dest` (or only counts, when `dest` is nil) and
    /// returns the bytes used, the NUL included.
    pub(super) fn latin1_to_utf8_fn(&self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function("bruto_latin1_to_utf8") {
            return f;
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i8_ty = self.context.i8_type();
        let i64_ty = self.context.i64_type();
        let f = self.module.add_function(
            "bruto_latin1_to_utf8",
            i64_ty.fn_type(
                &[ptr.into(), i64_ty.into(), ptr.into(), i64_ty.into()],
                false,
            ),
            None,
        );
        f.set_linkage(Linkage::Internal);
        let b = self.context.create_builder();
        let entry = self.context.append_basic_block(f, "entry");
        let head = self.context.append_basic_block(f, "head");
        let body = self.context.append_basic_block(f, "body");
        let write = self.context.append_basic_block(f, "write");
        let write_ascii = self.context.append_basic_block(f, "write_ascii");
        let write_pair = self.context.append_basic_block(f, "write_pair");
        let next = self.context.append_basic_block(f, "next");
        let done = self.context.append_basic_block(f, "done");
        let terminate = self.context.append_basic_block(f, "terminate");
        let finish = self.context.append_basic_block(f, "finish");
        let dest = f.get_nth_param(0).unwrap().into_pointer_value();
        let max = f.get_nth_param(1).unwrap().into_int_value();
        let src = f.get_nth_param(2).unwrap().into_pointer_value();
        let n = f.get_nth_param(3).unwrap().into_int_value();
        let zero = i64_ty.const_zero();
        let one = i64_ty.const_int(1, false);

        b.position_at_end(entry);
        let counting = b.build_is_null(dest, "counting").unwrap();
        b.build_unconditional_branch(head).unwrap();

        // head: i, out
        b.position_at_end(head);
        let i = b.build_phi(i64_ty, "i").unwrap();
        let out = b.build_phi(i64_ty, "out").unwrap();
        let iv = i.as_basic_value().into_int_value();
        let outv = out.as_basic_value().into_int_value();
        let more = b
            .build_int_compare(inkwell::IntPredicate::SLT, iv, n, "more")
            .unwrap();
        b.build_conditional_branch(more, body, done).unwrap();

        // body: one character, one or two bytes
        b.position_at_end(body);
        let at = unsafe { b.build_in_bounds_gep(i8_ty, src, &[iv], "at").unwrap() };
        let c = b.build_load(i8_ty, at, "c").unwrap().into_int_value();
        let c = b.build_int_z_extend(c, i64_ty, "c64").unwrap();
        let ascii = b
            .build_int_compare(
                inkwell::IntPredicate::ULT,
                c,
                i64_ty.const_int(0x80, false),
                "ascii",
            )
            .unwrap();
        let need = b
            .build_select(ascii, one, i64_ty.const_int(2, false), "need")
            .unwrap()
            .into_int_value();
        let new_out = b.build_int_add(outv, need, "new_out").unwrap();
        // Room for the bytes and the NUL?
        let fits = b
            .build_int_compare(inkwell::IntPredicate::SLT, new_out, max, "fits")
            .unwrap();
        let ok = b.build_or(counting, fits, "ok").unwrap();
        b.build_conditional_branch(ok, write, done).unwrap();

        b.position_at_end(write);
        let skip_write = counting;
        let to_ascii = b
            .build_and(ascii, b.build_not(skip_write, "w").unwrap(), "to_ascii")
            .unwrap();
        let to_pair = b
            .build_and(
                b.build_not(ascii, "na").unwrap(),
                b.build_not(skip_write, "w2").unwrap(),
                "to_pair",
            )
            .unwrap();
        let check_pair = self.context.append_basic_block(f, "check_pair");
        b.build_conditional_branch(to_ascii, write_ascii, check_pair)
            .unwrap();
        b.position_at_end(check_pair);
        b.build_conditional_branch(to_pair, write_pair, next)
            .unwrap();

        b.position_at_end(write_ascii);
        let slot = unsafe { b.build_in_bounds_gep(i8_ty, dest, &[outv], "slot").unwrap() };
        let byte = b.build_int_truncate(c, i8_ty, "byte").unwrap();
        b.build_store(slot, byte).unwrap();
        b.build_unconditional_branch(next).unwrap();

        b.position_at_end(write_pair);
        let lead = b
            .build_or(
                b.build_right_shift(c, i64_ty.const_int(6, false), false, "hi6")
                    .unwrap(),
                i64_ty.const_int(0xC0, false),
                "lead",
            )
            .unwrap();
        let trail = b
            .build_or(
                b.build_and(c, i64_ty.const_int(0x3F, false), "lo6")
                    .unwrap(),
                i64_ty.const_int(0x80, false),
                "trail",
            )
            .unwrap();
        let slot = unsafe { b.build_in_bounds_gep(i8_ty, dest, &[outv], "slot").unwrap() };
        b.build_store(slot, b.build_int_truncate(lead, i8_ty, "l8").unwrap())
            .unwrap();
        let out1 = b.build_int_add(outv, one, "out1").unwrap();
        let slot = unsafe {
            b.build_in_bounds_gep(i8_ty, dest, &[out1], "slot2")
                .unwrap()
        };
        b.build_store(slot, b.build_int_truncate(trail, i8_ty, "t8").unwrap())
            .unwrap();
        b.build_unconditional_branch(next).unwrap();

        b.position_at_end(next);
        let i1 = b.build_int_add(iv, one, "i1").unwrap();
        b.build_unconditional_branch(head).unwrap();
        i.add_incoming(&[(&zero, entry), (&i1, next)]);
        out.add_incoming(&[(&zero, entry), (&new_out, next)]);

        b.position_at_end(done);
        b.build_conditional_branch(counting, finish, terminate)
            .unwrap();
        b.position_at_end(terminate);
        let slot = unsafe { b.build_in_bounds_gep(i8_ty, dest, &[outv], "end").unwrap() };
        b.build_store(slot, i8_ty.const_zero()).unwrap();
        b.build_unconditional_branch(finish).unwrap();
        b.position_at_end(finish);
        let total = b.build_int_add(outv, one, "total").unwrap();
        b.build_return(Some(&total)).unwrap();
        f
    }

    /// `bruto_short_to_cstr(short) -> ptr`: copy a short string's
    /// characters into a new NUL-terminated buffer.
    fn short_to_c_string_fn(&self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function("bruto_short_to_cstr") {
            return f;
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i8_ty = self.context.i8_type();
        let i64_ty = self.context.i64_type();
        let f = self.module.add_function(
            "bruto_short_to_cstr",
            ptr.fn_type(&[ptr.into()], false),
            None,
        );
        f.set_linkage(Linkage::Internal);
        let b = self.context.create_builder();
        b.position_at_end(self.context.append_basic_block(f, "entry"));
        let src = f.get_first_param().unwrap().into_pointer_value();
        let len = b.build_load(i8_ty, src, "len").unwrap().into_int_value();
        let len = b.build_int_z_extend(len, i64_ty, "len64").unwrap();
        let size = b
            .build_int_add(len, i64_ty.const_int(1, false), "size")
            .unwrap();
        let malloc = self.module.get_function("malloc").unwrap();
        let buf = b
            .build_call(malloc, &[size.into()], "buf")
            .unwrap()
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        let chars = unsafe {
            b.build_in_bounds_gep(i8_ty, src, &[i64_ty.const_int(1, false)], "chars")
                .unwrap()
        };
        b.build_memcpy(buf, 1, chars, 1, len).unwrap();
        let end = unsafe { b.build_in_bounds_gep(i8_ty, buf, &[len], "end").unwrap() };
        b.build_store(end, i8_ty.const_zero()).unwrap();
        b.build_return(Some(&buf)).unwrap();
        f
    }

    /// `bruto_cstr_to_short(dest, src, max)`: store a C string (nil means
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
        let strlen = self.module.get_function("strlen").unwrap_or_else(|| {
            self.module
                .add_function("strlen", i64_ty.fn_type(&[ptr.into()], false), None)
        });
        let full = b
            .build_call(strlen, &[src.into()], "full")
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
