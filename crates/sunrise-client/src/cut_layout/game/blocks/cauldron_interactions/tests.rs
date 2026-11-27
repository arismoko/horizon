use mod_api::__private::game::RegistrationContextNew;
use mod_api::{
    BlockUpdateFlags, CauldronDecision, CauldronUse, Hooks, ItemPlace, ItemResource, ServerHost,
    StatType,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use sunrise_gameplay_common::game::cauldron_interactions::{Arc, Items, Registrar, snapshot};
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::cauldron_interactions::{clear, install};
use sunrise_gameplay_common::hit::Hit;
use sunrise_items::items::ItemStack;
use sunrise_sim::cauldron::{Outcome, interactions::Entry};
use sunrise_sim::first_person_sim::Hand;
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
};

pub(crate) const POS: [i32; 3] = [0, 64, 0];
#[derive(Default)]
pub(crate) struct Seen {
    pub(in crate::game) fail_preparation: AtomicBool,
    pub(in crate::game) calls: AtomicUsize,
    pub(in crate::game) actor: AtomicUsize,
    pub(in crate::game) drops: AtomicUsize,
}
pub(crate) struct Syrup {
    pub seen: Arc<Seen>,
}
impl mod_api::Interaction for Syrup {
    fn interact(&self, host: &mut dyn ServerHost, input: &CauldronUse) -> CauldronDecision {
        use mod_api::HostTransaction as _;
        self.seen.calls.fetch_add(1, Ordering::SeqCst);
        self.seen
            .actor
            .store(input.player.entity().id() as usize, Ordering::SeqCst);
        if input.player.entity().id() == 7 {
            assert_eq!(host.entity(7), Some(input.player.entity()));
        }
        let block = host.block_state_id("test:syrup_cauldron[level=1]").unwrap();
        let bottle = mod_api::ItemStack::new(
            mod_api::__private::item_key("minecraft:glass_bottle").unwrap(),
            1,
        )
        .unwrap();
        let hand = ItemPlace::PlayerHand {
            player: input.player.uuid(),
            hand: input.hand,
        };
        let resource = ItemResource::of(&bottle);
        host.transaction(|host, tx| {
            input
                .level
                .set_block(
                    host,
                    input.pos,
                    block,
                    BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
                )
                .map_err(|_| ())?;
            if hand.exchange(host, tx, &resource, 1) != Ok(1) {
                return Err(());
            }
            let fluid = mod_api::__private::fluid_key("minecraft:water").unwrap();
            assert_eq!(
                input.level.play_fluid_effects(
                    host,
                    input.pos,
                    fluid,
                    false,
                    Some(input.player),
                    tx,
                ),
                Ok(true)
            );
            Ok(())
        })
        .unwrap();
        input
            .player
            .award_stat(host, StatType::Custom, "minecraft:use_cauldron", 1);
        input
            .player
            .award_stat(host, StatType::Used, "test:syrup_bottle", 1);
        CauldronDecision::Success
    }
}
pub(crate) struct TestMod {
    pub seen: Arc<Seen>,
    pub(crate) dye: bool,
}
#[mod_api::__subscriptions]
impl Hooks for TestMod {
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new(
                "test:syrup_cauldron",
                "minecraft:water_cauldron",
            ))
            .unwrap();
    }
    fn items(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::ItemsEvent<'_>) {
        event
            .register(mod_api::Item::new(
                "test:syrup_bottle",
                "minecraft:glass_bottle",
            ))
            .unwrap();
    }
    fn register_cauldron_interaction(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterCauldronInteractionEvent<'_>,
    ) {
        let handler: Arc<dyn mod_api::Interaction> = Arc::new(Syrup {
            seen: self.seen.clone(),
        });
        let item = mod_api::__private::item_key(if self.dye {
            "minecraft:leather_helmet"
        } else {
            "test:syrup_bottle"
        })
        .unwrap();
        let block = mod_api::__private::block_key(if self.dye {
            "minecraft:water_cauldron"
        } else {
            "minecraft:cauldron"
        })
        .unwrap();
        event
            .interactions
            .register(block, item, handler.clone())
            .unwrap();
        if self.seen.fail_preparation.load(Ordering::SeqCst) {
            let _ = event
                .interactions
                .register_tag_to_all("invalid tag", handler.clone());
        }
        event
            .interactions
            .register(
                mod_api::__private::block_key("test:syrup_cauldron").unwrap(),
                mod_api::__private::item_key("test:syrup_bottle").unwrap(),
                handler,
            )
            .unwrap();
    }
    fn item_spawn(&self, _: &mut dyn ServerHost, _: &mut mod_api::ItemSpawnEvent<'_>) {
        self.seen.drops.fetch_add(1, Ordering::SeqCst);
    }
}
#[test]
fn a_panicking_cauldron_registrar_leaves_no_exact_tag_or_all_entries() {
    use mod_api::{ModEntry, RegistrationContext};
    use sunrise_mod_runtime::block_definitions::BlockDefinition;
    use sunrise_world::{loot::PackOverlay, registry::BlockRegistry};

    struct Registrant(Option<Arc<AtomicBool>>);
    #[mod_api::__subscriptions]
    impl Hooks for Registrant {
        fn register_cauldron_interaction(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::RegisterCauldronInteractionEvent<'_>,
        ) {
            let block = mod_api::__private::block_key("minecraft:water_cauldron").unwrap();
            let item = |id| mod_api::__private::item_key(id).unwrap();
            let handler: Arc<dyn mod_api::Interaction> = Arc::new(Syrup {
                seen: Arc::new(Seen::default()),
            });
            let interactions = &mut *event.interactions;
            let Some(added) = &self.0 else {
                interactions
                    .register(block, item("test:winner"), handler)
                    .unwrap();
                return;
            };
            interactions
                .register(block, item("test:exact"), handler.clone())
                .unwrap();
            interactions
                .register_tag(block, "test:tag", handler.clone())
                .unwrap();
            interactions
                .register_to_all(item("test:all"), handler.clone())
                .unwrap();
            interactions
                .register_tag_to_all("test:all_tag", handler.clone())
                .unwrap();
            // Overwriting a built-in key or an earlier tag in the same batch succeeds in place.
            assert_eq!(
                interactions.register(block, item("minecraft:water_bucket"), handler.clone()),
                Ok(()),
            );
            assert_eq!(
                interactions.register_tag(block, "#test:tag", handler),
                Ok(())
            );
            added.store(true, Ordering::Relaxed);
            panic!("after registering exact, tag and all-dispatcher entries");
        }
    }

    let added = Arc::new(AtomicBool::new(false));
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("failed", Box::new(Registrant(Some(added.clone())))),
            ModEntry::new("healthy", Box::new(Registrant(None))),
        ]
        .into(),
        || {
            sunrise_gameplay_common::game::api_values::install_parts(&[]).unwrap();
            mod_api::__private::resolve_block_keys(["minecraft:water_cauldron"], |_| {
                BlockDefinition::new(&[&Default::default()], 0).map_err(|err| format!("{err:?}"))
            })
            .unwrap();
            let ids = [
                "test:exact",
                "test:all",
                "test:winner",
                "minecraft:water_bucket",
            ];
            mod_api::__private::resolve_items(ids).unwrap();
            let root = tempfile::tempdir().unwrap();
            let item_json = ids
                .map(|id| (id, serde_json::json!({})))
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>();
            std::fs::write(
                root.path().join("items.json"),
                serde_json::to_vec(&item_json).unwrap(),
            )
            .unwrap();
            let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
            let items = Items::catalog(root.path(), &PackOverlay::NONE, &defaults).unwrap();
            let registry = BlockRegistry::from_facts_json(
                &br#"{
                "defaults":{"minecraft:water_cauldron":0},
                "states":[["water_cauldron",false,false,"","minecraft:water_cauldron"]]
            }"#[..],
            )
            .unwrap();
            let mut registrar = Registrar::new(&items, &registry);
            sunrise_gameplay_common::game::native_hooks::native_register_cauldron_interaction(
                &RegistrationContext::new(),
                &mut registrar,
            );
            assert!(
                added.load(Ordering::Relaxed),
                "the failed listener reached the intended panic"
            );
            let table = registrar.finish().unwrap();
            let water = "minecraft:water_cauldron";
            assert!(
                table
                    .select(water, "test:exact", false, |_| false)
                    .is_none()
            );
            assert!(
                table
                    .select(water, "test:other", false, |tag| tag == "test:tag")
                    .is_none()
            );
            for block in ["minecraft:cauldron", water] {
                assert!(table.select(block, "test:all", false, |_| false).is_none());
                assert!(
                    table
                        .select(block, "test:other", false, |tag| tag == "test:all_tag")
                        .is_none()
                );
            }
            assert!(
                matches!(table.select(water, "test:winner", false, |_| false), Some(Entry::Mod(found)) if found.owner == "healthy")
            );
            assert!(matches!(
                table.select(water, "minecraft:water_bucket", false, |_| false),
                Some(Entry::Vanilla(sunrise_sim::cauldron::Action::EmptyWater))
            ));
        },
    );
}

