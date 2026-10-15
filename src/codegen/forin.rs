//! `for X in Collection do`: lowered to a `while` loop over hidden locals.
//!
//! A string (of any kind) yields its characters, an array its elements.
//! Anything else must be an enumerator built in place, as Free Vision
//! writes `for EGC in TUnicodeStringExtendedGraphemeClustersEnumerator
//! .Create(S) do`: Bruto has no classes, so `T` is an `object` type, the
//! call runs constructor `Create` on a hidden instance, and the loop runs
//! `while E.MoveNext do X := E.Current`.

use super::{CodeGen, CodeGenError};
use crate::ast::*;

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_for_in(
        &mut self,
        var: &str,
        collection: &Expr,
        body: &Block,
        span: Span,
    ) -> Result<(), CodeGenError> {
        self.set_debug_loc(span);
        // This loop's hidden locals are the ones added from here on.
        let first_local = self.for_in_locals.len();
        let var_expr = |name: &str| Expr::Var(name.to_string(), span);
        let mut statements = Vec::new();
        let condition;

        if let Some((type_name, constructor, args)) = self.enumerator_construction(collection) {
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
            let (first, last) = match &collection_ty {
                PascalType::String | PascalType::ShortString { .. } | PascalType::UnicodeString => {
                    (1, None)
                }
                PascalType::Array { lo, hi, .. } => (*lo, Some(*hi)),
                _ => {
                    return Err(CodeGenError::new(
                        "for-in needs a string, an array or an enumerator (T.Create(...))",
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
                        name: "length".to_string(),
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
        self.compile_statement(&Statement::While {
            condition,
            body: Block {
                span: body.span,
                end_span: body.end_span,
                statements,
            },
            span,
        })?;
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
