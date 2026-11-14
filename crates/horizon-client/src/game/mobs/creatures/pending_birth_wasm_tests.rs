//! The pending-children matrix crosses native and real wasm dispatch: the same listener source
//! (`horizon-mod-wasm/tests/hook-fixture/src/pending_children.rs`) runs as a native mod and inside the
//! wasm fixture, and the same rows check what spawns. The listener's mode is the last digit of the
//! parent id; what a guest learns from a refused call comes back as decimal digits in the first
//! child's yaw (see the fixture's header).

use super::pending_birth_tests::{SEED, YAWS, breed_pair, cube, find, near};
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{BabyEntitySpawnEvent, Flow, Hooks, MobSplitEvent, ServerHost};

#[path = "../../../../../horizon-mod-wasm/tests/hook-fixture/src/pending_children.rs"]
mod fixture;

struct Native;
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

/// A size-4 slime whose entity id ends in `mode`, split by the real route.
fn split(game: &mut Game<'_>, mode: u32) -> Vec<Mob> {
    game.mobs.clear();
    let mut dying = cube(game, "minecraft:slime", 4, SEED);
    dying.id = 9000 + mode;
    game.split_cube_children(&mut dying);
    game.mobs.to_vec()
}

/// Two cows bred by the real route; the parent's id ends in `mode`.
fn breed(game: &mut Game<'_>, mode: u32) -> (Vec<Mob>, Option<u32>) {
    game.mobs.clear();
    game.next_entity = 8999 + mode;
    let (_, _, child) = breed_pair(game, "minecraft:cow");
    (game.mobs.to_vec(), child)
}

fn matrix(mods: mod_api::Mods) {
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
            assert!(near(kids[1].position.x, 10.0 - 0.52));
            assert_eq!(
                (kids[2].position.x, kids[2].yaw, kids[2].extra.size),
                (3.0, 12.0, 2)
            );
            // Replace C with E; a second append of E reports Ownership (2) in A's yaw.
            let kids = split(game, 3);
            assert_eq!(kids.len(), 3);
            assert_eq!(kids[0].yaw, 2.0);
            assert_eq!(kids[2].kind.id, "minecraft:magma_cube");
            assert_eq!((kids[2].position.x, kids[2].yaw), (8.0, 33.0));
            // Invalid input: NaN, unknown kind, wrong dimension (4, 4, 4), duplicate token (2),
            // the 65th child (3) and a NaN move (4): digits 4,4,4,2,3,4 read from the right.
            let kids = split(game, 4);
            assert_eq!(kids.len(), 64);
            assert_eq!(kids[0].yaw, 432_444.0);
            // A removed child's borrow is stale (1).
            let kids = split(game, 5);
            assert_eq!(kids.len(), 2);
            assert_eq!(kids[0].yaw, 1.0);

            // Breeding: untouched baby.
            let (_, child) = breed(game, 0);
            assert_eq!(find(game, child.unwrap()).extra.age, -24000);
            // Veto, and removing the baby: no child, parents resting.
            for mode in [1, 3] {
                let (mobs, child) = breed(game, mode);
                assert_eq!(child, None, "mode {mode}");
                assert!(mobs.iter().all(|m| m.extra.age >= 0));
                assert_eq!(find(game, 9000 + mode).extra.age, 6000);
                assert_eq!(find(game, 9001 + mode).extra.in_love, 0);
            }
            // Replace with a pig baby; a second baby reports Capacity (3).
            let (_, child) = breed(game, 2);
            let pig = find(game, child.unwrap());
            assert_eq!(pig.kind.id, "minecraft:pig");
            assert!(pig.extra.age < 0);
            assert_eq!(pig.yaw, 3.0);
            // A grown replacement is Invalid (4); the cow baby stays.
            let (_, child) = breed(game, 4);
            let baby = find(game, child.unwrap());
            assert_eq!(baby.kind.id, "minecraft:cow");
            assert_eq!(baby.yaw, 4.0);
        });
    });
}

/// A guest that edits and then traps: the failed call commits nothing, whatever it changed
/// first. A trapped guest is stopped, so each trap runs against a freshly loaded module.
fn trap(mods: mod_api::Mods, baby: bool) {
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            if baby {
                let (mobs, child) = breed(game, 6);
                assert_eq!(child, None, "a trapped baby listener's edit never commits");
                assert!(
                    mobs.iter().all(|m| m.extra.age >= 0),
                    "no baby after a trap"
                );
            } else {
                assert!(split(game, 6).is_empty(), "a trapped split spawns nothing");
            }
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn native_pending_children_shared_matrix() {
    matrix(vec![mod_api::ModEntry::new("wasm_fixture", Box::new(Native))].into());
}

/// The fixture module, loaded and initialized as the game does.
fn load_wasm() -> mod_api::Mods {
    use horizon_mod_package::manifest::Package;
    use horizon_mod_wasm::{WasmPackage, load};
    let path = std::path::PathBuf::from(std::env::var_os("HORIZON_WASM_FIXTURE").unwrap());
    let manifest = Package::read(&path).unwrap().manifest;
    let info = mod_api::PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
        vec![manifest.id.clone()],
        vec![],
    );
    let loaded = load(
        &std::fs::read(std::env::var_os("HORIZON_WASM_STD").unwrap()).unwrap(),
        vec![WasmPackage {
            manifest,
            module: Package::read_wasm(&path).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    assert_eq!(loaded.mods.len(), 1);
    loaded.initialize(&[info]).unwrap();
    loaded.mods.into()
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES, HORIZON_WASM_FIXTURE and HORIZON_WASM_STD; build with horizon-mod-wasm/listener-fixture"]
fn wasm_pending_children_shared_matrix() {
    // Enters the guest: libtest's thread is smaller than the guest stack policy requires.
    horizon_guest_stack::run_on_guest_stack(|| matrix(load_wasm()));
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES, HORIZON_WASM_FIXTURE and HORIZON_WASM_STD; build with horizon-mod-wasm/listener-fixture"]
fn wasm_trapped_split_listener_commits_nothing() {
    horizon_guest_stack::run_on_guest_stack(|| trap(load_wasm(), false));
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES, HORIZON_WASM_FIXTURE and HORIZON_WASM_STD; build with horizon-mod-wasm/listener-fixture"]
fn wasm_trapped_baby_listener_commits_nothing() {
    horizon_guest_stack::run_on_guest_stack(|| trap(load_wasm(), true));
}
