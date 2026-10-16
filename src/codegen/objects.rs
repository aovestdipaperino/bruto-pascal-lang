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
/// Words before the first slot in a class's VMT: size, -size, parent
/// link, class name (a short string), the routine that releases the
/// strings an instance holds (or nil), the table of the interfaces the
/// class implements (or nil), and its type information (`ClassInfo`).
pub(super) const CLASS_VMT_HEADER_WORDS: u64 = 7;

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
    /// An Object Pascal class rather than a TP object.
    pub is_class: bool,
    /// A class's properties by (lower-case) name, own and inherited.
    pub properties: HashMap<String, PropertyDecl>,
    /// The name of a class's default array property.
    pub default_property: Option<String>,
    /// Class variables, own and inherited: name -> the global holding it.
    pub class_vars: HashMap<String, String>,
    /// An interface rather than a class or object. Its `slots` hold the
    /// keys of its methods, in method table order.
    pub is_interface: bool,
    /// The interfaces a class implements, with the class that declares
    /// each, own first and then inherited.
    pub interfaces: Vec<(String, String)>,
    /// An interface's GUID.
    pub guid: Option<String>,
    /// A CORBA interface: its references aren't counted.
    pub is_corba: bool,
    /// A class's method resolution clauses, own and inherited:
    /// (interface, its method) -> the class's method.
    pub resolutions: HashMap<(String, String), String>,
    /// The interfaces a class implements through a property
    /// (`implements`), own and inherited: interface -> (the property,
    /// the class declaring it).
    pub delegates: HashMap<String, (PropertyDecl, String)>,
    /// Compiled with `{$M+}`, or descending from a class that was: its
    /// default section is published.
    pub type_info: bool,
    /// A class's own published fields and properties, in order.
    pub own_published: Vec<String>,
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
    /// A class method: its hidden `Self` is the class (a VMT).
    pub is_class_method: bool,
    /// A class's destructor: it takes a flag, after `Self`, saying whether
    /// to free the instance when it is done (not for `inherited Destroy`).
    pub frees_instance: bool,
}

