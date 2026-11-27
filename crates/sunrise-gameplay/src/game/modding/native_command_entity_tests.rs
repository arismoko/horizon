//! `CommandInvocation::entity`, `rotation` and `player()`: the entity a native command executes
//! as and its rotation (`CommandSourceStack.getEntity`, `getRotation`, `getPlayer`), read from
//! the leaf source of each `execute` fork.
use crate::game::ServerGame;
use mod_api::{CommandInvocation, CommandSource, DimensionKey, EntityRef};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
use sunrise_level::chunk::Chunk;

type Heard = Arc<Mutex<Vec<(CommandSource, CommandInvocation)>>>;

struct Where(Heard);

#[mod_api::__subscriptions]
impl mod_api::Hooks for Where {
    fn register_commands(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RegisterCommandsEvent<'_>,
    ) {
        let heard = self.0.clone();
        event.commands.push(
            mod_api::Command::new("where").executes(move |_, source, command| {
                heard
                    .lock()
                    .unwrap()
                    .push((source.clone(), command.clone()));
                Ok(1)
            }),
        );
    }
}

/// Two guests (12 and 13 at 3 64 5) with the `where` command, and an empty chunk at the
/// origin of the Overworld and the Nether.
fn with_where(test: impl FnOnce(&mut ServerGame<'_>, &Heard)) {
    let heard: Heard = Arc::default();
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "where",
            Box::new(Where(heard.clone())),
        )]
        .into(),
        || {
            super::native_guest_turn_tests::with_guests(|game| {
                game.ensure_datapacks();
                game.build_registries();
                game.register_native_commands();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let back = game.enter_command_level(Dim::Nether);
                assert_eq!(game.dim, Dim::Nether, "the fixture has a Nether");
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                if let Some(back) = back {
                    game.enter_command_level(back);
                }
                test(game, &heard);
            });
        },
    );
}

/// What the `where` executor heard last, emptying the record.
#[track_caller]
fn last(heard: &Heard) -> (CommandSource, CommandInvocation) {
    let mut heard = heard.lock().unwrap();
    let last = heard.pop().expect("the command reached the executor");
    heard.clear();
    last
}

/// The handle of the only mob in `dim`, looked up with that level in place.
fn only_mob(game: &mut ServerGame<'_>, dim: Dim) -> EntityRef {
    let back = game.enter_command_level(dim);
    let id = game.mobs.iter().next().expect("the mob was summoned").id;
    let entity = game.entity(id).expect("a summoned mob has a handle");
    if let Some(back) = back {
        game.enter_command_level(back);
    }
    entity
}

fn set_rotation(game: &mut ServerGame<'_>, conn: u32, yaw: f32, pitch: f32) {
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(conn, |guest| {
            guest.yaw = yaw;
            guest.pitch = pitch;
            guest.permission = 4;
        })
        .unwrap();
}

