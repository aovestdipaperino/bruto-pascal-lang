//! Run-time type information: what `TypeInfo(T)` and a class's
//! `ClassInfo` point at, and the TypInfo unit reads.
//!
//! The records are the System prelude's `TTypeInfo`, `TTypeData` and
//! `TPropInfo` (`rtl/system.pp`): each type gets one constant global, a
//! `TTypeInfo` followed by its `TTypeData`, whose fields are filled by
//! name, so the layout is always the one the Pascal declarations give.
//! An enumeration's names are a table of short strings indexed by value
//! minus `MinValue`. A class's published properties are an array of
//! `TPropInfo`, its own ones; `ParentInfo` leads to the inherited ones.
//!
//! A property's `GetProc`/`SetProc` hold, as `PropProcs` says, a field's
//! offset (`ptField`), a method's code (`ptStatic`) or a virtual method's
//! offset in the VMT (`ptVirtual`); `StoredProc` may also be a constant
//! (`ptConst`).

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::types::{BasicTypeEnum, StructType};
use inkwell::values::{BasicValueEnum, PointerValue};
use std::collections::HashMap;

const PT_FIELD: u64 = 0;
const PT_STATIC: u64 = 1;
const PT_VIRTUAL: u64 = 2;
const PT_CONST: u64 = 3;

/// `Low(LongInt)`: no index, or no default.
const NO_VALUE: i64 = i32::MIN as i64;

// TTypeKind, TOrdType and TFloatType ordinals.
const TK_UNKNOWN: u64 = 0;
const TK_INTEGER: u64 = 1;
const TK_CHAR: u64 = 2;
const TK_ENUMERATION: u64 = 3;
const TK_FLOAT: u64 = 4;
const TK_SET: u64 = 5;
const TK_METHOD: u64 = 6;
const TK_SSTRING: u64 = 7;
const TK_ASTRING: u64 = 9;
const TK_VARIANT: u64 = 11;
const TK_ARRAY: u64 = 12;
const TK_RECORD: u64 = 13;
const TK_INTERFACE: u64 = 14;
const TK_CLASS: u64 = 15;
const TK_OBJECT: u64 = 16;
const TK_WCHAR: u64 = 17;
const TK_BOOL: u64 = 18;
const TK_INT64: u64 = 19;
const TK_QWORD: u64 = 20;
const TK_DYNARRAY: u64 = 21;
const TK_PROCVAR: u64 = 23;
const TK_USTRING: u64 = 24;
const TK_FILE: u64 = 27;
const TK_CLASSREF: u64 = 28;
const TK_POINTER: u64 = 29;

const OT_SBYTE: u64 = 0;
const OT_UBYTE: u64 = 1;
const OT_SWORD: u64 = 2;
const OT_UWORD: u64 = 3;
const OT_SLONG: u64 = 4;
const OT_ULONG: u64 = 5;
const OT_SQWORD: u64 = 6;

const FT_SINGLE: u64 = 0;
const FT_DOUBLE: u64 = 1;
const FT_CURR: u64 = 4;

/// A record's field values by (lower-case) name; the rest are zero.
type Fields<'ctx> = HashMap<&'static str, BasicValueEnum<'ctx>>;

impl<'ctx> CodeGen<'ctx> {
    /// The LLVM struct type of the prelude record `name`, if declared.
    fn rtti_record(&self, name: &str) -> Option<(StructType<'ctx>, Vec<String>)> {
        let ty = self.type_defs.get(name)?;
        match self.resolve_type(ty) {
            record @ PascalType::Record { .. } => {
                let PascalType::Record { fields, .. } = &record else {
                    unreachable!()
                };
                let names = fields.iter().map(|(n, _)| n.clone()).collect();
                Some((self.llvm_type_for(&record).into_struct_type(), names))
            }
            _ => None,
        }
    }

