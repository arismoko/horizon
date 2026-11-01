//! Typed native-mod data attached to server-owned holders.
use std::{any::Any, collections::BTreeMap, fmt};

use crate::nbt::{Compound, JavaString, Tag};

/// An attachment operation could not be completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentError {
    /// The holder does not exist or is unavailable on this side.
    MissingHolder,
    /// The key was used with a different Rust type.
    TypeMismatch,
    /// The saved value could not be decoded.
    Decode,
    /// The value could not be encoded.
    Encode,
    /// A namespace in the saved compound is not a compound.
    InvalidNamespace,
}

/// The optional on-disk representation of an attachment.
pub struct AttachmentCodec<T> {
    pub encode: fn(&T) -> Result<Compound, AttachmentError>,
    pub decode: fn(&Compound) -> Result<T, AttachmentError>,
}

/// A mod-defined attachment key and its value's behavior.
pub struct AttachmentType<T: 'static> {
    pub namespace: &'static str,
    pub name: &'static str,
    pub default: fn() -> T,
    pub codec: Option<AttachmentCodec<T>>,
    pub copy_on_death: bool,
}

trait Value: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn clone_box(&self) -> Box<dyn Value>;
    fn encode(&self) -> Result<Option<Compound>, AttachmentError>;
}
struct TypedValue<T> {
    value: T,
    encode: Option<fn(&T) -> Result<Compound, AttachmentError>>,
}
impl<T: Clone + Send + Sync + 'static> Value for TypedValue<T> {
    fn as_any(&self) -> &dyn Any {
        &self.value
    }
    fn clone_box(&self) -> Box<dyn Value> {
        Box::new(Self {
            value: self.value.clone(),
            encode: self.encode,
        })
    }
    fn encode(&self) -> Result<Option<Compound>, AttachmentError> {
        self.encode.map(|encode| encode(&self.value)).transpose()
    }
}
enum Entry {
    Raw(Compound),
    Decoded(Box<dyn Value>),
}
impl Clone for Entry {
    fn clone(&self) -> Self {
        match self {
            Self::Raw(value) => Self::Raw(value.clone()),
            Self::Decoded(value) => Self::Decoded(value.clone_box()),
        }
    }
}

/// The data held by one entity, block entity, chunk or level.
#[derive(Clone, Default)]
pub struct Attachments {
    entries: BTreeMap<(JavaString, JavaString), Entry>,
    // Keep malformed or otherwise unrecognized NBT exactly as it was read.
    raw: Compound,
}
impl fmt::Debug for Attachments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attachments")
            .field("keys", &self.entries.keys())
            .finish()
    }
}