/// The console and a player, forked through `as` (a player, a mob, a mob in another level)
/// and combined with `at`, `rotated` and `facing`: the entity and rotation follow the fork,
/// the origin does not.
#[test]
#[ignore = "daily"]
fn execute_as_at_rotated_and_facing_set_the_entity_and_rotation() {
    sunrise_gameplay_common::game::in_own_process(
        concat!(
            module_path!(),
            "::execute_as_at_rotated_and_facing_set_the_entity_and_rotation"
        )
        .split_once("::")
        .unwrap()
        .1,
        || {
            with_where(|game, heard| {
                let acting = game.player(12).unwrap();
                let other = game.player(13).unwrap();
                set_rotation(game, 12, 12.0, -8.0);
                set_rotation(game, 13, 50.0, 20.0);
                assert!(
                    game.native_run_command("summon minecraft:pig 0.5 64 0.5 {Tags:[\"here\"]}")
                );
                assert!(game.native_run_command(
                    "execute in minecraft:the_nether run summon minecraft:pig 0.5 64 0.5 {Tags:[\"there\"]}"
                ));
                let pig = only_mob(game, Dim::Overworld);
                let nether_pig = only_mob(game, Dim::Nether);

                // The console: no entity, zero rotation.
                assert!(game.native_run_command("where"));
                let (source, command) = last(heard);
                assert!(
                    matches!(source, CommandSource::Console { .. }),
                    "{source:?}"
                );
                assert_eq!((command.entity, command.player()), (None, None));
                assert_eq!(command.rotation, [0.0, 0.0]);

                // `as` a player changes only the entity.
                assert!(game.native_run_command("execute as guest13 run where"));
                let (source, command) = last(heard);
                assert!(
                    matches!(source, CommandSource::Console { .. }),
                    "{source:?}"
                );
                assert_eq!(command.entity, Some(other.entity()));
                assert_eq!(command.player(), Some(other));
                assert_eq!(command.rotation, [0.0, 0.0]);
                assert_eq!(command.position, game.server_position());

                // `at` takes the player's position and [yaw, pitch]; `rotated` replaces it.
                assert!(game.native_run_command("execute as guest13 at @s run where"));
                let (_, command) = last(heard);
                assert_eq!(command.player(), Some(other));
                assert_eq!(command.position, [3.0, 64.0, 5.0]);
                assert_eq!(command.rotation, [50.0, 20.0]);
                assert!(
                    game.native_run_command("execute as guest13 at @s rotated 30 -10 run where")
                );
                let (_, command) = last(heard);
                assert_eq!(command.player(), Some(other));
                assert_eq!(command.rotation, [30.0, -10.0]);

                // `as` a mob: an entity that is not a player.
                assert!(game.native_run_command(
                    "execute as @e[tag=here] at @s facing 1.5 64 -0.5 run where"
                ));
                let (source, command) = last(heard);
                assert!(
                    matches!(source, CommandSource::Console { .. }),
                    "{source:?}"
                );
                assert_eq!((command.entity, command.player()), (Some(pig), None));
                assert_eq!(command.position, [0.5, 64.0, 0.5]);
                // One block +x and -z from the pig's feet: yaw -135 and level, as vanilla's
                // `CommandSourceStack.facing` gives it (`execute_facing_tests`).
                assert_eq!(command.rotation, [-135.0, 0.0]);

                // A target in another level: `as` alone keeps the console's level.
                assert!(game.native_run_command("execute as @e[tag=there] run where"));
                let (_, command) = last(heard);
                assert_eq!((command.entity, command.player()), (Some(nether_pig), None));
                assert_eq!(command.dimension, DimensionKey::OVERWORLD);
                // `at` a player in another level executes in that level.
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(13, |guest| {
                        guest.dimension = "minecraft:the_nether".to_owned();
                    })
                    .unwrap();
                assert!(game.native_run_command("execute as guest13 at @s run where"));
                let (_, command) = last(heard);
                assert_eq!(command.player(), Some(other));
                assert_eq!(command.dimension, DimensionKey::NETHER);
                assert_eq!(command.position, [3.0, 64.0, 5.0]);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(13, |guest| {
                        guest.dimension = "minecraft:overworld".to_owned();
                    })
                    .unwrap();

                // A player's own command executes as them, from their rotation.
                for line in [
                    "where",
                    "execute as @e[tag=here] at @s rotated 10 20 run where",
                    "execute as guest13 positioned 0.5 64 0.5 facing 1.5 64 1.5 run where",
                ] {
                    game.guest_command(12, line, None);
                }
                game.run_guest_commands();
                let heard = std::mem::take(&mut *heard.lock().unwrap());
                assert_eq!(heard.len(), 3);
                for (source, _) in &heard {
                    assert!(
                        matches!(source, CommandSource::Player { player, .. } if *player == acting),
                        "the origin stays the submitting player: {source:?}"
                    );
                }
                assert_eq!(heard[0].1.entity, Some(acting.entity()));
                assert_eq!(heard[0].1.player(), Some(acting));
                assert_eq!(heard[0].1.rotation, [12.0, -8.0]);
                assert_eq!((heard[1].1.entity, heard[1].1.player()), (Some(pig), None));
                assert_eq!(heard[1].1.rotation, [10.0, 20.0]);
                // One block +x and +z: vanilla's `Mth.atan2` table gives -45.000004
                // (`execute_facing_tests`), level.
                assert_eq!(heard[2].1.player(), Some(other));
                assert_eq!(heard[2].1.rotation[0].to_bits(), 0xc234_0001);
                assert_eq!(heard[2].1.rotation[1], 0.0);
            });
        },
    );
}

