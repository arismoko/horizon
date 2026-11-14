//! `StringTagVisitor` below the text-component layer. The shared visitor emits
//! exact Java UTF-16; UTF-8 callers explicitly escape unpaired surrogate units.
use horizon_core::{
    java_number::{double_to_string, float_to_string},
    nbt::{JavaString, Tag},
};

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// `StringTag.quoteAndEscape`, including SnbtGrammar's uppercase hex controls.
fn quoted_units(units: impl Iterator<Item = u16> + Clone, sink: &mut impl FnMut(u16)) {
    // A streaming sink cannot patch the opening delimiter afterward.
    let quote = match units.clone().find(|unit| matches!(*unit, 34 | 39)) {
        Some(34) => 39,
        _ => 34,
    };
    sink(quote);
    for unit in units {
        if unit == 92 || unit == quote {
            sink(92);
            sink(unit);
        } else if unit < 32 {
            sink(92);
            match unit {
                8 => sink(98),
                9 => sink(116),
                10 => sink(110),
                12 => sink(102),
                13 => sink(114),
                _ => {
                    sink(120);
                    sink(u16::from(HEX[usize::from(unit >> 4)]));
                    sink(u16::from(HEX[usize::from(unit & 15)]));
                }
            }
        } else {
            sink(unit);
        }
    }
    sink(quote);
}

#[derive(Default)]
struct Utf8Output {
    value: String,
    high: Option<u16>,
}
impl Utf8Output {
    fn escaped_surrogate(&mut self, unit: u16) {
        self.value.push_str("\\u");
        for shift in [12, 8, 4, 0] {
            self.value
                .push(char::from(HEX[usize::from((unit >> shift) & 15)]));
        }
    }
    fn unit(&mut self, unit: u16) {
        if let Some(high) = self.high.take() {
            if (0xdc00..=0xdfff).contains(&unit) {
                let scalar = 0x10000 + (u32::from(high - 0xd800) << 10) + u32::from(unit - 0xdc00);
                self.value
                    .push(char::from_u32(scalar).expect("surrogate pair"));
                return;
            }
            self.escaped_surrogate(high);
        }
        if (0xd800..=0xdbff).contains(&unit) {
            self.high = Some(unit);
        } else if (0xdc00..=0xdfff).contains(&unit) {
            self.escaped_surrogate(unit);
        } else {
            self.value
                .push(char::from_u32(u32::from(unit)).expect("non-surrogate UTF-16"));
        }
    }
    fn finish(mut self) -> String {
        if let Some(high) = self.high.take() {
            self.escaped_surrogate(high);
        }
        self.value
    }
}

/// `StringTag.quoteAndEscape` for a representable UTF-8 input string.
pub fn quote_and_escape(input: &str) -> String {
    let mut out = Utf8Output::default();
    quoted_units(input.encode_utf16(), &mut |unit| out.unit(unit));
    out.finish()
}

fn unquoted_key(key: &[u16]) -> bool {
    let ascii_lower = |unit: u16| {
        if (65..=90).contains(&unit) {
            unit + 32
        } else {
            unit
        }
    };
    for reserved in [b"true".as_slice(), b"false".as_slice()] {
        if key.len() == reserved.len()
            && key
                .iter()
                .zip(reserved)
                .all(|(&unit, &byte)| ascii_lower(unit) == u16::from(byte))
        {
            return false;
        }
    }
    let letter = |unit| (65..=90).contains(&unit) || (97..=122).contains(&unit);
    key.first()
        .is_some_and(|&unit| letter(unit) || matches!(unit, 46 | 95))
        && key.iter().all(|&unit| {
            letter(unit) || (48..=57).contains(&unit) || matches!(unit, 46 | 95 | 43 | 45)
        })
}

/// UI-only UTF-8 projection; lossless serialization uses the UTF-16 visitor.
pub fn string_of(value: &JavaString) -> String {
    value.to_string_lossy()
}

/// UTF-8 compact SNBT. Unpaired units become explicit `\uXXXX` SNBT escapes:
/// the NBT meaning is lossless, but exact Java String identity requires UTF-16.
/// The adapter streams directly into String without an intermediate UTF-16 buffer.
pub fn to_snbt(tag: &Tag) -> String {
    let mut out = Utf8Output::default();
    write_snbt_utf16(tag, |unit| out.unit(unit));
    out.finish()
}

/// Owning exact `Tag.toString` result, including raw unpaired UTF-16 units.
pub fn to_snbt_utf16(tag: &Tag) -> JavaString {
    let mut out = JavaString::default();
    write_snbt_utf16(tag, |unit| out.0.push(unit));
    out
}

/// `StringTagVisitor`: exact Java UTF-16 units, without an owning output buffer.
/// Compounds sort borrowed UTF-16 keys explicitly; storage traversal is not sorted.
pub fn write_snbt_utf16(tag: &Tag, mut sink: impl FnMut(u16)) {
    visit(tag, &mut sink);
}

fn ascii(value: &str, sink: &mut impl FnMut(u16)) {
    for byte in value.bytes() {
        sink(u16::from(byte));
    }
}

fn integer(value: i64, sink: &mut impl FnMut(u16)) {
    let mut digits = [0u16; 20];
    let mut start = digits.len();
    let mut magnitude = value.unsigned_abs();
    loop {
        start -= 1;
        digits[start] = 48 + (magnitude % 10) as u16;
        magnitude /= 10;
        if magnitude == 0 {
            break;
        }
    }
    if value < 0 {
        start -= 1;
        digits[start] = 45;
    }
    for &unit in &digits[start..] {
        sink(unit);
    }
}

