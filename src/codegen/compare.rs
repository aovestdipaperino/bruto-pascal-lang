//! The default comparison of two values of any type, as FPC's
//! Generics.Defaults makes it: `BrutoCompare(A, B)` (negative, zero or
//! positive) and `BrutoEquals(A, B)`, built-ins for the generic
//! comparers (`TComparer<T>.Default`, `TEqualityComparer<T>.Default`).
//!
//! Numbers, characters, Booleans, enumerations, strings and Variants
//! compare by value; pointers, classes, interfaces and procedural values
//! by address; anything else (records, static arrays, sets) by its bytes,
//! as FPC's binary comparer does. `BrutoHash(A)` hashes the same way
//! (equal values, equal hashes), with Generics.Defaults' helpers.

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

impl<'ctx> CodeGen<'ctx> {
    /// `BrutoCompare(A, B)` (`equals` false: -1, 0 or 1) or
    /// `BrutoEquals(A, B)` (`equals` true: a Boolean).
    pub(super) fn compile_default_compare(
        &mut self,
        a: &Expr,
        b: &Expr,
        equals: bool,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ty = self.resolve_type(&self.infer_declared_type(a));
        let i64_ty = self.context.i64_type();
        let by_value = matches!(
            ty,
            PascalType::Integer
                | PascalType::SizedInt { .. }
                | PascalType::Real
                | PascalType::Single
                | PascalType::Currency
                | PascalType::Variant
                | PascalType::String
                | PascalType::UnicodeString
                | PascalType::ShortString { .. }
                | PascalType::Boolean
                | PascalType::Char
                | PascalType::WideChar
                | PascalType::Enum { .. }
                | PascalType::Subrange { .. }
        );
        let bin = |op| Expr::BinOp {
            op,
            left: Box::new(a.clone()),
            right: Box::new(b.clone()),
            span,
        };
        if by_value {
            if equals {
                return self.compile_expr(&bin(BinOp::Eq));
            }
            let gt = self.compile_expr(&bin(BinOp::Gt))?.into_int_value();
            let lt = self.compile_expr(&bin(BinOp::Lt))?.into_int_value();
            let gt = self
                .builder
                .build_int_z_extend(gt, i64_ty, "gt")
                .map_err(err)?;
            let lt = self
                .builder
                .build_int_z_extend(lt, i64_ty, "lt")
                .map_err(err)?;
            return Ok(self
                .builder
                .build_int_sub(gt, lt, "cmp")
                .map_err(err)?
                .into());
        }
        let llvm_ty = self.llvm_type_for(&ty);
        let (l, r) = if llvm_ty.is_pointer_type() {
            // By address.
            let l = self.compile_expr(a)?.into_pointer_value();
            let r = self.compile_expr(b)?.into_pointer_value();
            let l = self
                .builder
                .build_ptr_to_int(l, i64_ty, "pl")
                .map_err(err)?;
            let r = self
                .builder
                .build_ptr_to_int(r, i64_ty, "pr")
                .map_err(err)?;
            (l, r)
        } else {
            // By bytes.
            let pa = self.value_address(a, span)?;
            let pb = self.value_address(b, span)?;
            let size = llvm_ty
                .size_of()
                .ok_or_else(|| CodeGenError::new("type has no size", Some(span)))?;
            let memcmp = self.module.get_function("memcmp").unwrap_or_else(|| {
                let ptr = self.context.ptr_type(AddressSpace::default());
                let fn_ty = self
                    .context
                    .i32_type()
                    .fn_type(&[ptr.into(), ptr.into(), i64_ty.into()], false);
                self.module.add_function("memcmp", fn_ty, None)
            });
            let c = self
                .builder
                .build_call(memcmp, &[pa.into(), pb.into(), size.into()], "memcmp")
                .map_err(err)?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_int_value();
            let c = self
                .builder
                .build_int_s_extend(c, i64_ty, "c")
                .map_err(err)?;
            (c, i64_ty.const_zero())
        };
        if equals {
            return Ok(self
                .builder
                .build_int_compare(IntPredicate::EQ, l, r, "eq")
                .map_err(err)?
                .into());
        }
        let gt = self
            .builder
            .build_int_compare(IntPredicate::UGT, l, r, "gt")
            .map_err(err)?;
        let lt = self
            .builder
            .build_int_compare(IntPredicate::ULT, l, r, "lt")
            .map_err(err)?;
        if !llvm_ty.is_pointer_type() {
            // memcmp's result is signed.
            let gt = self
                .builder
                .build_int_compare(IntPredicate::SGT, l, r, "gt")
                .map_err(err)?;
            let lt = self
                .builder
                .build_int_compare(IntPredicate::SLT, l, r, "lt")
                .map_err(err)?;
            let gt = self
                .builder
                .build_int_z_extend(gt, i64_ty, "gt")
                .map_err(err)?;
            let lt = self
                .builder
                .build_int_z_extend(lt, i64_ty, "lt")
                .map_err(err)?;
            return Ok(self
                .builder
                .build_int_sub(gt, lt, "cmp")
                .map_err(err)?
                .into());
        }
        let gt = self
            .builder
            .build_int_z_extend(gt, i64_ty, "gt")
            .map_err(err)?;
        let lt = self
            .builder
            .build_int_z_extend(lt, i64_ty, "lt")
            .map_err(err)?;
        Ok(self
            .builder
            .build_int_sub(gt, lt, "cmp")
            .map_err(err)?
            .into())
    }

