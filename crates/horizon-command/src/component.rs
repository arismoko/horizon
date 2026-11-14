//! ComponentSerialization.CODEC and Style.Serializer.MAP_CODEC decoding of
//! SNBT text components, with the data-fixer-upper error messages
//! (ComponentArgument, StyleArgument and component-valued item data).
use super::codec::{get, no_key, not_list, not_map};
use horizon_client_core::{command, registries::Registries};
use horizon_core::nbt::{Compound, Tag};

/// A DataResult error: its message and whether it carries a partial result.
struct E {
    msg: String,
    partial: bool,
}
type R = Result<(), E>;
fn err(msg: impl Into<String>) -> R {
    Err(E {
        msg: msg.into(),
        partial: false,
    })
}
fn partial(msg: impl Into<String>) -> R {
    Err(E {
        msg: msg.into(),
        partial: true,
    })
}
/// Codec.either.
fn either(first: R, second: impl FnOnce() -> R) -> R {
    let a = match first {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    let b = match second() {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    if a.partial {
        return Err(a);
    }
    if b.partial {
        return Err(b);
    }
    err(format!(
        "Failed to parse either. First: {}; Second: {}",
        a.msg, b.msg
    ))
}
fn is_string(t: &Tag) -> bool {
    matches!(t, Tag::String(_))
}
fn string(t: &Tag) -> R {
    if is_string(t) {
        Ok(())
    } else {
        err("Not a string")
    }
}
fn string_of(t: &Tag) -> Option<String> {
    match t {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}
/// ListCodec: element errors (newest first) with the decoded rest as partial.
fn list(t: &Tag, element: &dyn Fn(&Tag) -> R) -> Result<usize, E> {
    let Tag::List(items) = t else {
        return Err(E {
            msg: not_list(t),
            partial: false,
        });
    };
    let mut errors: Vec<String> = Vec::new();
    let mut ok = 0;
    for i in items {
        match element(i) {
            Ok(()) => ok += 1,
            Err(e) => {
                if e.partial {
                    ok += 1;
                }
                errors.push(e.msg);
            }
        }
    }
    if errors.is_empty() {
        return Ok(ok);
    }
    errors.reverse();
    Err(E {
        msg: errors.join("; "),
        partial: true,
    })
}
/// ExtraCodecs.nonEmptyList(listOf(element)).
fn non_empty_list(t: &Tag, element: &dyn Fn(&Tag) -> R) -> R {
    match list(t, element) {
        Ok(0) => err("List must have contents"),
        Ok(_) => Ok(()),
        Err(e) if !e.partial => Err(e),
        Err(e) => {
            // The partial result is validated too.
            let decoded = match t {
                Tag::List(items) => items.iter().filter(|i| element(i).is_ok()).count(),
                _ => 0,
            };
            if decoded == 0 {
                err(format!("{}; List must have contents", e.msg))
            } else {
                Err(e)
            }
        }
    }
}
fn bool_value(t: &Tag) -> R {
    if super::codec::is_number(t) {
        Ok(())
    } else {
        err("Not a number")
    }
}
/// Identifier.CODEC.
pub fn identifier(t: &Tag) -> Result<(), String> {
    let Some(s) = string_of(t) else {
        return Err("Not a string".into());
    };
    let (ns, path) = match s.split_once(':') {
        Some(("", p)) => ("minecraft".to_owned(), p.to_owned()),
        Some((n, p)) => (n.to_owned(), p.to_owned()),
        None => ("minecraft".to_owned(), s.clone()),
    };
    let ns_ok = ns != ".."
        && ns
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-'));
    if !ns_ok {
        return Err(format!(
            "Not a valid resource location: {s} Non [a-z0-9_.-] character in namespace of location: {ns}:{path}"
        ));
    }
    let path_ok = path.chars().all(|c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-' | '/')
    });
    if !path_ok {
        return Err(format!(
            "Not a valid resource location: {s} Non [a-z0-9/._-] character in path of location: {ns}:{path}"
        ));
    }
    Ok(())
}
/// TextColor.CODEC.
fn color(t: &Tag) -> R {
    let Some(s) = string_of(t) else {
        return partial("Not a string");
    };
    if let Some(hex) = s.strip_prefix('#') {
        let digits = hex.strip_prefix(['+', '-']).unwrap_or(hex);
        let valid = !digits.is_empty() && digits.chars().all(|c| c.is_ascii_hexdigit());
        return match i64::from_str_radix(hex, 16) {
            Ok(v) if valid && i32::try_from(v).is_ok() => {
                if (0..=0xFF_FFFF).contains(&v) {
                    Ok(())
                } else {
                    partial(format!("Color value out of range: {s}"))
                }
            }
            _ => partial(format!("Invalid color value: {s}")),
        };
    }
    if horizon_client_core::text::named_color(&s).is_some() {
        Ok(())
    } else {
        partial(format!("Invalid color name: {s}"))
    }
}
/// A KeyDispatch over an action enum (`Unknown element name:`), with
/// required fields per action.
fn action_event(t: &Tag, actions: &[(&str, &[&str])]) -> R {
    let Tag::Compound(map) = t else {
        return partial(not_map(t));
    };
    let Some(action) = get(map, "action") else {
        return partial(no_key("action", t));
    };
    let Some(name) = string_of(action) else {
        return partial("Not a string");
    };
    let Some((_, fields)) = actions.iter().find(|(a, _)| *a == name) else {
        return partial(format!("Unknown element name:{name}"));
    };
    for f in *fields {
        if get(map, f).is_none() {
            return partial(no_key(f, t));
        }
    }
    Ok(())
}
const CLICK_ACTIONS: &[(&str, &[&str])] = &[
    ("open_url", &["url"]),
    ("open_file", &["path"]),
    ("run_command", &["command"]),
    ("suggest_command", &["command"]),
    ("show_dialog", &["dialog"]),
    ("change_page", &["page"]),
    ("copy_to_clipboard", &["value"]),
    ("custom", &["id"]),
];
const HOVER_ACTIONS: &[(&str, &[&str])] = &[
    ("show_text", &["value"]),
    ("show_item", &["id"]),
    ("show_entity", &["id", "uuid"]),
];
/// Style.Serializer.MAP_CODEC (every field optional; errors keep a partial style).
fn style_fields(map: &Compound) -> R {
    if let Some(c) = get(map, "color") {
        color(c)?;
    }
    for key in [
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
    ] {
        if let Some(v) = get(map, key)
            && bool_value(v).is_err()
        {
            return partial("Not a number");
        }
    }
    if let Some(v) = get(map, "click_event") {
        action_event(v, CLICK_ACTIONS)?;
    }
    if let Some(v) = get(map, "hover_event") {
        action_event(v, HOVER_ACTIONS)?;
    }
    if let Some(v) = get(map, "insertion")
        && !is_string(v)
    {
        return partial("Not a string");
    }
    if let Some(v) = get(map, "font")
        && let Err(m) = identifier(v)
    {
        return partial(m);
    }
    Ok(())
}
fn selector_string(t: &Tag, reg: &Registries) -> bool {
    let Tag::String(source) = t else {
        return false;
    };
    command::selector::parse_utf16(&source.0, reg).is_ok()
}
fn optional_component(map: &Compound, key: &str, reg: &Registries) -> bool {
    get(map, key).is_none_or(|v| component(v, reg).is_ok())
}
/// The content codecs (PlainText, Translatable, Keybind, Score, Selector, Nbt, Object).
fn content(kind: &str, map: &Compound, reg: &Registries) -> bool {
    match kind {
        "text" => get(map, "text").is_some_and(is_string),
        "translatable" => {
            get(map, "translate").is_some_and(is_string)
                && get(map, "fallback").is_none_or(is_string)
                && get(map, "with").is_none_or(|w| matches!(w, Tag::List(_)))
        }
        "keybind" => get(map, "keybind").is_some_and(is_string),
        "score" => match get(map, "score") {
            Some(Tag::Compound(s)) => {
                get(s, "name").is_some_and(is_string) && get(s, "objective").is_some_and(is_string)
            }
            _ => false,
        },
        "selector" => {
            get(map, "selector").is_some_and(|v| selector_string(v, reg))
                && optional_component(map, "separator", reg)
        }
        "nbt" => {
            let path_ok = get(map, "nbt").is_some_and(|value| match value {
                Tag::String(path) => {
                    horizon_client_core::nbt_path::NbtPath::parse_utf16(&path.0).is_ok()
                }
                _ => false,
            });
            let source = if let Some(e) = get(map, "entity") {
                selector_string(e, reg)
            } else if let Some(b) = get(map, "block") {
                match b {
                    Tag::String(source) => command::parse_block_pos_utf16(&source.0).is_ok(),
                    _ => false,
                }
            } else if let Some(s) = get(map, "storage") {
                identifier(s).is_ok()
            } else {
                false
            };
            path_ok && source && optional_component(map, "separator", reg)
        }
        "object" => {
            let kind = get(map, "object").and_then(string_of);
            let atlas = || {
                get(map, "sprite").is_some_and(|s| identifier(s).is_ok())
                    && get(map, "atlas").is_none_or(|a| identifier(a).is_ok())
            };
            let player = || get(map, "player").is_some();
            match kind.as_deref() {
                Some("atlas") => atlas(),
                Some("player") => player(),
                Some(_) => false,
                None => atlas() || player(),
            }
        }
        _ => false,
    }
}
const CONTENT_TYPES: [&str; 7] = [
    "text",
    "translatable",
    "keybind",
    "score",
    "selector",
    "nbt",
    "object",
];
fn full(t: &Tag, reg: &Registries) -> R {
    let Tag::Compound(map) = t else {
        return err(not_map(t));
    };
    let contents_ok = if let Some(ty) = get(map, "type") {
        let Some(name) = string_of(ty) else {
            return err("Not a string");
        };
        if !CONTENT_TYPES.contains(&name.as_str()) {
            return err(format!("Unknown element id: {name}"));
        }
        content(&name, map, reg)
    } else {
        CONTENT_TYPES.iter().any(|k| content(k, map, reg))
    };
    if !contents_ok {
        return err("No matching codec found");
    }
    if let Some(extra) = get(map, "extra") {
        match non_empty_list(extra, &|v| component(v, reg)) {
            Ok(()) => {}
            Err(e) => return partial(e.msg),
        }
    }
    style_fields(map)
}
fn component(t: &Tag, reg: &Registries) -> R {
    either(
        either(string(t), || non_empty_list(t, &|v| component(v, reg))),
        || full(t, reg),
    )
}
/// ComponentSerialization.CODEC.parse: the error message on failure.
pub fn decode(t: &Tag, reg: &Registries) -> Result<(), String> {
    component(t, reg).map_err(|e| e.msg)
}
/// Style.Serializer.CODEC.parse.
pub fn decode_style(t: &Tag) -> Result<(), String> {
    match t {
        Tag::Compound(map) => style_fields(map).map_err(|e| e.msg),
        _ => Err(not_map(t)),
    }
}

#[cfg(test)]
mod lossless_source_tests {
    use super::*;
    use horizon_core::nbt::JavaString;
    fn registries() -> Registries {
        Registries {
            permission: 4,
            entity_types: vec!["minecraft:pig".into()],
            ..Registries::default()
        }
    }
    #[test]
    fn persistent_component_sources_accept_raw_java_surrogates() {
        let selector = JavaString(vec![64, 101, 91, 110, 97, 109, 101, 61, 34, 0xd800, 34, 93]);
        assert!(
            decode(
                &Tag::compound([("selector", Tag::String(selector.clone()))]),
                &registries()
            )
            .is_ok()
        );
        let nbt = Tag::compound([
            ("nbt", Tag::String(JavaString(vec![34, 0xdfff, 34]))),
            ("entity", Tag::String(selector)),
        ]);
        assert!(decode(&nbt, &registries()).is_ok());
        let block = Tag::compound([
            ("nbt", Tag::String(JavaString(vec![34, 0xd800, 34]))),
            ("block", Tag::String("~ ~ ~".into())),
        ]);
        assert!(decode(&block, &registries()).is_ok());
    }
    #[test]
    fn persistent_component_sources_require_complete_grammar_and_real_registry_ids() {
        let reg = registries();
        for source in [
            "@e[type=missing]",
            "@e[] trailing",
            "@e[name=\"bad\\q\"]",
            "@e[distance=3..1]",
        ] {
            assert!(
                decode(
                    &Tag::compound([("selector", Tag::String(source.into()))]),
                    &reg
                )
                .is_err(),
                "{source}"
            );
        }
        for (path, position) in [
            (
                JavaString(vec![34, 0xd800, 34, 32]),
                JavaString::from("~ ~ ~"),
            ),
            (JavaString::from("a"), JavaString::from("~ ~ ~ trailing")),
            (
                JavaString::from("a"),
                JavaString(vec![48, 32, 48, 32, 0xd800]),
            ),
        ] {
            assert!(
                decode(
                    &Tag::compound([("nbt", Tag::String(path)), ("block", Tag::String(position))]),
                    &reg
                )
                .is_err()
            );
        }
    }
}
