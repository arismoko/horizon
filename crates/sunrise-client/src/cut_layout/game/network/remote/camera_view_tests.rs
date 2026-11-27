//! #3948: the shader-pack uniforms tied to the view read the camera entity (the entity a
//! spectator's click or the server's `SetCamera` attached the view to), and the client keeps the
//! other living entities' effects for it (`ClientPacketListener.handleUpdateMobEffect`).
#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use std::sync::Arc;
use sunrise_client::{gfx::pack_uniforms::ViewEntity, level_loading::Reason, options_file::V};
use sunrise_core::geometry::Vec3;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C, SpawnInfo},
};
use sunrise_sim::motion::PlayerMotion;

/// The local player's entity id.
pub(crate) const OWN: i32 = 7;
/// A zombie, living.
pub(crate) const ZOMBIE: i32 = 31;
/// An oak boat, not living.
pub(crate) const BOAT: i32 = 40;

pub(crate) fn spawn(dimension: &str) -> SpawnInfo {
    SpawnInfo {
        dimension_type: 0,
        dimension: dimension.into(),
        seed: 0,
        game_type: 3,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    }
}

/// A spectator client in the overworld, holding a zombie at (10, 64, 12) and an oak boat.
pub(crate) fn with_spectator(
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "CameraViewTest").unwrap();
        game.enter_remote_dimension(
            &spawn("minecraft:overworld"),
            sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry::Login,
            Reason::Other,
        );
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = OWN;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        add(game, ZOMBIE, "minecraft:zombie", [10.0, 64.0, 12.0]);
        add(game, BOAT, "minecraft:oak_boat", [-6.5, 64.0, 3.5]);
        assert_eq!(game.mobs.len(), 1);
        assert_eq!(game.boats.len(), 1);
        test(game);
    });
}

pub(crate) fn add(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    kind: &str,
    position: [f64; 3],
) {
    let kind = registry_id(game, "minecraft:entity_type", kind);
    game.handle_packet(C::AddEntity(play::AddEntity {
        id,
        uuid: id as u128,
        kind,
        position,
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    }))
    .unwrap();
}

pub(crate) fn registry_id(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    registry: &str,
    name: &str,
) -> i32 {
    let remote = game.client.remote.as_ref().unwrap();
    remote.builtin.get(registry).unwrap().id(name).unwrap()
}

pub(crate) fn update(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    effect: &str,
    duration: i32,
    flags: i8,
) {
    let effect = registry_id(game, "minecraft:mob_effect", effect);
    game.handle_packet(C::UpdateMobEffect(play::UpdateMobEffect {
        id,
        effect,
        amplifier: 0,
        duration,
        flags,
    }))
    .unwrap();
}

pub(crate) fn remove(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    effect: &str,
) {
    let effect = registry_id(game, "minecraft:mob_effect", effect);
    game.handle_packet(C::RemoveMobEffect(play::RemoveMobEffect { id, effect }))
        .unwrap();
}

pub(crate) fn set_camera(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, camera_id: i32) {
    game.handle_packet(C::SetCamera(play::SetCamera { camera_id }))
        .unwrap();
}

/// The view's values but its light, read at the partial tick 0.25.
pub(crate) fn view(game: &sunrise_client_gameplay::game::ClientGame<'_>) -> ViewEntity {
    ViewEntity {
        eye_light: [0; 2],
        ..game.pack_view_entity(glam::DVec3::new(0.5, 65.62, 0.5), 0.25)
    }
}

/// The effects the client keeps for the entity `id` (a mob, another player or an armor stand),
/// `None` when the level holds no such entity or it has none.
pub(crate) fn kept(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
) -> Option<Vec<(&'static str, i32)>> {
    let id = id as u32;
    let list = match game.mobs.iter().find(|m| m.id == id) {
        Some(m) => &m.extra.client_effects.list,
        None => {
            &game
                .armor_stands
                .iter()
                .find(|a| a.id == id)?
                .client_effects
                .list
        }
    };
    (!list.is_empty()).then(|| list.iter().map(|e| (e.id, e.duration)).collect())
}

pub(crate) const PLAYER_VIEW: ViewEntity = ViewEntity {
    living: true,
    blindness: 1.0,
    night_vision: 0.0,
    darkness: 0.0,
    mood: 0.375,
    eye_light: [0; 2],
};

