//! The component grammar of `ComponentSerialization.CODEC`, as `text_from_json` reads it: a
//! component is checked and rebuilt in one pass, so what the text capture sees is exactly
//! the value the codec would have decoded, never a partial or ambiguous one.
use crate::command::Registries;
use crate::component;
use serde_json::{Map, Value, json};
use sunrise_core::identifier::Identifier;

/// Content kinds in the order the codec tries them without a `type`.
pub const KINDS: [&str; 7] = [
    "text",
    "translatable",
    "keybind",
    "score",
    "selector",
    "nbt",
    "object",
];
const STYLE_FLAGS: [&str; 5] = [
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
];

/// `value` as the codec decodes it: a string; a non-empty list of components; an object with
/// one content (the one its `type` names, else the first kind its keys satisfy) plus
/// well-formed style fields and `extra`, holding only the keys that decode. None when it
/// does not decode.
pub fn build(value: &Value, reg: &Registries, snbt: bool) -> Option<Value> {
    build_component(
        value,
        &Grammar {
            reg,
            snbt,
            dialog_item: None,
        },
    )
}

/// A dialog supplies the click codec (with its registry-aware holder lookup).
/// Keep its click fields for that decoder, without first converting an inline
/// holder from JSON to NBT and changing its scalar types.
pub fn build_for_dialog(
    value: &Value,
    reg: &Registries,
    snbt: bool,
    item: &dyn Fn(&str) -> bool,
) -> Option<Value> {
    build_component(
        value,
        &Grammar {
            reg,
            snbt,
            dialog_item: Some(item),
        },
    )
}

struct Grammar<'a> {
    reg: &'a Registries,
    snbt: bool,
    dialog_item: Option<&'a dyn Fn(&str) -> bool>,
}

fn build_component(value: &Value, ctx: &Grammar<'_>) -> Option<Value> {
    match value {
        Value::String(_) => Some(value.clone()),
        Value::Array(items) if !items.is_empty() => items
            .iter()
            .map(|item| build_component(item, ctx))
            .collect::<Option<_>>()
            .map(Value::Array),
        Value::Object(fields) => build_object(fields, ctx).map(Value::Object),
        _ => None,
    }
}

fn build_object(fields: &Map<String, Value>, ctx: &Grammar<'_>) -> Option<Map<String, Value>> {
    // A null `type` is no `type`; any other non-string refuses.
    let mut out = match fields.get("type") {
        None | Some(Value::Null) => KINDS.iter().find_map(|kind| content(kind, fields, ctx))?,
        Some(Value::String(kind)) => content(kind, fields, ctx)?,
        Some(_) => return None,
    };
    let boolean = |value: &Value| {
        if ctx.snbt && ctx.dialog_item.is_some() {
            // NbtOps.getBooleanValue reads a NumericTag's boolean value.
            value.as_f64().map(|n| n != 0.0)
        } else if ctx.snbt {
            component::style_flag(Some(value))
        } else {
            value.as_bool()
        }
    };
    for flag in STYLE_FLAGS {
        if let Some(v) = present(fields, flag) {
            out.insert(flag.into(), boolean(v)?.into());
        }
    }
    if let Some(v) = present(fields, "color") {
        let name = v.as_str()?;
        let color = if name.starts_with('#') {
            let rgb = component::parse_color(name)?;
            format!("#{rgb:06X}")
        } else {
            crate::text::named_color(name)?;
            name.to_owned()
        };
        out.insert("color".into(), color.into());
    }
    if let Some(v) = present(fields, "shadow_color") {
        out.insert(
            "shadow_color".into(),
            Value::from(component::shadow_color(v)? as i32),
        );
    }
    if let Some(v) = present(fields, "font") {
        out.insert("font".into(), crate::text::font_id(v)?.into());
    }
    if let Some(v) = present(fields, "insertion") {
        out.insert("insertion".into(), v.as_str()?.into());
    }
    if let Some(v) = present(fields, "click_event") {
        let event = v.as_object()?;
        use crate::text::ClickEvent;
        let decoded = if ctx.dialog_item.is_some() {
            // The dialog reader takes this field from the original source.
            Value::Object(Map::new())
        } else {
            match component::parse_click_event(event).ok()?? {
                ClickEvent::OpenUrl(url) => json!({"action":"open_url","url":url}),
                ClickEvent::ChangePage(page) => json!({"action":"change_page","page":page}),
                ClickEvent::SuggestCommand(command) => {
                    json!({"action":"suggest_command","command":command})
                }
                ClickEvent::RunCommand(command) => {
                    json!({"action":"run_command","command":command})
                }
                ClickEvent::CopyToClipboard(value) => {
                    json!({"action":"copy_to_clipboard","value":value})
                }
                // The decoder already checked these opaque NBT values. Retain the JSON form so
                // JsonOps booleans do not lose their identity in an NbtOps→JsonOps round trip.
                ClickEvent::ShowDialog(_) => {
                    json!({"action":"show_dialog","dialog":event["dialog"]})
                }
                ClickEvent::Custom { id, payload } => {
                    let mut decoded = json!({"action":"custom","id":id});
                    if payload.is_some() {
                        decoded["payload"] = event["payload"].clone();
                    }
                    decoded
                }
                ClickEvent::OpenFile(_) | ClickEvent::Profiler(_) => return None,
            }
        };
        out.insert("click_event".into(), decoded);
    }
    if let Some(v) = present(fields, "hover_event") {
        out.insert("hover_event".into(), hover(v.as_object()?, ctx)?);
    }
    if let Some(v) = present(fields, "extra") {
        let items = v.as_array().filter(|items| !items.is_empty())?;
        let built = items
            .iter()
            .map(|i| build_component(i, ctx))
            .collect::<Option<_>>()?;
        out.insert("extra".into(), Value::Array(built));
    }
    Some(out)
}

