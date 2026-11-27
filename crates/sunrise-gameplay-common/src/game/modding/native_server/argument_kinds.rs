//! The argument kinds a native command usage names: the built-in ones, each a Brigadier
//! parser, and the types mods registered, with the dispatcher type each node parses as.

use super::{ArgType, Text, TryIntoEngine};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use sunrise_command::{
    CommandError, Dispatcher, NodeKind, Range, Reader, RegistryId, args::Coordinates, snbt,
};

/// The argument kinds a native usage may name, each with a [`parser`]; `resource:<registry>`,
/// the bounded `integer(min,max)` and `long(min,max)` ([`bounded_parser`]) and `mod_id` come on
/// top.
pub(in crate::game) const KINDS: [&str; 23] = [
    "word",
    "string",
    "text",
    "integer",
    "long",
    "number",
    "boolean",
    "player",
    "game_profile",
    "entity",
    "entities",
    "players",
    "item",
    "block_pos",
    "vec3",
    "vec3_uncentered",
    "vec2",
    "vec2_uncentered",
    "column_pos",
    "rotation",
    "dimension",
    "nbt",
    "id",
];

/// The tree property marking `vec3_uncentered` and `vec2_uncentered`: vanilla's `vec3(false)` and
/// `vec2(false)` have the same parser id and properties as `vec3()` and `vec2()`, so the report
/// format cannot tell them apart.
pub const UNCENTERED: &str = "uncentered";

/// The custom argument types the mods registered, by namespaced id, for the command tree being
/// built and its server dispatcher.
pub type CommandArguments = HashMap<String, Arc<dyn mod_api::CommandArgumentType>>;

/// The argument types the installed mods registered at startup, and the built-in `mod_id`:
/// NeoForge's `ModIdArgument`, one of the loaded package ids, delivered as a string.
pub fn command_arguments() -> CommandArguments {
    let mod_ids = mod_api::__private::loaded_mods()
        .ids()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mod_id: Arc<dyn mod_api::CommandArgumentType> =
        Arc::new(mod_api::ChoiceArgument::new(mod_ids));
    mod_api::__private::command_argument_types()
        .into_iter()
        .chain([(MOD_ID.to_owned(), mod_id)])
        .collect()
}

/// The built-in kind whose value is a loaded package id. Mods' kinds are namespaced, so it
/// never meets one.
pub(in crate::game) const MOD_ID: &str = "mod_id";

/// The Brigadier parser and properties of `resource:<registry>`: `ResourceKeyArgument` on the
/// wire, so vanilla clients complete it, and `ResourceArgument` on the server, which refuses an id
/// the registry lacks before the mod runs.
pub fn resource_parser(name: &str) -> Result<(&'static str, Value), String> {
    if mod_api::Identifier::parse_namespaced(name).is_none() {
        return Err(format!(
            "registry `{name}` is not a `namespace:path` resource location"
        ));
    }
    Ok(("minecraft:resource_key", json!({ "registry": name })))
}

/// The Brigadier parser and properties of `resource_or_tag:<registry>`: vanilla's
/// `ResourceOrTagKeyArgument` (`minecraft:resource_or_tag_key`), so the chat box completes
/// the registry's ids and `#` tags itself. Neither is checked against the registry when parsed,
/// as in vanilla; the registry's name must be a `namespace:path` resource location.
pub fn resource_or_tag_parser(name: &str) -> Result<(&'static str, Value), String> {
    if mod_api::Identifier::parse_namespaced(name).is_none() {
        return Err(format!(
            "registry `{name}` is not a `namespace:path` resource location"
        ));
    }
    Ok(("minecraft:resource_or_tag_key", json!({ "registry": name })))
}

/// What a mod receives for a custom argument: `mod_id` as the plain package id, a mod's own type
/// with its id.
pub fn custom_value(kind: &str, value: String) -> mod_api::CommandArgumentValue {
    if kind == MOD_ID {
        mod_api::CommandArgumentValue::String(value)
    } else {
        mod_api::CommandArgumentValue::Custom {
            kind: kind.to_owned(),
            value,
        }
    }
}