fn compound_entry(key: &JavaString, value: &Tag, sink: &mut impl FnMut(u16)) {
    if unquoted_key(&key.0) {
        for &unit in &key.0 {
            sink(unit);
        }
    } else {
        quoted_units(key.0.iter().copied(), sink);
    }
    sink(58);
    visit(value, sink);
}

fn visit(tag: &Tag, sink: &mut impl FnMut(u16)) {
    match tag {
        Tag::End => ascii("END", sink),
        Tag::Byte(value) => {
            integer(i64::from(*value), sink);
            sink(98);
        }
        Tag::Short(value) => {
            integer(i64::from(*value), sink);
            sink(115);
        }
        Tag::Int(value) => integer(i64::from(*value), sink),
        Tag::Long(value) => {
            integer(*value, sink);
            sink(76);
        }
        Tag::Float(bits) => {
            ascii(&float_to_string(f32::from_bits(*bits)), sink);
            sink(102);
        }
        Tag::Double(bits) => {
            ascii(&double_to_string(f64::from_bits(*bits)), sink);
            sink(100);
        }
        Tag::String(value) => quoted_units(value.0.iter().copied(), sink),
        Tag::ByteArray(values) => {
            ascii("[B;", sink);
            for (i, value) in values.iter().enumerate() {
                if i != 0 {
                    sink(44);
                }
                integer(i64::from(*value as i8), sink);
                sink(66);
            }
            sink(93);
        }
        Tag::IntArray(values) => {
            ascii("[I;", sink);
            for (i, value) in values.iter().enumerate() {
                if i != 0 {
                    sink(44);
                }
                integer(i64::from(*value), sink);
            }
            sink(93);
        }
        Tag::LongArray(values) => {
            ascii("[L;", sink);
            for (i, value) in values.iter().enumerate() {
                if i != 0 {
                    sink(44);
                }
                integer(*value, sink);
                sink(76);
            }
            sink(93);
        }
        Tag::List(values) => {
            sink(91);
            for (i, value) in values.iter().enumerate() {
                if i != 0 {
                    sink(44);
                }
                visit(value, sink);
            }
            sink(93);
        }
        Tag::Compound(values) => {
            sink(123);
            if values.len() == 1 {
                let (key, value) = values.iter().next().expect("one entry");
                compound_entry(key, value, sink);
            } else {
                let mut entries: Vec<_> = values.iter().collect();
                entries.sort_unstable_by_key(|(key, _)| *key);
                for (i, (key, value)) in entries.into_iter().enumerate() {
                    if i != 0 {
                        sink(44);
                    }
                    compound_entry(key, value, sink);
                }
            }
            sink(125);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scientific_text_selects_the_closest_two_digit_decimal() {
        for (bits, expected) in [
            (1, "1.4E-45"),
            (2, "2.8E-45"),
            (7, "9.8E-45"),
            (71, "9.9E-44"),
            (100, "1.4E-43"),
        ] {
            for (sign, prefix) in [(0, ""), (1 << 31, "-")] {
                let bits = bits | sign;
                assert_eq!(
                    float_to_string(f32::from_bits(bits)),
                    format!("{prefix}{expected}")
                );
                assert_eq!(to_snbt(&Tag::Float(bits)), format!("{prefix}{expected}f"));
            }
        }
        for (bits, expected) in [
            (1, "4.9E-324"),
            (2, "9.9E-324"),
            (12, "5.9E-323"),
            (100, "4.94E-322"),
        ] {
            for (sign, prefix) in [(0, ""), (1 << 63, "-")] {
                let bits = bits | sign;
                assert_eq!(
                    double_to_string(f64::from_bits(bits)),
                    format!("{prefix}{expected}")
                );
                assert_eq!(to_snbt(&Tag::Double(bits)), format!("{prefix}{expected}d"));
            }
        }
    }

    #[test]
    fn utf16_strings_and_utf8_projection_keep_unpaired_units_lossless() {
        let tag = Tag::String(JavaString(vec![
            0, 8, 9, 10, 11, 12, 13, 31, 34, 39, 92, 0xd800, 0xd83d, 0xde00, 0xdc00,
        ]));
        let mut expected: Vec<u16> = "'\\x00\\b\\t\\n\\x0B\\f\\r\\x1F\"\\'\\\\"
            .encode_utf16()
            .collect();
        expected.extend([0xd800, 0xd83d, 0xde00, 0xdc00, 39]);
        assert_eq!(to_snbt_utf16(&tag).0, expected);
        let mut streamed = Vec::new();
        write_snbt_utf16(&tag, |unit| streamed.push(unit));
        assert_eq!(streamed, expected);
        assert_eq!(
            to_snbt(&tag),
            "'\\x00\\b\\t\\n\\x0B\\f\\r\\x1F\"\\'\\\\\\uD800\u{1f600}\\uDC00'"
        );
    }

    #[test]
    fn utf16_key_sorting_and_integer_extremes_match_the_compact_visitor() {
        let tag = Tag::compound([
            (JavaString(vec![0xe000]), Tag::Long(i64::MIN)),
            (JavaString(vec![0xd800, 0xdc00]), Tag::Int(i32::MIN)),
            (JavaString(vec![0xd800]), Tag::Byte(-128)),
        ]);
        let mut expected = vec![123, 34, 0xd800, 34];
        expected.extend(":-128b,".encode_utf16());
        expected.extend([34, 0xd800, 0xdc00, 34]);
        expected.extend(":-2147483648,".encode_utf16());
        expected.extend([34, 0xe000, 34]);
        expected.extend(":-9223372036854775808L}".encode_utf16());
        assert_eq!(to_snbt_utf16(&tag).0, expected);
        assert_eq!(to_snbt(&Tag::Float(0x80000000)), "-0.0f");
        assert_eq!(to_snbt(&Tag::Double(0x8000000000000000)), "-0.0d");
    }
}
