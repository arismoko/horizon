#![cfg(test)]
//! `ItemPart::is_primary_item_for`, `apply_enchantments` and `is_not_replaceable_by_pick_action`
//! through the game: the answers each part gets, starting from vanilla's, and what the game does
//! with what it leaves. Each test installs process-wide parts: run alone with the prepared
//! resources (`SUNRISE_TEST_RESOURCES`).
use mod_api::{
    ApplyEnchantmentsEvent, Hooks, IsNotReplaceableByPickActionEvent, IsPrimaryItemForEvent,
    ItemPart, ModEntry, Registrar, RegistrationContext, SupportsEnchantmentEvent,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use sunrise_core::random::LegacyRandom;
use sunrise_gameplay_common::game::native_hooks::enchantments::EnchantmentsGameExt1;
use sunrise_items::items::ItemStack;

const SHARPNESS: &str = "minecraft:sharpness";
const FIXTURE_ITEMS: [&str; 4] = [
    "minecraft:stick",
    "minecraft:diamond_sword",
    "minecraft:stone",
    "minecraft:enchanted_book",
];

/// What the parts answer, and what they were asked.
#[derive(Clone, Default)]
struct Answers {
    support: Arc<Mutex<Vec<(String, bool)>>>,
    support_asked: Arc<Mutex<usize>>,
    tag_answers: Option<Arc<Mutex<Vec<Option<bool>>>>>,
    tag_answer_enabled: Option<Arc<AtomicBool>>,
    primary_incoming: Arc<Mutex<Vec<(String, bool)>>>,
    /// `is_primary_item_for`: the item and the answer for sharpness.
    primary: Arc<Mutex<Vec<(String, bool)>>>,
    /// `is_primary_item_for`: how many times a part was asked.
    primary_asked: Arc<Mutex<usize>>,
    /// `apply_enchantments`: the damage and the count the part sets on the result.
    applied: Arc<Mutex<Option<(u32, u32)>>>,
    /// `apply_enchantments`: the list it was given and the result as it arrived.
    applied_asked: Arc<Mutex<Vec<(Vec<(String, u32)>, mod_api::ItemStack)>>>,
    /// `is_not_replaceable_by_pick_action`: the item and the answer.
    keeps: Arc<Mutex<Vec<(String, bool)>>>,
    /// `is_not_replaceable_by_pick_action`: the slot and the answer as it arrived.
    keeps_asked: Arc<Mutex<Vec<(u32, bool)>>>,
}

#[derive(Clone)]
struct Part {
    item: &'static str,
    answers: Answers,
}

#[mod_api::sunrise_part]
impl ItemPart for Part {
    fn supports_enchantment(
        &self,
        host: &dyn mod_api::Host,
        event: &mut SupportsEnchantmentEvent<'_>,
    ) -> Option<bool> {
        *self.answers.support_asked.lock().unwrap() += 1;
        if event.enchantment != SHARPNESS {
            return None;
        }
        if let Some(seen) = &self.answers.tag_answers {
            let sword = host
                .item_key(&mod_api::Identifier::parse("minecraft:diamond_sword").unwrap())
                .unwrap();
            let tags = host.tags();
            let swords = tags
                .key(&mod_api::Identifier::parse("minecraft:swords").unwrap())
                .unwrap()
                .typed();
            let answer = tags.is_in_tag(sword, swords);
            seen.lock().unwrap().push(answer);
            let enabled = self
                .answers
                .tag_answer_enabled
                .as_ref()
                .is_none_or(|enabled| enabled.load(Ordering::Relaxed));
            return answer.map(|contains_sword| contains_sword && enabled);
        }
        let answers = self.answers.support.lock().unwrap();
        answers
            .iter()
            .find(|(item, _)| item == self.item)
            .map(|(_, answer)| *answer)
    }
    fn is_primary_item_for(&self, _: &dyn mod_api::Host, event: &mut IsPrimaryItemForEvent<'_>) {
        *self.answers.primary_asked.lock().unwrap() += 1;
        if event.enchantment != SHARPNESS {
            return;
        }
        self.answers
            .primary_incoming
            .lock()
            .unwrap()
            .push((self.item.into(), event.primary));
        let answers = self.answers.primary.lock().unwrap();
        if let Some((_, answer)) = answers.iter().find(|(item, _)| item == self.item) {
            event.primary = *answer;
        }
    }
    fn apply_enchantments(&self, _: &dyn mod_api::Host, event: &mut ApplyEnchantmentsEvent<'_>) {
        let list = event
            .enchantments
            .iter()
            .map(|(id, level)| (id.to_string(), *level))
            .collect();
        self.answers
            .applied_asked
            .lock()
            .unwrap()
            .push((list, event.result.clone()));
        if let Some((damage, count)) = *self.answers.applied.lock().unwrap() {
            event.result.set_damage(damage);
            let _ = event.result.set_count(count);
        }
    }
    fn is_not_replaceable_by_pick_action(
        &self,
        _: &dyn mod_api::Host,
        event: &mut IsNotReplaceableByPickActionEvent<'_>,
    ) {
        self.answers
            .keeps_asked
            .lock()
            .unwrap()
            .push((event.slot, event.not_replaceable));
        let answers = self.answers.keeps.lock().unwrap();
        if let Some((_, answer)) = answers.iter().find(|(item, _)| item == self.item) {
            event.not_replaceable = *answer;
        }
    }
}

struct Mod(Vec<Part>);

#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for part in &self.0 {
            let key = r.item_key(part.item);
            r.item_part(key, part.clone());
        }
    }
    fn tags_updated(&self, _: &dyn mod_api::ReloadContext, event: &mut mod_api::TagsUpdatedEvent) {
        if matches!(
            event.cause,
            mod_api::TagsUpdateCause::ClientPacketReceived { .. }
        ) {
            for enabled in self
                .0
                .iter()
                .filter_map(|part| part.answers.tag_answer_enabled.as_ref())
            {
                enabled.fetch_xor(true, Ordering::Relaxed);
            }
        }
    }
}

