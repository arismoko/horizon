//! What a converted mob carries over that lives outside `Mob`: its tags, its team and its mods'
//! attachments (`Mob.convertTo`).

impl crate::game::ServerGame<'_> {
    /// Transfer the entity-owned state that is stored outside `Mob`.
    pub(in crate::game) fn transfer_conversion_identity(&mut self, from: u32, to: u32) {
        use crate::game::commands::Target;
        use sunrise_sim::save::uuid_text;

        let tags = self.tags_of(Target::Mob(from));
        if !tags.is_empty() {
            self.set_entity_tags(Target::Mob(to), &tags);
        }
        self.load_scoreboard();
        let old_name = uuid_text(self.uuid_for(from));
        let team = self
            .common
            .score
            .board
            .players_team(&old_name)
            .map(|t| t.name.clone());
        if let Some(team) = team {
            let new_name = uuid_text(self.uuid_for(to));
            self.common.score.board.add_to_team(&new_name, &team);
            self.common.score.board.remove_from_team(&old_name);
        }
    }
    /// `IEntityExtension.copyAttachmentsFrom(from, false)` (NeoForge's conversion copy): every
    /// attachment of the old mob with a codec, a mod's that isn't loaded included, goes to the
    /// new mob of type `kind` through its saved form, over what the new mob's construction
    /// gave it. A value without a codec stays behind. The copy is not synced on its own: the new
    /// mob's spawn carries its synced values. A mob with no attachments costs one lookup.
    pub(in crate::game) fn copy_conversion_attachments(&mut self, from: u32, to: u32, kind: &str) {
        let Some(saved) =
            self.common
                .entity_attachments
                .get(&from)
                .and_then(|map| match map.try_to_nbt() {
                    Ok(saved) => Some(saved),
                    Err(error) => {
                        log::error!(
                            "Sunrise: conversion attachment copy skipped for entity {from}: {error}"
                        );
                        None
                    }
                })
        else {
            return;
        };
        if saved.is_empty() {
            return;
        }
        // The new mob is constructed (`EntityConstructing`) before the copy, as `convertTo`
        // builds it before `copyAttachmentsFrom`, so the copy wins key by key.
        self.initialize_entity(mod_api::Holder::Entity(to), kind);
        self.common
            .entity_attachments
            .entry(to)
            .or_default()
            .read_saved_over(saved);
    }
}

use sunrise_gameplay_common::game::entity_initialization_common::EntityInitializationCommonGameExt1 as _;
