#[path = "api_text/style_eq.rs"]
mod style_eq;
#[path = "api_text/text_style_capture.rs"]
mod text_style_capture;
use mod_api::text::{
    Text, TextClick, TextContent, TextFormError, TextHover, TextNbtSource, TextObject, TextProfile,
    TextProfileProperty, TextStyle,
};
use serde_json::{Map, Value};
use std::collections::HashMap;
use sunrise_core::{
    json_nbt,
    nbt::{self, Tag},
};
use sunrise_game_data::{component, text as engine};
pub use text_style_capture::{style_to_api, style_to_api_inprocess};

type Error = TextFormError;
type Key = (u8, usize);
#[derive(Clone, Copy)]
enum Source<'a> {
    Engine(&'a engine::Text),
    Span(&'a engine::Span),
    Json(&'a Value),
    Str(&'a str),
}
impl Source<'_> {
    fn key(self) -> Key {
        match self {
            Self::Engine(v) => (0, v as *const _ as usize),
            Self::Span(v) => (1, v as *const _ as usize),
            Self::Json(v) => (2, v as *const _ as usize),
            Self::Str(v) => (3, v.as_ptr() as usize),
        }
    }
}
#[derive(Clone, Copy)]
enum NbtSource<'a> {
    Block(&'a str),
    Entity(&'a str),
    Storage(&'a str),
}
enum Content<'a> {
    Literal(&'a str),
    Scalar(&'a Value),
    Translate {
        key: &'a str,
        fallback: Option<&'a str>,
        arguments: &'a [Value],
    },
    Keybind(&'a str),
    Selector {
        pattern: &'a str,
        separator: Option<Source<'a>>,
    },
    Score {
        name: &'a str,
        objective: &'a str,
        fields: &'a Map<String, Value>,
    },
    Nbt {
        path: &'a str,
        interpret: bool,
        source: NbtSource<'a>,
        separator: Option<Source<'a>>,
    },
    Player {
        profile: &'a Value,
        hat: bool,
        fallback: Option<Source<'a>>,
    },
    Atlas {
        atlas: &'a str,
        sprite: &'a str,
        fallback: Option<Source<'a>>,
    },
}
enum Hover<'a> {
    Text(Source<'a>),
    Item {
        id: &'a str,
        count: u32,
        components: Option<&'a Value>,
    },
    Entity {
        kind: &'a str,
        id: [u64; 2],
        name: Option<Source<'a>>,
    },
}
#[derive(Clone, Copy)]
enum Style<'a> {
    Json(&'a Map<String, Value>),
    Native(&'a engine::Style),
    Empty,
}
#[derive(Clone, Copy)]
enum CaptureEdge {
    Child,
    Argument,
    Separator,
    Fallback,
    HoverText,
    HoverEntityName,
}
struct Node<'a> {
    source: Source<'a>,
    content: Content<'a>,
    style: Style<'a>,
    hover: Option<Hover<'a>>,
    hover_fields: Option<&'a Map<String, Value>>,
    fields: Option<&'a Map<String, Value>>,
    edges: Vec<(Source<'a>, CaptureEdge)>,
    merge: bool,
}

fn uuid(value: &Value) -> Option<[u64; 2]> {
    let words = if let Some(s) = value.as_str() {
        sunrise_game_data::uuid::uuid_from_text(s)?
    } else {
        let values: &[Value; 4] = value.as_array()?.as_slice().try_into().ok()?;
        let mut words = [0i32; 4];
        for (word, value) in words.iter_mut().zip(values) {
            let n = value.as_number()?;
            *word = if let Some(n) = n.as_i64() {
                n as i32
            } else if let Some(n) = n.as_u64() {
                n as i32
            } else {
                n.as_f64()? as i32
            };
        }
        words
    }
    .map(|v| v as u32 as u64);
    Some([words[0] << 32 | words[1], words[2] << 32 | words[3]])
}
fn hover(fields: &Map<String, Value>) -> Option<Hover<'_>> {
    Some(match fields.get("action")?.as_str()? {
        "show_text" => Hover::Text(Source::Json(
            fields
                .get("value")
                .or_else(|| fields.get("contents"))
                .or_else(|| fields.get("text"))?,
        )),
        "show_item" => Hover::Item {
            id: fields.get("id")?.as_str()?,
            count: fields
                .get("count")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(1),
            components: fields.get("components"),
        },
        "show_entity" => Hover::Entity {
            kind: fields.get("id")?.as_str()?,
            id: uuid(fields.get("uuid")?)?,
            name: fields.get("name").map(Source::Json),
        },
        _ => return None,
    })
}
fn content(fields: &Map<String, Value>, lenient: bool) -> Result<Content<'_>, Error> {
    use component::InlineObjectFields as Object;
    if let Some(object) = component::inline_object_fields(fields) {
        let fallback = fields.get("fallback").map(Source::Json);
        return Ok(match object {
            Object::Player { profile, hat } => Content::Player {
                profile,
                hat,
                fallback,
            },
            Object::Atlas { atlas, sprite } => Content::Atlas {
                atlas,
                sprite,
                fallback,
            },
        });
    }
    if let Some(value) = fields.get("text") {
        return Ok(match value {
            Value::String(s) => Content::Literal(s),
            Value::Number(_) | Value::Bool(_) | Value::Null => Content::Scalar(value),
            _ if lenient => Content::Literal(""),
            _ => return Err(Error::Malformed),
        });
    }
    if let Some(key) = fields.get("translate").and_then(Value::as_str) {
        return Ok(Content::Translate {
            key,
            fallback: fields.get("fallback").and_then(Value::as_str),
            arguments: fields
                .get("with")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        });
    }
    if let Some(pattern) = fields.get("selector").and_then(Value::as_str) {
        return Ok(Content::Selector {
            pattern,
            separator: fields.get("separator").map(Source::Json),
        });
    }
    if let Some(key) = fields.get("keybind").and_then(Value::as_str) {
        return Ok(Content::Keybind(key));
    }
    if let Some(score) = fields.get("score").and_then(Value::as_object)
        && let (Some(name), Some(objective)) = (
            score.get("name").and_then(Value::as_str),
            score.get("objective").and_then(Value::as_str),
        )
    {
        return Ok(Content::Score {
            name,
            objective,
            fields: score,
        });
    }
    if let Some(path) = fields.get("nbt").and_then(Value::as_str) {
        let source = fields
            .get("block")
            .and_then(Value::as_str)
            .map(NbtSource::Block)
            .or_else(|| {
                fields
                    .get("entity")
                    .and_then(Value::as_str)
                    .map(NbtSource::Entity)
            })
            .or_else(|| {
                fields
                    .get("storage")
                    .and_then(Value::as_str)
                    .map(NbtSource::Storage)
            });
        if let Some(source) = source {
            return Ok(Content::Nbt {
                path,
                interpret: component::style_flag(fields.get("interpret")).unwrap_or(false),
                source,
                separator: fields.get("separator").map(Source::Json),
            });
        }
    }
    Ok(Content::Literal(""))
}
fn node(source: Source<'_>, lenient: bool) -> Result<Node<'_>, Error> {
    let mut node = Node {
        source,
        content: Content::Literal(""),
        style: Style::Empty,
        hover: None,
        hover_fields: None,
        fields: None,
        edges: Vec::new(),
        merge: false,
    };
    match source {
        Source::Str(s) => node.content = Content::Literal(s),
        Source::Engine(text) => {
            node.merge = true;
            if let Some(source) = text.source_component() {
                node.edges.push((Source::Json(source), CaptureEdge::Child));
            } else {
                node.edges.extend(
                    text.spans
                        .iter()
                        .map(|span| (Source::Span(span), CaptureEdge::Child)),
                );
                // One span is the component itself; multiple spans need a parent.
                node.merge = text.spans.len() <= 1;
            }
        }
        Source::Json(Value::Array(values)) => {
            node.merge = true;
            node.edges
                .extend(values.iter().map(|v| (Source::Json(v), CaptureEdge::Child)));
        }
        Source::Json(Value::Object(fields)) => {
            node.content = content(fields, lenient)?;
            node.fields = Some(fields);
            node.style = Style::Json(fields);
            node.hover_fields = component::style_event_object(fields, "hover_event", "hoverEvent");
            node.hover = node.hover_fields.and_then(hover);
            if let Some(values) = fields.get("extra").and_then(Value::as_array) {
                node.edges
                    .extend(values.iter().map(|v| (Source::Json(v), CaptureEdge::Child)));
            }
        }
        Source::Json(Value::String(s)) => node.content = Content::Literal(s),
        Source::Json(value @ (Value::Bool(_) | Value::Number(_))) => {
            node.content = Content::Scalar(value)
        }
        Source::Json(Value::Null) => {}
        Source::Span(span) => {
            node.content = match &span.style.inline_object {
                Some(engine::InlineObject::Player {
                    profile,
                    hat,
                    fallback,
                }) => Content::Player {
                    profile,
                    hat: *hat,
                    fallback: Some(Source::Str(fallback)),
                },
                Some(engine::InlineObject::Atlas {
                    atlas,
                    sprite,
                    fallback,
                }) => Content::Atlas {
                    atlas,
                    sprite,
                    fallback: Some(Source::Str(fallback)),
                },
                Some(engine::InlineObject::Dynamic { id, fallback, .. }) => Content::Atlas {
                    atlas: "minecraft:gui",
                    sprite: id,
                    fallback: Some(Source::Str(fallback)),
                },
                None => Content::Literal(&span.text),
            };
            node.style = Style::Native(&span.style);
            if let Some(text) = &span.style.hover {
                node.hover = Some(Hover::Text(Source::Engine(text)));
            } else {
                node.hover_fields = span
                    .style
                    .hover_item
                    .as_deref()
                    .or(span.style.hover_entity.as_deref())
                    .and_then(Value::as_object);
                // Native retained item/entity events may omit action.
                if let Some(fields) = node.hover_fields {
                    node.hover = if span.style.hover_item.is_some() {
                        fields
                            .get("id")
                            .and_then(Value::as_str)
                            .map(|id| Hover::Item {
                                id,
                                count: fields
                                    .get("count")
                                    .and_then(Value::as_u64)
                                    .and_then(|v| u32::try_from(v).ok())
                                    .unwrap_or(1),
                                components: fields.get("components"),
                            })
                    } else {
                        match (
                            fields.get("id").and_then(Value::as_str),
                            fields.get("uuid").and_then(uuid),
                        ) {
                            (Some(kind), Some(id)) => Some(Hover::Entity {
                                kind,
                                id,
                                name: fields.get("name").map(Source::Json),
                            }),
                            _ => None,
                        }
                    };
                }
            }
        }
    }
    match &node.content {
        Content::Translate { arguments, .. } => node.edges.extend(
            arguments
                .iter()
                .map(|v| (Source::Json(v), CaptureEdge::Argument)),
        ),
        Content::Selector { separator, .. } | Content::Nbt { separator, .. } => node
            .edges
            .extend(separator.iter().map(|&v| (v, CaptureEdge::Separator))),
        Content::Player { fallback, .. } | Content::Atlas { fallback, .. } => node
            .edges
            .extend(fallback.iter().map(|&v| (v, CaptureEdge::Fallback))),
        _ => {}
    }
    match &node.hover {
        Some(Hover::Text(v)) => node.edges.push((*v, CaptureEdge::HoverText)),
        Some(Hover::Entity { name: Some(v), .. }) => {
            node.edges.push((*v, CaptureEdge::HoverEntityName))
        }
        _ => {}
    }
    Ok(node)
}
// Plan borrowed topology iteratively before cloning any data. Depth limits belong
// to serialization boundaries (Wire, JSON/SNBT codecs, NBT/network), not in-process text.
fn plan(root: Source<'_>, lenient: bool) -> Result<Vec<Node<'_>>, Error> {
    enum Work<'a> {
        Enter(Source<'a>),
        Finish(Node<'a>),
    }
    let mut work = vec![Work::Enter(root)];
    let mut nodes = Vec::new();
    while let Some(task) = work.pop() {
        match task {
            Work::Finish(node) => nodes.push(node),
            Work::Enter(source) => {
                let node = node(source, lenient)?;
                let children: Vec<_> = node.edges.iter().map(|&(source, _)| source).collect();
                work.push(Work::Finish(node));
                work.extend(children.into_iter().rev().map(Work::Enter));
            }
        }
    }
    Ok(nodes)
}

fn error(error: json_nbt::JsonNbtError) -> Error {
    match error {
        json_nbt::JsonNbtError::TooDeep => Error::TooDeep,
        json_nbt::JsonNbtError::NestedNull => Error::NestedNull,
        json_nbt::JsonNbtError::NonFinite => Error::NonFinite,
        json_nbt::JsonNbtError::InvalidString => Error::InvalidString,
    }
}
fn metadata<'a>(
    fields: impl IntoIterator<Item = (&'a str, &'a Value)>,
    parent_depth: usize,
) -> Result<Tag, Error> {
    json_nbt::json_fields_to_compound(fields, parent_depth)
        .map(Tag::Compound)
        .map_err(error)
}
fn reserved(key: &str) -> bool {
    [
        "type",
        "text",
        "translate",
        "with",
        "fallback",
        "keybind",
        "selector",
        "separator",
        "score",
        "nbt",
        "interpret",
        "block",
        "entity",
        "storage",
        "object",
        "player",
        "hat",
        "atlas",
        "sprite",
        "extra",
        "color",
        "shadow_color",
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
        "font",
        "insertion",
        "click_event",
        "hover_event",
        "clickEvent",
        "hoverEvent",
    ]
    .contains(&key)
}
fn click_kind(click: &TextClick) -> &'static str {
    match click {
        TextClick::OpenUrl(_) => "open_url",
        TextClick::ChangePage(_) => "change_page",
        TextClick::SuggestCommand(_) => "suggest_command",
        TextClick::RunCommand(_) => "run_command",
        TextClick::CopyToClipboard(_) => "copy_to_clipboard",
        TextClick::ShowDialog(_) => "show_dialog",
        TextClick::Custom { .. } => "custom",
    }
}
fn event_reserved(kind: &str, key: &str) -> bool {
    key == "action"
        || match kind {
            "open_url" => ["url", "value"].contains(&key),
            "change_page" => ["page", "value"].contains(&key),
            "suggest_command" | "run_command" => ["command", "value"].contains(&key),
            "copy_to_clipboard" => key == "value",
            "show_dialog" => key == "dialog",
            "custom" => ["id", "payload"].contains(&key),
            "show_text" => ["value", "contents", "text"].contains(&key),
            "show_item" => ["id", "count", "components"].contains(&key),
            "show_entity" => ["id", "uuid", "name"].contains(&key),
            _ => false,
        }
}
/// `ExtraCodecs.PLAYER_NAME` on a Rust string.
pub fn valid_profile_name(name: &str) -> bool {
    name.len() <= 16 && name.bytes().all(|b| (33..127).contains(&b))
}

/// The UTF-16 length limits of `ExtraCodecs.PROPERTY`.
pub fn valid_profile_property(name: &str, value: &str, signature: Option<&str>) -> bool {
    name.encode_utf16().count() <= 64
        && value.encode_utf16().count() <= 32767
        && signature.is_none_or(|s| s.encode_utf16().count() <= 1024)
}

fn profile(
    value: &Value,
    keep_metadata: bool,
    validate_codec: bool,
    lenient: bool,
) -> Result<TextProfile, Error> {
    fn name(value: &str, validate_codec: bool) -> Result<String, Error> {
        if validate_codec && !valid_profile_name(value) {
            return Err(Error::Malformed);
        }
        Ok(value.into())
    }
    fn field<'a>(
        fields: &'a Map<String, Value>,
        key: &str,
        lenient: bool,
    ) -> Result<Option<&'a str>, Error> {
        match fields.get(key).filter(|v| !v.is_null()) {
            None => Ok(None),
            Some(v) => match v.as_str() {
                Some(s) => Ok(Some(s)),
                None if lenient => Ok(None),
                None => Err(Error::Malformed),
            },
        }
    }
    if let Value::String(value) = value {
        let mut profile = TextProfile::default();
        profile.name = Some(name(value, validate_codec)?);
        return Ok(profile);
    }
    let Some(fields) = value.as_object() else {
        return if lenient {
            Ok(TextProfile::default())
        } else {
            Err(Error::Malformed)
        };
    };
    let name = field(fields, "name", lenient)?
        .map(|s| name(s, validate_codec))
        .transpose()?;
    let id = match fields.get("id").filter(|v| !v.is_null()) {
        None => None,
        Some(value) if value.is_array() => match uuid(value) {
            Some(id) => Some(id),
            None if lenient => None,
            None => return Err(Error::Malformed),
        },
        Some(_) if lenient => None,
        Some(_) => return Err(Error::Malformed),
    };
    let mut properties = Vec::new();
    match fields.get("properties").filter(|v| !v.is_null()) {
        None => {}
        Some(Value::Object(map)) => {
            if validate_codec && map.len() > 16 {
                return Err(Error::Malformed);
            }
            for (name, values) in map {
                let Some(values) = values.as_array() else {
                    if lenient {
                        continue;
                    }
                    return Err(Error::Malformed);
                };
                for value in values {
                    let Some(value) = value.as_str() else {
                        if lenient {
                            continue;
                        }
                        return Err(Error::Malformed);
                    };
                    let mut property = TextProfileProperty::default();
                    property.name = name.clone();
                    property.value = value.into();
                    properties.push(property);
                }
            }
        }
        Some(Value::Array(records)) => {
            if validate_codec && records.len() > 16 {
                return Err(Error::Malformed);
            }
            for record in records {
                let Some(record) = record.as_object() else {
                    if lenient {
                        continue;
                    }
                    return Err(Error::Malformed);
                };
                let (Some(name), Some(value)) = (
                    field(record, "name", lenient)?,
                    field(record, "value", lenient)?,
                ) else {
                    if lenient {
                        continue;
                    }
                    return Err(Error::Malformed);
                };
                let signature = field(record, "signature", lenient)?;
                if validate_codec && !valid_profile_property(name, value, signature) {
                    return Err(Error::Malformed);
                }
                let mut property = TextProfileProperty::default();
                property.name = name.into();
                property.value = value.into();
                property.signature = signature.map(str::to_owned);
                if keep_metadata {
                    let meta = metadata(
                        record
                            .iter()
                            .filter(|(k, _)| !["name", "value", "signature"].contains(&k.as_str()))
                            .map(|(k, v)| (k.as_str(), v)),
                        0,
                    );
                    property.metadata = match meta {
                        Ok(tag) => tag,
                        Err(_) if lenient => Tag::Compound(Default::default()),
                        Err(err) => return Err(err),
                    };
                }
                properties.push(property);
            }
        }
        _ if lenient => {}
        _ => return Err(Error::Malformed),
    }
    let mut profile = TextProfile::default();
    profile.name = name;
    profile.id = id;
    profile.properties = properties;
    if keep_metadata {
        let meta = metadata(
            fields
                .iter()
                .filter(|(k, _)| !["name", "id", "properties"].contains(&k.as_str()))
                .map(|(k, v)| (k.as_str(), v)),
            0,
        );
        profile.metadata = match meta {
            Ok(tag) => tag,
            Err(_) if lenient => Tag::Compound(Default::default()),
            Err(err) => return Err(err),
        };
    }
    Ok(profile)
}
fn take_data(tag: std::sync::Arc<Tag>) -> Tag {
    std::sync::Arc::try_unwrap(tag).unwrap_or_else(|tag| nbt::clone_tag(&tag))
}
fn native_click(click: engine::ClickEvent) -> Option<TextClick> {
    use engine::ClickEvent as E;
    Some(match click {
        E::OpenUrl(v) => {
            if !component::untrusted_http_url(&v) {
                return None;
            }
            TextClick::OpenUrl(v)
        }
        E::ChangePage(v) => {
            if v < 1 {
                return None;
            }
            TextClick::ChangePage(v)
        }
        E::SuggestCommand(v) => TextClick::SuggestCommand(v),
        E::RunCommand(v) => TextClick::RunCommand(v),
        E::CopyToClipboard(v) => TextClick::CopyToClipboard(v),
        E::ShowDialog(v) => TextClick::ShowDialog(take_data(v)),
        E::Custom { id, payload } => TextClick::Custom {
            id,
            payload: payload.map(take_data),
        },
        E::OpenFile(_) | E::Profiler(_) => return None,
    })
}
fn captured_style(node: &Node<'_>, lenient: bool) -> Result<TextStyle, Error> {
    let mut style = TextStyle::default();
    match node.style {
        Style::Empty => {}
        Style::Native(s) => {
            style.color = s.color.map(|v| format!("#{v:06X}"));
            style.shadow_color = s.shadow_color;
            style.bold = s.bold;
            style.italic = s.italic;
            style.underlined = s.underlined;
            style.strikethrough = s.strikethrough;
            style.obfuscated = s.obfuscated;
            style.font = s.font.clone();
            style.insertion = s.insertion.clone();
            style.click = s.click.clone().and_then(native_click);
        }
        Style::Json(fields) => {
            style.color = fields
                .get("color")
                .and_then(Value::as_str)
                .map(str::to_owned);
            style.shadow_color = fields.get("shadow_color").and_then(component::shadow_color);
            style.bold = component::style_flag(fields.get("bold"));
            style.italic = component::style_flag(fields.get("italic"));
            style.underlined = component::style_flag(fields.get("underlined"));
            style.strikethrough = component::style_flag(fields.get("strikethrough"));
            style.obfuscated = component::style_flag(fields.get("obfuscated"));
            style.font = fields
                .get("font")
                .and_then(Value::as_str)
                .map(str::to_owned);
            style.insertion = fields
                .get("insertion")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let parsed_click = component::style_event_object(fields, "click_event", "clickEvent")
                .map(component::parse_click_event)
                .transpose();
            style.click = match parsed_click {
                Ok(click) => click.flatten().and_then(native_click),
                Err(_) if lenient => None,
                Err(err) => return Err(error(err)),
            };
        }
    }
    style.hover = match &node.hover {
        None | Some(Hover::Text(_)) => None, // Child attachment supplies text.
        Some(Hover::Item {
            id,
            count,
            components,
        }) => Some(TextHover::Item {
            id: (*id).into(),
            count: *count,
            components: match components.map(json_nbt::json_to_tag).transpose() {
                Ok(tag) => tag,
                Err(_) if lenient => None,
                Err(err) => return Err(error(err)),
            },
        }),
        Some(Hover::Entity { kind, id, .. }) => Some(TextHover::Entity {
            kind: (*kind).into(),
            id: *id,
            name: None,
        }),
    };
    let click_fields = match node.style {
        Style::Json(fields) if style.click.is_some() => {
            component::style_event_object(fields, "click_event", "clickEvent")
        }
        _ => None,
    };
    let hover_fields = node.hover_fields.filter(|_| node.hover.is_some());
    let Tag::Compound(extra) = &mut style.metadata else {
        unreachable!()
    };
    let hover_kind = match &node.hover {
        Some(Hover::Text(_)) => "show_text",
        Some(Hover::Item { .. }) => "show_item",
        Some(Hover::Entity { .. }) => "show_entity",
        None => "",
    };
    for (key, fields, kind) in [
        (
            "click_event",
            click_fields,
            style.click.as_ref().map(click_kind).unwrap_or(""),
        ),
        ("hover_event", hover_fields, hover_kind),
    ] {
        if let Some(fields) = fields {
            let tag = match metadata(
                fields
                    .iter()
                    .filter(|(k, _)| !event_reserved(kind, k))
                    .map(|(k, v)| (k.as_str(), v)),
                1,
            ) {
                Ok(tag) => tag,
                Err(_) if lenient => continue,
                Err(err) => return Err(err),
            };
            if !matches!(&tag, Tag::Compound(v) if v.is_empty()) {
                extra.insert(key.into(), tag);
            }
        }
    }
    Ok(style)
}
fn captured_shallow(node: &Node<'_>, lenient: bool) -> Result<Text, Error> {
    let meta = match node
        .fields
        .map(|fields| {
            metadata(
                fields
                    .iter()
                    .filter(|(k, _)| !reserved(k))
                    .map(|(k, v)| (k.as_str(), v)),
                0,
            )
        })
        .transpose()
    {
        Ok(Some(tag)) => tag,
        Ok(None) => Tag::Compound(Default::default()),
        Err(_) if lenient => Tag::Compound(Default::default()),
        Err(err) => return Err(err),
    };
    let mut text = Text::new(match &node.content {
        Content::Literal(s) => TextContent::Literal((*s).into()),
        Content::Scalar(value) => TextContent::Literal(value.to_string()),
        Content::Translate { key, fallback, .. } => TextContent::Translatable {
            key: (*key).into(),
            fallback: fallback.map(str::to_owned),
            arguments: Vec::new(),
        },
        Content::Keybind(key) => TextContent::Keybind((*key).into()),
        Content::Selector { pattern, .. } => TextContent::Selector {
            pattern: (*pattern).into(),
            separator: None,
        },
        Content::Score {
            name, objective, ..
        } => TextContent::Score {
            name: (*name).into(),
            objective: (*objective).into(),
        },
        Content::Nbt {
            path,
            interpret,
            source,
            ..
        } => TextContent::Nbt {
            path: (*path).into(),
            interpret: *interpret,
            source: match source {
                NbtSource::Block(v) => TextNbtSource::Block((*v).into()),
                NbtSource::Entity(v) => TextNbtSource::Entity((*v).into()),
                NbtSource::Storage(v) => TextNbtSource::Storage((*v).into()),
            },
            separator: None,
        },
        Content::Player {
            profile: value,
            hat,
            ..
        } => TextContent::Object(TextObject::Player {
            profile: profile(value, true, false, lenient)?,
            hat: *hat,
            fallback: None,
        }),
        Content::Atlas { atlas, sprite, .. } => TextContent::Object(TextObject::Atlas {
            atlas: (*atlas).into(),
            sprite: (*sprite).into(),
            fallback: None,
        }),
    });
    text.metadata = meta;
    if let Content::Score { fields, .. } = &node.content {
        let score = match metadata(
            fields
                .iter()
                .filter(|(k, _)| !["name", "objective"].contains(&k.as_str()))
                .map(|(k, v)| (k.as_str(), v)),
            1,
        ) {
            Ok(tag) => Some(tag),
            Err(_) if lenient => None,
            Err(err) => return Err(err),
        };
        if let Some(score) = score
            && !matches!(&score, Tag::Compound(v) if v.is_empty())
        {
            let Tag::Compound(meta) = &mut text.metadata else {
                unreachable!()
            };
            meta.insert("score".into(), score);
        }
    }
    text.style = captured_style(node, lenient)?;
    Ok(text)
}
fn attach(text: &mut Text, relation: CaptureEdge, child: Text) {
    match relation {
        CaptureEdge::Child => text.children.push(child),
        CaptureEdge::Argument => match &mut text.content {
            TextContent::Translatable { arguments, .. } => arguments.push(child),
            _ => unreachable!(),
        },
        CaptureEdge::Separator => match &mut text.content {
            TextContent::Selector { separator, .. } | TextContent::Nbt { separator, .. } => {
                *separator = Some(Box::new(child))
            }
            _ => unreachable!(),
        },
        CaptureEdge::Fallback => match &mut text.content {
            TextContent::Object(
                TextObject::Player { fallback, .. } | TextObject::Atlas { fallback, .. },
            ) => *fallback = Some(Box::new(child)),
            _ => unreachable!(),
        },
        CaptureEdge::HoverText => text.style.hover = Some(TextHover::Text(Box::new(child))),
        CaptureEdge::HoverEntityName => match &mut text.style.hover {
            Some(TextHover::Entity { name, .. }) => *name = Some(Box::new(child)),
            _ => unreachable!(),
        },
    }
}
fn capture_with(root: Source<'_>, lenient: bool) -> Result<Text, Error> {
    let nodes = plan(root, lenient)?;
    let mut ready: HashMap<Key, Text> = HashMap::new();
    for node in nodes {
        let mut text = if node.merge {
            node.edges
                .first()
                .map(|(child, _)| ready.remove(&child.key()).expect("child was built"))
                .unwrap_or_default()
        } else {
            captured_shallow(&node, lenient)?
        };
        for &(child, relation) in node.edges.iter().skip(usize::from(node.merge)) {
            attach(
                &mut text,
                relation,
                ready.remove(&child.key()).expect("child was built"),
            );
        }
        ready.insert(node.source.key(), text);
    }
    Ok(ready.remove(&root.key()).expect("root was built"))
}
fn capture(root: Source<'_>) -> Result<Text, Error> {
    capture_with(root, false)
}
pub fn captured_text(value: &Value) -> Result<Text, Error> {
    capture(Source::Json(value))
}
/// A span's hover as a mod reads it (`GuiGraphicsExtractor.componentHoverEffect`'s style): the
/// same capture as the span's own, from the style's text, item or entity hover.
pub fn style_hover(style: &engine::Style) -> Option<TextHover> {
    if style.hover.is_none() && style.hover_item.is_none() && style.hover_entity.is_none() {
        return None;
    }
    let span = engine::Span::new(String::new(), style.clone());
    capture(Source::Span(&span)).ok()?.style.hover.take()
}
/// Capture an engine text in memory without imposing serialization depth or profile caps.
pub fn to_api(text: &engine::Text) -> Result<Text, TextFormError> {
    capture(Source::Engine(text))
}
/// Capture an engine text in memory for infallible in-process text reads, dropping any
/// unrepresentable opaque metadata rather than failing.
pub fn to_api_inprocess(text: &engine::Text) -> Text {
    capture_with(Source::Engine(text), true).expect("lenient capture is infallible")
}

fn api_nodes(root: &Text, local_layout: bool) -> Vec<&Text> {
    let mut work = vec![(root, false)];
    let mut nodes = Vec::new();
    while let Some((text, done)) = work.pop() {
        if done {
            nodes.push(text);
        } else {
            work.push((text, true));
            let hover = if local_layout {
                match &text.style.hover {
                    Some(TextHover::Text(child)) => Some(child.as_ref()),
                    Some(TextHover::Entity { name, .. }) => name.as_deref(),
                    _ => None,
                }
            } else {
                None
            };
            let first = work.len();
            text.visit_children(|child| {
                if !hover.is_some_and(|hover| std::ptr::eq(hover, child)) {
                    work.push((child, false));
                }
            });
            work[first..].reverse();
        }
    }
    nodes
}
struct Payloads<'a> {
    tags: HashMap<usize, json_nbt::JsonTag<'a>>,
}
impl<'a> Payloads<'a> {
    fn insert_tag(
        tags: &mut HashMap<usize, json_nbt::JsonTag<'a>>,
        tag: &'a Tag,
        lenient: bool,
    ) -> Result<(), Error> {
        if matches!(tag, Tag::Compound(v) if v.is_empty())
            || matches!(tag, Tag::List(v) if v.is_empty())
        {
            return Ok(());
        }
        match json_nbt::JsonTag::for_projection(tag) {
            Ok(json_tag) => {
                tags.insert(tag as *const _ as usize, json_tag);
                Ok(())
            }
            Err(_) if lenient => Ok(()),
            Err(err) => Err(error(err)),
        }
    }
    fn insert_metadata(
        tags: &mut HashMap<usize, json_nbt::JsonTag<'a>>,
        tag: &'a Tag,
        lenient: bool,
    ) -> Result<(), Error> {
        if !matches!(tag, Tag::Compound(_)) {
            return if lenient {
                Ok(())
            } else {
                Err(Error::Malformed)
            };
        }
        Self::insert_tag(tags, tag, lenient)
    }
    fn new(nodes: &[&'a Text], lenient: bool, local_layout: bool) -> Result<Self, Error> {
        let mut tags = HashMap::new();
        for text in nodes {
            if let TextContent::Object(TextObject::Player { profile, .. }) = &text.content {
                Self::insert_metadata(&mut tags, &profile.metadata, lenient)?;
                for p in &profile.properties {
                    Self::insert_metadata(&mut tags, &p.metadata, lenient)?;
                }
            }
            if local_layout {
                continue;
            }
            Self::insert_metadata(&mut tags, &text.metadata, lenient)?;
            Self::insert_metadata(&mut tags, &text.style.metadata, lenient)?;
            #[allow(clippy::single_match)] // TODO(#2273): rewrite as `if let`.
            match &text.style.click {
                Some(
                    TextClick::ShowDialog(tag)
                    | TextClick::Custom {
                        payload: Some(tag), ..
                    },
                ) => Self::insert_tag(&mut tags, tag, lenient)?,
                _ => {}
            }
            if let Some(TextHover::Item {
                components: Some(tag),
                ..
            }) = &text.style.hover
            {
                Self::insert_tag(&mut tags, tag, lenient)?;
            }
        }
        Ok(Self { tags })
    }
    fn emit(&self, tag: &Tag) -> Option<Value> {
        match tag {
            Tag::Compound(v) if v.is_empty() => Some(Value::Object(Map::new())),
            Tag::List(v) if v.is_empty() => Some(Value::Array(Vec::new())),
            _ => self
                .tags
                .get(&(tag as *const _ as usize))
                .map(|tag| tag.to_json()),
        }
    }
    fn object(&self, tag: &Tag) -> Map<String, Value> {
        match self.emit(tag) {
            Some(Value::Object(map)) => map,
            Some(value) => {
                component::discard_component(value);
                Map::new()
            }
            None => Map::new(),
        }
    }
}
fn filter_metadata(
    fields: Map<String, Value>,
    reserved: impl Fn(&str) -> bool,
) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in fields {
        if reserved(&key) {
            component::discard_component(value);
        } else {
            out.insert(key, value);
        }
    }
    out
}
fn object_or_empty(value: Option<Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(map)) => map,
        Some(value) => {
            component::discard_component(value);
            Map::new()
        }
        None => Map::new(),
    }
}
/// A component codec enforces `ExtraCodecs` profile/property bounds and drops fields
/// outside the profile schema; engine text capture, on the other hand, round-trips
/// in-memory profiles and metadata.
pub fn decoded_profile_json(value: &Value) -> Option<Value> {
    Some(profile_json(
        &profile(value, false, true, false).ok()?,
        None,
    ))
}

fn profile_json(profile: &TextProfile, payloads: Option<&Payloads<'_>>) -> Value {
    let mut out = payloads.map_or_else(Map::new, |payloads| {
        filter_metadata(payloads.object(&profile.metadata), |k| {
            ["name", "id", "properties"].contains(&k)
        })
    });
    if let Some(name) = &profile.name {
        out.insert("name".into(), name.clone().into());
    }
    if let Some([hi, lo]) = profile.id {
        out.insert(
            "id".into(),
            Value::Array(
                [(hi >> 32) as i32, hi as i32, (lo >> 32) as i32, lo as i32]
                    .map(Value::from)
                    .into(),
            ),
        );
    }
    if !profile.properties.is_empty() {
        let unsigned = profile.properties.iter().all(|p| {
            p.signature.is_none()
                && (payloads.is_none() || matches!(&p.metadata, Tag::Compound(m) if m.is_empty()))
        });
        let use_map = unsigned
            && (profile.properties.len() > 16
                || profile.properties.iter().any(|p| {
                    p.name.encode_utf16().take(65).count() > 64
                        || p.value.encode_utf16().take(32768).count() > 32767
                }));
        let properties = if use_map {
            let mut map = Map::new();
            for p in &profile.properties {
                let value = map
                    .entry(p.name.clone())
                    .or_insert_with(|| Value::Array(Vec::new()));
                let Value::Array(values) = value else {
                    unreachable!()
                };
                values.push(p.value.clone().into());
            }
            Value::Object(map)
        } else {
            Value::Array(
                profile
                    .properties
                    .iter()
                    .map(|p| {
                        let mut fields = payloads.map_or_else(Map::new, |payloads| {
                            filter_metadata(payloads.object(&p.metadata), |k| {
                                ["name", "value", "signature"].contains(&k)
                            })
                        });
                        fields.insert("name".into(), p.name.clone().into());
                        fields.insert("value".into(), p.value.clone().into());
                        if let Some(signature) = &p.signature {
                            fields.insert("signature".into(), signature.clone().into());
                        }
                        Value::Object(fields)
                    })
                    .collect(),
            )
        };
        out.insert("properties".into(), properties);
    }
    Value::Object(out)
}
fn action(action: &str, field: &str, value: Value) -> Map<String, Value> {
    [("action".into(), action.into()), (field.into(), value)]
        .into_iter()
        .collect()
}
fn emitted_style<'a>(
    style: &'a TextStyle,
    payloads: &Payloads<'_>,
    take: &mut impl FnMut(&'a Text) -> Value,
    local_layout: bool,
) -> Map<String, Value> {
    let mut metadata = payloads.object(&style.metadata);
    let click = object_or_empty(metadata.remove("click_event"));
    let hover = object_or_empty(metadata.remove("hover_event"));
    let mut out = filter_metadata(metadata, reserved);
    for (key, value) in [
        ("color", style.color.clone().map(Value::String)),
        (
            "shadow_color",
            style.shadow_color.map(|v| Value::from(v as i32)),
        ),
        ("bold", style.bold.map(Value::Bool)),
        ("italic", style.italic.map(Value::Bool)),
        ("underlined", style.underlined.map(Value::Bool)),
        ("strikethrough", style.strikethrough.map(Value::Bool)),
        ("obfuscated", style.obfuscated.map(Value::Bool)),
        ("font", style.font.clone().map(Value::String)),
        ("insertion", style.insertion.clone().map(Value::String)),
    ] {
        if let Some(v) = value {
            out.insert(key.into(), v);
        }
    }
    if local_layout {
        return out;
    }
    if let Some(value) = &style.click {
        let kind = click_kind(value);
        let emitted = match value {
            TextClick::OpenUrl(v) => Some(action("open_url", "url", v.clone().into())),
            TextClick::ChangePage(v) => Some(action("change_page", "page", (*v).into())),
            TextClick::SuggestCommand(v) => {
                Some(action("suggest_command", "command", v.clone().into()))
            }
            TextClick::RunCommand(v) => Some(action("run_command", "command", v.clone().into())),
            TextClick::CopyToClipboard(v) => {
                Some(action("copy_to_clipboard", "value", v.clone().into()))
            }
            TextClick::ShowDialog(v) => payloads
                .emit(v)
                .map(|dialog| action("show_dialog", "dialog", dialog)),
            TextClick::Custom { id, payload } => {
                let mut fields = action("custom", "id", id.clone().into());
                if let Some(tag) = payload
                    && let Some(value) = payloads.emit(tag)
                {
                    fields.insert("payload".into(), value);
                }
                Some(fields)
            }
        };
        if let Some(emitted) = emitted {
            let mut fields = filter_metadata(click, |key| event_reserved(kind, key));
            fields.extend(emitted);
            out.insert("click_event".into(), Value::Object(fields));
        } else {
            component::discard_component(Value::Object(click));
        }
    } else {
        component::discard_component(Value::Object(click));
    }
    if let Some(value) = &style.hover {
        let kind = match value {
            TextHover::Text(_) => "show_text",
            TextHover::Item { .. } => "show_item",
            TextHover::Entity { .. } => "show_entity",
        };
        let mut fields = filter_metadata(hover, |key| event_reserved(kind, key));
        match value {
            TextHover::Text(text) => {
                fields.insert("action".into(), "show_text".into());
                fields.insert("value".into(), take(text));
            }
            TextHover::Item {
                id,
                count,
                components,
            } => {
                fields.extend([
                    ("action".into(), "show_item".into()),
                    ("id".into(), id.clone().into()),
                    ("count".into(), (*count).into()),
                ]);
                if let Some(tag) = components
                    && let Some(value) = payloads.emit(tag)
                {
                    fields.insert("components".into(), value);
                }
            }
            TextHover::Entity {
                kind,
                id: [hi, lo],
                name,
            } => {
                fields.extend([
                    ("action".into(), "show_entity".into()),
                    ("id".into(), kind.clone().into()),
                    (
                        "uuid".into(),
                        Value::Array(
                            [
                                (*hi >> 32) as i32,
                                *hi as i32,
                                (*lo >> 32) as i32,
                                *lo as i32,
                            ]
                            .map(Value::from)
                            .into(),
                        ),
                    ),
                ]);
                if let Some(name) = name {
                    fields.insert("name".into(), take(name));
                }
            }
        }
        out.insert("hover_event".into(), Value::Object(fields));
    } else {
        component::discard_component(Value::Object(hover));
    }
    out
}
fn emitted_node<'a>(
    text: &'a Text,
    payloads: &Payloads<'_>,
    take: &mut impl FnMut(&'a Text) -> Value,
    local_layout: bool,
) -> Value {
    let mut metadata = payloads.object(&text.metadata);
    let mut score = metadata.remove("score");
    let mut out = filter_metadata(metadata, reserved);
    match &text.content {
        TextContent::Literal(v) => {
            out.insert("text".into(), v.clone().into());
        }
        TextContent::Translatable {
            key,
            fallback,
            arguments,
        } => {
            out.insert("translate".into(), key.clone().into());
            if let Some(fallback) = fallback {
                out.insert("fallback".into(), fallback.clone().into());
            }
            if !arguments.is_empty() {
                out.insert(
                    "with".into(),
                    Value::Array(
                        arguments
                            .iter()
                            .map(|arg| engine::argument_component(take(arg)))
                            .collect(),
                    ),
                );
            }
        }
        TextContent::Keybind(v) => {
            out.insert("keybind".into(), v.clone().into());
        }
        TextContent::Selector { pattern, separator } => {
            out.insert("selector".into(), pattern.clone().into());
            if let Some(separator) = separator {
                out.insert("separator".into(), take(separator));
            }
        }
        TextContent::Score { name, objective } => {
            let mut fields = filter_metadata(object_or_empty(score.take()), |k| {
                ["name", "objective"].contains(&k)
            });
            fields.insert("name".into(), name.clone().into());
            fields.insert("objective".into(), objective.clone().into());
            out.insert("score".into(), Value::Object(fields));
        }
        TextContent::Nbt {
            path,
            interpret,
            source,
            separator,
        } => {
            out.insert("nbt".into(), path.clone().into());
            out.insert("interpret".into(), (*interpret).into());
            let (key, value) = match source {
                TextNbtSource::Block(v) => ("block", v),
                TextNbtSource::Entity(v) => ("entity", v),
                TextNbtSource::Storage(v) => ("storage", v),
            };
            out.insert(key.into(), value.clone().into());
            if let Some(separator) = separator {
                out.insert("separator".into(), take(separator));
            }
        }
        TextContent::Object(object) => {
            let fallback = match object {
                TextObject::Player {
                    profile,
                    hat,
                    fallback,
                } => {
                    out.extend([
                        ("object".into(), "player".into()),
                        ("player".into(), profile_json(profile, Some(payloads))),
                        ("hat".into(), (*hat).into()),
                    ]);
                    fallback
                }
                TextObject::Atlas {
                    atlas,
                    sprite,
                    fallback,
                } => {
                    out.extend([
                        ("object".into(), "atlas".into()),
                        ("atlas".into(), atlas.clone().into()),
                        ("sprite".into(), sprite.clone().into()),
                    ]);
                    fallback
                }
            };
            if let Some(fallback) = fallback {
                out.insert("fallback".into(), take(fallback));
            }
        }
    }
    if let Some(score) = score {
        component::discard_component(score);
    }
    out.extend(emitted_style(&text.style, payloads, take, local_layout));
    if !text.children.is_empty() {
        out.insert(
            "extra".into(),
            Value::Array(text.children.iter().map(take).collect()),
        );
    }
    Value::Object(out)
}
fn owned_text_component_with(
    text: &Text,
    lenient: bool,
    local_styles: Option<&HashMap<usize, usize>>,
) -> Result<Value, Error> {
    let local_layout = local_styles.is_some();
    let nodes = api_nodes(text, local_layout);
    // During layout, omit input metadata and hover/click payloads entirely. Only the
    // preview's component fields and internally assigned indices reach the renderer.
    let payloads = Payloads::new(&nodes, lenient, local_layout)?;
    let mut ready = HashMap::new();
    for node in nodes {
        let mut value = emitted_node(
            node,
            &payloads,
            &mut |child| {
                ready
                    .remove(&(child as *const _ as usize))
                    .expect("child was built")
            },
            local_layout,
        );
        if let Some(styles) = local_styles
            && let Some(&id) = styles.get(&(node as *const _ as usize))
            && let Value::Object(ref mut map) = value
        {
            map.insert("__sunrise_local_style_id".into(), Value::from(id));
        }
        ready.insert(node as *const _ as usize, value);
    }
    Ok(ready
        .remove(&(text as *const _ as usize))
        .expect("root was built"))
}
#[cfg(test)]
fn owned_text_component(text: &Text) -> Result<Value, Error> {
    owned_text_component_with(text, false, None)
}
/// Encode API text in memory without imposing serialization depth or profile caps.
pub fn to_engine(text: &Text) -> Result<engine::Text, TextFormError> {
    owned_text_component_with(text, false, None).map(Into::into)
}
/// Encode API text in memory for infallible in-process text reads, omitting any
/// unrepresentable opaque metadata rather than failing.
pub fn to_engine_inprocess(text: &Text) -> engine::Text {
    owned_text_component_with(text, true, None)
        .expect("lenient emit is infallible")
        .into()
}
#[derive(Clone, Copy, Default)]
struct LocalStyle<'a> {
    font: Option<&'a str>,
    insertion: Option<&'a str>,
    click: Option<&'a TextClick>,
    hover: Option<&'a TextHover>,
    // Index of the latest style that contributed metadata.
    metadata: Option<usize>,
}
struct StyleEntry<'a> {
    style: LocalStyle<'a>,
    // The metadata this component contributes and the previous contributing style.
    layer: Option<(&'a Tag, Option<usize>)>,
}

