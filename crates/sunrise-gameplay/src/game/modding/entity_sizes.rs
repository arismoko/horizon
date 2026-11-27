//! `entity_size` (`EntityEvent.Size`): vanilla's `Entity.refreshDimensions` lets a listener
//! change the size it just computed, whenever the pose changes and from the constructor; a
//! player's standing, crouching and swimming candidates are also resolved before its motion, for
//! the pose fit tests.
//! Sunrise computes a mob's size from its state (`Mob::dims`) and keeps a player's in its body
//! (`PlayerMotion::refresh_dimensions`), so the hook is asked here instead: a pass tells it each
//! entity whose pose or vanilla size changed since it was last asked (or that was never asked,
//! the constructor's call), and keeps the answer on the entity (`pose::SizeAnswer`) for as long
//! as that pose and vanilla size hold. The server's mobs are asked before and after their ticks,
//! and its other entities (items, minecarts, boats, projectiles, armor stands, TNT, falling
//! blocks, orbs, rockets, tridents, displays, markers, interactions, end crystals...) at the same
//! passes, standing, each keeping the answer its box takes; a guest's body once per server tick,
//! with its candidates, before its motion; the client's own body for its candidates before its
//! tick and for its current pose after it.
//! A client's copy of any server entity (a mob, a minecart, an armor stand, a display...) asks
//! where a client's `refreshDimensions` runs: from its constructor, before `entity_constructing`;
//! after a synched value that refreshes it (`Entity.onSyncedDataUpdated(DATA_POSE)`, and a
//! kind's own: a baby flag, a cube's size, an armor stand's flags, an interaction's width);
//! and from its tick, where a mob applies a new `minecraft:scale`
//! (`LivingEntity.refreshDirtyAttributes`) and a shulker moves its lid
//! (`Shulker.onPeekAmountChange`). A mob keeps its answer, and other copies keep theirs on
//! their tracking row (`remote::Tracked`); the copy's box follows it.

use super::entity::{Entity as _, with_entity, with_entity_mut};
use crate::game::ServerGame;
use mod_api::Pose;

impl ServerGame<'_> {
    /// The server's mobs whose pose or size changed since they were last asked.
    pub(in crate::game) fn native_mob_sizes(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return;
        }
        for i in 0..self.common.mobs.len() {
            let m = &self.common.mobs[i];
            let (id, pose, vanilla, previous) = (m.id, m.pose(), m.vanilla_dims(), m.size_answer);
            if let Some(answer) = self.ask_entity_size(id, pose, vanilla, previous)
                && let Some(m) = self.common.mobs.get_mut(i)
                && m.id == id
            {
                m.size_answer = Some(answer);
            }
        }
    }

    /// The server's entities other than mobs whose vanilla size changed since they were last
    /// asked, or that were never asked (the constructor's call), standing.
    pub(in crate::game) fn native_entity_sizes(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return;
        }
        for handle in self.entity_handles() {
            let asked = with_entity!(self, handle, e => {
                e.vanilla_dimensions()
                    .map(|vanilla| (e.id(), vanilla, e.size_answer()))
            });
            if let Some((id, vanilla, previous)) = asked
                && let Some(answer) = self.ask_entity_size(id, Pose::Standing, vanilla, previous)
            {
                with_entity_mut!(self, handle, e => e.set_size_answer(answer));
            }
        }
        for i in 0..self.server.others.0.len() {
            let e = &self.server.others.0[i];
            let (id, previous) = (e.id, e.size_answer);
            let Some(vanilla) = e.vanilla_dimensions() else {
                continue;
            };
            if let Some(answer) = self.ask_entity_size(id, Pose::Standing, vanilla, previous) {
                self.server
                    .others
                    .0
                    .update(i, |e| e.size_answer = Some(answer));
            }
        }
        for i in 0..self.common.crystals.len() {
            let c = &self.common.crystals[i];
            // A crystal no server tracks yet has no entity id to name.
            let (id, previous) = (c.id, c.size_answer);
            if id == 0 {
                continue;
            }
            let vanilla = sunrise_gameplay_common::game::dragon_common::Crystal::DIMENSIONS;
            if let Some(answer) = self.ask_entity_size(id, Pose::Standing, vanilla, previous) {
                self.common
                    .crystals
                    .update(i, |c| c.size_answer = Some(answer));
            }
        }
    }

    /// Each guest's body on the server, once a tick.
    pub(in crate::game::server_side) fn native_guest_sizes(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return;
        }
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let asked: Vec<_> = host
            .hub
            .guests
            .as_slice()
            .iter()
            .filter_map(|guest| {
                let body = guest.motion.as_deref()?;
                let answer = self.ask_body_size(guest.entity_id as u32, body);
                Some((
                    guest.conn,
                    answer,
                    self.body_fit_sizes(guest.entity_id as u32, body, answer.or(body.size_answer)),
                ))
            })
            .collect();
        for (conn, answer, fit_sizes) in asked {
            if let Some(body) = self
                .server
                .lan
                .as_mut()
                .and_then(|host| host.hub.guests.motion_mut(conn))
            {
                if let Some(answer) = answer {
                    sunrise_gameplay_common::game::CommonGame::apply_body_size(body, answer);
                }
                body.pose_fit_sizes = fit_sizes;
            }
        }
    }
}

