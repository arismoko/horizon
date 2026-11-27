//! Mods' selector tokens (`@allies`, NeoForge's `EntitySelectorManager`): the frozen base
//! selections the command registries parse them with.
use mod_api::SelectorSort;
pub use sunrise_game_data::registries::SelectorToken;

/// The frozen base selections validated when the installed mods registered their tokens.
pub fn selector_tokens() -> Vec<SelectorToken> {
    let mut tokens = Vec::new();
    for (name, owner, selector) in mod_api::__private::entity_selector_types() {
        if !owner.is_enabled() {
            continue;
        }
        let base = match owner.call("entity_selector", "entity_selector.base", || {
            selector.base()
        }) {
            Ok(base) => base,
            Err(why) => {
                log::warn!("entity selector @{name} base failed: {why}");
                continue;
            }
        };
        tokens.push(SelectorToken {
            name: name.to_owned(),
            tooltip: base.tooltip,
            players_only: base.players_only,
            limit: base.limit.map_or(usize::MAX, |limit| limit as usize),
            sort: match base.sort {
                SelectorSort::Nearest => "nearest",
                SelectorSort::Furthest => "furthest",
                SelectorSort::Random => "random",
                _ => "arbitrary",
            },
        });
    }
    tokens
}
