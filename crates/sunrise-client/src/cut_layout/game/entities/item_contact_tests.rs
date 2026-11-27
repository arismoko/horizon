use mod_api::{
    self, BlockPart, Flow, Hooks, HostSide, ItemAfterVerticalCollisionEvent, Registrar,
    RegistrationContext,
};
use std::sync::Arc;
use std::sync::Mutex;
use sunrise_core::geometry::Vec3;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::world_items;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_level::world::World;
use sunrise_sim::entities::ItemEntity;
use sunrise_sim::join_seq::Joined as _;

type Contact = (bool, u32, [i32; 3], [f64; 3], u32);

#[derive(Clone, Default)]
struct Probe {
    contacts: Arc<Mutex<Vec<Contact>>>,
}

#[mod_api::sunrise_part]
impl BlockPart for Probe {
    fn item_after_vertical_collision(
        &self,
        _: HostSide,
        event: &mut ItemAfterVerticalCollisionEvent<'_>,
    ) -> Flow {
        let stack = event.stack.as_ref().expect("active item stack");
        self.contacts.lock().unwrap().push((
            event.client,
            event.entity,
            event.block_pos,
            event.item_pos,
            stack.count(),
        ));
        if stack.count() == 1 {
            return Flow::Cancel;
        }
        *event.stack = event.stack.take().unwrap().shrink(2);
        Flow::Continue
    }
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.block_key("minecraft:stone");
        r.block_part(key, self.clone());
    }
}

#[test]
#[ignore = "daily"]
fn the_active_stack_survives_a_partial_contact_and_consumption_removes_the_entity() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::item_contact_tests::the_active_stack_survives_a_partial_contact_and_consumption_removes_the_entity",
        the_active_stack_survives_a_partial_contact_and_consumption_removes_the_entity_isolated,
    );
}

fn the_active_stack_survives_a_partial_contact_and_consumption_removes_the_entity_isolated() {
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    let prepare = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
        game.options.dedicated = Some(Vec::new());
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        game.world = World::new(game.res.facts().air, 1);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(4, 63, 4, stone).unwrap();
    };

    // The centre of the item is outside the block, while its box overlaps it.
    let landing = [3.99, 64.0, 4.5];
    let mut input = ItemStack::new("minecraft:iron_ingot", 13);
    input
        .data_mut()
        .set_other("fixture:opaque", Some(sunrise_core::nbt::Tag::Long(99)));
    let spawn = |game: &mut sunrise_gameplay::game::ServerGame<'_>, count| {
        game.dropped.clear();
        let mut stack = input.clone();
        stack.count = count;
        game.spawn_item(stack, [landing[0], 64.1, landing[2]], [0.0, -0.3, 0.0], 20);
        game.server.ticked_entities_through = game.server.next_entity;
        game.dropped[0].clone()
    };
    let mut vanilla = None;
    with_game(|game| {
        prepare(game);
        let before = spawn(game, 13);
        game.tick_items();
        let item = game.dropped[0].clone();
        assert_eq!(item.id, before.id);
        assert_eq!(item.position, Vec3::new(landing[0], landing[1], landing[2]));
        assert_eq!(item.velocity, Vec3::ZERO);
        assert_eq!(item.stack, input);
        assert_eq!(item.age, 1);

        vanilla = Some(item);
    });
    let vanilla = vanilla.unwrap();

    let probe = Probe::default();
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("contact", Box::new(probe.clone()))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            prepare(game);
            let old = spawn(game, 13);
            game.tick_items();
            let item = &game.dropped[0];
            assert_eq!(item.id, old.id);
            assert_eq!(item.join_seq(), old.join_seq());
            assert_eq!(item.position, vanilla.position);
            assert_eq!(item.velocity, vanilla.velocity);
            let mut expected = input.clone();
            expected.count = 11;
            assert_eq!(item.stack, expected);
            assert_eq!(
                probe.contacts.lock().unwrap().as_slice(),
                &[(false, old.id, [4, 63, 4], landing, 13)]
            );

            for count in [1, 2] {
                spawn(game, count);
                game.tick_items();
                assert!(game.dropped.is_empty(), "consumed stack of {count}");
            }
        });
    });
}

type BounceContact = (bool, i32, String, [i32; 3]);

#[derive(Clone, Default)]
struct BounceProbe {
    calls: Arc<Mutex<Vec<BounceContact>>>,
    bounced: Arc<Mutex<Vec<(i32, [i32; 3])>>>,
}

#[mod_api::sunrise_part]
impl BlockPart for BounceProbe {
    fn bounce_restitution(
        &self,
        level: mod_api::LevelSide<'_>,
        event: &mut mod_api::BounceRestitutionEvent<'_>,
    ) {
        let (client, host) = match level {
            mod_api::LevelSide::Client(host) => (true, host),
            mod_api::LevelSide::Server(host) => (false, host),
        };
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        assert_eq!(host.block_state(pos), Some(event.state));
        self.calls
            .lock()
            .unwrap()
            .push((client, event.entity, event.kind.into(), pos));
        event.factor = 0.67_f32;
    }

    fn bounce_on(
        &self,
        _: &mut dyn mod_api::LevelRandomHost,
        event: &mut mod_api::BounceOnEvent<'_>,
    ) {
        self.bounced
            .lock()
            .unwrap()
            .push((event.entity, [event.pos.x, event.pos.y, event.pos.z]));
    }
}

#[mod_api::__subscriptions]
impl Hooks for BounceProbe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for block in [
            "minecraft:glass",
            "minecraft:white_carpet",
            "minecraft:honey_block",
        ] {
            let key = r.block_key(block);
            r.block_part(key, self.clone());
        }
    }
}

