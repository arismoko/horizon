//! `LevelRef::next_sound_seed`, `LevelRef::play_seeded_sound` and
//! `EntityRef::play_seeded_bound_sound` through the handles a mod calls: a draw takes the next
//! seed of the stream the level's sounds take theirs from (`Level.soundSeedGenerator.nextLong()`),
//! and a seeded play (`ServerLevel.playSeededSound`) sends the caller's seed and draws nothing.
//! The hub's stream is replaced by a known `LegacyRandom`, and a copy of it says which draw each
//! seed must be.

use mod_api::{HostTransaction, LevelRef, ServerHost, SoundSource, WorldEpoch};
use std::sync::{Arc, mpsc};
use sunrise_core::geometry::Vec3;
use sunrise_core::random::{LegacyRandom, RandomSource};
use sunrise_gameplay::game::native_guest_turn_tests::with_resident_guests;
use sunrise_protocol::{buf::Reader, packets::play::Clientbound as C};
use sunrise_sim::lan::{Out, PlayerSender};
use sunrise_sim::mobs::Mob;

const STREAM_SEED: i64 = 0x5EED_50DE;
const AT: [f64; 3] = [3.0, 64.0, 5.0];
const BELL: &str = "minecraft:block.bell.use";
const ANVIL: &str = "minecraft:block.anvil.use";
const BURP: &str = "minecraft:entity.player.burp";
const CHIME: &str = "minecraft:block.amethyst_block.chime";

/// Each sound packet a guest got, in order: its sound id, its seed and whether it is bound to an
/// entity (`ClientboundSoundEntityPacket`).
fn heard(rx: &mpsc::Receiver<Out>) -> Vec<(i32, i64, bool)> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            _ => Vec::new(),
        })
        .filter_map(|body| {
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            match C::decode(id, reader.rest()).ok()? {
                C::Sound(p) => Some((p.sound.0, p.seed, false)),
                C::SoundEntity(p) => Some((p.sound.0, p.seed, true)),
                _ => None,
            }
        })
        .collect()
}

/// The guests' packet receivers (connections 12 and 13), the level, guest 13's handle, a cow
/// beside them, and a copy of the sound-seed stream the hub now draws from.
struct Fixture {
    pub rx12: mpsc::Receiver<Out>,
    rx13: mpsc::Receiver<Out>,
    level: LevelRef,
    player13: mod_api::PlayerRef,
    cow: mod_api::EntityRef,
    stream: LegacyRandom,
}

fn fixture(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> Fixture {
    game.options.dedicated = Some(Vec::new());
    let (tx12, rx12) = mpsc::channel();
    let (tx13, rx13) = mpsc::channel();
    let hub = &mut game.server.lan.as_mut().unwrap().hub;
    hub.guests
        .update(0, |g| g.sender = PlayerSender::new(tx12, Arc::default()));
    hub.guests
        .update(1, |g| g.sender = PlayerSender::new(tx13, Arc::default()));
    hub.set_sound_seeds(LegacyRandom::new(STREAM_SEED));
    let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
    game.mobs
        .push(Mob::new(40, kind, Vec3::new(3.0, 64.0, 4.0), 0.0));
    Fixture {
        rx12,
        rx13,
        level: game.level(game.dim.id()).unwrap(),
        player13: game.player(13).unwrap(),
        cow: game.entity(40).unwrap(),
        stream: LegacyRandom::new(STREAM_SEED),
    }
}

fn sound_id(game: &sunrise_gameplay::game::ServerGame<'_>, name: &str) -> i32 {
    game.server
        .lan
        .as_ref()
        .unwrap()
        .hub
        .registry_id("minecraft:sound_event", name)
        .unwrap()
}

/// Outside a transaction: `next_sound_seed` is the stream's next draw, `play_sound` takes the one
/// after it at the call, the seeded plays keep the caller's seed and draw nothing, and a refused
/// call draws nothing either.
#[test]
#[ignore = "daily"]
fn seed_draws_and_seeded_sounds_take_the_level_sound_seed_stream_in_call_order() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_sound_seed_tests::seed_draws_and_seeded_sounds_take_the_level_sound_seed_stream_in_call_order",
        || {
            with_resident_guests(|game| {
                let Fixture {
                    rx12,
                    rx13,
                    level,
                    player13,
                    cow,
                    mut stream,
                } = fixture(game);
                let (bell, anvil, burp) = (
                    sound_id(game, BELL),
                    sound_id(game, ANVIL),
                    sound_id(game, BURP),
                );
                let first = stream.next_long();
                {
                    let host: &mut dyn ServerHost = &mut *game;
                    // `Level.playSound` as a mod models it: draw, then play with that seed.
                    let drawn = level.next_sound_seed(host).unwrap();
                    assert_eq!(drawn, first, "the stream's next draw");
                    assert!(level.play_seeded_sound(
                        host,
                        BELL,
                        AT,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None,
                        drawn
                    ));
                    assert!(level.play_sound(host, ANVIL, AT, SoundSource::Blocks, 1.0, 1.0, None));
                    // `playSeededSound(except, entity, ...)`: bound to the cow, guest 13 left out.
                    assert!(cow.play_seeded_bound_sound(
                        host,
                        BURP,
                        SoundSource::Neutral,
                        1.0,
                        1.0,
                        Some(player13),
                        0x1234_5678_9ABC
                    ));
                }
                let anvil_seed = stream.next_long();
                game.flush_native_sounds();
                assert_eq!(
                    heard(&rx12),
                    [
                        (bell, first, false),
                        (anvil, anvil_seed, false),
                        (burp, 0x1234_5678_9ABC, true),
                    ]
                );
                assert_eq!(
                    heard(&rx13),
                    [(bell, first, false), (anvil, anvil_seed, false)],
                    "the bound sound leaves out its except player"
                );

                // Refused: a stale level, a stale except player. None of them draws.
                let host: &mut dyn ServerHost = &mut *game;
                let stale_level =
                    LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
                let gone = mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(
                        WorldEpoch::new(player13.entity().epoch().get() + 1),
                        player13.entity().id(),
                    ),
                    player13.uuid(),
                );
                assert_eq!(stale_level.next_sound_seed(host), None);
                assert!(!stale_level.play_seeded_sound(
                    host,
                    BELL,
                    AT,
                    SoundSource::Blocks,
                    1.0,
                    1.0,
                    None,
                    1
                ));
                assert!(!level.play_seeded_sound(
                    host,
                    BELL,
                    AT,
                    SoundSource::Blocks,
                    1.0,
                    1.0,
                    Some(gone),
                    1
                ));
                assert!(!cow.play_seeded_bound_sound(
                    host,
                    BURP,
                    SoundSource::Neutral,
                    1.0,
                    1.0,
                    Some(gone),
                    1
                ));
                assert_eq!(
                    level.next_sound_seed(host),
                    Some(stream.next_long()),
                    "neither the seeded plays nor the refusals drew"
                );
                game.flush_native_sounds();
                assert!(heard(&rx12).is_empty() && heard(&rx13).is_empty());
            });
        },
    );
}

