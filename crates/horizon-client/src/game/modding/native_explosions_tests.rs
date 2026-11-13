//! `LevelRef::explode`'s contract through both mod targets: a native call of the handle method
//! and a wasm guest's call through the generated forwarder, each on a fresh game, on the stack
//! a wasm guest runs on.
#![cfg(test)]
use crate::game::Game;
use crate::game::native_entity_refs::tests::{bind_local_player, run_on_guest_stack, with_game};
use horizon_client::{chunk::Chunk, mobs::Mob, mobs::Who, motion::PlayerMotion};
use horizon_core::{
    geometry::Vec3,
    position::{BlockPos, ChunkPos},
};
use mod_api::{
    EntityRef, ExplosionInteraction, Flow, Hooks, LivingDeathEvent, ModEntry, ServerHost,
    wire::{Reader, Writer},
};
use std::sync::{Arc, Mutex};

/// How the call reaches the game.
#[derive(Clone, Copy, Debug)]
enum Via {
    Native,
    Wasm,
}
const BOTH: [Via; 2] = [Via::Native, Via::Wasm];

/// One `explode` call.
struct Blast {
    source: Option<EntityRef>,
    center: [f64; 3],
    radius: f32,
    fire: bool,
    interaction: ExplosionInteraction,
}

/// Runs `blast` in the overworld through `via` and checks its answer.
fn explode(game: &mut Game<'_>, via: Via, blast: Blast, expected: bool) {
    let level = game.level("minecraft:overworld").unwrap();
    match via {
        Via::Native => assert_eq!(
            level.explode(
                game,
                blast.source,
                blast.center,
                blast.radius,
                blast.fire,
                blast.interaction
            ),
            expected
        ),
        Via::Wasm => wasm_explode(game, level, blast, expected),
    }
}

/// A guest whose server tick calls `level_explode` through its import and
/// answers the tick only when the host's answer is `expected` (it traps otherwise).
fn wasm_explode(game: &mut Game<'_>, level: mod_api::LevelRef, blast: Blast, expected: bool) {
    let mut input = Writer::new();
    input.put(&level);
    input.put(&blast.source);
    input.put(&blast.center);
    input.put(&blast.radius);
    input.put(&blast.fire);
    input.put(&blast.interaction);
    let mut answer = Writer::new();
    answer.put(&expected);
    let mut reply = Writer::new();
    reply.put(&mod_api::ServerTickPhase::ServerTickEnd {
        world_running: true,
    });
    let escape = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|b| format!("\\{b:02x}"))
            .collect::<String>()
    };
    let check = answer
        .bytes()
        .iter()
        .enumerate()
        .map(|(i, byte)| {
            format!(
                "i32.const {} i32.load8_u i32.const {byte} i32.ne if unreachable end ",
                32768 + i
            )
        })
        .collect::<String>();
    let id = horizon_mod_wasm::fixture::import_id("BackendMut", "level_explode").unwrap();
    let bytes = wat::parse_str(format!(
        r#"(module
          (@custom "dylink.0" "\01\04\00\00\00\00")
          (import "env" "memory" (memory 1))
          (import "env" "__indirect_function_table" (table 1 funcref))
          (import "horizon" "host" (func $host (param i32 i32 i32 i32 i32) (result i32)))
          (import "horizon" "host_reply" (func (param i32)))
          (func (export "horizon_layout") (result i64) i64.const {layout})
          (data (i32.const 4096) "{input}")
          (data (i32.const 16384) "{reply}")
          (func (export "horizon_in") (param i32) (result i32) i32.const 512)
          (func (export "horizon_hook") (param i32) (result i64)
            i32.const {id} i32.const 4096 i32.const {input_len} i32.const 32768 i32.const 4096 call $host drop
            {check}
            i64.const {reply_at}))"#,
        layout = *mod_api::wire::LAYOUT,
        input = escape(input.bytes()),
        reply = escape(reply.bytes()),
        input_len = input.bytes().len(),
        reply_at = (16384u64 << 32) | reply.bytes().len() as u64,
    ))
    .unwrap();
    let module = horizon_mod_wasm::fixture::module(&bytes).unwrap();
    let mut context = Writer::new();
    context.put(&mod_api::Authority::DedicatedServer);
    context.put(&level.epoch());
    context.put(&mod_api::TickId::new(1));
    let context: mod_api::LifecycleContext = Reader::new(context.bytes()).take().unwrap();
    let mut tick = mod_api::ServerTickEvent {
        context: &context,
        phase: mod_api::ServerTickPhase::ServerTickStart {
            world_running: true,
        },
    };
    Hooks::server_tick(&module, game, &mut tick);
    assert!(
        matches!(
            tick.phase,
            mod_api::ServerTickPhase::ServerTickEnd {
                world_running: true
            }
        ),
        "the guest saw the answer {expected}"
    );
}

/// A server's overworld chunk (0, 0) with a stone floor at y 79.
fn floor(game: &mut Game<'_>) {
    game.build_registries();
    game.load_server_loot();
    game.set_allow_commands(true);
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("fill 0 79 0 15 79 15 stone"));
}

