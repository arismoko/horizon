//! Portable `Display` / `TextDisplay` synced data encoding and transformation decoding (26.3).
pub mod resident;

use crate::save::{float, number};
use sunrise_core::nbt::Tag;
use sunrise_protocol::entity_data::{Value, serializer};

const DEFAULT_TRANSLATION: [f32; 3] = [0.0; 3];
const DEFAULT_SCALE: [f32; 3] = [1.0; 3];
const DEFAULT_ROTATION: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// One synched data entry: kept when it differs from its default, or always
/// with `all` (the defaults a client is sent when a value goes back to one).
fn entry(
    out: &mut Vec<(u8, i32, Value)>,
    all: bool,
    id: u8,
    kind: i32,
    value: Value,
    default: Value,
) {
    if all || value != default {
        out.push((id, kind, if all { default } else { value }));
    }
}

/// `Display.readAdditionalSaveData`: the synched data entries (ids 8 to 22)
/// of a saved display, or with `all` their defaults.
pub fn display_data(tag: &Tag, all: bool) -> Vec<(u8, i32, Value)> {
    let int = |key: &str| number(tag.get(key)).map(|v| v as i32);
    let real = |key: &str| float(tag.get(key)).map(|v| v as f32);
    let mut out = Vec::new();
    let mut set = |id: u8, kind: i32, value: Value, default: Value| {
        entry(&mut out, all, id, kind, value, default);
    };
    // The start delay, the transformation duration, the teleport duration (0 to 59).
    let ints = [
        (8, "start_interpolation", 0, i32::MIN, i32::MAX),
        (9, "interpolation_duration", 0, i32::MIN, i32::MAX),
        (10, "teleport_duration", 0, 0, 59),
        (22, "glow_color_override", -1, i32::MIN, i32::MAX),
    ];
    for (id, key, default, low, high) in ints {
        let value = int(key).unwrap_or(default).clamp(low, high);
        set(id, serializer::INT, Value::Int(value), Value::Int(default));
    }
    // `Transformation.EXTENDED_CODEC` (four parts or MATRIX4F), else `Transformation.IDENTITY`.
    let t = tag.get("transformation").and_then(transformation);
    let (translation, left, scale, right) = t.unwrap_or((
        DEFAULT_TRANSLATION,
        DEFAULT_ROTATION,
        DEFAULT_SCALE,
        DEFAULT_ROTATION,
    ));
    let parts = [
        (
            11,
            serializer::VECTOR3,
            translation.to_vec(),
            DEFAULT_TRANSLATION.to_vec(),
        ),
        (
            12,
            serializer::VECTOR3,
            scale.to_vec(),
            DEFAULT_SCALE.to_vec(),
        ),
        (
            13,
            serializer::QUATERNION,
            left.to_vec(),
            DEFAULT_ROTATION.to_vec(),
        ),
        (
            14,
            serializer::QUATERNION,
            right.to_vec(),
            DEFAULT_ROTATION.to_vec(),
        ),
    ];
    for (id, kind, value, default) in parts {
        set(id, kind, Value::Floats(value), Value::Floats(default));
    }
    let billboard = match tag.get("billboard") {
        Some(Tag::String(s)) => match s.to_string_lossy().as_str() {
            "vertical" => 1,
            "horizontal" => 2,
            "center" => 3,
            _ => 0,
        },
        _ => 0,
    };
    set(15, serializer::BYTE, Value::Byte(billboard), Value::Byte(0));
    // `Brightness.pack`: block light in bits 4 to 7, sky light in bits 20 to 23.
    let brightness = tag.get("brightness").and_then(|b| {
        let light = |key: &str| number(b.get(key)).filter(|v| (0..=15).contains(v));
        Some((light("block")? as i32) << 4 | (light("sky")? as i32) << 20)
    });
    set(
        16,
        serializer::INT,
        Value::Int(brightness.unwrap_or(-1)),
        Value::Int(-1),
    );
    for (id, key, default) in [
        (17, "view_range", 1.0),
        (18, "shadow_radius", 0.0),
        (19, "shadow_strength", 1.0),
        (20, "width", 0.0),
        (21, "height", 0.0),
    ] {
        let value = real(key).unwrap_or(default);
        set(
            id,
            serializer::FLOAT,
            Value::Float(value),
            Value::Float(default),
        );
    }
    out
}

