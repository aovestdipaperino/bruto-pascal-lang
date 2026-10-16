//! Record layout, as C (and FPC) lays records out: each field at the next
//! multiple of its alignment, the record's size rounded up to its largest
//! alignment. A variant part is a union: its tag (only when it has a name,
//! `case Kind: Byte of`) and then an area as large as the largest variant
//! and aligned like its most demanding field; within a variant, fields
//! follow one another the same way. LLVM lays out the struct types
//! `llvm_type_for` builds with the same rules, so the offsets computed here
//! are the ones the generated code uses.

use super::{CodeGen, CodeGenError};
use crate::ast::*;
use inkwell::types::BasicType;
use inkwell::values::BasicValue;

/// Where a variant part lives in a record's LLVM struct.
pub(super) struct VariantSlots {
    /// The tag's struct index, when the tag has a name.
    pub tag: Option<u32>,
    /// The union area's struct index.
    pub area: u32,
}

fn align_up(offset: u64, align: u64) -> u64 {
    offset.div_ceil(align.max(1)) * align.max(1)
}

impl<'ctx> CodeGen<'ctx> {
    /// The alignment of a value of type `ty`, in bytes.
    pub(super) fn align_of(&self, ty: &PascalType) -> u64 {
        match ty {
            PascalType::Named(_) => self.align_of(&self.resolve_type(ty)),
            PascalType::SizedInt { bits, .. } => (*bits / 8).max(1) as u64,
            PascalType::Char
            | PascalType::Boolean
            | PascalType::Untyped
            | PascalType::ShortString { .. }
            | PascalType::Object(_) => 1,
            PascalType::WideChar => 2,
            PascalType::Single => 4,
            PascalType::Enum { size, .. } => u64::from(*size),
            PascalType::Array { elem, .. } => self.align_of(elem),
            PascalType::Record { packed: true, .. } => 1,
            PascalType::Set { .. } => match self.set_layout(ty).0 {
                size @ (1 | 2 | 4 | 8) => size,
                32 => 8,
                _ => 1,
            },
            PascalType::Record {
                fields, variant, ..
            } => {
                let mut align = fields
                    .iter()
                    .map(|(_, t)| self.align_of(t))
                    .max()
                    .unwrap_or(1);
                if let Some(v) = variant {
                    if !v.tag_name.is_empty() {
                        align = align.max(self.align_of(&v.tag_type));
                    }
                    align = align.max(self.variant_area(v).1);
                }
                align
            }
            _ => 8,
        }
    }

    /// The bytes a set of type `ty` takes and the byte of the 256-bit
    /// form its first byte holds, as FPC lays sets out: by default 4
    /// bytes when no element is above 31, else 32; with `{$PACKSET n}`
    /// (Delphi and TP modes: 1) the bytes from its lowest element's
    /// (rounded down to n) to its highest's, rounded up to n, and 3 made 4.
    pub(super) fn set_layout(&self, ty: &PascalType) -> (u64, u64) {
        let PascalType::Set { elem, pack } = self.resolve_type(ty) else {
            return (32, 0);
        };
        let (lo, hi) = self
            .ordinal_bounds(&self.resolve_type(&elem))
            .unwrap_or((0, 255));
        let (lo, hi) = (lo.clamp(0, 255) as u64, hi.clamp(0, 255) as u64);
        if pack == 0 {
            return if hi <= 31 { (4, 0) } else { (32, 0) };
        }
        let n = u64::from(pack);
        let base = lo / 8 / n * n;
        let mut size = (hi / 8 - base + 1).div_ceil(n) * n;
        if size == 3 {
            size = 4;
        }
        (size, base.min(32 - size))
    }

