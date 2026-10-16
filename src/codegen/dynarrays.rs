//! Dynamic arrays (`array of T`), open array parameters and `array of
//! const`.
//!
//! A dynamic array is a pointer to its first element (nil when empty),
//! with the string runtime's header before it (`rtl/brutostrings.pp`): a
//! marker flagged as a dynamic array, a reference count and the number
//! of elements; two more words hold the routines that release and count
//! a run of elements, generated here for element types that hold
//! strings or arrays. Assigning one shares it (FPC's dynamic arrays are
//! not copy-on-write); `SetLength` gives the variable a block of its
//! own; the last reference frees the block and its elements, through the
//! same paths as strings.
//!
//! An open array parameter is `{ first element's address, highest index }`,
//! made from a static array, a dynamic array, another open array or
//! `[a, b, c]`; a `var` open array writes through to the caller's array.
//! `array of const` is an open array of `TVarRec`, each element tagged
//! with its argument's type.

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::types::{BasicType, StructType};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// `{ ptr data, i64 high }`.
    pub(super) fn open_array_type(&self) -> StructType<'ctx> {
        self.context.struct_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i64_type().into(),
            ],
            false,
        )
    }

    /// The element type of a dynamic or open array type, if it is one.
    pub(super) fn array_elem_of(&self, ty: &PascalType) -> Option<PascalType> {
        match self.resolve_type(ty) {
            PascalType::DynArray(elem) | PascalType::OpenArray(elem) => Some(*elem),
            _ => None,
        }
    }

    /// The size of an element of type `elem`, in bytes.
    fn elem_size(&self, elem: &PascalType) -> IntValue<'ctx> {
        let resolved = self.resolve_type(elem);
        self.llvm_type_for(&resolved)
            .size_of()
            .unwrap_or_else(|| self.context.i64_type().const_int(8, false))
    }

    /// The routines that release and count a run of `elem` elements, or
    /// nils when `elem` holds nothing counted.
    pub(super) fn dynarray_elem_procs(
        &mut self,
        elem: &PascalType,
    ) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>), CodeGenError> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        if !self.arc || !self.holds_counted(elem) {
            return Ok((ptr.const_null(), ptr.const_null()));
        }
        let resolved = self.resolve_type(elem);
        let key = format!("{resolved:?}");
        let hash = key.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        });
        let mut procs = Vec::new();
        for op in ["bruto_astr_release", "bruto_astr_addref"] {
            let name = format!("bruto.dynelem.{hash:016x}.{op}");
            if let Some(f) = self.module.get_function(&name) {
                procs.push(f.as_global_value().as_pointer_value());
                continue;
            }
            let i64_ty = self.context.i64_type();
            let f = self.module.add_function(
                &name,
                self.context
                    .void_type()
                    .fn_type(&[ptr.into(), i64_ty.into()], false),
                None,
            );
            f.set_linkage(Linkage::Internal);
            // Build it on the side, then come back.
            let saved_block = self.builder.get_insert_block();
            let saved_fn = self.current_fn;
            let saved_loc = self.builder.get_current_debug_location();
            self.builder.unset_current_debug_location();
            self.current_fn = Some(f);
            let entry = self.context.append_basic_block(f, "entry");
            let head = self.context.append_basic_block(f, "head");
            let body = self.context.append_basic_block(f, "body");
            let done = self.context.append_basic_block(f, "done");
            let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), None);
            self.builder.position_at_end(entry);
            let base = f.get_nth_param(0).unwrap().into_pointer_value();
            let count = f.get_nth_param(1).unwrap().into_int_value();
            let index = self.builder.build_alloca(i64_ty, "i").map_err(err)?;
            self.builder
                .build_store(index, i64_ty.const_zero())
                .map_err(err)?;
            self.builder.build_unconditional_branch(head).map_err(err)?;
            self.builder.position_at_end(head);
            let i = self
                .builder
                .build_load(i64_ty, index, "i")
                .map_err(err)?
                .into_int_value();
            let more = self
                .builder
                .build_int_compare(inkwell::IntPredicate::SLT, i, count, "more")
                .map_err(err)?;
            self.builder
                .build_conditional_branch(more, body, done)
                .map_err(err)?;
            self.builder.position_at_end(body);
            let elem_llvm = self.llvm_type_for(&resolved);
            let slot = unsafe {
                self.builder
                    .build_in_bounds_gep(elem_llvm, base, &[i], "elem")
                    .map_err(err)?
            };
            self.arc_walk(slot, &resolved, op, Span::default())?;
            let next = self
                .builder
                .build_int_add(i, i64_ty.const_int(1, false), "next")
                .map_err(err)?;
            self.builder.build_store(index, next).map_err(err)?;
            self.builder.build_unconditional_branch(head).map_err(err)?;
            self.builder.position_at_end(done);
            self.builder.build_return(None).map_err(err)?;
            self.current_fn = saved_fn;
            if let Some(block) = saved_block {
                self.builder.position_at_end(block);
            }
            if let Some(loc) = saved_loc {
                self.builder.set_current_debug_location(loc);
            }
            procs.push(f.as_global_value().as_pointer_value());
        }
        Ok((procs[0], procs[1]))
    }

    /// The address of the array `expr` gives: its variable, or a
    /// temporary holding the value.
    fn array_slot(
        &mut self,
        expr: &Expr,
        span: Span,
    ) -> Result<(PointerValue<'ctx>, PascalType), CodeGenError> {
        let ty = self.infer_declared_type(expr);
        if self.is_addressable(expr) {
            return self.resolve_array_base(expr, span);
        }
        let value = self.compile_expr(expr)?;
        let slot = self
            .entry_alloca(value.get_type(), "array_tmp")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder
            .build_store(slot, value)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok((slot, ty))
    }

    /// The first element's address of the dynamic or open array whose
    /// value is at `slot`.
    pub(super) fn array_data(
        &mut self,
        slot: PointerValue<'ctx>,
        ty: &PascalType,
        span: Span,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr = self.context.ptr_type(AddressSpace::default());
        let at = match self.resolve_type(ty) {
            PascalType::OpenArray(_) => self
                .builder
                .build_struct_gep(self.open_array_type(), slot, 0, "open_data")
                .map_err(err)?,
            _ => slot,
        };
        Ok(self
            .builder
            .build_load(ptr, at, "array_data")
            .map_err(err)?
            .into_pointer_value())
    }

    /// The number of elements of the dynamic or open array at `slot`.
    fn array_length(
        &mut self,
        slot: PointerValue<'ctx>,
        ty: &PascalType,
        span: Span,
    ) -> Result<IntValue<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i64_ty = self.context.i64_type();
        match self.resolve_type(ty) {
            PascalType::OpenArray(_) => {
                let at = self
                    .builder
                    .build_struct_gep(self.open_array_type(), slot, 1, "open_high")
                    .map_err(err)?;
                let high = self
                    .builder
                    .build_load(i64_ty, at, "high")
                    .map_err(err)?
                    .into_int_value();
                self.builder
                    .build_int_add(high, i64_ty.const_int(1, false), "len")
                    .map_err(err)
            }
            _ => {
                let data = self.array_data(slot, ty, span)?;
                Ok(self
                    .call_ustr_runtime("bruto_astr_len", &[data.into()], span)?
                    .unwrap()
                    .into_int_value())
            }
        }
    }

    /// The address of element `index` of the dynamic or open array at
    /// `slot`.
    pub(super) fn array_elem(
        &mut self,
        slot: PointerValue<'ctx>,
        ty: &PascalType,
        index: IntValue<'ctx>,
        span: Span,
    ) -> Result<(PointerValue<'ctx>, PascalType), CodeGenError> {
        let elem = self.array_elem_of(ty).unwrap();
        let data = self.array_data(slot, ty, span)?;
        let elem_llvm = self.llvm_type_for(&self.resolve_type(&elem));
        let at = unsafe {
            self.builder
                .build_in_bounds_gep(elem_llvm, data, &[index], "dyn_elem")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
        };
        Ok((at, elem))
    }

    /// `Length`, `High`, `Low` and `Copy` on dynamic and open arrays.
    pub(super) fn dynarray_func(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let Some(first) = args.first() else {
            return Ok(None);
        };
        let ty = self.infer_declared_type(first);
        let Some(elem) = self.array_elem_of(&ty) else {
            return Ok(None);
        };
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i64_ty = self.context.i64_type();
        Ok(Some(match (name, args.len()) {
            ("length", 1) => {
                let (slot, ty) = self.array_slot(first, span)?;
                self.array_length(slot, &ty, span)?.into()
            }
            ("high", 1) => {
                let (slot, ty) = self.array_slot(first, span)?;
                let len = self.array_length(slot, &ty, span)?;
                self.builder
                    .build_int_sub(len, i64_ty.const_int(1, false), "high")
                    .map_err(err)?
                    .into()
            }
            ("low", 1) => i64_ty.const_zero().into(),
            ("copy", 1..=3) if matches!(self.resolve_type(&ty), PascalType::DynArray(_)) => {
                let array = self.compile_expr(first)?;
                let start = match args.get(1) {
                    Some(a) => self.int_arg(a, span)?,
                    None => i64_ty.const_zero(),
                };
                let count = match args.get(2) {
                    Some(a) => self.int_arg(a, span)?,
                    None => i64_ty.const_int(i64::MAX as u64, false),
                };
                let size = self.elem_size(&elem);
                self.call_ustr_runtime(
                    "bruto_dynarr_copy",
                    &[array.into(), start.into(), count.into(), size.into()],
                    span,
                )?
                .unwrap()
            }
            _ => return Ok(None),
        }))
    }

    /// `Delete(A, Index, Count)` and `Insert(Value, A, Index)` on a
    /// dynamic array; whether `args` were that.
    pub(super) fn dynarray_insert_delete(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<bool, CodeGenError> {
        let target = match (name, args.len()) {
            ("delete", 3) => &args[0],
            ("insert", 3) => &args[1],
            _ => return Ok(false),
        };
        let ty = self.infer_declared_type(target);
        let Some(PascalType::DynArray(elem)) = Some(self.resolve_type(&ty)) else {
            return Ok(false);
        };
        let (slot, ty) = self.resolve_array_base(target, span)?;
        let size = self.elem_size(&elem);
        if name == "delete" {
            let index = self.int_arg(&args[1], span)?;
            let count = self.int_arg(&args[2], span)?;
            self.call_ustr_runtime(
                "bruto_dynarr_delete",
                &[slot.into(), index.into(), count.into(), size.into()],
                span,
            )?;
            return Ok(true);
        }
        let value = self.compile_expr_for(&args[0], &elem)?;
        let index = self.int_arg(&args[2], span)?;
        let (release, addref) = self.dynarray_elem_procs(&elem)?;
        let at = self
            .call_ustr_runtime(
                "bruto_dynarr_insert_gap",
                &[
                    slot.into(),
                    index.into(),
                    size.into(),
                    release.into(),
                    addref.into(),
                ],
                span,
            )?
            .unwrap()
            .into_int_value();
        let (elem_slot, elem_ty) = self.array_elem(slot, &ty, at, span)?;
        let elem_resolved = self.resolve_type(&elem_ty);
        self.store_value_init(elem_slot, value, &elem_resolved, span)?;
        Ok(true)
    }

    /// `SetLength(A, N[, M...])` on a dynamic array; whether `args` were
    /// that.
    pub(super) fn dynarray_setlength(
        &mut self,
        args: &[Expr],
        span: Span,
    ) -> Result<bool, CodeGenError> {
        if args.len() < 2 {
            return Ok(false);
        }
        let ty = self.infer_declared_type(&args[0]);
        if !matches!(self.resolve_type(&ty), PascalType::DynArray(_)) {
            return Ok(false);
        }
        let (slot, ty) = self.resolve_array_base(&args[0], span)?;
        let mut sizes = Vec::new();
        for a in &args[1..] {
            sizes.push(self.int_arg(a, span)?);
        }
        self.set_length_at(slot, &ty, &sizes, span)?;
        Ok(true)
    }

    /// Give the dynamic array at `slot` the lengths `sizes`, one per
    /// dimension.
    fn set_length_at(
        &mut self,
        slot: PointerValue<'ctx>,
        ty: &PascalType,
        sizes: &[IntValue<'ctx>],
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let Some(elem) = self.array_elem_of(ty) else {
            return Err(CodeGenError::new(
                "SetLength has more lengths than the array has dimensions",
                Some(span),
            ));
        };
        let size = self.elem_size(&elem);
        let (release, addref) = self.dynarray_elem_procs(&elem)?;
        self.call_ustr_runtime(
            "bruto_dynarr_setlength",
            &[
                slot.into(),
                sizes[0].into(),
                size.into(),
                release.into(),
                addref.into(),
            ],
            span,
        )?;
        if sizes.len() == 1 {
            return Ok(());
        }
        // Each element gets the remaining lengths.
        let i64_ty = self.context.i64_type();
        let func = self.current_fn.unwrap();
        let index = self.entry_alloca(i64_ty, "setlen_i").map_err(err)?;
        self.builder
            .build_store(index, i64_ty.const_zero())
            .map_err(err)?;
        let head = self.context.append_basic_block(func, "setlen_head");
        let body = self.context.append_basic_block(func, "setlen_body");
        let done = self.context.append_basic_block(func, "setlen_done");
        self.builder.build_unconditional_branch(head).map_err(err)?;
        self.builder.position_at_end(head);
        let i = self
            .builder
            .build_load(i64_ty, index, "i")
            .map_err(err)?
            .into_int_value();
        let more = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SLT, i, sizes[0], "more")
            .map_err(err)?;
        self.builder
            .build_conditional_branch(more, body, done)
            .map_err(err)?;
        self.builder.position_at_end(body);
        let (at, elem_ty) = self.array_elem(slot, ty, i, span)?;
        self.set_length_at(at, &elem_ty, &sizes[1..], span)?;
        let next = self
            .builder
            .build_int_add(i, i64_ty.const_int(1, false), "next")
            .map_err(err)?;
        self.builder.build_store(index, next).map_err(err)?;
        self.builder.build_unconditional_branch(head).map_err(err)?;
        self.builder.position_at_end(done);
        Ok(())
    }

    /// A new dynamic array of `elem` holding `elements` (`A := [1, 2]`).
    pub(super) fn dynarray_from_elements(
        &mut self,
        elements: &[SetElement],
        elem: &PascalType,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let i64_ty = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let size = self.elem_size(elem);
        let (release, addref) = self.dynarray_elem_procs(elem)?;
        let count = i64_ty.const_int(elements.len() as u64, false);
        if elements.is_empty() {
            return Ok(ptr.const_null().into());
        }
        let block = self
            .call_ustr_runtime(
                "bruto_dynarr_block",
                &[count.into(), size.into(), release.into(), addref.into()],
                span,
            )?
            .unwrap()
            .into_pointer_value();
        let elem_resolved = self.resolve_type(elem);
        let elem_llvm = self.llvm_type_for(&elem_resolved);
        for (i, e) in elements.iter().enumerate() {
            let SetElement::Single(e) = e else {
                return Err(CodeGenError::new(
                    "a range can't be an array element",
                    Some(span),
                ));
            };
            let at = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        elem_llvm,
                        block,
                        &[i64_ty.const_int(i as u64, false)],
                        "init",
                    )
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?
            };
            let value = self.compile_expr_for(e, elem)?;
            self.store_value_init(at, value, &elem_resolved, span)?;
        }
        Ok(block.into())
    }

    /// The `{ data, high }` an open array parameter of `elem` gets from
    /// `arg`.
    pub(super) fn open_array_arg(
        &mut self,
        arg: &Expr,
        elem: &PascalType,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i64_ty = self.context.i64_type();
        let pair = self.open_array_type();
        let make = |s: &Self,
                    data: PointerValue<'ctx>,
                    high: IntValue<'ctx>|
         -> Result<BasicValueEnum<'ctx>, CodeGenError> {
            let v = s
                .builder
                .build_insert_value(pair.get_undef(), data, 0, "open")
                .map_err(err)?;
            let v = s
                .builder
                .build_insert_value(v, high, 1, "open")
                .map_err(err)?;
            Ok(v.into_struct_value().into())
        };
        // `[a, b, c]`: the elements in a temporary array.
        if let Expr::SetConstructor { elements, .. } = arg {
            let is_varrec = matches!(elem, PascalType::Named(n) if n == "tvarrec");
            let elem_resolved = self.resolve_type(elem);
            let elem_llvm = self.llvm_type_for(&elem_resolved);
            let count = elements.len().max(1) as u32;
            let array = self
                .entry_alloca(elem_llvm.array_type(count), "open_tmp")
                .map_err(err)?;
            for (i, e) in elements.iter().enumerate() {
                let SetElement::Single(e) = e else {
                    return Err(CodeGenError::new(
                        "a range can't be an array element",
                        Some(span),
                    ));
                };
                let at = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            elem_llvm,
                            array,
                            &[i64_ty.const_int(i as u64, false)],
                            "open_elem",
                        )
                        .map_err(err)?
                };
                if is_varrec {
                    self.store_varrec(at, e, span)?;
                } else {
                    let value = self.compile_expr_for(e, elem)?;
                    let value = self.convert_for_store(value, &elem_resolved, span)?;
                    self.builder.build_store(at, value).map_err(err)?;
                }
            }
            let high = i64_ty.const_int((elements.len() as i64 - 1) as u64, true);
            return make(self, array, high);
        }
        let ty = self.infer_declared_type(arg);
        match self.resolve_type(&ty) {
            PascalType::OpenArray(_) => self.compile_expr(arg),
            PascalType::DynArray(_) => {
                let (slot, ty) = self.array_slot(arg, span)?;
                let data = self.array_data(slot, &ty, span)?;
                let len = self.array_length(slot, &ty, span)?;
                let high = self
                    .builder
                    .build_int_sub(len, i64_ty.const_int(1, false), "high")
                    .map_err(err)?;
                make(self, data, high)
            }
            PascalType::Array { lo, hi, .. } => {
                let (slot, _) = self.array_slot(arg, span)?;
                make(self, slot, i64_ty.const_int((hi - lo) as u64, true))
            }
            _ => {
                // A single value for `array of const` or `array of T`.
                let one = Expr::SetConstructor {
                    elements: vec![SetElement::Single(arg.clone())],
                    span,
                };
                self.open_array_arg(&one, elem, span)
            }
        }
    }

    /// Fill the TVarRec at `at` from `value`, tagged with its type (FPC's
    /// `vtInteger`, `vtAnsiString`... codes).
    fn store_varrec(
        &mut self,
        at: PointerValue<'ctx>,
        value: &Expr,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let i64_ty = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let tag_at = at;
        let payload_at = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    at,
                    &[i64_ty.const_int(8, false)],
                    "vpayload",
                )
                .map_err(err)?
        };
        // A one-character literal is a Char, as in FPC.
        let ty = match value {
            Expr::StrLit(text, _) if text.len() == 1 => PascalType::Char,
            _ => self.resolve_type(&self.infer_declared_type(value)),
        };
        // A value kept in a temporary, passed by address.
        let boxed =
            |s: &mut Self, v: BasicValueEnum<'ctx>| -> Result<PointerValue<'ctx>, CodeGenError> {
                let slot = s.entry_alloca(v.get_type(), "varrec_value").map_err(err)?;
                s.builder.build_store(slot, v).map_err(err)?;
                Ok(slot)
            };
        let (tag, payload): (IntValue<'ctx>, BasicValueEnum<'ctx>) = match &ty {
            PascalType::Integer | PascalType::Enum { .. } | PascalType::Subrange { .. } => {
                // vtInteger when it fits a LongInt, vtInt64 otherwise.
                let v = self.compile_expr(value)?;
                let v = self.to_i64(v, span)?;
                let low = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::SGE,
                        v,
                        i64_ty.const_int(i32::MIN as i64 as u64, true),
                        "fits_lo",
                    )
                    .map_err(err)?;
                let high = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::SLE,
                        v,
                        i64_ty.const_int(i32::MAX as u64, true),
                        "fits_hi",
                    )
                    .map_err(err)?;
                let fits = self.builder.build_and(low, high, "fits").map_err(err)?;
                let boxed_v = boxed(self, v.into())?;
                let as_ptr = self
                    .builder
                    .build_ptr_to_int(boxed_v, i64_ty, "boxed")
                    .map_err(err)?;
                let tag = self
                    .builder
                    .build_select(
                        fits,
                        i64_ty.const_int(0, false),
                        i64_ty.const_int(16, false),
                        "vtype",
                    )
                    .map_err(err)?
                    .into_int_value();
                let payload = self
                    .builder
                    .build_select(fits, v, as_ptr, "vvalue")
                    .map_err(err)?;
                (tag, payload)
            }
            PascalType::SizedInt { bits, signed } => {
                let v = self.compile_expr(value)?;
                let v = self.to_i64(v, span)?;
                if *bits <= 32 || (*bits < 64 && *signed) {
                    (i64_ty.const_int(0, false), v.into())
                } else {
                    let boxed_v = boxed(self, v.into())?;
                    let tag = if *signed { 16 } else { 17 };
                    (i64_ty.const_int(tag, false), boxed_v.into())
                }
            }
            PascalType::Boolean => {
                let v = self.compile_expr(value)?;
                let v = self.to_i64(v, span)?;
                (i64_ty.const_int(1, false), v.into())
            }
            PascalType::Char => {
                let v = match value {
                    Expr::StrLit(text, _) if text.len() == 1 => {
                        i64_ty.const_int(text.as_bytes()[0] as u64, false)
                    }
                    _ => {
                        let v = self.compile_expr(value)?;
                        self.to_i64(v, span)?
                    }
                };
                (i64_ty.const_int(2, false), v.into())
            }
            PascalType::WideChar => {
                let v = self.compile_expr(value)?;
                let v = self.to_i64(v, span)?;
                (i64_ty.const_int(9, false), v.into())
            }
            PascalType::Real => {
                let v = self.compile_expr(value)?;
                let v = if v.is_float_value() {
                    v
                } else {
                    let i = self.to_i64(v, span)?;
                    self.builder
                        .build_signed_int_to_float(i, self.context.f64_type(), "real")
                        .map_err(err)?
                        .into()
                };
                (i64_ty.const_int(3, false), boxed(self, v)?.into())
            }
            PascalType::ShortString { .. } => {
                let (slot, _) = self.array_slot(value, span)?;
                (i64_ty.const_int(4, false), slot.into())
            }
            PascalType::String => {
                let v = self.compile_expr(value)?;
                let v = self.as_string_value(v, span)?;
                (i64_ty.const_int(11, false), v)
            }
            PascalType::UnicodeString => {
                let v = self.compile_expr(value)?;
                let p = self.ustr_ptr(v, span)?;
                (i64_ty.const_int(18, false), p.into())
            }
            PascalType::ClassOf(_) => (i64_ty.const_int(8, false), self.compile_expr(value)?),
            PascalType::Pointer(inner) => {
                let v = self.compile_expr(value)?;
                let tag = if self.class_instance_of(&ty).is_some() {
                    7
                } else if **inner == PascalType::Char {
                    6
                } else if **inner == PascalType::WideChar {
                    10
                } else {
                    5
                };
                (i64_ty.const_int(tag, false), v)
            }
            _ if matches!(value, Expr::Nil(_)) => {
                (i64_ty.const_int(5, false), ptr.const_null().into())
            }
            _ => {
                return Err(CodeGenError::new(
                    "this value can't be passed in an array of const",
                    Some(span),
                ));
            }
        };
        // Clear the value word, then store the tag and the value.
        self.builder
            .build_store(payload_at, i64_ty.const_zero())
            .map_err(err)?;
        self.builder.build_store(tag_at, tag).map_err(err)?;
        let payload = match payload {
            BasicValueEnum::IntValue(i) if i.get_type().get_bit_width() < 64 => self
                .builder
                .build_int_z_extend(i, i64_ty, "vwide")
                .map_err(err)?
                .into(),
            other => other,
        };
        self.builder.build_store(payload_at, payload).map_err(err)?;
        Ok(())
    }
}