/// Whether `word` is a usage's literal word: lowercase letters, digits, `_` and `-`. The first
/// word may also start with one `/`, so a mod can register WorldEdit-style `//set` (typed with the
/// chat slash): the node keeps the slash, and the client completes `//set ` against it.
pub(in crate::game) fn command_word(word: &str, first: bool) -> bool {
    let word = match first {
        true => word.strip_prefix('/').unwrap_or(word),
        false => word,
    };
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-".contains(c))
}

/// The brigadier parser of a built-in argument kind, its properties and its dispatcher type.
/// `EntityArgument.entity` / `entities` / `player` / `players` share the vanilla parser.
pub fn parser(kind: &str) -> Option<(&'static str, Value, ArgType)> {
    Some(match kind {
        "word" => ("brigadier:string", json!({"type": "word"}), ArgType::Word),
        // `StringArgumentType.string()`: `StringArgumentSerializer` writes `QUOTABLE_PHRASE`.
        "string" => ("brigadier:string", json!({"type": "phrase"}), ArgType::Str),
        "text" => (
            "brigadier:string",
            json!({"type": "greedy"}),
            ArgType::Greedy,
        ),
        "integer" => (
            "brigadier:integer",
            Value::Null,
            ArgType::Integer {
                min: i32::MIN,
                max: i32::MAX,
            },
        ),
        "long" => (
            "brigadier:long",
            Value::Null,
            ArgType::Long {
                min: i64::MIN,
                max: i64::MAX,
            },
        ),
        "number" => (
            "brigadier:double",
            Value::Null,
            ArgType::Double {
                min: f64::MIN,
                max: f64::MAX,
            },
        ),
        "boolean" => ("brigadier:bool", Value::Null, ArgType::Bool),
        "game_profile" => ("minecraft:game_profile", Value::Null, ArgType::GameProfile),
        "entity" | "entities" | "player" | "players" => {
            let single = matches!(kind, "entity" | "player");
            let players = matches!(kind, "player" | "players");
            (
                "minecraft:entity",
                json!({
                    "amount": if single { "single" } else { "multiple" },
                    "type": if players { "players" } else { "entities" },
                }),
                ArgType::Entity { single, players },
            )
        }
        "item" => ("minecraft:item_stack", Value::Null, ArgType::Item),
        "block_pos" => ("minecraft:block_pos", Value::Null, ArgType::BlockPos),
        // `Vec3Argument.vec3()` centres whole x and z on their block, `vec3(false)` does not.
        "vec3" => (
            "minecraft:vec3",
            Value::Null,
            ArgType::Vec3 { center: true },
        ),
        "vec3_uncentered" => (
            "minecraft:vec3",
            Value::Null,
            ArgType::Vec3 { center: false },
        ),
        // `Vec2Argument.vec2()` centres whole x and z on their block, `vec2(false)` does not.
        "vec2" => (
            "minecraft:vec2",
            Value::Null,
            ArgType::Vec2 { center: true },
        ),
        "vec2_uncentered" => (
            "minecraft:vec2",
            Value::Null,
            ArgType::Vec2 { center: false },
        ),
        "column_pos" => ("minecraft:column_pos", Value::Null, ArgType::ColumnPos),
        "rotation" => ("minecraft:rotation", Value::Null, ArgType::Rotation),
        "dimension" => ("minecraft:dimension", Value::Null, ArgType::Dimension),
        "nbt" => ("minecraft:nbt_compound_tag", Value::Null, ArgType::Nbt),
        // `ResourceLocationArgument`: a free `namespace:path` of identifier characters, with no
        // registry to check. An unmodded client parses and refuses it the same way; the node
        // declares `minecraft:ask_server` (`usage_nodes`), so the identifiers it completes are the
        // mods' own (`tab_complete`).
        "id" => (
            "minecraft:resource_location",
            Value::Null,
            ArgType::Identifier,
        ),
        _ => return None,
    })
}

