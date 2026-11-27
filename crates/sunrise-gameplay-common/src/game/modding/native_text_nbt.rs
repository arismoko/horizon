//! The persistent `ComponentSerialization.CODEC` over NbtOps. The host owns
//! this conversion: no renderer, text-node preflight or JSON projection.
use mod_api::text::{Text, TextContent, TextFormError as Error, TextNbtSource, TextObject};
use sunrise_command::Registries;
use sunrise_core::identifier::Identifier;
use sunrise_core::nbt::{self, Compound, Limits, Tag};
use sunrise_sim::text_grammar::{KINDS, block_pos_parses, path_parses, selector_parses};

#[path = "native_text_nbt/profile.rs"]
mod profile;
#[path = "native_text_nbt/style.rs"]
mod style;
#[cfg(test)]
#[path = "native_text_nbt/tests.rs"]
mod tests;

const DEFAULT_ATLAS: &str = "minecraft:blocks";
const NBT_PLAIN: &str = "plain";
const PRIMITIVE_ARGUMENT: &str = "__sunrise_nbt_argument";

impl crate::game::CommonGame<'_> {
    pub fn text_from_nbt(&self, tag: &Tag) -> Result<Option<Text>, Error> {
        Codec {
            registries: &self.ui.registries,
            defer_holders: false,
        }
        .read(tag)
    }

    pub fn text_to_nbt(&self, text: &Text) -> Result<Tag, Error> {
        Codec {
            registries: &self.ui.registries,
            defer_holders: false,
        }
        .write(text)
    }
}

struct Codec<'a> {
    registries: &'a Registries,
    defer_holders: bool,
}