fn assign_local_styles(root: &Text) -> (HashMap<usize, usize>, Vec<StyleEntry<'_>>) {
    let empty = TextStyle::default();
    let mut ids = HashMap::new();
    let mut styles: Vec<StyleEntry<'_>> = Vec::new();
    let mut stack = vec![(root, None::<usize>)];
    while let Some((node, parent_id)) = stack.pop() {
        let effective_id = if node.style == empty {
            parent_id
        } else {
            let parent = parent_id.map(|id| styles[id].style).unwrap_or_default();
            let child = &node.style;
            let id = styles.len();
            let layer = (!matches!(&child.metadata, Tag::Compound(map) if map.is_empty()))
                .then_some((&child.metadata, parent.metadata));
            styles.push(StyleEntry {
                style: LocalStyle {
                    font: child.font.as_deref().or(parent.font),
                    insertion: child.insertion.as_deref().or(parent.insertion),
                    click: child.click.as_ref().or(parent.click),
                    hover: child.hover.as_ref().or(parent.hover),
                    metadata: layer.map(|_| id).or(parent.metadata),
                },
                layer,
            });
            Some(id)
        };
        if let Some(id) = effective_id {
            ids.insert(node as *const Text as usize, id);
        }
        for child in node.children.iter().rev() {
            stack.push((child, effective_id));
        }
        match &node.content {
            TextContent::Translatable { arguments, .. } => {
                for arg in arguments.iter().rev() {
                    stack.push((arg, effective_id));
                }
            }
            TextContent::Selector {
                separator: Some(sep),
                ..
            }
            | TextContent::Nbt {
                separator: Some(sep),
                ..
            } => stack.push((sep, effective_id)),
            TextContent::Object(
                TextObject::Player {
                    fallback: Some(fb), ..
                }
                | TextObject::Atlas {
                    fallback: Some(fb), ..
                },
            ) => stack.push((fb, effective_id)),
            _ => {}
        }
    }
    (ids, styles)
}
/// In-process engine text paired with its inherited API styles so layout and
/// formatted reads (`text_split`, `visit_text_visual_order`, `visit_text_lines`)
/// preserve opaque click/hover/metadata payloads without JSON serialization.
pub struct LocalText<'a> {
    engine: engine::Text,
    styles: Vec<StyleEntry<'a>>,
}
impl LocalText<'_> {
    pub fn engine(&self) -> &engine::Text {
        &self.engine
    }
    /// Compare the effective style at a visual run boundary, not its recovery index.
    pub fn visual_styles_match(&self, a: &engine::Style, b: &engine::Style) -> bool {
        let local = |style: &engine::Style| {
            style
                .opaque_style_id
                .and_then(|id| self.styles.get(id))
                .map_or_else(LocalStyle::default, |entry| entry.style)
        };
        a.color == b.color
            && a.shadow_color == b.shadow_color
            && a.bold == b.bold
            && a.italic == b.italic
            && a.underlined == b.underlined
            && a.strikethrough == b.strikethrough
            && a.obfuscated == b.obfuscated
            && a.font == b.font
            && a.insertion == b.insertion
            && (a.opaque_style_id == b.opaque_style_id || style_eq::same_events(local(a), local(b)))
    }

    pub fn style_to_api(&self, style: &engine::Style) -> TextStyle {
        let Some(orig) = style.opaque_style_id.and_then(|id| self.styles.get(id)) else {
            return style_to_api_inprocess(style);
        };
        let orig = orig.style;
        let mut out = TextStyle::default();
        out.color = style.color.map(|v| format!("#{v:06X}"));
        out.shadow_color = style.shadow_color;
        out.bold = style.bold;
        out.italic = style.italic;
        out.underlined = style.underlined;
        out.strikethrough = style.strikethrough;
        out.obfuscated = style.obfuscated;
        out.font = orig.font.map(str::to_owned);
        out.insertion = orig.insertion.map(str::to_owned);
        out.click = orig.click.cloned();
        out.hover = orig.hover.cloned();
        if let Some((mut tag, mut prior)) = orig.metadata.and_then(|id| self.styles[id].layer) {
            let mut overlays = Vec::new();
            // Reuse the oldest compound's map order, then apply each child's overrides from
            // oldest to newest. A non-compound either replaces its ancestor or starts a new map.
            while let (Tag::Compound(map), Some((older @ Tag::Compound(_), next))) =
                (tag, prior.and_then(|id| self.styles[id].layer))
            {
                overlays.push(map);
                tag = older;
                prior = next;
            }
            out.metadata = nbt::clone_tag(tag);
            if let Tag::Compound(base) = &mut out.metadata {
                for overlay in overlays.into_iter().rev() {
                    for (key, value) in overlay {
                        if let Some(replaced) = base.insert(key.clone(), nbt::clone_tag(value)) {
                            nbt::discard_tag(replaced);
                        }
                    }
                }
            }
        }
        out
    }
    pub fn text_to_api(&self, text: &engine::Text) -> Text {
        let capture_span = |span: &engine::Span| {
            let mut text = if span.style.inline_object.is_some() {
                capture_with(Source::Span(span), true).expect("lenient capture is infallible")
            } else {
                Text::literal(span.text.as_str())
            };
            text.style = self.style_to_api(&span.style);
            text
        };
        match text.spans.as_slice() {
            [] => Text::literal(""),
            [only] => capture_span(only),
            spans => {
                let mut root = Text::literal("");
                root.children = spans.iter().map(capture_span).collect();
                root
            }
        }
    }
}
/// Convert API text for in-process layout/shaping while retaining every
/// inherited `TextStyle` (including deep NBT click/hover payloads and opaque
/// metadata) verbatim in [`LocalText`].
pub fn to_local_engine(text: &Text) -> LocalText<'_> {
    let (ids, styles) = assign_local_styles(text);
    let value =
        owned_text_component_with(text, true, Some(&ids)).expect("lenient emit is infallible");
    let engine = component::text_with_local_styles(&value);
    component::discard_component(value);
    LocalText { engine, styles }
}
/// Encode API text for a serialization boundary (Wire, JSON/SNBT, NBT/network),
/// enforcing the 512-node depth limit and `ExtraCodecs` profile/property bounds.
pub fn to_serialized_engine(text: &Text) -> Result<engine::Text, TextFormError> {
    text.validate()?;
    to_engine(text)
}