#[cfg(feature = "client")]
fn client_item(
    id: u32,
    stack: ItemStack,
    position: [f64; 3],
    velocity: [f64; 3],
    pickup_delay: i32,
) -> ItemEntity {
    let at = Vec3::new(position[0], position[1], position[2]);
    ItemEntity {
        size_answer: None,
        attachment_lifetime: Default::default(),
        rotation: [0.0, 0.0],
        join: Default::default(),
        health: 5,
        fire_ticks: 0,
        id,
        stack,
        thrower: None,
        target: None,
        position: at,
        previous: at,
        velocity: Vec3::new(velocity[0], velocity[1], velocity[2]),
        ground: Default::default(),
        fall_distance: 0.0,
        age: 0,
        lifespan: ItemEntity::LIFETIME,
        pickup_delay,
        stack_revision: 0,
        bob_offset: 0.0,
        ticks: 0,
        needs_sync: false,
        stuck: Vec3::ZERO,
        was_touching_water: false,
        no_gravity: false,
        vertical_collision: false,
        random: sunrise_core::random::LegacyRandom::new(0),
    }
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn item_restitution_uses_the_legacy_contact_on_both_simulating_sides() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::item_contact_tests::item_restitution_uses_the_legacy_contact_on_both_simulating_sides",
        || {
            use {
                crate::game::server_side::native_entity_refs::tests::with_client_game,
                sunrise_gameplay::game::native_entity_refs::tests::with_game,
            };
            let part = BounceProbe::default();
            let mods: mod_api::Mods =
                vec![mod_api::ModEntry::new("bounce", Box::new(part.clone()))].into();
            mod_platform::testing::with_mods(mods.clone(), || {
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                sunrise_gameplay_common::game::native_hooks::install_block_contact_hooks();
                fn run<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
                    game: &mut G,
                    client: bool,
                    part: &BounceProbe,
                    mut make_item: impl FnMut(&mut G, [f64; 3], [f64; 3]) -> ItemEntity,
                ) {
                    let registry = game.pack.assets.registry;
                    let block = |name| registry.default_state(name).unwrap();
                    let glass = block("minecraft:glass");
                    let carpet = block("minecraft:white_carpet");
                    let stone = block("minecraft:stone");
                    let slime = block("minecraft:slime_block");
                    let honey = block("minecraft:honey_block");
                    let cases = [
                        // The item's centre lies outside the block; its moved footprint does not.
                        (
                            glass,
                            None,
                            None,
                            3.99,
                            64.25,
                            -0.96,
                            64.0,
                            0.5278801619529518_f64,
                            true,
                        ),
                        // The carpet supports the box, but getOnPosLegacy addresses the glass below it.
                        (
                            glass,
                            Some(carpet),
                            None,
                            4.5,
                            64.3125,
                            -0.96,
                            64.0625,
                            0.5278801619529518_f64,
                            true,
                        ),
                        (
                            stone,
                            Some(carpet),
                            None,
                            4.5,
                            64.3125,
                            -0.96,
                            64.0625,
                            0.0_f64,
                            false,
                        ),
                        (
                            slime,
                            None,
                            None,
                            4.5,
                            64.25,
                            -0.96,
                            64.0,
                            0.7878808308504466_f64,
                            false,
                        ),
                        (
                            honey, None, None, 4.5, 64.25, -0.96, 63.9375, 0.0_f64, false,
                        ),
                        (glass, None, None, 4.5, 64.03125, 0.0, 64.0, 0.0_f64, false),
                        // A ceiling collision addresses the carpet cell but never queries its factor.
                        (
                            stone,
                            Some(carpet),
                            Some(glass),
                            4.5,
                            64.5,
                            1.0,
                            64.75,
                            -0.0_f64,
                            false,
                        ),
                    ];
                    for (floor, above, ceiling, x, y, vy, landed_y, expected_y, calls) in cases {
                        game.world = World::new(game.res.facts().air, 1);
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                        game.world.set(4, 63, 4, floor).unwrap();
                        if let Some(above) = above {
                            game.world.set(4, 64, 4, above).unwrap();
                        }
                        if let Some(ceiling) = ceiling {
                            game.world.set(4, 65, 4, ceiling).unwrap();
                        }
                        game.dropped.clear();
                        part.calls.lock().unwrap().clear();
                        part.bounced.lock().unwrap().clear();
                        game.persist.sync_body_position.clear();
                        let mut item = make_item(game, [x, y, 4.5], [0.0, vy, 0.0]);
                        let dim = game.dim.id();
                        let result =
                            sunrise_sim::block_contact::in_level(game.tick, Some(dim), || {
                                game.tick_item_with_block_contacts(
                                    &mut item,
                                    None,
                                    &[],
                                    client,
                                    None,
                                    &mut |_| {},
                                )
                            });
                        assert_eq!(result, (true, false));
                        assert_eq!(item.position.y, landed_y);
                        assert_eq!(
                            item.velocity.y.to_bits(),
                            expected_y.to_bits(),
                            "floor {floor:?}, start {y}"
                        );
                        assert_eq!(
                            game.persist.sync_body_position.contains(&item.id),
                            !client && expected_y > 0.0
                        );
                        let bounced = part.bounced.lock().unwrap();
                        if !client && calls {
                            assert_eq!(*bounced, [(item.id as i32, [4, 63, 4])]);
                        } else {
                            assert!(
                                bounced.is_empty(),
                                "unexpected bounce callback: {bounced:?}"
                            );
                        }
                        let got = part.calls.lock().unwrap();
                        if calls {
                            assert_eq!(
                                *got,
                                [(
                                    client,
                                    item.id as i32,
                                    "minecraft:item".to_owned(),
                                    [4, 63, 4]
                                )]
                            );
                        } else {
                            assert!(got.is_empty(), "unexpected query: {got:?}");
                        }
                    }
                }
                with_game(|game| {
                    run(game, false, &part, |game, pos, vel| {
                        game.spawn_item(ItemStack::new("minecraft:iron_ingot", 1), pos, vel, 20);
                        game.dropped[0].clone()
                    });
                });
                let mut next_id = 1_u32;
                with_client_game(|game| {
                    run(game, true, &part, |_game, pos, vel| {
                        let id = next_id;
                        next_id += 1;
                        client_item(id, ItemStack::new("minecraft:iron_ingot", 1), pos, vel, 20)
                    });
                });
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn tnt_cart_fall_dispatches_restitution_only_if_the_cart_survives() {
    use {
        sunrise_gameplay_common::game::minecarts::CartKind,
        sunrise_gameplay_common::game::minecarts::Minecart,
    };
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::item_contact_tests::tnt_cart_fall_dispatches_restitution_only_if_the_cart_survives",
        || {
            let part = BounceProbe::default();
            let mods: mod_api::Mods =
                vec![mod_api::ModEntry::new("bounce", Box::new(part.clone()))].into();
            mod_platform::testing::with_mods(mods.clone(), || {
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                sunrise_gameplay_common::game::native_hooks::install_block_contact_hooks();
                sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                    let glass = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:glass")
                        .unwrap();
                    use mod_api::MoveCause;
                    use sunrise_core::geometry::Aabb;
                    use sunrise_gameplay_common::game::piston_push::Lid;

                    for mover in [
                        None,
                        Some(MoveCause::SelfMovement),
                        Some(MoveCause::Piston),
                        Some(MoveCause::ShulkerBox),
                    ] {
                        for (id, fall, survives) in [(71, 0.0, true), (72, 3.5, false)] {
                            game.world = World::new(game.res.facts().air, 1);
                            game.world
                                .chunks
                                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                            game.world.set(4, 63, 4, glass).unwrap();
                            game.carts.clear();
                            let mut cart =
                                Minecart::new(id, CartKind::Tnt, Vec3::new(4.5, 64.25, 4.5));
                            cart.fall_distance = fall;
                            cart.velocity = Vec3::new(0.0, -1.0, 0.0);
                            game.carts.push(cart);
                            part.calls.lock().unwrap().clear();
                            let mut tick_cart = None;
                            sunrise_sim::block_contact::in_level(
                                game.tick,
                                Some(game.dim.id()),
                                || {
                                    match mover {
                                        None => {
                                            // The tick moves a clone of its joined resident row.
                                            let mut cart = game.carts[0].clone();
                                            assert_eq!(
                                                game.move_cart(
                                                    &mut cart,
                                                    Vec3::new(0.0, -0.5, 0.0)
                                                ),
                                                survives
                                            );
                                            tick_cart = Some(cart);
                                        }
                                        Some(MoveCause::ShulkerBox) => game.lid_push(
                                            Lid::Box,
                                            Aabb::new(
                                                Vec3::new(4.0, 64.5, 4.0),
                                                Vec3::new(5.0, 65.0, 5.0),
                                            ),
                                            Vec3::new(0.0, -0.5, 0.0),
                                        ),
                                        Some(cause) => {
                                            let entity = game.entity(id).unwrap();
                                            assert!(
                                                entity
                                                    .move_by(game, [0.0, -0.5, 0.0], cause)
                                                    .is_some()
                                            );
                                        }
                                    }
                                },
                            );
                            assert_eq!(
                                game.carts.iter().any(|cart| cart.id == id),
                                survives,
                                "{mover:?}"
                            );
                            let calls = part.calls.lock().unwrap();
                            if survives {
                                let cart = tick_cart.as_ref().unwrap_or(&game.carts[0]);
                                assert_eq!(cart.position.y.to_bits(), 64.0_f64.to_bits());
                                assert_eq!(
                                    *calls,
                                    [(
                                        false,
                                        id as i32,
                                        "minecraft:tnt_minecart".to_owned(),
                                        [4, 63, 4]
                                    )]
                                );
                                assert!(cart.velocity.y > 0.0);
                            } else {
                                assert!(calls.is_empty(), "{mover:?}");
                            }
                        }
                    }
                });
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn item_support_survives_idle_then_clears_on_an_airborne_move() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::item_contact_tests::item_support_survives_idle_then_clears_on_an_airborne_move",
        || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let air = game.res.facts().air;
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let support = BlockPos::new(4, 63, 4);
                game.world = World::new(air, 1);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.world
                    .set(support.x, support.y, support.z, stone)
                    .unwrap();
                game.spawn_item(
                    ItemStack::new("minecraft:iron_ingot", 1),
                    [4.5, 64.25, 4.5],
                    [1.0, -0.96, 0.0],
                    20,
                );
                let mut item = game.dropped[0].clone();
                let tick = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                            item: &mut ItemEntity| {
                    game.tick_item_with_block_contacts(item, None, &[], false, None, &mut |_| {})
                };
                assert_eq!(tick(game, &mut item), (true, false));
                assert_eq!(item.position, Vec3::new(5.5, 64.0, 4.5));
                // Only the old horizontal footprint had a supporting block.
                assert!(item.ground.on_ground);
                assert_eq!(item.ground.supporting_block, Some(support));

                game.world
                    .set(support.x, support.y, support.z, air)
                    .unwrap();
                item.velocity = Vec3::ZERO;
                // The next tick must not be one of the four-tick movement updates.
                item.ticks = item.id.wrapping_neg() & 3;
                assert_eq!(tick(game, &mut item), (true, false));
                assert_eq!(item.position, Vec3::new(5.5, 64.0, 4.5));
                assert_eq!(item.ground.supporting_block, Some(support));

                item.velocity = Vec3::new(0.1, 0.2, 0.0);
                assert_eq!(tick(game, &mut item), (true, false));
                assert!(item.position.y > 64.0);
                assert!(!item.ground.on_ground);
                assert_eq!(item.ground.supporting_block, None);
            });
        },
    );
}