impl Codec<'_> {
    fn read(&self, tag: &Tag) -> Result<Option<Text>, Error> {
        if !nbt::within_depth(tag, Limits::default().max_depth) {
            return Err(Error::TooDeep);
        }
        Ok(self.decode(tag))
    }

    fn write(&self, text: &Text) -> Result<Tag, Error> {
        let tag = self.encode(text, 0)?;
        // Count the final binary layout: both nested field codecs and the
        // writer's list wrappers can add containers after component encoding.
        if !nbt::within_write_depth(&tag, Limits::default().max_depth) {
            nbt::discard_tag(tag);
            return Err(Error::TooDeep);
        }
        Ok(tag)
    }

    fn decode(&self, tag: &Tag) -> Option<Text> {
        nbt::with_tree_stack(|| match tag {
            Tag::String(value) => Some(Text::literal(value.to_string().ok()?)),
            Tag::List(items) => {
                let (first, rest) = items.split_first()?;
                let mut result = self.decode(first)?;
                for item in rest {
                    result.children.push(self.decode(item)?);
                }
                Some(result)
            }
            Tag::Compound(_) => {
                let mut text = match tag.get("type") {
                    Some(kind) => self.content(&string(kind)?, tag)?,
                    None => KINDS.into_iter().find_map(|kind| self.content(kind, tag))?,
                };
                text.style = self.decode_style(tag)?;
                if let Some(extra) = tag.get("extra") {
                    let Tag::List(items) = extra else { return None };
                    if items.is_empty() {
                        return None;
                    }
                    text.children = self.decode_list(items)?;
                }
                Some(text)
            }
            _ => None,
        })
    }

    fn decode_list(&self, items: &[Tag]) -> Option<Vec<Text>> {
        items.iter().map(|item| self.decode(item)).collect()
    }

    fn content(&self, kind: &str, tag: &Tag) -> Option<Text> {
        let value = |key: &str| string(tag.get(key)?);
        let child = |v: &Tag| self.decode(v).map(Box::new);
        let mut plain = false;
        let content = match kind {
            "text" => TextContent::Literal(value("text")?),
            "translatable" => TextContent::Translatable {
                key: value("translate")?,
                fallback: value("fallback"),
                arguments: optional(tag, "with", |list| self.arguments(list))?.unwrap_or_default(),
            },
            "keybind" => TextContent::Keybind(value("keybind")?),
            "score" => {
                let score = tag.get("score")?;
                TextContent::Score {
                    name: string(score.get("name")?)?,
                    objective: string(score.get("objective")?)?,
                }
            }
            "selector" => TextContent::Selector {
                pattern: value("selector").filter(|s| selector_parses(s, self.registries))?,
                separator: optional(tag, "separator", child)?,
            },
            "nbt" => {
                let path = value("nbt").filter(|p| path_parses(p))?;
                let flag = |key| tag.get(key).and_then(boolean).unwrap_or(false);
                let interpret = flag("interpret");
                plain = flag(NBT_PLAIN);
                if interpret && plain {
                    return None;
                }
                let source = |key| match key {
                    "entity" => value(key)
                        .filter(|s| selector_parses(s, self.registries))
                        .map(TextNbtSource::Entity),
                    "block" => value(key)
                        .filter(|s| block_pos_parses(s, self.registries))
                        .map(TextNbtSource::Block),
                    "storage" => id(tag.get(key)?).map(TextNbtSource::Storage),
                    _ => None,
                };
                TextContent::Nbt {
                    path,
                    interpret,
                    source: match tag.get("source") {
                        Some(kind) => source(&string(kind)?)?,
                        None => ["entity", "block", "storage"]
                            .into_iter()
                            .find_map(source)?,
                    },
                    // NbtContents' separator, like its flags, is lenient.
                    separator: tag.get("separator").and_then(child),
                }
            }
            "object" => {
                let fallback = optional(tag, "fallback", child)?;
                let atlas = || {
                    Some((
                        optional(tag, "atlas", id)?.unwrap_or_else(|| DEFAULT_ATLAS.into()),
                        id(tag.get("sprite")?)?,
                    ))
                };
                let player = || {
                    Some((
                        profile::decode(tag.get("player")?)?,
                        optional(tag, "hat", boolean)?.unwrap_or(true),
                    ))
                };
                TextContent::Object(match tag.get("object") {
                    Some(kind) => match string(kind)?.as_str() {
                        "atlas" => {
                            let (atlas, sprite) = atlas()?;
                            TextObject::Atlas {
                                atlas,
                                sprite,
                                fallback,
                            }
                        }
                        "player" => {
                            let (profile, hat) = player()?;
                            TextObject::Player {
                                profile,
                                hat,
                                fallback,
                            }
                        }
                        _ => return None,
                    },
                    None => match atlas() {
                        Some((atlas, sprite)) => TextObject::Atlas {
                            atlas,
                            sprite,
                            fallback,
                        },
                        None => {
                            let (profile, hat) = player()?;
                            TextObject::Player {
                                profile,
                                hat,
                                fallback,
                            }
                        }
                    },
                })
            }
            _ => return None,
        };
        let mut text = Text::new(content);
        // This vanilla flag has no dedicated field on TextContent::Nbt.
        if plain {
            text.metadata = Tag::compound([(NBT_PLAIN, Tag::Byte(1))]);
        }
        Some(text)
    }

    fn arguments(&self, list: &Tag) -> Option<Vec<Text>> {
        let primitive = |tag: &Tag| {
            let mut arg = Text::literal(number_text(tag)?);
            arg.metadata = Tag::compound([(PRIMITIVE_ARGUMENT, tag.clone())]);
            Some(arg)
        };
        // NbtOps.getStream accepts primitive arrays as well as ListTag.
        match list {
            Tag::ByteArray(values) => values
                .iter()
                .map(|&n| primitive(&Tag::Byte(n as i8)))
                .collect(),
            Tag::IntArray(values) => values.iter().map(|&n| primitive(&Tag::Int(n))).collect(),
            Tag::LongArray(values) => values.iter().map(|&n| primitive(&Tag::Long(n))).collect(),
            Tag::List(items) => items
                .iter()
                .map(|tag| primitive(tag).or_else(|| self.decode(tag)))
                .collect(),
            _ => None,
        }
    }

    fn encode(&self, text: &Text, depth: usize) -> Result<Tag, Error> {
        nbt::with_tree_stack(|| {
            if let TextContent::Literal(value) = &text.content
                && text.children.is_empty()
                && style::is_empty(&text.style)
            {
                return Ok(s(value));
            }
            if depth >= Limits::default().max_depth {
                return Err(Error::TooDeep);
            }
            let mut fields = Compound::new();
            let mut add = |key: &str, value: Tag| {
                fields.insert(key.into(), value);
            };
            let child = |value: &Text| self.encode(value, depth + 1);
            match &text.content {
                TextContent::Literal(value) => add("text", s(value)),
                TextContent::Translatable {
                    key,
                    fallback,
                    arguments,
                } => {
                    add("translate", s(key));
                    if let Some(fallback) = fallback {
                        add("fallback", s(fallback));
                    }
                    if !arguments.is_empty() {
                        let args = arguments
                            .iter()
                            .map(|arg| {
                                // Only a decoded numeric argument has this origin. A fresh
                                // Text::literal("42") always encodes as a string.
                                if let TextContent::Literal(value) = &arg.content
                                    && arg.children.is_empty()
                                    && style::is_empty(&arg.style)
                                    && let Some(tag) = arg.metadata.get(PRIMITIVE_ARGUMENT)
                                    && number_text(tag).as_ref() == Some(value)
                                {
                                    Ok(tag.clone())
                                } else {
                                    self.encode(arg, depth + 2)
                                }
                            })
                            .collect::<Result<_, _>>()?;
                        add("with", Tag::List(args));
                    }
                }
                TextContent::Keybind(key) => add("keybind", s(key)),
                TextContent::Score { name, objective } => {
                    add(
                        "score",
                        Tag::compound([("name", s(name)), ("objective", s(objective))]),
                    );
                }
                TextContent::Selector { pattern, separator } => {
                    if !selector_parses(pattern, self.registries) {
                        return Err(Error::Malformed);
                    }
                    add("selector", s(pattern));
                    if let Some(separator) = separator {
                        add("separator", child(separator)?);
                    }
                }
                TextContent::Nbt {
                    path,
                    interpret,
                    source,
                    separator,
                } => {
                    let plain = text
                        .metadata
                        .get(NBT_PLAIN)
                        .and_then(boolean)
                        .unwrap_or(false);
                    if !path_parses(path) || (*interpret && plain) {
                        return Err(Error::Malformed);
                    }
                    add("nbt", s(path));
                    if *interpret {
                        add("interpret", Tag::Byte(1));
                    }
                    if plain {
                        add(NBT_PLAIN, Tag::Byte(1));
                    }
                    match source {
                        TextNbtSource::Block(pos) if block_pos_parses(pos, self.registries) => {
                            add("block", s(pos))
                        }
                        TextNbtSource::Entity(selector)
                            if selector_parses(selector, self.registries) =>
                        {
                            add("entity", s(selector))
                        }
                        TextNbtSource::Storage(id) => {
                            add("storage", s(&identifier(id).ok_or(Error::Malformed)?))
                        }
                        _ => return Err(Error::Malformed),
                    }
                    if let Some(separator) = separator {
                        add("separator", child(separator)?);
                    }
                }
                TextContent::Object(object) => {
                    let fallback = match object {
                        TextObject::Atlas {
                            atlas,
                            sprite,
                            fallback,
                        } => {
                            let atlas = identifier(atlas).ok_or(Error::Malformed)?;
                            if atlas != DEFAULT_ATLAS {
                                add("atlas", s(&atlas));
                            }
                            add("sprite", s(&identifier(sprite).ok_or(Error::Malformed)?));
                            fallback
                        }
                        TextObject::Player {
                            profile,
                            hat,
                            fallback,
                        } => {
                            add("player", profile::encode(profile)?);
                            if !hat {
                                add("hat", Tag::Byte(0));
                            }
                            fallback
                        }
                    };
                    if let Some(fallback) = fallback {
                        add("fallback", child(fallback)?);
                    }
                }
            }
            self.encode_style(&text.style, &mut fields, depth + 1)?;
            if !text.children.is_empty() {
                let siblings = text
                    .children
                    .iter()
                    .map(|child| self.encode(child, depth + 2))
                    .collect::<Result<_, _>>()?;
                fields.insert("extra".into(), Tag::List(siblings));
            }
            Ok(Tag::Compound(fields))
        })
    }
}

