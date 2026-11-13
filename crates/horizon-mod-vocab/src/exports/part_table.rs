mod parts;
pub use parts::limits as general_entity_limits;
pub use parts::{
    Attached, BlockCapabilityProvider, BlockProvider, BlockProviderOf, BlockProviders,
    CapabilityProviders, EntityCapabilityProvider, EntityProvider, EntityProviderOf,
    EntityProviders, GeneralEntityCategory, GeneralEntityRegistrationError, GeneralSavePolicy,
    ItemCapabilityProvider, ItemContainerProvider, ItemProvider, ItemProviderOf, OwnProvider,
    Parts, ProvidedBy, Registrar, block_capability_names, block_capability_proxyable,
    block_capability_proxyable_names, block_providers, entity_providers, has_entity_providers,
};
pub(crate) use parts::{
    InstalledProviders, answered, ask_providers, has_part_hook, parts, worldgen_parts,
};
mod __private {
    #[cfg(any(test, feature = "testing"))]
    pub use crate::parts::install_process_datapack_registry_declarations;
    #[cfg(any(test, feature = "testing"))]
    pub use crate::parts::tests as part_tests;
    pub use crate::parts::{
        AttributeDeclaration, BiomeModifierDeclaration, BiomeRegionDeclaration,
        EntityAttributeDeclaration, EntityAttributeModification, GeneralEntityTypeDeclaration,
        HookParts, ItemKeyError, KnownIds, PartsRef, ResolvePartsError, SpawnPlacementDeclaration,
        TooltipAppenderPart, attribute_declarations, biome_modifier_declarations,
        biome_region_declarations, block_entity_transform, block_entity_transforms,
        block_entity_types, block_key_listens, clear_game_parts, custom_stat_declarations,
        datapack_registry_declarations, declared_world_fluids, entity_attribute_declarations,
        entity_attribute_modifications, entity_parts, game_rule_bool_slot, game_rule_categories,
        game_rules, general_entity_type_declarations, has_entity_parts, has_part_hook,
        has_scoped_hook, install_parts, install_resolved_parts, item_listens, key_mappings,
        mark_game_thread, menu_listens, parts, prepare_parts, publish_worldgen_parts,
        resolve_parts, setting, settings, spawn_placement_declarations, tooltip_appenders,
        worldgen_feature, worldgen_part_mods,
    };
}