pub(crate) fn fixture_mods(
    mods: mod_api::Mods,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    with_game_at(root, test);
}
#[cfg(feature = "client")]
pub(crate) fn fixture_mods_client(
    mods: mod_api::Mods,
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    sunrise_client_gameplay::game::native_entity_refs::tests::with_game_role_client(
        root,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        test,
    );
}
#[cfg(feature = "client")]
pub(crate) fn fixture_mods_pair(
    mods: mod_api::Mods,
    test: impl FnOnce(
        &mut sunrise_gameplay::game::ServerGame<'_>,
        &mut sunrise_client_gameplay::game::ClientGame<'_>,
    ),
) {
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    with_game_at(root.clone(), |server| {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_game_role_client(
            root,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            |client| test(server, client),
        );
    });
}
pub(crate) fn fixture(
    dye: bool,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Arc<Seen>),
) {
    let seen = Arc::new(Seen::default());
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(TestMod {
            seen: seen.clone(),
            dye,
        }),
    )]
    .into();
    fixture_mods(mods, |game| {
        setup(game, "minecraft:cauldron", None, "test:syrup_bottle", 2);
        test(game, &seen);
    });
}
pub(crate) fn setup_common<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
    game: &mut G,
    block: &str,
    level: Option<&str>,
    item: &str,
    count: u32,
) {
    setup_common_with(game, block, level, item, count, |_| {});
}

fn setup_common_with<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
    game: &mut G,
    block: &str,
    level: Option<&str>,
    item: &str,
    count: u32,
    before_event_reset: impl FnOnce(&mut G),
) {
    game.world.chunks.insert(
        (0, 0),
        Arc::new({
            let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
            chunk.lit = true;
            chunk
        }),
    );
    let state = sunrise_world::chunk::BlockState {
        name: block.into(),
        properties: level
            .map(|v| ("level".into(), v.into()))
            .into_iter()
            .collect(),
    };
    let id = game.pack.assets.registry.resolve(&state).unwrap();
    game.world.set(0, 64, 0, id).unwrap();
    game.player = Some(
        sunrise_sim::motion::PlayerMotion::new(game.motion, sunrise_core::geometry::Vec3::ZERO)
            .unwrap(),
    );
    game.creative = false;
    game.selected = 0;
    game.hand = Hand::Main;
    game.inventory.fill(None);
    game.inventory[0] = Some(ItemStack::new(item, count));
    before_event_reset(game);
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|events| *events.borrow_mut() = Some(Vec::new()));
}
pub(crate) fn setup(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    block: &str,
    level: Option<&str>,
    item: &str,
    count: u32,
) {
    setup_common(game, block, level, item, count);
    bind_local_player(game, 7, 123);
    game.server.pending_native_sounds.clear();
    game.server.audio_capture_active = true;
    game.server.pending_sounds.clear();
}
pub(crate) fn state<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
    game: &G,
) -> (String, Option<String>) {
    let state = game
        .pack
        .assets
        .registry
        .state(game.world.block(0, 64, 0))
        .unwrap();
    (state.name.clone(), state.properties.get("level").cloned())
}
pub(crate) fn events() -> usize {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|events| events.borrow().as_ref().unwrap().len())
}
#[cfg(feature = "client")]
pub(crate) fn unchanged<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(game: &G, seen: &Seen) {
    assert_eq!(state(game), ("minecraft:cauldron".into(), None));
    let held = game.inventory[0].as_ref().unwrap();
    assert_eq!((&*held.item, held.count), ("test:syrup_bottle", 2));
    assert_eq!(seen.drops.load(Ordering::SeqCst), 0);
    assert_eq!(game.dropped.len(), 0);
    assert_eq!(
        game.inventory
            .iter()
            .flatten()
            .filter(|stack| &*stack.item == "minecraft:glass_bottle")
            .count(),
        0
    );
}
pub(crate) fn bottles(game: &sunrise_gameplay::game::ServerGame<'_>) -> u32 {
    game.inventory
        .iter()
        .flatten()
        .filter(|s| &*s.item == "minecraft:glass_bottle")
        .map(|s| s.count)
        .sum()
}
#[test]
#[ignore = "daily"]
fn cauldron_syrup_applies_once() {
    if run_fresh_cases(
        "cut_layout::game::cauldron_interactions::tests::cauldron_syrup_applies_once",
        "SUNRISE_CAULDRON_CASE",
        &["run"],
    ) {
        return;
    }
    fixture(false, |game, seen| {
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan(game);
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Success);
        assert_eq!(seen.calls.load(Ordering::SeqCst), 1);
        assert_eq!(seen.actor.load(Ordering::SeqCst), 7);
        assert_eq!(game.inventory[0].as_ref().unwrap().count, 1);
        assert_eq!(
            state(game),
            ("test:syrup_cauldron".into(), Some("1".into()))
        );
        assert_eq!(bottles(game), 1);
        assert_eq!(game.stats.get("custom", "minecraft:use_cauldron"), 1);
        assert_eq!(game.stats.get("used", "test:syrup_bottle"), 1);
        assert_eq!(
            game.server
                .pending_native_sounds
                .iter()
                .filter(|sound| sound.sound == "minecraft:item.bucket.empty")
                .count(),
            1,
        );
        assert_eq!(events(), 1);
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS.with(|events| {
            assert_eq!(
                events.borrow().as_ref().unwrap()[0].0,
                "minecraft:fluid_place"
            )
        });
        assert_eq!(seen.drops.load(Ordering::SeqCst), 0);

        setup(game, "minecraft:cauldron", None, "test:syrup_bottle", 2);
        game.creative = true;
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Success);
        assert_eq!(seen.calls.load(Ordering::SeqCst), 2);
        assert_eq!(game.inventory[0].as_ref().unwrap().count, 2);
        assert_eq!(
            state(game),
            ("test:syrup_cauldron".into(), Some("1".into()))
        );
        assert_eq!(bottles(game), 1);
        assert_eq!(game.stats.get("custom", "minecraft:use_cauldron"), 2);
        assert_eq!(game.stats.get("used", "test:syrup_bottle"), 2);
        assert_eq!(events(), 1);
        assert_eq!(seen.drops.load(Ordering::SeqCst), 0);

        let empty = game
            .pack
            .assets
            .registry
            .default_state("minecraft:cauldron")
            .unwrap();
        game.world.set(POS[0], POS[1], POS[2], empty).unwrap();
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Success);
        assert_eq!(game.inventory[0].as_ref().unwrap().count, 2);
        assert_eq!(bottles(game), 1);
    });
}
#[test]
#[ignore = "daily"]
fn cauldron_dye_tag_shadows_exact_handler_even_on_pass() {
    if run_fresh_cases(
        "cut_layout::game::cauldron_interactions::tests::cauldron_dye_tag_shadows_exact_handler_even_on_pass",
        "SUNRISE_CAULDRON_CASE",
        &["run"],
    ) {
        return;
    }
    fixture(true, |game, seen| {
        setup(
            game,
            "minecraft:water_cauldron",
            Some("2"),
            "minecraft:leather_helmet",
            1,
        );
        game.inventory[0].as_mut().unwrap().data_mut().dyed_color = Some(0x123456);
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Success);
        assert_eq!(
            state(game),
            ("minecraft:water_cauldron".into(), Some("1".into()))
        );
        assert_eq!(
            game.inventory[0]
                .as_ref()
                .unwrap()
                .data()
                .and_then(|d| d.dyed_color),
            None
        );
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Pass);
        assert_eq!(seen.calls.load(Ordering::SeqCst), 0);
    });
}
#[test]
#[ignore = "daily"]
fn cauldron_transfer_awards_two_statistics_once() {
    if run_fresh_cases(
        "cut_layout::game::cauldron_interactions::tests::cauldron_transfer_awards_two_statistics_once",
        "SUNRISE_CAULDRON_CASE",
        &["run"],
    ) {
        return;
    }
    fixture(false, |game, seen| {
        for slot in game.inventory.iter_mut().skip(1) {
            *slot = Some(ItemStack::new("minecraft:stone", 64));
        }
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Success);
        assert_eq!(game.inventory[0].as_ref().unwrap().count, 1);
        assert_eq!(
            state(game),
            ("test:syrup_cauldron".into(), Some("1".into()))
        );
        assert_eq!(seen.drops.load(Ordering::SeqCst), 1);
        assert_eq!(game.dropped.len(), 1);
        assert_eq!(
            (&*game.dropped[0].stack.item, game.dropped[0].stack.count),
            ("minecraft:glass_bottle", 1)
        );
        assert_eq!(game.stats.get("custom", "minecraft:use_cauldron"), 1);
        assert_eq!(game.stats.get("used", "test:syrup_bottle"), 1);
    });
}

