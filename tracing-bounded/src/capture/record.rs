use tracing_core::{
    field::{Field, Visit},
    Metadata,
};

use super::{BuildError, Reject};

#[derive(Debug, PartialEq)]
pub(super) struct Record {
    pub(super) metadata: &'static Metadata<'static>,
    pub(super) fields: Vec<OwnedField>,
}

#[derive(Debug, PartialEq)]
pub(super) struct OwnedField {
    pub(super) name: &'static str,
    pub(super) value: Value,
}

#[derive(Debug, PartialEq)]
pub(super) enum Value {
    Bool(bool),
    I64(i64),
    U64(u64),
    I128(i128),
    U128(u128),
    F64(f64),
    Str(String),
    Bytes(Vec<u8>),
}

pub(super) struct Slot {
    metadata: Option<&'static Metadata<'static>>,
    fields: Box<[Option<StoredField>]>,
    bytes: Box<[u8]>,
    fields_used: usize,
    bytes_used: usize,
}

#[derive(Clone, Copy)]
pub(super) struct StoredField {
    name: &'static str,
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

        Ok(Self {
            metadata: None,
            fields: fields.into_boxed_slice(),
            bytes: bytes.into_boxed_slice(),
            fields_used: 0,
            bytes_used: 0,
        })
    }

    pub(super) fn begin(&mut self, metadata: &'static Metadata<'static>) {
        self.metadata = Some(metadata);
        self.fields_used = 0;
        self.bytes_used = 0;
    }

    pub(super) fn visitor(&mut self) -> SlotVisitor<'_> {
        SlotVisitor {
            slot: self,
            rejection: None,
        }
    }

    pub(super) fn snapshot(&self) -> Record {
        let metadata = self.metadata.expect("live slot must have metadata");
        let fields = self.fields[..self.fields_used]
            .iter()
            .map(|field| {
                let field = field.expect("live field range must be initialized");
                OwnedField {
                    name: field.name,
                    value: self.owned_value(field.value),
                }
            })
            .collect();
        Record { metadata, fields }
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