/// The parser and properties of a bounded number kind, `integer(min,max)` or `long(min,max)`,
/// where either bound may be left empty (`integer(0,)`): Brigadier's
/// `IntegerArgumentType.integer(min, max)` and `LongArgumentType.longArg(min, max)`. `None` for
/// any other kind; the reason for one that names `integer(` or `long(` but does not parse.
pub fn bounded_parser(kind: &str) -> Option<Result<(&'static str, Value), String>> {
    let (name, bounds) = kind.strip_suffix(')')?.split_once('(')?;
    let integer = match name {
        "integer" => true,
        "long" => false,
        _ => return None,
    };
    Some(declared_bounds(kind, integer, bounds))
}

/// The `min,max` of a bounded number kind, each an `i32` for `integer` and an `i64` for `long`.
fn declared_bounds(
    kind: &str,
    integer: bool,
    bounds: &str,
) -> Result<(&'static str, Value), String> {
    let bound = |text: &str| -> Result<Option<i64>, String> {
        if text.is_empty() {
            return Ok(None);
        }
        let value = match integer {
            true => text.parse::<i32>().map(i64::from),
            false => text.parse::<i64>(),
        };
        value
            .map(Some)
            .map_err(|_| format!("`{kind}`: `{text}` is not a bound of that kind"))
    };
    let (min, max) = bounds
        .split_once(',')
        .ok_or_else(|| format!("`{kind}` is not `(min,max)` bounds"))?;
    let (min, max) = (bound(min)?, bound(max)?);
    if let (Some(min), Some(max)) = (min, max)
        && max < min
    {
        return Err(format!("`{kind}`: the maximum is below the minimum"));
    }
    let parser = match integer {
        true => "brigadier:integer",
        false => "brigadier:long",
    };
    Ok((parser, number_properties(integer, min, max)))
}

/// `IntegerArgumentInfo` / `LongArgumentInfo`'s properties: a bound is written only when it is not
/// the type's own extreme (`min != Integer.MIN_VALUE`, `max != Integer.MAX_VALUE`), so `integer(,)`
/// is the bare `integer`.
fn number_properties(integer: bool, min: Option<i64>, max: Option<i64>) -> Value {
    let (low, high) = match integer {
        true => (i64::from(i32::MIN), i64::from(i32::MAX)),
        false => (i64::MIN, i64::MAX),
    };
    let mut properties = serde_json::Map::new();
    if let Some(min) = min.filter(|&min| min != low) {
        properties.insert("min".into(), min.into());
    }
    if let Some(max) = max.filter(|&max| max != high) {
        properties.insert("max".into(), max.into());
    }
    match properties.is_empty() {
        true => Value::Null,
        false => Value::Object(properties),
    }
}

/// The parser and properties a mod's argument type goes to clients as, from its declared kind.
/// `None` for a kind this build cannot read.
pub fn custom_parser(kind: mod_api::CommandArgumentKind) -> Option<(&'static str, Value)> {
    use mod_api::CommandArgumentKind as Kind;
    Some(match kind {
        Kind::Word => ("brigadier:string", json!({"type": "word"})),
        Kind::Greedy => ("brigadier:string", json!({"type": "greedy"})),
        Kind::Quotable => ("brigadier:string", json!({"type": "phrase"})),
        Kind::Integer { min, max } => (
            "brigadier:integer",
            number_properties(true, min.map(i64::from), max.map(i64::from)),
        ),
        Kind::Long { min, max } => ("brigadier:long", number_properties(false, min, max)),
        Kind::Id => ("minecraft:resource_location", Value::Null),
        _ => return None,
    })
}

/// A validated `brigadier:integer` node's bounds, `int`'s extremes where its properties leave
/// one out.
fn integer_bounds(node: &Value) -> (i32, i32) {
    let (min, max) = long_bounds(node);
    (
        i32::try_from(min).unwrap_or(i32::MIN),
        i32::try_from(max).unwrap_or(i32::MAX),
    )
}

/// A validated `brigadier:long` node's bounds, `long`'s extremes where its properties leave one
/// out.
fn long_bounds(node: &Value) -> (i64, i64) {
    let properties = &node["properties"];
    (
        properties["min"].as_i64().unwrap_or(i64::MIN),
        properties["max"].as_i64().unwrap_or(i64::MAX),
    )
}

