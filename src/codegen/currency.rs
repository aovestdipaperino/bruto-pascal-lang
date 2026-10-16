//! Currency: FPC's fixed point number, an Int64 holding the value times
//! 10000 (FPC's own representation on 64-bit ARM, where it has no x87).
//!
//! Loaded into an expression, a Currency is a `Real` (`widen_loaded`),
//! so comparisons, `/`, `Round` and the math routines see its value.
//! Arithmetic whose result is a Currency (`+`, `-` or `*` with a Currency
//! operand) works on the scaled Int64s instead, as FPC's does: a real or
//! integer operand is first made a Currency (rounding halves to even),
//! and `*` keeps the 128-bit product's quotient by 10000, truncated. A
//! value stored into a Currency takes the same conversion.

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::values::{BasicValueEnum, IntValue};

/// Currency's scale.
const SCALE: u64 = 10000;

impl<'ctx> CodeGen<'ctx> {
    /// Whether `ty` is Currency.
    pub(super) fn is_currency(&self, ty: &PascalType) -> bool {
        self.resolve_type(ty) == PascalType::Currency
    }

    /// Whether `expr` is a Currency.
    pub(super) fn expr_is_currency(&self, expr: &Expr) -> bool {
        self.is_currency(&self.infer_declared_type(expr))
    }

    /// Whether `ty` takes part in Currency arithmetic: a number.
    pub(super) fn currency_operand(&self, ty: &PascalType) -> bool {
        matches!(
            self.resolve_type(ty),
            PascalType::Currency
                | PascalType::Real
                | PascalType::Single
                | PascalType::Integer
                | PascalType::SizedInt { .. }
        )
    }

