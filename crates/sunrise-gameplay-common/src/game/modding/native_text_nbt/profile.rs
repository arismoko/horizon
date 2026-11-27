use super::{Error, id, optional, s, string, uuid, uuid_tag};
use mod_api::text::{TextProfile, TextProfileProperty};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_sim::api_text::{valid_profile_name, valid_profile_property};

// ResolvableProfile also reads the PlayerSkin.Patch fields, which have no
// dedicated API member. Only those known fields cross in profile metadata.
fn skin_patch(tag: &Tag) -> Option<Compound> {
    let mut out = Compound::new();
    for key in ["texture", "cape", "elytra"] {
        if let Some(value) = optional(tag, key, id)? {
            out.insert(key.into(), s(&value));
        }
    }
    if let Some(model) = optional(tag, "model", |v| {
        string(v).filter(|s| matches!(s.as_str(), "slim" | "wide"))
    })? {
        out.insert("model".into(), s(&model));
    }
    Some(out)
}

pub(super) fn decode(tag: &Tag) -> Option<TextProfile> {
    let name = |v: &Tag| string(v).filter(|name| valid_profile_name(name));
    let mut profile = TextProfile::default();
    match tag {
        Tag::String(_) => profile.name = Some(name(tag)?),
        Tag::Compound(_) => {
            profile.name = optional(tag, "name", name)?;
            profile.id = optional(tag, "id", uuid)?;
            profile.metadata = Tag::Compound(skin_patch(tag)?);
            match tag.get("properties") {
                // ExtraCodecs.PROPERTY_MAP decodes the old unsigned map first.
                Some(Tag::Compound(map)) if map.len() <= 16 => {
                    for (key, values) in map {
                        let name = key.to_string().ok()?;
                        let Tag::List(values) = values else {
                            return None;
                        };
                        for value in values {
                            let mut property = TextProfileProperty::default();
                            property.name = name.clone();
                            property.value = string(value)?;
                            profile.properties.push(property);
                        }
                    }
                }
                Some(Tag::List(values)) if values.len() <= 16 => {
                    for value in values {
                        let mut property = TextProfileProperty::default();
                        property.name = string(value.get("name")?)?;
                        property.value = string(value.get("value")?)?;
                        property.signature = optional(value, "signature", string)?;
                        if !valid_profile_property(
                            &property.name,
                            &property.value,
                            property.signature.as_deref(),
                        ) {
                            return None;
                        }
                        profile.properties.push(property);
                    }
                }
                Some(Tag::ByteArray(a)) if a.is_empty() => {}
                Some(Tag::IntArray(a)) if a.is_empty() => {}
                Some(Tag::LongArray(a)) if a.is_empty() => {}
                None => {}
                _ => return None,
            }
        }
        _ => return None,
    }
    Some(profile)
}

pub(super) fn encode(profile: &TextProfile) -> Result<Tag, Error> {
    let mut fields = skin_patch(&profile.metadata).ok_or(Error::Malformed)?;
    if let Some(name) = &profile.name {
        if !valid_profile_name(name) {
            return Err(Error::Malformed);
        }
        fields.insert("name".into(), s(name));
    }
    if let Some(id) = profile.id {
        fields.insert("id".into(), uuid_tag(id));
    }
    if !profile.properties.is_empty() {
        if profile.properties.len() > 16 {
            return Err(Error::Malformed);
        }
        // The encoder's right branch is always the property-record list.
        // PropertyMap is an ordered multimap: group records by first name occurrence.
        let mut groups: Vec<(&str, Vec<Tag>)> = Vec::new();
        for property in &profile.properties {
            let (name, value, signature) = (
                &property.name,
                &property.value,
                property.signature.as_deref(),
            );
            if !valid_profile_property(name, value, signature) {
                return Err(Error::Malformed);
            }
            let record = Tag::compound(
                [("name", s(name)), ("value", s(value))]
                    .into_iter()
                    .chain(signature.map(|signature| ("signature", s(signature)))),
            );
            if let Some((_, group)) = groups.iter_mut().find(|(key, _)| *key == name) {
                group.push(record);
            } else {
                groups.push((name, vec![record]));
            }
        }
        fields.insert(
            "properties".into(),
            Tag::List(groups.into_iter().flat_map(|(_, group)| group).collect()),
        );
    }
    Ok(Tag::Compound(fields))
}
