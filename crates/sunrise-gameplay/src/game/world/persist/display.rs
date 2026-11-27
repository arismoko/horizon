use mod_api::{Billboard, ItemDisplayContext, TextAlignment, Transformation};
use std::sync::Arc;
use sunrise_core::nbt::Tag;
use sunrise_game_data::nbt_path::int_value;
use sunrise_items::item_nbt::{item_from_nbt, item_to_nbt};
use sunrise_sim::{
    entity_nbt,
    save::{float, number, text},
    text_display_data::{
        resident::{DisplayComponent, DisplayContents, DisplayState, DisplayText},
        transformation,
    },
};
use sunrise_world::registry::BlockStateId;

pub fn read_text_display_nbt(tag: &Tag) -> DisplayState {
    let int = |key, default| tag.get(key).and_then(int_value).unwrap_or(default);
    let mut value = DisplayText::default();
    if let Some(component) = tag.get("text").and_then(DisplayComponent::from_nbt) {
        value.component = Arc::new(component);
    }
    value.line_width = int("line_width", value.line_width);
    value.background = int("background", value.background);
    value.opacity = int("text_opacity", i32::from(value.opacity)) as i8;
    value.shadow = entity_nbt::flag(tag, "shadow");
    value.see_through = entity_nbt::flag(tag, "see_through");
    value.default_background = entity_nbt::flag(tag, "default_background");
    value.alignment = match text(tag.get("alignment")).as_deref() {
        Some("left") => TextAlignment::Left,
        Some("right") => TextAlignment::Right,
        _ => TextAlignment::Center,
    };
    read_display_fields(DisplayContents::Text(value), tag)
}

fn read_display_fields(contents: DisplayContents, tag: &Tag) -> DisplayState {
    let int = |key, default| tag.get(key).and_then(int_value).unwrap_or(default);
    let real = |key, default| float(tag.get(key)).map_or(default, |v| v as f32);
    let mut state = DisplayState::new(contents);
    if let Some((translation, left_rotation, scale, right_rotation)) =
        tag.get("transformation").and_then(transformation)
    {
        state.transformation = Transformation {
            translation,
            left_rotation,
            scale,
            right_rotation,
        };
    }
    state.billboard = text(tag.get("billboard"))
        .as_deref()
        .and_then(Billboard::from_name)
        .unwrap_or(state.billboard);
    state.brightness = tag.get("brightness").and_then(|value| {
        let light = |key| {
            // The brightness codec truncates floating-point NBT before checking its range.
            let value = value.get(key)?;
            let light = match value {
                Tag::Float(_) | Tag::Double(_) => float(Some(value))? as i32,
                _ => number(Some(value))? as i32,
            };
            (0..=15).contains(&light).then_some(light as u8)
        };
        Some((light("block")?, light("sky")?))
    });
    state.start_interpolation = int("start_interpolation", state.start_interpolation);
    state.interpolation_duration = int("interpolation_duration", state.interpolation_duration);
    // `Display.readAdditionalSaveData` clamps the loaded position/rotation duration.
    state.teleport_duration = int("teleport_duration", state.teleport_duration).clamp(0, 59);
    state.view_range = real("view_range", state.view_range);
    state.shadow_radius = real("shadow_radius", state.shadow_radius);
    state.shadow_strength = real("shadow_strength", state.shadow_strength);
    state.width = real("width", state.width);
    state.height = real("height", state.height);
    state.glow_color_override = int("glow_color_override", state.glow_color_override);
    state
}