    /// The type of `left op right` when it is a Currency: `+`, `-` or `*`
    /// of a Currency and a number.
    pub(super) fn currency_binop_type(&self, op: BinOp, ld: &PascalType, rd: &PascalType) -> bool {
        matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul)
            && (self.is_currency(ld) || self.is_currency(rd))
            && self.currency_operand(ld)
            && self.currency_operand(rd)
    }

    /// A Currency's scaled Int64 as the Real it stands for.
    pub(super) fn currency_to_real(
        &self,
        raw: IntValue<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let f64_ty = self.context.f64_type();
        let f = self
            .builder
            .build_signed_int_to_float(raw, f64_ty, "curr")
            .map_err(err)?;
        Ok(self
            .builder
            .build_float_div(f, f64_ty.const_float(SCALE as f64), "curr_value")
            .map_err(err)?
            .into())
    }

    /// A Real (or integer) value made a Currency's scaled Int64, rounding
    /// halves to even.
    pub(super) fn currency_from_value(
        &self,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i64_ty = self.context.i64_type();
        match val {
            BasicValueEnum::IntValue(i) => {
                let i = if i.get_type().get_bit_width() < 64 {
                    // Booleans and characters are unsigned.
                    if i.get_type().get_bit_width() <= 16 {
                        self.builder
                            .build_int_z_extend(i, i64_ty, "ext")
                            .map_err(err)?
                    } else {
                        self.builder
                            .build_int_s_extend(i, i64_ty, "ext")
                            .map_err(err)?
                    }
                } else {
                    i
                };
                self.builder
                    .build_int_mul(i, i64_ty.const_int(SCALE, false), "curr_int")
                    .map_err(err)
            }
            BasicValueEnum::FloatValue(f) => {
                let f64_ty = self.context.f64_type();
                let f = if f.get_type() != f64_ty {
                    self.builder
                        .build_float_ext(f, f64_ty, "fext")
                        .map_err(err)?
                } else {
                    f
                };
                let scaled = self
                    .builder
                    .build_float_mul(f, f64_ty.const_float(SCALE as f64), "scaled")
                    .map_err(err)?;
                let round = self.get_or_declare_f64_intrinsic("llvm.roundeven.f64");
                let rounded = self
                    .builder
                    .build_call(round, &[scaled.into()], "rounded")
                    .map_err(err)?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_float_value();
                self.builder
                    .build_float_to_signed_int(rounded, i64_ty, "curr_raw")
                    .map_err(err)
            }
            _ => Err(CodeGenError::new(
                "cannot convert this value to a Currency",
                Some(span),
            )),
        }
    }

    /// `expr` as a Currency's scaled Int64.
    pub(super) fn compile_currency_raw(
        &mut self,
        expr: &Expr,
    ) -> Result<IntValue<'ctx>, CodeGenError> {
        let span = expr.span();
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i64_ty = self.context.i64_type();
        let ty = self.resolve_type(&self.infer_declared_type(expr));
        match expr {
            Expr::IntLit(n, _) => {
                return Ok(i64_ty.const_int(n.wrapping_mul(SCALE as i64) as u64, true));
            }
            Expr::BinOp {
                op, left, right, ..
            } if ty == PascalType::Currency => {
                let l = self.compile_currency_raw(left)?;
                let r = self.compile_currency_raw(right)?;
                return match op {
                    BinOp::Add => self.builder.build_int_add(l, r, "curr_add").map_err(err),
                    BinOp::Sub => self.builder.build_int_sub(l, r, "curr_sub").map_err(err),
                    _ => {
                        // (l * r) div 10000 on 128 bits, truncated.
                        let i128_ty = self.context.i128_type();
                        let l = self
                            .builder
                            .build_int_s_extend(l, i128_ty, "l")
                            .map_err(err)?;
                        let r = self
                            .builder
                            .build_int_s_extend(r, i128_ty, "r")
                            .map_err(err)?;
                        let p = self.builder.build_int_mul(l, r, "curr_mul").map_err(err)?;
                        let q = self
                            .builder
                            .build_int_signed_div(p, i128_ty.const_int(SCALE, false), "curr_q")
                            .map_err(err)?;
                        self.builder
                            .build_int_truncate(q, i64_ty, "curr")
                            .map_err(err)
                    }
                };
            }
            Expr::UnaryOp {
                op: UnaryOp::Neg,
                operand,
                ..
            } if ty == PascalType::Currency => {
                let v = self.compile_currency_raw(operand)?;
                return self.builder.build_int_neg(v, "curr_neg").map_err(err);
            }
            // `Currency(X)`: X's value as a Currency.
            Expr::Call { name, args, .. }
                if args.len() == 1
                    && self
                        .cast_target(name)
                        .is_some_and(|t| t == PascalType::Currency)
                    && !self.is_addressable(&args[0]) =>
            {
                return self.compile_currency_raw(&args[0]);
            }
            _ => {}
        }
        if ty == PascalType::Currency && self.is_addressable(expr) {
            let (slot, _) = self.resolve_array_base(expr, span)?;
            return Ok(self
                .builder
                .build_load(i64_ty, slot, "curr")
                .map_err(err)?
                .into_int_value());
        }
        if ty == PascalType::Variant {
            let v = self.compile_expr(expr)?;
            return Ok(self
                .variant_to(v, &PascalType::Currency, span)?
                .into_int_value());
        }
        let v = self.compile_expr(expr)?;
        self.currency_from_value(v, span)
    }

    /// `left op right` with a Currency result, as the Real it stands for
    /// (an expression's form).
    pub(super) fn compile_currency_binop(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let expr = Expr::BinOp {
            op,
            left: Box::new(left.clone()),
            right: Box::new(right.clone()),
            span,
        };
        let raw = self.compile_currency_raw(&expr)?;
        self.currency_to_real(raw, span)
    }

    /// The text `Write(C:Width:Decimals)` writes for the Currency
    /// `expr`, as FPC formats it (`bruto_curr_str`).
    pub(super) fn currency_text(
        &mut self,
        arg: &WriteArg,
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        if self.in_runtime || !self.expr_is_currency(&arg.expr) {
            return Ok(None);
        }
        let Some(format) = self.module.get_function("bruto_curr_str") else {
            return Ok(None);
        };
        let i64_ty = self.context.i64_type();
        let raw = self.compile_currency_raw(&arg.expr)?;
        let int_arg = |cg: &mut Self, e: &Option<Expr>, default: i64| match e {
            Some(e) => {
                let v = cg.compile_expr(e)?;
                cg.to_i64(v, span)
            }
            None => Ok(i64_ty.const_int(default as u64, true)),
        };
        let width = int_arg(self, &arg.width, -32767)?;
        let decimals = int_arg(self, &arg.precision, -1)?;
        Ok(Some(
            self.builder
                .build_call(
                    format,
                    &[raw.into(), width.into(), decimals.into()],
                    "curr_text",
                )
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
                .try_as_basic_value()
                .basic()
                .unwrap(),
        ))
    }
}