use sunrise_gameplay_common::game::entity_sizes_common::EntitySizesCommonGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{EntitySizeEvent, Hooks, Host, Pose};
    use std::sync::{Arc, Mutex};
    use sunrise_gameplay_common::game::{
        collision_source::CollisionSourceGameExt1 as _,
        entity_sizes_common::EntitySizesCommonGameExt1 as _,
    };

    pub(in crate::game) type Calls = Arc<Mutex<Vec<(Pose, [f32; 3], [f32; 3])>>>;

    pub(in crate::game) struct Grow(pub(in crate::game) Calls);

    #[mod_api::__subscriptions]
    impl Hooks for Grow {
        fn entity_size(&self, _: &dyn Host, event: &mut EntitySizeEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push((event.pose, event.old_size, *event.size));
            event.size[0] *= 2.0;
            event.size[1] *= 2.0;
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_listeners_size_sizes_the_mobs_box_until_vanilla_changes_it() {
        let calls = Calls::default();
        let mods = vec![mod_api::ModEntry::new(
            "grow",
            Box::new(Grow(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let id = game
                    .spawn_mob_spawned(
                        "cow",
                        [1.0, 70.0, 3.0],
                        sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                    )
                    .unwrap();
                let i = game.mobs.iter().position(|m| m.id == id).unwrap();
                let vanilla = game.mobs[i].vanilla_dims();
                game.native_mob_sizes();
                let grown = [vanilla[0] * 2.0, vanilla[1] * 2.0, vanilla[2]];
                let m = &game.mobs[i];
                assert_eq!((m.width(), m.height()), (grown[0], grown[1]));
                assert_eq!(m.eye_height(), f64::from(vanilla[2]));
                assert!((m.bounds().size().y - f64::from(grown[1])).abs() < 1e-9);
                // Asked once for a pose and vanilla size, from the constructor's.
                game.native_mob_sizes();
                assert_eq!(
                    calls.lock().unwrap().as_slice(),
                    &[(m_pose(game, i), vanilla, vanilla)]
                );
                // Grown down to a baby: asked again, from the size it had.
                sunrise_sim::mobs::finalize::set_baby(&mut game.mobs[i], true);
                game.native_mob_sizes();
                let calls = calls.lock().unwrap();
                assert_eq!(calls.len(), 2);
                assert_eq!(calls[1].1, grown);
                assert_eq!(calls[1].2, game.mobs[i].vanilla_dims());
            })
        });
    }

    pub(in crate::game) struct SetSize(pub(in crate::game) Arc<Mutex<[f32; 3]>>);

    #[mod_api::__subscriptions]
    impl Hooks for SetSize {
        fn entity_size(&self, _: &dyn Host, event: &mut EntitySizeEvent<'_>) {
            *event.size = *self.0.lock().unwrap();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn invalid_size_answers_leave_the_mobs_default_box() {
        let size = Arc::new(Mutex::new([1.0; 3]));
        let mods = vec![mod_api::ModEntry::new(
            "set_size",
            Box::new(SetSize(size.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let id = game
                    .spawn_mob_spawned(
                        "cow",
                        [1.0, 70.0, 3.0],
                        sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                    )
                    .unwrap();
                let i = game.mobs.iter().position(|m| m.id == id).unwrap();
                let vanilla = game.mobs[i].vanilla_dims();
                let default_dims = game.mobs[i].dims();
                for invalid in [
                    f32::NAN,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    -1.0,
                    32.001,
                    1e30,
                ] {
                    for axis in 0..3 {
                        let mut answer = vanilla;
                        answer[axis] = invalid;
                        *size.lock().unwrap() = answer;
                        game.mobs[i].size_answer = None;
                        game.native_mob_sizes();
                        let mob = &game.mobs[i];
                        assert_eq!(mob.size_answer.unwrap().effective(), vanilla, "{answer:?}");
                        assert_eq!(mob.dims(), default_dims, "{answer:?}");
                    }
                }
                // The reported crash input must never become a pathfinding extent.
                *size.lock().unwrap() = [1e30; 3];
                game.mobs[i].size_answer = None;
                game.native_mob_sizes();
                assert_eq!(game.mobs[i].dims(), default_dims);
                // Both endpoints are valid: zero-size entities are vanilla too.
                for accepted in [0.0, 32.0] {
                    *size.lock().unwrap() = [accepted; 3];
                    game.mobs[i].size_answer = None;
                    game.native_mob_sizes();
                    assert_eq!(
                        game.mobs[i].dims(),
                        (accepted.into(), accepted.into(), accepted.into())
                    );
                }
                *size.lock().unwrap() = [0.0; 3];
                game.mobs[i].size_answer = None;
                for _ in 0..4 {
                    game.native_mob_sizes();
                    let mob = &game.mobs[i];
                    let path_mob = mob.path_mob();
                    assert_eq!((path_mob.width, path_mob.height), (0.0, 0.0));
                    let world = game.mob_world_base(&[]);
                    assert!(world.no_collision(mob.bounds()));
                    use sunrise_sim::mobs::path::{Evaluator, Finder};
                    for evaluator in [Evaluator::Walk, Evaluator::Swim { breach: true }] {
                        let finder = Finder::new(&world, &path_mob, evaluator);
                        let _ = finder.walk_path_type(1, 70, 3);
                        let _ = finder.find(&[[3, 70, 3]], 8.0, 0, 32);
                    }
                }
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn partial_collision_sizes_complete_navigation() {
        // Resource loading and the shared mod fixture lock are not navigation.
        // Start the deadline only once the fixture is ready, while keeping it
        // outside the worker so a saturated collision loop still fails boundedly.
        let (ready, started) = std::sync::mpsc::channel();
        let (done, result) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let size = Arc::new(Mutex::new([0.0; 3]));
            let mods = vec![mod_api::ModEntry::new(
                "set_size",
                Box::new(SetSize(size.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    ready.send(()).unwrap();
                    use sunrise_sim::mobs::path::{Evaluator, Finder, PathType};
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                    let registry = &game.pack.assets.registry;
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    let fence = registry.default_state("minecraft:oak_fence").unwrap();
                    for x in 0..4 {
                        for z in 0..4 {
                            game.world.set(x, 69, z, stone).unwrap();
                        }
                    }
                    game.world.set(0, 70, 0, fence).unwrap();
                    // At the block corner, even subnormal widths remain represented
                    // in the box's x/z extents; the fence post is not on the route.
                    let id = game
                        .spawn_mob_spawned(
                            "cow",
                            [0.0, 70.0, 0.0],
                            sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                        )
                        .unwrap();
                    let i = game.mobs.iter().position(|m| m.id == id).unwrap();
                    let vanilla = game.mobs[i].vanilla_dims();
                    for answer in [[0.0; 3], [1e-30; 3], [1e-45; 3], vanilla] {
                        *size.lock().unwrap() = answer;
                        game.mobs[i].size_answer = None;
                        game.native_mob_sizes();
                        let mut mob = game.mobs[i].path_mob();
                        mob.on_ground = true;
                        let world = game.mob_world_base(&[]);
                        let finder = Finder::new(&world, &mob, Evaluator::Walk);
                        assert_eq!(finder.walk_path_type(0, 70, 0), PathType::Fence);
                        let path = finder.find(&[[2, 70, 0]], 8.0, 0, 32);
                        assert!(path.is_some(), "{answer:?}");
                    }
                    let id = game
                        .spawn_mob_spawned(
                            "silverfish",
                            [0.99, 70.0, 0.921591],
                            sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                        )
                        .unwrap();
                    let i = game.mobs.iter().position(|m| m.id == id).unwrap();
                    game.mobs[i].extra.model_scale = 0.0625;
                    *size.lock().unwrap() = game.mobs[i].vanilla_dims();
                    game.native_mob_sizes();
                    let mut mob = game.mobs[i].path_mob();
                    mob.on_ground = true;
                    assert!((mob.width - 0.025).abs() < 1e-8);
                    assert!((mob.height - 0.01875).abs() < 1e-8);
                    let world = game.mob_world_base(&[]);
                    let finder = Finder::new(&world, &mob, Evaluator::Walk);
                    assert_eq!(finder.walk_path_type(0, 70, 0), PathType::Fence);
                    assert!(finder.find(&[[-1, 70, 0]], 8.0, 0, 32).is_some());
                });
            });
            done.send(()).unwrap();
        });
        started.recv().expect("navigation fixture must initialize");
        result
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("partial-collision navigation must finish within 30 seconds");
        worker.join().unwrap();
    }

    pub(in crate::game) struct ShortCrouch(pub(in crate::game) Arc<std::sync::atomic::AtomicUsize>);

    #[mod_api::__subscriptions]
    impl Hooks for ShortCrouch {
        fn entity_size(&self, _: &dyn Host, event: &mut EntitySizeEvent<'_>) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if event.pose == Pose::Crouching {
                event.size[1] = event.size[0] / 0.6 * 0.8;
                event.size[2] = event.size[0];
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn pose_fit_uses_the_resolved_crouch_size_including_cloned_motion() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::{
            motion::{MotionWorld, PlayerMotion},
            pose::Pose as MotionPose,
        };
        for subscribed in [false, true] {
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mods: mod_api::Mods = if subscribed {
                vec![mod_api::ModEntry::new(
                    "short_crouch",
                    Box::new(ShortCrouch(calls.clone())),
                )]
                .into()
            } else {
                Vec::new().into()
            };
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(0, 65, 0, stone).unwrap();
                    let mut body =
                        PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap();
                    body.shift_key_down = true;
                    body.pose_fit_sizes = game.body_fit_sizes(1, &body, None);
                    assert_eq!(body.pose_fit_sizes.iter().any(Option::is_some), subscribed);
                    assert_eq!(
                        calls.load(std::sync::atomic::Ordering::Relaxed),
                        if subscribed { 3 } else { 0 }
                    );
                    body.size_answer = body.pose_fit_sizes[0];
                    body.pose_fit_sizes = game.body_fit_sizes(1, &body, body.size_answer);
                    assert_eq!(
                        calls.load(std::sync::atomic::Ordering::Relaxed),
                        if subscribed { 5 } else { 0 }
                    );
                    // Motion snapshots own their candidate answers, not a pointer into Game.
                    let mut restored = body.clone();
                    assert_eq!(restored.pose_fit_sizes, body.pose_fit_sizes);
                    body.pose_fit_sizes = [None; 3];
                    let side = game_side!(game);
                    let mut world = MotionWorld {
                        side: &side,
                        entities: &[],
                        player: &mut restored,
                        scene: &mut game.common.world,
                        assets: &game.common.pack.assets,
                        collision_binding: game.common.collision,
                        resources: game.common.motion,
                        health: &game.common.health_resources,
                        tag_items: None,
                        loop_context: &game.common.loop_context,
                        damage_hooks: &game.common.damage_hooks,
                        block_registries: Some(&game.common.pack.block_registries),
                    };
                    assert_eq!(
                        world.can_fit_pose(MotionPose::Crouching).unwrap(),
                        subscribed
                    );
                    world.update_local_crouching().unwrap();
                    assert_eq!(world.player.local_crouching, subscribed);
                    world.update_player_pose().unwrap();
                    assert_eq!(
                        world.player.pose,
                        if subscribed {
                            MotionPose::Crouching
                        } else {
                            MotionPose::Swimming
                        }
                    );
                    if subscribed {
                        assert_eq!(world.player.dimensions.height, 0.8);
                        // A scale change after prefill refuses the old candidate answer.
                        let key = world.player.attributes.key("minecraft:scale").unwrap();
                        let handle = world
                            .player
                            .attributes
                            .get_instance(Some(key))
                            .unwrap()
                            .unwrap();
                        world
                            .player
                            .attributes
                            .edit(&handle, |value, observer| {
                                value.set_base_value(0.9, observer)
                            })
                            .unwrap();
                        assert!(!world.can_fit_pose(MotionPose::Crouching).unwrap());
                    }
                })
            });
        }
    }

    pub(in crate::game::server_side) fn m_pose(
        game: &crate::game::ServerGame<'_>,
        i: usize,
    ) -> Pose {
        game.mobs[i].pose()
    }

    /// Each client-copy test boots the normal resource loader, which replaces process-wide
    /// tables (block namespaces, points of interest): run it in a fresh child process.
    pub(in crate::game) fn in_fresh_process(test: &str) -> bool {
        crate::game::native_entity_refs::tests::run_fresh_cases(
            &format!("game::server_side::entity_sizes::tests::{test}"),
            "SUNRISE_TEST_CLIENT_COPY_SIZE",
            &[test],
        )
    }

    /// What a client's listeners saw: each `entity_size` call as (entity, pose, old size, new
    /// size), and the order of the size and constructing calls.
    #[derive(Default)]
    pub(in crate::game) struct Seen {
        pub(in crate::game) sizes: Vec<(u32, Pose, [f32; 3], [f32; 3])>,
        pub(in crate::game) order: Vec<String>,
    }

    /// Records every call, and doubles the width and height.
    pub(in crate::game) struct Log(pub(in crate::game) Arc<Mutex<Seen>>);

    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn entity_size(&self, _: &dyn Host, event: &mut EntitySizeEvent<'_>) {
            let mut seen = self.0.lock().unwrap();
            let id = event.entity.id();
            seen.sizes
                .push((id, event.pose, event.old_size, *event.size));
            seen.order.push(format!("size {id}"));
            event.size[0] *= 2.0;
            event.size[1] *= 2.0;
        }
        fn entity_constructing(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::EntityConstructingEvent,
        ) {
            let kind = event.kind.to_string();
            self.0
                .lock()
                .unwrap()
                .order
                .push(format!("constructing {kind}"));
        }
    }

    /// A client logged in to a memory server, receiving that server's packets by their wire
    /// form, with `Log` listening; its chunk at the origin is loaded and empty.
    #[cfg(any())]
    pub(in crate::game) fn with_loopback_client(
        test: impl FnOnce(&mut crate::game::ClientGame<'_>, &Mutex<Seen>),
    ) {
        use {
            crate::game::native_client_session::tests::finish_login,
            crate::game::native_client_session::tests::logged_client,
            crate::game::native_entity_refs::tests::with_client_game,
        };
        let seen = Arc::new(Mutex::new(Seen::default()));
        let mods = vec![mod_api::ModEntry::new("log", Box::new(Log(seen.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let mut server =
                    sunrise_protocol::connection::Connection::local(logged_client(game));
                server
                    .reader
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                finish_login(game, &mut server);
                game.native_lifecycle.begin_open(None);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                test(game, &seen);
            });
        });
    }

    #[cfg(any())]
    pub(in crate::game::server_side) fn deliver(
        game: &mut crate::game::ClientGame<'_>,
        packet: sunrise_protocol::packets::play::Clientbound,
    ) {
        use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
        let body = packet.encode();
        let mut reader = Reader::new(&body);
        let id = reader.varint().unwrap();
        game.handle_packet(Clientbound::decode(id, reader.rest()).unwrap())
            .unwrap();
    }

    #[cfg(any())]
    pub(in crate::game) fn add_entity(
        game: &mut crate::game::ClientGame<'_>,
        id: i32,
        kind: &str,
        position: [f64; 3],
    ) {
        use sunrise_protocol::packets::play::{AddEntity, Clientbound};
        let kind = game
            .side
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id(kind)
            .unwrap();
        deliver(
            game,
            Clientbound::AddEntity(AddEntity {
                id,
                uuid: id as u128,
                kind,
                position,
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }),
        );
    }

    #[cfg(any())]
    pub(in crate::game) fn entity_data(
        game: &mut crate::game::ClientGame<'_>,
        id: i32,
        values: Vec<(u8, i32, sunrise_protocol::entity_data::Value)>,
    ) {
        use sunrise_protocol::packets::play::{Clientbound, SetEntityData};
        deliver(
            game,
            Clientbound::SetEntityData(SetEntityData { id, values }),
        );
    }

    #[cfg(any())]
    pub(in crate::game::server_side) fn scale(
        game: &mut crate::game::ClientGame<'_>,
        id: i32,
        base: f64,
    ) {
        use sunrise_protocol::packets::play::{AttributeSnapshot, Clientbound, UpdateAttributes};
        let attribute = game
            .motion
            .attributes
            .id("minecraft:scale")
            .unwrap()
            .registry_id();
        deliver(
            game,
            Clientbound::UpdateAttributes(UpdateAttributes {
                id,
                values: vec![AttributeSnapshot {
                    attribute,
                    base,
                    modifiers: Vec::new(),
                }],
            }),
        );
    }

    /// The listener's answer for `size`: width and height doubled.
    pub(in crate::game) fn doubled([width, height, eye]: [f32; 3]) -> [f32; 3] {
        [width * 2.0, height * 2.0, eye]
    }

    /// NeoForge fires `EntityEvent.Size` from the constructor before `EntityConstructing`: a
    /// client's new copy is sized by its type, standing, before the constructing listeners run.
    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn a_client_copy_is_sized_before_entity_constructing() {
        if in_fresh_process("a_client_copy_is_sized_before_entity_constructing") {
            return;
        }
        with_loopback_client(|game, seen| {
            add_entity(game, 31, "minecraft:cow", [8.5, 70.0, 8.5]);
            add_entity(game, 32, "minecraft:minecart", [4.5, 70.0, 4.5]);
            let seen = seen.lock().unwrap();
            assert_eq!(
                seen.order,
                [
                    "size 31",
                    "constructing minecraft:cow",
                    "size 32",
                    "constructing minecraft:minecart",
                ]
            );
            // EntityType.COW: 0.9 by 1.4, its eyes at 1.3; MINECART: 0.98 by 0.7, eyes 0.595.
            let (cow, cart) = ([0.9, 1.4, 1.3], [0.98, 0.7, 0.595]);
            assert_eq!(
                seen.sizes,
                [
                    (31, Pose::Standing, cow, cow),
                    (32, Pose::Standing, cart, cart),
                ]
            );
        });
    }

    /// `Entity.onSyncedDataUpdated(DATA_POSE)` → `refreshDimensions` on a client: a copy of a
    /// server villager is sized by its synched pose, whichever of the pose and the bed position
    /// arrives first, asleep and waking, and the same pose sent again asks nothing.
    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn a_client_copy_is_sized_by_its_synched_pose_not_its_bed() {
        use sunrise_protocol::entity_data::{Value, serializer};
        if in_fresh_process("a_client_copy_is_sized_by_its_synched_pose_not_its_bed") {
            return;
        }
        // EntityType.VILLAGER: 0.6 by 1.95, its eyes at 1.62; LivingEntity.SLEEPING_DIMENSIONS:
        // 0.2 by 0.2, eyes 0.2.
        let (standing, sleeping) = ([0.6, 1.95, 1.62], [0.2_f32; 3]);
        let pose = |id| (6, serializer::POSE, Value::Id(id));
        let bed = |at: Option<[i32; 3]>| (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(at));
        with_loopback_client(|game, seen| {
            for (id, bed_first) in [(66, true), (67, false)] {
                add_entity(game, id, "minecraft:villager", [8.5, 70.0, 8.5]);
                let calls = |seen: &Mutex<Seen>| {
                    let seen = seen.lock().unwrap();
                    let calls = seen.sizes.iter().filter(|call| call.0 == id as u32);
                    calls.copied().collect::<Vec<_>>()
                };
                let mut expected = vec![(id as u32, Pose::Standing, standing, standing)];
                if bed_first {
                    entity_data(game, id, vec![bed(Some([8, 70, 8]))]);
                    // The bed moves it but doesn't size it.
                    assert_eq!(calls(seen), expected, "bed first");
                    entity_data(game, id, vec![pose(2)]);
                } else {
                    entity_data(game, id, vec![pose(2)]);
                    entity_data(game, id, vec![bed(Some([8, 70, 8]))]);
                }
                expected.push((id as u32, Pose::Sleeping, doubled(standing), sleeping));
                assert_eq!(calls(seen), expected, "asleep, bed first {bed_first}");
                // The same pose again: no new size, no new call.
                entity_data(game, id, vec![pose(2)]);
                assert_eq!(calls(seen), expected, "asleep, same pose again");
                let m = game.mobs.iter().find(|m| m.id == id as u32).unwrap();
                assert_eq!((m.width(), m.height()), (0.4, 0.4));
                // Waking: the pose stands it up, before or after the bed is cleared.
                if bed_first {
                    entity_data(game, id, vec![bed(None)]);
                    assert_eq!(calls(seen), expected, "bed cleared first");
                    entity_data(game, id, vec![pose(0)]);
                } else {
                    entity_data(game, id, vec![pose(0)]);
                    entity_data(game, id, vec![bed(None)]);
                }
                expected.push((id as u32, Pose::Standing, doubled(sleeping), standing));
                assert_eq!(calls(seen), expected, "awake, bed first {bed_first}");
            }
        });
    }

    /// `LivingEntity.refreshDirtyAttributes` applies a new `minecraft:scale` at the entity's
    /// tick, so two scale updates before a client tick ask once, with the last scale and the
    /// size the copy had. Its box and F3+B outline then take the answer.
    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn a_client_copy_takes_a_new_scale_at_its_tick() {
        if in_fresh_process("a_client_copy_takes_a_new_scale_at_its_tick") {
            return;
        }
        with_loopback_client(|game, seen| {
            let at = [8.5, 70.0, 8.5];
            add_entity(game, 31, "minecraft:cow", at);
            let cow = [0.9, 1.4, 1.3];
            scale(game, 31, 1.5);
            scale(game, 31, 2.0);
            assert_eq!(
                seen.lock().unwrap().sizes.len(),
                1,
                "no call before the tick"
            );
            game.tick_remote_entities(true);
            let scaled = [0.9_f32 * 2.0, 1.4_f32 * 2.0, 1.3_f32 * 2.0];
            assert_eq!(
                seen.lock().unwrap().sizes,
                [
                    (31, Pose::Standing, cow, cow),
                    (31, Pose::Standing, doubled(cow), scaled),
                ]
            );
            game.tick_remote_entities(true);
            assert_eq!(seen.lock().unwrap().sizes.len(), 2, "the answer holds");
            let [width, height, _] = doubled(scaled);
            let m = game.mobs.iter().find(|m| m.id == 31).unwrap();
            let p = m.position;
            let half = f64::from(width) / 2.0;
            let min = [p.x - half, p.y, p.z - half];
            let max = [p.x + half, p.y + f64::from(height), p.z + half];
            let b = m.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            assert_eq!(([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]), (min, max));
            assert_eq!(white_outline(game), (min, max));
        });
    }

    /// `Shulker.onPeekAmountChange` → `refreshDimensions` as a floor shulker's lid opens on a
    /// client: each lid move asks again, and the copy's box keeps the answer as the lid moves.
    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn a_floor_shulkers_client_copy_asks_as_its_lid_moves() {
        use sunrise_protocol::entity_data::{Value, serializer};
        if in_fresh_process("a_floor_shulkers_client_copy_asks_as_its_lid_moves") {
            return;
        }
        with_loopback_client(|game, seen| {
            add_entity(game, 51, "minecraft:shulker", [8.5, 70.0, 8.5]);
            // Shulker.DATA_ATTACH_FACE_ID down, then DATA_PEEK_ID wide open.
            entity_data(
                game,
                51,
                vec![
                    (16, serializer::DIRECTION, Value::Id(0)),
                    (17, serializer::BYTE, Value::Byte(100)),
                ],
            );
            let mut heights = Vec::new();
            for _ in 0..4 {
                let before = seen.lock().unwrap().sizes.len();
                game.tick_remote_entities(true);
                let seen = seen.lock().unwrap();
                assert_eq!(seen.sizes.len(), before + 1, "one call per lid move");
                let (_, pose, _, vanilla) = *seen.sizes.last().unwrap();
                assert_eq!(pose, Pose::Standing);
                let m = game.mobs.iter().find(|m| m.id == 51).unwrap();
                assert_eq!(m.vanilla_dims(), vanilla);
                assert_eq!(m.height(), vanilla[1] * 2.0, "the lid's answer is applied");
                heights.push(vanilla[1]);
            }
            // Shulker.getDefaultDimensions: 1 + the lid's opening, rising.
            assert!(heights.windows(2).all(|h| h[0] < h[1]), "{heights:?}");
        });
    }

    /// A server game with `Log` listening (`seen`), or with no listener at all.
    pub(in crate::game::server_side) fn with_server(
        listen: bool,
        test: impl FnOnce(&mut crate::game::ServerGame<'_>, &Mutex<Seen>),
    ) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let mods: mod_api::Mods = if listen {
            vec![mod_api::ModEntry::new("log", Box::new(Log(seen.clone())))].into()
        } else {
            Vec::new().into()
        };
        mod_platform::testing::with_mods(mods, || with_game(|game| test(game, &seen)));
    }

    /// The box `EntityDimensions.makeBoundingBox` makes for `width` by `height` at `at`.
    pub(in crate::game::server_side) fn made_box(
        at: sunrise_core::geometry::Vec3,
        [width, height, _]: [f32; 3],
    ) -> [[f64; 3]; 2] {
        let half = f64::from(width / 2.0);
        [
            [at.x - half, at.y, at.z - half],
            [at.x + half, at.y + f64::from(height), at.z + half],
        ]
    }

    pub(in crate::game::server_side) fn corners(b: sunrise_core::geometry::Aabb) -> [[f64; 3]; 2] {
        let (lo, hi) = (b.min_position(), b.max_position());
        [[lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]]
    }

    /// The server asks for every family, not only mobs (`EntityEvent.Size` from each
    /// constructor): an item, two minecarts, an armor stand, a snowball and a block display,
    /// each from its type's size, standing; a leash knot, whose box doesn't follow its size,
    /// isn't asked. Their boxes take the answer, and so does what a minecart collides with
    /// (another cart's box, `Mover::Cart`). An armor stand turned small asks again; nothing
    /// asks twice for a size already answered.
    #[test]
    #[ignore = "daily"]
    fn server_entities_ask_and_their_boxes_take_the_answer() {
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay_common::game::collision_source::Mover;
        use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
        use sunrise_sim::entities::ArmorStand;
        use sunrise_sim::projectiles::{Projectile, ProjectileKind};
        if in_fresh_process("server_entities_ask_and_their_boxes_take_the_answer") {
            return;
        }
        with_server(true, |game, seen| {
            let (cart_at, other_at) = (Vec3::new(8.5, 70.0, 8.5), Vec3::new(10.0, 70.0, 8.5));
            game.carts
                .push(Minecart::new(9001, CartKind::Rideable, cart_at));
            game.carts
                .push(Minecart::new(9002, CartKind::Rideable, other_at));
            game.spawn_item(
                sunrise_items::items::ItemStack::new("minecraft:stone", 1),
                [4.5, 70.0, 4.5],
                [0.0; 3],
                0,
            );
            let item = game.dropped.last().unwrap().id;
            let stand_at = Vec3::new(2.5, 70.0, 6.5);
            game.armor_stands.push(ArmorStand::new(9004, stand_at, 0.0));
            let shot_at = Vec3::new(2.5, 70.0, 2.5);
            let mut shot = Projectile::new(ProjectileKind::Snowball, shot_at, None);
            shot.id = 9003;
            game.creatures.projectiles.push(shot);
            for (id, kind) in [
                (9005, "minecraft:leash_knot"),
                (9006, "minecraft:block_display"),
            ] {
                game.server
                    .others
                    .0
                    .push(crate::game::commands::OtherEntity {
                        size_answer: None,
                        attachment_lifetime: Default::default(),
                        knot_check: None,
                        eye: None,
                        join: Default::default(),
                        id,
                        kind: kind.into(),
                        position: Vec3::new(6.5, 70.0, 2.5),
                        yaw: 0.0,
                        pitch: 0.0,
                        display: None,
                        interaction: None,
                    });
            }
            game.native_entity_sizes();
            // EntityType.ITEM: 0.25 by 0.25, eyes 0.2125; MINECART: 0.98 by 0.7, eyes 0.595;
            // ARMOR_STAND: 0.5 by 1.975, eyes 1.7775; SNOWBALL: 0.25 by 0.25, eyes 0.2125.
            let (item_size, cart, stand) = (
                [0.25, 0.25, 0.2125],
                [0.98, 0.7, 0.595],
                [0.5, 1.975, 1.7775],
            );
            let standing = |id, size| (id, Pose::Standing, size, size);
            assert_eq!(
                seen.lock().unwrap().sizes,
                [
                    standing(item, item_size),
                    standing(9001, cart),
                    standing(9002, cart),
                    standing(9004, stand),
                    standing(9003, item_size),
                    // EntityType.BLOCK_DISPLAY: none. The knot (9005) isn't asked.
                    standing(9006, [0.0; 3]),
                ]
            );
            let cart_box = made_box(cart_at, doubled(cart));
            assert_eq!(corners(game.carts[0].bounds()), cart_box);
            let item_at = game.dropped.last().unwrap().position;
            assert_eq!(
                corners(game.dropped.last().unwrap().bounds()),
                made_box(item_at, doubled(item_size))
            );
            assert_eq!(
                corners(game.creatures.projectiles[0].bounds()),
                made_box(shot_at, doubled(item_size))
            );
            assert_eq!(
                corners(game.armor_stands[0].bounds()),
                made_box(stand_at, doubled(stand))
            );
            // The other cart moves against the first one's answered box.
            let other = game.carts[1].clone();
            let solids = game.entity_collisions(Mover::Cart(&other));
            assert!(solids.iter().any(|b| corners(*b) == cart_box), "{solids:?}");
            // The same sizes again: nothing to ask. ArmorStand.setSmall refreshes its size.
            game.native_entity_sizes();
            assert_eq!(seen.lock().unwrap().sizes.len(), 6);
            game.armor_stands.update(0, |a| a.small = true);
            game.native_entity_sizes();
            assert_eq!(
                seen.lock().unwrap().sizes.last(),
                Some(&(9004, Pose::Standing, doubled(stand), [0.25, 0.9875, 0.9875]))
            );
        });
    }

    /// A falling block's landing hurts what its box covers (`FallingBlockEntity.causeFallDamage`
    /// selects by `getBoundingBox`): an anvil whose answer doubled it hits a pig a block away,
    /// which its vanilla 0.98-wide box doesn't reach.
    #[test]
    #[ignore = "daily"]
    fn a_resized_falling_anvil_hits_what_its_answer_covers() {
        use sunrise_core::geometry::Vec3;
        if in_fresh_process("a_resized_falling_anvil_hits_what_its_answer_covers") {
            return;
        }
        with_server(true, |game, _| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let at = Vec3::new(8.5, 64.0, 8.5);
            let pig = game
                .spawn_mob_spawned(
                    "pig",
                    [9.5, 64.0, 8.5],
                    sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                )
                .unwrap();
            let health = game.mobs.iter().find(|m| m.id == pig).unwrap().health;
            game.common
                .falling
                .push(sunrise_gameplay_common::game::FallingBlock {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0; 2],
                    join: Default::default(),
                    id: 9101,
                    blocks_building: true,
                    state: game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:anvil")
                        .unwrap(),
                    start_pos: sunrise_core::position::BlockPos::containing(at),
                    position: at,
                    previous: at,
                    velocity: Vec3::ZERO,
                    age: 1,
                    no_gravity: false,
                    portal: Default::default(),
                    drop_item: true,
                    fall_distance: 2.0,
                    ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
                    fire_ticks: 0,
                    sync_velocity: false,
                    stuck: Vec3::ZERO,
                    damage: sunrise_sim::falling_block::Damage::anvil(),
                    saved: None,
                });
            game.native_entity_sizes();
            let mut anvil = game.falling[0].clone();
            game.falling.remove(0);
            game.falling_block_impact(&mut anvil);
            let hurt = game.mobs.iter().find(|m| m.id == pig).unwrap().health;
            assert!(hurt < health, "{hurt} < {health}");
        });
    }

    /// `AgeableMob.setAge` crossing to grown up refreshes the size (`DATA_BABY_ID` →
    /// `refreshDimensions`): a server calf that grows up in its tick asks once, from the size
    /// it had as a calf.
    #[test]
    #[ignore = "daily"]
    fn a_server_calf_asks_when_it_grows_up() {
        if in_fresh_process("a_server_calf_asks_when_it_grows_up") {
            return;
        }
        with_server(true, |game, seen| {
            crate::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
                game,
                sunrise_core::position::ChunkPos::new(0, 0),
            );
            let id = game
                .spawn_mob_spawned(
                    "cow",
                    [8.5, 70.0, 8.5],
                    sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                )
                .unwrap();
            let i = game.mobs.iter().position(|m| m.id == id).unwrap();
            sunrise_sim::mobs::finalize::set_baby(&mut game.mobs[i], true);
            let calf = game.mobs[i].vanilla_dims();
            // One tick from growing up (`AgeableMob.aiStep`: ++age).
            game.mobs[i].extra.age = -1;
            game.native_mob_sizes();
            game.tick_creature_entities().unwrap();
            let i = game.mobs.iter().position(|m| m.id == id).unwrap();
            assert!(!game.mobs[i].is_baby(), "the calf grew up in its tick");
            // EntityType.COW: 0.9 by 1.4, eyes 1.3.
            let cow = [0.9, 1.4, 1.3];
            let calls: Vec<_> = seen
                .lock()
                .unwrap()
                .sizes
                .iter()
                .filter(|call| call.0 == id)
                .copied()
                .collect();
            assert_eq!(
                calls,
                [
                    (id, Pose::Standing, calf, calf),
                    (id, Pose::Standing, doubled(calf), cow),
                ]
            );
            assert_eq!(game.mobs[i].height(), 1.4_f32 * 2.0);
        });
    }

    /// No listener, no call: the server's entities keep no answer and their vanilla boxes.
    #[test]
    #[ignore = "daily"]
    fn without_a_listener_the_server_asks_nothing() {
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
        if in_fresh_process("without_a_listener_the_server_asks_nothing") {
            return;
        }
        with_server(false, |game, _| {
            let at = Vec3::new(8.5, 70.0, 8.5);
            game.carts.push(Minecart::new(9001, CartKind::Rideable, at));
            game.native_mob_sizes();
            game.native_entity_sizes();
            assert_eq!(game.carts[0].size_answer, None);
            assert_eq!(
                corners(game.carts[0].bounds()),
                made_box(at, [0.98, 0.7, 0.0])
            );
        });
    }
}
