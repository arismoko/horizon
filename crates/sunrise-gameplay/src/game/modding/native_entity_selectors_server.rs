//! Server-side mod entity-selector filtering (`@allies`, NeoForge's `EntitySelectorManager`).
use super::commands::Target;
use crate::game::ServerGame;
use sunrise_command::args::Selector;

impl ServerGame<'_> {
    /// `select_in_level`, then, for a mod's token, only the candidates its type keeps.
    pub(in crate::game) fn native_selector_kept(
        &mut self,
        s: &Selector,
        candidates: Vec<Target>,
        origin: [f64; 3],
    ) -> Vec<(Target, f64)> {
        let selected = self.select_in_level(candidates, s, origin);
        let Some(token) = &s.token else {
            return selected;
        };
        let Some((owner, selector)) = mod_api::__private::entity_selector_type(token) else {
            return Vec::new();
        };
        if !owner.is_enabled() {
            return Vec::new();
        }
        let Ok(source) = self.native_command_origin(self.common.ui.registries.permission) else {
            return Vec::new();
        };
        let mut kept = Vec::new();
        for (target, distance) in selected {
            if !owner.is_enabled() {
                break;
            }
            let Some(entity) = self.native_command_entity_target(target) else {
                continue;
            };
            let host: &mut dyn mod_api::ServerHost = self;
            match owner.call("entity_selector", "entity_selector.matches", || {
                selector.matches(host, &source, entity)
            }) {
                Ok(true) => kept.push((target, distance)),
                Ok(false) => {}
                Err(why) => {
                    log::warn!("entity selector @{token} failed: {why}");
                    if !owner.is_enabled() {
                        break;
                    }
                }
            }
        }
        kept
    }
}
