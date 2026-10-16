//! Variants: FPC's `TVarData` (see `variant_type`), worked on by the
//! Variants unit (`rtl/variants.pp`).
//!
//! A value converted to a Variant, an operator on Variants and a Variant
//! array's element are computed by the unit's `bruto_var_*` routines into
//! a zeroed temporary, which then goes to the statement's pool (the
//! string it may hold is freed when the statement ends unless it was
//! stored), just as a string a function returns does. Storing and
//! releasing Variants counts their strings and arrays (see `arc.rs`).

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, PointerValue};

/// FPC's `TVarOp` codes, as the unit numbers them.
const OP_ADD: u64 = 0;
const OP_SUBTRACT: u64 = 1;
const OP_MULTIPLY: u64 = 2;
const OP_DIVIDE: u64 = 3;
const OP_INT_DIVIDE: u64 = 4;
const OP_MODULUS: u64 = 5;
const OP_SHIFT_LEFT: u64 = 6;
const OP_SHIFT_RIGHT: u64 = 7;
const OP_AND: u64 = 8;
const OP_OR: u64 = 9;
const OP_XOR: u64 = 10;
const OP_NEGATE: u64 = 12;
const OP_NOT: u64 = 13;
const OP_CMP_EQ: u64 = 14;
const OP_CMP_NE: u64 = 15;
const OP_CMP_LT: u64 = 16;
const OP_CMP_LE: u64 = 17;
const OP_CMP_GT: u64 = 18;
const OP_CMP_GE: u64 = 19;

/// The variant type codes (`varSmallInt`...).
const VAR_SMALLINT: u64 = 2;
const VAR_INTEGER: u64 = 3;
const VAR_SHORTINT: u64 = 16;
const VAR_BYTE: u64 = 17;
const VAR_WORD: u64 = 18;
const VAR_LONGWORD: u64 = 19;
const VAR_INT64: u64 = 20;
const VAR_QWORD: u64 = 21;

/// The variant type FPC gives an integer constant: the smallest that
/// holds it.
fn literal_var_type(n: i64) -> u64 {
    if n >= 0 {
        match n {
            0..=127 => VAR_SHORTINT,
            128..=255 => VAR_BYTE,
            256..=32767 => VAR_SMALLINT,
            32768..=65535 => VAR_WORD,
            65536..=2147483647 => VAR_INTEGER,
            2147483648..=4294967295 => VAR_LONGWORD,
            _ => VAR_INT64,
        }
    } else {
        match n {
            -128..=-1 => VAR_SHORTINT,
            -32768..=-129 => VAR_SMALLINT,
            -2147483648..=-32769 => VAR_INTEGER,
            _ => VAR_INT64,
        }
    }
}

impl<'ctx> CodeGen<'ctx> {
    /// Whether `ty` is a Variant.
    pub(super) fn is_variant(&self, ty: &PascalType) -> bool {
        self.resolve_type(ty) == PascalType::Variant
    }

    /// Whether `expr` is a Variant.
    pub(super) fn expr_is_variant(&self, expr: &Expr) -> bool {
        self.is_variant(&self.infer_declared_type(expr))
    }