#[test]
#[ignore = "daily"]
fn cauldron_register_to_all_and_admission_errors() {
    if run_fresh_cases(
        "cut_layout::game::cauldron_interactions::tests::cauldron_register_to_all_and_admission_errors",
        "SUNRISE_CAULDRON_CASE",
        &["run"],
    ) {
        return;
    }
    fixture(false, |game, seen| {
        use mod_api::CauldronInteractions as _;
        let handler: Arc<dyn mod_api::Interaction> = Arc::new(Syrup { seen: seen.clone() });
        let item = mod_api::__private::item_key("test:syrup_bottle").unwrap();
        let custom = mod_api::__private::block_key("test:syrup_cauldron").unwrap();
        let make = || Registrar::new(game.items(), game.pack.assets.registry);
        let _owner = mod_api::__private::enter("all_mod", "register_cauldron_interaction");
        let mut registrar = make();
        registrar.register_to_all(item, handler.clone()).unwrap();
        // The custom dispatcher is declared after register_to_all was queued.
        registrar
            .register(
                custom,
                mod_api::__private::item_key("minecraft:stick").unwrap(),
                handler.clone(),
            )
            .unwrap();
        let table = registrar.finish().unwrap();
        assert_eq!(table.blocks().len(), 5);
        for block in [
            "minecraft:cauldron",
            "minecraft:water_cauldron",
            "minecraft:lava_cauldron",
            "minecraft:powder_snow_cauldron",
            "test:syrup_cauldron",
        ] {
            assert!(
                matches!(table.select(block,"test:syrup_bottle",false,|_| false),Some(Entry::Mod(callback)) if callback.owner=="all_mod")
            );
        }
        let mut registrar = make();
        assert_eq!(
            registrar.register_to_all(
                mod_api::__private::item_key("minecraft:water_bucket").unwrap(),
                handler.clone()
            ),
            Ok(())
        );
        let table = registrar.finish().unwrap();
        for block in [
            "minecraft:cauldron",
            "minecraft:water_cauldron",
            "minecraft:lava_cauldron",
            "minecraft:powder_snow_cauldron",
        ] {
            assert!(
                matches!(
                    table.select(block, "minecraft:water_bucket", false, |_| false),
                    Some(Entry::Mod(callback)) if callback.owner == "all_mod"
                ),
                "{block}"
            );
        }
        let mut items = game.items().clone();
        items.items.remove("test:syrup_bottle");
        let mut registrar = Registrar::new(&items, game.pack.assets.registry);
        assert_eq!(
            registrar.register_to_all(item, handler.clone()),
            Err(mod_api::CauldronRegistrationError::UnknownItem)
        );
        assert!(
            registrar
                .finish()
                .err()
                .unwrap()
                .to_string()
                .contains("UnknownItem")
        );
        let mut registrar = Registrar::new(&items, game.pack.assets.registry);
        assert_eq!(
            registrar.register(custom, item, handler.clone()),
            Err(mod_api::CauldronRegistrationError::UnknownItem)
        );
        assert!(
            registrar
                .finish()
                .err()
                .unwrap()
                .to_string()
                .contains("UnknownItem")
        );
        let registry = sunrise_world::registry::BlockRegistry::from_facts_json(
            &br#"{"defaults":{"minecraft:air":0},"states":[["air",true,false,"","minecraft:air"]]}"#[..],
        )
        .unwrap();
        let mut registrar = Registrar::new(game.items(), &registry);
        assert_eq!(
            registrar.register(custom, item, handler),
            Err(mod_api::CauldronRegistrationError::UnknownBlock)
        );
        assert!(
            registrar
                .finish()
                .err()
                .unwrap()
                .to_string()
                .contains("UnknownBlock")
        );
    });
}

#[test]
#[ignore = "daily"]
fn cauldron_world_close_discards_callbacks_before_failed_preparation() {
    if run_fresh_cases(
        "cut_layout::game::cauldron_interactions::tests::cauldron_world_close_discards_callbacks_before_failed_preparation",
        "SUNRISE_CAULDRON_CASE",
        &["run"],
    ) {
        return;
    }
    fixture(false, |game, seen| {
        let old = snapshot();
        assert!(matches!(
            old.select("minecraft:cauldron", "test:syrup_bottle", false, |_| false),
            Some(Entry::Mod(_))
        ));
        seen.fail_preparation.store(true, Ordering::SeqCst);
        assert!(
            game.resolve_parts()
                .unwrap_err()
                .to_string()
                .contains("InvalidTag")
        );
        assert!(
            Arc::ptr_eq(&old, &snapshot()),
            "failed active reload retains its snapshot"
        );
        game.native_lifecycle_world_closed();
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        assert!(
            game.resolve_parts()
                .unwrap_err()
                .to_string()
                .contains("InvalidTag")
        );
        assert_eq!(snapshot().blocks().len(), 4);
        for block in [
            "minecraft:cauldron",
            "minecraft:water_cauldron",
            "minecraft:lava_cauldron",
            "minecraft:powder_snow_cauldron",
            "test:syrup_cauldron",
        ] {
            assert!(
                snapshot()
                    .select(block, "test:syrup_bottle", false, |_| false)
                    .is_none()
            );
        }
        game.inventory[0] = Some(ItemStack::new("minecraft:water_bucket", 1));
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Success);
        assert_eq!(
            state(game),
            ("minecraft:water_cauldron".into(), Some("3".into()))
        );
        assert_eq!(
            &*game.inventory[0].as_ref().unwrap().item,
            "minecraft:bucket"
        );
        assert_eq!(seen.calls.load(Ordering::SeqCst), 0);
    });
}

