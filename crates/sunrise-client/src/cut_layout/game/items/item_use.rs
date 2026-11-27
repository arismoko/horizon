#![cfg(test)]
use sunrise_items::items::ItemStack;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1;

    use sunrise_core::use_animation::UseAnimation;

    /// `ItemStack::use_animation` (#4677): `ItemStack.getUseAnimation` with a mod's
    /// `ItemPart::use_animation` answer applied, the value the renderer reads, through a client
    /// host and a server host alike; the Part is asked on both sides with a read-only `Host`.
    #[test]
    #[ignore = "daily"]
    fn a_stacks_use_animation_carries_the_mods_answer_on_both_sides() {
        use mod_api::{Hooks, Host, ItemPart, ModEntry, Mods, Registrar, UseAnimationEvent};
        use sunrise_gameplay::game::native_entity_refs::tests::with_game;
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::item_use::tests::a_stacks_use_animation_carries_the_mods_answer_on_both_sides",
            || {
                /// A stick is drunk from.
                #[derive(Clone)]
                struct Part;
                #[mod_api::sunrise_part]
                impl ItemPart for Part {
                    fn use_animation(&self, _: &dyn Host, event: &mut UseAnimationEvent<'_>) {
                        *event.animation = UseAnimation::Drink;
                    }
                }
                struct Mod;
                #[mod_api::__subscriptions]
                impl Hooks for Mod {
                    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                        let stick = r.item_key("minecraft:stick");
                        r.item_part(stick, Part);
                    }
                }
                fn check<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
                    game: &G,
                    host: &dyn Host,
                    side: &str,
                ) {
                    let stack = |id: &str| {
                        mod_api::ItemStack::of(mod_api::__private::item_key(id).unwrap())
                    };
                    assert_eq!(
                        stack("minecraft:bow").use_animation(host),
                        UseAnimation::Bow,
                        "{side}"
                    );
                    assert_eq!(
                        stack("minecraft:stick").use_animation(host),
                        UseAnimation::Drink,
                        "{side}"
                    );
                    assert_eq!(
                        game.item_use_animation(&sunrise_items::items::ItemStack::new(
                            "minecraft:stick",
                            1
                        )),
                        UseAnimation::Drink,
                        "{side}"
                    );
                }
                let mods: Mods = vec![ModEntry::new("animation", Box::new(Mod))].into();
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                mod_platform::testing::with_mods(mods, || {
                    #[cfg(feature = "client")]
                    sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(
                        |game| {
                            check(game, game, "client");
                        },
                    );
                    with_game(|game| {
                        check(game, game, "server");
                    });
                });
            },
        );
    }
    /// A Part that reads `ItemStack::use_animation` of its own stack from inside its callback
    /// gets the item's own answer: the read does not ask the Part again (which never ended).
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_use_animation_read_inside_the_part_is_not_asked_again() {
        use crate::game::server_side::native_entity_refs::tests::with_client_game;
        use mod_api::{Hooks, Host, ItemPart, ModEntry, Mods, Registrar, UseAnimationEvent};
        use std::sync::{Arc, Mutex};
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::item_use::tests::a_use_animation_read_inside_the_part_is_not_asked_again",
            || {
                /// Reads its stack's effective animation, then answers Drink.
                #[derive(Clone)]
                struct Part(Arc<Mutex<Vec<UseAnimation>>>);
                #[mod_api::sunrise_part]
                impl ItemPart for Part {
                    fn use_animation(&self, host: &dyn Host, event: &mut UseAnimationEvent<'_>) {
                        let inner = mod_api::ItemStack::of(event.stack.item()).use_animation(host);
                        self.0.lock().unwrap().push(inner);
                        *event.animation = UseAnimation::Drink;
                    }
                }
                struct Mod(Arc<Mutex<Vec<UseAnimation>>>);
                #[mod_api::__subscriptions]
                impl Hooks for Mod {
                    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                        let stick = r.item_key("minecraft:stick");
                        r.item_part(stick, Part(self.0.clone()));
                    }
                }
                let inner = Arc::new(Mutex::new(Vec::new()));
                let mods: Mods =
                    vec![ModEntry::new("reentry", Box::new(Mod(inner.clone())))].into();
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                mod_platform::testing::with_mods(mods, || {
                    with_client_game(|game| {
                        let stick = mod_api::ItemStack::of(
                            mod_api::__private::item_key("minecraft:stick").unwrap(),
                        );
                        assert_eq!(
                            stick.use_animation(&*game as &dyn Host),
                            UseAnimation::Drink
                        );
                        // The read inside got the stick's own answer, and the Part was asked once.
                        assert_eq!(*inner.lock().unwrap(), [UseAnimation::None]);
                        // The renderer's read after it asks the Part again, as before.
                        assert_eq!(
                            game.item_use_animation(&sunrise_items::items::ItemStack::new(
                                "minecraft:stick",
                                1
                            )),
                            UseAnimation::Drink
                        );
                        assert_eq!(inner.lock().unwrap().len(), 2);
                    });
                });
            },
        );
    }
    /// Inside a Part, reading another stack's effective animation still asks that stack's Part:
    /// only a read of the stack being asked is answered without its Part.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn another_stacks_part_is_asked_from_inside_a_part() {
        use crate::game::server_side::native_entity_refs::tests::with_client_game;
        use mod_api::{Hooks, Host, ItemPart, ModEntry, Mods, Registrar, UseAnimationEvent};
        use std::sync::{Arc, Mutex};
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::item_use::tests::another_stacks_part_is_asked_from_inside_a_part",
            || {
                fn stack(id: &str) -> mod_api::ItemStack {
                    mod_api::ItemStack::of(mod_api::__private::item_key(id).unwrap())
                }
                /// A bow is drawn as a crossbow.
                #[derive(Clone)]
                struct Bow;
                #[mod_api::sunrise_part]
                impl ItemPart for Bow {
                    fn use_animation(&self, _: &dyn Host, event: &mut UseAnimationEvent<'_>) {
                        *event.animation = UseAnimation::Crossbow;
                    }
                }
                /// A stick reads a bow's and its own effective animation, then answers Drink.
                #[derive(Clone)]
                struct Stick(Arc<Mutex<Vec<UseAnimation>>>);
                #[mod_api::sunrise_part]
                impl ItemPart for Stick {
                    fn use_animation(&self, host: &dyn Host, event: &mut UseAnimationEvent<'_>) {
                        let bow = stack("minecraft:bow").use_animation(host);
                        let own = mod_api::ItemStack::of(event.stack.item()).use_animation(host);
                        self.0.lock().unwrap().extend([bow, own]);
                        *event.animation = UseAnimation::Drink;
                    }
                }
                struct Mod(Arc<Mutex<Vec<UseAnimation>>>);
                #[mod_api::__subscriptions]
                impl Hooks for Mod {
                    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                        let bow = r.item_key("minecraft:bow");
                        r.item_part(bow, Bow);
                        let stick = r.item_key("minecraft:stick");
                        r.item_part(stick, Stick(self.0.clone()));
                    }
                }
                let read = Arc::new(Mutex::new(Vec::new()));
                let mods: Mods = vec![ModEntry::new("nested", Box::new(Mod(read.clone())))].into();
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                mod_platform::testing::with_mods(mods, || {
                    with_client_game(|game| {
                        let host: &dyn Host = game;
                        assert_eq!(
                            stack("minecraft:stick").use_animation(host),
                            UseAnimation::Drink
                        );
                        // The bow's Part answered; the stick's own read did not ask it again.
                        assert_eq!(
                            *read.lock().unwrap(),
                            [UseAnimation::Crossbow, UseAnimation::None]
                        );
                    });
                });
            },
        );
    }
    /// A Part that reads the animation of a transformed copy of its own stack (one more in the
    /// count, each time) is not asked again: the copy gets the item's own answer, and the Part
    /// runs once per read instead of re-entering itself with each new count.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_part_reading_a_transformed_copy_is_not_asked_again() {
        use crate::game::server_side::native_entity_refs::tests::with_client_game;
        use mod_api::{Hooks, Host, ItemPart, ModEntry, Mods, Registrar, UseAnimationEvent};
        use std::sync::{Arc, Mutex};
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::item_use::tests::a_part_reading_a_transformed_copy_is_not_asked_again",
            || {
                /// Reads a copy of its stack with one more in the count, then answers Drink.
                #[derive(Clone)]
                struct Part(Arc<Mutex<Vec<UseAnimation>>>);
                #[mod_api::sunrise_part]
                impl ItemPart for Part {
                    fn use_animation(&self, host: &dyn Host, event: &mut UseAnimationEvent<'_>) {
                        let mut copy = event.stack.clone();
                        let _ = copy.set_count(event.stack.count() + 1);
                        let inner = copy.use_animation(host);
                        self.0.lock().unwrap().push(inner);
                        *event.animation = UseAnimation::Drink;
                    }
                }
                struct Mod(Arc<Mutex<Vec<UseAnimation>>>);
                #[mod_api::__subscriptions]
                impl Hooks for Mod {
                    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                        let stick = r.item_key("minecraft:stick");
                        r.item_part(stick, Part(self.0.clone()));
                    }
                }
                let inner = Arc::new(Mutex::new(Vec::new()));
                let mods: Mods = vec![ModEntry::new("copy", Box::new(Mod(inner.clone())))].into();
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                mod_platform::testing::with_mods(mods, || {
                    with_client_game(|game| {
                        let stick = mod_api::ItemStack::of(
                            mod_api::__private::item_key("minecraft:stick").unwrap(),
                        );
                        assert_eq!(
                            stick.use_animation(&*game as &dyn Host),
                            UseAnimation::Drink
                        );
                        // Asked once: the copy (count 2) read the stick's own answer.
                        assert_eq!(*inner.lock().unwrap(), [UseAnimation::None]);
                    });
                });
            },
        );
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn patched_bow_keeps_vanilla_animation_without_listener() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            use UseAnimation::*;
            for (id, expected) in [
                ("minecraft:bread", Eat),
                ("minecraft:milk_bucket", Drink),
                ("minecraft:shield", Block),
                ("minecraft:bow", Bow),
                ("minecraft:crossbow", Crossbow),
                ("minecraft:trident", Trident),
                ("minecraft:spyglass", Spyglass),
                ("minecraft:goat_horn", TootHorn),
                ("minecraft:brush", Brush),
                ("minecraft:bundle", Bundle),
                ("minecraft:iron_spear", Spear),
                ("minecraft:stick", None),
            ] {
                assert_eq!(
                    game.item_use_animation(&sunrise_items::items::ItemStack::new(id, 1)),
                    expected,
                    "{id}"
                );
            }
            let mut bow = sunrise_items::items::ItemStack::new("minecraft:bow", 1);
            bow.data_mut().set_other(
                "minecraft:consumable",
                Some(sunrise_core::nbt::Tag::compound([(
                    "animation",
                    sunrise_core::nbt::Tag::String("drink".into()),
                )])),
            );
            assert_eq!(game.item_use_animation(&bow), UseAnimation::Bow);
        });
    }
}

