//! Object Pascal classes, built on the TP object machinery in
//! `objects.rs`.
//!
//! A class lowers to the same record and VMT an object does, but its
//! instances always live on the heap and a variable of class type holds a
//! reference to one: the type name `TFoo` means a pointer to the record
//! `tfoo.inst` (see [`instance_type_name`]). Every class descends from
//! `TObject` (`rtl/objpas.pp`), so the VMT pointer is the first word of
//! every instance, as in FPC.
//!
//! A class's VMT header has two words more than an object's: the class
//! name (a short string) and the routine that releases the strings an
//! instance holds, which the destructor calls before freeing it.
//!
//! The language-level conveniences are rewritten into object operations
//! just before each statement is compiled ([`CodeGen::desugar_classes`]),
//! when the types of the variables in scope are known:
//! - `Obj.Field` becomes `Obj^.Field`;
//! - a property read becomes its field, or a call to its getter, and a
//!   property assignment becomes a field store or a call to its setter;
//! - `Obj[I]` uses the default array property;
//! - `TFoo.Create`, `C.Create` (`C` a class reference) and
//!   `Obj.ClassName` become method calls;
//! - a class name used as a value is its VMT (a `TClass`).

use super::objects::CLASS_VMT_HEADER_WORDS;
use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};

/// The name of the record type a class's instances are.
pub(super) fn instance_type_name(class: &str) -> String {
    format!("{class}.inst")
}

/// An indexed property access, found by `array_property`.
struct ArrayProperty {
    /// The instance.
    recv: Expr,
    class: String,
    prop: PropertyDecl,
    /// The property's index arguments.
    args: Vec<Expr>,
    /// Indices into the value the property gives.
    rest: Vec<Expr>,
}

/// Built-ins whose arguments name a type rather than give a value.
const TYPE_ARG_BUILTINS: &[&str] = &["sizeof", "typeof", "bitsizeof", "high", "low", "default"];

impl<'ctx> CodeGen<'ctx> {
    /// Whether `name` is a class type (not hidden by a variable).
    pub(super) fn is_class_name(&self, name: &str) -> bool {
        self.lookup_var(name).is_none() && self.objects.get(name).is_some_and(|o| o.is_class)
    }

    /// The class whose instances a value of type `ty` refers to.
    pub(super) fn class_instance_of(&self, ty: &PascalType) -> Option<String> {
        match self.resolve_type(ty) {
            PascalType::Pointer(inner) => match *inner {
                PascalType::Record {
                    object: Some(name), ..
                } if self.objects.get(&name).is_some_and(|o| o.is_class) => Some(name),
                _ => None,
            },
            _ => None,
        }
    }

    /// The class a class reference type (`class of T`) is bounded by.
    pub(super) fn class_ref_of(&self, ty: &PascalType) -> Option<String> {
        match self.resolve_type(ty) {
            PascalType::ClassOf(name) => Some(name),
            _ => None,
        }
    }

    /// The class `object.method(...)` is called on, when `object` is a
    /// class name, a class reference or an instance.
    pub(super) fn class_receiver(&self, object: &Expr) -> Option<String> {
        if let Expr::Var(name, _) = object
            && self.is_class_name(name)
        {
            return Some(name.clone());
        }
        let ty = self.infer_declared_type(object);
        self.class_instance_of(&ty)
            .or_else(|| self.class_ref_of(&ty))
    }

    /// Words before the first virtual slot in `object`'s VMT.
    pub(super) fn vmt_header_words(&self, object: &str) -> u64 {
        if self.objects.get(object).is_some_and(|o| o.is_class) {
            CLASS_VMT_HEADER_WORDS
        } else {
            3
        }
    }

    /// Load the VMT of the instance at `instance` (of type `object`).
    pub(super) fn load_vmt(
        &self,
        object: &str,
        instance: PointerValue<'ctx>,
        span: Span,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let info = &self.objects[object];
        let Some(index) = info.vmt_field else {
            return Ok(self.vmt_of(object));
        };
        let record_ty = self.llvm_type_for(&info.record);
        let addr = self
            .builder
            .build_struct_gep(record_ty, instance, index as u32, "vmt_addr")
            .map_err(err)?;
        Ok(self
            .builder
            .build_load(self.context.ptr_type(AddressSpace::default()), addr, "vmt")
            .map_err(err)?
            .into_pointer_value())
    }