#[test]
#[ignore = "daily"]
fn cauldron_tag_registration_normalizes_admits_and_expands_over_the_complete_set() {
    if run_fresh_cases(
        "cut_layout::game::cauldron_interactions::tests::cauldron_tag_registration_normalizes_admits_and_expands_over_the_complete_set",
        "SUNRISE_CAULDRON_CASE",
        &["run"],
    ) {
        return;
    }
    fixture(false, |game, seen| {
        use mod_api::CauldronInteractions as _;
        use mod_api::CauldronRegistrationError as Error;
        let handler: Arc<dyn mod_api::Interaction> = Arc::new(Syrup { seen: seen.clone() });
        let custom = mod_api::__private::block_key("test:syrup_cauldron").unwrap();
        let make = || Registrar::new(game.items(), game.pack.assets.registry);
        let _owner = mod_api::__private::enter("tag_mod", "register_cauldron_interaction");
        // The leading `#` is dropped, and the custom dispatcher is declared after the
        // register_tag_to_all that expands over it.
        let mut registrar = make();
        registrar
            .register_tag_to_all("#c:buckets/milk", handler.clone())
            .unwrap();
        registrar
            .register_tag(custom, "c:foods", handler.clone())
            .unwrap();
        let table = registrar.finish().unwrap();
        assert_eq!(table.blocks().len(), 5);
        for block in [
            "minecraft:cauldron",
            "minecraft:water_cauldron",
            "minecraft:lava_cauldron",
            "minecraft:powder_snow_cauldron",
            "test:syrup_cauldron",
        ] {
            assert!(
                matches!(
                    table.select(block, "test:anything", false, |tag| tag == "c:buckets/milk"),
                    Some(Entry::Mod(callback)) if callback.owner == "tag_mod"
                ),
                "{block}"
            );
            assert!(
                table
                    .select(block, "test:anything", false, |_| false)
                    .is_none()
            );
        }
        assert!(matches!(
            table.select("test:syrup_cauldron", "test:anything", false, |tag| tag
                == "c:foods"),
            Some(Entry::Mod(_))
        ));
        assert!(
            table
                .select("minecraft:cauldron", "test:anything", false, |tag| tag
                    == "c:foods")
                .is_none()
        );
        // Admission: a string that is no resource location is refused, and fails preparation.
        let mut registrar = make();
        assert_eq!(
            registrar.register_tag(custom, "no tag here", handler.clone()),
            Err(Error::InvalidTag)
        );
        assert_eq!(
            registrar.register_tag_to_all("#no tag here", handler.clone()),
            Err(Error::InvalidTag)
        );
        assert!(
            registrar
                .finish()
                .err()
                .unwrap()
                .to_string()
                .contains("InvalidTag")
        );
        // Registering the same tag again (with or without `#`) overwrites in place.
        let mut registrar = make();
        registrar
            .register_tag(custom, "c:foods", handler.clone())
            .unwrap();
        {
            let _second = mod_api::__private::enter("second_mod", "register_cauldron_interaction");
            registrar
                .register_tag(custom, "#c:foods", handler.clone())
                .unwrap();
        }
        let table = registrar.finish().unwrap();
        assert!(matches!(
            table.select("test:syrup_cauldron", "test:anything", false, |tag| tag
                == "c:foods"),
            Some(Entry::Mod(callback)) if callback.owner == "second_mod"
        ));
        // Expansion via register_tag_to_all also overwrites an existing tag entry in place.
        let mut registrar = make();
        registrar
            .register_tag(custom, "c:foods", handler.clone())
            .unwrap();
        {
            let _second = mod_api::__private::enter("second_mod", "register_cauldron_interaction");
            registrar.register_tag_to_all("c:foods", handler).unwrap();
        }
        let table = registrar.finish().unwrap();
        assert!(matches!(
            table.select("test:syrup_cauldron", "test:anything", false, |tag| tag
                == "c:foods"),
            Some(Entry::Mod(callback)) if callback.owner == "second_mod"
        ));
    });
}

#[test]
#[ignore = "daily"]
fn cauldron_decision_controls_swing_and_fallback() {
    if run_fresh_cases(
        "cut_layout::game::cauldron_interactions::tests::cauldron_decision_controls_swing_and_fallback",
        "SUNRISE_CAULDRON_CASE",
        &["run"],
    ) {
        return;
    }
    struct DecisionHandler {
        decision: Arc<std::sync::Mutex<CauldronDecision>>,
        calls: Arc<AtomicUsize>,
    }
    impl mod_api::Interaction for DecisionHandler {
        fn interact(&self, _host: &mut dyn ServerHost, _input: &CauldronUse) -> CauldronDecision {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.decision.lock().unwrap()
        }
    }
    struct DecisionMod {
        decision: Arc<std::sync::Mutex<CauldronDecision>>,
        calls: Arc<AtomicUsize>,
        fallthrough: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for DecisionMod {
        fn register_cauldron_interaction(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterCauldronInteractionEvent<'_>,
        ) {
            event
                .interactions
                .register(
                    mod_api::__private::block_key("minecraft:cauldron").unwrap(),
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    Arc::new(DecisionHandler {
                        decision: self.decision.clone(),
                        calls: self.calls.clone(),
                    }),
                )
                .unwrap();
        }
        fn right_click_item(
            &self,
            _: mod_api::HostSide<'_>,
            _: &mut mod_api::RightClickItemEvent<'_>,
        ) -> mod_api::Flow {
            self.fallthrough.fetch_add(1, Ordering::SeqCst);
            mod_api::Flow::Continue
        }
    }

    let decision = Arc::new(std::sync::Mutex::new(CauldronDecision::Success));
    let calls = Arc::new(AtomicUsize::new(0));
    let fallthrough = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "decision_mod",
        Box::new(DecisionMod {
            decision: decision.clone(),
            calls: calls.clone(),
            fallthrough: fallthrough.clone(),
        }),
    )]
    .into();
    fixture_mods(mods, |game| {
        setup(game, "minecraft:cauldron", None, "minecraft:stick", 1);
        let hit = Hit {
            pos: POS,
            face: [0, 1, 0],
            point: [0.5, 65.0, 0.5],
            inside: false,
        };

        *decision.lock().unwrap() = CauldronDecision::Success;
        game.swing = Default::default();
        let before = game.swings_started;
        assert!(game.use_hand(Some(&hit)).unwrap());
        assert_eq!(game.swings_started, before + 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallthrough.load(Ordering::SeqCst), 0);

        *decision.lock().unwrap() = CauldronDecision::Consume;
        game.swing = Default::default();
        let before = game.swings_started;
        assert!(game.use_hand(Some(&hit)).unwrap());
        assert_eq!(game.swings_started, before);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(fallthrough.load(Ordering::SeqCst), 0);

        *decision.lock().unwrap() = CauldronDecision::Pass;
        game.swing = Default::default();
        let before = game.swings_started;
        assert!(!game.use_hand(Some(&hit)).unwrap());
        assert_eq!(game.swings_started, before);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(fallthrough.load(Ordering::SeqCst), 1);
    });
}

