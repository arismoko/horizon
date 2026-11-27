#![cfg(test)]
use mod_api::SignEditView;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::Screen;
#[cfg(test)]
pub(crate) mod client_screen_tests {
    use super::*;
    use crate::game::remote::tests::connect_menu_test;
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_client_gameplay::game::books::BookScreen;
    use sunrise_core::geometry::Vec3;
    use sunrise_items::items::ItemStack;
    use sunrise_level::containers::BlockEntity;
    use sunrise_level::signs_data::{SignData, SignText};
    use sunrise_protocol::connection::ConnReader;
    use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
    use sunrise_sim::mobs::Mob;

    /// What the window sent since the last call, as play packets.
    fn sent(reader: &mut ConnReader) -> Vec<S> {
        reader
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let mut packets = Vec::new();
        while let Ok((id, body)) = reader.recv() {
            packets.extend(S::decode(id, &body).ok());
        }
        packets
    }

    /// A window logged in to the overworld at (0.5, 65, 0.5), its player made
    /// by the server's position packet, with everything the login sent drained.
    fn logged_in(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) -> ConnReader {
        let (mut reader, _writer) = connect_menu_test(game);
        game.handle_packet(C::Login(play::Login {
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
        }))
        .unwrap();
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [0.5, 65.0, 0.5],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives: 0,
        }))
        .unwrap();
        sent(&mut reader);
        reader
    }

    /// The chest menu's network id, from the prepared registries report.
    fn chest_menu() -> i32 {
        menu_id("minecraft:generic_9x3")
    }
    fn menu_id(name: &str) -> i32 {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
        let report = sunrise_protocol::registry::Builtin::find(&root)
            .expect("the prepared resources carry registries.json");
        let builtin = sunrise_protocol::registry::Builtin::load(&report).unwrap();
        builtin.get("minecraft:menu").unwrap().id(name).unwrap()
    }

    /// LecternScreen.onClose closes its menu (`closeContainer`): a lectern opened by the server
    /// closes through the screen close with the menu's id, and its book overlay is gone.
    #[test]
    #[ignore = "daily"]
    fn closing_a_lectern_sends_its_menu_id() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            let menu = menu_id("minecraft:lectern");
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 3,
                menu,
                title: sunrise_core::nbt::Tag::String("Lectern".into()),
            }))
            .unwrap();
            sent(&mut reader);

            assert!(game.close_screen(), "the lectern's book is open");
            assert!(game.client.book.is_none());
            let Some(S::ContainerClose(close)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::ContainerClose(_)))
            else {
                panic!("the lectern's menu close goes out");
            };
            assert_eq!(close.container, 3);
        });
    }

    /// A link's `ConfirmLinkScreen` over a lectern is the foreground screen: closing it answers
    /// no and leaves the lectern's book and the server's menu open, with nothing sent. The next
    /// close is the lectern's own.
    #[test]
    #[ignore = "daily"]
    fn a_confirmation_over_a_lectern_closes_alone() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 3,
                menu: menu_id("minecraft:lectern"),
                title: sunrise_core::nbt::Tag::String("Lectern".into()),
            }))
            .unwrap();
            // Screen.clickUrlAction with Prompt on Links on, as a page's link opens it.
            (
                game.client.settings.chat_links,
                game.client.settings.chat_links_prompt,
            ) = (true, true);
            game.click_url("https://example.org".into());
            assert!(
                game.client.link_prompt.is_some(),
                "the confirmation is over the book"
            );
            sent(&mut reader);
            fn menu(game: &sunrise_client_gameplay::game::ClientGame<'_>) -> Option<i32> {
                game.client
                    .remote
                    .as_ref()
                    .unwrap()
                    .menu
                    .as_ref()
                    .map(|m| m.0)
            }

            assert!(game.close_screen(), "the confirmation was open");
            assert!(game.client.link_prompt.is_none());
            assert!(
                matches!(
                    game.client.book,
                    Some(BookScreen::View {
                        lectern: Some(_),
                        ..
                    })
                ),
                "the lectern's book stays open"
            );
            assert_eq!(menu(game), Some(3), "the server's lectern menu stays open");
            assert!(sent(&mut reader).is_empty(), "no menu close went out");

            assert!(game.close_screen(), "the lectern is the screen now");
            assert!(game.client.book.is_none());
            assert_eq!(menu(game), None);
            assert!(
                sent(&mut reader)
                    .iter()
                    .any(|p| matches!(p, S::ContainerClose(c) if c.container == 3))
            );
        });
    }

    /// `BookViewScreen` and `BookEditScreen` close as `Screen.onClose`: the book goes and
    /// nothing is sent, neither a menu close (no menu is open) nor the edited pages.
    #[test]
    #[ignore = "daily"]
    fn closing_an_ordinary_book_sends_nothing() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.open_book(BookScreen::View {
                pages: Vec::new(),
                page: 0,
                lectern: None,
            });
            assert!(game.client.book.is_some());
            sent(&mut reader);
            assert!(game.close_screen(), "the written book was open");
            assert!(game.client.book.is_none());
            assert!(
                sent(&mut reader).is_empty(),
                "a written book's close sends nothing"
            );

            *game.held_mut() = Some(ItemStack::new("minecraft:writable_book", 1));
            let stack = game.held().unwrap().clone();
            assert!(game.remote_use_book(&stack), "the editor opened");
            game.book_input(None, Some("unsaved"));
            sent(&mut reader);
            assert!(game.close_screen(), "the editor was open");
            assert!(game.client.book.is_none());
            assert!(
                sent(&mut reader).is_empty(),
                "Escape in the editor sends no edit and no menu close"
            );
        });
    }

    /// Vanilla 26.3 `BookEditScreen` Done (`saveChanges`) matches the witness recorded by `tools/test_writable_book_parity.py`.
    #[test]
    #[ignore = "run by tools/test_writable_book_parity.py"]
    fn done_on_an_unedited_book_sends_it_and_drops_its_filtered_text() {
        run_done_on_an_unedited_book_sends_it_and_drops_its_filtered_text();
    }

    pub(crate) fn run_done_on_an_unedited_book_sends_it_and_drops_its_filtered_text() {
        use std::fmt::Write as _;
        #[derive(serde::Deserialize)]
        struct Witness {
            minecraft: String,
            book_edit_cases: Vec<BookEditCase>,
        }
        #[derive(serde::Deserialize)]
        struct BookEditCase {
            name: String,
            slot: usize,
            input_snbt: String,
            screen_closed: bool,
            sent_edit_book_packets: Vec<EditBookPacket>,
            after_raw_pages: Vec<String>,
            after_filtered_pages: Vec<Option<String>>,
            after_component_canon: String,
            after_stack_canon: String,
        }
        #[derive(serde::Deserialize)]
        struct EditBookPacket {
            slot: i32,
            pages: Vec<String>,
            title: Option<String>,
            wire_hex: String,
        }

        let witness_path = std::env::var_os("SUNRISE_WRITABLE_BOOK_WITNESS").unwrap_or_else(|| {
            panic!(
                "SUNRISE_WRITABLE_BOOK_WITNESS is unset; run tools/test_writable_book_parity.py, which records the witness"
            )
        });
        let witness_path = std::path::PathBuf::from(witness_path);
        let witness_path = if witness_path.is_dir() {
            witness_path.join("writable_book.json")
        } else {
            witness_path
        };
        let witness_raw = std::fs::read_to_string(&witness_path).unwrap_or_else(|e| {
            panic!(
                "failed to read witness {} (record via tools/test_writable_book_parity.py): {e}",
                witness_path.display()
            )
        });
        let witness: Witness = serde_json::from_str(&witness_raw).unwrap();
        assert_eq!(witness.minecraft, "26.3");
        with_client_game(|game| {
            let mut reader = logged_in(game);
            for case in &witness.book_edit_cases {
                game.selected = case.slot;
                let book = sunrise_items::item_data::parse_stack(&case.input_snbt)
                    .unwrap()
                    .unwrap();
                *game.held_mut() = Some(book.clone());
                assert!(
                    game.remote_use_book(&book),
                    "the editor opened ({})",
                    case.name
                );
                sent(&mut reader);
                game.save_book(false);
                assert_eq!(
                    game.client.book.is_none(),
                    case.screen_closed,
                    "{}",
                    case.name
                );
                let edits: Vec<_> = sent(&mut reader)
                    .into_iter()
                    .filter(|p| matches!(p, S::EditBook(_)))
                    .collect();
                let expected_edits: Vec<_> = case
                    .sent_edit_book_packets
                    .iter()
                    .map(|p| {
                        S::EditBook(play::EditBook {
                            slot: p.slot,
                            pages: p.pages.clone(),
                            title: p.title.clone(),
                        })
                    })
                    .collect();
                assert_eq!(edits, expected_edits, "{}", case.name);
                for (sent_pkt, expected) in edits.iter().zip(&case.sent_edit_book_packets) {
                    let S::EditBook(edit) = sent_pkt else {
                        unreachable!();
                    };
                    let mut w = sunrise_core::vanilla_bytes::Writer::new();
                    <play::EditBook as sunrise_protocol::buf::Codec<play::EditBook>>::write(
                        edit, &mut w,
                    );
                    let mut hex = String::with_capacity(w.data.len() * 2);
                    for b in &w.data {
                        write!(&mut hex, "{b:02x}").unwrap();
                    }
                    assert_eq!(
                        hex, expected.wire_hex,
                        "wire_hex mismatch for {}",
                        case.name
                    );
                }
                let held = game.held().expect("book stays in hand");
                let data = held.data().expect("book keeps pages");
                assert_eq!(data.writable_pages, case.after_raw_pages, "{}", case.name);
                let expected_filtered = if case.after_filtered_pages.iter().all(Option::is_none) {
                    Vec::new()
                } else {
                    case.after_filtered_pages.clone()
                };
                assert_eq!(data.writable_filtered, expected_filtered, "{}", case.name);
                let saved = sunrise_items::item_data::component_snapshot(held).unwrap();
                assert_eq!(
                    sunrise_items::item_data::canon_tag(
                        saved.get("minecraft:writable_book_content").unwrap()
                    ),
                    case.after_component_canon,
                    "{}",
                    case.name
                );
                assert_eq!(
                    sunrise_items::item_data::canon_stack(Some(held), game.res.items()),
                    case.after_stack_canon,
                    "{}",
                    case.name
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn closing_a_chest_sends_its_menu_id_and_closes_the_client_menu() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            let menu = chest_menu();
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 7,
                menu,
                title: sunrise_core::nbt::Tag::String("Chest".into()),
            }))
            .unwrap();
            assert_eq!(game.client.screen, Screen::Inventory);
            assert!(game.screen_title().is_some(), "the server's menu is open");
            sent(&mut reader);

            assert!(game.close_screen(), "a chest screen is open");
            assert_eq!(game.client.screen, Screen::Playing);
            assert!(game.screen_title().is_none(), "the client menu is closed");
            let Some(S::ContainerClose(close)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::ContainerClose(_)))
            else {
                panic!("the container close with the menu's id goes out");
            };
            assert_eq!(close.container, 7);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn closing_the_inventory_sends_the_close_for_id_zero() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.set_screen(Screen::Inventory);
            assert!(game.close_screen());
            assert_eq!(game.client.screen, Screen::Playing);
            let Some(S::ContainerClose(close)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::ContainerClose(_)))
            else {
                panic!("the player container close goes out");
            };
            assert_eq!(close.container, 0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn closing_with_no_screen_reports_false_and_sends_nothing() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            assert_eq!(game.client.screen, Screen::Playing);
            assert!(!game.close_screen());
            assert!(sent(&mut reader).is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn opening_the_inventory_is_local_and_needs_no_screen() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            assert!(game.open_inventory());
            assert_eq!(game.client.screen, Screen::Inventory);
            assert!(
                sent(&mut reader).is_empty(),
                "the inventory opens without telling the server"
            );
            assert!(!game.open_inventory(), "a screen is already open");
            assert_eq!(game.client.screen, Screen::Inventory);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn opening_the_inventory_riding_a_horse_sends_the_player_command() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            // The player's keys act once the client told the server it loaded.
            game.client.streaming.insert_remote(
                &mut game.common.world,
                sunrise_level::chunk::Chunk::empty(0, 0),
            );
            game.poll_stationary_client().unwrap();
            sent(&mut reader); // PlayerLoaded precedes the inventory action.
            let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            let mut horse = Mob::new(42, kind, Vec3::new(0.5, 65.0, 0.5), 0.0);
            horse.extra.tame = true;
            game.mobs.push(horse);
            game.riding = Some(42);
            // An overlay counts as an open screen even when a mount would otherwise
            // make open_inventory report that its command was queued.
            let at = (3, 64, -2);
            game.world
                .block_entities
                .insert(at, BlockEntity::Sign(SignData::default()));
            game.remote_open_sign_editor(play::OpenSignEditor {
                pos: [3, 64, -2],
                front: true,
            });
            assert!(!game.open_inventory(), "the sign editor is already open");
            assert!(game.sign_editor().is_some());
            assert_eq!(game.client.screen, Screen::Playing);
            assert!(
                sent(&mut reader).is_empty(),
                "no vehicle menu command was sent"
            );
            assert!(game.close_screen());
            sent(&mut reader);
            assert!(game.open_inventory());
            // The key's press is replayed at the player's next tick, so the
            // command goes out there, as the key's does.
            game.tick_remote_level().unwrap();
            let Some(S::PlayerCommand(command)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::PlayerCommand(_)))
            else {
                panic!("the open-inventory command goes out");
            };
            assert_eq!((command.id, command.action), (7, 5));
            assert_eq!(command.data, 0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_sign_editor_reports_its_sign_and_closing_sends_the_update() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            let at = (3, 64, -2);
            let sign = SignData {
                front: SignText {
                    lines: [
                        "first".into(),
                        "second".into(),
                        "third".into(),
                        "fourth".into(),
                    ],
                    ..SignText::default()
                },
                ..SignData::default()
            };
            game.world
                .block_entities
                .insert(at, BlockEntity::Sign(sign));
            game.remote_open_sign_editor(play::OpenSignEditor {
                pos: [at.0, at.1, at.2],
                front: true,
            });
            assert_eq!(
                game.sign_editor(),
                Some(SignEditView {
                    pos: BlockPos::new(3, 64, -2),
                    front: true,
                })
            );

            assert!(game.close_screen());
            assert_eq!(game.sign_editor(), None);
            let Some(S::SignUpdate(update)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::SignUpdate(_)))
            else {
                panic!("the sign update goes out with the close");
            };
            assert_eq!(update.pos, [3, 64, -2]);
            assert!(update.front);
            assert_eq!(
                update.lines,
                [
                    "first".to_owned(),
                    "second".to_owned(),
                    "third".to_owned(),
                    "fourth".to_owned()
                ]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn no_sign_editor_reports_no_sign() {
        with_client_game(|game| {
            logged_in(game);
            assert_eq!(game.sign_editor(), None);
        });
    }
}
