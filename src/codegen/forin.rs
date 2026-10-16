//! `for X in Collection do`: lowered to a `while` loop over hidden locals.
//!
//! A string (of any kind) yields its characters, an array its elements.
//! An instance of a class yields what its `GetEnumerator` enumerates (or
//! is an enumerator itself), and the enumerator is freed after the loop.
//! An `object` type's enumerator is built in place, as Free Vision writes
//! `for EGC in TUnicodeStringExtendedGraphemeClustersEnumerator.Create(S)
//! do`: the call runs constructor `Create` on a hidden instance. Either
//! way the loop runs `while E.MoveNext do X := E.Current`.

use super::{CodeGen, CodeGenError};
use crate::ast::*;

impl<'ctx> CodeGen<'ctx> {
    /// The lowest and highest values of an ordinal type.
    pub(super) fn ordinal_bounds(&self, ty: &PascalType) -> Option<(i64, i64)> {
        match ty {
            PascalType::Enum { ordinals, .. } => Some((
                ordinals.iter().copied().min().unwrap_or(0),
                ordinals.iter().copied().max().unwrap_or(0),
            )),
            PascalType::Subrange { lo, hi } => Some((*lo, *hi)),
            PascalType::Boolean => Some((0, 1)),
            PascalType::Char => Some((0, 255)),
            PascalType::SizedInt {
                bits: 8,
                signed: false,
            } => Some((0, 255)),
            PascalType::SizedInt {
                bits: 8,
                signed: true,
            } => Some((-128, 127)),
            _ => None,
        }
    }