#[cfg(test)]
pub(crate) mod interact_vibration_tests {
    use super::*;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::GameGameExt1;

    use sunrise_gameplay::game::server_side::item_use::interact_vibrations;
    use sunrise_gameplay::game::server_side::sculk_server::item_use_event_fixture::{
        EVENTS, RECORDING,
    };

    /// Runs `f` on a windowless game built from the prepared resources with
    /// `held` in the main hand and returns the game events it posted;
    /// `None` without the resources.
    pub(crate) fn with_game(
        held: &str,
        f: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) -> Option<Vec<String>> {
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES")?);
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(10.5, 64.0, -3.25),
            )
            .unwrap(),
        );
        game.reset_entity_residency();
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        super::super::native_entity_refs::tests::bind_local_player(&mut game, 41, 123);
        *game.held_mut() = Some(ItemStack::new(held, 1));
        sunrise_gameplay::game::dispense_fixture::start_recording();
        RECORDING.set(true);
        f(&mut game);
        RECORDING.set(false);
        Some(sunrise_gameplay::game::dispense_fixture::stop_recording())
    }

    /// `LivingEntity.startUsingItem` ends with
    /// `this.useItem.causeUseVibration(this, GameEvent.ITEM_INTERACT_START)`,
    /// and `ItemStack.causeUseVibration` posts only when the stack's use effects
    /// ask for interaction vibrations (the default). `stopUsingItem` posts the
    /// finish the same way, which Sunrise already did, so a use is a pair of
    /// vibrations and an item that turns them off is neither.
    #[test]
    #[ignore = "daily"]
    fn a_use_start_is_an_item_interact_start_vibration() {
        let bow = || ItemStack::new("minecraft:bow", 1);
        let Some(lines) = with_game("minecraft:bow", |game| {
            assert!(game.start_using_item(), "the bow's use starts");
            game.stop_using_item(&bow());
        }) else {
            return;
        };
        assert_eq!(
            lines,
            vec![
                "G 10,64,-4 minecraft:item_interact_start".to_owned(),
                "G 10,64,-4 minecraft:item_interact_finish".to_owned(),
            ]
        );
        // A spear's use effects set `interact_vibrations` false: no vibration.
        let spear = || ItemStack::new("minecraft:iron_spear", 1);
        let Some(lines) = with_game("minecraft:iron_spear", |game| {
            assert!(game.start_using_item(), "the spear's use starts");
            game.stop_using_item(&spear());
        }) else {
            return;
        };
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn cake_context_uses_can_eat_instead_of_raw_hunger() {
        use mod_api::{CanEatEvent, Hooks};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Reverse(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Reverse {
            fn can_eat(
                &self,
                host: &dyn mod_api::Host,
                event: &mut CanEatEvent<'_>,
            ) -> Option<bool> {
                assert_eq!(event.player.entity().id(), 41);
                assert_eq!(
                    event.player.food(host).unwrap().level,
                    if self.0.load(Ordering::Relaxed) < 4 {
                        20
                    } else {
                        10
                    }
                );
                let call = self.0.fetch_add(1, Ordering::Relaxed);
                if event
                    .stack
                    .is_some_and(|stack| stack.item().id(host) == Some("minecraft:rotten_flesh"))
                {
                    return Some(false);
                }
                assert!(event.stack.is_none(), "cake has no food stack");
                // First exercise player-based overrides, then the stack-only refusal.
                (call < 4).then_some(!event.vanilla_can_eat)
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cake-refusal",
            Box::new(Reverse(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game("minecraft:rotten_flesh", |game| {
                let use_block = |game: &mut sunrise_gameplay::game::ServerGame<'_>, name: &str| {
                    let player = game.use_context();
                    game.with_block_use_level(|b, level, ticks| {
                        let state = b.table.default_state(b.table.block_by_name(name).unwrap());
                        b.use_without_item(
                            level,
                            ticks,
                            state,
                            sunrise_core::geometry::BlockHit {
                                block: sunrise_core::position::BlockPos::new(0, 64, 0),
                                direction: sunrise_core::geometry::Direction::Up,
                                location: sunrise_core::geometry::Vec3::new(0.5, 65.0, 0.5),
                                inside: false,
                            },
                            player,
                        )
                    })
                };
                game.creative = true;
                use_block(game, "minecraft:stone");
                assert_eq!(calls.load(Ordering::Relaxed), 0);
                assert_eq!(
                    use_block(game, "minecraft:cake"),
                    sunrise_world::level::InteractionResult::Pass
                );
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                assert_eq!(
                    use_block(game, "minecraft:candle_cake"),
                    sunrise_world::level::InteractionResult::Pass
                );
                assert_eq!(calls.load(Ordering::Relaxed), 2);
                game.creative = false;
                game.player.as_mut().unwrap().health.food.food = 20;
                assert_eq!(
                    use_block(game, "minecraft:cake"),
                    sunrise_world::level::InteractionResult::Success
                );
                assert_eq!(calls.load(Ordering::Relaxed), 3);
                assert_eq!(
                    use_block(game, "minecraft:candle_cake"),
                    sunrise_world::level::InteractionResult::Success,
                    "a stack-based food refusal does not refuse a candle-cake bite"
                );
                assert_eq!(calls.load(Ordering::Relaxed), 4);
                game.player.as_mut().unwrap().health.food.food = 10;
                assert_eq!(
                    use_block(game, "minecraft:cake"),
                    sunrise_world::level::InteractionResult::Success,
                    "holding refused food does not refuse a hungry player's cake bite"
                );
                assert_eq!(
                    use_block(game, "minecraft:candle_cake"),
                    sunrise_world::level::InteractionResult::Success
                );
            })
            .expect("SUNRISE_TEST_RESOURCES must name prepared resources");
            sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
                let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(client, "CakeClient").unwrap();
                let remote = game.client.remote.as_mut().unwrap();
                remote.logged_in = true;
                remote.entity_id = 41;
                let mut player = sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(0.5, 65.0, 0.5),
                )
                .unwrap();
                player.entity_id = 41;
                player.health.food.food = 10;
                game.player = Some(player);
                game.creative = false;
                *game.held_mut() = Some(ItemStack::new("minecraft:rotten_flesh", 1));
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                let cake = game.with_level(|b, level, _| {
                    let cake = b
                        .table
                        .default_state(b.table.block_by_name("minecraft:cake").unwrap());
                    assert!(
                        sunrise_world::level::LevelAccess::set_raw(
                            level,
                            sunrise_core::position::BlockPos::new(0, 64, 0),
                            cake,
                        )
                        .is_some()
                    );
                    cake
                });
                let hit = sunrise_gameplay_common::game::Hit {
                    pos: [0, 64, 0],
                    face: [0, 1, 0],
                    point: [0.5, 65.0, 0.5],
                    inside: false,
                };
                let held = game.held().cloned();
                let (result, (changes, _)) = game.remote_block_template(
                    &hit,
                    cake,
                    held.as_ref(),
                    Some(sunrise_world::level::UseBranch::WithoutItem),
                    true,
                );
                assert_eq!(
                    result,
                    Some(sunrise_world::level::InteractionResult::Success)
                );
                assert_eq!(changes.len(), 1, "the client predicts a bitten cake");
                let bitten = game.with_level(|b, _, _| {
                    let cake = b
                        .table
                        .default_state(b.table.block_by_name("minecraft:cake").unwrap());
                    b.table.set_int(cake, b.props.bites, 1)
                });
                assert_eq!(changes[0].1, bitten);
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    7,
                    "client dry-level path asks the hook"
                );
            });
        });
        can_eat_player_on_client_and_without_eater();
    }

    /// The listener sees the logged-in remote player on a client, and a food
    /// stack with no resolvable eater keeps vanilla's answer without asking it.
    #[cfg(feature = "client")]
    fn can_eat_player_on_client_and_without_eater() {
        use mod_api::{CanEatEvent, Hooks};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::motion::PlayerMotion;

        const REMOTE: u128 = 0xC0FF_EE00_0000_0000_0000_0000_0000_002A;
        struct Allow(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Allow {
            fn can_eat(
                &self,
                host: &dyn mod_api::Host,
                event: &mut CanEatEvent<'_>,
            ) -> Option<bool> {
                assert_eq!(event.player.entity().id(), 55);
                assert_eq!(event.player.uuid(), REMOTE);
                let food = event.player.food(host).expect("the client's own food data");
                assert_eq!((food.level, food.saturation), (20, 2.5));
                assert!(!event.vanilla_can_eat, "a full survival player cannot eat");
                self.0.fetch_add(1, Ordering::Relaxed);
                Some(true)
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "allow-eating",
            Box::new(Allow(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
                let food = ItemStack::new("minecraft:rotten_flesh", 1);
                let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(client, "Eater").unwrap();
                game.level.player_uuid = None;
                let remote = game.client.remote.as_mut().unwrap();
                remote.logged_in = true;
                remote.profile = REMOTE;
                remote.entity_id = 55;
                let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 65.0, 0.5)).unwrap();
                player.entity_id = 55;
                player.health.food.food = 20;
                player.health.food.saturation = 2.5;
                game.player = Some(player);
                assert!(
                    game.can_eat(Some(&food), false),
                    "the listener lets a full player eat"
                );
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                // Keep the body and food state, but no logged-in identity resolves.
                game.client.remote.as_mut().unwrap().logged_in = false;
                assert!(!game.can_eat(Some(&food), false));
                assert_eq!(calls.load(Ordering::Relaxed), 1, "no eater, no question");
            });
        });
    }

    #[test]
    fn stack_use_effects_patch_controls_vibrations() {
        let mut rod = ItemStack::new("minecraft:fishing_rod", 1);
        assert!(interact_vibrations(&rod, Some(true)));
        rod.data_mut().set_loot_component(
            "minecraft:use_effects",
            &Some(
                sunrise_core::json_nbt::json_to_tag(
                    &serde_json::json!({"interact_vibrations": false}),
                )
                .unwrap(),
            ),
        );
        assert!(!interact_vibrations(&rod, Some(true)));
        rod.data_mut().set_loot_component(
            "minecraft:use_effects",
            &Some(sunrise_core::nbt::Tag::Compound(Default::default())),
        );
        assert!(interact_vibrations(&rod, Some(false)));
        rod.data_mut().set_other("minecraft:use_effects", None);
        rod.data_mut().set_other(
            "!minecraft:use_effects",
            Some(sunrise_core::nbt::Tag::Compound(Default::default())),
        );
        assert!(!interact_vibrations(&rod, Some(true)));
    }

    /// `FishingRodItem.use` and the cast model's `FishingRodCast` take a
    /// content mod's item copied from the rod (`ItemInfo::is`) as a rod: it
    /// casts, is drawn cast while its hook is out, and reels in. Another item
    /// is not a rod.
    #[test]
    #[ignore = "daily"]
    fn a_content_mods_rod_casts_and_reels_in() {
        let Some(_) = with_game("minecraft:stick", |game| {
            // A world with a data pack has tables of its own, which take the mod's item.
            let dir =
                std::env::temp_dir().join(format!("sunrise-content-rod-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let tags = dir.join("datapacks/rods/data/ruby/tags/item");
            std::fs::create_dir_all(&tags).unwrap();
            std::fs::write(
                dir.join("datapacks/rods/pack.mcmeta"),
                r#"{"pack":{"description":"fixture"}}"#,
            )
            .unwrap();
            std::fs::write(
                tags.join("rods.json"),
                r#"{"values":["minecraft:fishing_rod"]}"#,
            )
            .unwrap();
            std::fs::write(
                dir.join("datapacks.json"),
                r#"{"enabled":["file/rods"],"disabled":[]}"#,
            )
            .unwrap();
            game.server.world_root = dir.clone();
            let stack = game.reload_datapacks(false, &[]).unwrap();
            let root = game.options.resources.clone();
            game.server.world_tables.0 = game
                .stage_world_tables(&stack, false, &root)
                .unwrap()
                .tables;
            let items = &mut std::sync::Arc::make_mut(
                &mut game.server.world_tables.0.as_mut().unwrap().items,
            )
            .items;
            let mut info = items["minecraft:fishing_rod"].clone();
            info.id = std::sync::Arc::from("ruby:rod");
            info.template = Some(std::sync::Arc::from("minecraft:fishing_rod"));
            assert!(
                sunrise_gameplay_common::game::remote::item_has_use(&info),
                "remote use accepts content rods"
            );
            items.insert(info.id.clone(), info);

            let items = &mut std::sync::Arc::make_mut(
                &mut game.server.world_tables.0.as_mut().unwrap().items,
            )
            .items;
            let mut stick_info = items["minecraft:stick"].clone();
            stick_info.id = std::sync::Arc::from("ruby:stick");
            stick_info.template = Some(std::sync::Arc::from("minecraft:stick"));
            items.insert(stick_info.id.clone(), stick_info);
            // Publish the edited tables as world open or reload would.
            game.load_server_loot();
            let stick = ItemStack::new("ruby:stick", 1);
            assert!(!game.use_fishing_rod(&stick), "a content stick is no rod");
            assert!(game.hook.is_none());
            let rod = ItemStack::new("ruby:rod", 1);
            *game.held_mut() = Some(rod.clone());
            assert!(game.main_hand_rod());
            let used_before = game.stats.get("used", "ruby:rod");
            let vanilla_before = game.stats.get("used", "minecraft:fishing_rod");
            assert!(game.use_fishing_rod(&rod), "the mod's rod casts");
            assert_eq!(game.stats.get("used", "ruby:rod"), used_before + 1);
            assert_eq!(
                game.stats.get("used", "minecraft:fishing_rod"),
                vanilla_before
            );
            assert!(game.hook.is_some());
            assert_eq!(game.cast_rod(), Some(&rod), "drawn cast");
            game.tick_fishing();
            assert!(
                game.hook.is_some(),
                "the content rod keeps its hook when ticked"
            );
            assert!(game.use_fishing_rod(&rod), "and reels in");
            assert_eq!(game.stats.get("used", "ruby:rod"), used_before + 1);
            assert_eq!(
                game.stats.get("used", "minecraft:fishing_rod"),
                vanilla_before
            );
            assert!(game.hook.is_none());
            assert_eq!(game.cast_rod(), None);
            std::fs::remove_dir_all(&dir).unwrap();
        }) else {
            panic!("prepared resources failed to load")
        };
    }

    #[test]
    #[ignore = "daily"]
    fn fishing_cast_and_reel_post_player_vibrations() {
        use sunrise_sim::mobs::Who;
        let Some(_) = with_game("minecraft:fishing_rod", |game| {
            let rod = ItemStack::new("minecraft:fishing_rod", 1);
            EVENTS.with(|events| events.borrow_mut().clear());
            assert!(game.use_fishing_rod(&rod));
            assert!(game.use_fishing_rod(&rod));
            EVENTS.with(|events| {
                assert_eq!(
                    *events.borrow(),
                    vec![
                        (
                            "minecraft:item_interact_start",
                            [10.5, 64.0, -3.25],
                            Some(Who::Player)
                        ),
                        (
                            "minecraft:item_interact_finish",
                            [10.5, 64.0, -3.25],
                            Some(Who::Player)
                        ),
                    ]
                );
            });
            let mut silent = rod.clone();
            silent.data_mut().set_loot_component(
                "minecraft:use_effects",
                &Some(
                    sunrise_core::json_nbt::json_to_tag(
                        &serde_json::json!({"interact_vibrations": false}),
                    )
                    .unwrap(),
                ),
            );
            *game.held_mut() = Some(silent.clone());
            EVENTS.with(|events| events.borrow_mut().clear());
            assert!(game.use_fishing_rod(&silent));
            assert!(game.use_fishing_rod(&silent));
            EVENTS.with(|events| assert!(events.borrow().is_empty()));
            *game.held_mut() = Some(rod.clone());
            assert!(game.use_fishing_rod(&rod));
            game.hook
                .as_mut()
                .unwrap()
                .move_to([10.5, 64.0, -3.25], true);
            let max = game
                .items()
                .get("minecraft:fishing_rod")
                .unwrap()
                .max_damage;
            game.held_mut().as_mut().unwrap().damage = max - 1;
            EVENTS.with(|events| events.borrow_mut().clear());
            assert!(game.use_fishing_rod(&rod));
            EVENTS.with(|events| assert!(events.borrow().is_empty(), "{:?}", *events.borrow()));
        }) else {
            panic!("prepared resources failed to load")
        };
    }

    #[test]
    #[ignore = "daily"]
    fn thrown_eye_posts_projectile_shoot_at_eye_from_player() {
        use sunrise_sim::mobs::Who;
        let Some(_) = with_game("minecraft:ender_eye", |game| {
            let root =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let terrain = sunrise_sim::vanillagen::VanillaTerrain::new(
                sunrise_sim::vanillagen::TerrainResources {
                    dir: &root.join("worldgen"),
                    data: Some(&game.pack.data_changes),
                    registry: game.pack.assets.registry,
                    biomes: &game.res.core.biomes,
                    item_data: &game.res.core.item_data,
                },
                1234,
            )
            .unwrap();
            game.end.set_locator(Some(std::sync::Arc::new(terrain)));
            EVENTS.with(|events| events.borrow_mut().clear());
            assert!(
                game.use_ender_eye(&ItemStack::new("minecraft:ender_eye", 1), false)
                    .unwrap()
            );
            EVENTS.with(|events| {
                let events = events.borrow();
                assert!(
                    events.contains(&(
                        "minecraft:projectile_shoot",
                        [10.5, 64.0 + f64::from(1.8_f32) * 0.5, -3.25],
                        Some(Who::Player),
                    )),
                    "{events:?}"
                );
            });
        }) else {
            panic!("prepared resources failed to load")
        };
    }

    #[test]
    #[ignore = "daily"]
    fn admitted_minecraft_and_custom_items_start_and_complete_timed_native_use() {
        use mod_api::{
            Hooks, Host, HostSide, ItemPart, ItemsEvent, LivingEntityUseItemFinishEvent, ModEntry,
            Mods, Registrar, RegistrationContext, UseDurationEvent,
        };
        use std::sync::{Arc, Mutex};
        use sunrise_gameplay::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};

        if run_fresh_cases(
            "cut_layout::game::server_side::item_use::interact_vibration_tests::admitted_minecraft_and_custom_items_start_and_complete_timed_native_use",
            "SUNRISE_MINECRAFT_ITEM_USE_CHILD",
            &["valid"],
        ) {
            return;
        }

        const MC_ITEM: &str = "minecraft:p3_04_wand";
        const ALT_ITEM: &str = "p3_04_use:wand";
        const BUILTIN_ITEM: &str = "minecraft:stick";

        #[derive(Clone)]
        struct TimedUseMod(Arc<Mutex<Vec<String>>>);
        #[mod_api::__subscriptions]
        impl Hooks for TimedUseMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                for id in [MC_ITEM, ALT_ITEM, BUILTIN_ITEM] {
                    let key = r.item_key(id);
                    r.item_part(key, self.clone());
                }
            }
            fn items(&self, _: &RegistrationContext, event: &mut ItemsEvent<'_>) {
                for id in [MC_ITEM, ALT_ITEM] {
                    event
                        .register(mod_api::Item::new(id, BUILTIN_ITEM))
                        .unwrap();
                }
            }
            fn living_entity_use_item_finish(
                &self,
                _: HostSide<'_>,
                event: &mut LivingEntityUseItemFinishEvent<'_>,
            ) {
                if let Some(id) = mod_api::__private::item_key_id(event.stack.item()) {
                    self.0.lock().unwrap().push(id.to_owned());
                }
            }
        }
        #[mod_api::sunrise_part]
        impl ItemPart for TimedUseMod {
            fn use_duration(&self, _: &dyn Host, event: &mut UseDurationEvent<'_>) {
                event.duration = 4;
            }
        }

        let finished = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "p3_04_use",
            Box::new(TimedUseMod(finished.clone())),
        )]
        .into();
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        with_game_at(root.prepared().to_path_buf(), |game| {
            game.options.dedicated = Some(Vec::new());
            // Built-in vanilla items keep their normal path even when use_duration is non-zero.
            let stick = ItemStack::new(BUILTIN_ITEM, 1);
            let selected = game.selected;
            game.inventory[selected] = Some(stick.clone());
            assert_eq!(game.use_duration(&stick), 4);
            assert!(!game.mod_use(&stick));
            assert!(!game.start_mod_use(&stick));
            assert!(game.using.is_none());

            for id in [MC_ITEM, ALT_ITEM] {
                let stack = ItemStack::new(id, 1);
                let selected = game.selected;
                game.inventory[selected] = Some(stack.clone());
                assert!(game.mod_use(&stack), "{id}");
                assert!(game.start_mod_use(&stack), "{id}");
                assert_eq!(game.using, Some((selected, 0)), "{id}");
                assert_eq!(game.use_duration, 4, "{id}");
                for ticks in 0..4 {
                    game.tick_mod_use(selected, ticks, &stack);
                }
                assert!(game.using.is_none(), "{id} should complete after 4 ticks");
            }
            assert_eq!(*finished.lock().unwrap(), [MC_ITEM, ALT_ITEM]);
        });
    }
}
use sunrise_gameplay_common::game::tables::GameItems as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::cake_eligibility::CakeEligibilityGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
