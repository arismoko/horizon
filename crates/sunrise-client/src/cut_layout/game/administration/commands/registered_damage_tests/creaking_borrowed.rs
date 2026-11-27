use super::setup_live_damage_world;
use mod_api::{
    ActiveEffect, ApplyEffectTickEvent, BlockState, BlockUpdateFlags, Flow, Hooks, Host,
    HostBlockStates, MobEffectExpiredEvent, MobEffectPart, Registrar, RegistrationContext,
    ServerHost, ShouldApplyEffectTickThisTickEvent, VanillaGameEventEvent,
};
use std::sync::{Arc, Mutex};
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_level::{brewing, containers::BlockEntity};
use sunrise_sim::mobs::{Who, creaking};
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay_common::game::combat_common::PlayerCredit,
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENT_ORDER,
};

const HOME: [i32; 3] = [8, 64, 8];
const FISH_AFTER_HURT: [f64; 3] = [12.5, 64.0, 4.5];
const FINAL_HURT: &str = "watch:final_hurt";

#[derive(Clone, Copy)]
struct Reaction {
    position: [f64; 3],
    peer_velocity: Option<[f64; 3]>,
}

#[derive(Default)]
struct Heard {
    mob: u32,
    peer: Option<u32>,
    sting_fish: Option<u32>,
    reactions: Vec<Reaction>,
    expired_amplifiers: Vec<i32>,
}

struct Watch {
    heard: Arc<Mutex<Heard>>,
    replace_protector: bool,
    effect_edit: Option<EffectEdit>,
}

#[derive(Clone, Copy)]
enum EffectEdit {
    RemoveEarlier,
    ReplaceCurrent,
    InsertAndRemove,
    ReplaceRegistered,
}

struct FinalHurt;
#[mod_api::sunrise_part]
impl MobEffectPart for FinalHurt {
    fn should_apply_effect_tick_this_tick(
        &self,
        _: &dyn Host,
        event: &mut ShouldApplyEffectTickThisTickEvent,
    ) {
        event.apply = event.tick_count == 1;
    }

    fn apply_effect_tick(&self, _: &dyn Host, event: &mut ApplyEffectTickEvent<'_>) {
        event.hurt("minecraft:magic", 1.0);
    }
}