/// `TextDisplay.readAdditionalSaveData`: the text, line width, background,
/// opacity and style flags (ids 23 to 27), or with `all` their defaults.
pub fn text_display_data(tag: &Tag, all: bool) -> Vec<(u8, i32, Value)> {
    let int = |key: &str| number(tag.get(key)).map(|v| v as i32);
    let mut out = Vec::new();
    let mut set = |id: u8, kind: i32, value: Value, default: Value| {
        entry(&mut out, all, id, kind, value, default);
    };
    let empty = Tag::String("".into());
    set(
        23,
        serializer::COMPONENT,
        Value::Text(tag.get("text").unwrap_or(&empty).clone()),
        Value::Text(empty.clone()),
    );
    set(
        24,
        serializer::INT,
        Value::Int(int("line_width").unwrap_or(200)),
        Value::Int(200),
    );
    let background = Value::Int(int("background").unwrap_or(0x4000_0000));
    set(25, serializer::INT, background, Value::Int(0x4000_0000));
    let opacity = Value::Byte(int("text_opacity").unwrap_or(-1) as i8);
    set(26, serializer::BYTE, opacity, Value::Byte(-1));
    // Shadow 1, see-through 2, default background 4, then the alignment: left 8, right 16.
    let flag = |key: &str, bit: i8| {
        if int(key).is_some_and(|v| v != 0) {
            bit
        } else {
            0
        }
    };
    let align = match tag.get("alignment") {
        Some(Tag::String(s)) => match s.to_string_lossy().as_str() {
            "left" => 8,
            "right" => 16,
            _ => 0,
        },
        _ => 0,
    };
    let flags = flag("shadow", 1) | flag("see_through", 2) | flag("default_background", 4) | align;
    set(27, serializer::BYTE, Value::Byte(flags), Value::Byte(0));
    out
}

/// `Transformation.EXTENDED_CODEC`'s translation, left rotation, scale and
/// right rotation: all four parts, or a `MATRIX4F` (sixteen floats, row-major)
/// that `new Transformation(matrix)` decomposes (`MatrixUtil.svdDecompose`).
pub fn transformation(tag: &Tag) -> Option<([f32; 3], [f32; 4], [f32; 3], [f32; 4])> {
    use sunrise_assets::math::{
        Mat4, normalize_quaternion, quaternion_from_axis_angle, svd_decompose,
    };
    let list = |tag: Option<&Tag>| -> Option<Vec<f32>> {
        match tag? {
            Tag::List(l) => l.iter().map(|v| float(Some(v)).map(|v| v as f32)).collect(),
            _ => None,
        }
    };
    if let Some(values) = list(Some(tag)) {
        let values: [f32; 16] = values.try_into().ok()?;
        return Some(svd_decompose(&Mat4::from_row_major(values)));
    }
    let vector = |key: &str| -> Option<[f32; 3]> { list(tag.get(key))?.try_into().ok() };
    // `ExtraCodecs.QUATERNIONF`: four components (normalized) or an axis and an
    // angle (`new Quaternionf(AxisAngle4f)`: the angle wrapped, the axis as given).
    let rotation = |key: &str| -> Option<[f32; 4]> {
        let value = tag.get(key)?;
        if let Some(q) = list(Some(value)) {
            return Some(normalize_quaternion(q.try_into().ok()?));
        }
        let angle = float(value.get("angle"))? as f32;
        let axis: [f32; 3] = list(value.get("axis"))?.try_into().ok()?;
        Some(quaternion_from_axis_angle(angle, axis))
    };
    Some((
        vector("translation")?,
        rotation("left_rotation")?,
        vector("scale")?,
        rotation("right_rotation")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt::Compound;

    #[test]
    fn a_matrix_transformation_is_decomposed() {
        let matrix = [
            2.0, 0.0, 0.0, 1.0, 0.0, 3.0, 0.0, -2.0, 0.0, 0.0, 0.5, 0.25, 0.0, 0.0, 0.0, 1.0,
        ];
        let tag = Tag::Compound(
            [("transformation".into(), crate::save::floats(&matrix))]
                .into_iter()
                .collect(),
        );
        let data = display_data(&tag, false);
        let part = |id: u8| data.iter().find(|e| e.0 == id).map(|e| e.2.clone());
        assert_eq!(part(11), Some(Value::Floats(vec![1.0, -2.0, 0.25])));
        assert_eq!(part(12), Some(Value::Floats(vec![2.0, 3.0, 0.5])));
        assert_eq!(part(13), None);
    }
    #[test]
    fn axis_angle_rotations_keep_the_axis_and_wrap_the_angle() {
        let mut rotation = Compound::new();
        rotation.insert(
            "angle".into(),
            Tag::Float((-std::f32::consts::PI).to_bits()),
        );
        rotation.insert("axis".into(), crate::save::floats(&[0.0, 0.0, 2.0]));
        let mut t = Compound::new();
        t.insert("translation".into(), crate::save::floats(&[0.0; 3]));
        t.insert("scale".into(), crate::save::floats(&[1.0; 3]));
        t.insert("left_rotation".into(), Tag::Compound(rotation));
        t.insert(
            "right_rotation".into(),
            crate::save::floats(&[0.0, 0.0, 0.0, 2.0]),
        );
        let (_, left, _, right) = transformation(&Tag::Compound(t)).unwrap();
        assert_eq!(
            left,
            sunrise_assets::math::quaternion_from_axis_angle(
                -std::f32::consts::PI,
                [0.0, 0.0, 2.0]
            )
        );
        assert!(left[2] > 1.9);
        assert_eq!(right, [0.0, 0.0, 0.0, 1.0]);
    }
}