/// The method being compiled: its object type and `Self`.
#[derive(Debug, Clone, Copy)]
pub(super) struct MethodContext<'ctx> {
    /// The instance, or in a class method, the class.
    pub self_ptr: PointerValue<'ctx>,
    /// In a constructor's own body, where `Fail` can be used.
    pub in_constructor: bool,
    /// In a class method, where `Self` is a class.
    pub in_class_method: bool,
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
    /// A call on a class rather than an instance (`TFoo.Create`,
    /// `C.Create` with a class reference `C`): the class's VMT.
    pub class: Option<PointerValue<'ctx>>,
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
        // Every class descends from TObject.
        // (`BrutoTObject` where the program has a TObject of its own; the
        // first class declared is the root itself.)
        let root = ["tobject", "brutotobject"]
            .into_iter()
            .find(|r| self.objects.get(*r).is_some_and(|o| o.is_class));
        // `class(IFoo, ...)`: no parent but TObject, implementing IFoo.
        let decl_resolutions = decl.resolutions.clone();
        let mut listed = decl.interfaces.clone();
        let mut decl_parent = decl.parent.clone();
        if decl.is_class
            && let Some(p) = &decl.parent
            && self.interface_of(&PascalType::Named(p.clone())).is_some()
        {
            listed.insert(0, p.clone());
            decl_parent = None;
        }
        let parent_name = match (&decl_parent, root) {
            (None, Some(root)) if decl.is_class => Some(root.to_string()),
            // `class(TObject)` where TObject is the program's own object
            // type (Free Vision's): in FPC the unit declaring the class
            // sees System's TObject.
            (Some(p), Some(root))
                if decl.is_class
                    && p == "tobject"
                    && self.objects.get(p).is_some_and(|o| !o.is_class) =>
            {
                Some(root.to_string())
            }
            (other, _) => other.clone(),
        };
        let parent = match &parent_name {
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
        if let Some(p) = &parent
            && p.is_class != decl.is_class
        {
            return Err(CodeGenError::new(
                format!(
                    "'{}' can't descend from '{}': classes and objects don't mix",
                    self.display(name),
                    self.display(parent_name.as_deref().unwrap_or_default())
                ),
                Some(span),
            ));
        }
        let mut vmt_field = parent.as_ref().and_then(|p| p.vmt_field);
        if vmt_field.is_none() && (decl.is_class || decl.methods.iter().any(|m| m.is_virtual)) {
            vmt_field = Some(fields.len());
            fields.push((
                VMT_FIELD.to_string(),
                PascalType::Pointer(Box::new(PascalType::Untyped)),
            ));
        }
        for (fname, fty) in &decl.fields {
            // A pointer keeps its declared type, resolved where it is
            // used: it may point to an object declared further down
            // (`Owner: PGroup` in `TView`, before `TGroup`).
            let ty = match self.resolve_type(fty) {
                PascalType::Pointer(_) => fty.clone(),
                resolved => resolved,
            };
            fields.push((fname.clone(), ty));
        }

        let mut methods = parent
            .as_ref()
            .map(|p| p.methods.clone())
            .unwrap_or_default();
        let mut slots = parent.as_ref().map(|p| p.slots.clone()).unwrap_or_default();
        for m in &decl.methods {
            let symbol = format!("{name}.{}", m.name);
            let slot = if m.is_virtual {
                // An object's `virtual` method replaces its parent's; a
                // class's replaces it only with `override`.
                let inherited = methods.get(&m.name).and_then(|prev| prev.slot);
                if m.is_override && inherited.is_none() {
                    return Err(CodeGenError::new(
                        format!(
                            "'{}' overrides nothing: no virtual '{}' in an ancestor",
                            self.display(&m.name),
                            self.display(&m.name)
                        ),
                        Some(m.span),
                    ));
                }
                let inherited = inherited.filter(|_| !decl.is_class || m.is_override);
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
                    is_class_method: m.is_class_method,
                    frees_instance: decl.is_class && m.kind == RoutineKind::Destructor,
                },
            );
        }

        let mut properties = parent
            .as_ref()
            .map(|p| p.properties.clone())
            .unwrap_or_default();
        let mut default_property = parent.as_ref().and_then(|p| p.default_property.clone());
        for prop in &decl.properties {
            if prop.is_default {
                default_property = Some(prop.name.clone());
            }
            properties.insert(prop.name.clone(), prop.clone());
        }

        let mut class_vars = parent
            .as_ref()
            .map(|p| p.class_vars.clone())
            .unwrap_or_default();
        for (vname, vty) in &decl.class_vars {
            let global = format!("{name}.{vname}");
            self.declare_global(&global, vty, span);
            class_vars.insert(vname.clone(), global);
        }

        let mut interfaces = Vec::new();
        for i in &listed {
            let resolved = self
                .interface_of(&PascalType::Named(i.clone()))
                .filter(|r| self.objects.get(r).is_some_and(|o| o.is_interface));
            let Some(i) = resolved else {
                return Err(CodeGenError::new(
                    format!("'{}' is not an interface", self.display(i)),
                    Some(span),
                ));
            };
            interfaces.push((i, name.to_string()));
        }
        for inherited in parent.iter().flat_map(|p| p.interfaces.iter()) {
            if !interfaces.iter().any(|(i, _)| *i == inherited.0) {
                interfaces.push(inherited.clone());
            }
        }

        let mut resolutions = parent
            .as_ref()
            .map(|p| p.resolutions.clone())
            .unwrap_or_default();
        for (intf, method, target) in &decl_resolutions {
            let intf = self
                .interface_of(&PascalType::Named(intf.clone()))
                .unwrap_or_else(|| intf.clone());
            resolutions.insert((intf, method.clone()), target.clone());
        }
        let type_info = decl.type_info || parent.as_ref().is_some_and(|p| p.type_info);
        let own_published: Vec<String> = decl
            .fields
            .iter()
            .map(|(n, _)| n)
            .chain(decl.properties.iter().map(|p| &p.name))
            .filter(|n| {
                decl.published.contains(n) || (type_info && decl.default_section.contains(n))
            })
            .cloned()
            .collect();
        let mut delegates = parent
            .as_ref()
            .map(|p| p.delegates.clone())
            .unwrap_or_default();
        for prop in &decl.properties {
            for intf in &prop.implements {
                let intf = self
                    .interface_of(&PascalType::Named(intf.clone()))
                    .unwrap_or_else(|| intf.clone());
                delegates.insert(intf, (prop.clone(), name.to_string()));
            }
        }

        let record = PascalType::Record {
            fields,
            variant: None,
            object: Some(name.to_string()),
        };
        // A class's name means a reference to an instance (see
        // `class_instance_type`); the record is the instance itself.
        let record_name = if decl.is_class {
            super::classes::instance_type_name(name)
        } else {
            name.to_string()
        };
        self.type_defs.insert(record_name, record.clone());
        let info = ObjectInfo {
            parent: parent_name,
            record,
            vmt_field,
            methods,
            slots,
            is_class: decl.is_class,
            properties,
            default_property,
            class_vars,
            is_interface: false,
            interfaces,
            guid: None,
            is_corba: false,
            resolutions,
            delegates,
            type_info,
            own_published,
        };

        // Declare this type's own methods so calls and VMTs can refer to
        // them before their bodies are compiled.
        for m in &decl.methods {
            let method = &info.methods[&m.name];
            let fn_type = self.method_fn_type(method);
            let symbol = method.symbol.clone();
            // An abstract method has no function, but calls through the VMT
            // still need its parameter modes.
            if !m.is_abstract && self.module.get_function(&symbol).is_none() {
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
            let defaults = Self::param_defaults(&method.params);
            if defaults.iter().any(Option::is_some) {
                self.proc_param_defaults.insert(symbol.clone(), defaults);
            }
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
        if method.frees_instance {
            params.push(self.context.i64_type().into());
        }
        for group in &method.params {
            let ty = self.resolve_type(&group.ty);
            for _ in &group.names {
                params.push(if group.mode == ParamMode::Var {
                    ptr.into()
                } else {
                    self.abi_type_for(&ty).into()
                });
            }
        }
        match (&method.kind, &method.return_type) {
            (RoutineKind::Constructor, _) => ptr.fn_type(&params, false),
            (RoutineKind::Function, Some(rt)) => self.abi_type_for(rt).fn_type(&params, false),
            _ => self.context.void_type().fn_type(&params, false),
        }
    }

    /// Emit every object type's VMT. Called once all object types are
    /// registered, so parent links and method functions all exist.
    pub(super) fn emit_vmts(&mut self) -> Result<(), CodeGenError> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        let mut names: Vec<String> = self
            .objects
            .keys()
            .filter(|n| !self.class_aliases.contains_key(*n) && !self.objects[*n].is_interface)
            .cloned()
            .collect();
        names.sort();
        // Cells holding each VMT's address, for parent links, and the VMTs
        // themselves (filled below), which type information refers to.
        for name in &names {
            let cell = self.module.add_global(ptr, None, &format!("vmtref.{name}"));
            cell.set_linkage(Linkage::Internal);
            cell.set_constant(true);
            let info = &self.objects[name];
            let slots_ty = ptr.array_type(info.slots.len() as u32);
            let header: Vec<inkwell::types::BasicTypeEnum> = if info.is_class {
                vec![
                    i64_ty.into(),
                    i64_ty.into(),
                    ptr.into(),
                    ptr.into(),
                    ptr.into(),
                    ptr.into(),
                    ptr.into(),
                    slots_ty.into(),
                ]
            } else {
                vec![i64_ty.into(), i64_ty.into(), ptr.into(), slots_ty.into()]
            };
            let vmt_ty = self.context.struct_type(&header, false);
            let vmt = self.module.add_global(vmt_ty, None, &format!("vmt.{name}"));
            vmt.set_linkage(Linkage::Internal);
            vmt.set_constant(true);
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
            let (vmt_ty, init) = if info.is_class {
                let [class_name, cleanup] = self.class_header_words(name);
                let interfaces = self.interface_table(name)?;
                let class_ty = PascalType::Named(name.clone());
                let class_info = self
                    .type_info(&class_ty, Some(name))
                    .unwrap_or_else(|| ptr.const_null());
                let vmt_ty = self.context.struct_type(
                    &[
                        i64_ty.into(),
                        i64_ty.into(),
                        ptr.into(),
                        ptr.into(),
                        ptr.into(),
                        ptr.into(),
                        ptr.into(),
                        slots_ty.into(),
                    ],
                    false,
                );
                let init = vmt_ty.const_named_struct(&[
                    size.into(),
                    size.const_neg().into(),
                    parent_link.into(),
                    class_name.into(),
                    cleanup.into(),
                    interfaces.into(),
                    class_info.into(),
                    ptr.const_array(&slot_values).into(),
                ]);
                (vmt_ty, init)
            } else {
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
                (vmt_ty, init)
            };
            let vmt = self.module.get_global(&format!("vmt.{name}")).unwrap();
            let _ = vmt_ty;
            vmt.set_initializer(&init);
            let cell = self.module.get_global(&format!("vmtref.{name}")).unwrap();
            cell.set_initializer(&vmt.as_pointer_value());
        }
        Ok(())
    }

    /// The VMT of an object type.
    pub(super) fn vmt_of(&self, name: &str) -> PointerValue<'ctx> {
        let name = self.class_aliases.get(name).map_or(name, |n| n.as_str());
        self.module
            .get_global(&format!("vmt.{name}"))
            .map(|g| g.as_pointer_value())
            .unwrap_or_else(|| self.context.ptr_type(AddressSpace::default()).const_null())
    }

    /// The function an abstract VMT slot points at: runtime error 211.
    pub(super) fn abstract_error_fn(&self) -> FunctionValue<'ctx> {
        self.runtime_error_fn(211, "abstract method called")
    }

    /// A function that reports runtime error `code` and stops the
    /// program, as FPC does (or, with SysUtils, raises the matching
    /// exception). Its body is written at the end of compilation, once
    /// the program's globals exist (see `emit_runtime_error_stubs`).
    pub(super) fn runtime_error_fn(&self, code: u32, what: &str) -> FunctionValue<'ctx> {
        let name = if code == 211 {
            "bruto_abstract_error".to_string()
        } else {
            format!("bruto_runtime_error_{code}")
        };
        if let Some(f) = self.module.get_function(&name) {
            return f;
        }
        let void_ty = self.context.void_type();
        let f = self
            .module
            .add_function(&name, void_ty.fn_type(&[], false), None);
        f.set_linkage(Linkage::Internal);
        self.runtime_error_stubs
            .borrow_mut()
            .push((name, code, what.to_string()));
        f
    }

    /// Write the bodies of the functions `runtime_error_fn` made.
    pub(super) fn emit_runtime_error_stubs(&mut self) -> Result<(), CodeGenError> {
        let saved_block = self.builder.get_insert_block();
        let saved_fn = self.current_fn;
        let stubs = std::mem::take(&mut *self.runtime_error_stubs.borrow_mut());
        for (name, code, what) in stubs {
            let f = self.module.get_function(&name).unwrap();
            let entry = self.context.append_basic_block(f, "entry");
            self.builder.position_at_end(entry);
            self.builder.unset_current_debug_location();
            self.current_fn = Some(f);
            let i32_ty = self.context.i32_type();
            let i64_ty = self.context.i64_type();
            let ptr = self.context.ptr_type(AddressSpace::default());
            let void_ty = self.context.void_type();
            let _ = (i64_ty, ptr, void_ty, what);
            // FPC's report (or SysUtils' exception), at the caller.
            let ret_addr = inkwell::intrinsics::Intrinsic::find("llvm.returnaddress")
                .and_then(|i| i.get_declaration(&self.module, &[]))
                .unwrap();
            let address = self
                .builder
                .build_call(ret_addr, &[i32_ty.const_zero().into()], "caller")
                .unwrap()
                .try_as_basic_value()
                .basic()
                .unwrap();
            let report = self.module.get_function("bruto_runtime_error").unwrap();
            self.builder
                .build_call(
                    report,
                    &[
                        i32_ty.const_int(code as u64, false).into(),
                        address.into(),
                        self.context.bool_type().const_int(1, false).into(),
                    ],
                    "",
                )
                .unwrap();
            self.builder.build_unreachable().unwrap();
        }
        self.current_fn = saved_fn;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(())
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
        if self.bypass_user || self.variables.contains_key(name) {
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

    /// A method function calling itself with arguments: its name is also
    /// its result variable, which hides it from `implicit_method`.
    pub(super) fn recursive_method(&self, name: &str) -> Option<(String, PointerValue<'ctx>)> {
        // (An overload, `get$2`, is called by its plain name too.)
        let current = self.current_function.as_deref()?;
        if self.bypass_user || current.split('$').next() != Some(name) {
            return None;
        }
        let object = self.current_object.clone()?;
        let ctx = self.method_context?;
        self.objects[&object]
            .methods
            .contains_key(name)
            .then_some((object, ctx.self_ptr))
    }

    /// Whether `object.method` is `TAncestor.Init(...)`: a constructor
    /// called through a type name, on Self.
    pub(super) fn ancestor_constructor(&self, object: &Expr, method: &str) -> bool {
        matches!(object, Expr::Var(type_name, _)
            if self.lookup_var(type_name).is_none()
                && self
                    .objects
                    .get(type_name)
                    .and_then(|o| o.methods.get(method))
                    .is_some_and(|m| m.kind == RoutineKind::Constructor))
    }

    /// Whether `inherited method` here calls a constructor.
    pub(super) fn inherited_is_constructor(&self, method: &str) -> bool {
        self.current_object
            .as_ref()
            .and_then(|o| self.objects[o].parent.as_ref())
            .and_then(|p| self.objects.get(p))
            .and_then(|p| p.methods.get(method))
            .is_some_and(|m| m.kind == RoutineKind::Constructor)
    }

    /// The object type `implicit_method` would call `name` on.
    pub(super) fn self_method(&self, name: &str) -> Option<String> {
        self.implicit_method(name).map(|(object, _)| object)
    }

    /// The method `method(args)` means on `object`: `method` itself, or
    /// for an overloaded method (`method$2`...), the overload whose
    /// parameters fit `args` best.
    pub(super) fn method_overload(&self, object: &str, method: &str, args: &[Expr]) -> String {
        let Some(info) = self.objects.get(object) else {
            return method.to_string();
        };
        let prefix = format!("{method}$");
        let candidates: Vec<&String> = info
            .methods
            .keys()
            .filter(|k| *k == method || k.starts_with(&prefix))
            .collect();
        if candidates.len() < 2 {
            return method.to_string();
        }
        let best = candidates
            .iter()
            .filter_map(|k| {
                self.overload_score(&info.methods[*k].symbol, args)
                    .map(|n| (n, *k))
            })
            .max_by_key(|(n, k)| (*n, std::cmp::Reverse(k.len())))
            .map(|(_, k)| k.clone());
        if let Some(best) = best {
            return best;
        }
        // No overload's types fit for sure: the only one taking that many
        // arguments, if there is one.
        let by_count: Vec<&&String> = candidates
            .iter()
            .filter(|k| {
                let symbol = &info.methods[**k].symbol;
                self.proc_param_modes
                    .get(symbol)
                    .is_some_and(|p| p.len() == self.with_defaults(symbol, args).len())
            })
            .collect();
        match by_count.as_slice() {
            [only] => (**only).clone(),
            _ => method.to_string(),
        }
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
            class,
        } = receiver;
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let info = self.objects.get(object).cloned().ok_or_else(|| {
            CodeGenError::new(format!("'{object}' is not an object type"), Some(span))
        })?;
        let chosen = self.method_overload(object, method, args);
        let method = chosen.as_str();
        let m = info.methods.get(method).cloned().ok_or_else(|| {
            CodeGenError::new(
                format!("'{}' has no method '{method}'", self.display(object)),
                Some(span),
            )
        })?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        // Inside a class method, `Self` is the class.
        let class = class.or_else(|| {
            self.method_context
                .filter(|c| c.in_class_method && c.self_ptr == self_ptr)
                .map(|c| c.self_ptr)
        });
        // The hidden `Self`, and the VMT a virtual call goes through when
        // it isn't the instance's.
        let (hidden_self, dispatch_vmt) = if m.is_class_method {
            let vmt = match class {
                Some(c) => c,
                None => self.load_vmt(object, self_ptr, span)?,
            };
            (vmt, Some(vmt))
        } else if let Some(c) = class {
            if m.kind != RoutineKind::Constructor {
                return Err(CodeGenError::new(
                    format!(
                        "'{}.{}' needs an instance to call it on",
                        self.display(object),
                        self.display(method)
                    ),
                    Some(span),
                ));
            }
            // `TFoo.Create`: a new instance.
            (ptr.const_null(), Some(c))
        } else {
            (self_ptr, None)
        };
        let mut call_args: Vec<BasicMetadataValueEnum> = vec![hidden_self.into()];
        if m.kind == RoutineKind::Constructor {
            call_args.push(class.or(vmt).unwrap_or_else(|| ptr.const_null()).into());
        }
        if m.frees_instance {
            // Free the instance, unless this is `inherited Destroy`.
            let free = u64::from(dispatch == Dispatch::Virtual);
            call_args.push(self.context.i64_type().const_int(free, false).into());
        }
        let hidden = call_args.len();
        call_args.extend(self.compile_call_args(&m.symbol, args, span)?);
        let fn_type = self.method_fn_type(&m);
        // As in FPC, a method hides the routine of the same name: `Close(F)`
        // inside a window's method means its `Close`, and is an error.
        if call_args.len() != fn_type.count_param_types() as usize {
            return Err(CodeGenError::new(
                format!(
                    "'{}.{}' takes {} argument(s), found {}",
                    self.display(object),
                    self.display(method),
                    fn_type.count_param_types() as usize - hidden,
                    args.len()
                ),
                Some(span),
            ));
        }

        let result = match (dispatch, m.slot, info.vmt_field) {
            (Dispatch::Virtual, Some(slot), Some(vmt_index)) => {
                let vmt = match dispatch_vmt {
                    Some(v) => v,
                    None => {
                        let record_ty = self.llvm_type_for(&info.record);
                        let vmt_addr = self
                            .builder
                            .build_struct_gep(record_ty, self_ptr, vmt_index as u32, "vmt_addr")
                            .map_err(err)?;
                        self.builder
                            .build_load(ptr, vmt_addr, "vmt")
                            .map_err(err)?
                            .into_pointer_value()
                    }
                };
                let header = self.vmt_header_words(object);
                let slot_addr = unsafe {
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
        // A class instance just made gets AfterConstruction, as in FPC.
        if m.kind == RoutineKind::Constructor
            && class.is_some()
            && info.is_class
            && info.methods.contains_key("afterconstruction")
            && let Some(made) = value
        {
            let made = made.into_pointer_value();
            let func = self.current_fn.unwrap();
            let call_bb = self.context.append_basic_block(func, "after_construction");
            let done_bb = self.context.append_basic_block(func, "constructed");
            let null = self.builder.build_is_null(made, "made_nil").map_err(err)?;
            self.builder
                .build_conditional_branch(null, done_bb, call_bb)
                .map_err(err)?;
            self.builder.position_at_end(call_bb);
            self.call_method(
                Receiver {
                    object,
                    self_ptr: made,
                    dispatch: Dispatch::Virtual,
                    vmt: None,
                    class: None,
                },
                "afterconstruction",
                &[],
                span,
            )?;
            self.builder
                .build_unconditional_branch(done_bb)
                .map_err(err)?;
            self.builder.position_at_end(done_bb);
        }
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
        // Through an interface reference.
        if let Some(intf) = self.interface_of(&self.infer_declared_type(object)) {
            return self.compile_interface_call(&intf, object, method, args, span);
        }
        // `TAncestor.Method(...)` inside a method: that type's
        // implementation, called on Self without virtual dispatch.
        // `TFoo.Create(...)`, `TFoo.ClassName`: a call on the class.
        if let Expr::Var(type_name, _) = object
            && self.is_class_name(type_name)
            && self.objects[type_name.as_str()]
                .methods
                .get(&self.method_overload(type_name, method, args))
                .is_some_and(|m| m.kind == RoutineKind::Constructor || m.is_class_method)
        {
            let method = &self.method_overload(type_name, method, args);
            let class = Some(self.vmt_of(type_name));
            return self.call_method(
                Receiver {
                    object: type_name,
                    self_ptr: self.context.ptr_type(AddressSpace::default()).const_null(),
                    dispatch: Dispatch::Virtual,
                    vmt: None,
                    class,
                },
                method,
                args,
                span,
            );
        }
        if let Expr::Var(type_name, _) = object
            && self.lookup_var(type_name).is_none()
            && self.objects.contains_key(type_name)
        {
            let ctx = self.method_context.ok_or_else(|| {
                CodeGenError::new(
                    format!(
                        "'{}.{method}' needs an object to call it on",
                        self.display(type_name)
                    ),
                    Some(span),
                )
            })?;
            let receiver = Receiver {
                object: type_name,
                self_ptr: ctx.self_ptr,
                dispatch: Dispatch::Static,
                vmt: None,
                class: None,
            };
            return self.call_method(receiver, method, args, span);
        }
        // On a class reference or an instance of a class: the value.
        let declared = self.infer_declared_type(object);
        if let Some(name) = self.class_ref_of(&declared) {
            let class = self.compile_expr(object)?.into_pointer_value();
            return self.call_method(
                Receiver {
                    object: &name,
                    self_ptr: self.context.ptr_type(AddressSpace::default()).const_null(),
                    dispatch: Dispatch::Virtual,
                    vmt: None,
                    class: Some(class),
                },
                method,
                args,
                span,
            );
        }
        if let Some(name) = self.class_instance_of(&declared) {
            let self_ptr = self.compile_expr(object)?.into_pointer_value();
            return self.call_method(
                Receiver {
                    object: &name,
                    self_ptr,
                    dispatch: Dispatch::Virtual,
                    vmt: None,
                    class: None,
                },
                method,
                args,
                span,
            );
        }
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
                class: None,
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
                class: None,
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
                class: None,
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
