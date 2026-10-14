//! Turbo Pascal `object` types: lowering to records, method tables,
//! VMTs and method calls.
//!
//! An object type becomes a `Record` (with `object` set to the type's
//! name) whose fields are the inherited fields, a hidden `__vmt` pointer
//! where the first type with virtual methods starts, and the type's own
//! fields. Its methods are LLVM functions named `<type>.<method>` taking a
//! hidden `Self` pointer first.
//!
//! The calling conventions follow what FPC's `objects.pp` relies on when
//! it calls through raw code pointers:
//! - methods: `(Self, params...)`;
//! - constructors: `(Self, VMT, params...) -> Self`. With a nil `Self`
//!   and a VMT, the constructor allocates the object (zeroed, using the
//!   VMT's size field); with a VMT it stores it in the object; with a nil
//!   VMT (`inherited Init`) it leaves the VMT field alone.
//!
//! Each type gets a VMT global laid out as FPC's is:
//! `{ size: i64, -size: i64, parent_link: ^^VMT, slots: [ptr; n] }`, where
//! `parent_link` points at a cell holding the parent's VMT.

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, FunctionType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue};
use std::collections::HashMap;

/// Name of the hidden VMT pointer field.
pub(super) const VMT_FIELD: &str = "__vmt";
/// Bytes before the first slot in a VMT: size, -size, parent link.
const VMT_HEADER_WORDS: u64 = 3;

/// What code generation knows about an object type.
#[derive(Debug, Clone)]
pub(super) struct ObjectInfo {
    pub parent: Option<String>,
    /// The lowered record type.
    pub record: PascalType,
    /// Index of `__vmt` among the record's fields, if any type in the
    /// chain has virtual methods.
    pub vmt_field: Option<usize>,
    /// Methods by (lower-case) name, own and inherited.
    pub methods: HashMap<String, MethodInfo>,
    /// The routine in each virtual slot: a method symbol, or `None` for an
    /// abstract method.
    pub slots: Vec<Option<String>>,
}

#[derive(Debug, Clone)]
pub(super) struct MethodInfo {
    /// The LLVM function implementing it: `<owner>.<name>`.
    pub symbol: String,
    pub kind: RoutineKind,
    pub params: Vec<ParamGroup>,
    pub return_type: Option<PascalType>,
    /// Its virtual slot, if virtual.
    pub slot: Option<usize>,
    pub is_abstract: bool,
}

/// The method being compiled: its object type and `Self`.
#[derive(Debug, Clone, Copy)]
pub(super) struct MethodContext<'ctx> {
    pub self_ptr: PointerValue<'ctx>,
}

/// The object a method is called on, and how.
#[derive(Debug, Clone, Copy)]
pub(super) struct Receiver<'a, 'ctx> {
    /// The object type whose method table is used.
    pub object: &'a str,
    pub self_ptr: PointerValue<'ctx>,
    pub dispatch: Dispatch,
    /// The VMT argument, for a constructor.
    pub vmt: Option<PointerValue<'ctx>>,
}

/// How a call reaches a method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Dispatch {
    /// Through the object's VMT if the method is virtual.
    Virtual,
    /// Straight to the named type's implementation (`inherited`).
    Static,
}

