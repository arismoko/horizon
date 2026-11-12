use super::*;

/// The immutable declarations installed after schema validation.
// hidden: the game builds engine registries from the installed declaration snapshot.
#[doc(hidden)]
pub fn game_rules() -> &'static [crate::GameRuleDecl] {
    installed().map_or(&[], |parts| &parts.game_rules)
}
/// The immutable category declarations installed by mods.
// hidden: the game builds engine registries from the installed category snapshot.
#[doc(hidden)]
pub fn game_rule_categories() -> &'static [String] {
    installed().map_or(&[], |parts| &parts.game_rule_categories)
}
/// Dense position for a validated rule key.
// hidden: the game resolves load-owned keys to dense positions.
#[doc(hidden)]
pub fn game_rule_bool_slot(key: crate::BoolGameRuleKey) -> Option<u32> {
    installed()?.game_rule_slots.get(key.0 as usize).copied()
}

/// Registrar operations for game rules.
pub trait GameRulesRegistrar {
    /// Declare a game rule category in a namespace owned by the calling mod's manifest. Invalid,
    /// foreign or duplicate IDs refuse mod loading even if the error is ignored.
    /// Categories appear in the host's game rules screen; remote clients do not
    /// receive them in this version.
    fn game_rule_category(&mut self, id: &str) -> Result<(), crate::GameRuleError>;

    /// Declare a boolean game rule in a namespace owned by this mod's manifest. Category references are
    /// resolved after all mods register; an unknown category refuses installation then.
    fn game_rule_bool(
        &mut self,
        id: &str,
        category: &str,
        default: bool,
    ) -> Result<crate::BoolGameRuleKey, crate::GameRuleError>;
}