    /// The LLVM type a set of type `ty` is stored as.
    pub(super) fn set_storage_type(&self, ty: &PascalType) -> inkwell::types::BasicTypeEnum<'ctx> {
        match self.set_layout(ty).0 {
            32 => self.context.i64_type().array_type(4).as_basic_type_enum(),
            size @ (1 | 2 | 4 | 8) => self
                .context
                .custom_width_int_type(size as u32 * 8)
                .as_basic_type_enum(),
            size => self
                .context
                .i8_type()
                .array_type(size as u32)
                .as_basic_type_enum(),
        }
    }

    /// A set value as stored (`set_storage_type`) made the 256-bit form
    /// expressions use.
    pub(super) fn set_from_storage(
        &self,
        val: inkwell::values::BasicValueEnum<'ctx>,
        ty: &PascalType,
        span: Span,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, CodeGenError> {
        let wide = self.context.i64_type().array_type(4);
        let (size, base) = self.set_layout(ty);
        if size == 32 || val.get_type() == wide.as_basic_type_enum() {
            return Ok(val);
        }
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let temp = self.entry_alloca(wide, "set_wide").map_err(err)?;
        self.builder
            .build_store(temp, wide.const_zero())
            .map_err(err)?;
        let i8_ty = self.context.i8_type();
        let at = unsafe {
            self.builder
                .build_in_bounds_gep(
                    i8_ty,
                    temp,
                    &[self.context.i64_type().const_int(base, false)],
                    "set_at",
                )
                .map_err(err)?
        };
        let store = self.builder.build_store(at, val).map_err(err)?;
        store
            .set_alignment(1)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        self.builder.build_load(wide, temp, "set").map_err(err)
    }

    /// A 256-bit set value made the form a set of type `ty` is stored in.
    pub(super) fn set_to_storage(
        &self,
        val: inkwell::values::BasicValueEnum<'ctx>,
        ty: &PascalType,
        span: Span,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, CodeGenError> {
        let wide = self.context.i64_type().array_type(4);
        let (size, base) = self.set_layout(ty);
        if size == 32 || val.get_type() != wide.as_basic_type_enum() {
            return Ok(val);
        }
        let err = |e: inkwell::builder::BuilderError| CodeGenError::new(e.to_string(), Some(span));
        let temp = self.entry_alloca(wide, "set_wide").map_err(err)?;
        self.builder.build_store(temp, val).map_err(err)?;
        let at = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    temp,
                    &[self.context.i64_type().const_int(base, false)],
                    "set_at",
                )
                .map_err(err)?
        };
        let load = self
            .builder
            .build_load(self.set_storage_type(ty), at, "set_bytes")
            .map_err(err)?;
        load.as_instruction_value()
            .unwrap()
            .set_alignment(1)
            .map_err(|e| CodeGenError::new(e.to_string(), Some(span)))?;
        Ok(load)
    }

    /// A field's alignment in a record: 1 when the record is packed.
    fn field_align(&self, ty: &PascalType, packed: bool) -> u64 {
        if packed { 1 } else { self.align_of(ty) }
    }

    /// The size of a record with `fields` and `variant`, padded to its
    /// alignment.
    pub(super) fn record_size(
        &self,
        fields: &[(String, PascalType)],
        variant: &Option<Box<RecordVariant>>,
        packed: bool,
    ) -> u64 {
        let mut offset = 0;
        let mut align = 1;
        for (_, t) in fields {
            let a = self.field_align(t, packed);
            offset = align_up(offset, a) + self.sizeof_type(t);
            align = align.max(a);
        }
        if let Some(v) = variant {
            if !v.tag_name.is_empty() {
                let a = self.field_align(&v.tag_type, packed);
                offset = align_up(offset, a) + self.sizeof_type(&v.tag_type);
                align = align.max(a);
            }
            let (size, a) = self.variant_area(v);
            offset = align_up(offset, a) + size;
            align = align.max(a);
        }
        align_up(offset, align)
    }

    /// The byte offset of each fixed field, and of the variant area.
    pub(super) fn record_offsets(
        &self,
        fields: &[(String, PascalType)],
        variant: &Option<Box<RecordVariant>>,
        packed: bool,
    ) -> (Vec<u64>, Option<u64>, u64) {
        let mut offsets = Vec::with_capacity(fields.len());
        let mut offset = 0;
        for (_, t) in fields {
            offset = align_up(offset, self.field_align(t, packed));
            offsets.push(offset);
            offset += self.sizeof_type(t);
        }
        let mut tag = None;
        let mut area = offset;
        if let Some(v) = variant {
            if !v.tag_name.is_empty() {
                offset = align_up(offset, self.field_align(&v.tag_type, packed));
                tag = Some(offset);
                offset += self.sizeof_type(&v.tag_type);
            }
            area = align_up(offset, self.variant_area(v).1);
        }
        (offsets, tag, area)
    }

    /// The size and alignment of a variant part's union area.
    pub(super) fn variant_area(&self, v: &RecordVariant) -> (u64, u64) {
        let mut size = 0;
        let mut align = 1;
        for (_, vfields) in &v.variants {
            let mut offset = 0;
            for (_, t) in vfields {
                let a = self.field_align(t, v.packed);
                offset = align_up(offset, a) + self.sizeof_type(t);
                align = align.max(a);
            }
            size = size.max(offset);
        }
        (align_up(size, align), align)
    }

    /// The offset of each field within its variant (variants overlap, so
    /// each starts at 0), by name.
    pub(super) fn variant_field_offset(
        &self,
        v: &RecordVariant,
        field: &str,
    ) -> Option<(u64, PascalType)> {
        for (_, vfields) in &v.variants {
            let mut offset = 0;
            for (name, ty) in vfields {
                offset = align_up(offset, self.field_align(ty, v.packed));
                if name == field {
                    return Some((offset, ty.clone()));
                }
                // A nested variant part: an unnamed record with its own tag
                // and area.
                if name.is_empty()
                    && let PascalType::Record {
                        fields,
                        variant: Some(nested),
                        packed,
                        ..
                    } = ty
                    && let Some((inner, inner_ty)) = self.variant_field_offset(nested, field)
                {
                    let (_, _, area) = self.record_offsets(fields, &Some(nested.clone()), *packed);
                    return Some((offset + area + inner, inner_ty));
                }
                offset += self.sizeof_type(ty);
            }
        }
        None
    }

    /// The struct indices of a variant part, after `field_count` fixed
    /// fields.
    pub(super) fn variant_slots(field_count: usize, v: &RecordVariant) -> VariantSlots {
        if v.tag_name.is_empty() {
            VariantSlots {
                tag: None,
                area: field_count as u32,
            }
        } else {
            VariantSlots {
                tag: Some(field_count as u32),
                area: field_count as u32 + 1,
            }
        }
    }
}
