mod parts;
#[cfg(test)]
pub(crate) use parts::install_parts;
#[cfg(test)]
pub(crate) use parts::mark_game_thread;
pub use parts::{
    Attached, BlockCapabilityProvider, BlockEntityProviders, BlockProvider, BlockProviderOf,
    CapabilityProviders, EntityCapabilityProvider, EntityProvider, EntityProviderOf,
    EntityProviders, ItemCapabilityProvider, ItemProvider, ItemProviderOf, OwnProvider, Parts,
    ProvidedBy, Registrar, block_entity_providers, entity_providers, has_entity_providers,
};
pub(crate) use parts::{InstalledProviders, answered, ask_providers, has_part_hook, parts};
mod __private {
    pub use crate::parts::{
        EntityAttributeDeclaration, HookParts, ItemKeyError, KnownIds, PartsRef, ResolvePartsError,
        TooltipAppenderPart, block_entity_types, block_listens, clear_game_parts,
        entity_attribute_declarations, entity_parts, game_rule_bool_slot, game_rule_categories,
        game_rules, has_entity_parts, has_part_hook, has_scoped_hook, install_parts,
        install_resolved_parts, key_mappings, mark_game_thread, parts, prepare_parts,
        resolve_parts, tooltip_appenders,
    };
    #[cfg(any(test, feature = "testing"))]
    pub use crate::{parts::tests as part_tests, phase::with_test_packages};
}