#[cfg(test)]
mod text_capture_tests {
    use super::{TryIntoApi, TryIntoEngine};

    #[test]
    fn missing_translation_template_survives_typed_iterative_boundary() {
        let language = sunrise_game_data::lang::Language::of(
            "en_us",
            [("Hi %s".into(), "Wrong translation".into())],
        );
        sunrise_game_data::lang::scoped(language, || {
            let engine = sunrise_game_data::text::Text::translate_with_fallback(
                "test.missing",
                "Hi %s",
                &[sunrise_game_data::text::Text::literal("Steve")],
            );
            let api = engine.try_into_api().unwrap();
            assert!(
                matches!(&api.content, mod_api::text::TextContent::Translatable { fallback: Some(template), .. } if template == "Hi %s")
            );
            assert_eq!(api.try_into_engine(&()).unwrap().plain(), "Hi Steve");
        });
    }

    /// `Text::translate_with_fallback` (#5389): a language without the key formats the fallback
    /// with the arguments, and one with the key uses its own template.
    #[test]
    fn a_mods_translation_with_a_fallback_formats_it_where_the_key_is_missing() {
        let text = mod_api::Text::translate_with_fallback(
            "item.container.item_count",
            "%s x %s",
            vec![mod_api::Text::literal("Stone"), mod_api::Text::literal("3")],
        );
        let missing = sunrise_game_data::lang::Language::of("en_us", []);
        sunrise_game_data::lang::scoped(missing, || {
            assert_eq!(text.try_into_engine(&()).unwrap().plain(), "Stone x 3");
        });
        let present = sunrise_game_data::lang::Language::of(
            "en_us",
            [("item.container.item_count".into(), "%s, %s of them".into())],
        );
        sunrise_game_data::lang::scoped(present, || {
            assert_eq!(
                text.try_into_engine(&()).unwrap().plain(),
                "Stone, 3 of them"
            );
        });
    }

