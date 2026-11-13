//! Persistent scalar codecs for component values backed by context number providers.
use super::{component_float, component_int};
use horizon_core::nbt::{JavaString, Tag};

/// A validated resource key, borrowing its UTF-16 namespace and path.
#[derive(Clone, Copy, Debug)]
pub struct Identifier<'a> {
    namespace: Option<&'a [u16]>,
    path: &'a [u16],
}

impl PartialEq for Identifier<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
            && match (self.namespace, other.namespace) {
                (None, None) => true,
                (Some(a), Some(b)) => a == b,
                (None, Some(namespace)) | (Some(namespace), None) => {
                    namespace.iter().copied().eq("minecraft".encode_utf16())
                }
            }
    }
}
impl Eq for Identifier<'_> {}

impl Identifier<'_> {
    /// Encode the canonical key once while loading, not while matching a stack.
    pub fn to_tag(self) -> Tag {
        let namespace_len = self.namespace.map_or("minecraft".len(), <[u16]>::len);
        let mut value = Vec::with_capacity(namespace_len + 1 + self.path.len());
        if let Some(namespace) = self.namespace {
            value.extend_from_slice(namespace);
        } else {
            value.extend("minecraft".encode_utf16());
        }
        value.push(u16::from(b':'));
        value.extend_from_slice(self.path);
        Tag::String(JavaString(value))
    }
}

fn namespace_char(value: u16) -> bool {
    u8::try_from(value).is_ok_and(|value| {
        value.is_ascii_lowercase() || value.is_ascii_digit() || matches!(value, b'_' | b'-' | b'.')
    })
}

/// `Identifier.CODEC`, without a resource lookup or a lossy string conversion.
pub fn identifier(value: &Tag) -> Option<Identifier<'_>> {
    let Tag::String(value) = value else {
        return None;
    };
    let (namespace, path) = match value.0.iter().position(|&value| value == u16::from(b':')) {
        Some(colon) => (&value.0[..colon], &value.0[colon + 1..]),
        None => (&[][..], value.0.as_slice()),
    };
    if namespace == [u16::from(b'.'); 2]
        || !namespace.iter().copied().all(namespace_char)
        || !path
            .iter()
            .copied()
            .all(|value| value == u16::from(b'/') || namespace_char(value))
    {
        return None;
    }
    Some(Identifier {
        namespace: (!namespace.is_empty()).then_some(namespace),
        path,
    })
}

/// `ResolvableInt`: an integer constant or an unresolved context-provider key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Int<'a> {
    Constant(i32),
    Reference(Identifier<'a>),
}
impl Int<'_> {
    /// Canonical persistent representation, allocated only at load time.
    pub fn to_tag(self) -> Tag {
        match self {
            Self::Constant(value) => Tag::Int(value),
            Self::Reference(key) => key.to_tag(),
        }
    }
}

/// `ResolvableInt.CODEC`: Java boxed-number conversion, then a resource key.
pub fn int(value: &Tag) -> Option<Int<'_>> {
    if let Some(value) = component_int(value) {
        Some(Int::Constant(value))
    } else {
        identifier(value).map(Int::Reference)
    }
}

/// `ResolvableFloat`: boxed-float equality canonicalizes NaNs but preserves signed zero.
#[derive(Clone, Copy, Debug)]
pub enum Float<'a> {
    Constant(f32),
    Reference(Identifier<'a>),
}
impl PartialEq for Float<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Self::Constant(a), Self::Constant(b)) => {
                a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
            }
            (Self::Reference(a), Self::Reference(b)) => a == b,
            _ => false,
        }
    }
}
impl Eq for Float<'_> {}
impl Float<'_> {
    /// Canonical persistent representation, allocated only at load time.
    pub fn to_tag(self) -> Tag {
        match self {
            Self::Constant(value) => Tag::Float(value.to_bits()),
            Self::Reference(key) => key.to_tag(),
        }
    }
}

/// `ResolvableFloat.CODEC`: unrestricted numeric f32 or an unresolved resource key.
pub fn float(value: &Tag) -> Option<Float<'_>> {
    if let Some(value) = component_float(value) {
        Some(Float::Constant(value))
    } else {
        identifier(value).map(Float::Reference)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_and_provider_codecs_preserve_java_equality_and_identifier_rules() {
        assert_eq!(
            int(&Tag::Long(4_294_967_297)).unwrap().to_tag(),
            Tag::Int(1)
        );
        assert_eq!(
            int(&Tag::Double(f64::NAN.to_bits())).unwrap().to_tag(),
            Tag::Int(0)
        );
        assert_eq!(
            int(&Tag::String("provider/path".into())).unwrap().to_tag(),
            Tag::String("minecraft:provider/path".into())
        );
        for (a, b) in [
            ("provider", "minecraft:provider"),
            (":", "minecraft:"),
            ("", "minecraft:"),
        ] {
            let a = Tag::String(a.into());
            let b = Tag::String(b.into());
            assert_eq!(int(&a).unwrap(), int(&b).unwrap());
            assert_eq!(float(&a).unwrap(), float(&b).unwrap());
        }
        let zero = Tag::Float(0.0_f32.to_bits());
        let negative_zero = Tag::Double((-0.0_f64).to_bits());
        assert_ne!(float(&zero).unwrap(), float(&negative_zero).unwrap());
        let nan = Tag::Float(f32::NAN.to_bits());
        let other_nan = Tag::Double(0x7ff8_0000_0000_0001);
        assert_eq!(float(&nan).unwrap(), float(&other_nan).unwrap());
        assert_eq!(
            float(&Tag::Double(f64::INFINITY.to_bits()))
                .unwrap()
                .to_tag(),
            Tag::Float(f32::INFINITY.to_bits())
        );
        for value in [
            Tag::String("UPPER:path".into()),
            Tag::String("..:path".into()),
            Tag::String("x:bad:path".into()),
            Tag::String(JavaString(vec![0xd800])),
            Tag::Compound(Default::default()),
            Tag::List(Vec::new()),
        ] {
            assert!(int(&value).is_none(), "{value:?}");
            assert!(float(&value).is_none(), "{value:?}");
        }
    }
}
