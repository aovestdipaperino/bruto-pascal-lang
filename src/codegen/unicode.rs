//! `UnicodeString` and `WideChar`.
//!
//! A UnicodeString is a pointer to UTF-16 code units laid out as in FPC
//! (see `rtl/ustrings.pp`, whose routines do the work). In expressions it
//! is the struct `{ ptr }` rather than a bare pointer, and a WideChar is an
//! `i16`, so a value says what it is: a `String` (a C string, UTF-8 by
//! FPC's default code page on Unix), a `Char` (`i8`), a WideChar or a
//! UnicodeString. Conversions between them are then chosen from the
//! value, the way chars already become strings.

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::types::StructType;
use inkwell::values::{AsValueRef, BasicValue};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// The expression form of a UnicodeString, `pascal.ustr = { ptr }`.
    pub(super) fn ustr_type(&self) -> StructType<'ctx> {
        self.context
            .get_struct_type("pascal.ustr")
            .unwrap_or_else(|| {
                let t = self.context.opaque_struct_type("pascal.ustr");
                t.set_body(
                    &[self.context.ptr_type(AddressSpace::default()).into()],
                    false,
                );
                t
            })
    }

    /// Whether `val` is a UnicodeString.
    pub(super) fn is_ustr(&self, val: BasicValueEnum<'ctx>) -> bool {
        matches!(val, BasicValueEnum::StructValue(s) if s.get_type() == self.ustr_type())
    }

    /// Whether `val` is a WideChar.
    pub(super) fn is_wide_char(val: BasicValueEnum<'ctx>) -> bool {
        matches!(val, BasicValueEnum::IntValue(i) if i.get_type().get_bit_width() == 16)
    }

    /// The code-unit pointer inside UnicodeString `val`.
    pub(super) fn ustr_ptr(
        &self,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        Ok(self
            .builder
            .build_extract_value(val.into_struct_value(), 0, "ustr_ptr")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .into_pointer_value())
    }

    /// Code-unit pointer `ptr` as a UnicodeString value.
    pub(super) fn wrap_ustr(
        &self,
        ptr: PointerValue<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        Ok(self
            .builder
            .build_insert_value(self.ustr_type().get_undef(), ptr, 0, "ustr")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .into_struct_value()
            .as_basic_value_enum())
    }

    /// Call the UnicodeString runtime routine `name`.
    pub(super) fn call_ustr_runtime(
        &self,
        name: &str,
        args: &[BasicMetadataValueEnum<'ctx>],
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let f = self.module.get_function(name).ok_or_else(|| {
            CodeGenError::new(
                format!("the UnicodeString runtime ({name}) is missing"),
                Some(span),
            )
        })?;
        Ok(self
            .builder
            .build_call(f, args, name)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            .try_as_basic_value()
            .basic())
    }

    /// Call a runtime routine returning a code-unit pointer, giving the
    /// UnicodeString value.
    pub(super) fn ustr_runtime(
        &self,
        name: &str,
        args: &[BasicMetadataValueEnum<'ctx>],
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let ptr = self
            .call_ustr_runtime(name, args, span)?
            .unwrap()
            .into_pointer_value();
        self.wrap_ustr(ptr, span)
    }

    /// A character (`i8` or `i16`) as a 64-bit code.
    pub(super) fn char_code(
        &self,
        ch: IntValue<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, CodeGenError> {
        if ch.get_type().get_bit_width() >= 64 {
            return Ok(ch);
        }
        self.builder
            .build_int_z_extend(ch, self.context.i64_type(), "code")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))
    }

    /// `val` (a string, char, WideChar or UnicodeString) as a
    /// UnicodeString. Anything else is returned unchanged.
    pub(super) fn to_ustr(
        &self,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        match val {
            _ if self.is_ustr(val) => Ok(val),
            BasicValueEnum::IntValue(ch) if matches!(ch.get_type().get_bit_width(), 8 | 16) => {
                let code = self.char_code(ch, span)?;
                self.ustr_runtime("bruto_wchar_to_ustr", &[code.into()], span)
            }
            BasicValueEnum::PointerValue(p) => match self.literal_text_of(p) {
                Some(text) => self.ustr_constant(&text, span),
                None => self.ustr_runtime("bruto_utf8_to_ustr", &[p.into()], span),
            },
            _ => Ok(val),
        }
    }

    /// The Unicode text of `p`, if it is a string literal's global.
    pub(super) fn literal_text_of(&self, p: PointerValue<'ctx>) -> Option<String> {
        self.literal_texts.get(&(p.as_value_ref() as usize)).cloned()
    }

    /// A fresh UnicodeString holding `text`, built from UTF-16 data fixed
    /// at compile time.
    fn ustr_constant(&self, text: &str, span: Span) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let units: Vec<u16> = text.encode_utf16().collect();
        if units.is_empty() {
            return Ok(self.empty_ustr());
        }
        let i16_ty = self.context.i16_type();
        let values: Vec<_> = units
            .iter()
            .map(|&u| i16_ty.const_int(u64::from(u), false))
            .collect();
        let data = i16_ty.const_array(&values);
        let global = self.module.add_global(data.get_type(), None, "ustr_lit");
        global.set_linkage(inkwell::module::Linkage::Private);
        global.set_constant(true);
        global.set_unnamed_addr(true);
        global.set_initializer(&data);
        let len = self.context.i64_type().const_int(units.len() as u64, false);
        self.ustr_runtime(
            "bruto_ustr_from_units",
            &[global.as_pointer_value().into(), len.into()],
            span,
        )
    }

    /// `val` as a `String` when it is a UnicodeString or a WideChar
    /// (encoded in UTF-8); anything else is returned unchanged.
    pub(super) fn ustr_to_ansi(
        &self,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        if self.is_ustr(val) {
            let ptr = self.ustr_ptr(val, span)?;
            return Ok(self
                .call_ustr_runtime("bruto_ustr_to_utf8", &[ptr.into()], span)?
                .unwrap());
        }
        if Self::is_wide_char(val) {
            let code = self.char_code(val.into_int_value(), span)?;
            return Ok(self
                .call_ustr_runtime("bruto_wchar_to_utf8", &[code.into()], span)?
                .unwrap());
        }
        Ok(val)
    }

    /// `val` stored into a WideChar: a one-character string literal
    /// (`'a'`, `'打'`) is its first character; integers and chars are
    /// cut or extended to 16 bits.
    pub(super) fn to_wide_char(
        &self,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i16_ty = self.context.i16_type();
        if let BasicValueEnum::PointerValue(p) = val
            && let Some(text) = self.literal_text_of(p)
        {
            let unit = text.encode_utf16().next().unwrap_or(0);
            return Ok(i16_ty.const_int(u64::from(unit), false).into());
        }
        let code = match val {
            BasicValueEnum::PointerValue(p) => self
                .call_ustr_runtime("bruto_utf8_first_wchar", &[p.into()], span)?
                .unwrap()
                .into_int_value(),
            _ if self.is_ustr(val) => {
                let ptr = self.ustr_ptr(val, span)?;
                let first = self
                    .call_ustr_runtime(
                        "bruto_ustr_copy",
                        &[
                            ptr.into(),
                            self.context.i64_type().const_int(1, false).into(),
                            self.context.i64_type().const_int(1, false).into(),
                        ],
                        span,
                    )?
                    .unwrap()
                    .into_pointer_value();
                let is_nil = self.builder.build_is_null(first, "empty").map_err(err)?;
                let unit = self
                    .builder
                    .build_load(i16_ty, first, "unit")
                    .map_err(err)?
                    .into_int_value();
                let unit = self
                    .builder
                    .build_select(is_nil, i16_ty.const_zero(), unit, "first")
                    .map_err(err)?;
                return Ok(unit);
            }
            BasicValueEnum::IntValue(i) => i,
            _ => return Ok(val),
        };
        let width = code.get_type().get_bit_width();
        Ok(if width > 16 {
            self.builder
                .build_int_truncate(code, i16_ty, "wchar")
                .map_err(err)?
                .into()
        } else if width < 16 {
            self.builder
                .build_int_z_extend(code, i16_ty, "wchar")
                .map_err(err)?
                .into()
        } else {
            code.into()
        })
    }

    /// Whether a value of type `ty` holds a UnicodeString (in a field
    /// or an element), which must start out nil.
    pub(super) fn holds_unicode_string(&self, ty: &PascalType) -> bool {
        match self.resolve_type(ty) {
            PascalType::UnicodeString => true,
            PascalType::Array { elem, .. } => self.holds_unicode_string(&elem),
            PascalType::Record {
                fields, variant, ..
            } => {
                fields.iter().any(|(_, t)| self.holds_unicode_string(t))
                    || variant.is_some_and(|v| {
                        v.variants.iter().any(|(_, fs)| {
                            fs.iter().any(|(_, t)| self.holds_unicode_string(t))
                        })
                    })
            }
            _ => false,
        }
    }

    /// Whether `ty` is UnicodeString.
    pub(super) fn is_unicode_type(&self, ty: &PascalType) -> bool {
        self.resolve_type(ty) == PascalType::UnicodeString
    }

    /// `S[i]` of UnicodeString `s` (1-based), a WideChar.
    pub(super) fn ustr_char_at(
        &self,
        s: BasicValueEnum<'ctx>,
        index: IntValue<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr = self.ustr_ptr(s, span)?;
        let i64_ty = self.context.i64_type();
        let index = self
            .builder
            .build_int_s_extend_or_bit_cast(index, i64_ty, "idx")
            .map_err(err)?;
        let offset = self
            .builder
            .build_int_sub(index, i64_ty.const_int(1, false), "off")
            .map_err(err)?;
        let at = unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i16_type(), ptr, &[offset], "wchar_at")
                .map_err(err)?
        };
        self.builder
            .build_load(self.context.i16_type(), at, "wchar")
            .map_err(err)
    }

    /// Two strings compared, at least one a UnicodeString: the runtime's
    /// three-way result as an `i64`.
    pub(super) fn ustr_compare(
        &self,
        a: BasicValueEnum<'ctx>,
        b: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, CodeGenError> {
        let a = self.to_ustr(a, span)?;
        let b = self.to_ustr(b, span)?;
        let a = self.ustr_ptr(a, span)?;
        let b = self.ustr_ptr(b, span)?;
        Ok(self
            .call_ustr_runtime("bruto_ustr_compare", &[a.into(), b.into()], span)?
            .unwrap()
            .into_int_value())
    }

    /// `a + b`, at least one of them a UnicodeString or a WideChar.
    pub(super) fn ustr_concat(
        &self,
        a: BasicValueEnum<'ctx>,
        b: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let a = self.to_ustr(a, span)?;
        let b = self.to_ustr(b, span)?;
        let a = self.ustr_ptr(a, span)?;
        let b = self.ustr_ptr(b, span)?;
        self.ustr_runtime("bruto_ustr_concat", &[a.into(), b.into()], span)
    }

    /// The null UnicodeString, for initializers.
    pub(super) fn empty_ustr(&self) -> BasicValueEnum<'ctx> {
        self.ustr_type().const_zero().as_basic_value_enum()
    }

    /// A binary operator with a UnicodeString or WideChar operand:
    /// `Ok(result)` when it is a string operation (concatenation or
    /// comparison), else `Err` with the operands to compile as usual
    /// (WideChars widened to 64 bits, so they compare as codes).
    #[allow(clippy::type_complexity)]
    pub(super) fn unicode_binop(
        &self,
        op: BinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<
        Result<BasicValueEnum<'ctx>, (BasicValueEnum<'ctx>, BasicValueEnum<'ctx>)>,
        CodeGenError,
    > {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let stringy = |v: BasicValueEnum<'ctx>| {
            self.is_ustr(v)
                || v.is_pointer_value()
                || matches!(v, BasicValueEnum::IntValue(i) if matches!(i.get_type().get_bit_width(), 8 | 16))
        };
        let (l_ustr, r_ustr) = (self.is_ustr(lhs), self.is_ustr(rhs));
        let (l_wide, r_wide) = (Self::is_wide_char(lhs), Self::is_wide_char(rhs));
        let comparison = |op: BinOp| match op {
            BinOp::Eq => Some(inkwell::IntPredicate::EQ),
            BinOp::Neq => Some(inkwell::IntPredicate::NE),
            BinOp::Lt => Some(inkwell::IntPredicate::SLT),
            BinOp::Lte => Some(inkwell::IntPredicate::SLE),
            BinOp::Gt => Some(inkwell::IntPredicate::SGT),
            BinOp::Gte => Some(inkwell::IntPredicate::SGE),
            _ => None,
        };
        // String operations: one side a UnicodeString, or a WideChar
        // added to a string or char.
        let string_op = (l_ustr || r_ustr) && stringy(lhs) && stringy(rhs)
            || op == BinOp::Add
                && (l_wide || r_wide)
                && stringy(lhs)
                && stringy(rhs);
        if string_op {
            if op == BinOp::Add {
                return Ok(Ok(self.ustr_concat(lhs, rhs, span)?));
            }
            if let Some(pred) = comparison(op) {
                let order = self.ustr_compare(lhs, rhs, span)?;
                let result = self
                    .builder
                    .build_int_compare(pred, order, self.context.i64_type().const_zero(), "ustr_cmp")
                    .map_err(err)?;
                return Ok(Ok(result.into()));
            }
        }
        if !(l_wide || r_wide) || op == BinOp::In && !l_wide {
            return Ok(Err((lhs, rhs)));
        }
        // A WideChar against a char, a one-character string or another
        // WideChar: compare the codes.
        let as_code = |v: BasicValueEnum<'ctx>| -> Result<BasicValueEnum<'ctx>, CodeGenError> {
            match v {
                BasicValueEnum::PointerValue(_) => {
                    let w = self.to_wide_char(v, span)?.into_int_value();
                    Ok(self.char_code(w, span)?.into())
                }
                BasicValueEnum::IntValue(i) if i.get_type().get_bit_width() <= 16 && i.get_type().get_bit_width() > 1 => {
                    Ok(self.char_code(i, span)?.into())
                }
                _ => Ok(v),
            }
        };
        if op == BinOp::In {
            return Ok(Err((as_code(lhs)?, rhs)));
        }
        Ok(Err((as_code(lhs)?, as_code(rhs)?)))
    }

    /// `s[i] := ch` for the UnicodeString in `slot`: the string is copied
    /// with the unit replaced, and the copy stored back (strings may be
    /// shared).
    pub(super) fn set_ustr_char(
        &mut self,
        slot: PointerValue<'ctx>,
        index: BasicValueEnum<'ctx>,
        ch: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let old = self.builder.build_load(ptr_ty, slot, "ustr_old").map_err(err)?;
        let idx = self.to_i64(index, span)?;
        let ch = self.to_wide_char(ch, span)?.into_int_value();
        let code = self.char_code(ch, span)?;
        let new = self
            .call_ustr_runtime(
                "bruto_ustr_setchar",
                &[old.into(), idx.into(), code.into()],
                span,
            )?
            .unwrap();
        self.builder.build_store(slot, new).map_err(err)?;
        Ok(())
    }

    /// Whether `expr` is a UnicodeString or a WideChar.
    pub(super) fn is_wide_expr(&self, expr: &Expr) -> bool {
        matches!(
            self.resolve_type(&self.infer_declared_type(expr)),
            PascalType::UnicodeString | PascalType::WideChar
        )
    }

    /// The type of built-in function `name` applied to UnicodeStrings or
    /// WideChars, when it differs from the AnsiString one.
    pub(super) fn unicode_builtin_type(&self, name: &str, args: &[Expr]) -> Option<PascalType> {
        let first = args.first()?;
        let first_ty = self.resolve_type(&self.infer_declared_type(first));
        match name {
            "copy" if first_ty == PascalType::UnicodeString => Some(PascalType::UnicodeString),
            "concat" if args.iter().any(|a| self.is_wide_expr(a)) => {
                Some(PascalType::UnicodeString)
            }
            "upcase" | "lowercase" | "uppercase"
                if matches!(first_ty, PascalType::UnicodeString | PascalType::WideChar) =>
            {
                Some(first_ty)
            }
            _ => None,
        }
    }

    /// Built-in function `name` on UnicodeStrings or WideChars, or `None`
    /// when the arguments aren't any (the AnsiString version applies).
    pub(super) fn unicode_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let Some(first) = args.first() else {
            return Ok(None);
        };
        let first_ty = self.resolve_type(&self.infer_declared_type(first));
        let is_ustr = first_ty == PascalType::UnicodeString;
        let i64_ty = self.context.i64_type();
        Ok(Some(match (name, args.len()) {
            ("length", 1) if is_ustr => {
                let s = self.compile_expr(first)?;
                let s = self.ustr_ptr(s, span)?;
                self.call_ustr_runtime("bruto_ustr_length", &[s.into()], span)?
                    .unwrap()
            }
            ("high", 1) if is_ustr => {
                let s = self.compile_expr(first)?;
                let s = self.ustr_ptr(s, span)?;
                self.call_ustr_runtime("bruto_ustr_length", &[s.into()], span)?
                    .unwrap()
            }
            ("copy", 2 | 3) if is_ustr => {
                let s = self.compile_expr(first)?;
                let s = self.ustr_ptr(s, span)?;
                let idx = self.compile_expr(&args[1])?;
                let idx = self.to_i64(idx, span)?;
                let count = match args.get(2) {
                    Some(c) => {
                        let c = self.compile_expr(c)?;
                        self.to_i64(c, span)?
                    }
                    None => i64_ty.const_int(i64::MAX as u64, false),
                };
                self.ustr_runtime("bruto_ustr_copy", &[s.into(), idx.into(), count.into()], span)?
            }
            ("pos", 2 | 3) if args[..2].iter().any(|a| self.is_wide_expr(a)) => {
                let sub = self.compile_expr(first)?;
                let sub = self.to_ustr(sub, span)?;
                let sub = self.ustr_ptr(sub, span)?;
                let s = self.compile_expr(&args[1])?;
                let s = self.to_ustr(s, span)?;
                let s = self.ustr_ptr(s, span)?;
                let offset = match args.get(2) {
                    Some(o) => {
                        let o = self.compile_expr(o)?;
                        self.to_i64(o, span)?
                    }
                    None => i64_ty.const_int(1, false),
                };
                self.call_ustr_runtime(
                    "bruto_ustr_pos",
                    &[sub.into(), s.into(), offset.into()],
                    span,
                )?
                .unwrap()
            }
            ("concat", n) if n >= 2 && args.iter().any(|a| self.is_wide_expr(a)) => {
                let mut result = self.compile_expr(first)?;
                result = self.to_ustr(result, span)?;
                for arg in &args[1..] {
                    let next = self.compile_expr(arg)?;
                    result = self.ustr_concat(result, next, span)?;
                }
                result
            }
            ("upcase" | "uppercase" | "lowercase", 1)
                if first_ty == PascalType::WideChar || is_ustr =>
            {
                let lower = name == "lowercase";
                let val = self.compile_expr(first)?;
                if is_ustr {
                    let s = self.ustr_ptr(val, span)?;
                    let f = if lower {
                        "bruto_ustr_lowercase"
                    } else {
                        "bruto_ustr_upcase"
                    };
                    self.ustr_runtime(f, &[s.into()], span)?
                } else {
                    let code = self.char_code(val.into_int_value(), span)?;
                    let f = if lower {
                        "bruto_wchar_lowercase"
                    } else {
                        "bruto_wchar_upcase"
                    };
                    let r = self.call_ustr_runtime(f, &[code.into()], span)?.unwrap();
                    self.to_wide_char(r, span)?
                }
            }
            _ => return Ok(None),
        }))
    }

    /// A string argument for a routine that takes a C string (a file
    /// name, a path): UnicodeStrings and chars are converted.
    pub(super) fn ansi_arg(
        &mut self,
        arg: &Expr,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let val = self.compile_expr(arg)?;
        let val = self.ustr_to_ansi(val, span)?;
        self.as_string_value(val, span)
    }
}
