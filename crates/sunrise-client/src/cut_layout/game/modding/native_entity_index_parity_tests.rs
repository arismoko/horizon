#![cfg(test)]

use super::native_entity_refs::tests::with_game;
use std::collections::HashSet;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::creatures::{SpawnReason, SpawnSite};
use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
use sunrise_sim::join_seq::residency::table::EntityLookup;

fn live_mob_ids(game: &sunrise_gameplay::game::ServerGame<'_>, expected: usize) -> HashSet<u32> {
    let mut ids = Vec::new();
    ids.extend(game.mobs.iter().map(|mob| mob.id));
    for dimension in [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ] {
        if let Some(parked) = game.server.levels.0.parked_of(dimension) {
            ids.extend(parked.mobs.iter().map(|mob| mob.id));
        }
    }
    assert_eq!(ids.len(), expected, "no live body lost during travel");
    assert!(ids.iter().all(|&id| id != 0));
    let unique: HashSet<_> = ids.iter().copied().collect();
    assert_eq!(
        unique.len(),
        expected,
        "live runtime IDs never collide across levels"
    );
    for &id in &unique {
        assert!(
            matches!(game.server_resident_entity(id), EntityLookup::Found { value: Target::Mob(found), .. } if found == id)
        );
    }
    unique
}

#[test]
#[ignore = "daily"]
fn runtime_id_allocator_remains_unique_across_loaded_levels_and_real_transfer() {
    with_game(|game| {
        game.server.world_root = game.server.save.0.root.clone();
        game.native_level_opened(Dim::Overworld);
        game.native_lifecycle.world_open_ready().unwrap();
        let mut spawned = Vec::new();
        for _ in 0..3 {
            for dimension in [Dim::Overworld, Dim::Nether, Dim::End] {
                game.enter_level(dimension).unwrap();
                fixture_resident_column(&mut game.world, (0, 0));
                let id = game
                    .spawn_mob(
                        "minecraft:cow",
                        [0.5, 80.0, 0.5],
                        true,
                        SpawnSite::Finalizing(SpawnReason::Command),
                    )
                    .unwrap();
                assert!(
                    !spawned.contains(&id),
                    "allocation cannot reuse any still-live ID"
                );
                spawned.push(id);
                live_mob_ids(game, spawned.len());
            }
        }
        game.enter_level(Dim::Overworld).unwrap();
        let entity = game.entity(spawned[0]).unwrap();
        assert!(game.entity_teleport_to_level(
            entity,
            "minecraft:the_nether",
            [2.5, 80.0, 2.5],
            None
        ));
        let live = live_mob_ids(game, spawned.len());
        for dimension in [Dim::Overworld, Dim::Nether, Dim::End] {
            game.enter_level(dimension).unwrap();
            let id = game
                .spawn_mob(
                    "minecraft:cow",
                    [4.5, 80.0, 4.5],
                    true,
                    SpawnSite::Finalizing(SpawnReason::Command),
                )
                .unwrap();
            assert!(
                !live.contains(&id),
                "post-transfer allocation cannot reuse a live ID"
            );
            spawned.push(id);
            live_mob_ids(game, spawned.len());
        }
    });
}

#[test]
#[ignore = "daily"]
fn cushion_get_entity_matches_vanilla_membership_oracle() {
    with_game(|game| {
        game.summon_with_nbt("minecraft:cushion", [0.5, 70.0, 0.5], "{}")
            .unwrap();
        game.lan_assign_ids();
        let id = game.cushions[0].id;
        assert!(
            matches!(game.level_entity(id as i32), Some(Target::Cushion(found)) if found == id)
        );
        assert!(game.level_entity(0).is_none());
        assert!(game.level_entity(i32::MAX).is_none());
        game.enter_level(Dim::Nether).unwrap();
        assert!(game.level_entity(id as i32).is_none());
        game.enter_level(Dim::Overworld).unwrap();
        let cushion = game.cushions.remove(0);
        assert!(game.level_entity(id as i32).is_none());
        game.enter_level(Dim::Nether).unwrap();
        game.cushions.push(cushion);
        assert!(
            matches!(game.level_entity(id as i32), Some(Target::Cushion(found)) if found == id)
        );
        game.enter_level(Dim::Overworld).unwrap();
        assert!(game.level_entity(id as i32).is_none());
    });
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
