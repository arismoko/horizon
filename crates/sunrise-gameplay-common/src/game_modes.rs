//! Game mode rules (`GameType`).
use sunrise_command::args::GameMode;

pub fn mode_from_id(id: i32) -> GameMode {
    GameMode::from_id(id).unwrap_or(GameMode::Survival)
}
/// `ChunkMap.skipPlayer`: a player is skipped when `player.isSpectator() &&
/// !level.getGameRules().getBoolean(SPECTATORS_GENERATE_CHUNKS)`. A skipped
/// player holds no distance-manager ticket and streams no chunks around
/// itself, but still receives the chunks other players load (chunk sending
/// is untouched).
pub fn loads_chunks(spectators_generate_chunks: bool, spectator: bool) -> bool {
    !spectator || spectators_generate_chunks
}

/// Fail closed at the new projectile caller when the existing matcher cannot
/// evaluate an entire conjunct. This does not change held-item interpretation.
pub fn projectile_predicate_supported(tag: &sunrise_core::nbt::Tag) -> bool {
    use sunrise_core::nbt::Tag;
    let supported = |p: &Tag| {
        matches!(p, Tag::Compound(_))
            && ["nbt", "components", "predicates"]
                .iter()
                .all(|key| p.get(key).is_none())
            && match p.get("blocks") {
                None | Some(Tag::String(_)) => true,
                Some(Tag::List(ids)) => ids.iter().all(|id| matches!(id, Tag::String(_))),
                _ => false,
            }
            && match p.get("state") {
                None => true,
                Some(Tag::Compound(properties)) => properties.values().all(|value| match value {
                    Tag::String(_) => true,
                    Tag::Compound(_) => ["min", "max"].iter().all(|key| {
                        value
                            .get(key)
                            .is_none_or(|bound| matches!(bound, Tag::String(_)))
                    }),
                    _ => false,
                }),
                _ => false,
            }
    };
    match tag {
        Tag::Compound(_) => supported(tag),
        Tag::List(predicates) => !predicates.is_empty() && predicates.iter().all(supported),
        _ => false,
    }
}