    /// `BrutoHash(A)`: a QWord hash of `a`, for the default equality
    /// comparer (the same hashes as Generics.Collections' `BrutoHashOf`).
    pub(super) fn compile_default_hash(
        &mut self,
        a: &Expr,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let helper = |this: &Self, name: &str| {
            this.module.get_function(name).ok_or_else(|| {
                CodeGenError::new("BrutoHash needs the Generics.Defaults unit", Some(span))
            })
        };
        let ty = self.resolve_type(&self.infer_declared_type(a));
        let i64_ty = self.context.i64_type();
        let call =
            |this: &Self, name: &str, args: &[inkwell::values::BasicMetadataValueEnum<'ctx>]| {
                let f = helper(this, name)?;
                Ok::<_, CodeGenError>(
                    this.builder
                        .build_call(f, args, "hash")
                        .map_err(err)?
                        .try_as_basic_value()
                        .basic()
                        .unwrap(),
                )
            };
        match &ty {
            PascalType::String | PascalType::UnicodeString => {
                let p = self.compile_expr(a)?;
                let len = Expr::Call {
                    name: "length".into(),
                    args: vec![a.clone()],
                    span,
                };
                let n = self.compile_expr(&len)?;
                let name = if ty == PascalType::String {
                    "bruto_hash_bytes"
                } else {
                    "bruto_hash_words"
                };
                call(self, name, &[p.into(), n.into()])
            }
            PascalType::ShortString { .. } => {
                let p = self.value_address(a, span)?;
                let n = self
                    .builder
                    .build_load(self.context.i8_type(), p, "slen")
                    .map_err(err)?
                    .into_int_value();
                let n = self
                    .builder
                    .build_int_z_extend(n, i64_ty, "slen")
                    .map_err(err)?;
                let data = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            p,
                            &[i64_ty.const_int(1, false)],
                            "sdata",
                        )
                        .map_err(err)?
                };
                call(self, "bruto_hash_bytes", &[data.into(), n.into()])
            }
            PascalType::Integer
            | PascalType::SizedInt { .. }
            | PascalType::Boolean
            | PascalType::Char
            | PascalType::WideChar
            | PascalType::Enum { .. }
            | PascalType::Subrange { .. } => {
                let v = self.compile_expr(a)?.into_int_value();
                let unsigned = matches!(
                    ty,
                    PascalType::SizedInt { signed: false, .. }
                        | PascalType::Boolean
                        | PascalType::Char
                        | PascalType::WideChar
                );
                let v = if v.get_type().get_bit_width() >= 64 {
                    v
                } else if unsigned {
                    self.builder
                        .build_int_z_extend(v, i64_ty, "h")
                        .map_err(err)?
                } else {
                    self.builder
                        .build_int_s_extend(v, i64_ty, "h")
                        .map_err(err)?
                };
                call(self, "bruto_hash_int", &[v.into()])
            }
            PascalType::Real | PascalType::Single | PascalType::Currency => {
                let v = self.compile_expr(a)?.into_float_value();
                let f64_ty = self.context.f64_type();
                let v = if v.get_type() == f64_ty {
                    v
                } else {
                    self.builder.build_float_ext(v, f64_ty, "h").map_err(err)?
                };
                let bits = self
                    .builder
                    .build_bit_cast(v, i64_ty, "bits")
                    .map_err(err)?;
                call(self, "bruto_hash_int", &[bits.into()])
            }
            _ if self.llvm_type_for(&ty).is_pointer_type() => {
                let p = self.compile_expr(a)?.into_pointer_value();
                let v = self.builder.build_ptr_to_int(p, i64_ty, "h").map_err(err)?;
                call(self, "bruto_hash_int", &[v.into()])
            }
            _ => {
                let size = self
                    .llvm_type_for(&ty)
                    .size_of()
                    .ok_or_else(|| CodeGenError::new("type has no size", Some(span)))?;
                let p = self.value_address(a, span)?;
                call(self, "bruto_hash_bytes", &[p.into(), size.into()])
            }
        }
    }

    /// The address of `expr`'s value: its variable's, or a temporary's.
    fn value_address(
        &mut self,
        expr: &Expr,
        span: Span,
    ) -> Result<inkwell::values::PointerValue<'ctx>, CodeGenError> {
        if self.is_addressable(expr) {
            return Ok(self.resolve_array_base(expr, span)?.0);
        }
        let v = self.compile_expr(expr)?;
        let temp = self
            .entry_alloca(v.get_type(), "cmp_temp")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder
            .build_store(temp, v)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(temp)
    }
}