    #[test]
    fn native_translation_fallback_survives_noop_owned_round_trip() {
        let language = sunrise_game_data::lang::Language::of(
            "en_us",
            [("test.fallback".into(), "Wrong translation".into())],
        );
        sunrise_game_data::lang::scoped(language, || {
            let native = sunrise_game_data::text::Text::translate_with_fallback(
                "test.missing",
                "test.fallback",
                &[],
            );
            let mut api = native.try_into_api().unwrap();
            let listener = |_: &mut mod_api::text::Text| {};
            listener(&mut api);
            let actual = api.try_into_engine(&()).unwrap();
            assert_eq!(actual.plain(), "test.fallback");
            assert_eq!(actual.spans, native.spans);
            assert_eq!(actual.component(), native.component());
        });
    }
    use mod_api::text::{TextContent as C, TextHover};
    use serde_json::json;
    use sunrise_game_data::text::Text as EngineText;

    #[test]
    fn item_and_entity_hovers_survive_a_round_trip() {
        for hover in [
            json!({"action":"show_item", "id":"minecraft:stick", "count":3, "components":{"minecraft:rarity":"epic"}}),
            json!({"action":"show_entity", "id":"minecraft:pig", "uuid":[0,1,0,2], "name":{"translate":"test.name"}}),
        ] {
            let source = json!({"text":"x", "hover_event":hover});
            let api = EngineText::from(source).try_into_api().unwrap();
            let engine = api.try_into_engine(&()).unwrap();
            assert_eq!(engine.component()["hover_event"], hover);
        }
    }

