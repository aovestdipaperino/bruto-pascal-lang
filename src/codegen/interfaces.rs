//! COM interfaces, as FPC's default `{$interfaces com}` has them.
//!
//! An interface type's methods are numbered in declaration order after
//! its parent's, starting with IInterface's `QueryInterface`, `_AddRef`
//! and `_Release`. A reference to an interface is a pair `{ instance,
//! method table }`: the table ("itable") of a class for an interface
//! holds, slot by slot, the class's routine for each method, so a call
//! through the reference passes the instance as `Self` to the routine in
//! the method's slot. A class has one itable per interface it implements
//! (or inherits from an ancestor class). As in FPC, a class implements
//! only the interfaces it lists, not their ancestors.
//!
//! A class's VMT points at its interface table: a count, then (interface
//! descriptor, itable) pairs, which `as`, `is` and `Supports` search at run
//! time. A descriptor holds the interface's GUID and name.
//!
//! References are counted: storing one calls `_AddRef` through it, and the
//! one replaced, or held by a local going out of scope, gets `_Release`.
//! As in FPC, the reference a function returns (or `as` makes) is held in
//! a hidden slot of the calling routine until a later statement reuses the
//! slot or the routine returns (the main program's at its end).
//! Global interface variables are left alone at the end, as in FPC.

use super::objects::{MethodInfo, ObjectInfo};
use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue, StructValue,
};
use std::collections::HashMap;

/// The interface every other descends from.
const ROOT: &str = "iinterface";

impl<'ctx> CodeGen<'ctx> {
    /// Lower an interface type declaration: number its methods and
    /// register their signatures.
    pub(super) fn register_interface(
        &mut self,
        name: &str,
        decl: &ObjectType,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let parent_name = match &decl.parent {
            Some(p) => Some(
                self.class_aliases
                    .get(p)
                    .cloned()
                    .unwrap_or_else(|| p.clone()),
            ),
            None if !decl.is_corba
                && name != ROOT
                && self.objects.get(ROOT).is_some_and(|o| o.is_interface) =>
            {
                Some(ROOT.to_string())
            }
            None => None,
        };
        let parent = match &parent_name {
            Some(p) => Some(
                self.objects
                    .get(p)
                    .filter(|o| o.is_interface)
                    .cloned()
                    .ok_or_else(|| {
                        CodeGenError::new(
                            format!(
                                "'{}' is not an interface declared before '{}'",
                                self.display(p),
                                self.display(name)
                            ),
                            Some(span),
                        )
                    })?,
            ),
            None => None,
        };
        let mut methods: HashMap<String, MethodInfo> = parent
            .as_ref()
            .map(|p| p.methods.clone())
            .unwrap_or_default();
        let mut slots = parent.as_ref().map(|p| p.slots.clone()).unwrap_or_default();
        for m in &decl.methods {
            let symbol = format!("{name}.{}", m.name);
            slots.push(Some(m.name.clone()));
            let method = MethodInfo {
                symbol: symbol.clone(),
                kind: m.kind,
                params: m.params.clone(),
                return_type: m.return_type.clone(),
                slot: Some(slots.len() - 1),
                is_abstract: true,
                is_class_method: false,
                frees_instance: false,
            };
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
            methods.insert(m.name.clone(), method);
        }
        let mut properties = parent
            .as_ref()
            .map(|p| p.properties.clone())
            .unwrap_or_default();
        for prop in &decl.properties {
            properties.insert(prop.name.clone(), prop.clone());
        }
        if let Some(guid) = &decl.guid {
            self.declare_guid(name, guid, span);
        }
        self.objects.insert(
            name.to_string(),
            ObjectInfo {
                parent: parent_name,
                record: PascalType::Record {
                    fields: Vec::new(),
                    variant: None,
                    object: None,
                },
                vmt_field: None,
                methods,
                slots,
                is_class: false,
                properties,
                default_property: None,
                class_vars: HashMap::new(),
                is_interface: true,
                interfaces: Vec::new(),
                guid: decl.guid.clone(),
                is_corba: decl.is_corba,
                resolutions: HashMap::new(),
                delegates: HashMap::new(),
            },
        );
        Ok(())
    }