impl crate::game::ServerGame<'_> {
    /// `Display.readAdditionalSaveData` and the three subclasses' saved fields.
    pub fn read_display_nbt(&self, kind: &str, tag: &Tag) -> Option<DisplayState> {
        let contents = match kind {
            "minecraft:block_display" => DisplayContents::Block(
                tag.get("block_state")
                    .and_then(|value| self.state_from_nbt(value))
                    .or_else(|| {
                        self.common
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:air")
                    })
                    .unwrap_or(BlockStateId(0)),
            ),
            "minecraft:item_display" => DisplayContents::Item {
                stack: tag
                    .get("item")
                    .and_then(item_from_nbt)
                    .filter(|stack| {
                        stack.count != 0
                            && &*stack.item != "minecraft:air"
                            && self.items().get(&stack.item).is_some()
                    })
                    .map(Arc::new),
                context: text(tag.get("item_display"))
                    .as_deref()
                    .and_then(ItemDisplayContext::from_name)
                    .unwrap_or(ItemDisplayContext::None),
            },
            "minecraft:text_display" => return Some(read_text_display_nbt(tag)),
            _ => return None,
        };
        Some(read_display_fields(contents, tag))
    }

    /// `Display.addAdditionalSaveData`: save the settings as the codecs read
    /// them, including defaults. The raw start delay is only used by tracking.
    pub(super) fn write_display_nbt(&self, state: &DisplayState, tag: &mut Tag) {
        let Tag::Compound(fields) = tag else { return };
        let real = |v: f32| Tag::Float(if v == 0.0 { 0 } else { v.to_bits() });
        let floats = |parts: &[f32]| Tag::List(parts.iter().copied().map(real).collect());
        let t = state.transformation;
        let transform = Tag::compound([
            ("translation", floats(&t.translation)),
            ("left_rotation", floats(&t.left_rotation)),
            ("scale", floats(&t.scale)),
            ("right_rotation", floats(&t.right_rotation)),
        ]);
        for (key, value) in [
            ("transformation", transform),
            ("billboard", Tag::String(state.billboard.name().into())),
            (
                "interpolation_duration",
                Tag::Int(state.interpolation_duration),
            ),
            ("teleport_duration", Tag::Int(state.teleport_duration)),
            ("view_range", real(state.view_range)),
            ("shadow_radius", real(state.shadow_radius)),
            ("shadow_strength", real(state.shadow_strength)),
            ("width", real(state.width)),
            ("height", real(state.height)),
            ("glow_color_override", Tag::Int(state.glow_color_override)),
        ] {
            fields.insert(key.into(), value);
        }
        fields.remove(&"start_interpolation".into());
        if let Some((block, sky)) = state.brightness {
            fields.insert(
                "brightness".into(),
                Tag::compound([
                    ("block", Tag::Int(i32::from(block))),
                    ("sky", Tag::Int(i32::from(sky))),
                ]),
            );
        } else {
            fields.remove(&"brightness".into());
        }
        match &state.contents {
            DisplayContents::Block(state) => {
                fields.insert("block_state".into(), self.common.state_to_nbt(*state));
            }
            DisplayContents::Item { stack, context } => {
                fields.insert("item_display".into(), Tag::String(context.name().into()));
                if let Some(stack) = stack {
                    fields.insert("item".into(), item_to_nbt(stack, self.items()));
                } else {
                    fields.remove(&"item".into());
                }
            }
            DisplayContents::Text(text) => {
                for (key, value) in [
                    ("text", text.component.to_nbt()),
                    ("line_width", Tag::Int(text.line_width)),
                    ("background", Tag::Int(text.background)),
                    ("text_opacity", Tag::Byte(text.opacity)),
                    ("shadow", Tag::Byte(i8::from(text.shadow))),
                    ("see_through", Tag::Byte(i8::from(text.see_through))),
                    (
                        "default_background",
                        Tag::Byte(i8::from(text.default_background)),
                    ),
                    ("alignment", Tag::String(text.alignment.name().into())),
                ] {
                    fields.insert(key.into(), value);
                }
            }
        }
    }
}

/// Keep only the fields that are not owned by the resident display.
pub(in crate::game) fn strip_display_nbt(state: &DisplayState, tag: &mut Tag) {
    let Tag::Compound(fields) = tag else { return };
    let contents: &[&str] = match state.contents {
        DisplayContents::Block(_) => &["block_state"],
        DisplayContents::Item { .. } => &["item", "item_display"],
        DisplayContents::Text(_) => &[
            "text",
            "line_width",
            "background",
            "text_opacity",
            "shadow",
            "see_through",
            "default_background",
            "alignment",
        ],
    };
    for &key in [
        "transformation",
        "billboard",
        "brightness",
        "start_interpolation",
        "interpolation_duration",
        "teleport_duration",
        "view_range",
        "shadow_radius",
        "shadow_strength",
        "width",
        "height",
        "glow_color_override",
    ]
    .iter()
    .chain(contents)
    {
        fields.remove(&key.into());
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