    #[test]
    fn a_span_style_hover_matches_the_component_capture() {
        for hover in [
            json!({"action":"show_text", "value":{"text":"hi", "bold":true}}),
            json!({"action":"show_item", "id":"minecraft:stick", "count":3, "components":{"minecraft:rarity":"epic"}}),
            json!({"action":"show_entity", "id":"minecraft:pig", "uuid":[0,1,0,2], "name":"Babe"}),
            json!({"action":"show_entity", "id":"minecraft:pig", "uuid":"11223344-5566-7788-99aa-bbccddeeff00"}),
        ] {
            let source = json!({"text":"x", "hover_event":hover});
            let engine = EngineText::from(source);
            let expected = engine.try_into_api().unwrap().style.hover.take();
            assert!(expected.is_some());
            assert_eq!(super::style_hover(&engine.spans[0].style), expected);
        }
        let plain = EngineText::from(json!({"text":"x"}));
        assert_eq!(super::style_hover(&plain.spans[0].style), None);
    }

    #[test]
    fn untrusted_file_action_cannot_become_local_after_a_noop_value_round_trip() {
        let source = json!({"text":"server", "click_event":{"action":"open_file", "path":"test-file"},
            "extra":[{"text":"child", "clickEvent":{"action":"open_file", "value":"test-file"}}]});
        let mut api = EngineText::from(source).try_into_api().unwrap();
        assert!(api.style.click.is_none());
        assert!(api.metadata.get("click_event").is_none());
        assert!(api.style.metadata.get("click_event").is_none());
        assert!(api.children[0].style.click.is_none());
        let listener = |_: &mut mod_api::text::Text| {};
        listener(&mut api);
        let engine = api.try_into_engine(&()).unwrap();
        assert!(engine.spans.iter().all(|span| span.style.click.is_none()));
        let component = engine.component();
        assert!(component.get("click_event").is_none());
        assert!(component["extra"][0].get("clickEvent").is_none());
    }