fn name(game: &Game<'_>, pos: [i32; 3]) -> String {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    game.res.facts().get(state).name.to_string()
}

/// How many floor blocks are still stone.
fn stone(game: &Game<'_>) -> usize {
    (0..16)
        .flat_map(|x| (0..16).map(move |z| [x, 79, z]))
        .filter(|&p| name(game, p) == "minecraft:stone")
        .count()
}

fn mob(game: &mut Game<'_>, id: u32, kind: &str, at: Vec3, health: f32) {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    let mut mob = Mob::new(id, kind, at, 0.0);
    mob.persistent = true;
    mob.no_ai = true;
    mob.on_ground = true;
    mob.health = health;
    game.mobs.push(mob);
}

/// What `living_death` heard: the dying entity, the damage type and the causing entity.
type Deaths = Arc<Mutex<Vec<(u32, String, i32)>>>;
struct Deathwatch(Deaths);
#[mod_api::__subscriptions]
impl Hooks for Deathwatch {
    fn living_death(&self, _: &mut dyn ServerHost, event: &mut LivingDeathEvent<'_>) -> Flow {
        let source = event.source;
        self.0
            .lock()
            .unwrap()
            .push((event.entity, source.damage_type.into(), source.causing));
        Flow::Continue
    }
}

/// A blast at (4.5, 80, 4.5) whose source `source` makes (it answers the
/// source's entity id) kills a zombie on one health beside it: what the
/// zombie's death names, and whether a player is credited with the kill
/// (`last_hurt_by_player`). The player in place is entity 11, out of reach.
fn kill(via: Via, source: impl Fn(&mut Game<'_>) -> u32) -> (Vec<(u32, String, i32)>, bool) {
    crate::entry::install_client_hooks();
    let deaths = Deaths::default();
    let mut credited = false;
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "explode-deaths",
            Box::new(Deathwatch(deaths.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                floor(game);
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(14.5, 80.0, 14.5)).unwrap());
                bind_local_player(game, 11, 123);
                game.set_mode_id(0);
                game.next_entity = 100;
                mob(game, 41, "minecraft:zombie", Vec3::new(6.0, 80.0, 4.5), 1.0);
                let id = source(game);
                let source = game.entity(id);
                assert!(source.is_some(), "the source {id} resolves");
                let blast = Blast {
                    source,
                    center: [4.5, 80.0, 4.5],
                    radius: 4.0,
                    fire: false,
                    interaction: ExplosionInteraction::None,
                };
                explode(game, via, blast, true);
                credited = game.last_hurt_by_player.contains_key(&41);
                game.tick_creature_entities().unwrap();
            })
        },
    );
    let deaths = deaths.lock().unwrap().clone();
    (deaths, credited)
}

/// [`kill`] through both targets, each answering `expected`.
fn kills(name: &str, source: fn(&mut Game<'_>) -> u32, expected: (i32, &'static str, bool)) {
    crate::game::in_own_process(&format!("game::native_explosions_tests::{name}"), || {
        run_on_guest_stack(move || {
            for via in BOTH {
                let (causing, damage_type, credited) = expected;
                assert_eq!(
                    kill(via, source),
                    (vec![(41, damage_type.to_owned(), causing)], credited),
                    "{via:?}"
                );
            }
        });
    });
}

/// Primed TNT lit by `owner` beside the zombie.
fn tnt(game: &mut Game<'_>, owner: Option<Who>) -> u32 {
    game.add_primed_tnt(BlockPos::new(4, 80, 4), 0.0, 80, owner, None);
    game.tnts.last().unwrap().id
}

/// A summoned entity's id.
fn summoned(game: &mut Game<'_>, kind: &str, nbt: &str) -> u32 {
    let target = game
        .summon_with_nbt(kind, [4.5, 80.5, 4.5], nbt)
        .unwrap()
        .unwrap();
    use crate::game::commands::Target;
    match target {
        Target::Stand(id) | Target::Trident(id) | Target::Rocket(id) => id,
        other => panic!("{kind} summoned as {other:?}"),
    }
}