#[cfg(feature = "client")]
type StepCall = (bool, i32, [i32; 3], bool);

#[cfg(feature = "client")]
#[derive(Clone, Default)]
struct StepProbe {
    calls: Arc<Mutex<Vec<StepCall>>>,
}

#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for StepProbe {
    fn step_on(&self, level: mod_api::LevelRandomSide<'_>, event: &mut mod_api::StepOnEvent<'_>) {
        let (client, host): (bool, &dyn mod_api::LevelHost) = match level {
            mod_api::LevelRandomSide::Client(host) => (true, host),
            mod_api::LevelRandomSide::Server(host) => (false, host),
        };
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        assert_eq!(host.block_state(pos), Some(event.state));
        let first = {
            let mut calls = self.calls.lock().unwrap();
            let first = !calls.iter().any(|call| call.1 == event.entity);
            calls.push((client, event.entity, pos, event.stepping_carefully));
            first
        };
        event.movement[0] = 0.25;
        if !client {
            let write = match event.kind {
                "minecraft:armor_stand" => mod_api::EntityWrite::Push([0.25, 0.0, 0.0]),
                "minecraft:item" => mod_api::EntityWrite::StuckIn([0.5, 0.25, 0.5]),
                "minecraft:oak_boat" if first => mod_api::EntityWrite::SetFireTicks(40),
                _ => return,
            };
            assert!(event.write_entity(write));
        }
    }

    fn entity_inside(
        &self,
        _: &mut dyn mod_api::LevelRandomHost,
        event: &mut mod_api::EntityInsideEvent<'_>,
    ) {
        // A later inside write wins over the step; settle then drags this movement.
        event.run_after(mod_api::InsideEffect::Freeze, |view| {
            view.set_velocity([0.5, 0.0, 0.0]);
        });
    }

    fn item_after_vertical_collision(
        &self,
        _: HostSide<'_>,
        event: &mut ItemAfterVerticalCollisionEvent<'_>,
    ) -> Flow {
        if event.stack.as_ref().is_some_and(|stack| stack.count() == 1) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for StepProbe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        let cobweb = r.block_key("minecraft:cobweb");
        r.block_part(stone, self.clone());
        r.block_part(cobweb, self.clone());
    }
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn step_on_edits_grounded_bodies_before_inside_effects_and_settle() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::item_contact_tests::step_on_edits_grounded_bodies_before_inside_effects_and_settle",
        || {
            use sunrise_sim::motion::{MotionWorld, PlayerMotion};
            use {
                crate::game::server_side::native_entity_refs::tests::with_client_game,
                sunrise_gameplay::game::native_entity_refs::tests::with_game,
            };

            let part = StepProbe::default();
            let mods: mod_api::Mods =
                vec![mod_api::ModEntry::new("step", Box::new(part.clone()))].into();
            mod_platform::testing::with_mods(mods.clone(), || {
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                sunrise_gameplay_common::game::native_hooks::install_block_contact_hooks();
                fn run_common<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
                    game: &mut G,
                    client: bool,
                    part: &StepProbe,
                    mut spawn: impl FnMut(&mut G, u32, f64) -> ItemEntity,
                ) {
                    let registry = game.pack.assets.registry;
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    let web = registry.default_state("minecraft:cobweb").unwrap();
                    let air = game.res.facts().air;
                    let floor = [4, 63, 4];
                    game.world = World::new(air, 1);
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.world.set(4, 63, 4, stone).unwrap();
                    if !client {
                        game.world.set(4, 64, 4, web).unwrap();
                    }
                    let tick = |game: &mut G, item: &mut ItemEntity, escape| {
                        sunrise_sim::block_contact::in_level(
                            sunrise_sim::parallel::TickThread::mint(),
                            Some("minecraft:overworld"),
                            || {
                                game.tick_item_with_block_contacts(
                                    item,
                                    escape,
                                    &[],
                                    client,
                                    None,
                                    &mut |_| {},
                                )
                            },
                        )
                    };
                    let clear_calls = || std::mem::take(&mut *part.calls.lock().unwrap());
                    let mut item = spawn(game, 2, 64.25);
                    assert_eq!(tick(game, &mut item, None), (true, false));
                    assert_eq!(item.position.y, 64.0);
                    let settled = if client { 0.25 } else { 0.5 } * f64::from(0.98_f32 * 0.6_f32);
                    assert_eq!(item.velocity.x, settled);
                    assert_eq!(clear_calls(), [(client, item.id as i32, floor, false)]);

                    game.world.set(4, 64, 4, air).unwrap();
                    item.velocity = Vec3::ZERO;
                    item.ticks = item.id.wrapping_neg() & 3;
                    assert_eq!(tick(game, &mut item, None), (true, false));
                    assert_eq!(item.position, Vec3::new(4.5, 64.0, 4.5));
                    assert_eq!(item.velocity.x, 0.25); // no movement drag on an idle tick
                    assert_eq!(clear_calls(), [(client, item.id as i32, floor, false)]);
                    assert_eq!(tick(game, &mut item, None), (true, false));
                    let displacement = if client { 0.25 } else { 0.125 };
                    assert_eq!(item.position.x, 4.5 + displacement);
                    assert_eq!(clear_calls(), [(client, item.id as i32, floor, false)]);
                    if !client {
                        // The actual next move spent the preceding multiplier and
                        // the stone Part installed a fresh one on its own step.
                        assert_eq!(item.stuck, Vec3::new(0.5, 0.25, 0.5));
                    }

                    assert_eq!(tick(game, &mut item, Some(0.1)), (true, false));
                    let mut airborne = spawn(game, 2, 69.0);
                    assert_eq!(tick(game, &mut airborne, None), (true, false));
                    let mut consumed = spawn(game, 1, 64.25);
                    assert_eq!(tick(game, &mut consumed, None), (false, true));
                    assert!(clear_calls().is_empty());

                    let position = Vec3::new(4.5, 64.0, 4.5);
                    let mut player = PlayerMotion::new(game.motion, position).unwrap();
                    player.entity_id = 77;
                    player.ground.on_ground = true;
                    player.shift_key_down = true;
                    let (common, state) =
                        sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
                    let level = common.native_lifecycle.epoch().and_then(|epoch| {
                        let key = common.dim.key();
                        let id = mod_api::__private::dimension_id(key)?;
                        Some(mod_api::LevelRef::with_generation(
                            epoch,
                            key,
                            sunrise_gameplay_common::game::Side::levels_generation(state, id),
                        ))
                    });
                    let side = sunrise_gameplay_common::game_side!(
                        common,
                        (sunrise_gameplay_common::game::Side::lan_acting_uuid(state), level, false),
                        sunrise_gameplay_common::game::side_ops::tables::SideOps::side_level_features(state, common.level.enabled_features.as_arc()),
                        sunrise_gameplay_common::game::side_ops::tables::SideOps::side_game_tag_items(state, &common.res)
                    );
                    let mut motion = MotionWorld {
                        side: &side,
                        entities: &[],
                        player: &mut player,
                        scene: &mut common.world,
                        assets: &common.pack.assets,
                        collision_binding: common.collision,
                        resources: common.motion,
                        health: &common.health_resources,
                        loop_context: &common.loop_context,
                        damage_hooks: &common.damage_hooks,
                        tag_items: None,
                        block_registries: Some(&common.pack.block_registries),
                    };
                    if client {
                        motion.apply_effects_from_blocks().unwrap();
                    } else {
                        motion.server_stuck_in_blocks(position).unwrap();
                        assert!(clear_calls().is_empty());
                        motion.server_block_damage(position).unwrap();
                    }
                    assert_eq!(player.velocity.x, 0.25);
                    assert_eq!(clear_calls(), [(client, 77, floor, true)]);
                }
                with_game(|game| {
                    run_common(game, false, &part, |game, count, height| {
                        game.dropped.clear();
                        game.spawn_item(
                            ItemStack::new("minecraft:iron_ingot", count),
                            [4.5, height, 4.5],
                            [0.0, -0.96, 0.0],
                            20,
                        );
                        game.dropped[0].clone()
                    });
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    let floor = [4, 63, 4];
                    let clear_calls = || std::mem::take(&mut *part.calls.lock().unwrap());
                    game.server
                        .tickets
                        .0
                        .update(Some(sunrise_sim::ticket_levels::Sources {
                            forced: vec![(0, 0)],
                            ..Default::default()
                        }));
                    let mut stand =
                        sunrise_sim::entities::ArmorStand::new(78, Vec3::new(4.5, 64.25, 4.5), 0.0);
                    stand.velocity.y = -0.5;
                    game.armor_stands.push(stand);
                    game.tick_armor_stands().unwrap();
                    assert_eq!(game.armor_stands[0].position.y, 64.0);
                    assert_eq!(game.armor_stands[0].velocity.x, 0.5);
                    assert_eq!(clear_calls(), [(false, 78, floor, false)]);

                    use {
                        sunrise_gameplay::game::native_entity_refs::tests::bind_acting_guest,
                        sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
                    };
                    let seat = Vec3::new(8.5, 64.0, 4.5);
                    game.world.set(8, 63, 4, stone).unwrap();
                    game.player = Some(PlayerMotion::new(game.motion, seat).unwrap());
                    bind_local_player(game, 80, 1);
                    game.server.lan_acting = Some(2);
                    game.server.lan_acting_name = Some("boat_step".into());
                    bind_acting_guest(game);
                    game.server.lan_acting = None;
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.vehicle = Some(79);
                        });
                    let mut boat = sunrise_gameplay_common::game::boats_common::Boat::new(
                        79,
                        "oak".into(),
                        seat,
                        0.0,
                    );
                    boat.ground.on_ground = true;
                    game.boats.push(boat);
                    game.tick_boats();
                    assert_eq!(game.boats[0].fire_ticks, 40);
                    assert_eq!(clear_calls(), [(false, 79, [8, 63, 4], false); 2]);
                });
                let mut next_id = 1_u32;
                with_client_game(|game| {
                    run_common(game, true, &part, |_game, count, height| {
                        let id = next_id;
                        next_id += 1;
                        client_item(
                            id,
                            ItemStack::new("minecraft:iron_ingot", count),
                            [4.5, height, 4.5],
                            [0.0, -0.96, 0.0],
                            20,
                        )
                    });
                });
            });
        },
    );
}

