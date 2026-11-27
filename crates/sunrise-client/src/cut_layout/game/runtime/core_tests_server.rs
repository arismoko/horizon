use mod_api::ServerHost;
use std::sync::Arc;
use sunrise_gameplay_common::game::GuestUse;
use sunrise_gameplay_common::game::Hit;
use sunrise_items::items::ItemStack;
use sunrise_sim::first_person_sim::Hand;

#[test]
#[ignore = "daily"]
fn eager_saves_wait_for_an_open_world_and_for_light() {
    let mut resources = None;
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
    let dir = std::env::temp_dir().join(format!("sunrise-eager-game-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    game.server.save.0 = Arc::new(sunrise_sim::save::SaveDir::new(
        dir.clone(),
        sunrise_sim::save::LevelShape::OVERWORLD,
        Some(Default::default()),
    ));
    Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
    game.world
        .chunks
        .insert((1, 0), Arc::new(sunrise_level::chunk::Chunk::empty(1, 0)));
    game.world.modified.extend([(0, 0), (1, 0)]);
    game.save_chunks_eagerly(); // no world open: nothing to save into
    assert!(game.world.modified.contains(&(0, 0)));
    game.server.world_root = dir.clone();
    game.save_chunks_eagerly();
    assert!(!game.world.modified.contains(&(0, 0)));
    assert!(game.world.modified.contains(&(1, 0))); // not lit yet
    Arc::make_mut(game.world.chunks.get_mut(&(1, 0)).unwrap()).lit = true;
    game.save_chunks_eagerly(); // still queued: no new dirty transition needed
    assert!(!game.world.modified.contains(&(1, 0)));
    game.server.save.0.flush().unwrap();
    assert!(dir.join("region/r.0.0.mca").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn fence_ties_without_a_lead_only_on_main_hand() {
    use mod_api::BlockUpdateFlags;
    use sunrise_sim::mobs::leash::Holder;
    super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let level = game.level("minecraft:overworld").unwrap();
        let fence = game
            .pack
            .assets
            .registry
            .default_state("minecraft:oak_fence")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], fence, BlockUpdateFlags::NEIGHBORS),
            Ok(true)
        );
        let hit = Hit {
            pos: [0, 64, 0],
            face: [0, 1, 0],
            point: [0.5, 65.0, 0.5],
            inside: false,
        };
        let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
        let mut mob = sunrise_sim::mobs::Mob::new(
            12,
            kind,
            sunrise_core::geometry::Vec3::new(1.0, 64.0, 1.0),
            0.0,
        );
        mob.extra.leash = Some(Holder::Player);
        game.mobs.push(mob);
        game.server.forced_hit = Some(hit);
        game.server.guest_use = Some(GuestUse::on_block(Hand::Off));
        game.with_hand(Hand::Off, |game| game.use_hand(Some(&hit)))
            .unwrap();
        assert_eq!(game.mobs[0].extra.leash, Some(Holder::Player));
        game.offhand = Some(ItemStack::new("minecraft:lead", 1));
        game.with_hand(Hand::Off, |game| game.use_hand(Some(&hit)))
            .unwrap();
        assert_eq!(game.mobs[0].extra.leash, Some(Holder::Knot(hit.pos)));
        game.mobs[0].extra.leash = Some(Holder::Player);
        game.offhand = None;
        game.server.guest_use = Some(GuestUse::on_block(Hand::Main));
        game.with_hand(Hand::Main, |game| game.use_hand(Some(&hit)))
            .unwrap();
        assert_eq!(game.mobs[0].extra.leash, Some(Holder::Knot(hit.pos)));
    });
}

