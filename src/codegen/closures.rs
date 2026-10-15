//! Procedural values, plain and nested.
//!
//! A plain procedural value is a code pointer. A nested one (FPC's
//! `... is nested`) can also refer to a local routine, which reaches its
//! parent's variables: it is a pointer to a closure `{ code, env }` built
//! in the frame that creates it. `code` is a trampoline taking the
//! routine's parameters plus `env` last; `env` is an array of pointers to
//! the variables the routine captures (Bruto passes captures to a nested
//! routine as leading pointer parameters, so the trampoline unpacks them).
//! Top-level routines get a trampoline too, with an empty `env`, so every
//! nested call has the same shape.

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType};
use inkwell::values::BasicValue;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// Call the procedural value `target` (a code pointer, or a closure
    /// for a nested type) of type `proc_ty` with `args`.
    pub(super) fn call_procedural(
        &mut self,
        target: PointerValue<'ctx>,
        proc_ty: &PascalType,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let PascalType::Proc {
            params,
            return_type,
            nested,
            of_object,
        } = self.resolve_type(proc_ty)
        else {
            return Err(CodeGenError::new(
                "this isn't a procedural value",
                Some(span),
            ));
        };
        if params.len() != args.len() {
            return Err(CodeGenError::new(
                format!("expected {} arguments, found {}", params.len(), args.len()),
                Some(span),
            ));
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let mut param_types: Vec<BasicMetadataTypeEnum> = Vec::new();
        let mut call_args: Vec<BasicMetadataValueEnum> = Vec::new();
        // A method pointer (`target` is its slot): `Code(Data, args...)`.
        let mut method_code = None;
        if of_object {
            let pair = self.method_pointer_type();
            let code_addr = self
                .builder
                .build_struct_gep(pair, target, 0, "code_addr")
                .map_err(err)?;
            let data_addr = self
                .builder
                .build_struct_gep(pair, target, 1, "data_addr")
                .map_err(err)?;
            let code = self
                .builder
                .build_load(ptr, code_addr, "code")
                .map_err(err)?
                .into_pointer_value();
            let data = self
                .builder
                .build_load(ptr, data_addr, "data")
                .map_err(err)?;
            param_types.push(ptr.into());
            call_args.push(data.into());
            method_code = Some(code);
        }
        for (param, arg) in params.iter().zip(args) {
            // A `var` parameter: the argument's address.
            if let PascalType::VarParam(_) = param {
                param_types.push(ptr.into());
                call_args.push(self.address_of_arg(arg, span)?.into());
                continue;
            }
            let param = self.resolve_type(param);
            param_types.push(self.abi_type_for(&param).into());
            let val = self.compile_expr(arg)?;
            call_args.push(self.convert_for_store(val, &param, span)?.into());
        }
        let code = if let Some(code) = method_code {
            code
        } else if nested {
            // Unpack the closure: the trampoline, and its environment last.
            let code = self
                .builder
                .build_load(ptr, target, "code")
                .map_err(err)?
                .into_pointer_value();
            let env_slot = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        ptr,
                        target,
                        &[self.context.i64_type().const_int(1, false)],
                        "env_slot",
                    )
                    .map_err(err)?
            };
            let env = self.builder.build_load(ptr, env_slot, "env").map_err(err)?;
            param_types.push(ptr.into());
            call_args.push(env.into());
            code
        } else {
            target
        };
        let fn_type = match &return_type {
            Some(rt) => self.abi_type_for(rt).fn_type(&param_types, false),
            None => self.context.void_type().fn_type(&param_types, false),
        };
        let result = self
            .builder
            .build_indirect_call(fn_type, code, &call_args, "pcall")
            .map_err(err)?
            .try_as_basic_value()
            .basic();
        match (result, return_type) {
            (Some(v), Some(rt)) => Ok(Some(self.widen_loaded(v, &rt, span)?)),
            (v, _) => Ok(v),
        }
    }

    /// A closure for routine `name` as a value of the nested procedural
    /// type `proc_ty`, built in the current frame (so it is valid while
    /// that routine runs).
    pub(super) fn make_closure(
        &mut self,
        name: &str,
        proc_ty: &PascalType,
        span: Span,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let symbol = self.overload_for_proc_type(name, proc_ty);
        let func = self.module.get_function(&symbol).ok_or_else(|| {
            CodeGenError::new(
                format!("'{}' is not a procedure or function", self.display(name)),
                Some(span),
            )
        })?;
        let captures = self.proc_captures.get(&symbol).cloned().unwrap_or_default();
        let trampoline = self.trampoline(&symbol, func, captures.len());
        let ptr = self.context.ptr_type(AddressSpace::default());
        let env = if captures.is_empty() {
            ptr.const_null()
        } else {
            let env_ty = ptr.array_type(captures.len() as u32);
            let env = self.entry_alloca(env_ty, "closure_env").map_err(err)?;
            for (i, (cname, _)) in captures.iter().enumerate() {
                let address = *self.lookup_var(cname).ok_or_else(|| {
                    CodeGenError::new(
                        format!(
                            "'{}' uses '{cname}', which is not in scope here",
                            self.display(name)
                        ),
                        Some(span),
                    )
                })?;
                let slot = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            env_ty,
                            env,
                            &[
                                self.context.i64_type().const_zero(),
                                self.context.i64_type().const_int(i as u64, false),
                            ],
                            "env_entry",
                        )
                        .map_err(err)?
                };
                self.builder.build_store(slot, address).map_err(err)?;
            }
            env
        };
        let closure_ty = self.context.struct_type(&[ptr.into(), ptr.into()], false);
        let closure = self.entry_alloca(closure_ty, "closure").map_err(err)?;
        let code_slot = self
            .builder
            .build_struct_gep(closure_ty, closure, 0, "closure_code")
            .map_err(err)?;
        self.builder
            .build_store(code_slot, trampoline.as_global_value().as_pointer_value())
            .map_err(err)?;
        let env_slot = self
            .builder
            .build_struct_gep(closure_ty, closure, 1, "closure_env_slot")
            .map_err(err)?;
        self.builder.build_store(env_slot, env).map_err(err)?;
        Ok(closure)
    }

    /// For an overloaded routine, the overload matching a procedural
    /// type's parameter types.
    fn overload_for_proc_type(&self, name: &str, proc_ty: &PascalType) -> String {
        let (Some(candidates), PascalType::Proc { params, .. }) =
            (self.overloads.get(name), self.resolve_type(proc_ty))
        else {
            return name.to_string();
        };
        candidates
            .iter()
            .find(|symbol| {
                self.proc_param_modes.get(*symbol).is_some_and(|ps| {
                    ps.len() == params.len()
                        && ps
                            .iter()
                            .zip(&params)
                            .all(|((_, _, t), p)| self.resolve_type(t) == self.resolve_type(p))
                })
            })
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    /// `<symbol>$closure(params..., env)`: unpack `captures` pointers from
    /// `env` and call the routine with them first.
    fn trampoline(
        &self,
        symbol: &str,
        func: FunctionValue<'ctx>,
        captures: usize,
    ) -> FunctionValue<'ctx> {
        let name = format!("{symbol}$closure");
        if let Some(f) = self.module.get_function(&name) {
            return f;
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let target_ty = func.get_type();
        let user_params: Vec<BasicMetadataTypeEnum> = target_ty
            .get_param_types()
            .into_iter()
            .skip(captures)
            .chain(std::iter::once(ptr.into()))
            .collect();
        let fn_ty = match target_ty.get_return_type() {
            Some(rt) => rt.fn_type(&user_params, false),
            None => self.context.void_type().fn_type(&user_params, false),
        };
        let tramp = self.module.add_function(&name, fn_ty, None);
        tramp.set_linkage(Linkage::Internal);
        let b = self.context.create_builder();
        b.position_at_end(self.context.append_basic_block(tramp, "entry"));
        let n_user = user_params.len() - 1;
        let env = tramp
            .get_nth_param(n_user as u32)
            .unwrap()
            .into_pointer_value();
        let mut args: Vec<BasicMetadataValueEnum> = Vec::new();
        for i in 0..captures {
            let slot = unsafe {
                b.build_in_bounds_gep(
                    ptr,
                    env,
                    &[self.context.i64_type().const_int(i as u64, false)],
                    "cap_slot",
                )
                .unwrap()
            };
            args.push(b.build_load(ptr, slot, "cap").unwrap().into());
        }
        for i in 0..n_user {
            args.push(tramp.get_nth_param(i as u32).unwrap().into());
        }
        let call = b.build_call(func, &args, "forward").unwrap();
        match call.try_as_basic_value().basic() {
            Some(v) => b.build_return(Some(&v)).unwrap(),
            None => b.build_return(None).unwrap(),
        };
        tramp
    }

    /// FPC's `TMethod`: `{ Code, Data }`.
    pub(super) fn method_pointer_type(&self) -> inkwell::types::StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(&[ptr.into(), ptr.into()], false)
    }

    /// Whether `ty` (resolved) is a method pointer type (`... of object`).
    pub(super) fn is_method_pointer(ty: &PascalType) -> bool {
        matches!(
            ty,
            PascalType::Proc {
                of_object: true,
                ..
            }
        )
    }

    /// The code pointer of `val`: itself, or a method pointer's `Code`.
    pub(super) fn code_part(
        &self,
        val: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        match val {
            BasicValueEnum::StructValue(s) if s.get_type() == self.method_pointer_type() => self
                .builder
                .build_extract_value(s, 0, "code")
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span))),
            other => Ok(other),
        }
    }

    /// The address of the method pointer `expr` gives (held in a
    /// temporary if it isn't a variable or field).
    pub(super) fn method_pointer_slot(
        &mut self,
        expr: &Expr,
        span: Span,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        if self.is_addressable(expr) {
            return Ok(self.resolve_array_base(expr, span)?.0);
        }
        let value = self.compile_expr(expr)?;
        let slot = self
            .entry_alloca(self.method_pointer_type(), "method_tmp")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder
            .build_store(slot, value)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(slot)
    }

    /// `expr` as a method pointer value: `nil`, another method pointer,
    /// or a method of an instance (`@Obj.Method`, `Obj.Method`, or a
    /// method of `Self` by name). A virtual method is looked up in the
    /// instance's VMT now, as FPC does.
    pub(super) fn compile_method_pointer(
        &mut self,
        expr: &Expr,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let span = expr.span();
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let pair = self.method_pointer_type();
        let inner = match expr {
            Expr::Nil(_) => return Ok(pair.const_zero().into()),
            Expr::AddrOf(inner, _) => inner.as_ref(),
            other => other,
        };
        // The instance and the method's type and name.
        let (object, instance, method) = match inner {
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } if args.is_empty() => {
                if let Some(class) = self.class_instance_of(&self.infer_declared_type(object)) {
                    let instance = self.compile_expr(object)?.into_pointer_value();
                    (class, instance, method.clone())
                } else {
                    let (instance, ty) = self.resolve_array_base(object, span)?;
                    let name = self.object_name_of(&ty).ok_or_else(|| {
                        CodeGenError::new("this isn't a method of an instance", Some(span))
                    })?;
                    (name, instance, method.clone())
                }
            }
            Expr::FieldAccess { record, field, .. }
                if self.is_parameterless_method(record, field) =>
            {
                let (instance, ty) = self.resolve_array_base(record, span)?;
                let name = self.object_name_of(&ty).unwrap();
                (name, instance, field.clone())
            }
            Expr::Var(name, _) if self.implicit_method(name).is_some() => {
                let (object, instance) = self.implicit_method(name).unwrap();
                (object, instance, name.clone())
            }
            _ => {
                // Already a method pointer.
                let value = self.compile_expr(expr)?;
                if value.get_type() == pair.as_basic_type_enum() {
                    return Ok(value);
                }
                return Err(CodeGenError::new(
                    "expected a method of an instance (`@Obj.Method`) or nil",
                    Some(span),
                ));
            }
        };
        let info = self.objects[&object].clone();
        let m = info.methods.get(&method).cloned().ok_or_else(|| {
            CodeGenError::new(
                format!(
                    "'{}' has no method '{}'",
                    self.display(&object),
                    self.display(&method)
                ),
                Some(span),
            )
        })?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let code: PointerValue = match (m.slot, info.vmt_field) {
            (Some(slot), Some(_)) => {
                let vmt = self.load_vmt(&object, instance, span)?;
                let header = self.vmt_header_words(&object);
                let addr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            ptr,
                            vmt,
                            &[self
                                .context
                                .i64_type()
                                .const_int(header + slot as u64, false)],
                            "slot_addr",
                        )
                        .map_err(err)?
                };
                self.builder
                    .build_load(ptr, addr, "method")
                    .map_err(err)?
                    .into_pointer_value()
            }
            _ => self
                .module
                .get_function(&m.symbol)
                .map(|f| f.as_global_value().as_pointer_value())
                .ok_or_else(|| {
                    CodeGenError::new(format!("method '{}' has no body", m.symbol), Some(span))
                })?,
        };
        let value = self
            .builder
            .build_insert_value(pair.get_undef(), code, 0, "code")
            .map_err(err)?;
        let value = self
            .builder
            .build_insert_value(value, instance, 1, "data")
            .map_err(err)?;
        Ok(value.as_basic_value_enum())
    }
}