/// A command block starts with no entity at its facing, a command-block minecart and a player-run
/// function as themselves; a function keeps its caller's entity and rotation; a target that no
/// longer resolves is None.
#[test]
#[ignore = "daily"]
fn blocks_carts_and_functions_keep_their_entity() {
    sunrise_gameplay_common::game::in_own_process(
        concat!(
            module_path!(),
            "::blocks_carts_and_functions_keep_their_entity"
        )
        .split_once("::")
        .unwrap()
        .1,
        || {
            use sunrise_core::{geometry::Vec3, position::BlockPos};
            use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
            use sunrise_level::{
                chunk::block_entity::{BlockEntity, CommandBlock},
                datapack::{Function, FunctionLine},
            };
            with_where(|game, heard| {
                let acting = game.player(12).unwrap();
                let other = game.player(13).unwrap();
                set_rotation(game, 12, 12.0, -8.0);

                // A command block: no entity; its rotation faces its `facing` (north: yaw 180,
                // `Direction.toYRot`), the rotation its local coordinates use.
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:command_block")
                    .unwrap();
                let block = |game: &mut ServerGame<'_>, x: i32, line: &str| {
                    game.world.set(x, 64, 1, state).unwrap();
                    game.world.block_entities.insert(
                        (x, 64, 1),
                        BlockEntity::CommandBlock(Box::new(CommandBlock {
                            command: line.into(),
                            condition_met: true,
                            ..Default::default()
                        })),
                    );
                    game.command_block_tick(BlockPos::new(x, 64, 1));
                    last(heard)
                };
                let (source, command) = block(game, 1, "where");
                assert!(
                    matches!(source, CommandSource::CommandBlock { .. }),
                    "{source:?}"
                );
                assert_eq!((command.entity, command.player()), (None, None));
                assert_eq!(command.rotation, [180.0, 0.0]);
                let (_, command) = block(game, 2, "execute positioned ^ ^ ^1 run where");
                assert_eq!(command.rotation, [180.0, 0.0]);
                // One block north of the block's centre.
                assert_eq!(command.position, [2.5, 64.5, 0.5]);
                let (_, command) = block(game, 3, "execute rotated ~10 ~ run where");
                assert_eq!(command.rotation, [190.0, 0.0]);

                // A command-block minecart executes as itself, at its yaw.
                let mut cart = Minecart::new(99, CartKind::CommandBlock, Vec3::new(1.0, 64.0, 1.0));
                cart.yaw = 30.0;
                cart.command.command = "where".into();
                game.carts.push(cart);
                assert!(game.perform_cart_command(99));
                let (source, command) = last(heard);
                let CommandSource::CommandBlockMinecart { entity, .. } = source else {
                    panic!("{source:?}")
                };
                assert_eq!(entity.id(), 99);
                assert_eq!((command.entity, command.player()), (Some(entity), None));
                assert_eq!(command.rotation, [30.0, 0.0]);

                let function = Function {
                    id: "test:where".into(),
                    lines: vec![FunctionLine::Command("where".into())],
                    parameters: vec![],
                };
                game.server
                    .fns
                    .0
                    .store
                    .functions
                    .insert(function.id.clone(), function.clone());
                game.server.fns.0.store.functions.insert(
                    "test:vanish".into(),
                    Function {
                        id: "test:vanish".into(),
                        lines: vec![
                            FunctionLine::Command("kill @s".into()),
                            FunctionLine::Command("where".into()),
                        ],
                        parameters: vec![],
                    },
                );

                // A function from the server has no entity; one forked through `as` and
                // `rotated` keeps that entity and rotation.
                game.run_function(&function, None, game.server_source())
                    .unwrap();
                let (source, command) = last(heard);
                assert!(
                    matches!(source, CommandSource::Function { .. }),
                    "{source:?}"
                );
                assert_eq!((command.entity, command.player()), (None, None));
                assert!(
                    game.native_run_command(
                        "execute as guest13 rotated 5 6 run function test:where"
                    )
                );
                let (source, command) = last(heard);
                assert!(
                    matches!(source, CommandSource::Function { .. }),
                    "{source:?}"
                );
                assert_eq!(command.player(), Some(other));
                assert_eq!(command.rotation, [5.0, 6.0]);

                // A player-run function executes as that player, at their rotation.
                game.guest_command(12, "function test:where", None);
                game.run_guest_commands();
                let (source, command) = last(heard);
                assert!(
                    matches!(source, CommandSource::Function { .. }),
                    "{source:?}"
                );
                assert_eq!(command.entity, Some(acting.entity()));
                assert_eq!(command.player(), Some(acting));
                assert_eq!(command.rotation, [12.0, -8.0]);

                // An entity killed before the command runs no longer resolves: None, not the
                // player in place.
                assert!(game.native_run_command(
                    "summon minecraft:item 3.5 64 5.5 {Item:{id:\"minecraft:stone\",count:1}}"
                ));
                game.guest_command(
                    12,
                    "execute as @e[type=minecraft:item] run function test:vanish",
                    None,
                );
                game.run_guest_commands();
                let (_, command) = last(heard);
                assert_eq!((command.entity, command.player()), (None, None));
            });
        },
    );
}