#[test]
#[ignore = "daily"]
fn reload_resolves_parts_against_reloaded_tags() {
    use mod_api::{self, BlockPart, DataPackFilesEvent, Hooks, Registrar};
    struct TagPart;
    #[mod_api::sunrise_part]
    impl BlockPart for TagPart {
        fn analog_output(
            &self,
            _: &mut dyn mod_api::LevelRandomHost,
            _: &mut mod_api::AnalogOutputEvent,
        ) {
        }
    }
    let reload_tag = std::sync::Arc::new(std::sync::OnceLock::new());
    struct ReloadMod(std::sync::Arc<std::sync::OnceLock<mod_api::TagKey>>);
    #[mod_api::__subscriptions]
    impl Hooks for ReloadMod {
        fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let _ = self.0.set(registrar.tag_key("test:reload"));
            registrar.block_part_tag("test:reload", TagPart);
        }
        fn data_pack_files(&self, _: &mut dyn ServerHost, event: &mut DataPackFilesEvent<'_>) {
            event.files.push((
                "tags/test:block/reload".into(),
                br#"{"values":["minecraft:stone"]}"#.to_vec(),
            ));
        }
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "reload",
        Box::new(ReloadMod(reload_tag.clone())),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::__private::install(mods);
    super::native_entity_refs::tests::with_game(|game| {
        let stone_key = mod_platform::__private::part_tests::block_key("stone");
        let name = stone_key.id(game).expect("stone is registered").to_owned();
        let dir = std::env::temp_dir().join(format!("sunrise-key-tags-{}", std::process::id()));
        game.server.session_lock.0 = Some(sunrise_sim::save::SessionLock::acquire(&dir).unwrap());
        let reload_tag = (*reload_tag.get().unwrap()).typed();
        let base_stone_tag = mod_api::Tags::key(
            &*game,
            &mod_api::Identifier::parse("base_stone_overworld").unwrap(),
        )
        .unwrap()
        .typed();
        let logs_tag = mod_api::Tags::key(
            &*game,
            &mod_api::Identifier::parse("minecraft:logs").unwrap(),
        )
        .unwrap()
        .typed();
        assert_eq!(stone_key.is_in_tag(&*game, reload_tag), Some(false));
        assert_eq!(stone_key.is_in_tag(&*game, base_stone_tag), Some(true));
        game.load_server_loot();
        let stack = sunrise_gameplay::game::server_side::functions::pack_stack(
            &game.server.world_root,
            &game.options.resources,
            &game.pack.data_source,
            &game.pack.data_changes,
            game.server.cmds.0.vanilla_disabled,
            &game.level.enabled_features,
            &game.pack.additional_data,
        );
        let root = game.options.resources.clone();
        let previous = std::sync::Arc::new(mod_api::DatapackRegistrySet::from_candidate(
            mod_api::DatapackCandidate::build(
                &[],
                &[],
                &mut mod_api::__private::NoVanillaValueDecoder,
            )
            .unwrap(),
        ));
        sunrise_sim::datapack_net::serve(Some(previous.clone()));
        let prepared = game.prepare_world_reload(&stack, &root).unwrap();
        // Preparation must not publish to joining clients.
        assert!(std::sync::Arc::ptr_eq(
            &previous,
            &sunrise_sim::datapack_net::served().unwrap(),
        ));
        game.install_world_tables(prepared);
        // A successful empty publication clears the previous world's served registries too.
        assert!(sunrise_sim::datapack_net::served().is_none());
        game.run_native_advancement_grants();
        assert_eq!(stone_key.id(game), Some(name.as_str()));
        assert_eq!(
            stone_key,
            mod_platform::__private::part_tests::block_key("minecraft:stone")
        );
        assert_eq!(stone_key.is_in_tag(&*game, reload_tag), Some(true));
        assert_eq!(stone_key.is_in_tag(&*game, logs_tag), Some(false));
        assert_eq!(stone_key.is_in_tag(&*game, base_stone_tag), Some(true));
        let stone = game.block_state_default(stone_key).unwrap();
        assert!(
            sunrise_gameplay_common::game::api_values::block_state::block_listens(
                mod_api::HookId::AnalogOutput,
                stone
            )
        );
        game.server.session_lock.0 = None;
        let _ = std::fs::remove_dir_all(dir);
    });
}