#[test]
#[ignore = "daily"]
fn player_lit_tnt_as_the_source_credits_the_player_with_the_kill() {
    kills(
        "player_lit_tnt_as_the_source_credits_the_player_with_the_kill",
        |game| tnt(game, Some(Who::Player)),
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn unowned_tnt_as_the_source_credits_no_one() {
    kills(
        "unowned_tnt_as_the_source_credits_no_one",
        |game| tnt(game, None),
        (0, "minecraft:explosion", false),
    );
}

#[test]
#[ignore = "daily"]
fn a_player_as_the_source_is_credited_as_a_player() {
    kills(
        "a_player_as_the_source_is_credited_as_a_player",
        |_| 11,
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn a_living_entity_outside_the_mob_list_causes_its_own_blast() {
    kills(
        "a_living_entity_outside_the_mob_list_causes_its_own_blast",
        |game| summoned(game, "minecraft:armor_stand", "{}"),
        (101, "minecraft:player_explosion", false),
    );
}

#[test]
#[ignore = "daily"]
fn a_players_thrown_trident_credits_the_player() {
    kills(
        "a_players_thrown_trident_credits_the_player",
        |game| {
            let id = summoned(game, "minecraft:trident", "{NoGravity:1b}");
            game.tridents.for_each_mut(|t| t.owner = 11);
            id
        },
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn a_players_firework_rocket_credits_the_player() {
    kills(
        "a_players_firework_rocket_credits_the_player",
        |game| {
            let id = summoned(game, "minecraft:firework_rocket", "{LifeTime:100}");
            game.rockets.for_each_mut(|r| r.owner = Some(Who::Player));
            id
        },
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn a_players_fishing_bobber_credits_the_player() {
    kills(
        "a_players_fishing_bobber_credits_the_player",
        |game| {
            let hook =
                crate::game::fishing::Hook::remote(900, Vec3::new(4.5, 81.0, 4.5), [1, 2, 3, 4]);
            game.install_fishing_hook(hook);
            900
        },
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn a_mob_blast_breaks_blocks_only_when_its_source_may_grief() {
    run_on_guest_stack(|| {
        for via in BOTH {
            for griefing in [false, true] {
                with_game(|game| {
                    floor(game);
                    assert!(game.ui.rules.set(
                        "mob_griefing",
                        horizon_client::gamerules::Value::Bool(griefing),
                    ));
                    mob(
                        game,
                        42,
                        "minecraft:creeper",
                        Vec3::new(8.5, 80.0, 8.5),
                        20.0,
                    );
                    let blast = Blast {
                        source: game.entity(42),
                        center: [8.5, 80.0, 8.5],
                        radius: 3.0,
                        fire: false,
                        interaction: ExplosionInteraction::Mob,
                    };
                    explode(game, via, blast, true);
                    let (under, floor) = (name(game, [8, 79, 8]), stone(game));
                    if griefing {
                        assert_eq!(under, "minecraft:air", "{via:?}");
                        assert!(floor < 256, "{via:?}: {floor}");
                    } else {
                        assert_eq!((under.as_str(), floor), ("minecraft:stone", 256), "{via:?}");
                    }
                    // The source is spared by its own blast.
                    assert_eq!(game.mobs[0].health, 20.0, "{via:?}");
                });
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_trigger_blast_presses_a_button_and_breaks_nothing() {
    run_on_guest_stack(|| {
        for via in BOTH {
            with_game(|game| {
                floor(game);
                assert!(game.run_command("setblock 6 80 4 minecraft:stone_button[face=floor]"));
                let blast = Blast {
                    source: None,
                    center: [4.5, 80.5, 4.5],
                    radius: 3.0,
                    fire: false,
                    interaction: ExplosionInteraction::Trigger,
                };
                explode(game, via, blast, true);
                let state = game.world.block(6, 80, 4);
                let powered = game
                    .pack
                    .assets
                    .registry
                    .state(state)
                    .and_then(|s| s.properties.get("powered").cloned());
                assert_eq!(name(game, [6, 80, 4]), "minecraft:stone_button", "{via:?}");
                assert_eq!(powered.as_deref(), Some("true"), "{via:?}");
                assert_eq!(stone(game), 256, "{via:?}");
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_blast_that_keeps_blocks_still_lights_fire() {
    run_on_guest_stack(|| {
        for via in BOTH {
            with_game(|game| {
                floor(game);
                let blast = Blast {
                    source: None,
                    center: [8.5, 80.5, 8.5],
                    radius: 4.0,
                    fire: true,
                    interaction: ExplosionInteraction::None,
                };
                explode(game, via, blast, true);
                let fires = (0..16)
                    .flat_map(|x| (0..16).map(move |z| [x, 80, z]))
                    .filter(|&p| name(game, p) == "minecraft:fire")
                    .count();
                assert!(fires > 0, "{via:?}");
                assert_eq!(stone(game), 256, "{via:?}");
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn an_invalid_source_explodes_nothing() {
    run_on_guest_stack(|| {
        for via in BOTH {
            with_game(|game| {
                floor(game);
                mob(
                    game,
                    43,
                    "minecraft:zombie",
                    Vec3::new(8.5, 80.0, 9.5),
                    20.0,
                );
                let live = game.entity(43).unwrap();
                let stale =
                    EntityRef::new(mod_api::WorldEpoch::new(live.epoch().get() + 1), live.id());
                let nothing = EntityRef::new(live.epoch(), 9999);
                for source in [stale, nothing] {
                    let blast = Blast {
                        source: Some(source),
                        center: [8.5, 80.5, 8.5],
                        radius: 4.0,
                        fire: true,
                        interaction: ExplosionInteraction::Block,
                    };
                    explode(game, via, blast, false);
                }
                assert_eq!(stone(game), 256, "{via:?}");
                assert_eq!(game.mobs[0].health, 20.0, "{via:?}");
            });
        }
    });
}
