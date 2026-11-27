#![cfg(test)]

#[cfg(test)]
mod tests {
    use mod_api::SelectorSort;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_entity_selectors::SelectorToken;
    use sunrise_gameplay_common::game::native_entity_selectors::selector_tokens;

    #[test]
    fn panicking_selector_base_disables_only_that_selector_and_emits_panic_notice() {
        struct PanickingBase(Arc<AtomicUsize>);
        impl mod_api::EntitySelectorType for PanickingBase {
            fn base(&self) -> mod_api::SelectorBase {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("fixture selector base panic")
            }
            fn matches(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mod_api::CommandSource,
                _: mod_api::EntityRef,
            ) -> bool {
                false
            }
        }
        struct HealthyBase;
        impl mod_api::EntitySelectorType for HealthyBase {
            fn base(&self) -> mod_api::SelectorBase {
                mod_api::SelectorBase::new(
                    true,
                    Some(2),
                    SelectorSort::Nearest,
                    "test.healthy_base",
                )
            }
            fn matches(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mod_api::CommandSource,
                _: mod_api::EntityRef,
            ) -> bool {
                true
            }
        }
        struct BasePanicMod(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for BasePanicMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_entity_selector("broken_base", PanickingBase(self.0.clone()))
                    .unwrap();
                r.register_entity_selector("Healthy_1.0", HealthyBase)
                    .unwrap();
            }
        }

        let base_calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "selector_base_panic_fixture",
            Box::new(BasePanicMod(base_calls.clone())),
        )]
        .into();
        let _ = mod_api::testing::rebuild_parts(|| -> Result<(), ()> {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, || {
                let _ = mod_api::__private::take_hook_panics();
                let first = selector_tokens();
                assert_eq!(first.len(), 1);
                assert_eq!(first[0].name, "Healthy_1.0");
                assert_eq!(base_calls.load(Ordering::Relaxed), 1);

                let notices: Vec<_> = mod_api::__private::take_hook_panics()
                    .into_iter()
                    .filter(|notice| notice.mod_id == "selector_base_panic_fixture")
                    .collect();
                assert_eq!(notices.len(), 1);
                assert_eq!(notices[0].hook, "entity_selector.base");

                // The poisoned selector is skipped on subsequent calls while its sibling stays active.
                let second = selector_tokens();
                assert_eq!(second, first);
                assert_eq!(base_calls.load(Ordering::Relaxed), 1);
                let later_notices: Vec<_> = mod_api::__private::take_hook_panics()
                    .into_iter()
                    .filter(|notice| notice.mod_id == "selector_base_panic_fixture")
                    .collect();
                assert!(later_notices.is_empty());
            });
            Err(())
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn synced_selector_tokens_survive_client_registry_refresh() {
        struct ClientMustNotBuildSelector(Arc<AtomicUsize>);
        impl mod_api::EntitySelectorType for ClientMustNotBuildSelector {
            fn base(&self) -> mod_api::SelectorBase {
                self.0.fetch_add(1, Ordering::Relaxed);
                mod_api::SelectorBase::new(
                    true,
                    Some(3),
                    SelectorSort::Nearest,
                    "test.local_allies",
                )
            }

            fn matches(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mod_api::CommandSource,
                _: mod_api::EntityRef,
            ) -> bool {
                true
            }
        }

        struct LocalSelector(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for LocalSelector {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                registrar
                    .register_entity_selector(
                        "local_allies",
                        ClientMustNotBuildSelector(self.0.clone()),
                    )
                    .unwrap();
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "selector_client_fixture",
            Box::new(LocalSelector(calls.clone())),
        )]
        .into();
        let _ = mod_api::testing::rebuild_parts(|| -> Result<(), ()> {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, || {
                crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(client, "SelectorSyncTest").unwrap();
                    assert!(game.ui.registries.selector_tokens.is_empty());
                    assert_eq!(calls.load(Ordering::Relaxed), 0);
                    let tokens = vec![SelectorToken {
                        name: "remote_allies".into(),
                        tooltip: "test.remote_allies".into(),
                        players_only: true,
                        limit: 1,
                        sort: "nearest",
                    }];
                    game.apply_packet(sunrise_protocol::packets::play::Clientbound::CustomPayload(
                        sunrise_protocol::packets::configuration::CustomPayload {
                            channel: sunrise_sim::lan::commands::selector_sync::CHANNEL.into(),
                            data: sunrise_sim::lan::commands::selector_sync::encode(&tokens),
                        },
                    ))
                    .unwrap();
                    assert_eq!(game.ui.registries.selector_tokens, tokens);
                    game.build_registries();
                    assert_eq!(game.ui.registries.selector_tokens, tokens);
                    assert_eq!(calls.load(Ordering::Relaxed), 0);
                    for data in [
                        vec![2],
                        vec![9],
                        sunrise_sim::lan::commands::selector_sync::encode(&tokens),
                    ] {
                        game.apply_packet(
                            sunrise_protocol::packets::play::Clientbound::CustomPayload(
                                sunrise_protocol::packets::configuration::CustomPayload {
                                    channel: sunrise_sim::lan::commands::selector_sync::CHANNEL
                                        .into(),
                                    data,
                                },
                            ),
                        )
                        .unwrap();
                        assert!(game.ui.registries.selector_tokens.is_empty());
                    }
                });
            });
            Err(())
        });
    }
}
