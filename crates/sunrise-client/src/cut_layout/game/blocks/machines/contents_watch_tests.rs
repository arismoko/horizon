//! Behavioral coverage of source-keyed contents watches through real Parts and handles.
use mod_api::*;
use std::sync::{Arc, Mutex};
use sunrise_core::position::BlockPos;
use sunrise_level::chunk::Chunk;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
};

const SOURCE: [i32; 3] = [0, 64, 0];
const SECOND: [i32; 3] = [2, 64, 0];
const OBSERVER: [i32; 3] = [18, 64, 0];
const OTHER: [i32; 3] = [20, 64, 0];

#[derive(Clone, Debug, PartialEq)]
struct Notice {
    owner: &'static str,
    observer: Option<BlockEntityRef>,
    pos: [i32; 3],
    state: BlockState,
    amount: u32,
}

#[derive(Clone, Copy, Default)]
enum Action {
    #[default]
    None,
    Reenter(BlockEntityRef),
    Register(BlockEntityRef),
    Reregister,
}

#[derive(Clone)]
struct Watcher {
    owner: &'static str,
    notices: Arc<Mutex<Vec<Notice>>>,
    action: Arc<Mutex<Action>>,
}

#[mod_api::sunrise_part]
impl BlockPart for Watcher {
    fn contents_changed(&self, host: &mut dyn ServerHost, event: &mut ContentsChangedEvent) {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let amount = event
            .level
            .capability(host, &ITEMS, pos, None)
            .map_or(0, |items| items.amount(host, 0));
        self.notices.lock().unwrap().push(Notice {
            owner: self.owner,
            observer: event.observer,
            pos,
            state: event.state,
            amount,
        });
        // Do not hold the action lock across reentrant dispatch.
        let action = *self.action.lock().unwrap();
        if self.owner == "watch_a"
            && matches!(action, Action::Reregister)
            && event.observer.is_none()
            && pos == OBSERVER
        {
            host.block_entity(event.level, pos)
                .unwrap()
                .watch_contents(host, source_pos(), true)
                .unwrap();
        }
        if self.owner != "watch_a" || event.observer.is_none() || pos != SOURCE {
            return;
        }
        match action {
            Action::None | Action::Reregister => {}
            Action::Reenter(other) => {
                host.block_entity(event.level, SOURCE)
                    .unwrap()
                    .set_changed(host)
                    .unwrap();
                other.set_changed(host).unwrap();
            }
            Action::Register(observer) => {
                observer.watch_contents(host, event.pos, true).unwrap();
            }
        }
    }
}

#[mod_api::__subscriptions]
impl Hooks for Watcher {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let chest = r.block_key("minecraft:chest");
        r.block_part(chest, self.clone());
    }
}

/// The second mod's watcher. A part type attaches to a block once (`insert_part` refuses the same
/// type from two mods), so it is a type of its own that behaves as [`Watcher`] does.
#[derive(Clone)]
struct SecondWatcher(Watcher);

#[mod_api::sunrise_part]
impl BlockPart for SecondWatcher {
    fn contents_changed(&self, host: &mut dyn ServerHost, event: &mut ContentsChangedEvent) {
        self.0.contents_changed(host, event);
    }
}

#[mod_api::__subscriptions]
impl Hooks for SecondWatcher {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let chest = r.block_key("minecraft:chest");
        r.block_part(chest, self.clone());
    }
}

fn with_watches(
    name: &str,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, LevelRef, Watcher),
) {
    if run_fresh_cases(name, "SUNRISE_CONTENTS_WATCH_CHILD", &["game"]) {
        return;
    }
    let a = Watcher {
        owner: "watch_a",
        notices: Default::default(),
        action: Default::default(),
    };
    let b = Watcher {
        owner: "watch_b",
        ..a.clone()
    };
    let mods: Mods = vec![
        ModEntry::new("watch_a", Box::new(a.clone())),
        ModEntry::new("watch_b", Box::new(SecondWatcher(b))),
    ]
    .into();
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    with_game_at(root, |game| {
        game.options.dedicated = Some(Vec::new());
        for x in [0, 1] {
            game.world
                .chunks
                .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
        }
        let level = game.level("minecraft:overworld").unwrap();
        for (pos, block) in [
            (SOURCE, "minecraft:barrel"),
            (SECOND, "minecraft:barrel"),
            (OBSERVER, "minecraft:chest"),
            (OTHER, "minecraft:chest"),
        ] {
            let state = game.pack.assets.registry.default_state(block).unwrap();
            assert_eq!(
                game.level_set_block(level, pos, state, BlockUpdateFlags::NONE),
                Ok(true)
            );
        }
        a.notices.lock().unwrap().clear();
        test(game, level, a);
    });
}