    #[test]
    fn component_tree_capture_keeps_dynamic_contents_and_unknown_metadata() {
        let source = json!({"translate":"test.message", "fallback":"%s", "with":[{"text":"name", "color":"red"}],
            "bold":false, "test:meta":{"n":7}, "click_event":{"action":"custom", "id":"test:open", "payload":{"n":9}, "test:trace":1},
            "extra":[{"nbt":"value", "storage":"test:data", "interpret":true, "separator":{"text":","}},
                {"score":{"name":"@s", "objective":"test", "test:trace":2}}]});
        let api = EngineText::from(source.clone()).try_into_api().unwrap();
        assert!(
            matches!(&api.content, C::Translatable { key, fallback:Some(fallback), arguments } if key == "test.message" && fallback == "%s" && arguments.len() == 1)
        );
        assert_eq!(api.style.bold, Some(false));
        assert!(
            matches!(&api.children[0].content, C::Nbt { path, interpret:true, .. } if path == "value")
        );
        assert_eq!(
            api.style.metadata.get("click_event"),
            Some(&mod_api::Tag::compound([(
                "test:trace",
                mod_api::Tag::Byte(1)
            )]))
        );
        assert_eq!(api.try_into_engine(&()).unwrap().component(), source);
    }

    #[test]
    fn object_fallback_round_trip_keeps_omitted_styled_and_translatable() {
        for object in [
            json!({"object":"player", "player":{"name":"Test", "properties":[{"name":"textures", "value":"data", "signature":"signed"}]}, "hat":true}),
            json!({"object":"atlas", "atlas":"minecraft:blocks", "sprite":"test:icon"}),
        ] {
            for fallback in [
                None,
                Some(
                    json!({"text":"label", "italic":false, "extra":[{"text":"child", "bold":true}]}),
                ),
                Some(json!({"translate":"test.head", "with":[{"text":"name"}]})),
            ] {
                let mut source = object.clone();
                if let Some(fallback) = fallback {
                    source["fallback"] = fallback;
                }
                let api = EngineText::from(source.clone()).try_into_api().unwrap();
                assert_eq!(api.try_into_engine(&()).unwrap().component(), source);
            }
        }
    }