#[test]
#[ignore = "daily"]
fn cauldron_overwrites_vanilla_and_mod_entries_and_prefers_tags_over_items() {
    if run_fresh_cases(
        "cut_layout::game::cauldron_interactions::tests::cauldron_overwrites_vanilla_and_mod_entries_and_prefers_tags_over_items",
        "SUNRISE_CAULDRON_CASE",
        &["run"],
    ) {
        return;
    }
    struct RecordHandler {
        label: &'static str,
        decision: CauldronDecision,
        log: Arc<std::sync::Mutex<Vec<&'static str>>>,
    }
    impl mod_api::Interaction for RecordHandler {
        fn interact(&self, _host: &mut dyn ServerHost, _input: &CauldronUse) -> CauldronDecision {
            self.log.lock().unwrap().push(self.label);
            self.decision
        }
    }
    struct FirstMod {
        log: Arc<std::sync::Mutex<Vec<&'static str>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for FirstMod {
        fn register_cauldron_interaction(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterCauldronInteractionEvent<'_>,
        ) {
            let cauldron = mod_api::__private::block_key("minecraft:cauldron").unwrap();
            event
                .interactions
                .register(
                    cauldron,
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    Arc::new(RecordHandler {
                        label: "first_mod_stick",
                        decision: CauldronDecision::Success,
                        log: self.log.clone(),
                    }),
                )
                .unwrap();
        }
    }
    struct SecondMod {
        log: Arc<std::sync::Mutex<Vec<&'static str>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for SecondMod {
        fn register_cauldron_interaction(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterCauldronInteractionEvent<'_>,
        ) {
            let cauldron = mod_api::__private::block_key("minecraft:cauldron").unwrap();
            event
                .interactions
                .register(
                    cauldron,
                    mod_api::__private::item_key("minecraft:water_bucket").unwrap(),
                    Arc::new(RecordHandler {
                        label: "vanilla_override",
                        decision: CauldronDecision::Consume,
                        log: self.log.clone(),
                    }),
                )
                .unwrap();
            event
                .interactions
                .register(
                    cauldron,
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    Arc::new(RecordHandler {
                        label: "second_mod_stick",
                        decision: CauldronDecision::Success,
                        log: self.log.clone(),
                    }),
                )
                .unwrap();
            event
                .interactions
                .register_tag(
                    cauldron,
                    "minecraft:arrows",
                    Arc::new(RecordHandler {
                        label: "tag_arrows",
                        decision: CauldronDecision::Success,
                        log: self.log.clone(),
                    }),
                )
                .unwrap();
            event
                .interactions
                .register(
                    cauldron,
                    mod_api::__private::item_key("minecraft:arrow").unwrap(),
                    Arc::new(RecordHandler {
                        label: "exact_arrow",
                        decision: CauldronDecision::Consume,
                        log: self.log.clone(),
                    }),
                )
                .unwrap();
        }
    }

    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("first_mod", Box::new(FirstMod { log: log.clone() })),
        mod_api::ModEntry::new("second_mod", Box::new(SecondMod { log: log.clone() })),
    ]
    .into();
    fixture_mods(mods, |game| {
        setup(
            game,
            "minecraft:cauldron",
            None,
            "minecraft:water_bucket",
            1,
        );
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Consume);
        assert_eq!(state(game), ("minecraft:cauldron".into(), None));
        assert_eq!(
            &*game.inventory[0].as_ref().unwrap().item,
            "minecraft:water_bucket"
        );

        setup(game, "minecraft:cauldron", None, "minecraft:stick", 1);
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Success);

        assert!(
            game.items()
                .item_in_tag("minecraft:arrows", "minecraft:arrow")
        );
        setup(game, "minecraft:cauldron", None, "minecraft:arrow", 1);
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Success);

        assert_eq!(
            log.lock().unwrap().as_slice(),
            &["vanilla_override", "second_mod_stick", "tag_arrows"]
        );
    });
}