fn source_pos() -> BlockPos {
    BlockPos::new(SOURCE[0], SOURCE[1], SOURCE[2])
}
fn notify(game: &mut sunrise_gameplay::game::ServerGame<'_>, level: LevelRef, at: [i32; 3]) {
    game.block_entity(level, at)
        .unwrap()
        .set_changed(game)
        .unwrap();
}
fn take(watcher: &Watcher) -> Vec<Notice> {
    std::mem::take(&mut *watcher.notices.lock().unwrap())
}

#[test]
#[ignore = "daily"]
fn keyed_observers_route_source_data_without_global_or_self_duplicates() {
    with_watches(
        "cut_layout::game::server_side::machines::contents_watch_tests::keyed_observers_route_source_data_without_global_or_self_duplicates",
        |game, level, watcher| {
            let observer = game.block_entity(level, OBSERVER).unwrap();
            let barrel = game.block_entity(level, SOURCE).unwrap();
            assert_eq!(
                barrel.watch_contents(game, source_pos(), true),
                Err(ContentsWatchError::NotSubscribed)
            );
            observer.watch_contents(game, source_pos(), true).unwrap();
            observer.watch_contents(game, source_pos(), true).unwrap();
            notify(game, level, SECOND);
            assert!(take(&watcher).is_empty());
            notify(game, level, SOURCE);
            let notices = take(&watcher);
            assert_eq!(
                notices
                    .iter()
                    .map(|n| (n.owner, n.observer, n.pos))
                    .collect::<Vec<_>>(),
                [
                    ("watch_a", Some(observer), SOURCE),
                    ("watch_b", Some(observer), SOURCE)
                ]
            );
            assert!(
                notices
                    .iter()
                    .all(|n| n.state == level.block_state(game, SOURCE).unwrap())
            );
            observer.watch_contents(game, source_pos(), false).unwrap();
            notify(game, level, SOURCE);
            assert!(take(&watcher).is_empty());
            observer
                .watch_contents(game, BlockPos::new(18, 64, 0), true)
                .unwrap();
            notify(game, level, OBSERVER);
            assert_eq!(
                take(&watcher)
                    .iter()
                    .map(|n| n.observer)
                    .collect::<Vec<_>>(),
                [None, None]
            );
            // A second watched chest demonstrates source-first, then stable observer order.
            let other = game.block_entity(level, OTHER).unwrap();
            observer
                .watch_contents(game, BlockPos::new(20, 64, 0), true)
                .unwrap();
            notify(game, level, OTHER);
            assert_eq!(
                take(&watcher)
                    .iter()
                    .map(|n| n.observer)
                    .collect::<Vec<_>>(),
                [None, None, Some(observer), Some(observer)]
            );
            assert_eq!(other.level(game), Some(level));
        },
    );
}