    /// Call the Variants unit's routine `name`.
    fn call_var_runtime(
        &self,
        name: &str,
        args: &[BasicMetadataValueEnum<'ctx>],
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let f = self.module.get_function(name).ok_or_else(|| {
            CodeGenError::new(
                "Variant operations need the Variants unit (add Variants to the uses clause)",
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

    /// A zeroed Variant temporary.
    fn variant_slot(&self, span: Span) -> Result<PointerValue<'ctx>, CodeGenError> {
        let ty = self.variant_type();
        let slot = self
            .entry_alloca(ty, "var_slot")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder
            .build_store(slot, ty.const_zero())
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(slot)
    }

    /// The Variant a runtime routine left in `slot`, its reference handed
    /// to the statement's pool.
    fn variant_result(
        &self,
        slot: PointerValue<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        self.call_ustr_runtime("bruto_var_return", &[slot.into()], span)?;
        self.builder
            .build_load(self.variant_type(), slot, "variant")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))
    }

    /// The Variant value `val` in memory, for the runtime to read.
    fn variant_in_memory(
        &self,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        let slot = self
            .entry_alloca(self.variant_type(), "var_arg")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder
            .build_store(slot, val)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(slot)
    }

    /// The variant type an integer expression of type `ty` converts to,
    /// as FPC picks it: a constant by its value, a variable by its
    /// declared type, other integer expressions as Int64.
    fn integer_var_type(expr: &Expr, ty: &PascalType) -> u64 {
        match expr {
            Expr::IntLit(n, _) => return literal_var_type(*n),
            Expr::UnaryOp {
                op: UnaryOp::Neg,
                operand,
                ..
            } => {
                if let Expr::IntLit(n, _) = operand.as_ref() {
                    return literal_var_type(n.wrapping_neg());
                }
                return VAR_INT64;
            }
            Expr::BinOp { .. } | Expr::UnaryOp { .. } => return VAR_INT64,
            _ => {}
        }
        match ty {
            PascalType::SizedInt { bits: 8, signed } => {
                if *signed {
                    VAR_SHORTINT
                } else {
                    VAR_BYTE
                }
            }
            PascalType::SizedInt { bits: 16, signed } => {
                if *signed {
                    VAR_SMALLINT
                } else {
                    VAR_WORD
                }
            }
            PascalType::SizedInt { bits: 32, signed } => {
                if *signed {
                    VAR_INTEGER
                } else {
                    VAR_LONGWORD
                }
            }
            PascalType::SizedInt { signed: false, .. } => VAR_QWORD,
            // Bruto's Integer is 64 bits wide (FPC's Integer is a LongInt
            // and arrives here as one).
            _ => VAR_INT64,
        }
    }

    /// Compile `expr` as a Variant: converted when it is another type.
    pub(super) fn compile_as_variant(
        &mut self,
        expr: &Expr,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let span = expr.span();
        let ty = self.resolve_type(&self.infer_declared_type(expr));
        if ty == PascalType::Variant {
            return self.compile_expr(expr);
        }
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i64_ty = self.context.i64_type();
        let slot = self.variant_slot(span)?;
        match &ty {
            PascalType::Boolean => {
                let v = self.compile_expr(expr)?.into_int_value();
                let v = self
                    .builder
                    .build_int_z_extend(v, i64_ty, "b")
                    .map_err(err)?;
                self.call_var_runtime("bruto_var_from_bool", &[slot.into(), v.into()], span)?;
            }
            PascalType::Currency => {
                let v = self.compile_currency_raw(expr)?;
                self.call_var_runtime("bruto_var_from_curr", &[slot.into(), v.into()], span)?;
            }
            PascalType::Real | PascalType::Single => {
                let v = self.compile_expr(expr)?;
                let v = match v {
                    BasicValueEnum::FloatValue(f) if f.get_type() != self.context.f64_type() => {
                        self.builder
                            .build_float_ext(f, self.context.f64_type(), "fext")
                            .map_err(err)?
                    }
                    BasicValueEnum::FloatValue(f) => f,
                    BasicValueEnum::IntValue(i) => self
                        .builder
                        .build_signed_int_to_float(i, self.context.f64_type(), "itof")
                        .map_err(err)?,
                    _ => return Err(CodeGenError::new("not a number", Some(span))),
                };
                self.call_var_runtime("bruto_var_from_double", &[slot.into(), v.into()], span)?;
            }
            PascalType::UnicodeString | PascalType::WideChar => {
                let v = self.compile_expr(expr)?;
                let v = self.to_ustr(v, span)?;
                let p = self.ustr_ptr(v, span)?;
                self.call_var_runtime("bruto_var_from_ustr", &[slot.into(), p.into()], span)?;
            }
            PascalType::String | PascalType::Char | PascalType::ShortString { .. } => {
                let v = self.compile_expr_for(expr, &PascalType::String)?;
                let v = self.convert_for_store(v, &PascalType::String, span)?;
                self.call_var_runtime("bruto_var_from_str", &[slot.into(), v.into()], span)?;
            }
            PascalType::Pointer(inner) if **inner == PascalType::Char => {
                let v = self.compile_expr_for(expr, &PascalType::String)?;
                self.call_var_runtime("bruto_var_from_str", &[slot.into(), v.into()], span)?;
            }
            PascalType::Integer | PascalType::SizedInt { .. } | PascalType::Enum { .. } => {
                let var_type = Self::integer_var_type(expr, &ty);
                let v = self.compile_expr(expr)?.into_int_value();
                let v = if v.get_type().get_bit_width() < 64 {
                    self.builder
                        .build_int_s_extend(v, i64_ty, "i")
                        .map_err(err)?
                } else {
                    v
                };
                self.call_var_runtime(
                    "bruto_var_from_int",
                    &[
                        slot.into(),
                        v.into(),
                        i64_ty.const_int(var_type, false).into(),
                    ],
                    span,
                )?;
            }
            other => {
                return Err(CodeGenError::new(
                    format!("cannot convert {other:?} to a Variant"),
                    Some(span),
                ));
            }
        }
        self.variant_result(slot, span)
    }

    /// Whether a Variant converts to a value of type `ty`.
    pub(super) fn variant_converts_to(&self, ty: &PascalType) -> bool {
        matches!(
            self.resolve_type(ty),
            PascalType::Boolean
                | PascalType::Real
                | PascalType::Single
                | PascalType::Currency
                | PascalType::String
                | PascalType::Char
                | PascalType::ShortString { .. }
                | PascalType::UnicodeString
                | PascalType::WideChar
                | PascalType::Integer
                | PascalType::SizedInt { .. }
                | PascalType::Enum { .. }
        )
    }

    /// The Variant `val` converted to type `target` (one
    /// [`Self::variant_converts_to`] accepts), in expression form.
    pub(super) fn variant_to(
        &mut self,
        val: BasicValueEnum<'ctx>,
        target: &PascalType,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let p = self.variant_in_memory(val, span)?;
        let name = match self.resolve_type(target) {
            PascalType::Boolean => {
                let v = self
                    .call_var_runtime("bruto_var_to_bool", &[p.into()], span)?
                    .unwrap()
                    .into_int_value();
                return Ok(self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        v,
                        v.get_type().const_zero(),
                        "vbool",
                    )
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into());
            }
            PascalType::Real | PascalType::Single => "bruto_var_to_double",
            PascalType::Currency => "bruto_var_to_curr",
            PascalType::String | PascalType::Char | PascalType::ShortString { .. } => {
                "bruto_var_to_str"
            }
            PascalType::UnicodeString | PascalType::WideChar => "bruto_var_to_ustr",
            _ => "bruto_var_to_int",
        };
        Ok(self.call_var_runtime(name, &[p.into()], span)?.unwrap())
    }

    /// `left op right` where a side is a Variant: through the runtime.
    /// `None` when `op` isn't an operator on Variants.
    pub(super) fn compile_variant_binop(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let (code, compare) = match op {
            BinOp::Add => (OP_ADD, false),
            BinOp::Sub => (OP_SUBTRACT, false),
            BinOp::Mul => (OP_MULTIPLY, false),
            BinOp::RealDiv => (OP_DIVIDE, false),
            BinOp::Div => (OP_INT_DIVIDE, false),
            BinOp::Mod => (OP_MODULUS, false),
            BinOp::Shl => (OP_SHIFT_LEFT, false),
            BinOp::Shr => (OP_SHIFT_RIGHT, false),
            BinOp::And => (OP_AND, false),
            BinOp::Or => (OP_OR, false),
            BinOp::Xor => (OP_XOR, false),
            BinOp::Eq => (OP_CMP_EQ, true),
            BinOp::Neq => (OP_CMP_NE, true),
            BinOp::Lt => (OP_CMP_LT, true),
            BinOp::Lte => (OP_CMP_LE, true),
            BinOp::Gt => (OP_CMP_GT, true),
            BinOp::Gte => (OP_CMP_GE, true),
            _ => return Ok(None),
        };
        let l = self.compile_as_variant(left)?;
        let l = self.variant_in_memory(l, span)?;
        let r = self.compile_as_variant(right)?;
        let r = self.variant_in_memory(r, span)?;
        let i64_ty = self.context.i64_type();
        let code = i64_ty.const_int(code, false);
        if compare {
            let v = self
                .call_var_runtime("bruto_var_cmp", &[l.into(), r.into(), code.into()], span)?
                .unwrap()
                .into_int_value();
            return Ok(Some(
                self.builder
                    .build_int_compare(inkwell::IntPredicate::NE, v, i64_ty.const_zero(), "vcmp")
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                    .into(),
            ));
        }
        let slot = self.variant_slot(span)?;
        self.call_var_runtime(
            "bruto_var_op",
            &[slot.into(), l.into(), r.into(), code.into()],
            span,
        )?;
        Ok(Some(self.variant_result(slot, span)?))
    }

    /// `-V`, `not V`.
    pub(super) fn compile_variant_unary(
        &mut self,
        op: UnaryOp,
        operand: &Expr,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let v = self.compile_expr(operand)?;
        let p = self.variant_in_memory(v, span)?;
        let slot = self.variant_slot(span)?;
        let code = match op {
            UnaryOp::Neg => OP_NEGATE,
            UnaryOp::Not => OP_NOT,
        };
        let code = self.context.i64_type().const_int(code, false);
        self.call_var_runtime(
            "bruto_var_unary",
            &[slot.into(), p.into(), code.into()],
            span,
        )?;
        self.variant_result(slot, span)
    }

    /// `T(X)` converting to or from a Variant: T.
    pub(super) fn variant_cast_type(&self, name: &str, arg: &Expr) -> Option<PascalType> {
        let target = match name {
            "integer" => PascalType::Integer,
            "real" => PascalType::Real,
            "currency" => PascalType::Currency,
            "char" => PascalType::Char,
            "boolean" => PascalType::Boolean,
            "string" | "ansistring" => PascalType::String,
            _ => self.cast_target(name)?,
        };
        if self.is_variant(&target)
            || (self.variant_converts_to(&target) && self.expr_is_variant(arg))
        {
            Some(target)
        } else {
            None
        }
    }

    /// The index of a variant array's element, as an `i64`.
    fn variant_index(&mut self, index: &Expr) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let span = index.span();
        if self.expr_is_variant(index) {
            let v = self.compile_expr(index)?;
            return self.variant_to(v, &PascalType::Integer, span);
        }
        let v = self.compile_expr(index)?;
        Ok(self.to_i64(v, span)?.into())
    }