    pub(super) fn compile_for_in(
        &mut self,
        var: &str,
        collection: &Expr,
        body: &Block,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        // `for C in TColor`: every value of an ordinal type.
        if let Expr::Var(name, _) = collection
            && self.lookup_var(name).is_none()
            && let Some((lo, hi)) = self
                .type_of_name(name)
                .and_then(|t| self.ordinal_bounds(&self.resolve_type(&t)))
        {
            return self.compile_statement(&Statement::For {
                var: var.to_string(),
                from: Expr::IntLit(lo, span),
                to: Expr::IntLit(hi, span),
                downto: false,
                body: body.clone(),
                span,
            });
        }
        // `for X in S`: the members of a set, in order.
        if let PascalType::Set { elem, .. } =
            self.resolve_type(&self.infer_declared_type(collection))
            && let Some((lo, hi)) = self.ordinal_bounds(&self.resolve_type(&elem))
        {
            let test = Statement::If {
                condition: Expr::BinOp {
                    op: BinOp::In,
                    left: Box::new(Expr::Var(var.to_string(), span)),
                    right: Box::new(collection.clone()),
                    span,
                },
                then_branch: body.clone(),
                else_branch: None,
                span,
            };
            return self.compile_statement(&Statement::For {
                var: var.to_string(),
                from: Expr::IntLit(lo, span),
                to: Expr::IntLit(hi.min(lo + 255), span),
                downto: false,
                body: Block {
                    statements: vec![test],
                    span,
                    end_span: span,
                },
                span,
            });
        }
        // This loop's hidden locals are the ones added from here on.
        let first_local = self.for_in_locals.len();
        let var_expr = |name: &str| Expr::Var(name.to_string(), span);
        let mut statements = Vec::new();
        let condition;

        // A class instance: `Collection.GetEnumerator`, or the instance if it
        // is an enumerator itself (`TEnum.Create(...)`); freed afterwards.
        let mut free_enumerator = None;
        let class = self.class_instance_of(&self.infer_declared_type(collection));
        if let Some(class) = class {
            let info = &self.objects[&class];
            let (source, enum_class) = if info.methods.contains_key("getenumerator") {
                let result = self
                    .method_result_type(&class, "getenumerator")
                    .and_then(|t| self.class_instance_of(&t))
                    .ok_or_else(|| {
                        CodeGenError::new(
                            "GetEnumerator must return an instance of a class",
                            Some(span),
                        )
                    })?;
                let source = Expr::MethodCall {
                    object: Box::new(collection.clone()),
                    method: "getenumerator".to_string(),
                    args: Vec::new(),
                    span,
                };
                (source, result)
            } else if info.methods.contains_key("movenext") {
                (collection.clone(), class.clone())
            } else {
                return Err(CodeGenError::new(
                    format!(
                        "for-in over a '{}' needs a GetEnumerator method",
                        self.display(&class)
                    ),
                    Some(span),
                ));
            };
            let enumerator = self.hidden_local(&PascalType::Named(enum_class), span)?;
            self.compile_statement(&Statement::Assignment {
                target: enumerator.clone(),
                expr: source,
                span,
            })?;
            condition = Expr::MethodCall {
                object: Box::new(var_expr(&enumerator)),
                method: "movenext".to_string(),
                args: Vec::new(),
                span,
            };
            statements.push(Statement::Assignment {
                target: var.to_string(),
                expr: Expr::FieldAccess {
                    record: Box::new(var_expr(&enumerator)),
                    field: "current".to_string(),
                    span,
                },
                span,
            });
            free_enumerator = Some(Statement::MethodCall {
                object: var_expr(&enumerator),
                method: "free".to_string(),
                args: Vec::new(),
                span,
            });
        } else if let Some((type_name, constructor, args)) =
            self.enumerator_construction(collection)
        {
            let enumerator = self.hidden_local(&PascalType::Named(type_name), span)?;
            self.compile_statement(&Statement::MethodCall {
                object: var_expr(&enumerator),
                method: constructor,
                args,
                span,
            })?;
            condition = Expr::MethodCall {
                object: Box::new(var_expr(&enumerator)),
                method: "movenext".to_string(),
                args: Vec::new(),
                span,
            };
            statements.push(Statement::Assignment {
                target: var.to_string(),
                expr: Expr::FieldAccess {
                    record: Box::new(var_expr(&enumerator)),
                    field: "current".to_string(),
                    span,
                },
                span,
            });
        } else {
            let collection_ty = self.resolve_type(&self.infer_declared_type(collection));
            // The first index, and the last one (`None`: the length).
            // (A dynamic or open array runs to its High.)
            let zero_based = matches!(
                collection_ty,
                PascalType::DynArray(_) | PascalType::OpenArray(_)
            );
            let (first, last) = match &collection_ty {
                PascalType::String | PascalType::ShortString { .. } | PascalType::UnicodeString => {
                    (1, None)
                }
                PascalType::DynArray(_) | PascalType::OpenArray(_) => (0, None),
                PascalType::Array { lo, hi, .. } => (*lo, Some(*hi)),
                _ => {
                    return Err(CodeGenError::new(
                        "for-in needs a string, an array, a class with GetEnumerator or an enumerator",
                        Some(span),
                    ));
                }
            };
            // The collection is evaluated once.
            let items = self.hidden_local(&collection_ty, span)?;
            let value = self.compile_expr(collection)?;
            let slot = *self.lookup_var(&items).unwrap();
            self.store_value(slot, value, &collection_ty, span)?;
            let index = self.hidden_local(&PascalType::Integer, span)?;
            self.compile_statement(&Statement::Assignment {
                target: index.clone(),
                expr: Expr::IntLit(first - 1, span),
                span,
            })?;
            condition = Expr::BinOp {
                op: BinOp::Lt,
                left: Box::new(var_expr(&index)),
                right: Box::new(match last {
                    Some(hi) => Expr::IntLit(hi, span),
                    None => Expr::Call {
                        name: if zero_based { "high" } else { "length" }.to_string(),
                        args: vec![var_expr(&items)],
                        span,
                    },
                }),
                span,
            };
            statements.push(Statement::Assignment {
                target: index.clone(),
                expr: Expr::BinOp {
                    op: BinOp::Add,
                    left: Box::new(var_expr(&index)),
                    right: Box::new(Expr::IntLit(1, span)),
                    span,
                },
                span,
            });
            statements.push(Statement::Assignment {
                target: var.to_string(),
                expr: Expr::Index {
                    array: Box::new(var_expr(&items)),
                    index: Box::new(var_expr(&index)),
                    span,
                },
                span,
            });
        }
        statements.push(Statement::Block(body.clone()));
        let looped = Statement::While {
            condition,
            body: Block {
                span: body.span,
                end_span: body.end_span,
                statements,
            },
            span,
        };
        match free_enumerator {
            // `try ... finally E.Free end` where an exception or `exit` can
            // leave the loop.
            Some(free) if self.global_vars.contains_key("brutoexctop") => {
                self.compile_statement(&Statement::Try {
                    body: Block {
                        statements: vec![looped],
                        span,
                        end_span: body.end_span,
                    },
                    handler: TryHandler::Finally(Block {
                        statements: vec![free],
                        span,
                        end_span: body.end_span,
                    }),
                    span,
                })?;
            }
            Some(free) => {
                self.compile_statement(&looped)?;
                if self.block_is_open() {
                    self.compile_statement(&free)?;
                }
            }
            None => self.compile_statement(&looped)?,
        }
        // The hidden locals let go of their strings.
        for (name, ty) in self.for_in_locals.split_off(first_local) {
            if self.block_is_open()
                && self.needs_arc(&ty)
                && let Some(&slot) = self.variables.get(&name)
            {
                self.arc_release_slot(slot, &ty, span)?;
                let llvm_ty = self.llvm_type_for(&self.resolve_type(&ty));
                self.builder
                    .build_store(slot, llvm_ty.const_zero())
                    .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
            }
        }
        Ok(())
    }

    /// For `T.Create(args)` (or `T.Create`) with `T` an object type: the
    /// type, the constructor and its arguments.
    fn enumerator_construction(&self, expr: &Expr) -> Option<(String, String, Vec<Expr>)> {
        let (object, method, args) = match expr {
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => (object, method, args.clone()),
            Expr::FieldAccess { record, field, .. } => (record, field, Vec::new()),
            _ => return None,
        };
        let Expr::Var(type_name, _) = object.as_ref() else {
            return None;
        };
        if self.lookup_var(type_name).is_some() {
            return None;
        }
        let ty = self.type_defs.get(type_name)?;
        matches!(
            self.resolve_type(ty),
            PascalType::Record {
                object: Some(_),
                ..
            }
        )
        .then(|| (type_name.clone(), method.clone(), args))
    }

    /// A fresh local of type `ty`, zeroed, under a name no program can
    /// spell.
    fn hidden_local(&mut self, ty: &PascalType, span: Span) -> Result<String, CodeGenError> {
        self.hidden_count += 1;
        let name = format!("for$in{}", self.hidden_count);
        let llvm_ty = self.llvm_type_for(&self.resolve_type(ty));
        let slot = self
            .entry_alloca(llvm_ty, &name)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder
            .build_store(slot, llvm_ty.const_zero())
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.variables.insert(name.clone(), slot);
        self.var_types.insert(name.clone(), ty.clone());
        self.for_in_locals.push((name.clone(), ty.clone()));
        Ok(name)
    }
}