/// The other entities' effects are kept one of a kind, for living entities the level holds,
/// and go with the entity: when it is removed, added again under its id, or the level is
/// replaced; a respawn in the same level keeps them.
#[test]
#[ignore = "daily"]
fn other_entities_effects_go_with_the_entity() {
    with_spectator(|game| {
        // Not held, or not an effect the registry names: nothing.
        update(game, 99, "minecraft:speed", 100, 0);
        assert_eq!(kept(game, 99), None);
        game.handle_packet(C::UpdateMobEffect(play::UpdateMobEffect {
            id: ZOMBIE,
            effect: 9_999,
            amplifier: 0,
            duration: 100,
            flags: 0,
        }))
        .unwrap();
        assert_eq!(kept(game, ZOMBIE), None);
        // forceAddEffect: the second of a kind replaces the first.
        update(game, ZOMBIE, "minecraft:speed", 100, 0);
        update(game, ZOMBIE, "minecraft:speed", 30, 0);
        update(game, ZOMBIE, "minecraft:glowing", -1, 0);
        assert_eq!(
            kept(game, ZOMBIE),
            Some(vec![("speed", 30), ("glowing", -1)])
        );
        // The player's own effects are not the zombie's.
        assert!(game.potions.effects.list.is_empty());
        remove(game, ZOMBIE, "minecraft:speed");
        assert_eq!(kept(game, ZOMBIE), Some(vec![("glowing", -1)]));

        game.handle_packet(C::RemoveEntities(play::RemoveEntities {
            ids: vec![ZOMBIE],
        }))
        .unwrap();
        assert_eq!(kept(game, ZOMBIE), None);

        add(game, ZOMBIE, "minecraft:zombie", [10.0, 64.0, 12.0]);
        update(game, ZOMBIE, "minecraft:speed", 100, 0);
        add(game, ZOMBIE, "minecraft:zombie", [10.0, 64.0, 12.0]);
        assert_eq!(
            kept(game, ZOMBIE),
            None,
            "ClientLevel.addEntity removes it first"
        );

        // An armor stand is living.
        add(game, 50, "minecraft:armor_stand", [2.0, 64.0, 2.0]);
        update(game, 50, "minecraft:slowness", 100, 0);
        assert_eq!(kept(game, 50), Some(vec![("slowness", 100)]));
        // The view on the stand reads its effects, and RemoveMobEffect takes them off it.
        update(game, 50, "minecraft:blindness", 100, 0);
        set_camera(game, 50);
        assert_eq!(view(game).blindness, 1.0);
        remove(game, 50, "minecraft:blindness");
        assert_eq!(view(game).blindness, 0.0);
        assert_eq!(kept(game, 50), Some(vec![("slowness", 100)]));
        set_camera(game, OWN);

        update(game, ZOMBIE, "minecraft:speed", 100, 0);
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:overworld"),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(kept(game, ZOMBIE), Some(vec![("speed", 100)]));
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:the_nether"),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(kept(game, ZOMBIE), None);
        assert_eq!(kept(game, 50), None);
    });
}

/// Puts `light` (`sky << 4 | block`) at one block, every other block of its section dark.
pub(crate) fn set_light(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    at: glam::IVec3,
    light: u8,
) {
    let key = (at.x >> 4, at.z >> 4);
    let layout = game.world.layout();
    if game.world.chunks.get(&key).is_none() {
        let chunk = Chunk::with_layout(key.0, key.1, layout);
        game.world.chunks.insert(key, Arc::new(chunk));
    }
    let chunk = Arc::make_mut(game.world.chunks.get_mut(&key).unwrap());
    let s = chunk.section_index(at.y).unwrap();
    let section = Arc::make_mut(&mut chunk.sections[s]);
    let mut all = [0u8; 4096];
    for (i, v) in all.iter_mut().enumerate() {
        *v = section.light(i);
    }
    all[sunrise_level::chunk::index(at.x, at.y, at.z)] = light;
    section.light = sunrise_level::chunk::Light::from_slice(&all);
}

pub(crate) fn ride(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    vehicle: i32,
    passengers: Vec<i32>,
) {
    game.handle_packet(C::SetPassengers(play::SetPassengers {
        vehicle,
        passengers,
    }))
    .unwrap();
}

