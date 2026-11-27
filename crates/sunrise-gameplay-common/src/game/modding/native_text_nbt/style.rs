use super::{Codec, Error, boolean, id, identifier, optional, s, string, uuid, uuid_tag};
use mod_api::text::{TextClick, TextHover, TextStyle};
use sunrise_core::nbt::{self, Compound, Limits, Tag, numeric_int};
use sunrise_core::text::parse_java_uuid;
use sunrise_game_data::component::untrusted_http_url;
use sunrise_game_data::text::is_allowed_chat_character;
use sunrise_world::component_value::source::{
    normalize_template_patch, text_color_name, text_shadow_color, validate_template_count,
};

pub(super) fn is_empty(style: &TextStyle) -> bool {
    style.color.is_none()
        && style.shadow_color.is_none()
        && flags(style).into_iter().all(|(_, flag)| flag.is_none())
        && style.font.is_none()
        && style.insertion.is_none()
        && style.click.is_none()
        && style.hover.is_none()
}

fn flags(style: &TextStyle) -> [(&'static str, Option<bool>); 5] {
    [
        ("bold", style.bold),
        ("italic", style.italic),
        ("underlined", style.underlined),
        ("strikethrough", style.strikethrough),
        ("obfuscated", style.obfuscated),
    ]
}

fn command(value: &str) -> bool {
    value.chars().all(is_allowed_chat_character)
}

// Count the surrounding node/field containers before cloning NBT retained by
// another field codec. EndTag is NbtOps.empty(), not a stored field value.
fn check_opaque(tag: &Tag, depth: usize) -> Result<(), Error> {
    if !nbt::within_depth(tag, Limits::default().max_depth.saturating_sub(depth)) {
        return Err(Error::TooDeep);
    }
    if matches!(tag, Tag::End) {
        return Err(Error::Malformed);
    }
    Ok(())
}

impl Codec<'_> {
    pub(super) fn decode_style(&self, tag: &Tag) -> Option<TextStyle> {
        let mut style = TextStyle::default();
        style.color = optional(tag, "color", |v| {
            Some(text_color_name(&string(v)?).ok()?.into_owned())
        })?;
        style.shadow_color = optional(tag, "shadow_color", |v| {
            Some(text_shadow_color(v).ok()? as u32)
        })?;
        style.bold = optional(tag, "bold", boolean)?;
        style.italic = optional(tag, "italic", boolean)?;
        style.underlined = optional(tag, "underlined", boolean)?;
        style.strikethrough = optional(tag, "strikethrough", boolean)?;
        style.obfuscated = optional(tag, "obfuscated", boolean)?;
        style.font = optional(tag, "font", id)?;
        style.insertion = optional(tag, "insertion", string)?;
        style.click = optional(tag, "click_event", |v| self.decode_click(v))?;
        style.hover = optional(tag, "hover_event", |v| self.decode_hover(v))?;
        Some(style)
    }

    pub(super) fn encode_style(
        &self,
        style: &TextStyle,
        fields: &mut Compound,
        depth: usize,
    ) -> Result<(), Error> {
        let mut add = |name: &str, tag| {
            fields.insert(name.into(), tag);
        };
        if let Some(color) = &style.color {
            add(
                "color",
                s(&text_color_name(color).map_err(|_| Error::Malformed)?),
            );
        }
        if let Some(shadow_color) = style.shadow_color {
            add("shadow_color", Tag::Int(shadow_color as i32));
        }
        for (name, flag) in flags(style) {
            if let Some(value) = flag {
                add(name, Tag::Byte(i8::from(value)));
            }
        }
        if let Some(font) = &style.font {
            add("font", s(&identifier(font).ok_or(Error::Malformed)?));
        }
        if let Some(insertion) = &style.insertion {
            add("insertion", s(insertion));
        }
        if let Some(click) = &style.click {
            add("click_event", self.encode_click(click, depth + 1)?);
        }
        if let Some(hover) = &style.hover {
            add("hover_event", self.encode_hover(hover, depth + 1)?);
        }
        Ok(())
    }

    fn decode_click(&self, tag: &Tag) -> Option<TextClick> {
        let field = |key: &str| string(tag.get(key)?);
        Some(match field("action")?.as_str() {
            "open_url" => TextClick::OpenUrl(field("url").filter(|url| untrusted_http_url(url))?),
            "change_page" => {
                TextClick::ChangePage(numeric_int(tag.get("page")?).filter(|&n| n > 0)?)
            }
            "suggest_command" => {
                TextClick::SuggestCommand(field("command").filter(|s| command(s))?)
            }
            "run_command" => TextClick::RunCommand(field("command").filter(|s| command(s))?),
            "copy_to_clipboard" => TextClick::CopyToClipboard(field("value")?),
            "show_dialog" => {
                let raw = tag.get("dialog")?;
                TextClick::ShowDialog(if self.defer_holders {
                    nbt::clone_tag(raw)
                } else {
                    self.dialog_holder(raw)?
                })
            }
            "custom" => TextClick::Custom {
                id: id(tag.get("id")?)?,
                payload: optional(tag, "payload", |v| {
                    (!matches!(v, Tag::End)).then(|| nbt::clone_tag(v))
                })?,
            },
            // ClickEvent.Action.CODEC does not admit open_file.
            _ => return None,
        })
    }

    fn encode_click(&self, click: &TextClick, depth: usize) -> Result<Tag, Error> {
        let (action, key, value) = match click {
            TextClick::OpenUrl(url) if untrusted_http_url(url) => ("open_url", "url", s(url)),
            TextClick::ChangePage(page) if *page > 0 => ("change_page", "page", Tag::Int(*page)),
            TextClick::SuggestCommand(value) if command(value) => {
                ("suggest_command", "command", s(value))
            }
            TextClick::RunCommand(value) if command(value) => ("run_command", "command", s(value)),
            TextClick::CopyToClipboard(value) => ("copy_to_clipboard", "value", s(value)),
            TextClick::ShowDialog(value) => {
                check_opaque(value, depth)?;
                (
                    "show_dialog",
                    "dialog",
                    self.dialog_holder(value).ok_or(Error::Malformed)?,
                )
            }
            TextClick::Custom { id, payload } => {
                let action = [
                    ("action", s("custom")),
                    ("id", s(&identifier(id).ok_or(Error::Malformed)?)),
                ];
                let payload = payload
                    .as_ref()
                    .map(|tag| {
                        check_opaque(tag, depth)?;
                        Ok(("payload", nbt::clone_tag(tag)))
                    })
                    .transpose()?;
                return Ok(Tag::compound(action.into_iter().chain(payload)));
            }
            _ => return Err(Error::Malformed),
        };
        Ok(Tag::compound([("action", s(action)), (key, value)]))
    }

    fn dialog_holder(&self, tag: &Tag) -> Option<Tag> {
        sunrise_sim::dialog::persistent_holder(
            tag,
            self.registries,
            &|id| self.item_exists(id),
            &|tag| self.canonical_text_tag(tag),
        )
    }

    fn item_exists(&self, id: &str) -> bool {
        id != "minecraft:air"
            && (self.registries.items.is_empty()
                || self.registries.items.iter().any(|item| item == id))
    }

    fn item_patch(&self, tag: &Tag) -> Result<Option<Tag>, Error> {
        normalize_template_patch(
            tag,
            &|id| self.item_exists(id),
            |id| sunrise_protocol::item::custom::id(id).is_some(),
            &|tag| self.canonical_text_tag(tag),
        )
        .map_err(|_| Error::Malformed)
    }

    fn canonical_text_tag(&self, tag: &Tag) -> Option<Tag> {
        // Encode validates opaque holders. Leave them raw during this intermediate
        // decode so nested holders don't get projected twice at every level.
        let codec = Codec {
            registries: self.registries,
            defer_holders: true,
        };
        codec.encode(&codec.decode(tag)?, 0).ok()
    }

    fn entity_exists(&self, id: &str) -> bool {
        self.registries.entity_types.is_empty()
            || self.registries.entity_types.iter().any(|kind| kind == id)
    }

    fn decode_hover(&self, tag: &Tag) -> Option<TextHover> {
        Some(match string(tag.get("action")?)?.as_str() {
            "show_text" => TextHover::Text(Box::new(self.decode(tag.get("value")?)?)),
            "show_item" => TextHover::Item {
                id: id(tag.get("id")?).filter(|id| self.item_exists(id))?,
                count: optional(tag, "count", hover_count)?.unwrap_or(1) as u32,
                components: optional(tag, "components", |patch| {
                    if self.defer_holders {
                        Some(Some(nbt::clone_tag(patch)))
                    } else {
                        self.item_patch(patch).ok()
                    }
                })?
                .flatten(),
            },
            "show_entity" => {
                let raw = tag.get("uuid")?;
                TextHover::Entity {
                    kind: id(tag.get("id")?).filter(|id| self.entity_exists(id))?,
                    id: uuid(raw).or_else(|| {
                        let id = parse_java_uuid(&string(raw)?)?;
                        Some([(id >> 64) as u64, id as u64])
                    })?,
                    name: optional(tag, "name", |v| self.decode(v).map(Box::new))?,
                }
            }
            _ => return None,
        })
    }

    fn encode_hover(&self, hover: &TextHover, depth: usize) -> Result<Tag, Error> {
        let mut fields = Compound::new();
        let mut add = |key: &str, value| {
            fields.insert(key.into(), value);
        };
        match hover {
            TextHover::Text(text) => {
                add("action", s("show_text"));
                add("value", self.encode(text, depth)?);
            }
            TextHover::Item {
                id,
                count,
                components,
            } => {
                let id = identifier(id)
                    .filter(|id| self.item_exists(id))
                    .ok_or(Error::Malformed)?;
                let count = i32::try_from(*count).map_err(|_| Error::Malformed)?;
                validate_template_count(count).map_err(|_| Error::Malformed)?;
                add("action", s("show_item"));
                add("id", s(&id));
                if count != 1 {
                    add("count", Tag::Int(count));
                }
                if let Some(components) = components {
                    check_opaque(components, depth)?;
                    if let Some(canonical) = self.item_patch(components)? {
                        add("components", canonical);
                    }
                }
            }
            TextHover::Entity { kind, id, name } => {
                let kind = identifier(kind)
                    .filter(|kind| self.entity_exists(kind))
                    .ok_or(Error::Malformed)?;
                add("action", s("show_entity"));
                add("id", s(&kind));
                add("uuid", uuid_tag(*id));
                if let Some(name) = name {
                    add("name", self.encode(name, depth)?);
                }
            }
        }
        Ok(Tag::Compound(fields))
    }
}

// ItemStackTemplate.MAP_CODEC.
fn hover_count(tag: &Tag) -> Option<i32> {
    let count = numeric_int(tag)?;
    validate_template_count(count).ok()?;
    Some(count)
}