    /// The constant parts of a class's VMT header after the parent link:
    /// its name, as a short string, and its cleanup routine (or nil).
    pub(super) fn class_header_words(&mut self, name: &str) -> [PointerValue<'ctx>; 2] {
        let ptr = self.context.ptr_type(AddressSpace::default());
        // The root class is TObject, even where it had to be renamed.
        let display = if name == "brutotobject" {
            "TObject".to_string()
        } else {
            self.display(name)
        };
        let mut bytes = vec![display.len().min(255) as u8];
        bytes.extend(display.bytes().take(255));
        let text = self.context.const_string(&bytes, false);
        let global = self
            .module
            .add_global(text.get_type(), None, &format!("classname.{name}"));
        global.set_linkage(Linkage::Internal);
        global.set_constant(true);
        global.set_initializer(&text);
        let record = self.objects[name].record.clone();
        let cleanup = if self.needs_arc(&record) {
            let f = self.module.add_function(
                &format!("{name}.__cleanup"),
                self.context.void_type().fn_type(&[ptr.into()], false),
                None,
            );
            f.set_linkage(Linkage::Internal);
            self.class_cleanups.push(name.to_string());
            f.as_global_value().as_pointer_value()
        } else {
            ptr.const_null()
        };
        [global.as_pointer_value(), cleanup]
    }

    /// Give each class's cleanup routine its body: release every string
    /// the instance holds. Called once the string runtime is declared.
    pub(super) fn emit_class_cleanups(&mut self) -> Result<(), CodeGenError> {
        let saved_block = self.builder.get_insert_block();
        let saved_fn = self.current_fn;
        for name in std::mem::take(&mut self.class_cleanups) {
            let f = self
                .module
                .get_function(&format!("{name}.__cleanup"))
                .unwrap();
            let entry = self.context.append_basic_block(f, "entry");
            self.builder.position_at_end(entry);
            self.builder.unset_current_debug_location();
            self.current_fn = Some(f);
            let instance = f.get_nth_param(0).unwrap().into_pointer_value();
            let record = self.objects[&name].record.clone();
            self.arc_release_slot(instance, &record, Span::default())?;
            self.builder
                .build_return(None)
                .map_err(|e| CodeGenError::new(e.to_string(), None))?;
        }
        self.current_fn = saved_fn;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(())
    }

    /// The end of a class's destructor: when `flag` is set (the
    /// destructor was called on the instance rather than as `inherited
    /// Destroy`), release the instance's strings and free it.
    pub(super) fn free_instance_if(
        &mut self,
        instance: PointerValue<'ctx>,
        flag: inkwell::values::IntValue<'ctx>,
        span: Span,
    ) -> Result<(), CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        let func = self.current_fn.unwrap();
        let free_bb = self.context.append_basic_block(func, "free_instance");
        let clean_bb = self.context.append_basic_block(func, "cleanup_instance");
        let release_bb = self.context.append_basic_block(func, "release_instance");
        let done_bb = self.context.append_basic_block(func, "instance_done");
        let set = self
            .builder
            .build_int_compare(inkwell::IntPredicate::NE, flag, i64_ty.const_zero(), "free")
            .map_err(err)?;
        let not_nil = self
            .builder
            .build_is_not_null(instance, "has_self")
            .map_err(err)?;
        let go = self.builder.build_and(set, not_nil, "go").map_err(err)?;
        self.builder
            .build_conditional_branch(go, free_bb, done_bb)
            .map_err(err)?;

        self.builder.position_at_end(free_bb);
        let vmt = self
            .builder
            .build_load(ptr, instance, "vmt")
            .map_err(err)?
            .into_pointer_value();
        let cleanup_addr = unsafe {
            self.builder
                .build_in_bounds_gep(ptr, vmt, &[i64_ty.const_int(4, false)], "cleanup_addr")
                .map_err(err)?
        };
        let cleanup = self
            .builder
            .build_load(ptr, cleanup_addr, "cleanup")
            .map_err(err)?
            .into_pointer_value();
        let has_cleanup = self
            .builder
            .build_is_not_null(cleanup, "has_cleanup")
            .map_err(err)?;
        self.builder
            .build_conditional_branch(has_cleanup, clean_bb, release_bb)
            .map_err(err)?;

        self.builder.position_at_end(clean_bb);
        let cleanup_ty = self.context.void_type().fn_type(&[ptr.into()], false);
        self.builder
            .build_indirect_call(cleanup_ty, cleanup, &[instance.into()], "")
            .map_err(err)?;
        self.builder
            .build_unconditional_branch(release_bb)
            .map_err(err)?;

