//! `LevelRef::entity_by_uuid` (`ServerLevel.getEntity(UUID)`): a hook in one level finds an
//! entity of another, parked, level through that level's UUID index, and the handle it gets
//! reads and removes the entity once its level is in place.
use super::tests::{run_fresh_cases, with_game};
use mod_api::{EntityRef, EntityUuid, Hooks, HostSide, LevelRef, LevelTickPreEvent, ModEntry};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::Dim;

const FIRST: EntityUuid = EntityUuid((7 << 96) | 1);
const SECOND: EntityUuid = EntityUuid((7 << 96) | 2);

/// What one hook call saw.
#[derive(Debug, PartialEq)]
enum Seen {
    /// In the Overworld's tick: the Nether's lookups of both zombies, the Overworld's own
    /// lookup of the first, the first one's level, then its health and its discard while the
    /// Nether is parked.
    Overworld {
        first: Option<EntityRef>,
        second: Option<EntityRef>,
        first_at_home: Option<EntityRef>,
        first_level: Option<LevelRef>,
        parked_health: Option<f32>,
        parked_discard: bool,
    },
    /// In the Nether's tick, with the handle the Overworld found: its health, its discard,
    /// then the Nether's lookup again.
    Nether {
        health: Option<f32>,
        discarded: bool,
        after: Option<EntityRef>,
    },
}

#[derive(Default)]
struct Log {
    found: Option<EntityRef>,
    seen: Vec<Seen>,
}

struct Lookup(Arc<Mutex<Log>>);

#[mod_api::__subscriptions]
impl Hooks for Lookup {
    fn level_tick_pre(&self, host: HostSide, event: &mut LevelTickPreEvent) {
        let HostSide::Server(host) = host else {
            return;
        };
        let nether = host.level("minecraft:the_nether").unwrap();
        let mut log = self.0.lock().unwrap();
        if event.level == nether {
            let first = log.found;
            let health = first
                .and_then(|entity| host.living(entity))
                .and_then(|living| living.health(host));
            let discarded = first.is_some_and(|entity| entity.discard(host));
            let after = nether.entity_by_uuid(host, FIRST);
            log.seen.push(Seen::Nether {
                health,
                discarded,
                after,
            });
            return;
        }
        let first = nether.entity_by_uuid(host, FIRST);
        let second = nether.entity_by_uuid(host, SECOND);
        let first_at_home = event.level.entity_by_uuid(host, FIRST);
        let first_level = first.and_then(|entity| entity.level(host));
        let parked_health = first
            .and_then(|entity| host.living(entity))
            .and_then(|living| living.health(host));
        let parked_discard = first.is_some_and(|entity| entity.discard(host));
        log.found = log.found.or(first);
        log.seen.push(Seen::Overworld {
            first,
            second,
            first_at_home,
            first_level,
            parked_health,
            parked_discard,
        });
    }
}

/// From an Overworld hook, the Nether's handle finds a zombie of the parked Nether by UUID;
/// the zombie's health and removal answer through that handle once the Nether is in place,
/// after which the lookup answers `None`, as it does for a zombie whose chunk unloaded.
#[test]
#[ignore = "daily"]
fn a_parked_levels_uuid_lookup_finds_its_entity_until_removed_or_unloaded() {
    if run_fresh_cases(
        "cut_layout::game::server_side::native_entity_refs::level_uuid_tests::a_parked_levels_uuid_lookup_finds_its_entity_until_removed_or_unloaded",
        "SUNRISE_TEST_LEVEL_UUID_CASE",
        &["lookup"],
    ) {
        return;
    }
    let log = Arc::new(Mutex::new(Log::default()));
    let mods = vec![ModEntry::new("level_uuid", Box::new(Lookup(log.clone())))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.build_registries();
            assert_eq!(game.dim, Dim::Overworld);
            for (uuid, x) in [(1, 8.5), (2, 9.5)] {
                assert!(game.as_console(|game| game.run_command(&format!(
                    "execute in minecraft:the_nether run summon zombie {x} 70 8.5 {{UUID:[I;7,0,0,{uuid}],NoAI:1b}}"
                ))));
            }
            assert_eq!(game.dim, Dim::Overworld);
            let home = game.level("minecraft:overworld").unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            // The zombies' ids, read from the Nether's own memory while it is in place.
            game.enter_level(Dim::Nether).unwrap();
            let id = |game: &sunrise_gameplay::game::ServerGame<'_>, last: i32| {
                let (&id, _) = game
                    .persist
                    .uuids
                    .iter()
                    .find(|(_, words)| **words == [7, 0, 0, last])
                    .unwrap();
                EntityRef::new(nether.epoch(), id)
            };
            let (first, second) = (id(game, 1), id(game, 2));
            game.enter_level(Dim::Overworld).unwrap();

            game.native_level_tick_pre(home);
            game.enter_level(Dim::Nether).unwrap();
            game.native_level_tick_pre(nether);
            game.enter_level(Dim::Overworld).unwrap();
            game.native_level_tick_pre(home);
            game.enter_level(Dim::Nether).unwrap();
            game.entity_chunks(&[(0, 0)].into_iter().collect(), true);
            game.enter_level(Dim::Overworld).unwrap();
            game.native_level_tick_pre(home);

            assert_eq!(
                log.lock().unwrap().seen,
                [
                    Seen::Overworld {
                        first: Some(first),
                        second: Some(second),
                        first_at_home: None,
                        first_level: Some(nether),
                        parked_health: None,
                        parked_discard: false,
                    },
                    Seen::Nether {
                        health: Some(20.0),
                        discarded: true,
                        after: None,
                    },
                    Seen::Overworld {
                        first: None,
                        second: Some(second),
                        first_at_home: None,
                        first_level: None,
                        parked_health: None,
                        parked_discard: false,
                    },
                    Seen::Overworld {
                        first: None,
                        second: None,
                        first_at_home: None,
                        first_level: None,
                        parked_health: None,
                        parked_discard: false,
                    },
                ]
            );
        });
    });
}

use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