    /// A constant of prelude record `name` with `values` in its fields.
    fn rtti_const(&self, name: &str, values: &Fields<'ctx>) -> Option<BasicValueEnum<'ctx>> {
        let (ty, names) = self.rtti_record(name)?;
        let fields: Vec<BasicValueEnum> = names
            .iter()
            .zip(ty.get_field_types())
            .map(|(n, field_ty)| match values.get(n.as_str()).copied() {
                // An integer at the field's width (an enumeration's is
                // its own).
                Some(BasicValueEnum::IntValue(v)) if field_ty.is_int_type() => {
                    let want = field_ty.into_int_type();
                    match v.get_sign_extended_constant() {
                        Some(n) if v.get_type() != want => want.const_int(n as u64, true).into(),
                        _ => v.into(),
                    }
                }
                Some(v) => v,
                None => zero_of(field_ty),
            })
            .collect();
        Some(ty.const_named_struct(&fields).into())
    }

    /// A short string constant (`[256 x i8]`) holding `text`.
    fn short_string_const(&self, text: &str) -> BasicValueEnum<'ctx> {
        let mut bytes = vec![text.len().min(255) as u8];
        bytes.extend(text.bytes().take(255));
        bytes.resize(256, 0);
        self.context.const_string(&bytes, false).into()
    }