        self.builder.position_at_end(release_bb);
        let free = self.module.get_function("free").unwrap_or_else(|| {
            self.module.add_function(
                "free",
                self.context.void_type().fn_type(&[ptr.into()], false),
                None,
            )
        });
        self.builder
            .build_call(free, &[instance.into()], "")
            .map_err(err)?;
        self.builder
            .build_unconditional_branch(done_bb)
            .map_err(err)?;
        self.builder.position_at_end(done_bb);
        Ok(())
    }

    /// `bruto_class_is(Obj, VMT)`: whether the instance `Obj` (or nil) is
    /// of the class with that VMT or a descendant. Walks the parent links.
    pub(super) fn class_is_fn(&self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function("bruto_class_is") {
            return f;
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let bool_ty = self.context.bool_type();
        let i64_ty = self.context.i64_type();
        let f = self.module.add_function(
            "bruto_class_is",
            bool_ty.fn_type(&[ptr.into(), ptr.into()], false),
            None,
        );
        f.set_linkage(Linkage::Internal);
        let b = self.context.create_builder();
        let entry = self.context.append_basic_block(f, "entry");
        let start = self.context.append_basic_block(f, "start");
        let check = self.context.append_basic_block(f, "check");
        let up = self.context.append_basic_block(f, "up");
        let next = self.context.append_basic_block(f, "next");
        let yes = self.context.append_basic_block(f, "yes");
        let no = self.context.append_basic_block(f, "no");
        let obj = f.get_nth_param(0).unwrap().into_pointer_value();
        let target = f.get_nth_param(1).unwrap().into_pointer_value();

        b.position_at_end(entry);
        let nil = b.build_is_null(obj, "nil").unwrap();
        b.build_conditional_branch(nil, no, start).unwrap();

        b.position_at_end(start);
        let first = b.build_load(ptr, obj, "vmt").unwrap().into_pointer_value();
        b.build_unconditional_branch(check).unwrap();

        b.position_at_end(check);
        let vmt = b.build_phi(ptr, "cur").unwrap();
        vmt.add_incoming(&[(&first, start)]);
        let cur = vmt.as_basic_value().into_pointer_value();
        let same = b
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                b.build_ptr_to_int(cur, i64_ty, "a").unwrap(),
                b.build_ptr_to_int(target, i64_ty, "b").unwrap(),
                "same",
            )
            .unwrap();
        b.build_conditional_branch(same, yes, up).unwrap();

        b.position_at_end(up);
        let link_addr = unsafe {
            b.build_in_bounds_gep(ptr, cur, &[i64_ty.const_int(2, false)], "link_addr")
                .unwrap()
        };
        let link = b
            .build_load(ptr, link_addr, "link")
            .unwrap()
            .into_pointer_value();
        let top = b.build_is_null(link, "top").unwrap();
        b.build_conditional_branch(top, no, next).unwrap();

        b.position_at_end(next);
        let parent = b
            .build_load(ptr, link, "parent")
            .unwrap()
            .into_pointer_value();
        vmt.add_incoming(&[(&parent, next)]);
        b.build_unconditional_branch(check).unwrap();

        b.position_at_end(yes);
        b.build_return(Some(&bool_ty.const_int(1, false))).unwrap();
        b.position_at_end(no);
        b.build_return(Some(&bool_ty.const_zero())).unwrap();
        f
    }

    /// `Obj is TFoo` and `Obj as TFoo`.
    pub(super) fn compile_class_test(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let class = match right {
            Expr::Var(name, _) if self.is_class_name(name) => name.clone(),
            _ => {
                return Err(CodeGenError::new(
                    format!(
                        "'{}' needs a class name on its right",
                        if op == BinOp::Is { "is" } else { "as" }
                    ),
                    Some(span),
                ));
            }
        };
        let value = self.compile_expr(left)?;
        if !value.is_pointer_value()
            || self
                .class_instance_of(&self.infer_declared_type(left))
                .is_none()
        {
            return Err(CodeGenError::new(
                format!(
                    "'{}' needs an instance of a class on its left",
                    if op == BinOp::Is { "is" } else { "as" }
                ),
                Some(span),
            ));
        }
        let obj = value.into_pointer_value();
        let vmt = self.vmt_of(&class);
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
        // `as`: nil stays nil; anything else must be of the class.
        let func = self.current_fn.unwrap();
        let bad_bb = self.context.append_basic_block(func, "as_bad");
        let ok_bb = self.context.append_basic_block(func, "as_ok");
        let nil = self.builder.build_is_null(obj, "as_nil").map_err(err)?;
        let fine = self.builder.build_or(is, nil, "as_fine").map_err(err)?;
        self.builder
            .build_conditional_branch(fine, ok_bb, bad_bb)
            .map_err(err)?;
        self.builder.position_at_end(bad_bb);
        let fail = self.runtime_error_fn(219, "invalid type cast");
        self.builder.build_call(fail, &[], "").map_err(err)?;
        self.builder.build_unreachable().map_err(err)?;
        self.builder.position_at_end(ok_bb);
        Ok(obj.into())
    }

    // ── the rewrite ──────────────────────────────────────

    /// `stmt` with class operations spelled as object operations (see the
    /// module documentation), or `None` when it has none. Nested
    /// statements are left alone: each is rewritten when it is compiled,
    /// in its own scope.
    pub(super) fn desugar_classes(
        &self,
        stmt: &Statement,
    ) -> Result<Option<Statement>, CodeGenError> {
        let d = |e: &Expr| self.dx(e);
        Ok(Some(match stmt {
            Statement::Assignment { target, expr, span } => {
                if self.lookup_var(target).is_none()
                    && let Some((class, prop, recv)) = self.implicit_property(target)
                {
                    return self
                        .property_write(recv, &class, &prop, vec![], d(expr)?, *span)
                        .map(Some);
                }
                Statement::Assignment {
                    target: target.clone(),
                    expr: d(expr)?,
                    span: *span,
                }
            }
            Statement::FieldAssignment {
                target,
                field,
                expr,
                span,
            } => {
                let recv = Expr::Var(target.clone(), *span);
                if let Some(class) = self.class_instance_of(&self.infer_declared_type(&recv)) {
                    return self
                        .member_write(recv, &class, field, d(expr)?, *span)
                        .map(Some);
                }
                if self.lookup_var(target).is_none() && self.implicit_property(target).is_some() {
                    let place = Expr::FieldAccess {
                        record: Box::new(recv),
                        field: field.clone(),
                        span: *span,
                    };
                    return self.place_write(&place, d(expr)?, *span).map(Some);
                }
                Statement::FieldAssignment {
                    target: target.clone(),
                    field: field.clone(),
                    expr: d(expr)?,
                    span: *span,
                }
            }
            Statement::ChainedAssignment {
                target,
                chain,
                expr,
                span,
            } => {
                let mut place = Expr::Var(target.clone(), *span);
                for access in chain {
                    place = match access {
                        LValueAccess::Field(f) => Expr::FieldAccess {
                            record: Box::new(place),
                            field: f.clone(),
                            span: *span,
                        },
                        LValueAccess::Index(i) => Expr::Index {
                            array: Box::new(place),
                            index: Box::new(i.clone()),
                            span: *span,
                        },
                        LValueAccess::Deref => Expr::Deref(Box::new(place), *span),
                    };
                }
                if !self.involves_class(&place) {
                    return Ok(None);
                }
                return self.place_write(&place, d(expr)?, *span).map(Some);
            }
            Statement::PlaceAssignment { target, expr, span } => {
                return self.place_write(target, d(expr)?, *span).map(Some);
            }
            Statement::IndexAssignment {
                target,
                index,
                expr,
                span,
            } => {
                let place = Expr::Index {
                    array: Box::new(Expr::Var(target.clone(), *span)),
                    index: Box::new(index.clone()),
                    span: *span,
                };
                if self.involves_class(&place) {
                    return self.place_write(&place, d(expr)?, *span).map(Some);
                }
                Statement::IndexAssignment {
                    target: target.clone(),
                    index: d(index)?,
                    expr: d(expr)?,
                    span: *span,
                }
            }
            Statement::MultiIndexAssignment {
                target,
                indices,
                expr,
                span,
            } => {
                let mut place = Expr::Var(target.clone(), *span);
                for index in indices {
                    place = Expr::Index {
                        array: Box::new(place),
                        index: Box::new(index.clone()),
                        span: *span,
                    };
                }
                if self.involves_class(&place) {
                    return self.place_write(&place, d(expr)?, *span).map(Some);
                }
                Statement::MultiIndexAssignment {
                    target: target.clone(),
                    indices: indices.iter().map(d).collect::<Result<_, _>>()?,
                    expr: d(expr)?,
                    span: *span,
                }
            }
            Statement::MethodCall {
                object,
                method,
                args,
                span,
            } if self.event_call(object, method, args, *span)?.is_some() => {
                Statement::IndirectCall {
                    call: self.event_call(object, method, args, *span)?.unwrap(),
                    span: *span,
                }
            }
            Statement::MethodCall {
                object,
                method,
                args,
                span,
            } => Statement::MethodCall {
                object: self.dx_receiver(object)?,
                method: method.clone(),
                args: args.iter().map(d).collect::<Result<_, _>>()?,
                span: *span,
            },
            Statement::ProcCall { name, args, span } => Statement::ProcCall {
                name: name.clone(),
                args: self.dx_args(name, args)?,
                span: *span,
            },
            Statement::Inherited { method, args, span } => Statement::Inherited {
                method: method.clone(),
                args: args.iter().map(d).collect::<Result<_, _>>()?,
                span: *span,
            },
            Statement::If {
                condition,
                then_branch,
                else_branch,
                span,
            } => Statement::If {
                condition: d(condition)?,
                then_branch: then_branch.clone(),
                else_branch: else_branch.clone(),
                span: *span,
            },
            Statement::While {
                condition,
                body,
                span,
            } => Statement::While {
                condition: d(condition)?,
                body: body.clone(),
                span: *span,
            },
            Statement::RepeatUntil {
                body,
                condition,
                span,
            } => Statement::RepeatUntil {
                body: body.clone(),
                condition: d(condition)?,
                span: *span,
            },
            Statement::For {
                var,
                from,
                to,
                downto,
                body,
                span,
            } => Statement::For {
                var: var.clone(),
                from: d(from)?,
                to: d(to)?,
                downto: *downto,
                body: body.clone(),
                span: *span,
            },
            Statement::ForIn {
                var,
                collection,
                body,
                span,
            } => Statement::ForIn {
                var: var.clone(),
                collection: d(collection)?,
                body: body.clone(),
                span: *span,
            },
            Statement::Case {
                expr,
                branches,
                else_branch,
                span,
            } => Statement::Case {
                expr: d(expr)?,
                branches: branches.clone(),
                else_branch: else_branch.clone(),
                span: *span,
            },
            Statement::WriteLn { args, span } | Statement::Write { args, span } => {
                let args = args
                    .iter()
                    .map(|a| {
                        Ok(WriteArg {
                            expr: d(&a.expr)?,
                            width: a.width.as_ref().map(d).transpose()?,
                            precision: a.precision.as_ref().map(d).transpose()?,
                        })
                    })
                    .collect::<Result<Vec<_>, CodeGenError>>()?;
                if matches!(stmt, Statement::WriteLn { .. }) {
                    Statement::WriteLn { args, span: *span }
                } else {
                    Statement::Write { args, span: *span }
                }
            }
            Statement::Exit {
                value: Some(v),
                span,
            } => Statement::Exit {
                value: Some(d(v)?),
                span: *span,
            },
            Statement::With { target, body, span } => {
                let target = d(target)?;
                let target = if self
                    .class_instance_of(&self.infer_declared_type(&target))
                    .is_some()
                {
                    Expr::Deref(Box::new(target), *span)
                } else {
                    target
                };
                Statement::With {
                    target,
                    body: body.clone(),
                    span: *span,
                }
            }
            Statement::DerefAssignment { target, expr, span } => Statement::DerefAssignment {
                target: target.clone(),
                expr: d(expr)?,
                span: *span,
            },
            Statement::IndirectCall { call, span } => Statement::IndirectCall {
                call: d(call)?,
                span: *span,
            },
            Statement::Raise {
                exception: Some(e),
                span,
            } => Statement::Raise {
                exception: Some(d(e)?),
                span: *span,
            },
            _ => return Ok(None),
        }))
    }

    /// Whether the place `expr` goes through a class instance or a
    /// property, so it needs rewriting.
    fn involves_class(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Var(name, _) => {
                self.lookup_var(name).is_none() && self.implicit_property(name).is_some()
            }
            Expr::FieldAccess { record, .. } | Expr::Index { array: record, .. } => {
                self.involves_class(record)
                    || self
                        .class_instance_of(&self.infer_declared_type(record))
                        .is_some()
            }
            Expr::Deref(inner, _) => self.involves_class(inner),
            _ => false,
        }
    }

    /// The property `name` reachable by its bare name: of an instance
    /// named by an enclosing `with`, or of the class whose method is being
    /// compiled. The class, the property and the instance to use.
    fn implicit_property(&self, name: &str) -> Option<(String, PropertyDecl, Expr)> {
        for (class, var) in self.with_classes.iter().rev() {
            if let Some(prop) = self.objects[class].properties.get(name) {
                let recv = Expr::Var(var.clone(), Span::default());
                return Some((class.clone(), prop.clone(), recv));
            }
        }
        let class = self.current_object.as_ref()?;
        let info = self.objects.get(class)?;
        if !info.is_class || self.method_context?.in_class_method {
            return None;
        }
        let prop = info.properties.get(name)?;
        let recv = Expr::Var("self".into(), Span::default());
        Some((class.clone(), prop.clone(), recv))
    }

    fn default_property(&self, class: &str) -> Option<PropertyDecl> {
        let info = self.objects.get(class)?;
        info.properties
            .get(info.default_property.as_ref()?)
            .cloned()
    }

    /// A method call's receiver: a class name stays as it is.
    fn dx_receiver(&self, object: &Expr) -> Result<Expr, CodeGenError> {
        match object {
            Expr::Var(name, _) if self.is_class_name(name) => Ok(object.clone()),
            _ => self.dx(object),
        }
    }

    fn dx_args(&self, routine: &str, args: &[Expr]) -> Result<Vec<Expr>, CodeGenError> {
        if TYPE_ARG_BUILTINS.contains(&routine) {
            return Ok(args.to_vec());
        }
        args.iter().map(|a| self.dx(a)).collect()
    }

    /// Rewrite the class operations in an expression.
    fn dx(&self, expr: &Expr) -> Result<Expr, CodeGenError> {
        let d = |e: &Expr| self.dx(e);
        let b = |e: &Expr| -> Result<Box<Expr>, CodeGenError> { Ok(Box::new(self.dx(e)?)) };
        Ok(match expr {
            Expr::Var(name, span) if self.lookup_var(name).is_none() => {
                // A class name as a value: its VMT, a class reference.
                if self.is_class_name(name) {
                    return Ok(Expr::Call {
                        name: "typeof".into(),
                        args: vec![expr.clone()],
                        span: *span,
                    });
                }
                if let Some((class, prop, recv)) = self.implicit_property(name) {
                    return self.property_read(recv, &class, &prop, vec![], *span);
                }
                expr.clone()
            }
            Expr::FieldAccess {
                record,
                field,
                span,
            } => {
                // `TFoo.Create`, `TFoo.ClassName`: a call on the class.
                if let Expr::Var(name, _) = record.as_ref()
                    && self.is_class_name(name)
                {
                    return Ok(Expr::MethodCall {
                        object: record.clone(),
                        method: field.clone(),
                        args: Vec::new(),
                        span: *span,
                    });
                }
                let rec = self.dx(record)?;
                let ty = self.infer_declared_type(&rec);
                if let Some(class) = self.class_instance_of(&ty) {
                    return self.member_read(rec, &class, field, *span);
                }
                if self.class_ref_of(&ty).is_some() {
                    return Ok(Expr::MethodCall {
                        object: Box::new(rec),
                        method: field.clone(),
                        args: Vec::new(),
                        span: *span,
                    });
                }
                Expr::FieldAccess {
                    record: Box::new(rec),
                    field: field.clone(),
                    span: *span,
                }
            }
            Expr::Index { array, index, span } => {
                if let Some(access) = self.array_property(expr)? {
                    let ArrayProperty {
                        recv,
                        class,
                        prop,
                        args,
                        rest,
                    } = access;
                    let mut value = self.property_read(recv, &class, &prop, args, *span)?;
                    for index in rest {
                        value = Expr::Index {
                            array: Box::new(value),
                            index: Box::new(index),
                            span: *span,
                        };
                    }
                    return Ok(value);
                }
                Expr::Index {
                    array: b(array)?,
                    index: b(index)?,
                    span: *span,
                }
            }
            // `Obj.OnClick(Self)`: a call through a method pointer field or
            // property.
            Expr::MethodCall {
                object,
                method,
                args,
                span,
            } if self.event_call(object, method, args, *span)?.is_some() => {
                self.event_call(object, method, args, *span)?.unwrap()
            }
            // `EFoo.CreateFmt(Fmt, [Args])`: `Create(Format(Fmt, [Args]))`
            // (Format is built in, so it needs its values written out).
            Expr::MethodCall {
                object,
                method,
                args,
                span,
            } if method == "createfmt"
                && args.len() == 2
                && self
                    .class_receiver(object)
                    .is_some_and(|c| !self.objects[&c].methods.contains_key("createfmt")) =>
            {
                Expr::MethodCall {
                    object: Box::new(self.dx_receiver(object)?),
                    method: "create".into(),
                    args: vec![Expr::Call {
                        name: "format".into(),
                        args: args.iter().map(d).collect::<Result<_, _>>()?,
                        span: *span,
                    }],
                    span: *span,
                }
            }
            Expr::MethodCall {
                object,
                method,
                args,
                span,
            } => Expr::MethodCall {
                object: Box::new(self.dx_receiver(object)?),
                method: method.clone(),
                args: args.iter().map(d).collect::<Result<_, _>>()?,
                span: *span,
            },
            Expr::Call { name, args, span } => Expr::Call {
                name: name.clone(),
                args: self.dx_args(name, args)?,
                span: *span,
            },
            Expr::BinOp {
                op: op @ (BinOp::Is | BinOp::As),
                left,
                right,
                span,
            } => Expr::BinOp {
                op: *op,
                left: b(left)?,
                right: right.clone(),
                span: *span,
            },
            Expr::BinOp {
                op,
                left,
                right,
                span,
            } => Expr::BinOp {
                op: *op,
                left: b(left)?,
                right: b(right)?,
                span: *span,
            },
            Expr::UnaryOp { op, operand, span } => Expr::UnaryOp {
                op: *op,
                operand: b(operand)?,
                span: *span,
            },
            Expr::Deref(inner, span) => Expr::Deref(b(inner)?, *span),
            // `@TFoo.Method` is the method's code.
            Expr::AddrOf(inner, span) => match inner.as_ref() {
                Expr::FieldAccess { record, .. } if matches!(record.as_ref(), Expr::Var(n, _) if self.is_class_name(n)) => {
                    expr.clone()
                }
                _ => Expr::AddrOf(b(inner)?, *span),
            },
            Expr::IndirectCall { callee, args, span } => Expr::IndirectCall {
                callee: b(callee)?,
                args: args.iter().map(d).collect::<Result<_, _>>()?,
                span: *span,
            },
            Expr::Inherited { method, args, span } => Expr::Inherited {
                method: method.clone(),
                args: args.iter().map(d).collect::<Result<_, _>>()?,
                span: *span,
            },
            Expr::SetConstructor { elements, span } => Expr::SetConstructor {
                elements: elements
                    .iter()
                    .map(|e| {
                        Ok(match e {
                            SetElement::Single(x) => SetElement::Single(d(x)?),
                            SetElement::Range(lo, hi) => SetElement::Range(d(lo)?, d(hi)?),
                        })
                    })
                    .collect::<Result<_, CodeGenError>>()?,
                span: *span,
            },
            _ => expr.clone(),
        })
    }

    /// What `expr` reaches if it indexes an array property:
    /// `Obj.Cells[I, J]`, `Cells[I, J]` inside a method or a `with`, or
    /// `Obj[I]` (the default property). `A[I, J]` arrives as
    /// `Index(Index(A, I), J)`; indices beyond the property's go to the
    /// value it gives.
    fn array_property(&self, expr: &Expr) -> Result<Option<ArrayProperty>, CodeGenError> {
        let mut indices = Vec::new();
        let mut base = expr;
        while let Expr::Index { array, index, .. } = base {
            indices.push(index.as_ref());
            base = array;
        }
        indices.reverse();
        let with_params = |p: &PropertyDecl| !p.params.is_empty();
        let mut found = None;
        if let Expr::FieldAccess { record, field, .. } = base {
            let rec = self.dx(record)?;
            if let Some(class) = self.class_instance_of(&self.infer_declared_type(&rec))
                && let Some(prop) = self.objects[&class].properties.get(field)
                && with_params(prop)
            {
                found = Some((rec, class, prop.clone()));
            }
        }
        if found.is_none()
            && let Expr::Var(name, _) = base
            && self.lookup_var(name).is_none()
            && let Some((class, prop, recv)) = self.implicit_property(name)
            && with_params(&prop)
        {
            found = Some((recv, class, prop));
        }
        if found.is_none() {
            let value = self.dx(base)?;
            if let Some(class) = self.class_instance_of(&self.infer_declared_type(&value))
                && let Some(prop) = self.default_property(&class)
            {
                found = Some((value, class, prop));
            }
        }
        let Some((recv, class, prop)) = found else {
            return Ok(None);
        };
        let count: usize = prop.params.iter().map(|g| g.names.len()).sum();
        if indices.len() < count {
            return Err(CodeGenError::new(
                format!(
                    "property '{}' takes {count} index(es), found {}",
                    self.display(&prop.name),
                    indices.len()
                ),
                Some(expr.span()),
            ));
        }
        let mut dx = indices.into_iter().map(|i| self.dx(i));
        let args = dx.by_ref().take(count).collect::<Result<_, _>>()?;
        let rest = dx.collect::<Result<_, _>>()?;
        Ok(Some(ArrayProperty {
            recv,
            class,
            prop,
            args,
            rest,
        }))
    }

    /// `Obj.Name(args)` where `Name` is a field or property of `Obj`'s
    /// class holding a method pointer: a call through it.
    fn event_call(
        &self,
        object: &Expr,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<Expr>, CodeGenError> {
        if matches!(object, Expr::Var(n, _) if self.is_class_name(n)) {
            return Ok(None);
        }
        let Some(class) = self.class_instance_of(&self.infer_declared_type(object)) else {
            return Ok(None);
        };
        let info = &self.objects[&class];
        if info.methods.contains_key(name) {
            return Ok(None);
        }
        let ty = match &info.record {
            PascalType::Record { fields, .. } => fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| t.clone()),
            _ => None,
        }
        .or_else(|| info.properties.get(name).map(|p| p.ty.clone()));
        if !ty.is_some_and(|t| Self::is_method_pointer(&self.resolve_type(&t))) {
            return Ok(None);
        }
        let rec = self.dx(object)?;
        let callee = self.member_read(rec, &class, name, span)?;
        Ok(Some(Expr::IndirectCall {
            callee: Box::new(callee),
            args: args.iter().map(|a| self.dx(a)).collect::<Result<_, _>>()?,
            span,
        }))
    }

    /// `rec.field` where `rec` refers to an instance of `class`.
    fn member_read(
        &self,
        rec: Expr,
        class: &str,
        field: &str,
        span: Span,
    ) -> Result<Expr, CodeGenError> {
        let info = &self.objects[class];
        if Self::record_has_field(&info.record, field) {
            return Ok(Expr::FieldAccess {
                record: Box::new(Expr::Deref(Box::new(rec), span)),
                field: field.to_string(),
                span,
            });
        }
        if let Some(prop) = info.properties.get(field) {
            let prop = prop.clone();
            return self.property_read(rec, class, &prop, vec![], span);
        }
        if info.methods.contains_key(field) {
            return Ok(Expr::MethodCall {
                object: Box::new(rec),
                method: field.to_string(),
                args: Vec::new(),
                span,
            });
        }
        Err(CodeGenError::new(
            format!(
                "'{}' has no field, property or method '{}'",
                self.display(class),
                self.display(field)
            ),
            Some(span),
        ))
    }

    fn record_has_field(record: &PascalType, field: &str) -> bool {
        matches!(record, PascalType::Record { fields, .. } if fields.iter().any(|(n, _)| n == field))
    }

    /// Reading property `prop` of the instance `recv` refers to.
    fn property_read(
        &self,
        recv: Expr,
        class: &str,
        prop: &PropertyDecl,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<Expr, CodeGenError> {
        let Some(read) = &prop.read else {
            return Err(CodeGenError::new(
                format!("property '{}' can't be read", self.display(&prop.name)),
                Some(span),
            ));
        };
        let info = &self.objects[class];
        let first = read.split('.').next().unwrap_or_default();
        if Self::record_has_field(&info.record, first) {
            return Ok(Self::field_path(recv, read, span));
        }
        let mut call_args = Vec::new();
        if let Some(i) = prop.index {
            call_args.push(Expr::IntLit(i, span));
        }
        call_args.extend(args);
        Ok(Expr::MethodCall {
            object: Box::new(recv),
            method: read.clone(),
            args: call_args,
            span,
        })
    }

    /// `recv^.a.b` for the accessor path `a.b`.
    fn field_path(recv: Expr, path: &str, span: Span) -> Expr {
        let mut place = Expr::Deref(Box::new(recv), span);
        for part in path.split('.') {
            place = Expr::FieldAccess {
                record: Box::new(place),
                field: part.to_string(),
                span,
            };
        }
        place
    }

    /// Assigning `value` to property `prop` of the instance `recv` refers
    /// to.
    fn property_write(
        &self,
        recv: Expr,
        class: &str,
        prop: &PropertyDecl,
        args: Vec<Expr>,
        value: Expr,
        span: Span,
    ) -> Result<Statement, CodeGenError> {
        let Some(write) = &prop.write else {
            return Err(CodeGenError::new(
                format!("property '{}' is read-only", self.display(&prop.name)),
                Some(span),
            ));
        };
        let info = &self.objects[class];
        let first = write.split('.').next().unwrap_or_default();
        if Self::record_has_field(&info.record, first) {
            return Ok(Statement::PlaceAssignment {
                target: Self::field_path(recv, write, span),
                expr: value,
                span,
            });
        }
        let mut call_args = Vec::new();
        if let Some(i) = prop.index {
            call_args.push(Expr::IntLit(i, span));
        }
        call_args.extend(args);
        call_args.push(value);
        Ok(Statement::MethodCall {
            object: recv,
            method: write.clone(),
            args: call_args,
            span,
        })
    }

    /// `rec.field := value` where `rec` refers to an instance of `class`.
    fn member_write(
        &self,
        rec: Expr,
        class: &str,
        field: &str,
        value: Expr,
        span: Span,
    ) -> Result<Statement, CodeGenError> {
        let info = &self.objects[class];
        if Self::record_has_field(&info.record, field) {
            return Ok(Statement::PlaceAssignment {
                target: Expr::FieldAccess {
                    record: Box::new(Expr::Deref(Box::new(rec), span)),
                    field: field.to_string(),
                    span,
                },
                expr: value,
                span,
            });
        }
        if let Some(prop) = info.properties.get(field) {
            let prop = prop.clone();
            return self.property_write(rec, class, &prop, vec![], value, span);
        }
        Err(CodeGenError::new(
            format!(
                "'{}' has no field or property '{}' to assign",
                self.display(class),
                self.display(field)
            ),
            Some(span),
        ))
    }

    /// Assigning `value` to the place `target`.
    fn place_write(
        &self,
        target: &Expr,
        value: Expr,
        span: Span,
    ) -> Result<Statement, CodeGenError> {
        match target {
            Expr::Var(name, _) if self.lookup_var(name).is_none() => {
                if let Some((class, prop, recv)) = self.implicit_property(name) {
                    return self.property_write(recv, &class, &prop, vec![], value, span);
                }
            }
            Expr::FieldAccess { record, field, .. } => {
                let rec = self.dx(record)?;
                if let Some(class) = self.class_instance_of(&self.infer_declared_type(&rec)) {
                    return self.member_write(rec, &class, field, value, span);
                }
                return Ok(Statement::PlaceAssignment {
                    target: Expr::FieldAccess {
                        record: Box::new(rec),
                        field: field.clone(),
                        span,
                    },
                    expr: value,
                    span,
                });
            }
            Expr::Index { .. } => {
                if let Some(access) = self.array_property(target)? {
                    let ArrayProperty {
                        recv,
                        class,
                        prop,
                        args,
                        rest,
                    } = access;
                    if rest.is_empty() {
                        return self.property_write(recv, &class, &prop, args, value, span);
                    }
                    // `Obj.Rows[I][J] := V`: an element of what it reads.
                    let mut place = self.property_read(recv, &class, &prop, args, span)?;
                    for index in rest {
                        place = Expr::Index {
                            array: Box::new(place),
                            index: Box::new(index),
                            span,
                        };
                    }
                    return Ok(Statement::PlaceAssignment {
                        target: place,
                        expr: value,
                        span,
                    });
                }
            }
            _ => {}
        }
        Ok(Statement::PlaceAssignment {
            target: self.dx(target)?,
            expr: value,
            span,
        })
    }
}