    #[test]
    fn hover_uuid_forms_keep_identity_and_unresolved_name() {
        for uuid in [
            json!("11223344-5566-7788-99aa-bbccddeeff00"),
            json!([287454020, 1432778632, -1716864052_i64, -571539712]),
        ] {
            let source = json!({"text":"entity", "hover_event":{"action":"show_entity", "id":"minecraft:pig", "uuid":uuid, "name":{"translate":"test.name"}, "test:trace":true}});
            let api = EngineText::from(source).try_into_api().unwrap();
            let Some(TextHover::Entity {
                id,
                name: Some(name),
                ..
            }) = &api.style.hover
            else {
                panic!("entity hover")
            };
            assert_eq!(*id, [0x1122334455667788, 0x99aabbccddeeff00]);
            assert!(matches!(&name.content, C::Translatable { key, .. } if key == "test.name"));
            assert_eq!(
                api.try_into_engine(&()).unwrap().component()["hover_event"],
                json!({"action":"show_entity", "id":"minecraft:pig", "uuid":[287454020,1432778632,-1716864052_i64,-571539712], "name":{"translate":"test.name"}, "test:trace":1})
            );
        }
    }

    #[test]
    fn style_capture_reuses_snbt_flags_and_rgba_shadow_rules() {
        let source = json!({"text":"styled", "bold":0, "italic":1, "shadow_color":[1,0,0.5,1]});
        let api = EngineText::from(source).try_into_api().unwrap();
        assert_eq!(api.style.bold, Some(false));
        assert_eq!(api.style.italic, Some(true));
        assert_eq!(api.style.shadow_color, Some(0xffff007f));
    }
}

#[cfg(test)]
mod text_encoding_tests {
    fn test_profile(signed: bool) -> mod_api::text::TextProfile {
        let mut profile = mod_api::text::TextProfile::default();
        profile.name = Some("Test".into());
        if signed {
            let mut p = mod_api::text::TextProfileProperty::default();
            p.name = "textures".into();
            p.value = "data".into();
            p.signature = Some("signed".into());
            profile.properties.push(p);
        }
        profile
    }
    use super::TryIntoEngine;
    use mod_api::text::{Text, TextClick, TextContent as C, TextHover, TextNbtSource, TextObject};
    use serde_json::json;

    #[test]
    fn owned_text_keeps_unresolved_translation_style_and_ordered_children() {
        let mut text = Text::translate("test.message", vec![Text::literal("7")]);
        text.style.bold = Some(false);
        text.style.color = Some("red".into());
        text.style.shadow_color = Some(0xff001122);
        text.style.click = Some(TextClick::RunCommand("/test".into()));
        text.style.metadata = mod_api::Tag::compound([(
            "click_event",
            mod_api::Tag::compound([("test:extra", mod_api::Tag::Byte(1))]),
        )]);
        text.metadata = mod_api::Tag::compound([(
            "test:context",
            mod_api::Tag::compound([("value", mod_api::Tag::Int(7))]),
        )]);
        text.children.push(Text::literal("first"));
        text.children.push(Text::literal("second"));
        let engine: Result<sunrise_game_data::text::Text, mod_api::text::TextFormError> =
            text.try_into_engine(&());
        assert_eq!(
            engine.unwrap().component(),
            json!({
                "translate":"test.message", "with":[7], "bold":false, "color":"red",
                "shadow_color":-16772830, "click_event":{"action":"run_command", "command":"/test", "test:extra":1},
                "test:context":{"value":7}, "extra":[{"text":"first"}, {"text":"second"}]
            })
        );
    }

    #[test]
    fn owned_text_retains_dynamic_sources_and_inline_profile_properties() {
        for (content, expected) in [
            (C::Keybind("key.jump".into()), json!({"keybind":"key.jump"})),
            (
                C::Selector {
                    pattern: "@p".into(),
                    separator: Some(Box::new(Text::literal(","))),
                },
                json!({"selector":"@p", "separator":{"text":","}}),
            ),
            (
                C::Score {
                    name: "@s".into(),
                    objective: "test".into(),
                },
                json!({"score":{"name":"@s", "objective":"test"}}),
            ),
            (
                C::Nbt {
                    path: "value".into(),
                    interpret: true,
                    source: TextNbtSource::Storage("test:data".into()),
                    separator: None,
                },
                json!({"nbt":"value", "interpret":true, "storage":"test:data"}),
            ),
            (
                C::Object(TextObject::Player {
                    profile: test_profile(true),
                    hat: false,
                    fallback: Some(Box::new(Text::literal("head"))),
                }),
                json!({"object":"player", "player":{"name":"Test", "properties":[{"name":"textures", "value":"data", "signature":"signed"}]}, "hat":false, "fallback":{"text":"head"}}),
            ),
        ] {
            let engine = Text::new(content).try_into_engine(&()).unwrap();
            assert_eq!(engine.component(), expected);
        }
    }

    #[test]
    fn object_fallback_encoder_keeps_default_styling_and_translation_tree() {
        let mut styled = Text::literal("styled");
        styled.style.italic = Some(false);
        for player in [true, false] {
            for (fallback, expected) in [
                (None, None),
                (
                    Some(Box::new(styled.clone())),
                    Some(json!({"text":"styled", "italic":false})),
                ),
                (
                    Some(Box::new(Text::translate("test.head", vec![]))),
                    Some(json!({"translate":"test.head"})),
                ),
            ] {
                let object = if player {
                    TextObject::Player {
                        profile: test_profile(false),
                        hat: true,
                        fallback,
                    }
                } else {
                    TextObject::Atlas {
                        atlas: "minecraft:blocks".into(),
                        sprite: "test:icon".into(),
                        fallback,
                    }
                };
                let engine = Text::new(C::Object(object)).try_into_engine(&()).unwrap();
                match expected {
                    None => {
                        assert!(engine.component().get("fallback").is_none());
                        assert_eq!(
                            engine.plain(),
                            if player { "[Test head]" } else { "[test:icon]" }
                        );
                    }
                    Some(expected) => assert_eq!(engine.component()["fallback"], expected),
                }
            }
        }
    }

    #[test]
    fn raw_file_action_metadata_does_not_reach_the_engine() {
        let mut text = Text::literal("remote");
        for metadata in [&mut text.metadata, &mut text.style.metadata] {
            for field in ["click_event", "clickEvent"] {
                let mod_api::Tag::Compound(fields) = metadata else {
                    unreachable!()
                };
                fields.insert(
                    field.into(),
                    mod_api::Tag::compound([
                        ("action", mod_api::Tag::String("open_file".into())),
                        ("path", mod_api::Tag::String("test-file".into())),
                    ]),
                );
            }
        }
        let engine = text.try_into_engine(&()).unwrap();
        assert!(engine.spans.iter().all(|span| span.style.click.is_none()));
        assert!(engine.component().get("click_event").is_none());
        assert!(engine.component().get("clickEvent").is_none());
    }

    #[test]
    fn owned_text_keeps_hover_uuid_name_and_component_removal_patch() {
        let mut text = Text::literal("entity");
        text.style.hover = Some(TextHover::Entity {
            kind: "minecraft:pig".into(),
            id: [0x1122334455667788, 0x99aabbccddeeff00],
            name: Some(Box::new(Text::literal("name"))),
        });
        let engine = text.try_into_engine(&()).unwrap();
        assert_eq!(
            engine.component()["hover_event"],
            json!({"action":"show_entity", "id":"minecraft:pig", "uuid":[287454020,1432778632,-1716864052_i64,-571539712], "name":{"text":"name"}})
        );
        text.style.hover = Some(TextHover::Item {
            id: "minecraft:stone".into(),
            count: 3,
            components: Some(mod_api::Tag::compound([(
                "!minecraft:custom_name",
                mod_api::Tag::Compound(Default::default()),
            )])),
        });
        let engine = text.try_into_engine(&()).unwrap();
        assert_eq!(
            engine.component()["hover_event"],
            json!({"action":"show_item", "id":"minecraft:stone", "count":3, "components":{"!minecraft:custom_name":{}}})
        );
    }
}

#[cfg(test)]
mod iterative_tests {
    use super::*;
    use mod_api::wire::Writer;
    use serde_json::json;