/// `HoverEvent.CODEC`'s three actions; a `show_text` value is itself a component.
fn hover(event: &Map<String, Value>, ctx: &Grammar<'_>) -> Option<Value> {
    let action = event.get("action")?.as_str()?;
    let out = match action {
        "show_text" => json!({"action":action,"value":build_component(event.get("value")?, ctx)?}),
        "show_item" => {
            let id = Identifier::parse(event.get("id")?.as_str()?)
                .ok()?
                .to_string();
            let registered = ctx.dialog_item.map_or_else(
                || ctx.reg.items.is_empty() || ctx.reg.items.contains(&id),
                |item| item(&id),
            );
            if id == "minecraft:air" || !registered {
                return None;
            }
            let mut out = json!({"action":action,"id":id});
            if let Some(v) = present(event, "count") {
                let count = v.as_i64().filter(|n| (1..=99).contains(n))?;
                if count != 1 {
                    out["count"] = count.into();
                }
            }
            if let Some(v) = present(event, "components") {
                let patch = v.as_object()?;
                let mut normalized = Map::new();
                for (key, value) in patch {
                    if let Some(removed) = key.strip_prefix('!') {
                        value.as_object()?;
                        let id = Identifier::parse(removed).ok()?;
                        normalized.insert(format!("!{id}"), json!({}));
                    } else {
                        let id = Identifier::parse(key).ok()?;
                        normalized.insert(id.to_string(), value.clone());
                    }
                }
                if !normalized.is_empty() {
                    out["components"] = normalized.into();
                }
            }
            out
        }
        "show_entity" => {
            let id = Identifier::parse(event.get("id")?.as_str()?)
                .ok()?
                .to_string();
            if !ctx.reg.entity_types.is_empty() && !ctx.reg.entity_types.contains(&id) {
                return None;
            }
            let uuid = match event.get("uuid")? {
                Value::String(id) => sunrise_game_data::uuid::uuid_from_text(id)?,
                Value::Array(words) if words.len() == 4 => {
                    let mut out = [0; 4];
                    for (slot, word) in out.iter_mut().zip(words) {
                        *slot = i32::try_from(word.as_i64()?).ok()?;
                    }
                    out
                }
                _ => return None,
            };
            let mut out = json!({"action":action,"id":id,"uuid":uuid});
            if let Some(name) = present(event, "name") {
                out["name"] = build_component(name, ctx)?;
            }
            out
        }
        _ => return None,
    };
    Some(out)
}