#[test]
#[ignore = "daily"]
fn cauldron_panicking_handler_disables_callback_and_reports_toast() {
    if run_fresh_cases(
        "cut_layout::game::cauldron_interactions::tests::cauldron_panicking_handler_disables_callback_and_reports_toast",
        "SUNRISE_CAULDRON_CASE",
        &["run"],
    ) {
        return;
    }
    struct Panicking {
        calls: Arc<AtomicUsize>,
    }
    impl mod_api::Interaction for Panicking {
        fn interact(&self, _host: &mut dyn ServerHost, _input: &CauldronUse) -> CauldronDecision {
            self.calls.fetch_add(1, Ordering::SeqCst);
            panic!("boom in cauldron interaction");
        }
    }
    struct PanicMod {
        calls: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for PanicMod {
        fn register_cauldron_interaction(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterCauldronInteractionEvent<'_>,
        ) {
            event
                .interactions
                .register(
                    mod_api::__private::block_key("minecraft:cauldron").unwrap(),
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    Arc::new(Panicking {
                        calls: self.calls.clone(),
                    }),
                )
                .unwrap();
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "panic_cauldron",
        Box::new(PanicMod {
            calls: calls.clone(),
        }),
    )]
    .into();
    fixture_mods(mods, |game| {
        let _ = mod_api::__private::take_hook_panics();
        setup(game, "minecraft:cauldron", None, "minecraft:stick", 1);

        let before_swings = game.swings_started;
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Consume);
        assert_eq!(game.swings_started, before_swings);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            mod_api::__private::take_hook_panics(),
            [mod_api::__private::HookPanicReport {
                mod_id: "panic_cauldron",
                hook: "cauldron_interaction",
            }]
        );

        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Consume);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(mod_api::__private::take_hook_panics().is_empty());

        // Reloading rebuilds the cauldron table, but the panicked mod's callback stays
        // disabled for the rest of the session.
        game.resolve_parts().unwrap();
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Consume);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(mod_api::__private::take_hook_panics().is_empty());

        setup(game, "minecraft:cauldron", None, "minecraft:lava_bucket", 1);
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Success);
        assert_eq!(state(game), ("minecraft:lava_cauldron".into(), None));
        assert_eq!(
            &*game.inventory[0].as_ref().unwrap().item,
            "minecraft:bucket"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn cauldron_displaced_handlers_drop_under_original_owner_and_order_edits() {
    use mod_api::{ModEntry, RegistrationContext};
    use sunrise_mod_runtime::block_definitions::BlockDefinition;
    use sunrise_world::{loot::PackOverlay, registry::BlockRegistry};

    struct PanicDrop {
        label: &'static str,
        dropped: Arc<AtomicUsize>,
    }
    impl mod_api::Interaction for PanicDrop {
        fn interact(&self, _host: &mut dyn ServerHost, _input: &CauldronUse) -> CauldronDecision {
            CauldronDecision::Consume
        }
    }
    impl Drop for PanicDrop {
        fn drop(&mut self) {
            self.dropped.fetch_add(1, Ordering::SeqCst);
            panic!("drop panic for {}", self.label);
        }
    }
    struct Labelled(&'static str);
    impl mod_api::Interaction for Labelled {
        fn interact(&self, _host: &mut dyn ServerHost, _input: &CauldronUse) -> CauldronDecision {
            let _ = self.0;
            CauldronDecision::Consume
        }
    }
    struct OldOwner {
        dropped: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for OldOwner {
        fn register_cauldron_interaction(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::RegisterCauldronInteractionEvent<'_>,
        ) {
            let water = mod_api::__private::block_key("minecraft:water_cauldron").unwrap();
            let item = |id| mod_api::__private::item_key(id).unwrap();
            event
                .interactions
                .register(
                    water,
                    item("test:exact"),
                    Arc::new(PanicDrop {
                        label: "exact",
                        dropped: self.dropped.clone(),
                    }),
                )
                .unwrap();
            event
                .interactions
                .register_tag(
                    water,
                    "test:tag",
                    Arc::new(PanicDrop {
                        label: "tag",
                        dropped: self.dropped.clone(),
                    }),
                )
                .unwrap();
            event
                .interactions
                .register(
                    water,
                    item("test:direct_then_all"),
                    Arc::new(PanicDrop {
                        label: "direct_then_all",
                        dropped: self.dropped.clone(),
                    }),
                )
                .unwrap();
            event
                .interactions
                .register_tag_to_all(
                    "test:all_tag_first",
                    Arc::new(PanicDrop {
                        label: "all_tag_first",
                        dropped: self.dropped.clone(),
                    }),
                )
                .unwrap();
            // Registered before direct overrides so registration order can be verified.
            event
                .interactions
                .register_to_all(item("test:all_then_direct"), Arc::new(Labelled("all_old")))
                .unwrap();
            event
                .interactions
                .register_tag_to_all(
                    "test:all_then_direct_tag",
                    Arc::new(Labelled("all_tag_old")),
                )
                .unwrap();
            event
                .interactions
                .register_tag(
                    water,
                    "test:second_tag",
                    Arc::new(Labelled("second_tag_old")),
                )
                .unwrap();
        }
    }
    struct NewOwner;
    #[mod_api::__subscriptions]
    impl Hooks for NewOwner {
        fn register_cauldron_interaction(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::RegisterCauldronInteractionEvent<'_>,
        ) {
            let water = mod_api::__private::block_key("minecraft:water_cauldron").unwrap();
            let custom = mod_api::__private::block_key("test:custom_cauldron").unwrap();
            let item = |id| mod_api::__private::item_key(id).unwrap();
            event
                .interactions
                .register(water, item("test:exact"), Arc::new(Labelled("exact_new")))
                .unwrap();
            event
                .interactions
                .register_tag(water, "#test:tag", Arc::new(Labelled("tag_new")))
                .unwrap();
            event
                .interactions
                .register_to_all(
                    item("test:direct_then_all"),
                    Arc::new(Labelled("direct_then_all_new")),
                )
                .unwrap();
            event
                .interactions
                .register_tag_to_all(
                    "#test:all_tag_first",
                    Arc::new(Labelled("all_tag_first_new")),
                )
                .unwrap();
            // Direct overrides registered after `register_to_all` / `register_tag_to_all` win on
            // the target block while the earlier all-dispatcher entries remain on other blocks.
            event
                .interactions
                .register(
                    water,
                    item("test:all_then_direct"),
                    Arc::new(Labelled("direct_new")),
                )
                .unwrap();
            event
                .interactions
                .register_tag(
                    water,
                    "test:all_then_direct_tag",
                    Arc::new(Labelled("direct_tag_new")),
                )
                .unwrap();
            // Declaring a custom block late still receives earlier `register_to_all` edits in
            // registration order.
            event
                .interactions
                .register(
                    custom,
                    item("test:custom_only"),
                    Arc::new(Labelled("custom")),
                )
                .unwrap();
        }
    }

    let dropped = Arc::new(AtomicUsize::new(0));
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new(
                "old_owner",
                Box::new(OldOwner {
                    dropped: dropped.clone(),
                }),
            ),
            ModEntry::new("new_owner", Box::new(NewOwner)),
        ]
        .into(),
        || {
            let _ = mod_api::__private::take_hook_panics();
            sunrise_gameplay_common::game::api_values::install_parts(&[]).unwrap();
            mod_api::__private::resolve_block_keys(
                ["minecraft:water_cauldron", "test:custom_cauldron"],
                |_| {
                    BlockDefinition::new(&[&Default::default()], 0)
                        .map_err(|err| format!("{err:?}"))
                },
            )
            .unwrap();
            let ids = [
                "test:exact",
                "test:direct_then_all",
                "test:all_then_direct",
                "test:custom_only",
            ];
            mod_api::__private::resolve_items(ids).unwrap();
            let root = tempfile::tempdir().unwrap();
            let item_json = ids
                .map(|id| (id, serde_json::json!({})))
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>();
            std::fs::write(
                root.path().join("items.json"),
                serde_json::to_vec(&item_json).unwrap(),
            )
            .unwrap();
            let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
            let items = Items::catalog(root.path(), &PackOverlay::NONE, &defaults).unwrap();
            let registry = BlockRegistry::from_facts_json(
                &br#"{
                "defaults":{"minecraft:water_cauldron":0,"test:custom_cauldron":1},
                "states":[
                    ["water_cauldron",false,false,"","minecraft:water_cauldron"],
                    ["custom_cauldron",false,false,"","test:custom_cauldron"]
                ]
            }"#[..],
            )
            .unwrap();

            let mut registrar = Registrar::new(&items, &registry);
            sunrise_gameplay_common::game::native_hooks::native_register_cauldron_interaction(
                &RegistrationContext::new(),
                &mut registrar,
            );
            let table = registrar.finish().unwrap();

            assert_eq!(dropped.load(Ordering::SeqCst), 4);
            let panics = mod_api::__private::take_hook_panics();
            assert!(!panics.is_empty());
            assert!(
                panics.iter().all(|report| report.mod_id == "old_owner"),
                "drop panics must be attributed to the displaced owner, got {panics:?}"
            );

            let water = "minecraft:water_cauldron";
            let empty = "minecraft:cauldron";
            let custom = "test:custom_cauldron";
            assert!(matches!(
                table.select(water, "test:exact", false, |_| false),
                Some(Entry::Mod(cb)) if cb.owner == "new_owner"
            ));
            assert!(matches!(
                table.select(water, "test:any", false, |tag| tag == "test:tag"),
                Some(Entry::Mod(cb)) if cb.owner == "new_owner"
            ));
            // direct-then-all: later `register_to_all` overwrote the earlier direct entry.
            for block in [water, empty, custom] {
                assert!(
                    matches!(
                        table.select(block, "test:direct_then_all", false, |_| false),
                        Some(Entry::Mod(cb)) if cb.owner == "new_owner"
                    ),
                    "{block}"
                );
            }
            // all-then-direct: later direct entry wins on water, while earlier all-dispatcher
            // entry stays on empty and on the late-declared custom block.
            assert!(matches!(
                table.select(water, "test:all_then_direct", false, |_| false),
                Some(Entry::Mod(cb)) if cb.owner == "new_owner"
            ));
            for block in [empty, custom] {
                assert!(
                    matches!(
                        table.select(block, "test:all_then_direct", false, |_| false),
                        Some(Entry::Mod(cb)) if cb.owner == "old_owner"
                    ),
                    "{block}"
                );
            }
            // Tag replacement stays in place before later tags (and beats exact item entries).
            assert!(matches!(
                table.select(water, "test:all_then_direct", false, |tag| {
                    tag == "test:all_then_direct_tag" || tag == "test:second_tag"
                }),
                Some(Entry::Mod(cb)) if cb.owner == "new_owner"
            ));
            assert!(matches!(
                table.select(custom, "test:any", false, |tag| {
                    tag == "test:all_then_direct_tag"
                }),
                Some(Entry::Mod(cb)) if cb.owner == "old_owner"
            ));
            assert!(matches!(
                table.select(water, "test:any", false, |tag| {
                    tag == "test:all_tag_first" || tag == "test:second_tag"
                }),
                Some(Entry::Mod(cb)) if cb.owner == "new_owner"
            ));
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn cauldron_packet_pass_resumes_item_use_once_without_client_prediction() {
    if run_fresh_cases(
        "cut_layout::game::cauldron_interactions::tests::cauldron_packet_pass_resumes_item_use_once_without_client_prediction",
        "SUNRISE_CAULDRON_CASE",
        &[
            "pass",
            "consume",
            "success",
            "refused",
            "dead",
            "already-sent",
            "slot-changed",
            "aim-changed",
            "origin-changed",
            "no-swing",
            "server-sneak-override",
            "client-block-terminal",
            "client-item-on-terminal",
            "server-item-on-terminal",
            "skipped-block-brush",
            "packet-veto",
            "hold-drop",
            "hold-release",
        ],
    ) {
        return;
    }
    use sunrise_protocol::packets::play::{Clientbound as C, Serverbound as S};
    use {
        sunrise_gameplay_common::game::cauldron_prediction::USE_CHANNEL,
        sunrise_gameplay_common::game::cauldron_prediction::UseOnOutcome,
        sunrise_gameplay_common::game::cauldron_prediction::registration_packet,
        sunrise_gameplay_common::game::cauldron_prediction::use_result,
        sunrise_gameplay_common::game::server_side::native_sound_refs::tests::guest,
        sunrise_gameplay_common::game::server_side::native_sound_refs::tests::packets,
    };

    let case = std::env::var("SUNRISE_CAULDRON_CASE").unwrap();
    let already_sent = case == "already-sent";
    let block_terminal = case == "client-block-terminal";
    let sneak_override = case == "server-sneak-override";
    let brushing = case == "skipped-block-brush";
    let sneaking = sneak_override || brushing;
    let decision = match case.as_str() {
        "consume" | "server-sneak-override" => CauldronDecision::Consume,
        "success" => CauldronDecision::Success,
        _ => CauldronDecision::Pass,
    };
    let item = match case.as_str() {
        "server-sneak-override" => "minecraft:milk_bucket",
        "no-swing" | "aim-changed" => "minecraft:ender_pearl",
        "origin-changed" => "minecraft:bucket",
        "skipped-block-brush" => "minecraft:brush",
        _ => "minecraft:paper",
    };
    struct PacketDecision {
        decision: CauldronDecision,
        calls: Arc<AtomicUsize>,
    }
    impl mod_api::Interaction for PacketDecision {
        fn interact(&self, _host: &mut dyn ServerHost, _input: &CauldronUse) -> CauldronDecision {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.decision
        }
    }
    struct PacketMod {
        case: String,
        item: &'static str,
        decision: CauldronDecision,
        calls: Arc<AtomicUsize>,
        item_uses: Arc<AtomicUsize>,
        block_packets: Arc<AtomicUsize>,
        client_item_on: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for PacketMod {
        fn register_cauldron_interaction(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterCauldronInteractionEvent<'_>,
        ) {
            event
                .interactions
                .register(
                    mod_api::__private::block_key("minecraft:cauldron").unwrap(),
                    mod_api::__private::item_key(self.item).unwrap(),
                    Arc::new(PacketDecision {
                        decision: self.decision,
                        calls: self.calls.clone(),
                    }),
                )
                .unwrap();
        }
        fn right_click_item(
            &self,
            host: mod_api::HostSide<'_>,
            _: &mut mod_api::RightClickItemEvent<'_>,
        ) -> mod_api::Flow {
            if matches!(host, mod_api::HostSide::Server(_)) {
                self.item_uses.fetch_add(1, Ordering::SeqCst);
            }
            mod_api::Flow::Continue
        }
        fn right_click_block(
            &self,
            host: mod_api::HostSide<'_>,
            event: &mut mod_api::RightClickBlockEvent<'_>,
        ) -> mod_api::Flow {
            match (self.case.as_str(), host) {
                ("server-sneak-override", mod_api::HostSide::Server(_)) => {
                    *event.use_block = mod_api::TriState::True;
                }
                ("client-block-terminal", mod_api::HostSide::Client(_)) => {
                    *event.cancellation_result = mod_api::InteractionResult::Success;
                    return mod_api::Flow::Cancel;
                }
                _ => {}
            }
            mod_api::Flow::Continue
        }
        fn use_item_on_block(
            &self,
            host: mod_api::HostSide<'_>,
            event: &mut mod_api::UseItemOnBlockEvent<'_>,
        ) -> mod_api::Flow {
            if event.phase != mod_api::UsePhase::ItemAfterBlock {
                return mod_api::Flow::Continue;
            }
            let client = matches!(host, mod_api::HostSide::Client(_));
            if client {
                self.client_item_on.fetch_add(1, Ordering::SeqCst);
            }
            if (client && self.case == "client-item-on-terminal")
                || (!client && self.case == "server-item-on-terminal")
            {
                *event.cancellation_result = mod_api::InteractionResult::Consume;
                return mod_api::Flow::Cancel;
            }
            mod_api::Flow::Continue
        }
        fn interaction_key_mapping_triggered(
            &self,
            _: &mut dyn mod_api::ClientHost,
            event: &mut mod_api::InteractionKeyMappingTriggeredEvent,
        ) -> mod_api::Flow {
            if self.case == "no-swing" {
                event.swing_hand = false;
            }
            mod_api::Flow::Continue
        }
        fn client_packet_send(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut mod_api::ClientPacketSendEvent<'_>,
        ) -> mod_api::Flow {
            if event.packet.id.path() == "use_item_on" {
                self.block_packets.fetch_add(1, Ordering::SeqCst);
                if self.case == "packet-veto" {
                    return mod_api::Flow::Cancel;
                }
            }
            mod_api::Flow::Continue
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let item_uses = Arc::new(AtomicUsize::new(0));
    let block_packets = Arc::new(AtomicUsize::new(0));
    let client_item_on = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "packet_cauldron",
        Box::new(PacketMod {
            case: case.clone(),
            item,
            decision,
            calls: calls.clone(),
            item_uses: item_uses.clone(),
            block_packets: block_packets.clone(),
            client_item_on: client_item_on.clone(),
        }),
    )]
    .into();
    fixture_mods_pair(mods, |server, client| {
        let server_table = snapshot();
        let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(client);
        let mut read_uses = |client: &sunrise_client_gameplay::game::ClientGame<'_>| {
            // Use the same writer, outside the actor hold, to fence each burst.
            client
                .client
                .remote
                .as_ref()
                .unwrap()
                .session
                .send(S::PingRequest(sunrise_protocol::packets::status::Ping {
                    time: 0,
                }));
            let mut uses = Vec::new();
            loop {
                let (id, bytes) = peer.recv().unwrap();
                let packet = S::decode(id, &bytes).unwrap();
                match packet {
                    S::PingRequest(_) => return uses,
                    S::UseItemOn(_) | S::UseItem(_) => uses.push(packet),
                    S::CustomPayload(ref p) if p.channel == USE_CHANNEL => uses.push(packet),
                    _ => {}
                }
            }
        };
        assert!(read_uses(client).is_empty());
        client.client.remote.as_mut().unwrap().joined = true;
        client.set_local_mode(sunrise_command::args::GameMode::Survival);
        setup_common_with(client, "minecraft:cauldron", None, item, 1, |client| {
            client.client.audio.0.capture = Some(Vec::new());
        });
        let aim = (35.0, -12.0);
        (client.yaw, client.pitch) = aim;
        if sneaking {
            client.client.remote.as_mut().unwrap().last_input |=
                sunrise_gameplay_common::game::remote::SHIFT_INPUT;
        }
        // The client has no local registration. Selection must come from the server.
        clear();
        if !already_sent {
            client
                .apply_packet(registration_packet(&server_table))
                .unwrap();
            client.offhand = Some(ItemStack::new("minecraft:water_bucket", 1));
        }
        let hit = Hit {
            pos: POS,
            face: [0, 1, 0],
            point: [0.5, 65.0, 0.5],
            inside: false,
        };
        client.set_test_forced_hit(Some(hit));
        let client_swings = client.swings_started;
        let hold_call = || {
            mod_platform::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                id: "packet_cauldron",
                hook: "client_tick",
                part: None,
            })
        };
        let hold = matches!(case.as_str(), "hold-drop" | "hold-release").then(|| {
            let _owner = hold_call();
            client.hold_packets().unwrap()
        });
        client.remote_use(mod_api::InteractionAttempt::Press);
        // In the vanilla transport control, paper's air use passes, so the
        // off hand still gets its ordinary block turn even though it's empty.
        assert_eq!(
            block_packets.load(Ordering::SeqCst),
            1 + usize::from(already_sent)
        );
        assert_eq!(
            client_item_on.load(Ordering::SeqCst),
            usize::from(already_sent),
            "the item phase must wait for receipt on a registered click",
        );
        assert_eq!(
            client.swings_started - client_swings,
            u32::from(block_terminal)
        );
        assert!(client.using.is_none(), "no speculative item use");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(state(client).0, "minecraft:cauldron");
        if let Some(hold) = hold {
            assert!(
                read_uses(client).is_empty(),
                "held block use must not reach the peer"
            );
            let _owner = hold_call();
            if case == "hold-drop" {
                client.drop_held(hold).unwrap();
            } else {
                assert!(client.release_held(hold).unwrap() > 0);
            }
        }
        let mut uses = read_uses(client).into_iter();
        if matches!(case.as_str(), "hold-drop" | "packet-veto") {
            assert_eq!(uses.len(), 0);
            // Neither a dropped request nor a canceled action may leave a
            // continuation waiting for the first receipt of this connection.
            client
                .apply_packet(use_result(
                    1,
                    UseOnOutcome {
                        block_fell_through: true,
                        passed: true,
                    },
                ))
                .unwrap();
            assert!(read_uses(client).is_empty());
            return;
        }
        let block_use = uses.next().expect("client must send the block use");
        if already_sent || block_terminal {
            assert!(matches!(&block_use, S::UseItemOn(p) if p.hand == 0 && p.pos == POS));
        } else {
            assert!(matches!(&block_use, S::CustomPayload(p) if p.channel == USE_CHANNEL));
        }
        assert_eq!(
            uses.len(),
            2 * usize::from(already_sent),
            "no speculative follow-up on a registered click"
        );

        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan(server);
        setup(server, "minecraft:cauldron", None, item, 1);
        install(server_table);
        let at = if case == "refused" {
            [64.5, 64.0, 2.5]
        } else {
            [0.5, 64.0, 2.5]
        };
        let (mut player, output) = guest(1, 2123, at);
        let player_entity_id = player.entity_id;
        player.load_timeout = 0;
        player.inventory[0] = Some(ItemStack::new(item, 1));
        player.inventory[1] = Some(ItemStack::new("minecraft:ender_pearl", 1));
        player.sneaking = sneaking;
        player.dead = case == "dead";
        if !already_sent {
            player.offhand = Some(ItemStack::new("minecraft:water_bucket", 1));
        }
        server.server.lan.as_mut().unwrap().hub.guests.push(player);
        server.lan_guest_packet(1, block_use);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            usize::from(!matches!(
                case.as_str(),
                "refused" | "dead" | "skipped-block-brush"
            ))
        );
        if brushing {
            assert_eq!(
                server.as_guest(1, |guest| guest.using),
                Some(Some((0, 0))),
                "the server's BrushItem.useOn started using the brush"
            );
        }
        assert_eq!(
            item_uses.load(Ordering::SeqCst),
            0,
            "never synthesize Item.use"
        );

        if already_sent {
            let item_use = uses
                .next()
                .expect("client already sent the vanilla Item.use");
            assert!(matches!(&item_use, S::UseItem(p) if p.hand == 0));
            server.lan_guest_packet(1, item_use);
            let offhand = uses.next().expect("the vanilla click tries its other hand");
            assert!(matches!(&offhand, S::UseItemOn(p) if p.hand == 1 && p.pos == POS));
            server.lan_guest_packet(1, offhand);
            assert_eq!(item_uses.load(Ordering::SeqCst), 1);
            return;
        }

        let replies = packets(&output);
        // LivingEntity.swing broadcasts ClientboundSwingAnimationPacket, not Animate.
        let swings: Vec<_> = replies
            .iter()
            .filter_map(|p| match p {
                C::SwingAnimation(swing) => Some((swing.id, swing.hand)),
                _ => None,
            })
            .collect();
        assert_eq!(
            swings.as_slice(),
            (case == "success")
                .then_some((player_entity_id, 0))
                .as_slice(),
        );
        let response = replies
            .into_iter()
            .find(|p| matches!(p, C::CustomPayload(p) if p.channel == USE_CHANNEL));
        if block_terminal {
            assert!(
                response.is_none(),
                "locally terminal input cannot have a continuation"
            );
            return;
        }
        let response = response.expect("server must answer the exact request");
        // The off hand must use the captured block hit even if picking has
        // changed meanwhile. A changed slot or ray must not start another use.
        client.set_test_forced_hit(None);
        (client.yaw, client.pitch) = aim;
        if case == "slot-changed" {
            client.inventory[1] = Some(ItemStack::new("minecraft:ender_pearl", 1));
            client.selected = 1;
        }
        if case == "aim-changed" {
            (client.yaw, client.pitch) = (90.0, 25.0);
        }
        if case == "origin-changed" {
            client.player.as_mut().unwrap().position.x += 1.0;
            assert_eq!((client.yaw, client.pitch), aim);
            assert_eq!(
                client.inventory[0],
                Some(ItemStack::new("minecraft:bucket", 1))
            );
            assert_eq!(
                client.offhand,
                Some(ItemStack::new("minecraft:water_bucket", 1))
            );
        }
        client.apply_packet(response.clone()).unwrap();
        client.apply_packet(response).unwrap();
        let followup = read_uses(client);
        let pass_to_offhand = matches!(case.as_str(), "pass" | "hold-release");
        if pass_to_offhand {
            assert!(
                matches!(followup.as_slice(), [S::UseItem(main), S::UseItemOn(off)]
                    if main.hand == 0 && (main.yaw, main.pitch) == aim && off.hand == 1 && off.pos == POS),
                "{followup:?}",
            );
        } else if case == "no-swing" {
            assert!(matches!(followup.as_slice(), [S::UseItem(p)]
                if p.hand == 0 && (p.yaw, p.pitch) == aim));
            assert_eq!(
                client.swings_started, client_swings,
                "keep the original input swing policy"
            );
        } else {
            assert!(followup.is_empty(), "{followup:?}");
        }
        assert_eq!(
            client_item_on.load(Ordering::SeqCst),
            usize::from(matches!(
                case.as_str(),
                "pass"
                    | "hold-release"
                    | "no-swing"
                    | "client-item-on-terminal"
                    | "server-item-on-terminal"
                    | "skipped-block-brush"
            )),
            "run the local item phase only once and only after block fallthrough",
        );
        assert_eq!(
            client.using,
            brushing.then_some((0, 0)),
            "brush use begins locally even when the server's item phase consumed",
        );

        // Terminal or refused clicks cannot eat the next independent item packet.
        let independent = if case == "no-swing" {
            // This case used up the guest's pearl; there is no second item to use.
            Vec::new()
        } else {
            client.selected = 0;
            client.inventory[0] = Some(ItemStack::new("minecraft:paper", 1));
            assert!(!client.remote_use_item(Hand::Main));
            let independent = read_uses(client);
            assert!(matches!(independent.as_slice(), [S::UseItem(p)] if p.hand == 0));
            independent
        };
        for packet in followup.into_iter().chain(independent) {
            server.lan_guest_packet(1, packet);
        }
        assert_eq!(
            item_uses.load(Ordering::SeqCst),
            usize::from(case != "dead") + usize::from(pass_to_offhand)
        );
        assert_eq!(
            state(server).0,
            if pass_to_offhand {
                "minecraft:water_cauldron"
            } else {
                "minecraft:cauldron"
            }
        );
    });
}