/// The camera entity's effect lifecycle against vanilla 26.3's (`MobEffectInstance.tickClient`,
/// `BlendState.tick` and `getFactor`, `LivingEntity.getEffectBlendFactor`,
/// `GameRenderer.nightVisionScale`), read through the view at the partial tick 0.25 with the
/// Darkness Effect option at 0.5. Each expected value is vanilla's float expression evaluated
/// outside Sunrise (a float32 model of the Java code), compared exactly.
#[test]
#[ignore = "daily"]
fn the_camera_entitys_effect_lifecycle_is_vanillas() {
    with_spectator(|game| {
        game.client
            .settings
            .file
            .set("darknessEffectScale", V::Double(0.5));
        set_camera(game, ZOMBIE);
        // Blended (flag 8): from 0, 1/22 a tick while more than 22 ticks are left, then back
        // down by 1/22 a tick (fade out starts at 22 left, `endsWithin(22)`).
        update(game, ZOMBIE, "minecraft:darkness", 30, 8);
        assert_eq!(view(game).darkness, 0.0);
        let mut seen = Vec::new();
        for _ in 0..10 {
            game.tick_remote_entities(true);
            seen.push((kept(game, ZOMBIE).unwrap()[0].1, view(game).darkness));
        }
        assert_eq!(
            seen,
            [
                (29, 0.0056818184),
                (28, 0.028409092),
                (27, 0.051136367),
                (26, 0.07386364),
                (25, 0.096590914),
                (24, 0.11931819),
                (23, 0.14204545),
                (22, 0.1534091),
                (21, 0.13068181),
                (20, 0.10795453),
            ]
        );
        // An update keeps the fade (copyBlendState) and its new duration turns it back up.
        update(game, ZOMBIE, "minecraft:darkness", 100, 8);
        let mut seen = Vec::new();
        for _ in 0..2 {
            game.tick_remote_entities(true);
            seen.push((kept(game, ZOMBIE).unwrap()[0].1, view(game).darkness));
        }
        assert_eq!(seen, [(99, 0.0965909), (98, 0.11931817)]);
        // Removed: 0 at once, with no tick. Added again unblended: at its target at once
        // (skipBlending); added again blended: from 0.
        remove(game, ZOMBIE, "minecraft:darkness");
        assert_eq!(view(game).darkness, 0.0);
        update(game, ZOMBIE, "minecraft:darkness", 600, 0);
        assert_eq!(view(game).darkness, 0.5);
        remove(game, ZOMBIE, "minecraft:darkness");
        update(game, ZOMBIE, "minecraft:darkness", 600, 8);
        assert_eq!(view(game).darkness, 0.0);

        // nightVision: 1 above 200 ticks left and for the infinite -1 only; else the flicker.
        fn night_vision(
            game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
            duration: i32,
        ) -> f32 {
            update(game, ZOMBIE, "minecraft:night_vision", duration, 0);
            view(game).night_vision
        }
        assert_eq!(night_vision(game, 201), 1.0);
        assert_eq!(night_vision(game, -1), 1.0);
        assert_eq!(night_vision(game, 200), 0.6530526);
        assert_eq!(night_vision(game, 37), 0.43270847);
        assert_eq!(night_vision(game, 0), 0.653081);
        // A negative duration other than -1 is not infinite (`isInfiniteDuration`).
        assert_eq!(night_vision(game, -2), 0.40369618);
        // A count below 0 isn't counted down (`hasRemainingDuration`).
        game.tick_remote_entities(true);
        assert_eq!(view(game).night_vision, 0.40369618);
        remove(game, ZOMBIE, "minecraft:night_vision");
        assert_eq!(view(game).night_vision, 0.0);

        // The local player's darknessFactor while it is the camera: also 0 once removed, with
        // no tick.
        set_camera(game, OWN);
        update(game, OWN, "minecraft:darkness", 600, 0);
        assert_eq!(view(game).darkness, 0.5);
        remove(game, OWN, "minecraft:darkness");
        assert_eq!(view(game).darkness, 0.0);
    });
}