impl Attachments {
    /// Read the namespaced attachment compound, leaving unknown data intact.
    pub fn from_nbt(raw: Compound) -> Self {
        let mut result = Self {
            entries: BTreeMap::new(),
            raw,
        };
        for (namespace, tag) in &result.raw {
            if let Tag::Compound(names) = tag {
                for (name, value) in names {
                    if let Tag::Compound(value) = value {
                        result
                            .entries
                            .insert((namespace.clone(), name.clone()), Entry::Raw(value.clone()));
                    }
                }
            }
        }
        result
    }
    /// Encode changed values, preserving raw entries and unrecognized tags.
    pub fn to_nbt(&self) -> Result<Compound, AttachmentError> {
        let mut raw = self.raw.clone();
        for ((namespace, name), entry) in &self.entries {
            let value = match entry {
                Entry::Raw(value) => Some(value.clone()),
                Entry::Decoded(value) => value.encode()?,
            };
            if let Some(value) = value {
                let tag = raw
                    .entry(namespace.clone())
                    .or_insert_with(|| Tag::Compound(Compound::new()));
                let Tag::Compound(names) = tag else {
                    return Err(AttachmentError::InvalidNamespace);
                };
                names.insert(name.clone(), Tag::Compound(value));
            }
        }
        Ok(raw)
    }
    /// Get an existing value or insert its default; saved values decode lazily.
    pub fn get<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
    ) -> Result<T, AttachmentError> {
        let key = (ty.namespace.into(), ty.name.into());
        let value = match self.entries.get(&key) {
            Some(Entry::Raw(raw)) => {
                (ty.codec.as_ref().ok_or(AttachmentError::Decode)?.decode)(raw)?
            }
            Some(Entry::Decoded(value)) => {
                return value
                    .as_any()
                    .downcast_ref::<T>()
                    .cloned()
                    .ok_or(AttachmentError::TypeMismatch);
            }
            None => (ty.default)(),
        };
        self.entries.insert(
            key,
            Entry::Decoded(Box::new(TypedValue {
                value: value.clone(),
                encode: ty.codec.as_ref().map(|codec| codec.encode),
            })),
        );
        Ok(value)
    }
    /// Replace a value. A key already decoded as a different type is rejected.
    pub fn set<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
        value: T,
    ) -> Result<(), AttachmentError> {
        let key = (ty.namespace.into(), ty.name.into());
        if let Some(Entry::Decoded(old)) = self.entries.get(&key)
            && !old.as_any().is::<T>()
        {
            return Err(AttachmentError::TypeMismatch);
        }
        self.entries.insert(
            key,
            Entry::Decoded(Box::new(TypedValue {
                value,
                encode: ty.codec.as_ref().map(|codec| codec.encode),
            })),
        );
        if let Some(Tag::Compound(names)) = self.raw.get_mut(&ty.namespace.into()) {
            names.remove(&ty.name.into());
        }
        Ok(())
    }
    /// Remove a value, retaining all other namespaces and names.
    pub fn remove<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
    ) -> Result<Option<T>, AttachmentError> {
        let key = (ty.namespace.into(), ty.name.into());
        if !self.entries.contains_key(&key) {
            return Ok(None);
        }
        let value = self.get(ty)?;
        self.entries.remove(&key);
        if let Some(Tag::Compound(names)) = self.raw.get_mut(&ty.namespace.into()) {
            names.remove(&ty.name.into());
        }
        Ok(Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(n: &i32) -> Result<Compound, AttachmentError> {
        Ok([(JavaString::from("value"), Tag::Int(*n))].into())
    }
    fn decode(c: &Compound) -> Result<i32, AttachmentError> {
        match c.get(&"value".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        }
    }
    static COUNTER: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "counter",
        default: || 0,
        codec: Some(AttachmentCodec { encode, decode }),
        copy_on_death: false,
    };
    static MEMORY: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "memory",
        default: || 3,
        codec: None,
        copy_on_death: false,
    };
    #[test]
    fn saved_values_round_trip_and_memory_values_do_not() {
        let mut map = Attachments::default();
        map.set(&COUNTER, 42).unwrap();
        map.set(&MEMORY, 8).unwrap();
        let mut loaded = Attachments::from_nbt(map.to_nbt().unwrap());
        assert_eq!(loaded.get(&COUNTER), Ok(42));
        assert_eq!(loaded.get(&MEMORY), Ok(3));
        assert_eq!(loaded.remove(&COUNTER), Ok(Some(42)));
        let mut reopened = Attachments::from_nbt(loaded.to_nbt().unwrap());
        assert_eq!(reopened.get(&COUNTER), Ok(0));
    }
    #[test]
    fn unknown_and_invalid_values_survive_a_failed_decode() {
        let saved: Compound = [(
            JavaString::from("test"),
            Tag::Compound(
                [
                    (
                        JavaString::from("counter"),
                        Tag::Compound(
                            [(JavaString::from("value"), Tag::String("bad".into()))].into(),
                        ),
                    ),
                    (JavaString(vec![0xd800]), Tag::Compound(Compound::new())),
                ]
                .into(),
            ),
        )]
        .into();
        let mut map = Attachments::from_nbt(saved.clone());
        assert_eq!(map.get(&COUNTER), Err(AttachmentError::Decode));
        assert_eq!(map.to_nbt(), Ok(saved));
    }
}