    /// Hold `value`, an interface reference already counted for us, in
    /// a hidden slot of the current routine (releasing the one held there
    /// before). As FPC's temps for calls' results, a `shared` slot that a
    /// statement used is free for the next statement once it ends; `as`
    /// gets a slot of its own.
    pub(super) fn hold_interface_temp(
        &mut self,
        value: BasicValueEnum<'ctx>,
        shared: bool,
        span: Span,
    ) -> Result<(), CodeGenError> {
        if !self.arc || self.in_runtime {
            return Ok(());
        }
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ty = self.method_pointer_type();
        let reuse = if shared {
            self.intf_pool.get(self.intf_temps_used).copied()
        } else {
            None
        };
        let slot = match reuse {
            Some(slot) => slot,
            None => {
                // A slot zeroed on entry, so the first release finds nil.
                let entry = self
                    .current_fn
                    .and_then(|f| f.get_first_basic_block())
                    .unwrap();
                let builder = self.context.create_builder();
                match entry.get_first_instruction() {
                    Some(first) => builder.position_before(&first),
                    None => builder.position_at_end(entry),
                }
                let slot = builder.build_alloca(ty, "intf_temp").map_err(err)?;
                builder.build_store(slot, ty.const_zero()).map_err(err)?;
                self.intf_temps.push(slot);
                if shared {
                    self.intf_pool.push(slot);
                }
                slot
            }
        };
        if shared {
            self.intf_temps_used += 1;
        }
        let intf = PascalType::Interface(String::new());
        self.arc_release_slot(slot, &intf, span)?;
        self.builder.build_store(slot, value).map_err(err)?;
        Ok(())
    }

    /// Release the current routine's hidden interface slots, newest
    /// first.
    pub(super) fn release_interface_temps(&mut self, span: Span) -> Result<(), CodeGenError> {
        let intf = PascalType::Interface(String::new());
        for slot in self.intf_temps.clone().into_iter().rev() {
            self.arc_release_slot(slot, &intf, span)?;
        }
        Ok(())
    }

    /// The name of the TGUID constant an interface's name means as a
    /// value (`QueryInterface(IFoo, Ref)`), if it has a GUID.
    pub(super) fn guid_constant(&self, intf: &str) -> Option<String> {
        let name = format!("{intf}.guid");
        self.global_vars.contains_key(&name).then_some(name)
    }

    /// Declare interface `intf`'s TGUID constant (when TGUID exists).
    fn declare_guid(&mut self, intf: &str, guid: &str, span: Span) {
        if !self.type_defs.contains_key("tguid") {
            return;
        }
        let name = format!("{intf}.guid");
        let ty = PascalType::Named("tguid".into());
        self.declare_global(&name, &ty, span);
        let bytes = parse_guid(guid);
        let llvm_ty = self
            .llvm_type_for(&self.resolve_type(&ty))
            .into_struct_type();
        let mut at = 0usize;
        let mut values = Vec::new();
        for field in llvm_ty.get_field_types() {
            match field {
                inkwell::types::BasicTypeEnum::IntType(it) => {
                    let width = (it.get_bit_width() / 8) as usize;
                    let mut v = 0u64;
                    for (k, b) in bytes[at..at + width].iter().enumerate() {
                        v |= u64::from(*b) << (8 * k);
                    }
                    values.push(it.const_int(v, false).into());
                    at += width;
                }
                inkwell::types::BasicTypeEnum::ArrayType(at_ty) => {
                    let n = at_ty.len() as usize;
                    let i8_ty = self.context.i8_type();
                    let elems: Vec<_> = bytes[at..at + n]
                        .iter()
                        .map(|b| i8_ty.const_int(u64::from(*b), false))
                        .collect();
                    values.push(i8_ty.const_array(&elems).into());
                    at += n;
                }
                _ => return,
            }
        }
        if let Some(g) = self.module.get_global(&format!("pascal.{name}")) {
            g.set_initializer(&llvm_ty.const_named_struct(&values));
            g.set_constant(true);
        }
    }

    /// The interface a value of type `ty` refers to.
    pub(super) fn interface_of(&self, ty: &PascalType) -> Option<String> {
        match self.resolve_type(ty) {
            PascalType::Interface(name) => {
                Some(self.class_aliases.get(&name).cloned().unwrap_or(name))
            }
            _ => None,
        }
    }

    /// Whether `name` (not hidden by a variable) is an interface type.
    pub(super) fn is_interface_name(&self, name: &str) -> bool {
        self.lookup_var(name).is_none() && self.objects.get(name).is_some_and(|o| o.is_interface)
    }