/// Values read from the camera entity's effects go with their source before any tick: the
/// entity removed, added again under its id, the camera switched away and back, the level
/// replaced.
#[test]
#[ignore = "daily"]
fn the_view_drops_values_whose_source_is_gone_before_a_tick() {
    with_spectator(|game| {
        game.client
            .settings
            .file
            .set("darknessEffectScale", V::Double(0.5));
        update(game, ZOMBIE, "minecraft:darkness", 600, 0);
        update(game, ZOMBIE, "minecraft:blindness", 600, 0);
        set_camera(game, ZOMBIE);
        let dark = ViewEntity {
            living: true,
            blindness: 1.0,
            night_vision: 0.0,
            darkness: 0.5,
            mood: 0.0,
            eye_light: [0; 2],
        };
        assert_eq!(view(game), dark);
        // Its darkness removed and added again, with no tick in between: 0, then at its target.
        remove(game, ZOMBIE, "minecraft:darkness");
        assert_eq!(view(game).darkness, 0.0);
        update(game, ZOMBIE, "minecraft:darkness", 600, 0);
        assert_eq!(view(game), dark);
        // Away and back with no tick: the same entity's values, none of the player's.
        set_camera(game, OWN);
        assert_eq!(view(game).blindness, 0.0);
        set_camera(game, ZOMBIE);
        assert_eq!(view(game), dark);
        // The id added again: a new entity with no effects.
        add(game, ZOMBIE, "minecraft:zombie", [10.0, 64.0, 12.0]);
        set_camera(game, ZOMBIE);
        let none = ViewEntity {
            blindness: 0.0,
            darkness: 0.0,
            ..dark
        };
        assert_eq!(view(game), none);
        // A new level: nothing kept, the view on the player.
        update(game, ZOMBIE, "minecraft:darkness", 600, 0);
        assert_eq!(view(game).darkness, 0.5);
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:the_nether"),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(view(game).darkness, 0.0);
        assert_eq!(kept(game, ZOMBIE), None);
    });
}

/// Another entity's effects count down inside its own tick, on the ticks vanilla 26.3 ticks it
/// (`ClientLevel.tickEntities`, `TickRateManager.isEntityFrozen`): all of them in a running
/// level; in a frozen one only a stack of riders whose root is a player or carries one, at any
/// depth (`countPlayerPassengers`), its passengers through it.
#[test]
#[ignore = "daily"]
fn effects_count_down_on_the_ticks_their_entity_ticks() {
    const PIG: i32 = 32;
    const OTHER: i32 = 33;
    const RIDER: i32 = 35;
    const STAND: i32 = 50;
    with_spectator(|game| {
        add(game, PIG, "minecraft:pig", [3.0, 64.0, 3.0]);
        // createEntityFromPacket: another player needs its PlayerInfo first.
        game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 0x01,
            entries: vec![play::PlayerInfo {
                uuid: OTHER as u128,
                name: Some("Other".into()),
                ..play::PlayerInfo::default()
            }],
        }))
        .unwrap();
        add(game, OTHER, "minecraft:player", [4.0, 64.0, 4.0]);
        add(game, RIDER, "minecraft:zombie", [5.0, 64.0, 5.0]);
        add(game, STAND, "minecraft:armor_stand", [2.0, 64.0, 2.0]);
        let ids = [ZOMBIE, PIG, OTHER, RIDER, STAND];
        for id in ids {
            update(game, id, "minecraft:speed", 100, 0);
        }
        let left = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
            ids.map(|id| kept(game, id).unwrap()[0].1)
        };
        game.tick_remote_entities(true);
        assert_eq!(left(game), [99; 5], "a running level ticks every entity");
        game.tick_remote_entities(false);
        assert_eq!(
            left(game),
            [99, 99, 98, 99, 99],
            "a frozen level ticks only the players"
        );
        // The other player rides the pig and carries a zombie; this client's player rides the
        // stand. The pig stands in water: its tick takes the fluid in too.
        ride(game, PIG, vec![OTHER]);
        ride(game, OTHER, vec![RIDER]);
        ride(game, STAND, vec![OWN]);
        (game.client.streaming).insert_remote(&mut game.common.world, Chunk::empty(0, 0));
        let water = game.pack.assets.registry.default_state("minecraft:water");
        game.world.set(3, 64, 3, water.unwrap()).unwrap();
        game.tick_remote_entities(false);
        assert_eq!(left(game), [99, 98, 97, 98, 98]);
        let pig = game.mobs.iter().find(|m| m.id == PIG as u32).unwrap();
        assert!(pig.fluids.in_water);
        game.tick_remote_entities(true);
        assert_eq!(left(game), [98, 97, 96, 97, 97]);
    });
}