/// The type of the argument node that parsed `range`: the parsed node, not the value, says which
/// kind a position, id or string is.
pub fn parsed_type<'a>(
    dispatcher: &'a Dispatcher,
    parsed_nodes: &[(usize, Range)],
    range: Range,
) -> Option<&'a ArgType> {
    parsed_nodes.iter().find_map(|(node, parsed)| {
        if *parsed != range {
            return None;
        }
        match &dispatcher.nodes[*node].kind {
            NodeKind::Argument { ty, .. } => Some(ty),
            _ => None,
        }
    })
}

/// What a `vec3`, `vec3_uncentered`, `column_pos`, `rotation` or `block_pos` argument delivers:
/// `position` is `coordinates` resolved against the source, `rotation` the source's yaw and pitch.
pub fn positional_value(
    ty: Option<&ArgType>,
    coordinates: &Coordinates,
    position: [f64; 3],
    rotation: [f32; 2],
) -> mod_api::CommandArgumentValue {
    use mod_api::CommandArgumentValue as Value;
    match ty {
        Some(ArgType::Vec3 { .. }) => Value::Vec3(position),
        Some(ArgType::Vec2 { .. }) => Value::Vec2([position[0] as f32, position[2] as f32]),
        Some(ArgType::ColumnPos) => {
            Value::ColumnPos([position[0].floor() as i32, position[2].floor() as i32])
        }
        Some(ArgType::Rotation) => Value::Rotation(coordinates.rotation(rotation)),
        _ => Value::BlockPos(position.map(|v| v.floor() as i32)),
    }
}

/// `DimensionArgument.getDimension`: the level of a dimension the world has loaded, or
/// `argument.dimension.invalid`.
pub fn dimension_value(
    id: &str,
    level: impl FnOnce(&str) -> Option<mod_api::LevelRef>,
) -> Result<mod_api::CommandArgumentValue, CommandError> {
    level(id)
        .map(mod_api::CommandArgumentValue::Dimension)
        .ok_or_else(|| CommandError::tr("argument.dimension.invalid", &[&id]))
}

/// An `nbt` argument's compound, from the SNBT the dispatcher accepted.
pub fn nbt_value(text: &str) -> Result<mod_api::CommandArgumentValue, CommandError> {
    let mut reader = Reader::new(text);
    snbt::parse_compound_argument(&mut reader).map(mod_api::CommandArgumentValue::Nbt)
}

/// The message a mod's argument parser refused with, as the engine's text. A text the
/// boundary cannot carry is reported as its own diagnostic: the refusal still fails the
/// argument, never silently passing it.
fn refused_argument_text(error: mod_api::CommandArgumentError) -> Text {
    let message: mod_api::Text = error.into();
    match message.try_into_engine(&()) {
        Ok(text) => text,
        Err(error) => Text::literal(error.to_string()),
    }
}