impl<'ctx> CodeGen<'ctx> {
    /// Lower an object type declaration: register its record type and
    /// method table, and declare its methods' functions.
    pub(super) fn register_object(
        &mut self,
        name: &str,
        decl: &ObjectType,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let parent = match &decl.parent {
            Some(p) => Some(self.objects.get(p).cloned().ok_or_else(|| {
                CodeGenError::new(
                    format!("'{p}' is not an object type declared before '{name}'"),
                    Some(span),
                )
            })?),
            None => None,
        };
        let mut fields = match &parent {
            Some(p) => match &p.record {
                PascalType::Record { fields, .. } => fields.clone(),
                _ => Vec::new(),
            },
            None => Vec::new(),
        };
        let mut vmt_field = parent.as_ref().and_then(|p| p.vmt_field);
        if vmt_field.is_none() && decl.methods.iter().any(|m| m.is_virtual) {
            vmt_field = Some(fields.len());
            fields.push((
                VMT_FIELD.to_string(),
                PascalType::Pointer(Box::new(PascalType::Untyped)),
            ));
        }
        for (fname, fty) in &decl.fields {
            fields.push((fname.clone(), self.resolve_type(fty)));
        }

        let mut methods = parent
            .as_ref()
            .map(|p| p.methods.clone())
            .unwrap_or_default();
        let mut slots = parent.as_ref().map(|p| p.slots.clone()).unwrap_or_default();
        for m in &decl.methods {
            let symbol = format!("{name}.{}", m.name);
            let slot = if m.is_virtual {
                let inherited = methods.get(&m.name).and_then(|prev| prev.slot);
                let slot = inherited.unwrap_or_else(|| {
                    slots.push(None);
                    slots.len() - 1
                });
                slots[slot] = (!m.is_abstract).then(|| symbol.clone());
                Some(slot)
            } else {
                None
            };
            methods.insert(
                m.name.clone(),
                MethodInfo {
                    symbol,
                    kind: m.kind,
                    params: m.params.clone(),
                    return_type: m.return_type.clone(),
                    slot,
                    is_abstract: m.is_abstract,
                },
            );
        }

        let record = PascalType::Record {
            fields,
            variant: None,
            object: Some(name.to_string()),
        };
        self.type_defs.insert(name.to_string(), record.clone());
        let info = ObjectInfo {
            parent: decl.parent.clone(),
            record,
            vmt_field,
            methods,
            slots,
        };

        // Declare this type's own methods so calls and VMTs can refer to
        // them before their bodies are compiled.
        for m in &decl.methods {
            if m.is_abstract {
                continue;
            }
            let method = &info.methods[&m.name];
            let fn_type = self.method_fn_type(method);
            let symbol = method.symbol.clone();
            if self.module.get_function(&symbol).is_none() {
                self.module.add_function(&symbol, fn_type, None);
            }
            let mut param_info = Vec::new();
            for group in &method.params {
                let ty = self.resolve_type(&group.ty);
                for pname in &group.names {
                    param_info.push((pname.clone(), group.mode, ty.clone()));
                }
            }
            self.proc_param_modes.insert(symbol.clone(), param_info);
            let ret = match method.kind {
                RoutineKind::Function => method.return_type.clone(),
                _ => None,
            };
            self.proc_return_types.insert(symbol, ret);
        }
        self.objects.insert(name.to_string(), info);
        Ok(())
    }