    /// `V[I, J]` (or `V[I][J]`): the Variant V indexed, and the indices
    /// in order. `None` unless `expr` indexes a Variant.
    pub(super) fn variant_index_chain<'e>(
        &self,
        expr: &'e Expr,
    ) -> Option<(&'e Expr, Vec<&'e Expr>)> {
        let Expr::Index { array, index, .. } = expr else {
            return None;
        };
        if !self.expr_is_variant(array) {
            return None;
        }
        match self.variant_index_chain(array) {
            Some((base, mut indices)) => {
                indices.push(index);
                Some((base, indices))
            }
            None => Some((array, vec![index])),
        }
    }

    /// `indices` as an array of `i64`s in memory, and their number.
    fn variant_indices(
        &mut self,
        indices: &[&Expr],
        span: Span,
    ) -> Result<(PointerValue<'ctx>, BasicValueEnum<'ctx>), CodeGenError> {
        let i64_ty = self.context.i64_type();
        let array_ty = i64_ty.array_type(indices.len() as u32);
        let buffer = self
            .entry_alloca(array_ty, "var_indices")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        for (n, index) in indices.iter().enumerate() {
            let i = self.variant_index(index)?;
            let at = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        array_ty,
                        buffer,
                        &[i64_ty.const_zero(), i64_ty.const_int(n as u64, false)],
                        "var_index",
                    )
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            };
            self.builder
                .build_store(at, i)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        }
        Ok((buffer, i64_ty.const_int(indices.len() as u64, false).into()))
    }

    /// `V[I, J]`: the element of variant array V (`expr`, an index of a
    /// Variant).
    pub(super) fn compile_variant_index(
        &mut self,
        expr: &Expr,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let span = expr.span();
        let (base, indices) = self.variant_index_chain(expr).unwrap();
        let a = self.compile_expr(base)?;
        let a = self.variant_in_memory(a, span)?;
        let (buffer, count) = self.variant_indices(&indices, span)?;
        let slot = self.variant_slot(span)?;
        self.call_var_runtime(
            "bruto_var_array_get",
            &[slot.into(), a.into(), buffer.into(), count.into()],
            span,
        )?;
        self.variant_result(slot, span)
    }

    /// `V[I, J] := X`, V's variant at `slot`.
    pub(super) fn store_variant_element(
        &mut self,
        slot: PointerValue<'ctx>,
        indices: &[&Expr],
        expr: &Expr,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let (buffer, count) = self.variant_indices(indices, span)?;
        let v = self.compile_as_variant(expr)?;
        let v = self.variant_in_memory(v, span)?;
        self.call_var_runtime(
            "bruto_var_array_put",
            &[slot.into(), buffer.into(), count.into(), v.into()],
            span,
        )?;
        Ok(())
    }
}