    /// A pointer to a global short string holding `text`.
    fn short_string_global(&self, text: &str) -> PointerValue<'ctx> {
        let mut bytes = vec![text.len().min(255) as u8];
        bytes.extend(text.bytes().take(255));
        let value = self.context.const_string(&bytes, false);
        let g = self.module.add_global(value.get_type(), None, "rtti.name");
        g.set_linkage(Linkage::Internal);
        g.set_constant(true);
        g.set_initializer(&value);
        g.as_pointer_value()
    }

    fn i64_const(&self, v: i64) -> BasicValueEnum<'ctx> {
        self.context.i64_type().const_int(v as u64, true).into()
    }

    /// The name FPC gives a type in its type information.
    fn rtti_type_name(&self, name: &str) -> String {
        match name {
            "integer" => "Int64".into(),
            "longint" | "int32" => "LongInt".into(),
            "shortint" | "int8" => "ShortInt".into(),
            "smallint" | "int16" => "SmallInt".into(),
            "byte" | "uint8" => "Byte".into(),
            "word" | "uint16" => "Word".into(),
            "longword" | "cardinal" | "dword" | "uint32" => "LongWord".into(),
            "int64" => "Int64".into(),
            "qword" | "uint64" => "QWord".into(),
            "real" | "double" | "extended" => "Double".into(),
            "single" => "Single".into(),
            "string" | "ansistring" => "AnsiString".into(),
            "shortstring" => "ShortString".into(),
            "unicodestring" | "widestring" => "UnicodeString".into(),
            "char" | "ansichar" => "Char".into(),
            "widechar" => "WideChar".into(),
            "boolean" => "Boolean".into(),
            "pointer" => "Pointer".into(),
            _ => self.display(name),
        }
    }

    /// `TypeInfo(T)` for the type written `expr` (a type name): a pointer
    /// to its type information, or nil if there is none.
    pub(super) fn compile_type_info(
        &mut self,
        expr: &Expr,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let Expr::Var(name, _) = expr else {
            return Err(CodeGenError::new("TypeInfo needs a type name", Some(span)));
        };
        let ty = self
            .type_defs
            .get(name.as_str())
            .cloned()
            .or_else(|| crate::sema::builtin_type(name))
            .or_else(|| match name.as_str() {
                "integer" => Some(PascalType::Integer),
                "real" => Some(PascalType::Real),
                "boolean" => Some(PascalType::Boolean),
                "char" => Some(PascalType::Char),
                "string" => Some(PascalType::String),
                _ => None,
            })
            .or_else(|| self.lookup_var_type(name).cloned());
        let Some(ty) = ty else {
            return Err(CodeGenError::new(
                format!("'{}' is not a type", self.display(name)),
                Some(span),
            ));
        };
        Ok(self
            .type_info(&ty, Some(name))
            .unwrap_or_else(|| ptr.const_null())
            .into())
    }

    /// The type information of `ty` (named `name`, if it has a name).
    pub(super) fn type_info(
        &mut self,
        ty: &PascalType,
        name: Option<&str>,
    ) -> Option<PointerValue<'ctx>> {
        // An alias (`TComponentName = string`, `TFoo = TBar`) shares its
        // target's information, as in FPC.
        if let PascalType::Named(n) = ty
            && !self.objects.contains_key(n.as_str())
            && let Some(def) = self.type_defs.get(n.as_str()).cloned()
        {
            match def {
                PascalType::Named(target) if target != *n => {
                    return self.type_info(&PascalType::Named(target.clone()), Some(&target));
                }
                PascalType::Integer
                | PascalType::SizedInt { .. }
                | PascalType::Real
                | PascalType::Single
                | PascalType::Currency
                | PascalType::String
                | PascalType::ShortString { .. }
                | PascalType::UnicodeString
                | PascalType::Boolean
                | PascalType::Char
                | PascalType::WideChar => return self.type_info(&def, None),
                _ => {}
            }
        }
        // A named type: by its name (an alias of a class: the class).
        let (ty, name) = match (ty, name) {
            (PascalType::Named(n), _) => (self.resolve_type(ty), Some(n.clone())),
            (t, n) => (self.resolve_type(t), n.map(str::to_string)),
        };
        let class = self.class_instance_of(&ty);
        let key = match (&class, &name) {
            (Some(c), _) => format!("class.{c}"),
            (None, Some(n)) => format!("named.{n}"),
            (None, None) => format!("anon.{ty:?}"),
        };
        let global_name = format!("rtti.{key}");
        if let Some(g) = self.module.get_global(&global_name) {
            return Some(g.as_pointer_value());
        }
        let (info_ty, _) = self.rtti_record("ttypeinfo")?;
        let (data_ty, _) = self.rtti_record("ttypedata")?;
        let both = self
            .context
            .struct_type(&[info_ty.into(), data_ty.into()], false);
        // The global first, so the information can refer to itself.
        let g = self.module.add_global(both, None, &global_name);
        g.set_linkage(Linkage::Internal);
        g.set_constant(true);
        let me = g.as_pointer_value();

        let shown = match (&class, &name) {
            (Some(c), _) => self.display(c),
            (None, Some(n)) => self.rtti_type_name(n),
            (None, None) => builtin_type_name(&ty).to_string(),
        };
        let mut data: Fields = HashMap::new();
        let kind = match &class {
            Some(c) => {
                self.class_type_data(c, &mut data);
                TK_CLASS
            }
            None => self.plain_type_data(&ty, me, &mut data),
        };
        let mut info: Fields = HashMap::new();
        info.insert("kind", self.i64_const(kind as i64));
        info.insert("name", self.short_string_const(&shown));
        let info_value = self.rtti_const("ttypeinfo", &info)?;
        let data_value = self.rtti_const("ttypedata", &data)?;
        g.set_initializer(&both.const_named_struct(&[info_value, data_value]));
        Some(me)
    }

    /// Fill the type data of a type other than a class; its kind.
    fn plain_type_data(
        &mut self,
        ty: &PascalType,
        me: PointerValue<'ctx>,
        data: &mut Fields<'ctx>,
    ) -> u64 {
        let i32_ty = self.context.i32_type();
        let ordinal = |data: &mut Fields<'ctx>, ord_type: u64, lo: i64, hi: i64| {
            data.insert("ordtype", self.i64_const(ord_type as i64));
            data.insert(
                "minvalue",
                i32_ty
                    .const_int(lo.clamp(i32::MIN as i64, i32::MAX as i64) as u64, true)
                    .into(),
            );
            data.insert(
                "maxvalue",
                i32_ty
                    .const_int(hi.clamp(i32::MIN as i64, i32::MAX as i64) as u64, true)
                    .into(),
            );
        };
        match ty {
            PascalType::Integer => {
                data.insert("minint64value", self.i64_const(i64::MIN));
                data.insert("maxint64value", self.i64_const(i64::MAX));
                TK_INT64
            }
            PascalType::SizedInt {
                bits: 64,
                signed: true,
            } => {
                data.insert("minint64value", self.i64_const(i64::MIN));
                data.insert("maxint64value", self.i64_const(i64::MAX));
                TK_INT64
            }
            PascalType::SizedInt {
                bits: 64,
                signed: false,
            } => {
                data.insert("minqwordvalue", self.i64_const(0));
                data.insert("maxqwordvalue", self.i64_const(-1));
                TK_QWORD
            }
            PascalType::SizedInt { bits, signed } => {
                let (ord_type, lo, hi) = match (bits, signed) {
                    (8, true) => (OT_SBYTE, -128, 127),
                    (8, false) => (OT_UBYTE, 0, 255),
                    (16, true) => (OT_SWORD, -32768, 32767),
                    (16, false) => (OT_UWORD, 0, 65535),
                    (_, true) => (OT_SLONG, i32::MIN as i64, i32::MAX as i64),
                    (_, false) => (OT_ULONG, 0, u32::MAX as i64),
                };
                ordinal(data, ord_type, lo, hi);
                TK_INTEGER
            }
            PascalType::Subrange { lo, hi } => {
                ordinal(data, OT_SQWORD, *lo, *hi);
                TK_INTEGER
            }
            PascalType::Char => {
                ordinal(data, OT_UBYTE, 0, 255);
                TK_CHAR
            }
            PascalType::WideChar => {
                ordinal(data, OT_UWORD, 0, 65535);
                TK_WCHAR
            }
            PascalType::Boolean => {
                ordinal(data, OT_UBYTE, 0, 1);
                data.insert("basetype", me.into());
                let names = self.enum_name_table(&["False".into(), "True".into()], &[0, 1]);
                data.insert("namelist", names.into());
                TK_BOOL
            }
            PascalType::Enum {
                values,
                ordinals,
                size,
                ..
            } => {
                let lo = ordinals.iter().copied().min().unwrap_or(0);
                let hi = ordinals.iter().copied().max().unwrap_or(0);
                // As its storage: unsigned unless an ordinal is negative.
                let ord_type = match (size, lo < 0) {
                    (1, false) => OT_UBYTE,
                    (1, true) => OT_SBYTE,
                    (2, false) => OT_UWORD,
                    (2, true) => OT_SWORD,
                    (_, false) => OT_ULONG,
                    (_, true) => OT_SLONG,
                };
                ordinal(data, ord_type, lo, hi);
                data.insert("basetype", me.into());
                let shown: Vec<String> = values.iter().map(|v| self.display(v)).collect();
                let names = self.enum_name_table(&shown, ordinals);
                data.insert("namelist", names.into());
                TK_ENUMERATION
            }
            PascalType::Set { elem, .. } => {
                // Ordinal access reads the set's bytes (the first 32
                // elements' bits of a large one).
                let ord_type = match self.set_layout(ty).0 {
                    1 => OT_UBYTE,
                    2 => OT_UWORD,
                    _ => OT_ULONG,
                };
                ordinal(data, ord_type, 0, 0);
                let elem = (**elem).clone();
                let elem_name = match &elem {
                    PascalType::Named(n) => Some(n.clone()),
                    _ => None,
                };
                if let Some(comp) = self.type_info(&elem, elem_name.as_deref()) {
                    data.insert("comptype", comp.into());
                }
                TK_SET
            }
            PascalType::Real => {
                data.insert("floattype", self.i64_const(FT_DOUBLE as i64));
                TK_FLOAT
            }
            PascalType::Single => {
                data.insert("floattype", self.i64_const(FT_SINGLE as i64));
                TK_FLOAT
            }
            PascalType::Currency => {
                data.insert("floattype", self.i64_const(FT_CURR as i64));
                TK_FLOAT
            }
            PascalType::String => TK_ASTRING,
            PascalType::ShortString { .. } => TK_SSTRING,
            PascalType::UnicodeString => TK_USTRING,
            PascalType::Variant => TK_VARIANT,
            PascalType::Proc {
                of_object: true, ..
            } => TK_METHOD,
            PascalType::Proc { .. } => TK_PROCVAR,
            PascalType::Interface(_) => TK_INTERFACE,
            PascalType::ClassOf(_) => TK_CLASSREF,
            PascalType::DynArray(_) => TK_DYNARRAY,
            PascalType::Array { .. } => TK_ARRAY,
            PascalType::Record {
                object: Some(_), ..
            } => TK_OBJECT,
            PascalType::Record { .. } => TK_RECORD,
            PascalType::File { .. } => TK_FILE,
            PascalType::Pointer(_) => TK_POINTER,
            _ => TK_UNKNOWN,
        }
    }

    /// A table of pointers to short strings, one per value from the lowest
    /// ordinal to the highest (nil for gaps).
    fn enum_name_table(&self, names: &[String], ordinals: &[i64]) -> PointerValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let lo = ordinals.iter().copied().min().unwrap_or(0);
        let hi = ordinals.iter().copied().max().unwrap_or(0);
        let count = ((hi - lo + 1).clamp(0, 65536)) as usize;
        let mut entries = vec![ptr.const_null(); count];
        for (i, n) in names.iter().enumerate() {
            let ord = ordinals.get(i).copied().unwrap_or(i as i64);
            if let Some(slot) = entries.get_mut((ord - lo) as usize) {
                *slot = self.short_string_global(n);
            }
        }
        let table = ptr.const_array(&entries);
        let g = self.module.add_global(table.get_type(), None, "rtti.names");
        g.set_linkage(Linkage::Internal);
        g.set_constant(true);
        g.set_initializer(&table);
        g.as_pointer_value()
    }

    /// The published properties of class `class` (own ones, in order).
    pub(super) fn published_properties(&self, class: &str) -> Vec<PropertyDecl> {
        let info = &self.objects[class];
        info.own_published
            .iter()
            .filter_map(|n| info.properties.get(n))
            .filter(|p| p.params.is_empty())
            .cloned()
            .collect()
    }

    /// How many published properties class `class` has, own and inherited.
    fn published_count(&self, class: &str) -> usize {
        let parent = self.objects[class].parent.clone();
        self.published_properties(class).len() + parent.map_or(0, |p| self.published_count(&p))
    }

    /// Fill a class's type data.
    fn class_type_data(&mut self, class: &str, data: &mut Fields<'ctx>) {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let info = self.objects[class].clone();
        data.insert("classtype", self.vmt_of(class).into());
        if let Some(parent) = &info.parent {
            let parent_ty = PascalType::Named(parent.clone());
            if let Some(p) = self.class_type_info(parent, &parent_ty) {
                data.insert("parentinfo", p.into());
            }
        }
        let total = self.published_count(class);
        data.insert(
            "propcount",
            self.context
                .i16_type()
                .const_int(total as u64, false)
                .into(),
        );
        let unit = self
            .type_units
            .get(class)
            .cloned()
            .unwrap_or_else(|| self.program_display_name.clone());
        data.insert("unitname", self.short_string_const(&unit));
        let own = self.published_properties(class);
        let inherited = total - own.len();
        let mut entries = Vec::new();
        for (i, prop) in own.iter().enumerate() {
            if let Some(entry) = self.prop_info(class, prop, inherited + i) {
                entries.push(entry);
            }
        }
        data.insert(
            "ownpropcount",
            self.context
                .i32_type()
                .const_int(entries.len() as u64, false)
                .into(),
        );
        self.class_field_and_method_tables(class, data);
        if let (Some((prop_ty, _)), false) = (self.rtti_record("tpropinfo"), entries.is_empty()) {
            let values: Vec<_> = entries.iter().map(|e| e.into_struct_value()).collect();
            let array = prop_ty.const_array(&values);
            let g = self
                .module
                .add_global(array.get_type(), None, &format!("rtti.props.{class}"));
            g.set_linkage(Linkage::Internal);
            g.set_constant(true);
            g.set_initializer(&array);
            data.insert("ownprops", g.as_pointer_value().into());
        } else {
            data.insert("ownprops", ptr.const_null().into());
        }
    }

    /// A class's published field table (its class-typed fields, by offset)
    /// and method table (name and code), for `FieldAddress`,
    /// `MethodAddress` and `MethodName`.
    fn class_field_and_method_tables(&mut self, class: &str, data: &mut Fields<'ctx>) {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i32_ty = self.context.i32_type();
        let info = self.objects[class].clone();
        let PascalType::Record { fields, .. } = self.resolve_type(&info.record) else {
            return;
        };
        let record_ty = self.llvm_type_for(&info.record);
        let mut field_entries = Vec::new();
        let mut method_entries = Vec::new();
        for name in &info.own_published {
            if let Some(index) = fields.iter().position(|(n, _)| n == name) {
                if self.class_instance_of(&fields[index].1).is_none() {
                    continue;
                }
                let offset = unsafe {
                    ptr.const_null().const_gep(
                        record_ty,
                        &[i32_ty.const_zero(), i32_ty.const_int(index as u64, false)],
                    )
                }
                .const_to_int(self.context.i64_type());
                let mut values: Fields = HashMap::new();
                values.insert("fieldoffset", offset.into());
                values.insert("name", self.short_string_const(&self.display(name)));
                if let Some(v) = self.rtti_const("tvmtfieldentry", &values) {
                    field_entries.push(v.into_struct_value());
                }
            } else if let Some(m) = info.methods.get(name)
                && m.symbol == format!("{class}.{name}")
                && let Some(f) = self.module.get_function(&m.symbol)
            {
                let mut values: Fields = HashMap::new();
                values.insert("name", self.short_string_global(&self.display(name)).into());
                values.insert("codeaddress", f.as_global_value().as_pointer_value().into());
                if let Some(v) = self.rtti_const("tvmtmethodentry", &values) {
                    method_entries.push(v.into_struct_value());
                }
            }
        }
        for (count_field, table_field, record, entries) in [
            (
                "ownfieldcount",
                "ownfields",
                "tvmtfieldentry",
                field_entries,
            ),
            (
                "ownmethodcount",
                "ownmethods",
                "tvmtmethodentry",
                method_entries,
            ),
        ] {
            data.insert(
                count_field,
                i32_ty.const_int(entries.len() as u64, false).into(),
            );
            let table = match (self.rtti_record(record), entries.is_empty()) {
                (Some((ty, _)), false) => {
                    let array = ty.const_array(&entries);
                    let g = self.module.add_global(array.get_type(), None, "rtti.table");
                    g.set_linkage(Linkage::Internal);
                    g.set_constant(true);
                    g.set_initializer(&array);
                    g.as_pointer_value()
                }
                _ => ptr.const_null(),
            };
            data.insert(table_field, table.into());
        }
    }

    /// The type information of class `class`.
    fn class_type_info(&mut self, class: &str, ty: &PascalType) -> Option<PointerValue<'ctx>> {
        self.type_info(ty, Some(class))
    }

    /// A published property's `TPropInfo`.
    fn prop_info(
        &mut self,
        class: &str,
        prop: &PropertyDecl,
        name_index: usize,
    ) -> Option<BasicValueEnum<'ctx>> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i32_ty = self.context.i32_type();
        let prop_name = match &prop.ty {
            PascalType::Named(n) => Some(n.clone()),
            _ => None,
        };
        let prop_type = self.type_info(&prop.ty, prop_name.as_deref());
        let mut values: Fields = HashMap::new();
        values.insert(
            "proptype",
            prop_type.unwrap_or_else(|| ptr.const_null()).into(),
        );
        let (get, get_kind) = self.prop_proc(class, prop.read.as_deref());
        let (set, set_kind) = self.prop_proc(class, prop.write.as_deref());
        // Stored: a constant (True unless `stored False`), or a method or
        // field saying.
        let (stored, stored_kind) = match &prop.stored {
            None => (self.ptr_const(1), PT_CONST),
            Some(DefaultValue(Expr::BoolLit(b, _))) => (self.ptr_const(u64::from(*b)), PT_CONST),
            Some(DefaultValue(Expr::Var(n, _))) if n == "true" || n == "false" => {
                (self.ptr_const(u64::from(n == "true")), PT_CONST)
            }
            Some(DefaultValue(Expr::Var(n, _))) => {
                let n = n.clone();
                self.prop_proc(class, Some(&n))
            }
            Some(_) => (self.ptr_const(1), PT_CONST),
        };
        values.insert("getproc", get.into());
        values.insert("setproc", set.into());
        values.insert("storedproc", stored.into());
        values.insert(
            "index",
            i32_ty
                .const_int(prop.index.unwrap_or(NO_VALUE) as u64, true)
                .into(),
        );
        // Without a default, an ordinal property has none ($80000000);
        // any other's is 0, as in FPC.
        let ordinal = matches!(
            self.resolve_type(&prop.ty),
            PascalType::Integer
                | PascalType::SizedInt { .. }
                | PascalType::Subrange { .. }
                | PascalType::Enum { .. }
                | PascalType::Set { .. }
                | PascalType::Char
                | PascalType::WideChar
                | PascalType::Boolean
        );
        let default = prop
            .default_value
            .as_ref()
            .and_then(|d| self.ordinal_constant(&d.0))
            .unwrap_or(if ordinal { NO_VALUE } else { 0 });
        values.insert("default", i32_ty.const_int(default as u64, true).into());
        values.insert(
            "nameindex",
            self.context
                .i16_type()
                .const_int(name_index as u64, false)
                .into(),
        );
        values.insert(
            "propprocs",
            self.context
                .i8_type()
                .const_int(get_kind | (set_kind << 2) | (stored_kind << 4), false)
                .into(),
        );
        values.insert("name", self.short_string_const(&self.display(&prop.name)));
        self.rtti_const("tpropinfo", &values)
    }

    fn ptr_const(&self, v: u64) -> PointerValue<'ctx> {
        self.context
            .i64_type()
            .const_int(v, false)
            .const_to_pointer(self.context.ptr_type(AddressSpace::default()))
    }

    /// What a property accessor (a field path or a method) is, for its
    /// `TPropInfo`: the value and its `pt*` kind.
    fn prop_proc(&mut self, class: &str, accessor: Option<&str>) -> (PointerValue<'ctx>, u64) {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let Some(accessor) = accessor else {
            return (ptr.const_null(), PT_FIELD);
        };
        let info = self.objects[class].clone();
        if let Some(m) = info.methods.get(accessor) {
            if let Some(slot) = m.slot {
                let header = super::objects::CLASS_VMT_HEADER_WORDS;
                return (self.ptr_const((header + slot as u64) * 8), PT_VIRTUAL);
            }
            return match self.module.get_function(&m.symbol) {
                Some(f) => (f.as_global_value().as_pointer_value(), PT_STATIC),
                None => (ptr.const_null(), PT_FIELD),
            };
        }
        // A field path: its offset in the instance.
        let mut ty = info.record.clone();
        let mut indices = vec![self.context.i32_type().const_zero()];
        for part in accessor.split('.') {
            let PascalType::Record { fields, .. } = self.resolve_type(&ty) else {
                return (ptr.const_null(), PT_FIELD);
            };
            let Some(index) = fields.iter().position(|(n, _)| n == part) else {
                return (ptr.const_null(), PT_FIELD);
            };
            indices.push(self.context.i32_type().const_int(index as u64, false));
            ty = fields[index].1.clone();
        }
        let record_ty = self.llvm_type_for(&info.record);
        let offset = unsafe { ptr.const_null().const_gep(record_ty, &indices) };
        let offset = offset.const_to_int(self.context.i64_type());
        (offset.const_to_pointer(ptr), PT_FIELD)
    }

    /// The ordinal value of a constant expression (a number, an
    /// enumeration value, a boolean or a character), if it is one.
    fn ordinal_constant(&self, expr: &Expr) -> Option<i64> {
        match expr {
            Expr::IntLit(v, _) => Some(*v),
            Expr::BoolLit(b, _) => Some(i64::from(*b)),
            Expr::CharLit(c, _) => Some(*c as i64),
            Expr::Var(n, _) => self
                .enum_values
                .get(n.as_str())
                .copied()
                .or(match n.as_str() {
                    "true" => Some(1),
                    "false" => Some(0),
                    _ => None,
                }),
            Expr::UnaryOp {
                op: UnaryOp::Neg,
                operand,
                ..
            } => self.ordinal_constant(operand).map(|v| -v),
            _ => None,
        }
    }
}