/// Inside a transaction: `next_sound_seed` draws at the call and an abort does not undo it; a held
/// `play_sound` draws only when published at commit, after that draw; a held seeded play keeps
/// its seed; an aborted transaction sends nothing.
#[test]
#[ignore = "daily"]
fn seed_draws_happen_at_the_call_inside_transactions_and_held_sounds_draw_at_commit() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_sound_seed_tests::seed_draws_happen_at_the_call_inside_transactions_and_held_sounds_draw_at_commit",
        || {
            with_resident_guests(|game| {
                let Fixture {
                    rx12,
                    rx13,
                    level,
                    player13,
                    mut stream,
                    ..
                } = fixture(game);
                let (bell, anvil) = (sound_id(game, BELL), sound_id(game, ANVIL));
                let host: &mut dyn ServerHost = &mut *game;

                let in_abort = stream.next_long();
                let aborted = host.transaction(|host, _| {
                    assert!(level.play_sound(host, CHIME, AT, SoundSource::Blocks, 1.0, 1.0, None));
                    assert_eq!(level.next_sound_seed(host), Some(in_abort));
                    assert!(level.play_seeded_sound(
                        host,
                        CHIME,
                        AT,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None,
                        7
                    ));
                    Err::<(), ()>(())
                });
                assert!(aborted.is_err());
                assert_eq!(
                    level.next_sound_seed(host),
                    Some(stream.next_long()),
                    "the abort leaves the draw made inside it, and the held sound drew nothing"
                );

                let in_commit = stream.next_long();
                let committed = host.transaction(|host, _| {
                    assert!(level.play_sound(
                        host,
                        BELL,
                        AT,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        Some(player13)
                    ));
                    assert_eq!(level.next_sound_seed(host), Some(in_commit));
                    assert!(level.play_seeded_sound(
                        host,
                        ANVIL,
                        AT,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None,
                        -9
                    ));
                    Ok::<(), ()>(())
                });
                assert!(committed.is_ok());
                // Published at commit in journal order: the bell takes the draw after the one made
                // inside the transaction, the anvil keeps its seed.
                let bell_seed = stream.next_long();
                assert_eq!(heard(&rx12), [(bell, bell_seed, false), (anvil, -9, false)]);
                assert_eq!(
                    heard(&rx13),
                    [(anvil, -9, false)],
                    "the bell leaves out guest 13"
                );
                assert_eq!(level.next_sound_seed(host), Some(stream.next_long()));
            });
        },
    );
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