#[test]
#[ignore = "daily"]
fn right_click_block_controls_cauldron_use() {
    use mod_api::{self, BlockUpdateFlags, Flow, Hooks, RightClickBlockEvent, TriState};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    struct Observer(Arc<AtomicBool>, Arc<AtomicBool>, Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn right_click_block(
            &self,
            _: mod_api::HostSide,
            event: &mut RightClickBlockEvent<'_>,
        ) -> Flow {
            self.2.fetch_add(1, Ordering::SeqCst);
            if self.1.load(Ordering::SeqCst) {
                return Flow::Cancel;
            }
            *event.use_block = if self.0.load(Ordering::SeqCst) {
                TriState::True
            } else {
                TriState::False
            };
            *event.use_item = TriState::False;
            Flow::Continue
        }
    }
    let allowed = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = Arc::from([mod_api::ModEntry::new(
        "right-click-observer",
        Box::new(Observer(allowed.clone(), cancel.clone(), calls.clone())),
    )]);
    mod_platform::__private::install(mods);
    super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let level = game.level("minecraft:overworld").unwrap();
        let empty = game
            .pack
            .assets
            .registry
            .default_state("minecraft:cauldron")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], empty, BlockUpdateFlags::NEIGHBORS),
            Ok(true)
        );
        game.common.inventory[game.common.selected] =
            Some(ItemStack::new("minecraft:water_bucket", 1));
        let hit = Hit {
            pos: [0, 64, 0],
            face: [0, 1, 0],
            point: [0.5, 65.0, 0.5],
            inside: false,
        };
        game.use_hand(Some(&hit)).unwrap();
        assert_eq!(game.world.block(0, 64, 0), empty);
        allowed.store(true, Ordering::SeqCst);
        game.use_hand(Some(&hit)).unwrap();
        assert_ne!(game.world.block(0, 64, 0), empty);
        assert_eq!(
            game.level_set_block(level, hit.pos, empty, BlockUpdateFlags::NEIGHBORS),
            Ok(true)
        );
        game.common.inventory[game.common.selected] =
            Some(ItemStack::new("minecraft:water_bucket", 1));
        game.server_set_acting_sneak(true);
        game.use_hand(Some(&hit)).unwrap();
        assert_ne!(game.world.block(0, 64, 0), empty);
        game.server_set_acting_sneak(false);

        let campfire = game
            .pack
            .assets
            .registry
            .default_state("minecraft:campfire")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, hit.pos, campfire, BlockUpdateFlags::NEIGHBORS),
            Ok(true)
        );
        use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;
        let entity = game.new_block_entity("minecraft:campfire").unwrap();
        game.world.block_entities.insert((0, 64, 0), entity);
        game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:beef", 2));
        allowed.store(false, Ordering::SeqCst);
        game.use_hand(Some(&hit)).unwrap();
        assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 2);
        allowed.store(true, Ordering::SeqCst);
        game.use_hand(Some(&hit)).unwrap();
        assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 1);

        // A canceled click on a fence must not tie the player's mob.
        let fence = game
            .pack
            .assets
            .registry
            .default_state("minecraft:oak_fence")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, hit.pos, fence, BlockUpdateFlags::NEIGHBORS),
            Ok(true)
        );
        let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
        let mut mob = sunrise_sim::mobs::Mob::new(
            12,
            kind,
            sunrise_core::geometry::Vec3::new(1.0, 64.0, 1.0),
            0.0,
        );
        mob.extra.leash = Some(sunrise_sim::mobs::leash::Holder::Player);
        game.mobs.push(mob);
        game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:lead", 1));
        game.server.guest_use = Some(GuestUse::on_block(Hand::Main));
        game.server.forced_hit = Some(hit);
        cancel.store(true, Ordering::SeqCst);
        let before = calls.load(Ordering::SeqCst);
        game.use_item_as_player().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), before + 1);
        assert_eq!(
            game.mobs.last().unwrap().extra.leash,
            Some(sunrise_sim::mobs::leash::Holder::Player)
        );

        // A spectator's packet reaches the event, even when canceled.
        game.set_mode_id(3);
        game.server.guest_use = Some(GuestUse::on_block(Hand::Main));
        game.server.forced_hit = Some(hit);
        cancel.store(true, Ordering::SeqCst);
        let before = calls.load(Ordering::SeqCst);
        game.use_item_as_player().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), before + 1);
    });
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