/// The dispatcher type of one validated tree node: a vanilla kind, or the mod's registered
/// argument type with its parser and suggestions.
pub fn argument_type(node: &Value, arguments: &CommandArguments) -> ArgType {
    if node["parser"] == "minecraft:resource_or_tag_key" {
        let registry = node["properties"]["registry"].as_str().unwrap_or_default();
        return ArgType::ResOrTagKey(
            RegistryId::parse(registry).expect("native command tree names validated registries"),
        );
    }
    if node["parser"] == "minecraft:resource_key" {
        let registry = node["properties"]["registry"].as_str().unwrap_or_default();
        return ArgType::Res(
            RegistryId::parse(registry).expect("native command tree names validated registries"),
        );
    }
    if let Some(id) = node["custom_argument"].as_str() {
        let argument = arguments
            .get(id)
            .unwrap_or_else(|| panic!("native command tree names unregistered argument type {id}"));
        let parse = argument.clone();
        let suggest = argument.clone();
        let kind_id = id.to_owned();
        // Use the validated tree properties rather than calling mod code again.
        use sunrise_command::CustomArgumentKind as Kind;
        let kind = match node["parser"].as_str() {
            Some("brigadier:integer") => {
                let (min, max) = integer_bounds(node);
                Kind::Integer { min, max }
            }
            Some("brigadier:long") => {
                let (min, max) = long_bounds(node);
                Kind::Long { min, max }
            }
            Some("minecraft:resource_location") => Kind::Id,
            _ if node["properties"]["type"] == "greedy" => Kind::Greedy,
            _ if node["properties"]["type"] == "phrase" => Kind::Quotable,
            _ => Kind::Word,
        };
        let custom = sunrise_command::CustomArgument::new(
            id,
            kind,
            move |input| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse.parse(input)))
                    .map_err(|_| Text::tr("commands.generic.failed", &[]))?
                    .map_err(refused_argument_text)
            },
            move |reg: &sunrise_command::Registries,
                  prefix: &str,
                  arguments: &[sunrise_command::ParsedArgument]| {
                // The mod decides what this sender may complete; the game keeps its answer as it
                // comes (`CommandSender::suggest`).
                let request = mod_api::SuggestionRequest::new(
                    prefix,
                    arguments
                        .iter()
                        .map(|argument| {
                            mod_api::TypedArgument::new(
                                argument.name.clone(),
                                argument.text.clone(),
                            )
                        })
                        .collect(),
                    reg.sender.as_ref().map(|sender| {
                        mod_api::__private::game::command_sender(
                            sender.uuid,
                            sender.name.clone(),
                            mod_api::PermissionLevel::from_level(i32::from(reg.permission)),
                        )
                    }),
                );
                let Ok(results) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    suggest.suggest(&request)
                })) else {
                    eprintln!(
                        "mod command argument type {kind_id} completer panicked; no suggestions"
                    );
                    return Vec::new();
                };
                results
                    .into_iter()
                    .map(|s| sunrise_command::Suggestion {
                        range: s.range,
                        text: s.text,
                        tooltip: s.tooltip,
                        int_value: s.int_value,
                    })
                    .collect()
            },
        );
        return ArgType::Custom(custom);
    }
    if node["parser"] == "minecraft:vec3" && node[UNCENTERED] == true {
        return ArgType::Vec3 { center: false };
    }
    if node["parser"] == "minecraft:vec2" && node[UNCENTERED] == true {
        return ArgType::Vec2 { center: false };
    }
    if node["parser"] == "brigadier:integer" {
        let (min, max) = integer_bounds(node);
        return ArgType::Integer { min, max };
    }
    if node["parser"] == "brigadier:long" {
        let (min, max) = long_bounds(node);
        return ArgType::Long { min, max };
    }
    KINDS
        .into_iter()
        .filter_map(parser)
        .find_map(|(id, properties, ty)| {
            (node["parser"] == id && node["properties"] == properties).then_some(ty)
        })
        .expect("native command tree contains registered parsers")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_kind_is_vanillas_free_identifier_parser() {
        let (parser, properties, ty) = super::parser("id").expect("the id kind has a parser");
        assert_eq!(parser, "minecraft:resource_location");
        assert_eq!(properties, Value::Null, "the parser takes no properties");
        assert!(matches!(ty, ArgType::Identifier), "{ty:?}");
        assert!(KINDS.contains(&"id"));
    }

    #[test]
    fn native_resource_arguments_own_registry_names_without_a_registration_budget() {
        for bad in [
            "a:b:c",
            "a/b:c",
            "item",
            "minecraft:",
            ":x",
            "Minecraft:x",
            "..:x",
        ] {
            let refusal = resource_parser(bad).unwrap_err();
            assert!(
                refusal.contains(bad) && refusal.contains("namespace:path"),
                "{refusal}"
            );
        }
        let arguments = CommandArguments::new();
        let names = (0..257).map(|n| format!("mymod:r{n}"));
        let long = format!("mymod:registry/{}", "b".repeat(300));
        for name in names.chain([long]) {
            let (parser, properties) = resource_parser(&name).unwrap();
            assert_eq!(parser, "minecraft:resource_key");
            let node = json!({ "parser": parser, "properties": properties });
            let ArgType::Res(registry) = argument_type(&node, &arguments) else {
                panic!("the server must check the registry for resource arguments");
            };
            drop(node);
            assert_eq!(registry.as_str(), name);
        }
    }
}