#[cfg(feature = "client")]
#[derive(Clone, Default)]
struct StepWrites(
    Arc<Mutex<Vec<[[f64; 3]; 2]>>>,
    Option<mod_api::DamageTypeKey>,
);

#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for StepWrites {
    fn step_on(&self, level: mod_api::LevelRandomSide<'_>, event: &mut mod_api::StepOnEvent<'_>) {
        self.0.lock().unwrap().push(event.bounds);
        let hurt = mod_api::EntityWrite::Hurt {
            damage_type: self.1.unwrap(),
            amount: 0.5,
        };
        match level {
            mod_api::LevelRandomSide::Server(level) => {
                let state = level.block_state_id("minecraft:glass").unwrap();
                assert!(event.write_entity(hurt));
                assert!(event.request(mod_api::LandingWrite::SetBlock(state)));
            }
            mod_api::LevelRandomSide::Client(_) => {
                assert!(!event.write_entity(hurt));
                assert!(!event.request(mod_api::LandingWrite::DestroyBlock { drop: false }));
            }
        }
    }
}

#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for StepWrites {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        let generic = r.damage_type_key("minecraft:generic");
        r.block_part(stone, Self(self.0.clone(), Some(generic)));
    }
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn step_on_hurts_the_stepping_body_and_guards_its_block_writes() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::item_contact_tests::step_on_hurts_the_stepping_body_and_guards_its_block_writes",
        || {
            use {
                crate::game::server_side::native_entity_refs::tests::with_client_game,
                sunrise_gameplay::game::native_entity_refs::tests::with_game,
            };
            let part = StepWrites::default();
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "step_writes",
                Box::new(part.clone()),
            )]
            .into();
            mod_platform::testing::with_mods(mods.clone(), || {
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                sunrise_gameplay_common::game::native_hooks::install_block_contact_hooks();
                let run = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                           client,
                           change_first| {
                    let registry = game.pack.assets.registry;
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    let glass = registry.default_state("minecraft:glass").unwrap();
                    let dirt = registry.default_state("minecraft:dirt").unwrap();
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.world.set(4, 63, 4, stone).unwrap();
                    game.dropped.clear();
                    game.spawn_item(
                        ItemStack::new("minecraft:iron_ingot", 2),
                        [4.5, 64.25, 4.5],
                        [0.0, -0.96, 0.0],
                        20,
                    );
                    let mut item = game.dropped[0].clone();
                    let before = item.health;
                    sunrise_sim::block_contact::in_level(game.tick, Some(game.dim.id()), || {
                        assert_eq!(
                            game.tick_item_with_block_contacts(
                                &mut item,
                                None,
                                &[],
                                client,
                                None,
                                &mut |_| {}
                            ),
                            (true, false)
                        );
                    });
                    assert_eq!(item.health, before - i32::from(!client));
                    assert_eq!(
                        game.world.block(4, 63, 4),
                        stone,
                        "block write stays queued"
                    );
                    if change_first {
                        game.world.set(4, 63, 4, dirt).unwrap();
                    }
                    game.drain_entity_inside();
                    let kept = if change_first {
                        dirt
                    } else if client {
                        stone
                    } else {
                        glass
                    };
                    assert_eq!(game.world.block(4, 63, 4), kept);
                    if !client && !change_first {
                        game.world.set(4, 63, 4, stone).unwrap();
                        let mut stack = ItemStack::new("minecraft:shulker_box", 1);
                        let contents: Vec<_> = (0..27)
                            .map(|slot| {
                                (
                                    slot,
                                    ItemStack::new("minecraft:diamond", u32::from(slot) + 1),
                                )
                            })
                            .collect();
                        stack.data_mut().container = contents.clone();
                        game.dropped.clear();
                        game.spawn_item(stack, [4.5, 64.25, 4.5], [0.0, -0.96, 0.0], 20);
                        game.dropped[0].health = 1;
                        let mut item = game.dropped.remove(0);
                        sunrise_sim::block_contact::in_level(
                            game.tick,
                            Some(game.dim.id()),
                            || {
                                assert_eq!(
                                    game.tick_item_with_block_contacts(
                                        &mut item,
                                        None,
                                        &[],
                                        false,
                                        None,
                                        &mut |_| {}
                                    ),
                                    (false, true),
                                );
                            },
                        );
                        assert_eq!(game.dropped.len(), contents.len());
                        for (drop, (_, stack)) in game.dropped.iter().zip(contents) {
                            assert_eq!(drop.stack, stack);
                            assert_eq!(drop.position, item.position);
                        }
                        game.drain_entity_inside();
                    }
                };
                with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    run(game, false, false);
                    run(game, false, true);
                });
                with_client_game(|game| {
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.world.set(4, 63, 4, stone).unwrap();
                    let mut item = client_item(
                        1,
                        ItemStack::new("minecraft:iron_ingot", 2),
                        [4.5, 64.25, 4.5],
                        [0.0, -0.96, 0.0],
                        20,
                    );
                    let before = item.health;
                    sunrise_sim::block_contact::in_level(game.tick, Some(game.dim.id()), || {
                        assert_eq!(
                            game.tick_item_with_block_contacts(
                                &mut item,
                                None,
                                &[],
                                true,
                                None,
                                &mut |_| {}
                            ),
                            (true, false)
                        );
                    });
                    assert_eq!(item.health, before);
                    assert_eq!(game.world.block(4, 63, 4), stone);
                });
                assert_eq!(
                    *part.0.lock().unwrap(),
                    [[[4.375, 64.0, 4.375], [4.625, 64.25, 4.625]]; 4]
                );
            });
        },
    );
}