fn string(tag: &Tag) -> Option<String> {
    match tag {
        Tag::String(value) => value.to_string().ok(),
        _ => None,
    }
}

fn s(value: &str) -> Tag {
    Tag::String(value.into())
}

fn identifier(value: &str) -> Option<String> {
    Some(Identifier::parse(value).ok()?.to_string())
}

fn id(tag: &Tag) -> Option<String> {
    identifier(&string(tag)?)
}

// A missing field defaults, but a present field that fails decoding is an error.
fn optional<T>(tag: &Tag, key: &str, read: impl FnOnce(&Tag) -> Option<T>) -> Option<Option<T>> {
    match tag.get(key) {
        Some(value) => Some(Some(read(value)?)),
        None => Some(None),
    }
}

fn number(tag: &Tag) -> Option<f64> {
    Some(match tag {
        Tag::Byte(n) => f64::from(*n),
        Tag::Short(n) => f64::from(*n),
        Tag::Int(n) => f64::from(*n),
        Tag::Long(n) => *n as f64,
        Tag::Float(n) => f64::from(f32::from_bits(*n)),
        Tag::Double(n) => f64::from_bits(*n),
        _ => return None,
    })
}

fn boolean(tag: &Tag) -> Option<bool> {
    Some(number(tag)? != 0.0)
}