#[test]
#[ignore = "daily"]
fn cauldron_input_projection_failure_consumes_without_fallthrough() {
    if run_fresh_cases(
        "cut_layout::game::cauldron_interactions::tests::cauldron_input_projection_failure_consumes_without_fallthrough",
        "SUNRISE_CAULDRON_CASE",
        &["run"],
    ) {
        return;
    }
    struct PassHandler {
        calls: Arc<AtomicUsize>,
    }
    impl mod_api::Interaction for PassHandler {
        fn interact(&self, _host: &mut dyn ServerHost, _input: &CauldronUse) -> CauldronDecision {
            self.calls.fetch_add(1, Ordering::SeqCst);
            CauldronDecision::Pass
        }
    }
    struct GuardMod {
        calls: Arc<AtomicUsize>,
        fallthrough: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for GuardMod {
        fn register_cauldron_interaction(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterCauldronInteractionEvent<'_>,
        ) {
            event
                .interactions
                .register(
                    mod_api::__private::block_key("minecraft:cauldron").unwrap(),
                    mod_api::__private::item_key("minecraft:milk_bucket").unwrap(),
                    Arc::new(PassHandler {
                        calls: self.calls.clone(),
                    }),
                )
                .unwrap();
        }
        fn right_click_item(
            &self,
            _: mod_api::HostSide<'_>,
            _: &mut mod_api::RightClickItemEvent<'_>,
        ) -> mod_api::Flow {
            self.fallthrough.fetch_add(1, Ordering::SeqCst);
            mod_api::Flow::Continue
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let fallthrough = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "guard_cauldron",
        Box::new(GuardMod {
            calls: calls.clone(),
            fallthrough: fallthrough.clone(),
        }),
    )]
    .into();
    fixture_mods(mods, |game| {
        let hit = Hit {
            pos: POS,
            face: [0, 1, 0],
            point: [0.5, 65.0, 0.5],
            inside: false,
        };
        // A malformed held stack (negative damage component fails `TryIntoApi`) is refused as
        // `Outcome::Consume` with no swing, no callback run and no fallthrough to `Item.use`.
        setup(game, "minecraft:cauldron", None, "minecraft:milk_bucket", 1);
        game.inventory[0]
            .as_mut()
            .unwrap()
            .data_mut()
            .set_other("minecraft:damage", Some(sunrise_core::nbt::Tag::Int(-1)));
        let swings = game.swings_started;
        assert_eq!(game.use_on_cauldron(POS).unwrap(), Outcome::Consume);
        assert_eq!(game.swings_started, swings);
        assert!(game.use_hand(Some(&hit)).unwrap());
        assert_eq!(game.swings_started, swings);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(fallthrough.load(Ordering::SeqCst), 0);
        assert!(game.using.is_none());

        // With a valid held stack, the callback runs and `Pass` falls through normally
        // to `Item.use` (starting to drink the milk bucket, which consumes the click).
        setup(game, "minecraft:cauldron", None, "minecraft:milk_bucket", 1);
        assert!(game.use_hand(Some(&hit)).unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallthrough.load(Ordering::SeqCst), 1);
        assert!(game.using.is_some());
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
