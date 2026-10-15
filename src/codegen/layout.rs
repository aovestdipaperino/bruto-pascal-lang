//! Record layout, as C (and FPC) lays records out: each field at the next
//! multiple of its alignment, the record's size rounded up to its largest
//! alignment. A variant part is a union: its tag (only when it has a name,
//! `case Kind: Byte of`) and then an area as large as the largest variant
//! and aligned like its most demanding field; within a variant, fields
//! follow one another the same way. LLVM lays out the struct types
//! `llvm_type_for` builds with the same rules, so the offsets computed here
//! are the ones the generated code uses.

use super::CodeGen;
use crate::ast::*;

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
            PascalType::Array { elem, .. } => self.align_of(elem),
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

    /// The size of a record with `fields` and `variant`, padded to its
    /// alignment.
    pub(super) fn record_size(
        &self,
        fields: &[(String, PascalType)],
        variant: &Option<Box<RecordVariant>>,
    ) -> u64 {
        let mut offset = 0;
        let mut align = 1;
        for (_, t) in fields {
            let a = self.align_of(t);
            offset = align_up(offset, a) + self.sizeof_type(t);
            align = align.max(a);
        }
        if let Some(v) = variant {
            if !v.tag_name.is_empty() {
                let a = self.align_of(&v.tag_type);
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
    ) -> (Vec<u64>, Option<u64>, u64) {
        let mut offsets = Vec::with_capacity(fields.len());
        let mut offset = 0;
        for (_, t) in fields {
            offset = align_up(offset, self.align_of(t));
            offsets.push(offset);
            offset += self.sizeof_type(t);
        }
        let mut tag = None;
        let mut area = offset;
        if let Some(v) = variant {
            if !v.tag_name.is_empty() {
                offset = align_up(offset, self.align_of(&v.tag_type));
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
                let a = self.align_of(t);
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
                offset = align_up(offset, self.align_of(ty));
                if name == field {
                    return Some((offset, ty.clone()));
                }
                // A nested variant part: an unnamed record with its own tag
                // and area.
                if name.is_empty()
                    && let PascalType::Record {
                        fields,
                        variant: Some(nested),
                        ..
                    } = ty
                    && let Some((inner, inner_ty)) = self.variant_field_offset(nested, field)
                {
                    let (_, _, area) = self.record_offsets(fields, &Some(nested.clone()));
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
