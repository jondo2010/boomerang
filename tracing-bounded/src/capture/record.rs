use tracing_core::{
    field::{Field, Visit},
    Metadata,
};

use super::{BuildError, Reject};

/// A complete event and its copied ancestry, independent of live span storage.
#[derive(Debug, PartialEq)]
pub struct Record {
    /// Native static event metadata.
    pub metadata: &'static Metadata<'static>,
    /// Event fields, separate from scope fields with the same names.
    pub fields: Vec<OwnedField>,
    /// Captured ancestry, ordered root to leaf and independent of live spans.
    pub scopes: Vec<Scope>,
}

/// An owned copy of one event's span context.
#[derive(Debug, PartialEq)]
pub struct Scope {
    /// Native static span metadata.
    pub metadata: &'static Metadata<'static>,
    /// Values as observed when the event was committed.
    pub fields: Vec<OwnedField>,
}

/// A field name and its owned native value.
#[derive(Debug, PartialEq)]
pub struct OwnedField {
    /// Static name declared by the callsite.
    pub name: &'static str,
    /// Captured value; strings and bytes are owned.
    pub value: Value,
}

/// Primitive representations retained without formatting or numeric narrowing.
#[derive(Debug, PartialEq)]
pub enum Value {
    /// Boolean.
    Bool(bool),
    /// Signed integer up to 64 bits.
    I64(i64),
    /// Unsigned integer up to 64 bits.
    U64(u64),
    /// Signed 128-bit integer.
    I128(i128),
    /// Unsigned 128-bit integer.
    U128(u128),
    /// Native floating-point value, including NaNs and signed zero.
    F64(f64),
    /// Copied UTF-8 string.
    Str(String),
    /// Copied byte slice.
    Bytes(Vec<u8>),
}

pub(super) struct Slot {
    metadata: Option<&'static Metadata<'static>>,
    fields: Box<[Option<StoredField>]>,
    bytes: Box<[u8]>,
    fields_used: usize,
    bytes_used: usize,
    scopes: Box<[Option<StoredScope>]>,
    scopes_used: usize,
}

#[derive(Clone, Copy)]
struct StoredScope {
    metadata: &'static Metadata<'static>,
    start: usize,
    end: usize,
}

#[derive(Clone, Copy)]
pub(super) struct StoredField {
    name: &'static str,
    index: usize,
    value: StoredValue,
}

#[derive(Clone, Copy)]
enum StoredValue {
    Bool(bool),
    I64(i64),
    U64(u64),
    I128(i128),
    U128(u128),
    F64(f64),
    Str(ByteRange),
    Bytes(ByteRange),
}

#[derive(Clone, Copy)]
struct ByteRange {
    start: usize,
    len: usize,
}

impl Slot {
    pub(super) fn new(field_capacity: usize, byte_capacity: usize) -> Result<Self, BuildError> {
        Self::with_scopes(field_capacity, byte_capacity, 0)
    }

    pub(super) fn with_scopes(
        field_capacity: usize,
        byte_capacity: usize,
        depth: usize,
    ) -> Result<Self, BuildError> {
        let mut fields = Vec::new();
        fields
            .try_reserve_exact(field_capacity)
            .map_err(|_| BuildError::Allocation)?;
        fields.resize(field_capacity, None);

        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(byte_capacity)
            .map_err(|_| BuildError::Allocation)?;
        bytes.resize(byte_capacity, 0);
        let mut scopes = Vec::new();
        scopes
            .try_reserve_exact(depth)
            .map_err(|_| BuildError::Allocation)?;
        scopes.resize(depth, None);

        Ok(Self {
            metadata: None,
            fields: fields.into_boxed_slice(),
            bytes: bytes.into_boxed_slice(),
            fields_used: 0,
            bytes_used: 0,
            scopes: scopes.into_boxed_slice(),
            scopes_used: 0,
        })
    }

    pub(super) fn begin(&mut self, metadata: &'static Metadata<'static>) {
        self.metadata = Some(metadata);
        self.fields_used = 0;
        self.bytes_used = 0;
        self.scopes_used = 0;
    }