/// One content kind of `fields` alone, as its `MapCodec` decodes it: None when it does not.
fn content(
    kind: &str,
    fields: &Map<String, Value>,
    ctx: &Grammar<'_>,
) -> Option<Map<String, Value>> {
    let string = |key: &str| fields.get(key).and_then(Value::as_str);
    let mut out = Map::new();
    let keep = |out: &mut Map<String, Value>, key: &str| {
        out.insert(key.into(), fields.get(key).cloned().unwrap_or(Value::Null));
    };
    match kind {
        "text" => {
            // A string only: a number is not coerced.
            string("text")?;
            keep(&mut out, "text");
        }
        "translatable" => {
            string("translate")?;
            keep(&mut out, "translate");
            // `fallback` is lenient: a wrong kind is dropped.
            if string("fallback").is_some() {
                keep(&mut out, "fallback");
            }
            if let Some(with) = present(fields, "with") {
                let args = with.as_array()?;
                let built = args
                    .iter()
                    .map(|arg| {
                        build_component(arg, ctx)
                            .or_else(|| (arg.is_number() || arg.is_boolean()).then(|| arg.clone()))
                    })
                    .collect::<Option<_>>()?;
                out.insert("with".into(), Value::Array(built));
            }
        }
        "keybind" => {
            string("keybind")?;
            keep(&mut out, "keybind");
        }
        "score" => {
            let score = fields.get("score")?.as_object()?;
            out.insert(
                "score".into(),
                json!({
                    "name": score.get("name")?.as_str()?,
                    "objective": score.get("objective")?.as_str()?,
                }),
            );
        }
        "selector" => {
            selector_parses(string("selector")?, ctx.reg).then_some(())?;
            keep(&mut out, "selector");
            // A selector's `separator` stays strict.
            if let Some(separator) = present(fields, "separator") {
                out.insert("separator".into(), build_component(separator, ctx)?);
            }
        }
        "nbt" => {
            path_parses(string("nbt")?).then_some(())?;
            keep(&mut out, "nbt");
            let source = match present(fields, "source") {
                Some(Value::String(source)) => data_source(source, fields, ctx.reg)?,
                Some(_) => return None,
                None => ["entity", "block", "storage"]
                    .into_iter()
                    .find_map(|key| data_source(key, fields, ctx.reg))?,
            };
            out.insert(source.0.into(), source.1.into());
            // `interpret`, `plain` and `separator` are lenient: a wrong kind is dropped.
            let flag = |key: &str| {
                present(fields, key)
                    .and_then(|v| {
                        if ctx.snbt {
                            component::style_flag(Some(v))
                        } else {
                            v.as_bool()
                        }
                    })
                    .unwrap_or(false)
            };
            let (interpret, plain) = (flag("interpret"), flag("plain"));
            if interpret && plain {
                return None;
            }
            for (key, value) in [("interpret", interpret), ("plain", plain)] {
                if value {
                    out.insert(key.into(), true.into());
                }
            }
            if let Some(separator) =
                present(fields, "separator").and_then(|s| build_component(s, ctx))
            {
                out.insert("separator".into(), separator);
            }
        }
        "object" => {
            let atlas = || {
                let sprite = Identifier::parse(string("sprite")?).ok()?.to_string();
                let mut out = Map::from_iter([
                    ("object".into(), "atlas".into()),
                    ("sprite".into(), sprite.into()),
                ]);
                if let Some(value) = present(fields, "atlas") {
                    let atlas = Identifier::parse(value.as_str()?).ok()?.to_string();
                    if atlas != "minecraft:blocks" {
                        out.insert("atlas".into(), atlas.into());
                    }
                }
                Some(out)
            };
            let player = || {
                let profile = crate::api_text::decoded_profile_json(fields.get("player")?)?;
                let mut out = Map::from_iter([
                    ("object".into(), "player".into()),
                    ("player".into(), profile),
                ]);
                if let Some(hat) = present(fields, "hat") {
                    let hat = if ctx.snbt {
                        component::style_flag(Some(hat))
                    } else {
                        hat.as_bool()
                    }?;
                    if !hat {
                        out.insert("hat".into(), false.into());
                    }
                }
                Some(out)
            };
            out = match present(fields, "object") {
                None => atlas().or_else(player),
                Some(value) => match value.as_str()? {
                    "atlas" => atlas(),
                    "player" => player(),
                    _ => return None,
                },
            }?;
            if let Some(fallback) = present(fields, "fallback") {
                out.insert("fallback".into(), build_component(fallback, ctx)?);
            }
        }
        _ => return None,
    }
    Some(out)
}

/// A field that is there and not null: null counts as absent.
fn present<'a>(fields: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    fields.get(key).filter(|v| !v.is_null())
}

/// The data source `source` (`entity`, `block` or `storage`) as `fields` holds it: the key
/// when its value is valid (a selector, a position string, a storage identifier).
fn data_source(
    source: &str,
    fields: &Map<String, Value>,
    reg: &Registries,
) -> Option<(&'static str, String)> {
    let value = fields.get(source)?.as_str()?;
    match source {
        "entity" => selector_parses(value, reg).then(|| ("entity", value.into())),
        "block" => block_pos_parses(value, reg).then(|| ("block", value.into())),
        "storage" => Some(("storage", Identifier::parse(value).ok()?.to_string())),
        _ => None,
    }
}

/// `BlockPosArgument.parse` on all of `source`.
pub fn block_pos_parses(source: &str, reg: &Registries) -> bool {
    let mut reader = crate::command::Reader::new(source);
    crate::command::ArgType::BlockPos
        .parse(&mut reader, reg)
        .is_ok()
        && !reader.can_read()
}

/// `EntitySelectorParser.parse` on all of `source`.
pub fn selector_parses(source: &str, reg: &Registries) -> bool {
    let mut reader = crate::command::Reader::new(source);
    crate::command::selector::parse(&mut reader, reg, true).is_ok() && !reader.can_read()
}

/// `NbtPathArgument.parse` on all of `path`.
pub fn path_parses(path: &str) -> bool {
    crate::nbt_path::NbtPath::parse_prefix(path).is_ok_and(|(_, used)| used == path.len())
}
