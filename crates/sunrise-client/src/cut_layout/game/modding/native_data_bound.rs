#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::native_data_bound::NativeDataBoundGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_data_bound::tests::DefaultsProbe;
    use sunrise_gameplay_common::game::native_data_bound::tests::Probe;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_data_bound::tests::dirt_stack_size;

    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_data_bound::tests::install_default_component_loads;

    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_data_bound::tests::login_packet;

    use crate::game::native_entity_refs::tests::with_game;

    #[cfg(feature = "client")]
    use mod_api::{
        DataLoadContext, DefaultComponentsBoundEvent, Hooks, ModifyDefaultComponentsEvent,
        ReloadContext, TagsUpdatedEvent,
    };
    use mod_api::{ModEntry, TagsUpdateCause};

    use std::sync::{Arc, Mutex};

    #[cfg(feature = "client")]
    use sunrise_protocol::packets::{
        configuration::{RegistryTags, TagEntry, UpdateTags},
        play::Clientbound as C,
    };

    #[test]
    #[ignore = "daily"]
    fn defaults_are_announced_bound_before_the_tags() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("bound_probe", Box::new(Probe(heard.clone())))].into();
        let reload = TagsUpdateCause::ServerDataLoad { reload: true };
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                heard.lock().unwrap().clear();
                game.announce_data_bound(reload);
            });
        });
        assert_eq!(
            *heard.lock().unwrap(),
            [("bound", reload), ("tags", reload)]
        );
    }
    /// A single player's first open: the data load announces the defaults
    /// bound before the tags, `should_update_static_data()` holds for its
    /// cause, and a listener's read inside the notice returns the stack
    /// size its own `modify_default_components` published.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn the_first_open_announces_bound_defaults_readable() {
        install_default_component_loads();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "defaults_probe",
            Box::new(DefaultsProbe(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let scratch = tempfile::tempdir().unwrap();
                let registry = Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla());
                let create = sunrise_sim::world_options::WorldOptions::default();
                let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
                    "defaults",
                    Some("defaults"),
                    Some(1),
                    true,
                    Some(&create),
                    registry.clone(),
                )
                .unwrap();
                request.save_root = scratch.path().to_owned();
                let loader = sunrise_gameplay::game::world_loading::WorldLoader::start(
                    request,
                    game.res.clone(),
                    game.pack.clone(),
                    game.options.clone(),
                    Arc::new(sunrise_world::datafix::progress::UpgradeProgress::new()),
                )
                .unwrap();
                let prepared = game.prepare_early_game_rules(loader).unwrap();
                game.finish_open_world(prepared).unwrap();
            });
            crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                game.announce_data_bound(TagsUpdateCause::ClientPacketReceived {
                    integrated_server: true,
                });
            });
        });
        let seen = seen.lock().unwrap();
        let server = TagsUpdateCause::ServerDataLoad { reload: false };
        let integrated = TagsUpdateCause::ClientPacketReceived {
            integrated_server: true,
        };
        assert_eq!(
            *seen,
            [
                (server, Some(mod_api::Tag::Int(99))),
                (integrated, Some(mod_api::Tag::Int(99)))
            ]
        );
        assert!(server.should_update_static_data());
        assert!(!integrated.should_update_static_data());
    }
    /// A remote login: the applied tags announce the defaults bound once,
    /// with the client cause, before the tags notice; a listener's read
    /// inside returns the published defaults. An update in play refires.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_remote_login_announces_the_client_cause_before_the_tags() {
        install_default_component_loads();
        struct Both(Arc<Mutex<Vec<&'static str>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Both {
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
                assert_eq!(
                    stack,
                    Some(mod_api::Tag::Int(99)),
                    "the defaults are published"
                );
                assert_eq!(
                    event.cause,
                    TagsUpdateCause::ClientPacketReceived {
                        integrated_server: false
                    }
                );
                assert!(event.cause.should_update_static_data());
                self.0.lock().unwrap().push("bound");
            }
            fn tags_updated(&self, _: &dyn ReloadContext, _: &mut TagsUpdatedEvent) {
                self.0.lock().unwrap().push("tags");
            }
        }

        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("login_probe", Box::new(Both(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                // A client's view of a server's tags: the login applies them.
                let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "BoundLogin").unwrap();
                let builtin = game.client.remote.as_ref().unwrap().builtin.clone();
                let id = |name: &str| builtin.get("minecraft:item").unwrap().id(name).unwrap();
                let registries = UpdateTags {
                    registries: vec![RegistryTags {
                        registry: "minecraft:item".into(),
                        tags: vec![TagEntry {
                            name: "test:items".into(),
                            ids: vec![id("minecraft:stick")],
                        }],
                    }],
                };
                heard.lock().unwrap().clear();
                game.handle_packet(login_packet()).unwrap();
                game.handle_packet(C::UpdateTags(registries)).unwrap();
            });
        });
        assert_eq!(*heard.lock().unwrap(), ["bound", "tags"]);
    }
}