/// `execute on origin` of a fake player's projectile executes as that fake player, and selects
/// nothing once the fake is gone.
#[test]
#[ignore = "daily"]
fn execute_on_origin_reaches_a_fake_players_projectile() {
    sunrise_gameplay_common::game::in_own_process(
        concat!(
            module_path!(),
            "::execute_on_origin_reaches_a_fake_players_projectile"
        )
        .split_once("::")
        .unwrap()
        .1,
        || {
            use sunrise_core::geometry::Vec3;
            use sunrise_sim::{
                mobs::Who,
                projectiles::{Projectile, ProjectileKind},
            };
            with_where(|game, heard| {
                let level = game.level(game.dim.id()).unwrap();
                let fake = game.fake_player(level, 77, "machine").unwrap();
                assert_eq!(game.player(77), None, "a fake is not an online player");
                let conn = game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guests
                    .iter()
                    .find(|guest| guest.uuid == 77)
                    .unwrap()
                    .conn;
                let mut snowball = Projectile::new(
                    ProjectileKind::Snowball,
                    Vec3::new(0.5, 64.0, 0.5),
                    Some(Who::Player),
                );
                game.server.next_entity += 1;
                snowball.id = game.server.next_entity;
                snowball.guest = Some(conn);
                game.creatures.projectiles.push(snowball);

                let line = "execute as @e[type=minecraft:snowball] on origin run where";
                assert!(game.native_run_command(line));
                let (_, command) = last(heard);
                assert_eq!(command.entity, Some(fake.entity()));
                assert_eq!(command.player(), Some(fake));

                // Removed, the fake is no owner: `on origin` selects nothing (vanilla's
                // `getOwner()` is null), so the executor never runs as the player in place.
                assert!(game.player_remove_fake(fake));
                assert!(game.native_run_command(line));
                assert!(heard.lock().unwrap().is_empty());
            });
        },
    );
}