/// The zero value of an LLVM type.
fn zero_of(ty: BasicTypeEnum<'_>) -> BasicValueEnum<'_> {
    match ty {
        BasicTypeEnum::ArrayType(t) => t.const_zero().into(),
        BasicTypeEnum::FloatType(t) => t.const_zero().into(),
        BasicTypeEnum::IntType(t) => t.const_zero().into(),
        BasicTypeEnum::PointerType(t) => t.const_zero().into(),
        BasicTypeEnum::StructType(t) => t.const_zero().into(),
        BasicTypeEnum::VectorType(t) => t.const_zero().into(),
        BasicTypeEnum::ScalableVectorType(t) => t.const_zero().into(),
    }
}

/// The name of a built-in type written without a name.
fn builtin_type_name(ty: &PascalType) -> &'static str {
    match ty {
        PascalType::Integer => "Int64",
        PascalType::SizedInt { bits, signed } => match (bits, signed) {
            (8, true) => "ShortInt",
            (8, false) => "Byte",
            (16, true) => "SmallInt",
            (16, false) => "Word",
            (32, true) => "LongInt",
            (32, false) => "LongWord",
            (_, true) => "Int64",
            (_, false) => "QWord",
        },
        PascalType::Real => "Double",
        PascalType::Single => "Single",
        PascalType::Currency => "Currency",
        PascalType::String => "AnsiString",
        PascalType::ShortString { .. } => "ShortString",
        PascalType::UnicodeString => "UnicodeString",
        PascalType::Boolean => "Boolean",
        PascalType::Char => "Char",
        PascalType::WideChar => "WideChar",
        PascalType::Pointer(_) => "Pointer",
        PascalType::Variant => "Variant",
        _ => "",
    }
}