type LivingStep = (i32, Option<bool>);

#[derive(Clone, Default)]
struct LivingFloor(Arc<Mutex<Vec<LivingStep>>>, Option<mod_api::DamageTypeKey>);

#[mod_api::sunrise_part]
impl BlockPart for LivingFloor {
    fn step_on(&self, level: mod_api::LevelRandomSide<'_>, event: &mut mod_api::StepOnEvent<'_>) {
        let mod_api::LevelRandomSide::Server(level) = level else {
            return;
        };
        let living = level.entity_type_is_living(event.kind);
        self.0.lock().unwrap().push((event.entity, living));
        if living == Some(true) && !event.stepping_carefully {
            // MagmaBlock.stepOn hurts with the hot floor damage type.
            event.write_entity(mod_api::EntityWrite::Hurt {
                damage_type: self.1.unwrap(),
                amount: 1.0,
            });
        }
    }
}

#[mod_api::__subscriptions]
impl Hooks for LivingFloor {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        let hot_floor = r.damage_type_key("minecraft:hot_floor");
        r.block_part(stone, Self(self.0.clone(), Some(hot_floor)));
    }
}

#[test]
#[ignore = "daily"]
fn step_on_can_hurt_only_living_bodies_on_the_same_block() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::item_contact_tests::step_on_can_hurt_only_living_bodies_on_the_same_block",
        || {
            let part = LivingFloor::default();
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "living_floor",
                Box::new(part.clone()),
            )]
            .into();
            mod_platform::testing::with_mods(mods.clone(), || {
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                sunrise_gameplay_common::game::native_hooks::install_block_contact_hooks();
                sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(4, 63, 4, stone).unwrap();
                    game.server.tickets.0.update(None);

                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    let mut pig =
                        sunrise_sim::mobs::Mob::new(77, kind, Vec3::new(4.5, 64.0, 4.5), 0.0);
                    pig.no_ai = true;
                    pig.ground.on_ground = true;
                    let pig_health = pig.health;
                    game.mobs.push(pig);
                    game.spawn_item(
                        ItemStack::new("minecraft:iron_ingot", 1),
                        [4.5, 64.25, 4.5],
                        [0.0, -0.96, 0.0],
                        20,
                    );
                    let mut item = game.dropped.remove(0);
                    let item_health = item.health;

                    sunrise_sim::block_contact::in_level(game.tick, Some(game.dim.id()), || {
                        game.tick_creature_entities().unwrap();
                        assert_eq!(
                            game.tick_item_with_block_contacts(
                                &mut item,
                                None,
                                &[],
                                false,
                                None,
                                &mut |_| {}
                            ),
                            (true, false)
                        );
                    });
                    let seen = part.0.lock().unwrap();
                    assert!(seen.contains(&(77, Some(true))), "pig contact: {seen:?}");
                    assert!(
                        seen.contains(&(item.id as i32, Some(false))),
                        "item contact: {seen:?}"
                    );
                    assert_eq!(game.mobs[0].health, pig_health - 1.0);
                    assert_eq!(item.health, item_health);
                });
            });
        },
    );
}

