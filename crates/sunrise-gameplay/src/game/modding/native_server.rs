#[cfg(any(test, feature = "test-support"))]
pub mod fake_action_tests {
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    #[cfg(test)]
    use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;

    #[cfg(test)]
    use sunrise_gameplay_common::game::native_server::fake_action_tests::on_top;

    #[cfg(test)]
    use mod_api::{ActionOutcome, ActionRefused, FakeEffect, PlayerAction};

    use mod_api::PlayerRef;

    #[cfg(test)]
    use mod_api::Hand;

    use sunrise_items::items::ItemStack;

    /// A server with a real guest (uuid 1) and a fake (uuid 2) standing at (0.5, 64, 0.5) on
    /// a stone floor, holding `item`, in game mode `mode`: the fake's handle and both conns.
    pub fn fake(
        game: &mut crate::game::ServerGame<'_>,
        item: Option<&str>,
        mode: i32,
    ) -> (PlayerRef, u32, u32) {
        crate::game::server_side::bossbars::audience_tests::add_real_and_fake(game);
        game.options.dedicated = Some(Vec::new());
        for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
            let mut chunk = sunrise_level::chunk::Chunk::empty(x, z);
            chunk.lit = true;
            game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
        }
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in -3..=3 {
            for z in -3..=3 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        let level = game.level(game.dim.id()).unwrap();
        let player = game.fake_player(level, 2, "fake").unwrap();
        let host = game.server.lan.as_mut().unwrap();
        let conn = |uuid| {
            host.hub
                .guests
                .iter()
                .find(|g| g.uuid == uuid)
                .unwrap()
                .conn
        };
        let (fake, real) = (conn(2), conn(1));
        host.hub.guests.update_all(|g| {
            g.position = [0.5, 64.0, 0.5];
            g.game_mode = mode;
            g.abilities.update_for_mode(mode);
            g.inventory[g.selected] = item.map(|i| ItemStack::new(i, 1));
        });
        (player, fake, real)
    }
    pub fn place(game: &mut crate::game::ServerGame<'_>, pos: [i32; 3], state: &str) {
        // `name[key=value,...]`, as a command writes a state.
        let (name, props) = state.split_once('[').unwrap_or((state, "]"));
        let properties = props
            .trim_end_matches(']')
            .split(',')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let state = sunrise_world::chunk::BlockState {
            name: name.into(),
            properties,
        };
        let id = game.pack.assets.registry.resolve(&state).unwrap();
        game.world.set(pos[0], pos[1], pos[2], id).unwrap();
    }
    pub fn held(game: &crate::game::ServerGame<'_>, uuid: u128) -> Option<ItemStack> {
        let host = game.server.lan.as_ref().unwrap();
        let g = host.hub.guests.iter().find(|g| g.uuid == uuid).unwrap();
        g.inventory[g.selected].clone()
    }
    /// A wandering trader at `at`, entity id `id`, and its handle.
    pub fn trader(
        game: &mut crate::game::ServerGame<'_>,
        id: u32,
        at: [f64; 3],
    ) -> mod_api::EntityRef {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:wandering_trader")
            .unwrap()
            .clone();
        let [x, y, z] = at;
        game.mobs.push(sunrise_sim::mobs::Mob::new(
            id,
            kind,
            sunrise_core::geometry::Vec3::new(x, y, z),
            0.0,
        ));
        mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id)
    }
    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn a_fake_uses_items_through_the_players_own_path() {
        use crate::game::native_entity_refs::tests::with_game;
        // A log on stone: placed, one used up.
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:oak_log"), 0);
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Ok(ActionOutcome::Consumed)
            );
            let level = game.level(game.dim.id()).unwrap();
            let log = game.level_block_property(level, [1, 64, 0], "axis");
            assert_eq!(log, Some("y"), "placed on a top face, standing");
            assert_eq!(held(game, 2), None);
        });
        // A bucket clicked on a block passes, as vanilla's does (BucketItem has no useOn), and
        // empties when used, by its ray from the eyes (looking straight down here).
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:water_bucket"), 0);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update_all(|g| {
                    g.pitch = 90.0;
                });
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Ok(ActionOutcome::Passed)
            );
            let water = PlayerAction::UseItem { hand: Hand::Main };
            assert_eq!(
                game.player_perform_action(player, water),
                Ok(ActionOutcome::Consumed)
            );
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                game.level_block(level, [0, 64, 0])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:water")
            );
            assert_eq!(
                held(game, 2).map(|s| s.item.to_string()).as_deref(),
                Some("minecraft:bucket")
            );
        });
        // A pearl thrown into the air: one used up.
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:ender_pearl"), 0);
            let pearl = PlayerAction::UseItem { hand: Hand::Main };
            assert_eq!(
                game.player_perform_action(player, pearl),
                Ok(ActionOutcome::Consumed)
            );
            assert_eq!(held(game, 2), None);
        });
    }
    /// A zombie standing at `at`, entity id `id`, and its handle.
    pub fn zombie(
        game: &mut crate::game::ServerGame<'_>,
        id: u32,
        at: [f64; 3],
    ) -> mod_api::EntityRef {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let [x, y, z] = at;
        game.mobs.push(sunrise_sim::mobs::Mob::new(
            id,
            kind,
            sunrise_core::geometry::Vec3::new(x, y, z),
            0.0,
        ));
        mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id)
    }
    #[cfg(test)]
    pub(in crate::game) fn health(game: &crate::game::ServerGame<'_>, id: u32) -> f32 {
        game.mobs.iter().find(|m| m.id == id).unwrap().health
    }
    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn a_fake_attacks_through_the_players_own_path() {
        use crate::game::native_entity_refs::tests::with_game;
        with_game(|game| {
            let (player, fake_conn, _) = fake(game, None, 0);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update_all(|g| {
                    g.yaw = -90.0;
                });
            // A fist does 1 (the attack damage attribute's base), scaled by the attack strength
            // of a player that hasn't waited (`getAttackStrengthScale(0.5)`: 0.5 of a 5-tick
            // delay, 0.1, so 0.2 + 0.1² × 0.8 = 0.208), then by a zombie's 2 armour
            // (`CombatRules.getDamageAfterAbsorb`: 1 - (2 - 0.208 / 2) / 25).
            let near = zombie(game, 5000, [2.0, 64.0, 0.5]);
            let full = health(game, 5000);
            assert_eq!(
                game.player_perform_action(player, PlayerAction::Attack { target: near }),
                Ok(ActionOutcome::Consumed)
            );
            let dealt = 0.208f32 * (1.0 - (2.0 - 0.208 / 2.0) / 25.0);
            assert!((full - health(game, 5000) - dealt).abs() < 1e-4);
            // No mob takes the fake as the player to answer.
            let bound = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .mob_players
                .get(&5000)
                .copied();
            assert_ne!(bound, Some(fake_conn));
            assert_eq!(bound, None);
            // Ten blocks off is past the attack range; the fake itself isn't a target, nor is
            // an id no entity has.
            let far = zombie(game, 5001, [10.5, 64.0, 0.5]);
            assert_eq!(
                game.player_perform_action(player, PlayerAction::Attack { target: far }),
                Err(ActionRefused::OutOfReach)
            );
            assert_eq!(health(game, 5001), full);
            assert_eq!(
                game.player_perform_action(
                    player,
                    PlayerAction::Attack {
                        target: player.entity()
                    }
                ),
                Err(ActionRefused::InvalidTarget)
            );
            let nobody = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 6000);
            assert_eq!(
                game.player_perform_action(player, PlayerAction::Attack { target: nobody }),
                Err(ActionRefused::InvalidTarget)
            );
        });
    }
    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn a_fake_breaks_blocks_at_once_by_the_game_modes_rules() {
        use crate::game::native_entity_refs::tests::with_game;
        let level_block = |game: &mut crate::game::ServerGame<'_>, pos| {
            let level = game.level(game.dim.id()).unwrap();
            game.level_block(level, pos)
        };
        // Survival, bare-handed: stone breaks at once, with no digging time.
        with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            let pos = [1, 63, 0];
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos }),
                Ok(ActionOutcome::Consumed)
            );
            assert_eq!(
                level_block(game, pos).and_then(mod_api::__private::block_key_id),
                Some("minecraft:air")
            );
            // Air breaks nothing; bedrock, which no digging breaks (destroy time -1), stays;
            // ten blocks off is out of reach.
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos }),
                Ok(ActionOutcome::Passed)
            );
            place(game, [-1, 63, 0], "minecraft:bedrock");
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: [-1, 63, 0] }),
                Ok(ActionOutcome::Passed)
            );
            assert_eq!(
                level_block(game, [-1, 63, 0]).and_then(mod_api::__private::block_key_id),
                Some("minecraft:bedrock")
            );
            place(game, [3, 64, 3], "minecraft:stone");
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update_all(|g| {
                    g.position = [-6.5, 64.0, -6.5];
                });
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: [3, 64, 3] }),
                Err(ActionRefused::OutOfReach)
            );
        });
        // Creative breaks what survival can't dig.
        with_game(|game| {
            let (player, ..) = fake(game, None, 1);
            place(game, [1, 63, 0], "minecraft:bedrock");
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: [1, 63, 0] }),
                Ok(ActionOutcome::Consumed)
            );
            assert_eq!(
                level_block(game, [1, 63, 0]).and_then(mod_api::__private::block_key_id),
                Some("minecraft:air")
            );
        });
        // Adventure mode can't break a block (blockActionRestricted), and a sword in creative
        // breaks nothing (canDestroyBlock).
        for (mode, item) in [(2, None), (1, Some("minecraft:iron_sword"))] {
            with_game(|game| {
                let (player, ..) = fake(game, item, mode);
                let pos = [1, 63, 0];
                assert_eq!(
                    game.player_perform_action(player, PlayerAction::BreakBlock { pos }),
                    Ok(ActionOutcome::Passed),
                    "game mode {mode}, holding {item:?}"
                );
                assert_eq!(
                    level_block(game, pos).and_then(mod_api::__private::block_key_id),
                    Some("minecraft:stone")
                );
            });
        }
    }
    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn a_fakes_effects_are_stopped_where_they_start() {
        use crate::game::native_entity_refs::tests::with_game;
        // A chest's menu (Game::open_menu).
        with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            place(game, [1, 64, 0], "minecraft:chest");
            assert_eq!(
                game.player_perform_action(player, on_top([1, 64, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
        });
        // A bed (Game::try_sleep).
        with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            place(
                game,
                [1, 64, 0],
                "minecraft:red_bed[facing=north,part=foot]",
            );
            place(
                game,
                [1, 64, -1],
                "minecraft:red_bed[facing=north,part=head]",
            );
            assert_eq!(
                game.player_perform_action(player, on_top([1, 64, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Sleep))
            );
        });
        // A spectator's chest, the menu that bypasses Game::open_menu (spectator_open_menu).
        with_game(|game| {
            let (player, ..) = fake(game, None, 3);
            place(game, [1, 64, 0], "minecraft:chest");
            assert_eq!(
                game.player_perform_action(player, on_top([1, 64, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
        });
        // A sign placed opens no editor (open_sign_editor), but the sign is placed.
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:oak_sign"), 0);
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                game.level_block(level, [1, 64, 0])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:oak_sign")
            );
        });
        // Boarding (native_may_board, every boarding path's gate): refused for the fake only.
        with_game(|game| {
            let (_, fake_conn, real_conn) = fake(game, None, 0);
            assert_eq!(
                game.as_guest(fake_conn, |g| g.native_may_board(99)),
                Some(false)
            );
            assert_eq!(
                game.as_guest(real_conn, |g| g.native_may_board(99)),
                Some(true)
            );
        });
    }
}