    #[test]
    fn default_stack_builders_build_cross_clone_and_drop_512_deep_text() {
        // Integrated and network workers use named builders without stack_size;
        // exercise that default as well as std::thread::spawn's unnamed default.
        for name in [None, Some("Server thread"), Some("sunrise-net")] {
            let builder = std::thread::Builder::new();
            let builder = match name {
                Some(name) => builder.name(name.into()),
                None => builder,
            };
            builder
                .spawn(|| {
                    let mut text = mod_api::Text::literal("leaf");
                    for _ in 1..512 {
                        let mut parent = mod_api::Text::literal("parent");
                        parent.children.push(text);
                        text = parent;
                    }
                    let engine = text.try_into_engine(&()).unwrap();
                    let restored = engine.try_into_api().unwrap();
                    assert_eq!(api_nodes(&restored, false).len(), 512);
                    drop((text.clone(), restored.clone(), engine.clone()));
                    drop((text, restored, engine));
                })
                .unwrap()
                .join()
                .unwrap();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn recursive_fields_survive_512_nodes_on_256_kib() {
        std::thread::Builder::new().stack_size(256 * 1024).spawn(|| {
            for (relation, depth) in (0..9).flat_map(|r| [(r, 512), (r, 513)]) {
                let mut source = json!({"text":"leaf", "bold":false, "test:metadata":17});
                for _ in 1..depth {
                    let mut parent = match relation {
                        0 => json!({"text":"parent"}),
                        1 => json!({"translate":"test:translation", "fallback":"%s"}),
                        2 => json!({"selector":"@s"}),
                        3 => json!({"object":"atlas", "sprite":"test:icon"}),
                        6 => json!({"nbt":"name", "storage":"test:data", "interpret":true}),
                        7 => json!({"object":"player", "player":{"name":"test"}}),
                        8 => json!({"text":""}),
                        _ => json!({"text":"parent"}),
                    };
                    let fields = parent.as_object_mut().unwrap();
                    match relation {
                        0 | 1 => { fields.insert(if relation == 0 { "extra" } else { "with" }.into(), serde_json::Value::Array(vec![source])); }
                        2 | 3 | 6 | 7 => { fields.insert(if relation == 2 || relation == 6 { "separator" } else { "fallback" }.into(), source); }
                        _ => {
                            let mut hover = if relation == 4 { json!({"action":"show_text"}) } else { json!({"action":"show_entity", "id":"minecraft:pig", "uuid":[0,0,0,1]}) };
                            hover.as_object_mut().unwrap().insert(if relation == 4 { "value" } else { "name" }.into(), source);
                            fields.insert("hover_event".into(), hover);
                        }
                    }
                    source = parent;
                }
                let engine = sunrise_game_data::text::Text::empty().with_component(source);
                let api = engine.try_into_api().unwrap();
                let nodes = api_nodes(&api, false);
                assert_eq!(nodes.len(), depth);
                assert!(matches!(&nodes[0].content, TextContent::Literal(s) if s == "leaf"));
                let emitted = api.try_into_engine(&()).unwrap();
                let restored = emitted.try_into_api().unwrap();
                assert_eq!(api_nodes(&restored, false).len(), depth);
                if depth == 513 {
                    // Serialization boundaries enforce the 512-node depth limit.
                    let mut wire = Writer::new();
                    wire.put(&api);
                    assert_eq!(wire.error(), Some(mod_api::wire::WireError::Depth));
                    assert!(matches!(
                        to_serialized_engine(&api),
                        Err(mod_api::text::TextFormError::TooDeep)
                    ));
                    std::mem::forget((engine, api, emitted, restored));
                    continue;
                }
                // Compare the full trees without calling recursive PartialEq on this small stack.
                let mut expected = Writer::new();
                expected.put(&api);
                assert_eq!(expected.error(), None);
                let mut actual = Writer::new();
                actual.put(&restored);
                assert_eq!(actual.error(), None);
                assert_eq!(actual.bytes(), expected.bytes());
                // Recursive ownership operations are not the crossing guarantee.
                std::mem::forget((engine, api, emitted, restored));
            }
        }).unwrap().join().unwrap();
    }

    #[test]
    fn too_deep_is_refused_before_cloning_opaque_metadata() {
        // Keep opaque metadata much deeper than the small stack could recursively clone.
        // At serialization boundaries, the component-depth refusal must win before any
        // opaque metadata is inspected or copied.
        let mut opaque = Tag::Byte(1);
        for _ in 0..4096 {
            opaque = Tag::List(vec![opaque]);
        }
        let mut api = mod_api::Text::literal("leaf");
        for _ in 1..513 {
            api = mod_api::Text::literal("parent").append(api);
        }
        api.metadata = Tag::compound([("test:opaque", opaque)]);
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                assert!(matches!(
                    to_serialized_engine(&api),
                    Err(mod_api::text::TextFormError::TooDeep)
                ));
                let mut wire = Writer::new();
                wire.put(&api);
                assert_eq!(wire.error(), Some(mod_api::wire::WireError::Depth));
                std::mem::forget(api);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn span_only_hover_sources_are_validated_and_materialized_iteratively() {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                use sunrise_game_data::text::{Span, Style, Text as Engine};
                for depth in [512, 513] {
                    let mut text = Engine::from_spans(vec![Span::new("leaf", Style::default())]);
                    for _ in 1..depth {
                        text = Engine::from_spans(vec![Span::new(
                            "parent",
                            Style {
                                hover: Some(std::sync::Arc::new(text)),
                                ..Style::default()
                            },
                        )]);
                    }
                    let api = text.try_into_api().unwrap();
                    assert_eq!(api_nodes(&api, false).len(), depth);
                    if depth == 513 {
                        assert!(matches!(
                            to_serialized_engine(&api),
                            Err(mod_api::text::TextFormError::TooDeep)
                        ));
                    }
                    std::mem::forget((text, api));
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn inprocess_text_allows_deep_trees_and_unbounded_profiles_while_serialization_enforces_limits()
    {
        // Depth > 512 is valid in memory (Component.java has no depth limit in memory).
        let mut deep = mod_api::Text::literal("leaf");
        for _ in 0..600 {
            deep = mod_api::Text::literal("p").append(deep);
        }
        let engine = to_engine(&deep).unwrap();
        assert_eq!(engine.plain().len(), 604);
        assert_eq!(to_engine_inprocess(&deep).plain().len(), 604);
        let roundtrip = to_api(&engine).unwrap();
        assert_eq!(api_nodes(&roundtrip, false).len(), 601);
        assert_eq!(api_nodes(&to_api_inprocess(&engine), false).len(), 601);
        assert_eq!(to_serialized_engine(&deep), Err(TextFormError::TooDeep));
        let mut wire = Writer::new();
        wire.put(&deep);
        assert_eq!(wire.error(), Some(mod_api::wire::WireError::Depth));

        // Player profiles exceeding ExtraCodecs serialization limits survive in memory,
        // while JSON codec / Wire / serialization boundaries enforce ExtraCodecs bounds.
        let mut profile = TextProfile::default();
        profile.name = Some("Player Name Over Sixteen".into());
        for i in 0..17 {
            let mut prop = TextProfileProperty::default();
            prop.name = format!("prop_{i}");
            prop.value = "v".into();
            prop.signature = Some("sig".into());
            profile.properties.push(prop);
        }
        let player = mod_api::Text::new(TextContent::Object(TextObject::Player {
            profile,
            hat: true,
            fallback: None,
        }));
        let engine_player = to_engine(&player).unwrap();
        let api_player = to_api(&engine_player).unwrap();
        assert_eq!(api_player, player);
        assert_eq!(to_api_inprocess(&to_engine_inprocess(&player)), player);
        assert_eq!(to_serialized_engine(&player), Err(TextFormError::Malformed));
        assert_eq!(
            decoded_profile_json(&engine_player.component()["player"]),
            None
        );
        let mut wire = Writer::new();
        wire.put(&player);
        assert!(matches!(
            wire.error(),
            Some(mod_api::wire::WireError::Invalid(_))
        ));

        // Non-compound metadata is refused by to_engine and safely ignored by to_engine_inprocess.
        let mut bad_meta = mod_api::Text::literal("ok");
        bad_meta.metadata = Tag::Int(7);
        assert_eq!(to_engine(&bad_meta), Err(TextFormError::Malformed));
        assert_eq!(to_engine_inprocess(&bad_meta).plain(), "ok");
    }

    #[test]
    fn local_engine_preserves_opaque_click_hover_and_metadata_across_legacy_split() {
        use super::{Tag, TextClick, TextHover, TextStyle, to_local_engine};
        let mut opaque_dialog = Tag::ByteArray(vec![1, 2, 3]);
        for _ in 0..600 {
            let mut m = sunrise_core::nbt::Compound::new();
            m.insert("n".into(), opaque_dialog);
            opaque_dialog = Tag::Compound(m);
        }
        let mut custom_meta = sunrise_core::nbt::Compound::new();
        custom_meta.insert("non_finite".into(), Tag::Float(f32::NAN.to_bits()));

        let mut parent_style = TextStyle::default();
        parent_style.color = Some("gold".into());
        parent_style.click = Some(TextClick::ShowDialog(opaque_dialog.clone()));
        parent_style.hover = Some(TextHover::Item {
            id: "minecraft:stone".into(),
            count: 2,
            components: Some(Tag::ByteArray(vec![9, 8])),
        });
        parent_style.metadata = Tag::Compound(custom_meta.clone());

        let mut root = mod_api::Text::literal("hi §lbold §cred").with_style(parent_style);
        let child = mod_api::Text::literal(" tail");
        root.children.push(child);

        let local = to_local_engine(&root);
        let resolved = local.engine().resolved();
        let rebuilt = local.text_to_api(&resolved);
        assert_eq!(rebuilt.children.len(), 4);
        assert_eq!(rebuilt.children[0].style.color.as_deref(), Some("#FFAA00"));
        assert_eq!(
            rebuilt.children[0].style.click,
            Some(TextClick::ShowDialog(opaque_dialog.clone()))
        );
        assert_eq!(
            rebuilt.children[0].style.metadata,
            Tag::Compound(custom_meta.clone())
        );
        assert_eq!(rebuilt.children[1].style.bold, Some(true));
        assert_eq!(
            rebuilt.children[1].style.click,
            Some(TextClick::ShowDialog(opaque_dialog.clone()))
        );
        assert_eq!(rebuilt.children[2].style.color.as_deref(), Some("#FF5555"));
        assert_eq!(rebuilt.children[2].style.bold, Some(false));
        assert_eq!(
            rebuilt.children[2].style.click,
            Some(TextClick::ShowDialog(opaque_dialog.clone()))
        );
        assert_eq!(rebuilt.children[3].style.color.as_deref(), Some("#FFAA00"));
        assert_eq!(
            rebuilt.children[3].style.click,
            Some(TextClick::ShowDialog(opaque_dialog))
        );
        assert_eq!(
            rebuilt.children[3].style.metadata,
            Tag::Compound(custom_meta)
        );
    }

    #[test]
    fn array_root_and_children_keep_order_and_metadata() {
        let source = json!([
            {"text":"root", "extra":[{"text":"first"}], "bold":false},
            {"text":"second", "test:metadata":{"id":7}}
        ]);
        let api = captured_text(&source).unwrap();
        assert_eq!(api.children.len(), 2);
        assert_eq!(
            owned_text_component(&api).unwrap(),
            json!({
                "text":"root", "bold":false,
                "extra":[{"text":"first"}, {"text":"second", "test:metadata":{"id":7}}]
            })
        );
    }

    #[test]
    fn component_view_borrows_retained_source() {
        let engine =
            sunrise_game_data::text::Text::empty().with_component(json!({"text":"source"}));
        assert!(matches!(
            engine.component_view(),
            std::borrow::Cow::Borrowed(_)
        ));
    }
}

#[cfg(test)]
trait TryIntoApi {
    fn try_into_api(&self) -> Result<Text, TextFormError>;
}
#[cfg(test)]
impl TryIntoApi for engine::Text {
    fn try_into_api(&self) -> Result<Text, TextFormError> {
        to_api(self)
    }
}
#[cfg(test)]
trait TryIntoEngine<C: ?Sized> {
    fn try_into_engine(&self, context: &C) -> Result<engine::Text, TextFormError>;
}
#[cfg(test)]
impl<C: ?Sized> TryIntoEngine<C> for Text {
    fn try_into_engine(&self, _: &C) -> Result<engine::Text, TextFormError> {
        to_engine(self)
    }
}
