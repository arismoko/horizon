#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;

    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;

    use mod_api::CommandInvocation;

    /// The issue's case: a console `execute in minecraft:the_nether` reaches the mod with the
    /// nether as its level; without it, the console's overworld and spawn corner.
    #[test]
    #[ignore = "daily"]
    fn a_console_command_executes_in_the_level_execute_in_names() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::native_command_reads::tests::a_console_command_executes_in_the_level_execute_in_names",
            || {
                use std::sync::{Arc, Mutex};
                type Heard = Arc<Mutex<Vec<(mod_api::CommandSource, CommandInvocation)>>>;
                struct Where(Heard);
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Where {
                    fn register_commands(
                        &self,
                        _: &mut dyn mod_api::ServerHost,
                        event: &mut mod_api::RegisterCommandsEvent<'_>,
                    ) {
                        let heard = self.0.clone();
                        event.commands.push(mod_api::Command::new("where").executes(
                            move |_, source, command| {
                                heard
                                    .lock()
                                    .unwrap()
                                    .push((source.clone(), command.clone()));
                                Ok(1)
                            },
                        ));
                    }
                }
                let heard: Heard = Arc::default();
                mod_platform::testing::with_mods(
                    vec![mod_api::ModEntry::new(
                        "where",
                        Box::new(Where(heard.clone())),
                    )]
                    .into(),
                    || {
                        super::super::native_guest_turn_tests::with_guests(|game| {
                            game.build_registries();
                            game.register_native_commands();
                            assert!(game.native_run_command("where"));
                            assert!(
                                game.native_run_command(
                                    "execute in minecraft:the_nether run where"
                                )
                            );
                            let heard = heard.lock().unwrap();
                            assert_eq!(heard.len(), 2);
                            for (source, _) in heard.iter() {
                                assert!(
                                    matches!(source, mod_api::CommandSource::Console { .. }),
                                    "{source:?}"
                                );
                            }
                            assert_eq!(heard[0].1.dimension, mod_api::DimensionKey::OVERWORLD);
                            assert_eq!(heard[0].1.position, game.server_position());
                            assert_eq!(heard[1].1.dimension, mod_api::DimensionKey::NETHER);
                        });
                    },
                );
            },
        );
    }
    #[test]
    #[ignore = "daily"]
    fn a_profile_keeps_its_login_properties() {
        use sunrise_core::profile::AccountProperty;

        super::super::native_guest_turn_tests::with_guests(|game| {
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.properties = vec![
                        AccountProperty {
                            name: "textures".into(),
                            value: "e30=".into(),
                            signature: Some("sig".into()),
                        },
                        AccountProperty {
                            name: "extra".into(),
                            value: "plain".into(),
                            signature: None,
                        },
                    ];
                })
                .unwrap();
            let player = game.player(12).unwrap();
            let check = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let profile = player.profile(game).unwrap();
                assert_eq!(profile.uuid, 12);
                assert_eq!(profile.name, "guest12");
                let mut properties = profile.properties();
                let signed = properties.next().unwrap();
                assert_eq!(
                    (signed.name, signed.value, signed.signature),
                    ("textures", "e30=", Some("sig"))
                );
                let unsigned = properties.next().unwrap();
                assert_eq!(
                    (unsigned.name, unsigned.value, unsigned.signature),
                    ("extra", "plain", None)
                );
                assert!(properties.next().is_none());
            };
            check(game);
            game.as_guest(12, |game| {
                check(game);
                game.as_guest(13, |game| check(game)).unwrap();
            })
            .unwrap();

            let offline = game.player(13).unwrap().profile(game).unwrap();
            assert_eq!((offline.uuid, offline.name), (13, "guest13"));
            assert!(offline.properties().next().is_none());

            let entity = player.entity();
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    mod_api::WorldEpoch::new(entity.epoch().get() + 1),
                    entity.id(),
                ),
                12,
            );
            assert!(stale.profile(game).is_none());
        });
    }
}
