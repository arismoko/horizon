//! Phase-scoped context trait implementations for [`Game`].

#[doc(hidden)]
#[macro_export]
macro_rules! impl_shared_phase_contexts {
    ($game:ty) => {
        impl mod_api::ConfigContext for $game {
            fn setting(
                &self,
                key: mod_api::SettingKey,
            ) -> Result<mod_api::SettingValue, mod_api::SettingAccessError> {
                self.common.setting(key)
            }

            fn config_dir(&self) -> &std::path::Path {
                <Self as mod_api::prelude::ConfigHost>::config_dir(self)
            }
        }

        impl mod_api::ReloadContext for $game {
            fn dist(&self) -> mod_api::Dist {
                <Self as mod_api::prelude::ConfigHost>::dist(self)
            }
            fn game_version(&self) -> &'static str {
                <Self as mod_api::prelude::ConfigHost>::game_version(self)
            }
            fn server_data_version(&self) -> i32 {
                <Self as mod_api::prelude::ConfigHost>::server_data_version(self)
            }
            fn sunrise_version(&self) -> &'static str {
                <Self as mod_api::prelude::ConfigHost>::sunrise_version(self)
            }
            fn on(&self, mod_id: &str) -> bool {
                <Self as mod_api::prelude::ConfigHost>::on(self, mod_id)
            }
            fn setting(
                &self,
                key: mod_api::SettingKey,
            ) -> Result<mod_api::SettingValue, mod_api::SettingAccessError> {
                self.common.setting(key)
            }
            fn game_tests_enabled(&self) -> bool {
                sunrise_level::runtime_env::runtime_environment::get().game_tests_enabled()
            }
            fn is_loaded(&self, id: &str) -> bool {
                <Self as mod_api::prelude::LevelsHost>::is_loaded(self, id)
            }
            fn item_cooldown_group(&self, stack: &mod_api::ItemStack) -> String {
                <Self as mod_api::prelude::PlayersHost>::item_cooldown_group(self, stack)
            }
            fn max_stack_size(&self, item: mod_api::ItemKey) -> Option<u32> {
                <Self as mod_api::prelude::ItemsHost>::max_stack_size(self, item)
            }
            fn max_damage(&self, item: mod_api::ItemKey) -> Option<u32> {
                <Self as mod_api::prelude::ItemsHost>::max_damage(self, item)
            }
            fn item_key(&self, id: &mod_api::Identifier) -> Option<mod_api::ItemKey> {
                <Self as mod_api::prelude::ItemsHost>::item_key(self, id)
            }
            fn item_default_component(
                &self,
                item: mod_api::ItemKey,
                component: &str,
            ) -> Option<mod_api::Tag> {
                <Self as mod_api::prelude::ItemsHost>::item_default_component(self, item, component)
            }
            fn hover_name(
                &self,
                stack: &mod_api::ItemStack,
            ) -> Result<mod_api::Text, mod_api::TextFormError> {
                <Self as mod_api::prelude::ItemsHost>::hover_name(self, stack)
            }
            fn item_burn_time(&self, item: mod_api::ItemKey) -> Option<i32> {
                <Self as mod_api::prelude::ItemsHost>::item_burn_time(self, item)
            }
            fn item_compost_chance(&self, item: mod_api::ItemKey) -> Option<f32> {
                <Self as mod_api::prelude::ItemsHost>::item_compost_chance(self, item)
            }
            fn entity_type_in_tag(&self, entity_type: &str, tag: &str) -> bool {
                <Self as mod_api::prelude::ItemsHost>::entity_type_in_tag(self, entity_type, tag)
            }
            fn data_map_value(
                &self,
                registry: &str,
                entry: &str,
                map: &str,
            ) -> Result<Option<&mod_api::Tag>, mod_api::DataMapError> {
                <Self as mod_api::prelude::DataMapsBackend>::data_map_value(
                    self, registry, entry, map,
                )
            }
            fn data_map_entries(
                &self,
                registry: &str,
                map: &str,
            ) -> Result<&[(String, mod_api::Tag)], mod_api::DataMapError> {
                <Self as mod_api::prelude::DataMapsBackend>::data_map_entries(self, registry, map)
            }
            fn game_event_key_id(
                &self,
                key: mod_api::GameEventKey,
            ) -> Result<mod_api::RegistryName, mod_api::GameEventKeyError> {
                <Self as mod_api::prelude::KeysBackend>::game_event_key_id(self, key)
            }
            fn block_key_id(&self, key: mod_api::BlockKey) -> Option<&str> {
                <Self as mod_api::prelude::KeysBackend>::block_key_id(self, key)
            }
            fn item_key_id(&self, key: mod_api::ItemKey) -> Option<&str> {
                <Self as mod_api::prelude::KeysBackend>::item_key_id(self, key)
            }
            fn item_registry_id(&self, key: mod_api::ItemKey) -> Option<u32> {
                <Self as mod_api::prelude::KeysBackend>::item_registry_id(self, key)
            }
            fn effect_key_id(&self, key: mod_api::EffectKey) -> Option<&str> {
                <Self as mod_api::prelude::KeysBackend>::effect_key_id(self, key)
            }
            fn effect_category(&self, key: mod_api::EffectKey) -> Option<mod_api::EffectCategory> {
                <Self as mod_api::prelude::KeysBackend>::effect_category(self, key)
            }
            fn fluid_key_id(&self, key: mod_api::FluidKey) -> Option<&str> {
                <Self as mod_api::prelude::KeysBackend>::fluid_key_id(self, key)
            }
            fn block_entity_type_key_id(&self, key: mod_api::BlockEntityTypeKey) -> Option<&str> {
                <Self as mod_api::prelude::KeysBackend>::block_entity_type_key_id(self, key)
            }
            fn entity_type_key_id(
                &self,
                key: mod_api::EntityTypeKey,
            ) -> Option<mod_api::RegistryName> {
                <Self as mod_api::prelude::KeysBackend>::entity_type_key_id(self, key)
            }
            fn entity_type_category(
                &self,
                key: mod_api::EntityTypeKey,
            ) -> Option<mod_api::MobCategory> {
                <Self as mod_api::prelude::KeysBackend>::entity_type_category(self, key)
            }
            fn menu_key_id(&self, key: mod_api::MenuKey) -> Option<mod_api::RegistryName> {
                <Self as mod_api::prelude::KeysBackend>::menu_key_id(self, key)
            }
            fn enchantment_key_id(
                &self,
                key: mod_api::EnchantmentKey,
            ) -> Option<mod_api::RegistryName> {
                <Self as mod_api::prelude::KeysBackend>::enchantment_key_id(self, key)
            }
            fn biome_key_id(
                &self,
                key: &mod_api::BiomeKey,
            ) -> Result<&str, mod_api::BiomeKeyError> {
                <Self as mod_api::prelude::KeysBackend>::biome_view(self, key).map(|view| view.id())
            }
            fn dimension_key_id(&self, key: mod_api::DimensionKey) -> Option<&str> {
                <Self as mod_api::prelude::LevelDimensionBackend>::dimension_key_id(self, key)
            }
            fn recipe_ingredient_item_keys(
                &self,
                ingredient: &mod_api::Ingredient,
            ) -> Result<std::sync::Arc<[mod_api::ItemKey]>, mod_api::IngredientKeysError> {
                <Self as mod_api::prelude::RecipesBackend>::recipe_ingredient_item_keys(
                    self, ingredient,
                )
            }
        }
    };
}