/// The mount this client drives ticks in this client's own tick (`remote_ride`, the vehicle
/// before its rider), not in the level's pass: its effects count down there, once a tick.
#[test]
#[ignore = "daily"]
fn a_driven_mounts_effects_count_down_in_its_own_tick() {
    const HORSE: i32 = 37;
    with_spectator(|game| {
        add(game, HORSE, "minecraft:horse", [3.0, 64.0, 3.0]);
        let horse = game.mobs.iter_mut().find(|m| m.id == HORSE as u32);
        horse.unwrap().extra.set_saddled(true);
        ride(game, HORSE, vec![OWN]);
        assert_eq!(game.driven_mount().map(|m| m.id), Some(HORSE as u32));
        update(game, HORSE, "minecraft:speed", 100, 0);
        assert!(game.remote_ride(sunrise_sim::sprint::Keys::default()));
        assert_eq!(kept(game, HORSE), Some(vec![("speed", 99)]));
        game.tick_remote_entities(true);
        assert_eq!(kept(game, HORSE), Some(vec![("speed", 99)]));
    });
}

/// A canceled tick (`EntityTickEvent.Pre`) counts no effect down: one told in the batch, and
/// one of a type a mod named, told alone right before its own turn.
#[test]
#[ignore = "daily"]
fn a_canceled_tick_counts_no_effect_down() {
    use mod_api::{EntityTickPreEvent, Hooks, HostSide, ModEntry, Mods};
    const HUSK: i32 = 34;
    const TICKING: i32 = 36;
    use std::sync::Mutex;
    /// Every id the listener's Pre events named, in the order they came.
    type Heard = Arc<Mutex<Vec<u32>>>;
    struct Cancel(Heard);
    #[mod_api::__subscriptions]
    impl Hooks for Cancel {
        fn entity_tick_pre(
            &self,
            _: HostSide<'_>,
            event: &mut EntityTickPreEvent<'_>,
        ) -> mod_api::Flow {
            self.0.lock().unwrap().push(event.entity);
            if event.entity == ZOMBIE as u32 || event.entity == HUSK as u32 {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }
    let heard: Heard = Arc::default();
    // The ids the Pre events of one tick named, sorted, and forgotten for the next tick.
    let take = |heard: &Heard| {
        let mut ids = std::mem::take(&mut *heard.lock().unwrap());
        ids.sort_unstable();
        ids
    };
    let mods: Mods = vec![ModEntry::new(
        "camera-view-cancel",
        Box::new(Cancel(heard.clone())),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_spectator(|game| {
            // The spectator views the zombie: it is the camera entity.
            set_camera(game, ZOMBIE);
            assert_eq!(game.client.remote.as_ref().unwrap().camera, Some(ZOMBIE));
            add(game, HUSK, "minecraft:husk", [6.0, 64.0, 6.0]);
            add(game, TICKING, "minecraft:zombie", [7.0, 64.0, 7.0]);
            for id in [ZOMBIE, HUSK, TICKING] {
                update(game, id, "minecraft:speed", 100, 0);
            }
            let left = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                [ZOMBIE, HUSK, TICKING].map(|id| kept(game, id).unwrap()[0].1)
            };
            take(&heard);
            game.tick_remote_entities(true);
            // The camera entity's tick was canceled from its own Pre event, the husk's from
            // its own, and the zombie whose tick ran was told too, so the listener was live.
            assert_eq!(take(&heard), [ZOMBIE as u32, HUSK as u32, TICKING as u32]);
            assert_eq!(left(game), [100, 100, 99]);
            // Frozen: the camera zombie, carrying this client's player, still ticks, so it is
            // told and its canceled tick still counts nothing down; the others are not told.
            ride(game, ZOMBIE, vec![OWN]);
            game.tick_remote_entities(false);
            assert_eq!(take(&heard), [ZOMBIE as u32]);
            assert_eq!(left(game), [100, 100, 99]);
        });
    });
}

pub(crate) const FRAME_NAME: &str = "cut_layout::game::client_side::remote_client::camera_view_tests::the_frame_reads_the_eye_light_in_first_and_third_person";
pub(crate) const FRAME_CASE: &str = "SUNRISE_CAMERA_VIEW_FRAME_CASE";

/// What a game's frame gives the pack: its camera, its `eyePosition` and the view's light.
pub(crate) fn framed(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
) -> (glam::DVec3, glam::DVec3, [i32; 2]) {
    let mut seen = None;
    game.render_with(|_, frame| {
        seen = Some((frame.camera, frame.eye, frame.pack.view.eye_light));
        Ok(true)
    })
    .unwrap();
    seen.expect("no frame was built")
}
