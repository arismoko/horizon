use sunrise_sim::mobs::Mob;

/// `Entity.DATA_CUSTOM_NAME` of a mob named `name`: its component with its style (a name
/// summoned or set as `{text: "..", color: "red"}` keeps its colour), else the plain text.
pub fn name_component(m: &Mob, name: &str) -> sunrise_core::nbt::Tag {
    match &m.custom_name_text {
        Some(text) => sunrise_game_data::scoreboard::component_tag(&text.component()),
        None => sunrise_protocol::text::literal(name),
    }
}