/// Runs `test` on an integrated-server game after the fixture parts resolve.
fn with_parts(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Answers)) {
    let answers = Answers::default();
    let parts = FIXTURE_ITEMS
        .map(|item| Part {
            item,
            answers: answers.clone(),
        })
        .to_vec();
    let mods: mod_api::Mods = vec![ModEntry::new("enchant_pick", Box::new(Mod(parts)))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            // with_game already resolves the installed parts when opening its fixture.
            test(game, &answers);
            // The resources may be shared with the next test's game: retire the answers.
            game.clear_world_parts();
        });
    });
}

/// `available_results` at value 30 over `[sharpness]` for `item`.
fn sharpness_at_30(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    item: &str,
) -> Vec<(String, u32)> {
    game.stations()
        .enchantments
        .available_results(game.items(), 30, item, &[SHARPNESS.into()])
        .into_iter()
        .map(|(id, level)| (id.to_string(), level))
        .collect()
}

#[test]
#[ignore = "daily"]
fn primary_answers_are_asked_at_install_and_read_by_selection() {
    with_parts(|game, answers| {
        let registered = game.stations().enchantments.list.len();
        assert!(registered > 0);
        // No answer: vanilla. A stick has no sharpness; a diamond sword gets level 3, the
        // highest whose window (23 to 43) holds 30.
        let level_3 = vec![(SHARPNESS.to_owned(), 3)];
        assert!(sharpness_at_30(game, "minecraft:stick").is_empty());
        assert_eq!(sharpness_at_30(game, "minecraft:diamond_sword"), level_3);
        // Asked once per registered enchantment and fixture item, at install.
        let per_install = FIXTURE_ITEMS.len() * registered;
        assert_eq!(*answers.primary_asked.lock().unwrap(), per_install);
        *answers.primary.lock().unwrap() = vec![
            ("minecraft:stick".into(), true),
            ("minecraft:diamond_sword".into(), false),
        ];
        // Read from the table: no new answer until the parts resolve again.
        assert!(sharpness_at_30(game, "minecraft:stick").is_empty());
        game.resolve_parts().unwrap();
        assert_eq!(*answers.primary_asked.lock().unwrap(), 2 * per_install);
        assert_eq!(sharpness_at_30(game, "minecraft:stick"), level_3);
        assert!(sharpness_at_30(game, "minecraft:diamond_sword").is_empty());
        // Selection asks no part: the count stays.
        for _ in 0..5 {
            sharpness_at_30(game, "minecraft:stick");
        }
        assert_eq!(*answers.primary_asked.lock().unwrap(), 2 * per_install);
        // An unsubscribed plain book keeps its default eligibility.
        assert_eq!(sharpness_at_30(game, "minecraft:book"), level_3);
        // The answers are this server's snapshot's: its stations and its loot items read them.
        let items = &game.items().enchantments;
        assert_eq!(
            items.primary_answers().answer("minecraft:stick", SHARPNESS),
            Some(true)
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_client_refresh_keeps_and_a_world_close_retires_the_servers_answers() {
    with_parts(|game, answers| {
        *answers.primary.lock().unwrap() = vec![("minecraft:stick".into(), true)];
        game.resolve_parts().unwrap();
        let level_3 = vec![(SHARPNESS.to_owned(), 3)];
        assert_eq!(sharpness_at_30(game, "minecraft:stick"), level_3);
        let asked = *answers.primary_asked.lock().unwrap();
        // A client sharing this Resources snapshot neither asks nor clears the server's answers.
        #[cfg(feature = "client")]
        {
            use {
                crate::game::{Audio, Settings, load_resources},
                sunrise_client_gameplay::game::ClientGame,
                sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
                sunrise_gameplay_common::game::Options,
                sunrise_gameplay_common::game::Progress,
                sunrise_gameplay_common::game::execution_role::ExecutionRole,
            };
            let root = fixture_root();
            let saves = tempfile::tempdir().unwrap();
            let settings = Settings::default();
            let loaded = load_resources(
                sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
                Vec::new(),
                None,
                &root,
                &settings,
                std::time::Instant::now(),
                false,
                ExecutionRole::Client,
                &Progress::new(),
                &mut |_, _| unreachable!(),
            )
            .unwrap();
            let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
            options.resources = root.clone();
            options.execution_role = ExecutionRole::Client;
            options.game_dir = Some(saves.path().to_path_buf());
            let client = ClientGame::new(
                &game.res,
                loaded.client_res.unwrap(),
                loaded.pack,
                loaded.client_pack.unwrap(),
                loaded.gui.unwrap(),
                options,
                settings,
                Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                    root,
                )),
                None,
                sunrise_gameplay::game::server_side::integrated::serve,
                sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
                crate::game::session_verifier(),
                crate::game::profile_resolver,
            )
            .unwrap();
            *answers.primary.lock().unwrap() = Vec::new();
            client.resolve_primary_items();
        }
        assert_eq!(*answers.primary_asked.lock().unwrap(), asked);
        assert_eq!(sharpness_at_30(game, "minecraft:stick"), level_3);
        // The server's world closes with its parts: the answers are retired.
        game.clear_world_parts();
        assert!(sharpness_at_30(game, "minecraft:stick").is_empty());
        assert!(game.stations().enchantments.primary_answers().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn apply_enchantments_starts_from_vanilla_and_refuses_another_count() {
    with_parts(|game, answers| {
        let sword = ItemStack::new("minecraft:diamond_sword", 1);
        let vanilla = sunrise_sim::stations::apply_enchantment(
            &game.stations().enchantments,
            game.items(),
            7,
            2,
            30,
            &sword,
        )
        .unwrap();
        let list = sunrise_sim::stations::enchantment_list(
            &game.stations().enchantments,
            game.items(),
            7,
            2,
            30,
            &sword,
        );
        // No answer: vanilla's result, and the part got the list and that result.
        assert_eq!(
            game.table_enchant(7, 2, 30, &sword).as_ref(),
            Some(&vanilla)
        );
        let asked = std::mem::take(&mut *answers.applied_asked.lock().unwrap());
        assert_eq!(asked.len(), 1);
        let want: Vec<_> = list.iter().map(|(id, l)| (id.to_string(), *l)).collect();
        assert_eq!(asked[0].0, want);
        assert_eq!(asked[0].1.count(), 1);
        assert_eq!(asked[0].1.damage(), 0);
        // A changed result is the table's.
        *answers.applied.lock().unwrap() = Some((7, 1));
        let mut changed = vanilla.clone();
        changed.damage = 7;
        assert_eq!(game.table_enchant(7, 2, 30, &sword), Some(changed));
        // A count other than the input's is refused: vanilla's result.
        *answers.applied.lock().unwrap() = Some((7, 2));
        assert_eq!(game.table_enchant(7, 2, 30, &sword), Some(vanilla));
        // A stick isn't enchantable: no result, and its part isn't asked.
        answers.applied_asked.lock().unwrap().clear();
        let stick = ItemStack::new("minecraft:stick", 1);
        assert_eq!(game.table_enchant(7, 2, 30, &stick), None);
        assert!(answers.applied_asked.lock().unwrap().is_empty());
    });
}

/// Enchanted diamond sword, stone, then dirt × 7 in the hotbar, oak planks in main slot 20,
/// slot 0 selected: picks the planks, and gives the selected slot and what went to slot 20.
fn pick_planks(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> (usize, String) {
    game.inventory = vec![None; game.inventory.len()];
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    sword.enchant(SHARPNESS, 1);
    game.inventory[0] = Some(sword);
    game.inventory[1] = Some(ItemStack::new("minecraft:stone", 5));
    for slot in 2..9 {
        game.inventory[slot] = Some(ItemStack::new("minecraft:dirt", 5));
    }
    game.inventory[20] = Some(ItemStack::new("minecraft:oak_planks", 5));
    game.selected = 0;
    assert!(game.pick_into_inventory(&ItemStack::new("minecraft:oak_planks", 1), false));
    let moved = game.inventory[20].as_ref().unwrap().item.to_string();
    (game.selected, moved)
}

#[test]
#[ignore = "daily"]
fn pick_asks_each_hotbar_stack_until_one_may_be_replaced() {
    with_parts(|game, answers| {
        // Vanilla: the sword stays (enchanted), the stone goes.
        assert_eq!(pick_planks(game), (1, "minecraft:stone".into()));
        // The sword's part and the stone's, with vanilla's answers.
        assert_eq!(
            std::mem::take(&mut *answers.keeps_asked.lock().unwrap()),
            [(0, true), (1, false)]
        );
        *answers.keeps.lock().unwrap() = vec![("minecraft:stone".into(), true)];
        assert_eq!(pick_planks(game), (2, "minecraft:dirt".into()));
        *answers.keeps.lock().unwrap() = vec![("minecraft:diamond_sword".into(), false)];
        assert_eq!(pick_planks(game), (0, "minecraft:diamond_sword".into()));
        // A hotbar with an empty slot asks nobody.
        answers.keeps_asked.lock().unwrap().clear();
        game.inventory = vec![None; game.inventory.len()];
        game.inventory[0] = Some(ItemStack::new("minecraft:stone", 5));
        game.inventory[20] = Some(ItemStack::new("minecraft:oak_planks", 5));
        game.selected = 0;
        assert!(game.pick_into_inventory(&ItemStack::new("minecraft:oak_planks", 1), false));
        assert_eq!(game.selected, 1);
        assert!(answers.keeps_asked.lock().unwrap().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn support_snapshot_reaches_both_anvils_and_enchant_without_dispatching_inside_them() {
    use sunrise_gameplay_common::game::execution_role::ExecutionRole;
    use sunrise_sim::menu::{Kind as MenuKind, Menu};
    use {
        sunrise_gameplay_common::game::stations_common::Kind as StationKind,
        sunrise_gameplay_common::game::stations_common::Station,
    };
    with_parts(|game, answers| {
        let registered = game.stations().enchantments.list.len();
        let per_install = FIXTURE_ITEMS.len() * registered;
        assert_eq!(*answers.support_asked.lock().unwrap(), per_install);
        *answers.support.lock().unwrap() = vec![
            ("minecraft:stick".into(), true),
            ("minecraft:diamond_sword".into(), false),
        ];
        answers.primary_incoming.lock().unwrap().clear();
        game.resolve_parts().unwrap();
        assert!(
            answers
                .primary_incoming
                .lock()
                .unwrap()
                .contains(&("minecraft:diamond_sword".into(), false)),
            "primary listeners start from the support-layered default"
        );
        assert!(sharpness_at_30(game, "minecraft:diamond_sword").is_empty());
        game.creative = false;
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(game.motion, sunrise_core::geometry::Vec3::ZERO)
                .unwrap(),
        );
        game.build_registries();
        let sharpness = [(Arc::<str>::from(SHARPNESS), 1)];
        let check_anvils =
            |game: &mut sunrise_gameplay::game::ServerGame<'_>, item: &str, allowed: bool| {
                let mut book = ItemStack::new("minecraft:enchanted_book", 1);
                book.set_crafting_enchantments(sharpness.to_vec());
                let input = ItemStack::new(item, 1);
                let mut menu = Menu::new(MenuKind::Anvil);
                let env = game.menu_env();
                menu.set_slot(&env, 0, Some(input.clone()));
                menu.set_slot(&env, 1, Some(book.clone()));
                menu.settle(&env);
                assert_eq!(menu.slot(2).is_some(), allowed, "client-lib anvil: {item}");
                if allowed {
                    assert_eq!(menu.slot(2).unwrap().crafting_enchantments(), sharpness);
                }
                let mut station = Station::new(StationKind::Anvil, [0, 64, 0]);
                station.inputs[0] = Some(input);
                station.inputs[1] = Some(book);
                game.station = Some(station);
                game.station_changed();
                let result = &game.station.as_ref().unwrap().result;
                assert_eq!(result.is_some(), allowed, "game anvil: {item}");
                if allowed {
                    assert_eq!(result.as_ref().unwrap().crafting_enchantments(), sharpness);
                }
            };
        for role in [ExecutionRole::IntegratedServer, ExecutionRole::Client] {
            game.options.execution_role = role;
            if role == ExecutionRole::Client {
                let asked = *answers.support_asked.lock().unwrap();
                game.resolve_primary_items();
                assert_eq!(
                    *answers.support_asked.lock().unwrap(),
                    asked + per_install,
                    "the client installs its support snapshot too"
                );
            }
            let asked = *answers.support_asked.lock().unwrap();
            if role == ExecutionRole::Client {
                // The client Host reads the supported_items set, not the answers
                // installed for the anvil below or the enchanted-book default.
                for (id, supported) in [
                    ("minecraft:stick", false),
                    ("minecraft:diamond_sword", true),
                    ("minecraft:enchanted_book", false),
                ] {
                    let key = mod_api::__private::item_key(id).unwrap();
                    assert_eq!(game.enchantment_supports(SHARPNESS, key), supported, "{id}");
                }
            }
            check_anvils(game, "minecraft:stick", true);
            check_anvils(game, "minecraft:diamond_sword", false);
            check_anvils(game, "minecraft:enchanted_book", true);
            if role == ExecutionRole::IntegratedServer {
                game.selected = 0;
                for (item, allowed) in [
                    ("minecraft:stick", true),
                    ("minecraft:diamond_sword", false),
                    ("minecraft:enchanted_book", false),
                ] {
                    game.inventory[0] = Some(ItemStack::new(item, 1));
                    assert_eq!(
                        game.as_console(|g| g.run_command("enchant @s minecraft:sharpness 1")),
                        allowed,
                        "/enchant: {item}"
                    );
                    let expected = if allowed { sharpness.as_slice() } else { &[] };
                    assert_eq!(
                        game.inventory[0].as_ref().unwrap().crafting_enchantments(),
                        expected,
                        "{item}"
                    );
                }
            }
            assert_eq!(
                *answers.support_asked.lock().unwrap(),
                asked,
                "anvil and /enchant read the snapshot, not listeners"
            );
        }
        game.options.execution_role = ExecutionRole::IntegratedServer;
        answers
            .support
            .lock()
            .unwrap()
            .push(("minecraft:enchanted_book".into(), false));
        game.resolve_parts().unwrap();
        check_anvils(game, "minecraft:enchanted_book", false);
        answers.support.lock().unwrap().last_mut().unwrap().1 = true;
        game.resolve_parts().unwrap();
        game.inventory[0] = Some(ItemStack::new("minecraft:enchanted_book", 1));
        assert!(game.as_console(|g| g.run_command("enchant @s minecraft:sharpness 1")));
        assert_eq!(
            game.inventory[0].as_ref().unwrap().stored_enchantments(),
            sharpness
        );
        answers.support.lock().unwrap().clear();
        assert!(
            game.stations()
                .enchantments
                .can_enchant(game.items(), SHARPNESS, "minecraft:stick"),
            "changing a listener does not change an installed answer"
        );
        game.resolve_primary_items();
        assert!(!game.stations().enchantments.can_enchant(
            game.items(),
            SHARPNESS,
            "minecraft:stick"
        ));
    });
}

#[test]
#[ignore = "daily"]
fn short_enchantment_item_holders_bind_for_both_selectors() {
    with_parts(|game, answers| {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("datapacks/enchant_test");
        let data = pack.join("data");
        std::fs::create_dir_all(data.join("minecraft/enchantment")).unwrap();
        std::fs::create_dir_all(data.join("test/loot_table")).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"Enchantment holder binding"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.path().join("datapacks.json"),
            r#"{"enabled":["file/enchant_test"],"disabled":[]}"#,
        )
        .unwrap();
        std::fs::write(
            data.join("test/loot_table/sword.json"),
            r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond_sword","modifier":[{"type":"minecraft:enchant_with_levels","levels":30,"options":["minecraft:sharpness"]}]}]}]}"#,
        )
        .unwrap();
        let resources = game.options.resources.clone();
        let mut sharpness: serde_json::Value = serde_json::from_slice(
            &std::fs::read(resources.join("data/minecraft/enchantment/sharpness.json")).unwrap(),
        )
        .unwrap();
        sharpness["supported_items"] = serde_json::json!("diamond_sword");
        sharpness["primary_items"] = serde_json::json!(["diamond_sword"]);
        std::fs::write(
            data.join("minecraft/enchantment/sharpness.json"),
            serde_json::to_vec(&sharpness).unwrap(),
        )
        .unwrap();
        game.server.world_root = root.path().to_owned();
        let packs = game.reload_datapacks(false, &[]).unwrap();
        game.server.world_tables.0 = game
            .stage_world_tables(&packs, false, &resources)
            .unwrap()
            .tables;
        game.load_server_loot();
        let roll = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            let loot = game.server.server_loot.0.get().unwrap();
            let stacks = loot.roll("test:sword", &mut LegacyRandom::new(1), 0.0, None, None);
            assert_eq!(stacks[0].item, "minecraft:diamond_sword");
            stacks[0].enchantments.clone()
        };
        let sword = "minecraft:diamond_sword";
        for support in [None, Some(true), Some(false)] {
            *answers.support.lock().unwrap() =
                support.into_iter().map(|v| (sword.into(), v)).collect();
            answers.primary_incoming.lock().unwrap().clear();
            game.resolve_parts().unwrap();
            let eligible = support.unwrap_or(true);
            assert!(
                answers
                    .primary_incoming
                    .lock()
                    .unwrap()
                    .contains(&(sword.into(), eligible))
            );
            assert_eq!(!sharpness_at_30(game, sword).is_empty(), eligible);
            let rolled = roll(game);
            assert_eq!(
                rolled.first().map(|(id, _)| id.as_str()),
                eligible.then_some(SHARPNESS)
            );
        }
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn support_resolves_once_against_the_tags_published_by_world_open_and_play_packets() {
    use sunrise_protocol::packets::{
        configuration::{RegistryTags, TagEntry, UpdateTags},
        play::{self, Clientbound as C},
    };
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
    };

    let seen = Arc::new(Mutex::new(Vec::new()));
    let enabled = Arc::new(AtomicBool::new(true));
    let mods = vec![ModEntry::new(
        "enchant_tag_probe",
        Box::new(Mod(vec![Part {
            item: "minecraft:stick",
            answers: Answers {
                tag_answers: Some(seen.clone()),
                tag_answer_enabled: Some(enabled.clone()),
                ..Answers::default()
            },
        }])),
    )]
    .into();
    fn supports<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(game: &G) -> bool {
        game.item_enchantments()
            .can_enchant(game.items(), SHARPNESS, "minecraft:stick")
    }
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let scratch = tempfile::tempdir().unwrap();
            let registry = Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla());
            let create = sunrise_sim::world_options::WorldOptions::default();
            let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
                "enchant_tags",
                Some("enchant_tags"),
                Some(1),
                true,
                Some(&create),
                registry,
            )
            .unwrap();
            request.save_root = scratch.path().to_owned();
            seen.lock().unwrap().clear();
            game.load_world(request).unwrap();
            assert_eq!(*seen.lock().unwrap(), [Some(true)]);
            assert!(supports(game));
            game.clear_world_parts();
        });
        enabled.store(false, Ordering::Relaxed);
        with_client_game(|game| {
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "EnchantTags").unwrap();
            let sword = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:item")
                .unwrap()
                .id("minecraft:diamond_sword")
                .unwrap();
            let tags = |ids| UpdateTags {
                registries: vec![RegistryTags {
                    registry: "minecraft:item".into(),
                    tags: vec![TagEntry {
                        name: "minecraft:swords".into(),
                        ids,
                    }],
                }],
            };
            let registries = &mut game.client.remote.as_mut().unwrap().registries;
            Arc::make_mut(registries).add_tags(&tags(vec![sword]));
            seen.lock().unwrap().clear();
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
            assert_eq!(*seen.lock().unwrap(), [Some(true)]);
            assert!(supports(game));
            game.handle_packet(C::UpdateTags(tags(vec![sword])))
                .unwrap();
            assert_eq!(*seen.lock().unwrap(), [Some(true), Some(true)]);
            assert!(
                !supports(game),
                "the notice changed the part's answer without changing the tag"
            );
            game.handle_packet(C::UpdateTags(tags(Vec::new()))).unwrap();
            assert_eq!(*seen.lock().unwrap(), [Some(true), Some(true), Some(false)]);
            assert!(!supports(game));
            game.clear_world_parts();
        });
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::stations_common::StationsCommonGameExt1 as _;