fn number_text(tag: &Tag) -> Option<String> {
    use sunrise_core::java_number::{double_to_string, float_to_string};
    Some(match tag {
        Tag::Byte(n) => n.to_string(),
        Tag::Short(n) => n.to_string(),
        Tag::Int(n) => n.to_string(),
        Tag::Long(n) => n.to_string(),
        Tag::Float(n) => float_to_string(f32::from_bits(*n)),
        Tag::Double(n) => double_to_string(f64::from_bits(*n)),
        _ => return None,
    })
}

fn uuid(tag: &Tag) -> Option<[u64; 2]> {
    let words: [i32; 4] = match tag {
        Tag::IntArray(values) => values.as_slice().try_into().ok()?,
        Tag::List(values) => {
            let [a, b, c, d] = values.as_slice() else {
                return None;
            };
            [
                nbt::numeric_int(a)?,
                nbt::numeric_int(b)?,
                nbt::numeric_int(c)?,
                nbt::numeric_int(d)?,
            ]
        }
        Tag::ByteArray(values) => <[u8; 4]>::try_from(values.as_slice())
            .ok()?
            .map(|n| i32::from(n as i8)),
        Tag::LongArray(values) => <[i64; 4]>::try_from(values.as_slice())
            .ok()?
            .map(|n| n as i32),
        _ => return None,
    };
    Some(uuid_halves(words))
}

fn uuid_halves([a, b, c, d]: [i32; 4]) -> [u64; 2] {
    let pair = |a, b| (u64::from(a as u32) << 32) | u64::from(b as u32);
    [pair(a, b), pair(c, d)]
}

fn uuid_tag([high, low]: [u64; 2]) -> Tag {
    Tag::IntArray(vec![
        (high >> 32) as i32,
        high as i32,
        (low >> 32) as i32,
        low as i32,
    ])
}