    /// `name` and its ancestors, nearest first.
    fn interface_chain(&self, name: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut cur = Some(name.to_string());
        while let Some(n) = cur {
            cur = self.objects.get(&n).and_then(|o| o.parent.clone());
            chain.push(n);
        }
        chain
    }

    /// The descriptor of interface `name`: `{ GUID: [16 x i8], Name: ^ShortString }`.
    pub(super) fn interface_descriptor(&self, name: &str) -> PointerValue<'ctx> {
        let global_name = format!("intfdesc.{name}");
        if let Some(g) = self.module.get_global(&global_name) {
            return g.as_pointer_value();
        }
        let guid = self.objects[name]
            .guid
            .as_deref()
            .map(parse_guid)
            .unwrap_or([0; 16]);
        let display = self.display(name);
        let mut bytes = vec![display.len().min(255) as u8];
        bytes.extend(display.bytes().take(255));
        let text = self.context.const_string(&bytes, false);
        let text_global =
            self.module
                .add_global(text.get_type(), None, &format!("intfname.{name}"));
        text_global.set_linkage(Linkage::Internal);
        text_global.set_constant(true);
        text_global.set_initializer(&text);
        let guid_value = self.context.const_string(&guid, false);
        let desc = self.context.const_struct(
            &[guid_value.into(), text_global.as_pointer_value().into()],
            false,
        );
        let g = self.module.add_global(desc.get_type(), None, &global_name);
        g.set_linkage(Linkage::Internal);
        g.set_constant(true);
        g.set_initializer(&desc);
        g.as_pointer_value()
    }

    /// The method table of class `class` for interface `intf`, which
    /// `declarer` (the class or an ancestor) lists as implemented.
    fn itable(
        &self,
        class: &str,
        declarer: &str,
        intf: &str,
        span: Span,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        let global_name = format!("itable.{class}.{intf}");
        if let Some(g) = self.module.get_global(&global_name) {
            return Ok(g.as_pointer_value());
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let class_info = &self.objects[class];
        let declarer_info = &self.objects[declarer];
        let mut entries = Vec::new();
        let chain = self.interface_chain(intf);
        for key in self.objects[intf].slots.iter().flatten() {
            // `function IFoo.Bar = MyBar;` names the method that implements it.
            let resolved = chain
                .iter()
                .find_map(|i| declarer_info.resolutions.get(&(i.clone(), key.clone())));
            let key = resolved.unwrap_or(key);
            let found = declarer_info.methods.get(key).ok_or_else(|| {
                CodeGenError::new(
                    format!(
                        "'{}' doesn't implement '{}.{}'",
                        self.display(declarer),
                        self.display(intf),
                        self.display(key)
                    ),
                    Some(span),
                )
            })?;
            // A virtual method: the class's own override.
            let f = match found.slot {
                Some(s) => match &class_info.slots[s] {
                    Some(symbol) => self.module.get_function(symbol),
                    None => Some(self.abstract_error_fn()),
                },
                None => self.module.get_function(&found.symbol),
            };
            entries.push(
                f.map(|f| f.as_global_value().as_pointer_value())
                    .unwrap_or_else(|| ptr.const_null()),
            );
        }
        let init = ptr.const_array(&entries);
        let g = self.module.add_global(init.get_type(), None, &global_name);
        g.set_linkage(Linkage::Internal);
        g.set_constant(true);
        g.set_initializer(&init);
        Ok(g.as_pointer_value())
    }

    /// The table of interfaces class `class` implements, for its VMT:
    /// `{ Count: i64, [Descriptor, ITable] x Count }`, or nil if none.
    pub(super) fn interface_table(
        &mut self,
        class: &str,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let interfaces = self.objects[class].interfaces.clone();
        if interfaces.is_empty() {
            return Ok(ptr.const_null());
        }
        // (descriptor, itable, getter): a delegated interface has a getter
        // giving the reference instead of an itable.
        let mut words = Vec::new();
        for (intf, declarer) in &interfaces {
            words.push(self.interface_descriptor(intf));
            if self.objects[class].delegates.contains_key(intf) {
                words.push(ptr.const_null());
                words.push(
                    self.delegate_getter(class, intf)
                        .as_global_value()
                        .as_pointer_value(),
                );
            } else {
                words.push(self.itable(class, declarer, intf, Span::default())?);
                words.push(ptr.const_null());
            }
        }
        let count = self
            .context
            .i64_type()
            .const_int(interfaces.len() as u64, false);
        let init = self
            .context
            .const_struct(&[count.into(), ptr.const_array(&words).into()], false);
        let g = self
            .module
            .add_global(init.get_type(), None, &format!("intftable.{class}"));
        g.set_linkage(Linkage::Internal);
        g.set_constant(true);
        g.set_initializer(&init);
        Ok(g.as_pointer_value())
    }

    /// The routine giving class `class`'s reference to `intf`, which a
    /// property implements: `(Self, Out)` stores the (uncounted)
    /// reference at `Out`.
    fn delegate_getter(&mut self, class: &str, intf: &str) -> FunctionValue<'ctx> {
        let name = format!("intfget.{class}.{intf}");
        if let Some(f) = self.module.get_function(&name) {
            return f;
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let f = self.module.add_function(
            &name,
            self.context
                .void_type()
                .fn_type(&[ptr.into(), ptr.into()], false),
            None,
        );
        f.set_linkage(Linkage::Internal);
        self.intf_getters
            .push((class.to_string(), intf.to_string()));
        f
    }

    /// Write the delegating getters' bodies: read the property, and make
    /// its value a reference to the interface.
    pub(super) fn emit_interface_getters(&mut self) -> Result<(), CodeGenError> {
        let saved_block = self.builder.get_insert_block();
        let saved_fn = self.current_fn;
        let saved_ctx = self.method_context;
        let mut done = 0;
        while done < self.intf_getters.len() {
            let (class, intf) = self.intf_getters[done].clone();
            done += 1;
            let span = Span::default();
            let err =
                |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
            let f = self
                .module
                .get_function(&format!("intfget.{class}.{intf}"))
                .unwrap();
            let entry = self.context.append_basic_block(f, "entry");
            self.builder.position_at_end(entry);
            self.builder.unset_current_debug_location();
            self.current_fn = Some(f);
            let self_ptr = f.get_nth_param(0).unwrap().into_pointer_value();
            let out = f.get_nth_param(1).unwrap().into_pointer_value();
            self.method_context = Some(super::objects::MethodContext {
                self_ptr,
                in_constructor: false,
                in_class_method: false,
            });
            let (prop, _) = self.objects[&class].delegates[&intf].clone();
            let accessor = prop.read.clone().ok_or_else(|| {
                CodeGenError::new(
                    format!("'{}' can't be read", self.display(&prop.name)),
                    Some(prop.span),
                )
            })?;
            let info = self.objects[&class].clone();
            let value = if info.methods.contains_key(&accessor) {
                self.call_method(
                    super::objects::Receiver {
                        object: &class,
                        self_ptr,
                        dispatch: super::objects::Dispatch::Virtual,
                        vmt: None,
                        class: None,
                    },
                    &accessor,
                    &[],
                    prop.span,
                )?
                .unwrap()
            } else {
                // A field, or a path of fields.
                let mut at = self_ptr;
                let mut ty = info.record.clone();
                for part in accessor.split('.') {
                    let (addr, field_ty) = self.field_address(at, &ty, part, prop.span)?;
                    at = addr;
                    ty = self.resolve_type(&field_ty);
                }
                let llvm_ty = self.llvm_type_for(&ty);
                self.builder
                    .build_load(llvm_ty, at, "delegate")
                    .map_err(err)?
            };
            let reference = match self.resolve_type(&prop.ty) {
                PascalType::Interface(_) => value,
                _ => match self.class_instance_of(&prop.ty) {
                    Some(inner) => {
                        let obj = value.into_pointer_value();
                        if self.objects[&inner].delegates.contains_key(&intf) {
                            let getter = self.delegate_getter(&inner, &intf);
                            self.builder
                                .build_call(getter, &[obj.into(), out.into()], "")
                                .map_err(err)?;
                            self.builder.build_return(None).map_err(err)?;
                            continue;
                        }
                        let declarer = self.objects[&inner]
                            .interfaces
                            .iter()
                            .find(|(i, _)| *i == intf)
                            .map(|(_, d)| d.clone())
                            .ok_or_else(|| {
                                CodeGenError::new(
                                    format!(
                                        "'{}' doesn't implement '{}'",
                                        self.display(&inner),
                                        self.display(&intf)
                                    ),
                                    Some(prop.span),
                                )
                            })?;
                        let itable = self.itable(&inner, &declarer, &intf, prop.span)?;
                        self.interface_value(obj, itable, prop.span)?.into()
                    }
                    None => {
                        return Err(CodeGenError::new(
                            format!(
                                "'{}' must be an interface or a class to implement '{}'",
                                self.display(&prop.name),
                                self.display(&intf)
                            ),
                            Some(prop.span),
                        ));
                    }
                },
            };
            self.builder.build_store(out, reference).map_err(err)?;
            self.builder.build_return(None).map_err(err)?;
        }
        self.current_fn = saved_fn;
        self.method_context = saved_ctx;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(())
    }

    /// Whether references to interface `name` are counted (not CORBA).
    pub(super) fn interface_counted(&self, name: &str) -> bool {
        !self.objects.get(name).is_some_and(|o| o.is_corba)
    }

    /// The interface reference `{ obj, itable }`.
    pub(super) fn interface_value(
        &self,
        obj: PointerValue<'ctx>,
        itable: PointerValue<'ctx>,
        span: Span,
    ) -> Result<StructValue<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ty = self.method_pointer_type();
        let v = self
            .builder
            .build_insert_value(ty.get_undef(), obj, 0, "intf_obj")
            .map_err(err)?;
        let v = self
            .builder
            .build_insert_value(v, itable, 1, "intf")
            .map_err(err)?;
        Ok(v.into_struct_value())
    }

    /// The instance and itable of an interface reference.
    pub(super) fn interface_parts(
        &self,
        value: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let s = value.into_struct_value();
        let obj = self
            .builder
            .build_extract_value(s, 0, "intf_obj")
            .map_err(err)?
            .into_pointer_value();
        let itable = self
            .builder
            .build_extract_value(s, 1, "itable")
            .map_err(err)?
            .into_pointer_value();
        Ok((obj, itable))
    }

    /// `expr` as a reference to interface `intf`: an instance of a class
    /// implementing it, a reference to it or a descendant, or nil.
    pub(super) fn compile_interface_value(
        &mut self,
        expr: &Expr,
        intf: &str,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let span = expr.span();
        if let Expr::Nil(_) = expr {
            return Ok(self.method_pointer_type().const_zero().into());
        }
        let declared = self.infer_declared_type(expr);
        if let Some(from) = self.interface_of(&declared) {
            if !self.interface_chain(&from).iter().any(|i| i == intf) {
                return Err(CodeGenError::new(
                    format!(
                        "incompatible types: got '{}', expected '{}'",
                        self.display(&from),
                        self.display(intf)
                    ),
                    Some(span),
                ));
            }
            return self.compile_expr(expr);
        }
        if let Some(class) = self.class_instance_of(&declared)
            && self.objects[&class].delegates.contains_key(intf)
        {
            // A property implements it: the reference it gives.
            let err =
                |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
            let obj = self.compile_expr(expr)?.into_pointer_value();
            let ty = self.method_pointer_type();
            let out = self.entry_alloca(ty, "delegated").map_err(err)?;
            self.builder
                .build_store(out, ty.const_zero())
                .map_err(err)?;
            let getter = self.delegate_getter(&class, intf);
            let func = self.current_fn.unwrap();
            let call_bb = self.context.append_basic_block(func, "delegate");
            let done_bb = self.context.append_basic_block(func, "delegated");
            let nil = self.builder.build_is_null(obj, "nil").map_err(err)?;
            self.builder
                .build_conditional_branch(nil, done_bb, call_bb)
                .map_err(err)?;
            self.builder.position_at_end(call_bb);
            self.builder
                .build_call(getter, &[obj.into(), out.into()], "")
                .map_err(err)?;
            self.builder
                .build_unconditional_branch(done_bb)
                .map_err(err)?;
            self.builder.position_at_end(done_bb);
            return Ok(self.builder.build_load(ty, out, "reference").map_err(err)?);
        }
        if let Some(class) = self.class_instance_of(&declared) {
            // As in FPC, only an interface the class (or an ancestor) lists.
            let found = self.objects[&class]
                .interfaces
                .iter()
                .find(|(i, _)| i == intf)
                .cloned();
            let Some((implemented, declarer)) = found else {
                return Err(CodeGenError::new(
                    format!(
                        "incompatible types: '{}' doesn't implement '{}'",
                        self.display(&class),
                        self.display(intf)
                    ),
                    Some(span),
                ));
            };
            let obj = self.compile_expr(expr)?.into_pointer_value();
            let itable = self.itable(&class, &declarer, &implemented, span)?;
            return Ok(self.interface_value(obj, itable, span)?.into());
        }
        self.compile_expr(expr)
    }

    /// `I.Method(args)` through interface reference `object`.
    pub(super) fn compile_interface_call(
        &mut self,
        intf: &str,
        object: &Expr,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let chosen = self.method_overload(intf, method, args);
        let m = self.objects[intf]
            .methods
            .get(&chosen)
            .cloned()
            .ok_or_else(|| {
                CodeGenError::new(
                    format!("'{}' has no method '{method}'", self.display(intf)),
                    Some(span),
                )
            })?;
        let value = self.compile_expr(object)?;
        let (obj, itable) = self.interface_parts(value, span)?;
        let mut call_args: Vec<BasicMetadataValueEnum> = vec![obj.into()];
        call_args.extend(self.compile_call_args(&m.symbol, args, span)?);
        let fn_type = self.method_fn_type(&m);
        if call_args.len() != fn_type.count_param_types() as usize {
            return Err(CodeGenError::new(
                format!(
                    "'{}.{}' takes {} argument(s), found {}",
                    self.display(intf),
                    self.display(method),
                    fn_type.count_param_types() as usize - 1,
                    args.len()
                ),
                Some(span),
            ));
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let slot_addr = unsafe {
            self.builder
                .build_in_bounds_gep(
                    ptr,
                    itable,
                    &[self
                        .context
                        .i64_type()
                        .const_int(m.slot.unwrap() as u64, false)],
                    "islot",
                )
                .map_err(err)?
        };
        let target = self
            .builder
            .build_load(ptr, slot_addr, "imethod")
            .map_err(err)?
            .into_pointer_value();
        let value = self
            .builder
            .build_indirect_call(fn_type, target, &call_args, "icall")
            .map_err(err)?
            .try_as_basic_value()
            .basic();
        match (value, &m.return_type) {
            (Some(v), Some(rt)) if m.kind == RoutineKind::Function => {
                let rt = rt.clone();
                Ok(Some(self.widen_loaded(v, &rt, span)?))
            }
            (value, _) => Ok(value),
        }
    }

    /// The instance `obj`'s reference to interface `intf`, looked up at
    /// run time (through a delegating property too): whether there is one,
    /// and the (uncounted) reference, nil if not.
    fn query_interface(
        &self,
        obj: PointerValue<'ctx>,
        intf: &str,
        span: Span,
    ) -> Result<(inkwell::values::IntValue<'ctx>, BasicValueEnum<'ctx>), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ty = self.method_pointer_type();
        let out = self.entry_alloca(ty, "intf_query").map_err(err)?;
        self.builder
            .build_store(out, ty.const_zero())
            .map_err(err)?;
        let desc = self.interface_descriptor(intf);
        self.call_ustr_runtime(
            "bruto_intf_query",
            &[obj.into(), desc.into(), out.into()],
            span,
        )?;
        let value = self.builder.build_load(ty, out, "intf_ref").map_err(err)?;
        let (found_obj, _) = self.interface_parts(value, span)?;
        let found = self
            .builder
            .build_is_not_null(found_obj, "supported")
            .map_err(err)?;
        Ok((found, value))
    }

    /// The instance an expression of class or interface type refers to.
    fn instance_of_expr(
        &mut self,
        expr: &Expr,
        op: &str,
        span: Span,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        let declared = self.infer_declared_type(expr);
        if self.interface_of(&declared).is_some() {
            let value = self.compile_expr(expr)?;
            return Ok(self.interface_parts(value, span)?.0);
        }
        if self.class_instance_of(&declared).is_some() {
            return Ok(self.compile_expr(expr)?.into_pointer_value());
        }
        Err(CodeGenError::new(
            format!("'{op}' needs a class instance or an interface on its left"),
            Some(span),
        ))
    }

    /// `X is IFoo`, `X as IFoo`, and `I as TFoo` / `I is TFoo`.
    pub(super) fn compile_interface_test(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &str,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let op_name = if op == BinOp::Is { "is" } else { "as" };
        let obj = self.instance_of_expr(left, op_name, span)?;
        if self.is_class_name(right) {
            // `I as TFoo`: the instance, if it is a TFoo.
            let vmt = self.vmt_of(right);
            let is = self
                .builder
                .build_call(self.class_is_fn(), &[obj.into(), vmt.into()], "is")
                .map_err(err)?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_int_value();
            if op == BinOp::Is {
                return Ok(is.into());
            }
            let nil = self.builder.build_is_null(obj, "as_nil").map_err(err)?;
            let fine = self.builder.build_or(is, nil, "as_fine").map_err(err)?;
            self.fail_unless(fine, span)?;
            return Ok(obj.into());
        }
        let (found, value) = self.query_interface(obj, right, span)?;
        if op == BinOp::Is {
            return Ok(found.into());
        }
        let nil = self.builder.build_is_null(obj, "as_nil").map_err(err)?;
        let fine = self.builder.build_or(found, nil, "as_fine").map_err(err)?;
        self.fail_unless(fine, span)?;
        // A counted reference, held as FPC holds its temps.
        if self.arc && !self.in_runtime && self.interface_counted(right) {
            let (ref_obj, itable) = self.interface_parts(value, span)?;
            self.call_ustr_runtime("bruto_intf_addref", &[ref_obj.into(), itable.into()], span)?;
            self.hold_interface_temp(value, false, span)?;
        }
        Ok(value)
    }

    /// `IFoo(X)`: as the implicit conversion where it applies, otherwise
    /// the instance's IFoo if its class implements it (nil if not).
    pub(super) fn interface_cast(
        &mut self,
        expr: &Expr,
        intf: &str,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let declared = self.infer_declared_type(expr);
        let static_ok = match (
            self.interface_of(&declared),
            self.class_instance_of(&declared),
        ) {
            (Some(from), _) => self.interface_chain(&from).iter().any(|i| i == intf),
            (_, Some(class)) => self.objects[&class]
                .interfaces
                .iter()
                .any(|(i, _)| i == intf),
            _ => true,
        };
        if static_ok {
            return self.compile_interface_value(expr, intf);
        }
        let obj = self.instance_of_expr(expr, "a type cast", span)?;
        Ok(self.query_interface(obj, intf, span)?.1)
    }

    /// Runtime error 219 (an invalid type cast) unless `ok`.
    fn fail_unless(
        &self,
        ok: inkwell::values::IntValue<'ctx>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let func = self.current_fn.unwrap();
        let bad_bb = self.context.append_basic_block(func, "as_bad");
        let ok_bb = self.context.append_basic_block(func, "as_ok");
        self.builder
            .build_conditional_branch(ok, ok_bb, bad_bb)
            .map_err(err)?;
        self.builder.position_at_end(bad_bb);
        let fail = self.runtime_error_fn(219, "invalid type cast");
        self.builder.build_call(fail, &[], "").map_err(err)?;
        self.builder.build_unreachable().map_err(err)?;
        self.builder.position_at_end(ok_bb);
        Ok(())
    }

    /// `Supports(X, IFoo)` and `Supports(X, IFoo, Ref)`: whether the
    /// instance `X` (or `X`'s) implements `IFoo`, storing the reference
    /// (or nil) into `Ref`. `None` if this isn't such a call.
    pub(super) fn compile_supports(
        &mut self,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let intf = match args {
            [_, Expr::Var(name, _)] | [_, Expr::Var(name, _), _]
                if self.is_interface_name(name) =>
            {
                self.class_aliases
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.clone())
            }
            _ => return Ok(None),
        };
        let obj = self.instance_of_expr(&args[0], "Supports", span)?;
        let (found, value) = self.query_interface(obj, &intf, span)?;
        if let Some(target) = args.get(2) {
            let (slot, ty) = self.resolve_array_base(target, span)?;
            self.store_value(slot, value, &ty, span)?;
        }
        Ok(Some(found.into()))
    }
}

/// The 16 bytes of a GUID written `{D1-D2-D3-D4a-D4b}`, laid out as
/// FPC's `TGUID` record.
fn parse_guid(text: &str) -> [u8; 16] {
    let hex: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let mut out = [0u8; 16];
    if hex.len() != 32 {
        return out;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap_or(0);
    // D1 (4 bytes), D2 and D3 (2 each) are little-endian; D4 is in order.
    for (dst, src) in [
        (0, 3),
        (1, 2),
        (2, 1),
        (3, 0),
        (4, 5),
        (5, 4),
        (6, 7),
        (7, 6),
    ] {
        out[dst] = byte(src);
    }
    for (i, b) in out.iter_mut().enumerate().skip(8) {
        *b = byte(i);
    }
    out
}
