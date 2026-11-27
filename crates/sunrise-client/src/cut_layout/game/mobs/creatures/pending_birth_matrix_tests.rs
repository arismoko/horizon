//! Test pending children through real splitting and breeding with a native listener.

use super::pending_birth_tests::{SPLIT_DISPLACEMENT, YAWS, find};
use mod_api::{BabyEntitySpawnEvent, Flow, Hooks, MobSplitEvent, ServerHost};
use sunrise_gameplay_common::game::commands_common::Target;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

#[path = "pending_birth_matrix_tests/fixture.rs"]
pub(crate) mod fixture;

pub(crate) struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn mob_split(&self, host: &mut dyn ServerHost, event: &mut MobSplitEvent<'_>) -> Flow {
        fixture::mob_split(host, event)
    }
    fn baby_entity_spawn(
        &self,
        host: &mut dyn ServerHost,
        event: &mut BabyEntitySpawnEvent<'_>,
    ) -> Flow {
        fixture::baby_entity_spawn(host, event)
    }
}

#[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::creatures_server::pending_birth_matrix_tests::split;

#[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::creatures_server::pending_birth_matrix_tests::breed;

pub(crate) fn matrix(mods: mod_api::Mods) {
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            // No change: the vanilla three, in order.
            let kids = split(game, 0);
            assert_eq!(kids.len(), 3);
            assert!(kids.iter().all(|k| k.extra.size == 2));
            assert_eq!(kids[0].yaw, YAWS[0]);
            // Cancel: none.
            assert!(split(game, 1).is_empty());
            // Edit A, remove B, append D: [A, C, D].
            let kids = split(game, 2);
            assert_eq!(kids.len(), 3);
            assert_eq!(kids[0].yaw, 90.0);
            assert_eq!(kids[1].yaw, YAWS[2]);
            assert_eq!(
                kids[1].position.x.to_bits(),
                (10.0 - SPLIT_DISPLACEMENT).to_bits()
            );
            assert_eq!(
                (kids[2].position.x, kids[2].yaw, kids[2].extra.size),
                (3.0, 12.0, 2)
            );
            // Replace C with E (and round-trip insert/remove at index 0).
            let kids = split(game, 3);
            assert_eq!(kids.len(), 3);
            assert_eq!(kids[0].yaw, YAWS[0]);
            assert_eq!(kids[2].kind.id, "minecraft:magma_cube");
            assert_eq!((kids[2].position.x, kids[2].yaw), (8.0, 33.0));
            // Invalid inputs (NaN pos, unknown kind, wrong dimension, out-of-bounds insert/replace,
            // NaN move -> six Invalid=4 digits) refuse cleanly while the unbounded list grows to 70.
            let kids = split(game, 4);
            assert_eq!(kids.len(), 70);
            assert_eq!(kids[0].yaw, 444_444.0);
            // Clearing the list spawns no children.
            assert!(split(game, 5).is_empty());

            // Breeding: untouched baby.
            let (_, _, child) = breed(game, 0);
            assert_eq!(find(game, child.unwrap()).extra.age, -24000);
            // Veto (mode 1): no child, both parents reset to age 6000 and in_love 0.
            let (parent, partner, child) = breed(game, 1);
            assert_eq!(child, None);
            assert!(game.common.mobs.iter().all(|m| m.extra.age >= 0));
            assert_eq!(find(game, parent).extra.age, 6000);
            assert_eq!(find(game, parent).extra.in_love, 0);
            assert_eq!(find(game, partner).extra.age, 6000);
            assert_eq!(find(game, partner).extra.in_love, 0);
            // Uncanceled empty slot (mode 3): no child, parents are NOT cooled down.
            let (parent, partner, child) = breed(game, 3);
            assert_eq!(child, None);
            assert!(game.common.mobs.iter().all(|m| m.extra.age >= 0));
            assert_eq!(find(game, parent).extra.age, 0);
            assert_eq!(find(game, parent).extra.in_love, 600);
            assert_eq!(find(game, partner).extra.age, 0);
            assert_eq!(find(game, partner).extra.in_love, 600);
            // Replace with a pig baby; post-event snap puts it at parent's position with yaw 0,
            // and a detached second baby dropped without attaching never joins.
            let (parent_id, _, child) = breed(game, 2);
            let parent = find(game, parent_id);
            let pig = find(game, child.unwrap());
            assert_eq!(pig.kind.id, "minecraft:pig");
            assert!(pig.extra.age < 0);
            assert_eq!(pig.position, parent.position);
            assert_eq!(pig.yaw, 0.0);
            assert_eq!(pig.head_pitch, 0.0);
            assert!(
                game.tags_of(Target::Mob(pig.id))
                    .iter()
                    .any(|t| t == "replaced_pig")
            );
            assert_eq!(game.common.mobs.len(), 3);
            // A grown replacement is accepted: `Animal.spawnChildFromBreeding` calls
            // `setBaby(true)` on whatever child the event leaves.
            let (_, _, child) = breed(game, 4);
            let baby = find(game, child.unwrap());
            assert_eq!(baby.kind.id, "minecraft:pig");
            assert_eq!(baby.extra.age, -24000);
            assert_eq!(baby.yaw, 0.0);
            assert!(
                game.tags_of(Target::Mob(baby.id))
                    .iter()
                    .any(|t| t == "grown_pig")
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn native_pending_children_shared_matrix() {
    if run_fresh_cases(
        "cut_layout::game::creatures::pending_birth_matrix_tests::native_pending_children_shared_matrix",
        "SUNRISE_PENDING_CHILDREN_MATRIX",
        &["native"],
    ) {
        return;
    }
    matrix(
        vec![mod_api::ModEntry::new(
            "pending_children_fixture",
            Box::new(Native),
        )]
        .into(),
    );
}