#[mod_api::__subscriptions]
impl Hooks for Watch {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        if matches!(self.effect_edit, Some(EffectEdit::ReplaceRegistered)) {
            let key = r.effect_key(FINAL_HURT);
            r.mob_effect_part(&key, FinalHurt).unwrap();
        }
    }

    fn mob_effect_expired(&self, host: &dyn Host, event: &mut MobEffectExpiredEvent) -> Flow {
        let mut heard = self.heard.lock().unwrap();
        if event.entity.entity().id() == heard.mob
            && host.effect_key(FINAL_HURT) == Some(event.effect)
        {
            assert_eq!(heard.reactions.len(), 1, "expiry follows the accepted hurt");
            heard.expired_amplifiers.push(event.amplifier);
        }
        Flow::Continue
    }

    fn vanilla_game_event(
        &self,
        host: &mut dyn ServerHost,
        event: &mut VanillaGameEventEvent<'_>,
    ) -> Flow {
        if event.event != "minecraft:entity_action" {
            return Flow::Continue;
        }
        let (mob, peer, sting_fish) = {
            let mut heard = self.heard.lock().unwrap();
            if event.cause != Some(heard.mob) {
                return Flow::Continue;
            }
            let mob = host.entity(heard.mob).unwrap();
            let peer = heard.peer.and_then(|id| host.entity(id));
            let peer_velocity = peer.and_then(|peer| peer.velocity(host));
            heard.reactions.push(Reaction {
                position: mob.position(host).unwrap(),
                peer_velocity,
            });
            (mob, peer, heard.sting_fish)
        };
        if let Some(edit) = self.effect_edit {
            let living = host.living(mob).unwrap();
            let id = match edit {
                EffectEdit::RemoveEarlier => "minecraft:speed",
                EffectEdit::ReplaceCurrent => "minecraft:instant_damage",
                EffectEdit::InsertAndRemove => "minecraft:glowing",
                EffectEdit::ReplaceRegistered => FINAL_HURT,
            };
            let effect = host.effect_key(id).unwrap();
            let added = ActiveEffect::new(effect, 40, 0, false, true, true);
            if matches!(edit, EffectEdit::InsertAndRemove) {
                assert!(living.add_effect(host, &added));
            }
            assert!(living.remove_effect(host, effect));
            if matches!(
                edit,
                EffectEdit::ReplaceCurrent | EffectEdit::ReplaceRegistered
            ) {
                assert!(living.add_effect(host, &added));
            }
        }
        if let Some(peer) = peer {
            // Move the already-selected peer to the opposite side, out of range.
            assert!(peer.teleport_to(host, [3.5, 64.0, 4.5], None));
        }
        if let Some(fish) = sting_fish {
            assert!(
                host.entity(fish)
                    .unwrap()
                    .teleport_to(host, FISH_AFTER_HURT, None)
            );
            for line in [
                "data merge entity 00000056-0000-0000-0000-000000000002 {PuffState:1}",
                "team join sting 00000056-0000-0000-0000-000000000003",
            ] {
                let result = host.commands().run_command_with_output(line);
                assert!(result.success, "{line}: {:?}", result.feedback);
            }
        }
        if self.replace_protector {
            let air = mod_api::__private::block_key("minecraft:air").unwrap();
            let state = BlockState::of(&host.block_states(), air, &[]).unwrap();
            assert_eq!(
                host.level(event.dimension).unwrap().set_block(
                    host,
                    HOME,
                    state,
                    BlockUpdateFlags::SKIP_BLOCK_ENTITY_SIDE_EFFECTS,
                ),
                Ok(true),
            );
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn borrowed_creaking_hurt_finishes_before_the_tick_continues() {
    run_borrowed_creaking_hurt_finishes_before_the_tick_continues();
}

pub(crate) fn run_borrowed_creaking_hurt_finishes_before_the_tick_continues() {
    const CASE: &str = "SUNRISE_BORROWED_CREAKING_CASE";
    if run_fresh_cases(
        "cut_layout::game::server_side::commands::world_cmds::registered_damage_tests::creaking_borrowed::borrowed_creaking_hurt_finishes_before_the_tick_continues",
        CASE,
        &[
            "expiry",
            "offline",
            "protector",
            "mismatched_protector",
            "cramming",
            "effect_shift",
            "effect_replaced",
            "effect_transient",
            "registered_effect_replaced",
            "sting",
        ],
    ) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    let effect_edit = match case.as_str() {
        "effect_shift" => Some(EffectEdit::RemoveEarlier),
        "effect_replaced" => Some(EffectEdit::ReplaceCurrent),
        "effect_transient" => Some(EffectEdit::InsertAndRemove),
        "registered_effect_replaced" => Some(EffectEdit::ReplaceRegistered),
        _ => None,
    };
    let heard = Arc::new(Mutex::new(Heard::default()));
    let mods = vec![mod_api::ModEntry::new(
        "watch",
        Box::new(Watch {
            heard: heard.clone(),
            replace_protector: case == "protector",
            effect_edit,
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        if matches!(effect_edit, Some(EffectEdit::ReplaceRegistered)) {
            assert!(brewing::register_mod_effects([brewing::ModEffect::new(
                FINAL_HURT,
                0,
                brewing::Category::Harmful,
                "watch",
            )]));
        }
        with_game(|game| {
            if case == "sting" {
                crate::game::server_side::native_living_flags::tests::guest_server(
                    game, true, false,
                );
            }
            setup_live_damage_world(game);
            if case == "sting" {
                assert!(game.run_command("summon pufferfish 4.5 64 4.5 {UUID:[I;86,0,0,2],PuffState:2,NoAI:1b,NoGravity:1b}"));
                let fish = game
                    .mobs
                    .iter()
                    .find(|m| m.kind.id == "minecraft:pufferfish")
                    .unwrap();
                heard.lock().unwrap().sting_fish = Some(fish.id);
            }
            for command in [
                "forceload add 0 0",
                "time set 13000",
                "summon creaking 4.5 64 4.5 {UUID:[I;86,0,0,1],NoAI:1b,NoGravity:1b}",
                "setblock 8 63 8 minecraft:pale_oak_log[axis=y]",
                "setblock 8 64 8 minecraft:creaking_heart[axis=y,creaking_heart_state=awake]{creaking:[I;86,0,0,1]}",
                "setblock 8 65 8 minecraft:pale_oak_log[axis=y]",
            ] {
                assert!(game.run_command(command), "{command}");
            }
            game.load_ticking_neighbourhood(0, 0);
            game.update_chunk_tickets();
            assert!(game.entity_ticking_area()(4, 4));
            let creaking = game
                .mobs
                .iter_mut()
                .find(|m| creaking::is_creaking(m))
                .unwrap();
            let mob = creaking.id;
            creaking::set_transient(creaking, HOME);
            creaking.ai.last_hurt_by_player = 1;
            heard.lock().unwrap().mob = mob;
            game.remember_mob_player_credit(mob);
            assert!(game.server.last_hurt_by_player.contains_key(&mob));
            if case == "offline" {
                game.server
                    .last_hurt_by_player
                    .insert(mob, PlayerCredit(0x23));
            }
            if case == "mismatched_protector" {
                let Some(BlockEntity::CreakingHeart(heart)) =
                    game.world.block_entities.get_mut(&(8, 64, 8))
                else {
                    panic!("missing heart");
                };
                heart.creaking = Some((None, [86, 0, 0, 99]));
            }
            if case == "expiry" {
                // Vanilla keeps the reference when the clock goes 1 -> 0. The
                // following tick's in-wall check is still before reference expiry.
                game.tick_creature_entities().unwrap();
                assert_eq!(game.mob_object(mob).unwrap().ai.last_hurt_by_player, 0);
                assert!(game.server.last_hurt_by_player.contains_key(&mob));
            }
            let peer = if case == "cramming" {
                game.set_difficulty_forced(2);
                assert!(game.run_command("gamerule minecraft:max_entity_cramming 1"));
                assert_eq!(game.mob_world_base(&[]).max_entity_cramming, 1);
                let peer = game
                    .spawn_mob_raw("minecraft:pig", [5.65, 64.0, 4.5], true)
                    .unwrap();
                let pig = game.mob_object_mut(peer).unwrap();
                pig.no_ai = true;
                pig.no_gravity = true;
                let pig_bounds = pig.bounds();
                assert!(
                    !game
                        .mob_object(mob)
                        .unwrap()
                        .bounds()
                        .intersects(pig_bounds)
                );
                heard.lock().unwrap().peer = Some(peer);
                // Mob.baseTick's ambient draw, then the pushEntities cramming draw.
                let seed = (0..100)
                    .find(|&seed| {
                        let mut rng = LegacyRandom::new(seed);
                        let _ = rng.next_int_bound(1000);
                        rng.next_int_bound(4) == Ok(0)
                    })
                    .unwrap();
                let creaking = game.mob_object_mut(mob).unwrap();
                creaking.random = LegacyRandom::new(seed);
                // Keep authoritative travel but skip brain/navigation draws so
                // the ambient and cramming rolls are the only mob RNG work.
                creaking.no_ai = false;
                creaking.ai.ai_paused = true;
                creaking.velocity = Vec3::new(0.8, 0.0, 0.0);
                Some(peer)
            } else if let Some(edit) = effect_edit {
                let creaking = game.mob_object_mut(mob).unwrap();
                creaking.add_effect("minecraft:speed", 40, 0);
                if matches!(edit, EffectEdit::ReplaceRegistered) {
                    creaking.add_effect(FINAL_HURT, 1, 3);
                } else {
                    creaking.add_effect("minecraft:instant_damage", 1, 0);
                }
                creaking.add_effect("minecraft:fire_resistance", 40, 0);
                None
            } else if case == "sting" {
                for command in [
                    "summon pig 4.5 64 4.9 {UUID:[I;86,0,0,3],NoAI:1b,NoGravity:1b}",
                    "summon pig 4.5 64 4.1 {UUID:[I;86,0,0,4],NoAI:1b,NoGravity:1b}",
                    "team add sting",
                    "team modify sting friendlyFire false",
                    "team join sting 00000056-0000-0000-0000-000000000002",
                ] {
                    assert!(game.run_command(command), "{command}");
                }
                None
            } else {
                let y = game.mob_object(mob).unwrap().eye_y().floor() as i32;
                assert!(game.run_command(&format!("setblock 4 {y} 4 minecraft:stone")));
                None
            };

            game.server.lan_entity_events.clear();
            game.start_audio_capture();
            TEST_POSTED_EVENT_ORDER.with(|order| *order.borrow_mut() = Some(Vec::new()));
            let live_reaction = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let flap = (mob as i32, 66);
                let flashes = game
                    .server
                    .lan_entity_events
                    .iter()
                    .filter(|&&entry| entry == flap)
                    .count();
                let sways = game
                    .server
                    .pending_sounds
                    .iter()
                    .filter(|sound| sound.event == "minecraft:entity.creaking.sway")
                    .count();
                (flashes, sways)
            };
            let parity_output = (case == "expiry")
                .then(|| std::env::var_os("SUNRISE_CREAKING_BORROWED_OUTPUT"))
                .flatten();
            let snapshot = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let (flashes, _) = live_reaction(game);
                let action = TEST_POSTED_EVENT_ORDER.with(|order| {
                    order
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .iter()
                        .filter_map(|(event, source, flashes, sounds)| {
                            (*event == "minecraft:entity_action" && *source == Some(Who::Mob(mob)))
                                .then_some([*flashes, *sounds])
                        })
                        .collect::<Vec<_>>()
                });
                let creaking = game.mob_object(mob).unwrap();
                let Some(BlockEntity::CreakingHeart(heart)) =
                    game.world.block_entities.get(&(8, 64, 8))
                else {
                    panic!("missing heart");
                };
                let sounds = game
                    .server
                    .pending_sounds
                    .iter()
                    .filter_map(|s| {
                        (s.event == "minecraft:entity.creaking.sway").then_some(&s.event)
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "action": action, "flashes": flashes, "sounds": sounds, "emitter": heart.emitter,
                    "animation": creaking.extra.ints[0], "remembered": game.server.last_hurt_by_player.contains_key(&mob),
                    "clock": creaking.ai.last_hurt_by_player,
                })
            };
            let start = parity_output.as_ref().map(|_| snapshot(game));
            game.tick_creature_entities().unwrap();
            let after_hurt = parity_output.as_ref().map(|_| snapshot(game));
            let reacted = usize::from(!matches!(case.as_str(), "offline" | "mismatched_protector"));
            let swayed = if case == "protector" { 0 } else { reacted };
            let reaction = live_reaction(game);
            let creaking = game.mob_object(mob).unwrap_or_else(|| {
                let residents: Vec<_> = game.mobs.iter().map(|m| (m.id, &m.kind.id, m.position)).collect();
                panic!("{case}: Creaking {mob} left the level, reaction {reaction:?}, resident mobs {residents:?}");
            });
            assert_eq!(
                reaction,
                (reacted, swayed),
                "{case}: at {:?}, velocity {:?}",
                creaking.position,
                creaking.velocity
            );
            assert_eq!(creaking.alive(), case != "mismatched_protector");
            if let Some(edit) = effect_edit {
                let duration = |id| {
                    creaking
                        .extra
                        .effects
                        .iter()
                        .find(|effect| effect.id == id)
                        .map(|effect| effect.duration)
                };
                let (speed, damage) = match edit {
                    EffectEdit::RemoveEarlier => (None, Some(0)),
                    EffectEdit::ReplaceCurrent => (Some(39), Some(40)),
                    EffectEdit::InsertAndRemove => (Some(39), Some(0)),
                    EffectEdit::ReplaceRegistered => (Some(39), None),
                };
                assert_eq!(duration("minecraft:speed"), speed);
                assert_eq!(duration("minecraft:instant_damage"), damage);
                assert_eq!(duration("minecraft:glowing"), None);
                if matches!(edit, EffectEdit::ReplaceRegistered) {
                    assert_eq!(heard.lock().unwrap().expired_amplifiers, [3]);
                    let replacement = creaking
                        .extra
                        .effects
                        .iter()
                        .find(|e| e.id == FINAL_HURT)
                        .unwrap();
                    assert_eq!((replacement.duration, replacement.amplifier), (40, 0));
                }
                assert_eq!(
                    duration("minecraft:fire_resistance"),
                    Some(40),
                    "the structurally changed pass stops after the current instance"
                );
            }
            if case == "sting" {
                let poison = |m: &sunrise_sim::mobs::Mob| {
                    m.extra
                        .effects
                        .iter()
                        .find(|e| e.id == "minecraft:poison")
                        .map(|e| e.duration)
                };
                assert_eq!(
                    poison(creaking),
                    Some(119),
                    "the first accepted hit keeps its own puff state"
                );
                let pig = |uuid| {
                    game.mob_object(game.mob_by_uuid([86, 0, 0, uuid]).unwrap())
                        .unwrap()
                };
                assert_eq!(pig(3).health, 10.0, "the newly allied pig is not stung");
                assert_eq!(poison(pig(3)), None);
                assert_eq!(
                    pig(4).health,
                    8.0,
                    "the later touch reads the new puff state"
                );
                assert_eq!(poison(pig(4)), Some(59));
                let sounds: Vec<_> = game
                    .server
                    .pending_sounds
                    .iter()
                    .filter(|s| s.event == "minecraft:entity.puffer_fish.sting")
                    .map(|s| s.position)
                    .collect();
                assert_eq!(sounds, [FISH_AFTER_HURT, FISH_AFTER_HURT]);
            }
            let phase = if peer.is_some() { 8 } else { 7 };
            assert_eq!(creaking.extra.ints[0], if reacted != 0 { phase } else { 0 });
            let first = heard.lock().unwrap().reactions.clone();
            assert_eq!(first.len(), reacted);
            if let Some(peer) = peer {
                let hit = first[0];
                assert!(
                    hit.position[0] > 5.0,
                    "the mob has travelled when cramming hurts it"
                );
                assert_eq!(
                    hit.peer_velocity,
                    Some([0.0; 3]),
                    "pushes happen after the hurt"
                );
                assert!(
                    game.mob_object(peer).unwrap().velocity.x < 0.0,
                    "resume pushes the selected peer using its new position"
                );
            }
            let heart = game.world.block_entities.get(&(8, 64, 8));
            if case == "protector" {
                let state = game.world.block(8, 64, 8);
                assert_eq!(
                    game.pack.assets.registry.state(state).unwrap().name,
                    "minecraft:air"
                );
                // The owning block-entity tick can still hold the stale row;
                // replacing the block already stopped it from responding.
                if let Some(BlockEntity::CreakingHeart(heart)) = heart {
                    assert_eq!(heart.emitter, 0);
                }
            } else {
                let Some(BlockEntity::CreakingHeart(heart)) = heart else {
                    panic!("the protecting heart is gone");
                };
                assert_eq!(heart.emitter, 100 * swayed as i32);
            }
            TEST_POSTED_EVENT_ORDER.with(|order| {
                let order = order.borrow();
                let action: Vec<_> = order
                    .as_ref()
                    .unwrap()
                    .iter()
                    .filter(|(event, source, ..)| {
                        *event == "minecraft:entity_action" && *source == Some(Who::Mob(mob))
                    })
                    .collect();
                assert_eq!(action.len(), reacted);
                if let Some((_, _, flashes, sounds)) = action.first() {
                    assert_eq!((*flashes, *sounds), (1, 0));
                }
            });
            if case == "expiry" {
                assert!(!game.server.last_hurt_by_player.contains_key(&mob));
                // Remove cooldown, but leave the wall in place. With no reference
                // there is neither a replay nor another environmental reaction.
                game.mob_object_mut(mob).unwrap().extra.ints[0] = 0;
            }
            game.tick_creature_entities().unwrap();
            assert_eq!(live_reaction(game), (reacted, swayed));
            assert_eq!(heard.lock().unwrap().reactions.len(), reacted);
            if matches!(effect_edit, Some(EffectEdit::ReplaceRegistered)) {
                assert_eq!(heard.lock().unwrap().expired_amplifiers, [3]);
            }
            if let Some(path) = parity_output {
                let trace = [start.unwrap(), after_hurt.unwrap(), snapshot(game)];
                std::fs::write(path, serde_json::to_vec(&trace).unwrap()).unwrap();
            }
            TEST_POSTED_EVENT_ORDER.with(|order| *order.borrow_mut() = None);
        })
    });
}
