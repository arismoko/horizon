// Included in entity_nbt.rs to retain its private scope.
/// Saved data carried by `ConversionType.convertCommon` outside the modelled
/// mob fields. The converted entity has its own identity and type-specific data;
/// only its command tags are inherited from the old entity's raw compound.
pub fn conversion_tags(raw: Option<&Tag>) -> Option<Tag> {
    let Tag::List(tags) = raw?.get("Tags")? else {
        return None;
    };
    (!tags.is_empty()).then(|| Tag::compound([("Tags", Tag::List(tags.clone()))]))
}

fn string(s: &str) -> Tag {
    Tag::String(s.into())
}

fn hidden_effect_nbt(effect: &crate::effects::EffectInstance) -> Tag {
    let mut fields = vec![("duration", Tag::Int(effect.duration))];
    if effect.amplifier != 0 {
        fields.push((
            "amplifier",
            Tag::Byte(effect.amplifier.clamp(0, 255) as u8 as i8),
        ));
    }
    if effect.ambient {
        fields.push(("ambient", Tag::Byte(1)));
    }
    if !effect.visible {
        fields.push(("show_particles", Tag::Byte(0)));
    }
    fields.push(("show_icon", Tag::Byte(i8::from(effect.show_icon))));
    if let Some(hidden) = &effect.hidden {
        fields.push(("hidden_effect", hidden_effect_nbt(hidden)));
    }
    Tag::compound(fields)
}

/// `MobEffectInstance.Details`' `amplifier`, `ExtraCodecs.UNSIGNED_BYTE` (default 0): Codec.BYTE
/// takes the stored number's `byteValue()`, read as 0..255, so `-56b` is 200 and an int `256` is
/// 0. The same codec reads an active effect and every hidden effect of its chain.
fn effect_amplifier(tag: &Tag) -> i32 {
    tag.get("amplifier")
        .and_then(sunrise_world::datafix::dynamic::as_number)
        .map_or(0, |n| i32::from(n.byte_value() as u8))
}

fn hidden_effect_from_nbt(effect: &'static str, tag: &Tag) -> crate::effects::EffectInstance {
    let mut instance = crate::effects::EffectInstance::new(
        effect,
        number(tag.get("duration")).unwrap_or(0) as i32,
        effect_amplifier(tag),
    );
    instance.ambient = number(tag.get("ambient")).is_some_and(|v| v != 0);
    instance.visible = number(tag.get("show_particles")) != Some(0);
    instance.show_icon = number(tag.get("show_icon")).map_or(instance.visible, |v| v != 0);
    instance.hidden = tag
        .get("hidden_effect")
        .map(|hidden| Box::new(hidden_effect_from_nbt(effect, hidden)));
    instance
}
/// Entity type ID (`id`).
pub fn kind(tag: &Tag) -> Option<String> {
    sunrise_game_data::identifier::normalize_id(&text(tag.get("id"))?)
}
pub fn position(tag: &Tag) -> Option<Vec3> {
    read_doubles::<3>(tag.get("Pos")).map(|p| Vec3::new(p[0], p[1], p[2]))
}
pub fn uuid(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("UUID"))
}
/// `UUIDUtil.CODEC` through `NbtOps.getIntStream`: a 4-element numeric array or
/// list (`Entity.load` ignores quoted UUID strings and keeps the constructor UUID).
pub fn entity_uuid(tag: &Tag) -> Option<[i32; 4]> {
    match tag.get("UUID")? {
        Tag::IntArray(v) if v.len() == 4 => Some([v[0], v[1], v[2], v[3]]),
        Tag::ByteArray(v) if v.len() == 4 => Some([
            i32::from(v[0] as i8),
            i32::from(v[1] as i8),
            i32::from(v[2] as i8),
            i32::from(v[3] as i8),
        ]),
        Tag::LongArray(v) if v.len() == 4 => {
            Some([v[0] as i32, v[1] as i32, v[2] as i32, v[3] as i32])
        }
        Tag::List(v) if v.len() == 4 => {
            let int = |t: &Tag| match t {
                Tag::Float(v) => Some(f32::from_bits(*v) as i32),
                Tag::Double(v) => Some(f64::from_bits(*v) as i32),
                other => number(Some(other)).map(|v| v as i32),
            };
            Some([int(&v[0])?, int(&v[1])?, int(&v[2])?, int(&v[3])?])
        }
        _ => None,
    }
}
pub fn motion(tag: &Tag) -> Vec3 {
    read_doubles::<3>(tag.get("Motion")).map_or(Vec3::ZERO, |m| Vec3::new(m[0], m[1], m[2]))
}
/// A saved Motion as `Entity.load` takes it: a component faster than 10 blocks
/// a tick loads as 0.
pub fn loaded_motion(tag: &Tag) -> Vec3 {
    let m = motion(tag);
    let clamp = |v: f64| if v.abs() > 10.0 { 0.0 } else { v };
    Vec3::new(clamp(m.x), clamp(m.y), clamp(m.z))
}
pub fn rotation(tag: &Tag) -> [f32; 2] {
    read_doubles::<2>(tag.get("Rotation")).map_or([0.0; 2], |r| [r[0] as f32, r[1] as f32])
}
/// `Entity.load`: the initial rotation on a new entity, before its subtype reads NBT.
pub fn loaded_rotation(tag: &Tag) -> [f32; 2] {
    let [yaw, pitch] = rotation(tag).map(|angle| {
        if angle.is_finite() {
            angle % 360.0
        } else {
            0.0
        }
    });
    [yaw, pitch.clamp(-90.0, 90.0)]
}
/// `TagValueInput.getBooleanOr`: a numeric tag's byteValue is nonzero, with a false default.
pub fn flag(tag: &Tag, key: &str) -> bool {
    flag_or(tag, key, false)
}
/// `getBooleanOr(key, default)`: any numeric tag's nonzero test, else `default`.
pub fn flag_or(tag: &Tag, key: &str, default: bool) -> bool {
    // NBT floats floor before int narrowing; the final byte keeps the low bits.
    tag.get(key)
        .and_then(int_value)
        .map_or(default, |value| value as i8 != 0)
}
/// `Entity.load`: `input.getShortOr("Fire", 0)` via vanilla's numeric tag short conversion.
pub fn fire_ticks_from_nbt(tag: &Tag) -> i32 {
    i32::from(tag.get("Fire").and_then(int_value).unwrap_or(0) as i16)
}

/// `Entity.saveWithoutId`: write the movement flags and the flags retained
/// from `Entity.load` as booleans. Only OnGround and Invulnerable write false.
pub fn write_saved_flags(tag: &mut Tag, on_ground: bool, no_gravity: bool) {
    let invulnerable = flag(tag, "Invulnerable");
    let optional = [
        ("NoGravity", no_gravity),
        ("CustomNameVisible", flag(tag, "CustomNameVisible")),
        ("Silent", flag(tag, "Silent")),
        ("Glowing", flag(tag, "Glowing")),
        ("HasVisualFire", flag(tag, "HasVisualFire")),
    ];
    let Tag::Compound(map) = tag else {
        return;
    };
    map.insert("OnGround".into(), Tag::Byte(i8::from(on_ground)));
    map.insert("Invulnerable".into(), Tag::Byte(i8::from(invulnerable)));
    for (key, value) in optional {
        let key = key.into();
        if value {
            map.insert(key, Tag::Byte(1));
        } else {
            map.remove(&key);
        }
    }
}