    /// The LLVM type of a method: `Self` (and the VMT for a constructor)
    /// first, then the declared parameters.
    pub(super) fn method_fn_type(&self, method: &MethodInfo) -> FunctionType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let mut params: Vec<BasicMetadataTypeEnum> = vec![ptr.into()];
        if method.kind == RoutineKind::Constructor {
            params.push(ptr.into());
        }
        for group in &method.params {
            let ty = self.resolve_type(&group.ty);
            for _ in &group.names {
                params.push(if group.mode == ParamMode::Var {
                    ptr.into()
                } else {
                    self.llvm_type_for(&ty).into()
                });
            }
        }
        match (&method.kind, &method.return_type) {
            (RoutineKind::Constructor, _) => ptr.fn_type(&params, false),
            (RoutineKind::Function, Some(rt)) => {
                let rt = self.resolve_type(rt);
                self.llvm_type_for(&rt).fn_type(&params, false)
            }
            _ => self.context.void_type().fn_type(&params, false),
        }
    }

    /// Emit every object type's VMT. Called once all object types are
    /// registered, so parent links and method functions all exist.
    pub(super) fn emit_vmts(&mut self) {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        let mut names: Vec<String> = self.objects.keys().cloned().collect();
        names.sort();
        // Cells holding each VMT's address, for parent links.
        for name in &names {
            let cell = self.module.add_global(ptr, None, &format!("vmtref.{name}"));
            cell.set_linkage(Linkage::Internal);
            cell.set_constant(true);
        }
        for name in &names {
            let info = self.objects[name].clone();
            let slot_values: Vec<_> = info
                .slots
                .iter()
                .map(|slot| {
                    let f = match slot {
                        Some(symbol) => self.module.get_function(symbol),
                        None => Some(self.abstract_error_fn()),
                    };
                    f.map(|f| f.as_global_value().as_pointer_value())
                        .unwrap_or_else(|| ptr.const_null())
                })
                .collect();
            let size = self
                .llvm_type_for(&info.record)
                .size_of()
                .unwrap_or_else(|| i64_ty.const_zero());
            let parent_link = info
                .parent
                .as_ref()
                .and_then(|p| self.module.get_global(&format!("vmtref.{p}")))
                .map(|g| g.as_pointer_value())
                .unwrap_or_else(|| ptr.const_null());
            let slots_ty = ptr.array_type(slot_values.len() as u32);
            let vmt_ty = self.context.struct_type(
                &[i64_ty.into(), i64_ty.into(), ptr.into(), slots_ty.into()],
                false,
            );
            let init = vmt_ty.const_named_struct(&[
                size.into(),
                size.const_neg().into(),
                parent_link.into(),
                ptr.const_array(&slot_values).into(),
            ]);
            let vmt = self.module.add_global(vmt_ty, None, &format!("vmt.{name}"));
            vmt.set_linkage(Linkage::Internal);
            vmt.set_constant(true);
            vmt.set_initializer(&init);
            let cell = self.module.get_global(&format!("vmtref.{name}")).unwrap();
            cell.set_initializer(&vmt.as_pointer_value());
        }
    }

    /// The VMT of an object type.
    pub(super) fn vmt_of(&self, name: &str) -> PointerValue<'ctx> {
        self.module
            .get_global(&format!("vmt.{name}"))
            .map(|g| g.as_pointer_value())
            .unwrap_or_else(|| self.context.ptr_type(AddressSpace::default()).const_null())
    }

    /// The function an abstract VMT slot points at: runtime error 211.
    pub(super) fn abstract_error_fn(&self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function("bruto_abstract_error") {
            return f;
        }
        let void_ty = self.context.void_type();
        let f = self
            .module
            .add_function("bruto_abstract_error", void_ty.fn_type(&[], false), None);
        f.set_linkage(Linkage::Internal);
        let builder = self.context.create_builder();
        let entry = self.context.append_basic_block(f, "entry");
        builder.position_at_end(entry);
        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let message = "Runtime error 211: abstract method called\n";
        let text = builder
            .build_global_string_ptr(message, "abstract_msg")
            .unwrap()
            .as_pointer_value();
        let write = self.module.get_function("write").unwrap_or_else(|| {
            self.module.add_function(
                "write",
                i64_ty.fn_type(&[i32_ty.into(), ptr.into(), i64_ty.into()], false),
                None,
            )
        });
        builder
            .build_call(
                write,
                &[
                    i32_ty.const_int(2, false).into(),
                    text.into(),
                    i64_ty.const_int(message.len() as u64, false).into(),
                ],
                "",
            )
            .unwrap();
        let exit = self.module.get_function("exit").unwrap_or_else(|| {
            self.module
                .add_function("exit", void_ty.fn_type(&[i32_ty.into()], false), None)
        });
        builder
            .build_call(exit, &[i32_ty.const_int(211, false).into()], "")
            .unwrap();
        builder.build_unreachable().unwrap();
        f
    }

    /// The object type a record type was lowered from, if any.
    pub(super) fn object_name_of(&self, ty: &PascalType) -> Option<String> {
        match self.resolve_type(ty) {
            PascalType::Record {
                object: Some(name), ..
            } => Some(name),
            _ => None,
        }
    }

    /// If `name` (not hidden by a variable) is a method of an object named
    /// by an enclosing `with`, or of the object whose method is being
    /// compiled, that object type and the instance to call it on.
    pub(super) fn implicit_method(&self, name: &str) -> Option<(String, PointerValue<'ctx>)> {
        if self.variables.contains_key(name) {
            return None;
        }
        let has = |object: &str| self.objects[object].methods.contains_key(name);
        if let Some((object, ptr)) = self.with_objects.iter().rev().find(|(o, _)| has(o)) {
            return Some((object.clone(), *ptr));
        }
        let object = self.current_object.clone()?;
        let ctx = self.method_context?;
        has(&object).then_some((object, ctx.self_ptr))
    }

    /// The object type `implicit_method` would call `name` on.
    pub(super) fn self_method(&self, name: &str) -> Option<String> {
        self.implicit_method(name).map(|(object, _)| object)
    }

    /// Call `method` on `receiver`.
    pub(super) fn call_method(
        &mut self,
        receiver: Receiver<'_, 'ctx>,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let Receiver {
            object,
            self_ptr,
            dispatch,
            vmt,
        } = receiver;
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let info = self.objects.get(object).cloned().ok_or_else(|| {
            CodeGenError::new(format!("'{object}' is not an object type"), Some(span))
        })?;
        let m = info.methods.get(method).cloned().ok_or_else(|| {
            CodeGenError::new(
                format!("'{}' has no method '{method}'", self.display(object)),
                Some(span),
            )
        })?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let mut call_args: Vec<BasicMetadataValueEnum> = vec![self_ptr.into()];
        if m.kind == RoutineKind::Constructor {
            call_args.push(vmt.unwrap_or_else(|| ptr.const_null()).into());
        }
        call_args.extend(self.compile_call_args(&m.symbol, args, span)?);
        let fn_type = self.method_fn_type(&m);

        let result = match (dispatch, m.slot, info.vmt_field) {
            (Dispatch::Virtual, Some(slot), Some(vmt_index)) => {
                let record_ty = self.llvm_type_for(&info.record);
                let vmt_addr = self
                    .builder
                    .build_struct_gep(record_ty, self_ptr, vmt_index as u32, "vmt_addr")
                    .map_err(err)?;
                let vmt = self
                    .builder
                    .build_load(ptr, vmt_addr, "vmt")
                    .map_err(err)?
                    .into_pointer_value();
                let slot_addr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            ptr,
                            vmt,
                            &[self
                                .context
                                .i64_type()
                                .const_int(VMT_HEADER_WORDS + slot as u64, false)],
                            "slot_addr",
                        )
                        .map_err(err)?
                };
                let target = self
                    .builder
                    .build_load(ptr, slot_addr, "method")
                    .map_err(err)?
                    .into_pointer_value();
                self.builder
                    .build_indirect_call(fn_type, target, &call_args, "vcall")
                    .map_err(err)?
            }
            _ => {
                if m.is_abstract {
                    return Err(CodeGenError::new(
                        format!("'{method}' is abstract and can't be called directly"),
                        Some(span),
                    ));
                }
                let f = self.module.get_function(&m.symbol).ok_or_else(|| {
                    CodeGenError::new(format!("method '{}' has no body", m.symbol), Some(span))
                })?;
                self.builder
                    .build_call(f, &call_args, "mcall")
                    .map_err(err)?
            }
        };
        let value = result.try_as_basic_value().basic();
        match (value, m.kind, &m.return_type) {
            (Some(v), RoutineKind::Function, Some(rt)) => {
                let rt = rt.clone();
                Ok(Some(self.widen_loaded(v, &rt, span)?))
            }
            (value, _, _) => Ok(value),
        }
    }

    /// `obj.Method(args)` / `p^.Method(args)`: the instance's address, its
    /// object type, then the call. Inside a method, `Self` works too.
    pub(super) fn compile_method_call(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let (self_ptr, ty) = self.resolve_array_base(object, span)?;
        let name = self.object_name_of(&ty).ok_or_else(|| {
            CodeGenError::new(
                format!("'{method}' is called on something that isn't an object"),
                Some(span),
            )
        })?;
        // `obj.Init(...)` on an instance passes that instance's type's VMT.
        let vmt = Some(self.vmt_of(&name));
        self.call_method(
            Receiver {
                object: &name,
                self_ptr,
                dispatch: Dispatch::Virtual,
                vmt,
            },
            method,
            args,
            span,
        )
    }

    /// `inherited Method(args)` inside a method: the parent type's
    /// implementation, called directly on `Self`. An inherited constructor
    /// gets a nil VMT so it doesn't overwrite the object's.
    pub(super) fn compile_inherited(
        &mut self,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let (object, ctx) = match (&self.current_object, self.method_context) {
            (Some(o), Some(c)) => (o.clone(), c),
            _ => {
                return Err(CodeGenError::new(
                    "'inherited' is only allowed inside a method",
                    Some(span),
                ));
            }
        };
        let parent = self.objects[&object].parent.clone().ok_or_else(|| {
            CodeGenError::new(
                format!("'{}' has no parent type", self.display(&object)),
                Some(span),
            )
        })?;
        self.call_method(
            Receiver {
                object: &parent,
                self_ptr: ctx.self_ptr,
                dispatch: Dispatch::Static,
                vmt: None,
            },
            method,
            args,
            span,
        )
    }

    /// `new(p, Init(args))` or `New(PType, Init(args))`: call the
    /// constructor with a nil `Self` so it allocates, and return the new
    /// object.
    pub(super) fn construct_new(
        &mut self,
        pointer_type: &PascalType,
        constructor: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let pointed = match self.resolve_type(pointer_type) {
            PascalType::Pointer(inner) => *inner,
            _ => {
                return Err(CodeGenError::new(
                    "new(p, Constructor) needs a pointer to an object",
                    Some(span),
                ));
            }
        };
        let name = self.object_name_of(&pointed).ok_or_else(|| {
            CodeGenError::new(
                "new(p, Constructor) needs a pointer to an object",
                Some(span),
            )
        })?;
        let null = self.context.ptr_type(AddressSpace::default()).const_null();
        let vmt = Some(self.vmt_of(&name));
        self.call_method(
            Receiver {
                object: &name,
                self_ptr: null,
                dispatch: Dispatch::Static,
                vmt,
            },
            constructor,
            args,
            span,
        )?
        .ok_or_else(|| {
            CodeGenError::new(format!("'{constructor}' is not a constructor"), Some(span))
        })
    }

    /// `TypeOf(T)` or `TypeOf(instance)`: the VMT address.
    pub(super) fn compile_typeof(
        &mut self,
        arg: &Expr,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        if let Expr::Var(name, _) = arg
            && self.lookup_var(name).is_none()
            && self.objects.contains_key(name)
        {
            return Ok(self.vmt_of(name).into());
        }
        let (self_ptr, ty) = self.resolve_array_base(arg, span)?;
        let name = self
            .object_name_of(&ty)
            .ok_or_else(|| CodeGenError::new("TypeOf needs an object", Some(span)))?;
        let info = self.objects[&name].clone();
        let Some(vmt_index) = info.vmt_field else {
            return Ok(self.vmt_of(&name).into());
        };
        let record_ty = self.llvm_type_for(&info.record);
        let addr = self
            .builder
            .build_struct_gep(record_ty, self_ptr, vmt_index as u32, "vmt_addr")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder
            .build_load(self.context.ptr_type(AddressSpace::default()), addr, "vmt")
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))
    }

    /// A constructor's prologue. `self_param` and `vmt_param` are its
    /// hidden arguments; returns the `Self` to use. With a nil `Self` the
    /// object is allocated (zeroed) from the VMT's size, or the
    /// constructor returns nil when there is no VMT either. With a VMT,
    /// it is stored in the object's VMT field.
    pub(super) fn constructor_prologue(
        &mut self,
        object: &str,
        self_param: PointerValue<'ctx>,
        vmt_param: PointerValue<'ctx>,
        span: Span,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let func = self.current_fn.unwrap();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        let start_bb = self.builder.get_insert_block().unwrap();
        let alloc_bb = self.context.append_basic_block(func, "ctor_alloc");
        let nil_bb = self.context.append_basic_block(func, "ctor_nil");
        let have_bb = self.context.append_basic_block(func, "ctor_self");

        let self_is_nil = self
            .builder
            .build_is_null(self_param, "self_nil")
            .map_err(err)?;
        let vmt_is_nil = self
            .builder
            .build_is_null(vmt_param, "vmt_nil")
            .map_err(err)?;
        let no_alloc = self
            .builder
            .build_and(self_is_nil, vmt_is_nil, "no_alloc")
            .map_err(err)?;
        let pick_bb = self.context.append_basic_block(func, "ctor_pick");
        self.builder
            .build_conditional_branch(no_alloc, nil_bb, pick_bb)
            .map_err(err)?;
        self.builder.position_at_end(pick_bb);
        self.builder
            .build_conditional_branch(self_is_nil, alloc_bb, have_bb)
            .map_err(err)?;

        // Nothing to construct into: return nil.
        self.builder.position_at_end(nil_bb);
        self.emit_prof_exit()?;
        self.builder
            .build_return(Some(&ptr.const_null()))
            .map_err(err)?;

        // Allocate from the VMT's size field.
        self.builder.position_at_end(alloc_bb);
        let size = self
            .builder
            .build_load(i64_ty, vmt_param, "obj_size")
            .map_err(err)?;
        let calloc = self.module.get_function("calloc").unwrap_or_else(|| {
            self.module.add_function(
                "calloc",
                ptr.fn_type(&[i64_ty.into(), i64_ty.into()], false),
                None,
            )
        });
        let fresh = self
            .builder
            .build_call(
                calloc,
                &[i64_ty.const_int(1, false).into(), size.into()],
                "obj",
            )
            .map_err(err)?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        self.builder
            .build_unconditional_branch(have_bb)
            .map_err(err)?;

        self.builder.position_at_end(have_bb);
        let phi = self.builder.build_phi(ptr, "self").map_err(err)?;
        phi.add_incoming(&[(&self_param, pick_bb), (&fresh, alloc_bb)]);
        let self_ptr = phi.as_basic_value().into_pointer_value();
        let _ = start_bb;

        // Store the VMT unless it is nil (an inherited constructor call).
        let info = self.objects[object].clone();
        if let Some(vmt_index) = info.vmt_field {
            let set_bb = self.context.append_basic_block(func, "ctor_set_vmt");
            let body_bb = self.context.append_basic_block(func, "ctor_body");
            self.builder
                .build_conditional_branch(vmt_is_nil, body_bb, set_bb)
                .map_err(err)?;
            self.builder.position_at_end(set_bb);
            let record_ty = self.llvm_type_for(&info.record);
            let addr = self
                .builder
                .build_struct_gep(record_ty, self_ptr, vmt_index as u32, "vmt_addr")
                .map_err(err)?;
            self.builder.build_store(addr, vmt_param).map_err(err)?;
            self.builder
                .build_unconditional_branch(body_bb)
                .map_err(err)?;
            self.builder.position_at_end(body_bb);
        }
        Ok(self_ptr)
    }

    /// Make the current object's fields usable by name inside a method
    /// body, unless a parameter or local of the same name hides them.
    pub(super) fn bind_object_fields(
        &mut self,
        object: &str,
        self_ptr: PointerValue<'ctx>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let info = self.objects[object].clone();
        let PascalType::Record { fields, .. } = &info.record else {
            return Ok(());
        };
        let record_ty = self.llvm_type_for(&info.record);
        for (i, (fname, fty)) in fields.iter().enumerate() {
            if fname == VMT_FIELD || self.variables.contains_key(fname) {
                continue;
            }
            let addr = self
                .builder
                .build_struct_gep(record_ty, self_ptr, i as u32, fname)
                .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            self.variables.insert(fname.clone(), addr);
            self.var_types.insert(fname.clone(), fty.clone());
        }
        Ok(())
    }
}
