#[cfg(feature = "client")]
use super::*;
#[cfg(feature = "client")]
use sunrise_protocol::{packets::play::CommandSuggestions, text::Language};

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `ClientPacketListener.handleCommandSuggestions`: forward the reply to the command edit box.
    pub(in crate::game) fn handle_command_suggestions(&mut self, packet: CommandSuggestions) {
        let session = self.client.remote.as_ref().map(|r| r.lang.as_ref());
        let list: Vec<_> = if sunrise_game_data::lang::has_mod_entries() {
            let current = sunrise_game_data::lang::current();
            packet
                .suggestions
                .into_iter()
                .map(|entry| {
                    (
                        entry.text,
                        entry.tooltip.as_ref().map(|tag| {
                            plain_with(tag, |key| {
                                current
                                    .as_deref()
                                    .and_then(|published| published.mod_entry(key))
                                    .or_else(|| {
                                        session.and_then(|map| map.get(key).map(String::as_str))
                                    })
                            })
                        }),
                    )
                })
                .collect()
        } else {
            let empty;
            let lang = match session {
                Some(lang) => lang,
                None => {
                    empty = Language::default();
                    &empty
                }
            };
            packet
                .suggestions
                .into_iter()
                .map(|entry| {
                    (
                        entry.text,
                        entry.tooltip.as_ref().map(|tag| plain(tag, lang)),
                    )
                })
                .collect()
        };
        let range = (packet.start.max(0) as usize, packet.length.max(0) as usize);
        self.chat_server_suggestions(packet.id, range, list.clone());
        self.command_block_server_suggestions(packet.id, range, list);
    }
}

#[cfg(all(test, feature = "client"))]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_client_game;
    use sunrise_client::{
        chat::{ChatAbilities, ChatScreen, ChatVisibility, Commands},
        command::Suggests,
        lang::{self, Language as ClientLanguage},
        screen_draw::{Draw, Measure},
    };
    use sunrise_protocol::packets::play::CommandSuggestion;

    #[test]
    #[ignore = "daily"]
    fn command_suggestion_tooltips_follow_runtime_translations_on_existing_connection() {
        with_client_game(|game| {
            let _publication = lang::isolate_publication(ClientLanguage::of(
                "en_us",
                [("test.cmd.tip".into(), "pack tip".into())],
            ));
            {
                let _guard = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "tip-mod",
                        hook: "test",
                        part: None,
                    },
                );
                assert_eq!(
                    game.set_translations(
                        "en_us",
                        vec![("test.cmd.tip".into(), "initial mod tip".into())],
                    ),
                    Some(1)
                );
            }
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "cmd-suggestions-lang").unwrap();
            game.common.ui.registries.permission = 4;

            let tree = &mut game.client.remote.as_mut().unwrap().chat_tree;
            let child = |d: &sunrise_client::command::Dispatcher, n: usize, name: &str| {
                d.nodes[n]
                    .children
                    .iter()
                    .copied()
                    .find(|&c| d.nodes[c].name() == name)
                    .expect(name)
            };
            let id_node = child(tree, child(tree, child(tree, 0, "bossbar"), "get"), "id");
            tree.nodes[id_node].suggests = Some(Suggests::AskServer);

            let receive_tooltip = |game: &mut crate::game::ClientGame<'_>, key: &str| -> String {
                let m = Measure(&game.client.gui.atlas.font);
                let c = Commands {
                    dispatcher: &game.client.remote.as_ref().unwrap().chat_tree,
                    registries: &game.common.ui.registries,
                };
                let mut screen = ChatScreen::new(
                    &m,
                    &c,
                    "",
                    false,
                    0,
                    640,
                    360,
                    ChatAbilities::new(ChatVisibility::Full),
                );
                screen.type_text(&m, &c, "/bossbar get pt");
                let (request_id, _) = screen.take_request().expect("server suggestion requested");
                game.client.chat_ui.chat_screen = Some(screen);

                let tooltip = Tag::Compound(
                    [("translate".into(), Tag::String(key.into()))]
                        .into_iter()
                        .collect(),
                );
                game.handle_command_suggestions(CommandSuggestions {
                    id: request_id,
                    start: 13,
                    length: 2,
                    suggestions: vec![CommandSuggestion {
                        text: "pt6:check".into(),
                        tooltip: Some(tooltip),
                    }],
                });

                let m = Measure(&game.client.gui.atlas.font);
                let screen = game.client.chat_ui.chat_screen.as_mut().unwrap();
                let initial = screen.draw(&m, (0, 0), 0, 0);
                let (x, y) = initial
                    .iter()
                    .find_map(|d| match d {
                        Draw::Text { text, x, y, .. } if text.plain() == "pt6:check" => {
                            Some((*x, *y))
                        }
                        _ => None,
                    })
                    .expect("suggestion row drawn");
                let hovered = screen.draw(&m, (x, y), 0, 0);
                hovered
                    .iter()
                    .find_map(|d| match d {
                        Draw::Tooltip { lines, .. } => Some(
                            lines
                                .iter()
                                .map(|line| line.plain())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        _ => None,
                    })
                    .expect("suggestion tooltip drawn")
            };

            assert_eq!(receive_tooltip(game, "test.cmd.tip"), "initial mod tip");
            {
                let _guard = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "tip-mod",
                        hook: "test",
                        part: None,
                    },
                );
                game.clear_translations();
            }
            assert_eq!(receive_tooltip(game, "test.cmd.tip"), "pack tip");
            {
                let _guard = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "tip-mod",
                        hook: "test",
                        part: None,
                    },
                );
                assert_eq!(
                    game.set_translations(
                        "en_us",
                        vec![
                            ("test.cmd.tip".into(), "mod tip".into()),
                            ("test.cmd.extra".into(), "extra tip".into()),
                        ],
                    ),
                    Some(2)
                );
            }
            assert_eq!(receive_tooltip(game, "test.cmd.tip"), "mod tip");
            assert_eq!(receive_tooltip(game, "test.cmd.extra"), "extra tip");
            {
                let _guard = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "tip-mod",
                        hook: "test",
                        part: None,
                    },
                );
                game.clear_translations();
            }
            assert_eq!(receive_tooltip(game, "test.cmd.tip"), "pack tip");
            assert_eq!(receive_tooltip(game, "test.cmd.extra"), "test.cmd.extra");
        });
    }
}