const ACCEPT_BEFORE: u8 = 0;
const REJECT: u8 = 1;
const ACCEPT_AFTER: u8 = 2;

#[derive(Clone, Copy)]
struct StepAnswer<const ORDER: u8>(Option<mod_api::DamageTypeKey>);

#[mod_api::sunrise_part]
impl<const ORDER: u8> BlockPart for StepAnswer<ORDER> {
    fn step_on(&self, level: mod_api::LevelRandomSide<'_>, event: &mut mod_api::StepOnEvent<'_>) {
        use mod_api::{EntityWrite, LandingWrite};
        let mod_api::LevelRandomSide::Server(level) = level else {
            return;
        };
        match ORDER {
            ACCEPT_BEFORE => {
                event.movement[0] = 0.25;
                let push = if event.pos.x == 9 { f64::MAX } else { 0.5 };
                event.write_entity(EntityWrite::Push([push, 0.0, 0.0]));
                event.request(LandingWrite::SetBlock(
                    level.block_state_id("minecraft:glass").unwrap(),
                ));
            }
            REJECT => {
                assert_eq!(event.movement[0], 0.25);
                event.movement[0] = 3.0;
                event.write_entity(EntityWrite::Hurt {
                    damage_type: self.0.unwrap(),
                    amount: 10.0,
                });
                event.request(LandingWrite::DestroyBlock { drop: false });
                match event.pos.x {
                    4 => {
                        let unknown = mod_api::__private::block_state_from_ordinal(
                            event.state.block(),
                            u16::MAX,
                        );
                        event.request(LandingWrite::SetBlock(unknown));
                    }
                    5 => {
                        event.write_entity(EntityWrite::SetMovement([f64::INFINITY, 0.0, 0.0]));
                    }
                    6 => {
                        event.write_entity(EntityWrite::Push([f64::NAN, 0.0, 0.0]));
                    }
                    8 => {
                        event.write_entity(EntityWrite::Push([f64::MAX, 0.0, 0.0]));
                        event.write_entity(EntityWrite::Push([f64::MAX, 0.0, 0.0]));
                    }
                    9 => {
                        event.write_entity(EntityWrite::Push([f64::MAX, 0.0, 0.0]));
                    }
                    _ => event.movement[0] = f64::NAN,
                }
            }
            ACCEPT_AFTER => {
                assert_eq!(event.movement[0], 0.25);
                event.movement[0] += 0.75;
                event.write_entity(EntityWrite::Push([0.5, 0.0, 0.0]));
            }
            _ => unreachable!(),
        }
    }
}