    pub(super) fn visitor(&mut self) -> SlotVisitor<'_> {
        SlotVisitor {
            slot: self,
            rejection: None,
        }
    }

    pub(super) fn snapshot(&self) -> Record {
        let metadata = self.metadata.expect("live slot must have metadata");
        let event_start = self
            .scopes_used
            .checked_sub(1)
            .map_or(0, |last| self.scopes[last].unwrap().end);
        let fields = self.owned_fields(event_start, self.fields_used);
        let scopes = self.scopes[..self.scopes_used]
            .iter()
            .map(|scope| {
                let scope = scope.unwrap();
                Scope {
                    metadata: scope.metadata,
                    fields: self.owned_fields(scope.start, scope.end),
                }
            })
            .collect();
        Record {
            metadata,
            fields,
            scopes,
        }
    }

    fn owned_fields(&self, start: usize, end: usize) -> Vec<OwnedField> {
        self.fields[start..end]
            .iter()
            .map(|field| {
                let field = field.expect("live field range must be initialized");
                OwnedField {
                    name: field.name,
                    value: self.owned_value(field.value),
                }
            })
            .collect()
    }

    pub(super) fn metadata(&self) -> &'static Metadata<'static> {
        self.metadata.expect("initialized slot")
    }

    pub(super) fn remaining_fields(&self) -> usize {
        self.fields.len() - self.fields_used
    }

    pub(super) fn append_scope(&mut self, source: &Self) -> Result<(), Reject> {
        if self.scopes_used == self.scopes.len() {
            return Err(Reject::InvalidContext);
        }
        let start = self.fields_used;
        for field in source.fields[..source.fields_used].iter().flatten() {
            self.copy_field(source, *field)?;
        }
        self.scopes[self.scopes_used] = Some(StoredScope {
            metadata: source.metadata(),
            start,
            end: self.fields_used,
        });
        self.scopes_used += 1;
        Ok(())
    }

    // Updates are first visited into scratch, then untouched values are copied.
    // Rebuilding compacts byte storage instead of consuming capacity per update.
    pub(super) fn retain_unmodified(&mut self, source: &Self) -> Result<(), Reject> {
        for field in source.fields[..source.fields_used].iter().flatten() {
            if !self.fields[..self.fields_used]
                .iter()
                .flatten()
                .any(|new| new.index == field.index)
            {
                self.copy_field(source, *field)?;
            }
        }
        Ok(())
    }

    fn copy_field(&mut self, source: &Self, mut field: StoredField) -> Result<(), Reject> {
        if self.remaining_fields() == 0 {
            return Err(Reject::FieldLimit);
        }
        if let StoredValue::Str(range) | StoredValue::Bytes(range) = field.value {
            let bytes = source.byte_range(range);
            if bytes.len() > self.bytes.len() - self.bytes_used {
                return Err(Reject::ByteLimit);
            }
            let copied = ByteRange {
                start: self.bytes_used,
                len: bytes.len(),
            };
            self.bytes[self.bytes_used..self.bytes_used + bytes.len()].copy_from_slice(bytes);
            self.bytes_used += bytes.len();
            field.value = if matches!(field.value, StoredValue::Str(_)) {
                StoredValue::Str(copied)
            } else {
                StoredValue::Bytes(copied)
            };
        }
        self.fields[self.fields_used] = Some(field);
        self.fields_used += 1;
        Ok(())
    }

    fn owned_value(&self, value: StoredValue) -> Value {
        match value {
            StoredValue::Bool(value) => Value::Bool(value),
            StoredValue::I64(value) => Value::I64(value),
            StoredValue::U64(value) => Value::U64(value),
            StoredValue::I128(value) => Value::I128(value),
            StoredValue::U128(value) => Value::U128(value),
            StoredValue::F64(value) => Value::F64(value),
            StoredValue::Str(range) => Value::Str(
                String::from_utf8(self.byte_range(range).to_vec())
                    .expect("stored str range must remain UTF-8"),
            ),
            StoredValue::Bytes(range) => Value::Bytes(self.byte_range(range).to_vec()),
        }
    }

    fn byte_range(&self, range: ByteRange) -> &[u8] {
        let end = range
            .start
            .checked_add(range.len)
            .expect("stored byte range must not overflow");
        &self.bytes[range.start..end]
    }
}

pub(super) struct SlotVisitor<'a> {
    slot: &'a mut Slot,
    rejection: Option<Reject>,
}

impl SlotVisitor<'_> {
    pub(super) fn rejection(&self) -> Option<Reject> {
        self.rejection
    }

    fn store(&mut self, field: &Field, value: StoredValue) {
        if self.rejection.is_some() {
            return;
        }
        let Some(destination) = self.slot.fields.get_mut(self.slot.fields_used) else {
            self.rejection = Some(Reject::FieldLimit);
            return;
        };
        *destination = Some(StoredField {
            name: field.name(),
            index: field.index(),
            value,
        });
        self.slot.fields_used += 1;
    }

    fn store_bytes(&mut self, field: &Field, value: &[u8], is_string: bool) {
        if self.rejection.is_some() {
            return;
        }
        let remaining = self.slot.bytes.len() - self.slot.bytes_used;
        if value.len() > remaining {
            self.rejection = Some(Reject::ByteLimit);
            return;
        }
        let start = self.slot.bytes_used;
        let end = start + value.len();
        self.slot.bytes[start..end].copy_from_slice(value);
        self.slot.bytes_used = end;
        let range = ByteRange {
            start,
            len: value.len(),
        };
        self.store(
            field,
            if is_string {
                StoredValue::Str(range)
            } else {
                StoredValue::Bytes(range)
            },
        );
    }

    fn reject_unsupported(&mut self) {
        if self.rejection.is_none() {
            self.rejection = Some(Reject::UnsupportedValue);
        }
    }
}

impl Visit for SlotVisitor<'_> {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.store(field, StoredValue::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.store(field, StoredValue::I64(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.store(field, StoredValue::U64(value));
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        self.store(field, StoredValue::I128(value));
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        self.store(field, StoredValue::U128(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.store(field, StoredValue::F64(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.store_bytes(field, value.as_bytes(), true);
    }

    fn record_bytes(&mut self, field: &Field, value: &[u8]) {
        self.store_bytes(field, value, false);
    }

    fn record_error(&mut self, _field: &Field, _value: &(dyn std::error::Error + 'static)) {
        self.reject_unsupported();
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {
        self.reject_unsupported();
    }
}
