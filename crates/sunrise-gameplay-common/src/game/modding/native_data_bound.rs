//! The notices a side's freshly bound data gives mods, in NeoForge's order.

pub trait NativeDataBoundGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Items' default components are bound (`DefaultDataComponentsBoundEvent`), then the
    /// tags (`TagsUpdatedEvent`), both for `cause`.
    fn announce_data_bound(&mut self, cause: mod_api::TagsUpdateCause) {
        self.native_default_components_bound(cause);
        self.native_tags_updated(cause);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeDataBoundGameExt1<'__sg> for G {}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use mod_api::{
        DataLoadContext, DefaultComponentsBoundEvent, Hooks, Identifier,
        ModifyDefaultComponentsEvent, ReloadContext, TagsUpdateCause, TagsUpdatedEvent,
    };
    use std::sync::{Arc, Mutex};
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    pub struct Probe(pub Arc<Mutex<Vec<(&'static str, TagsUpdateCause)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn default_components_bound(
            &self,
            _: &dyn ReloadContext,
            event: &mut DefaultComponentsBoundEvent,
        ) {
            self.0.lock().unwrap().push(("bound", event.cause));
        }
        fn tags_updated(&self, _: &dyn ReloadContext, event: &mut TagsUpdatedEvent) {
            self.0.lock().unwrap().push(("tags", event.cause));
        }
    }

    /// The bound notice's default read: the value the mod's
    /// `modify_default_components` set, off the published defaults.
    pub fn dirt_stack_size(context: &dyn ReloadContext) -> Option<mod_api::Tag> {
        let key = context.item_key(&Identifier::parse("minecraft:dirt").unwrap())?;
        context.item_default_component(key, "minecraft:max_stack_size")
    }

    /// The load boundary a test's defaults edit flows through: the same
    /// wiring `entry.rs` installs for the game, first-installed process-wide.
    pub fn install_default_component_loads() {
        let _ = sunrise_world::native_load_hooks::install(
            sunrise_world::native_load_hooks::NativeLoadHooks {
                item_ability_default: sunrise_items::item_abilities::vanilla_can_perform,
                loot_ability: |_, _, _, _| {},
                loot_table_load: |_, _, _| {},
                modify_default_components: |context, item, components| {
                    if !mod_api::__private::listened(mod_api::HookId::ModifyDefaultComponents) {
                        return;
                    }
                    let reader = sunrise_level::api_context::LoadReader(context);
                    let context = reader.context();
                    mod_api::dispatch_modify_default_components(&context, item, components);
                },
            },
        );
    }

    /// A mod that raises dirt's default stack size to 99 at its load, and
    /// records, per bound notice, the cause and the stack size it reads.
    pub struct DefaultsProbe(pub Arc<Mutex<Vec<(TagsUpdateCause, Option<mod_api::Tag>)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for DefaultsProbe {
        fn modify_default_components(
            &self,
            _: &dyn DataLoadContext,
            event: &mut ModifyDefaultComponentsEvent<'_>,
        ) {
            if event.item == "minecraft:dirt" {
                event
                    .components
                    .insert("minecraft:max_stack_size".into(), serde_json::json!(99));
            }
        }
        fn default_components_bound(
            &self,
            context: &dyn ReloadContext,
            event: &mut DefaultComponentsBoundEvent,
        ) {
            let stack = dirt_stack_size(context);
            self.0.lock().unwrap().push((event.cause, stack));
        }
    }

    /// The login packet the remote-tags tests use.
    pub fn login_packet() -> C {
        C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: play::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 1,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: false,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: false,
            enforces_secure_chat: false,
        })
    }
}

use crate::game::native_hooks::default_components_bound::GameHooks as _;

use crate::game::native_hooks::tags_updated::GameHooks as _;