#[mod_api::__subscriptions]
impl<const ORDER: u8> Hooks for StepAnswer<ORDER> {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        let generic = r.damage_type_key("minecraft:generic");
        r.block_part(stone, Self(Some(generic)));
    }
}

#[test]
#[ignore = "daily"]
fn step_on_refuses_only_the_invalid_part_outputs() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::item_contact_tests::step_on_refuses_only_the_invalid_part_outputs",
        || {
            let mods: mod_api::Mods = vec![
                mod_api::ModEntry::new("a_before", Box::new(StepAnswer::<ACCEPT_BEFORE>(None))),
                mod_api::ModEntry::new("b_invalid", Box::new(StepAnswer::<REJECT>(None))),
                mod_api::ModEntry::new("c_after", Box::new(StepAnswer::<ACCEPT_AFTER>(None))),
            ]
            .into();
            mod_platform::testing::with_mods(mods.clone(), || {
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                sunrise_gameplay_common::game::native_hooks::install_block_contact_hooks();
                sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let registry = game.pack.assets.registry;
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    let glass = registry.default_state("minecraft:glass").unwrap();
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    for x in 4..=9 {
                        game.world.set(x, 63, 4, stone).unwrap();
                        game.spawn_item(
                            ItemStack::new("minecraft:iron_ingot", 1),
                            [f64::from(x) + 0.5, 64.25, 4.5],
                            [0.0, -0.96, 0.0],
                            20,
                        );
                        let mut item = game.dropped.remove(0);
                        sunrise_sim::block_contact::in_level(
                            game.tick,
                            Some(game.dim.id()),
                            || {
                                assert_eq!(
                                    game.tick_item_with_block_contacts(
                                        &mut item,
                                        None,
                                        &[],
                                        false,
                                        None,
                                        &mut |_| {}
                                    ),
                                    (true, false),
                                );
                            },
                        );
                        assert_eq!(item.health, 5);
                        let written = if x == 9 { f64::MAX } else { 2.0 };
                        assert_eq!(item.velocity.x, written * f64::from(0.98_f32 * 0.6_f32));
                        assert_eq!(game.world.block(x, 63, 4), stone);
                        game.drain_entity_inside();
                        assert_eq!(game.world.block(x, 63, 4), glass);
                    }
                });
            });
        },
    );
}

struct ContactHeat(mod_api::DamageTypeKey);

#[mod_api::sunrise_part]
impl BlockPart for ContactHeat {
    fn step_on(&self, _: mod_api::LevelRandomSide<'_>, event: &mut mod_api::StepOnEvent<'_>) {
        event.write_entity(mod_api::EntityWrite::Hurt {
            damage_type: self.0,
            amount: 40.0,
        });
    }
}

struct HeatedBlocks;

#[mod_api::__subscriptions]
impl Hooks for HeatedBlocks {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for (block, damage_type) in [
            ("minecraft:stone", "minecraft:hot_floor"),
            ("minecraft:glass", "test:stove_burn"),
            ("minecraft:terracotta", "minecraft:fall"),
        ] {
            let block = r.block_key(block);
            let damage_type = r.damage_type_key(damage_type);
            r.block_part(block, ContactHeat(damage_type));
        }
    }
}

fn with_heated_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("heat", Box::new(HeatedBlocks))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        sunrise_gameplay_common::game::native_hooks::install_block_contact_hooks();
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            test(game);
        });
    });
}