#[test]
#[ignore = "daily"]
fn watches_journal_nested_changes_and_preserve_explicit_abort_notices() {
    with_watches(
        "cut_layout::game::server_side::machines::contents_watch_tests::watches_journal_nested_changes_and_preserve_explicit_abort_notices",
        |game, level, watcher| {
            let observer = game.block_entity(level, OBSERVER).unwrap();
            let source = game.block_entity(level, SOURCE).unwrap();
            assert!(
                game.storage_transaction(|host, _| {
                    observer.watch_contents(host, source_pos(), true).unwrap();
                    source.set_changed(host).unwrap();
                    Err::<(), ()>(())
                })
                .is_err()
            );
            assert_eq!(take(&watcher).len(), 2); // Already issued, not reverted by abort.
            notify(game, level, SOURCE);
            assert!(take(&watcher).is_empty());
            observer.watch_contents(game, source_pos(), true).unwrap();
            assert!(
                game.storage_transaction(|host, _| {
                    host.transaction(|host, _| {
                        observer.watch_contents(host, source_pos(), false).unwrap();
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                    Err::<(), ()>(())
                })
                .is_err()
            );
            notify(game, level, SOURCE);
            assert_eq!(take(&watcher).len(), 2);
            assert_eq!(
                game.storage_transaction(|host, _| {
                    assert!(
                        host.transaction(|host, _| {
                            observer.watch_contents(host, source_pos(), false).unwrap();
                            Err::<(), ()>(())
                        })
                        .is_err()
                    );
                    Ok::<_, ()>(())
                }),
                Ok(())
            );
            notify(game, level, SOURCE);
            assert_eq!(take(&watcher).len(), 2);
            let items = level.capability(game, &ITEMS, SOURCE, None).unwrap();
            let sticks = ItemResource::of(
                &mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    1,
                )
                .unwrap(),
            );
            assert!(
                game.storage_transaction(|host, tx| {
                    assert_eq!(items.insert_at(host, tx, 0, &sticks, 2), Ok(2));
                    Err::<(), ()>(())
                })
                .is_err()
            );
            assert!(take(&watcher).is_empty());
            game.storage_transaction(|host, tx| {
                assert_eq!(items.insert_at(host, tx, 0, &sticks, 2), Ok(2));
                assert_eq!(items.insert_at(host, tx, 0, &sticks, 3), Ok(3));
                Ok::<_, ()>(())
            })
            .unwrap();
            let notices = take(&watcher);
            assert_eq!(notices.len(), 2);
            assert!(notices.iter().all(|notice| notice.amount == 5));
        },
    );
}

#[test]
#[ignore = "daily"]
fn watch_dispatch_snapshots_membership_and_guards_the_source_not_the_observer() {
    with_watches(
        "cut_layout::game::server_side::machines::contents_watch_tests::watch_dispatch_snapshots_membership_and_guards_the_source_not_the_observer",
        |game, level, watcher| {
            let observer = game.block_entity(level, OBSERVER).unwrap();
            let other = game.block_entity(level, OTHER).unwrap();
            observer.watch_contents(game, source_pos(), true).unwrap();
            *watcher.action.lock().unwrap() = Action::Register(other);
            notify(game, level, SOURCE);
            assert_eq!(
                take(&watcher)
                    .iter()
                    .map(|n| n.observer)
                    .collect::<Vec<_>>(),
                [Some(observer), Some(observer)]
            );
            notify(game, level, SOURCE);
            assert_eq!(
                take(&watcher)
                    .iter()
                    .map(|n| n.observer)
                    .collect::<Vec<_>>(),
                [Some(observer), Some(observer), Some(other), Some(other)]
            );
            other.watch_contents(game, source_pos(), false).unwrap();
            let second = game.block_entity(level, SECOND).unwrap();
            observer
                .watch_contents(game, BlockPos::new(2, 64, 0), true)
                .unwrap();
            *watcher.action.lock().unwrap() = Action::Reenter(second);
            notify(game, level, SOURCE);
            assert_eq!(
                take(&watcher)
                    .iter()
                    .map(|n| (n.owner, n.pos))
                    .collect::<Vec<_>>(),
                [
                    ("watch_a", SOURCE),
                    ("watch_a", SECOND),
                    ("watch_b", SECOND),
                    ("watch_b", SOURCE)
                ]
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn observer_unload_reload_requires_explicit_reregistration_and_replacement_retires_it() {
    with_watches(
        "cut_layout::game::server_side::machines::contents_watch_tests::observer_unload_reload_requires_explicit_reregistration_and_replacement_retires_it",
        |game, level, watcher| {
            let observer = game.block_entity(level, OBSERVER).unwrap();
            observer.watch_contents(game, source_pos(), true).unwrap();
            // Removing an observer tentatively also journals its watch membership.
            game.storage_scope(|game, _| {
                game.journal_remove_block_entity((18, 64, 0));
                notify(game, level, SOURCE);
                assert!(take(&watcher).is_empty());
                ((), false)
            });
            notify(game, level, SOURCE);
            assert_eq!(take(&watcher).len(), 2);
            // Source unload does not retire the observer's positional watch.
            let source_key = (0, 64, 0);
            let source_saved = game.world.block_entities.get(&source_key).unwrap().clone();
            game.world.block_entities.unload(|pos| *pos == source_key);
            game.world
                .block_entities
                .load_chunk(vec![(source_key, source_saved)]);
            notify(game, level, SOURCE);
            assert_eq!(take(&watcher).len(), 2);
            let key = (OBSERVER[0], OBSERVER[1], OBSERVER[2]);
            let saved = game.world.block_entities.get(&key).unwrap().clone();
            game.world.block_entities.unload(|pos| pos.0 >> 4 == 1);
            assert!(
                game.world
                    .block_entities
                    .contents_watches
                    .observers((0, 64, 0))
                    .is_empty()
            );
            notify(game, level, SOURCE);
            assert!(take(&watcher).is_empty());
            assert_eq!(
                observer.watch_contents(game, source_pos(), true),
                Err(ContentsWatchError::Stale)
            );
            game.world
                .block_entities
                .load_chunk(vec![(key, saved.clone())]);
            let reloaded = game.block_entity(level, OBSERVER).unwrap();
            assert_ne!(reloaded, observer);
            notify(game, level, SOURCE);
            assert!(take(&watcher).is_empty()); // Merely loading does not restore subscriptions.
            *watcher.action.lock().unwrap() = Action::Reregister;
            notify(game, level, OBSERVER); // The Part explicitly registers the new identity.
            take(&watcher);
            *watcher.action.lock().unwrap() = Action::None;
            notify(game, level, SOURCE);
            assert_eq!(take(&watcher).len(), 2);
            // A temporary tick take/put preserves watches, unlike unload.
            let (lease, entity) = game.world.block_entities.take(&key).unwrap();
            reloaded.watch_contents(game, source_pos(), true).unwrap();
            notify(game, level, SOURCE);
            assert_eq!(take(&watcher).len(), 2);
            assert!(game.world.block_entities.put(lease, entity));
            game.world.block_entities.replace(key, saved);
            notify(game, level, SOURCE);
            assert!(take(&watcher).is_empty());
            assert_eq!(
                reloaded.watch_contents(game, source_pos(), true),
                Err(ContentsWatchError::Stale)
            );
        },
    );
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