#[test]
#[ignore = "daily"]
fn contact_hurt_uses_the_world_damage_type_tags_and_death_message() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::item_contact_tests::contact_hurt_uses_the_world_damage_type_tags_and_death_message",
        || {
            use sunrise_gameplay::game::native_entity_refs::tests::fixture_root;
            with_heated_game(|game| {
                for (id, x, kind, block) in [
                    (71, 4, "minecraft:blaze", "minecraft:stone"),
                    (72, 7, "minecraft:pig", "minecraft:stone"),
                    (73, 10, "minecraft:pig", "minecraft:glass"),
                    (74, 4, "minecraft:zombie", "minecraft:stone"),
                    (75, 7, "minecraft:cat", "minecraft:terracotta"),
                    (76, 10, "minecraft:pig", "minecraft:terracotta"),
                ] {
                    let z = if id <= 73 { 4 } else { 8 };
                    let support = game.pack.assets.registry.default_state(block).unwrap();
                    game.world.set(x, 63, z, support).unwrap();
                    let mut mob = sunrise_sim::mobs::Mob::new(
                        id,
                        game.res.mob_types().get(kind).unwrap().clone(),
                        Vec3::new(f64::from(x) + 0.5, 64.0, f64::from(z) + 0.5),
                        0.0,
                    );
                    mob.no_ai = true;
                    mob.ground.on_ground = true;
                    mob.velocity.y = -0.08;
                    if id == 74 {
                        let mut boots = ItemStack::new("minecraft:leather_boots", 1);
                        boots.enchant("minecraft:frost_walker", 1);
                        assert!(
                            mob.set_equipment_stack(sunrise_items::equipment::FEET, Some(boots))
                        );
                    }
                    game.mobs.push(mob);
                }
                let health = |game: &sunrise_gameplay::game::ServerGame<'_>, id| {
                    game.mobs.iter().find(|mob| mob.id == id).unwrap().health
                };
                game.tick_mobs().unwrap();
                assert_eq!(health(game, 71), 20.0, "hot floor is fire damage");
                assert_eq!(health(game, 72), 0.0, "the same hit kills a non-immune pig");
                assert_eq!(
                    health(game, 73),
                    10.0,
                    "no generic fallback for a missing type"
                );
                assert_eq!(health(game, 74), 20.0, "Frost Walker stops hot floor");
                assert_eq!(health(game, 75), 10.0, "cats are immune to fall damage");
                assert_eq!(health(game, 76), 0.0, "fall damage kills the untagged pig");

                let files = [(
                    "damage_type/test:stove_burn".into(),
                    vec![
                        br#"{"message_id":"stove_burn","scaling":"never","exhaustion":0.0}"#
                            .to_vec(),
                    ],
                )];
                game.health_resources = game
                    .health_resources
                    .with_packs(
                        &fixture_root(),
                        &sunrise_world::loot::PackOverlay::new(&files),
                        false,
                    )
                    .unwrap();
                game.tick_mobs().unwrap();
                let dead = game.mobs.iter().find(|mob| mob.id == 73).unwrap().clone();
                assert_eq!(dead.health, 0.0);
                assert_eq!(
                    game.mob_death_message(&dead, None).component()["translate"],
                    "death.attack.stove_burn"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn contact_hurt_uses_the_player_creative_and_frost_walker_bypass_tag() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::item_contact_tests::contact_hurt_uses_the_player_creative_and_frost_walker_bypass_tag",
        || {
            use sunrise_gameplay::game::native_entity_refs::tests::fixture_root;
            use sunrise_sim::motion::{MotionWorld, PlayerMotion};
            use sunrise_world::loot::PackOverlay;

            with_heated_game(|game| {
                let registry = game.pack.assets.registry;
                game.world
                    .set(4, 63, 4, registry.default_state("minecraft:glass").unwrap())
                    .unwrap();
                let at = Vec3::new(4.5, 64.0, 4.5);
                let mut boots = ItemStack::new("minecraft:leather_boots", 1);
                boots.enchant("minecraft:frost_walker", 1);
                for (bypass, creative, wears_boots, health) in [
                    (false, false, true, 20.0),
                    (false, false, false, 0.0),
                    (true, true, true, 0.0),
                ] {
                    let mut files = vec![
                        (
                            "damage_type/test:stove_burn".into(),
                            vec![br#"{"message_id":"stove_burn","scaling":"never","exhaustion":0.0}"#.to_vec()],
                        ),
                        (
                            "tags/minecraft:damage_type/burn_from_stepping".into(),
                            vec![br#"{"values":["test:stove_burn"]}"#.to_vec()],
                        ),
                    ];
                    if bypass {
                        files.push((
                            "tags/minecraft:damage_type/bypasses_invulnerability".into(),
                            vec![br#"{"values":["test:stove_burn"]}"#.to_vec()],
                        ));
                    }
                    game.health_resources = game
                        .health_resources
                        .with_packs(&fixture_root(), &PackOverlay::new(&files), false)
                        .unwrap();
                    let mut items = world_items!(game).clone();
                    Arc::make_mut(&mut items.enchantments)
                        .bind_damage_catalog(&game.health_resources.damage);
                    game.server_items = Some(Arc::new(items));
                    let resources = game.motion;
                    let new_player = || {
                        let mut player = PlayerMotion::new(resources, at).unwrap();
                        player.ground.on_ground = true;
                        player.invulnerable = creative;
                        player
                    };
                    game.player = Some(new_player());
                    // Equip or remove the boots and hit immediately, without a body tick.
                    game.armor[3] = wears_boots.then(|| boots.clone());
                    game.hurt_player_by("test:stove_burn", 40.0);
                    assert_eq!(
                        game.player.as_ref().unwrap().health.health(),
                        health,
                        "direct hit, boots: {wears_boots}, bypass tag: {bypass}"
                    );
                    game.player = Some(new_player());
                    let (common, state) =
                        sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
                    let items = match (
                        &common.server_items,
                        sunrise_gameplay_common::game::Side::world_tables(state),
                    ) {
                        (Some(items), _) => &**items,
                        (None, Some(tables)) => &tables.items,
                        (None, None) => common.res.items(),
                    };
                    let level = common.native_lifecycle.epoch().and_then(|epoch| {
                        let key = common.dim.key();
                        let id = mod_api::__private::dimension_id(key)?;
                        Some(mod_api::LevelRef::with_generation(
                            epoch,
                            key,
                            sunrise_gameplay_common::game::Side::levels_generation(state, id),
                        ))
                    });
                    let side = sunrise_gameplay_common::game_side!(
                        common,
                        (sunrise_gameplay_common::game::Side::lan_acting_uuid(state), level, false),
                        sunrise_gameplay_common::game::side_ops::tables::SideOps::side_level_features(state, common.level.enabled_features.as_arc()),
                        sunrise_gameplay_common::game::side_ops::tables::SideOps::side_game_tag_items(state, &common.res)
                    );
                    let mut world = MotionWorld {
                        player: common.player.as_mut().unwrap(),
                        scene: &mut common.world,
                        assets: &common.pack.assets,
                        collision_binding: common.collision,
                        resources: common.motion,
                        health: &common.health_resources,
                        tag_items: Some(items),
                        loop_context: &common.loop_context,
                        side: &side,
                        damage_hooks: &common.damage_hooks,
                        block_registries: Some(&common.pack.block_registries),
                        entities: &[],
                    };
                    sunrise_sim::block_contact::in_level(
                        common.tick,
                        Some("minecraft:overworld"),
                        || {
                            world.apply_effects_from_blocks().unwrap();
                        },
                    );
                    assert_eq!(
                        world.player.health.health(),
                        health,
                        "step-on hit, boots: {wears_boots}, bypass tag: {bypass}"
                    );
                }
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn contact_hurt_uses_the_orbs_level_fall_immunity_tag() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::item_contact_tests::contact_hurt_uses_the_orbs_level_fall_immunity_tag",
        || {
            use sunrise_items::items::EntityTypeTags;
            use sunrise_sim::{entities::ExperienceOrb, join_seq::JoinSeq};
            with_heated_game(|game| {
                let block = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:terracotta")
                    .unwrap();
                game.world.set(4, 63, 4, block).unwrap();
                game.server.tickets.0.update(None);
                for tagged in [false, true] {
                    let mut items = world_items!(game).clone();
                    let mut tags = items.entity_type_tags.ordered().clone();
                    let members = tags
                        .entry("minecraft:fall_damage_immune".into())
                        .or_default();
                    members.retain(|id| id != "minecraft:experience_orb");
                    if tagged {
                        members.push("minecraft:experience_orb".into());
                    }
                    items.set_entity_type_tags(EntityTypeTags::from_ordered(
                        tags,
                        items.entity_type_tags.ids().clone(),
                    ));
                    game.server_items = Some(Arc::new(items));
                    game.orbs.push(ExperienceOrb::new(
                        17,
                        Vec3::new(4.5, 64.25, 4.5),
                        Vec3::new(0.0, -0.5, 0.0),
                        1,
                    ));
                    game.server.ordered.announced = JoinSeq::MAX;
                    game.begin_ordered_ticks();
                    sunrise_sim::block_contact::in_level(
                        game.tick,
                        Some("minecraft:overworld"),
                        || {
                            game.run_ordered_ticks(JoinSeq::MAX, false).unwrap();
                        },
                    );
                    if tagged {
                        assert_eq!(game.orbs.len(), 1);
                        assert_eq!(game.orbs[0].position.y.to_bits(), 64.0_f64.to_bits());
                        assert_eq!(game.orbs[0].health, 5);
                    } else {
                        assert!(game.orbs.is_empty());
                    }
                }
            });
        },
    );
}

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
