//! Observable host query semantics through real holder writes and transaction journals.
#[cfg(test)]
use super::*;
use mod_api::{AttachmentDescriptor, AttachmentKey, AttachmentQueries, AttachmentQueriesMut};
use std::sync::{Arc, Mutex};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::Dim;
use sunrise_level::chunk::Chunk;
use sunrise_mod_runtime::attachments::Holder;

struct QueryMod(Arc<Mutex<Option<AttachmentKey<u32>>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for QueryMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let key = r
            .register_attachment_type(AttachmentDescriptor::new("query_test:value", || 0u32))
            .unwrap();
        *self.0.lock().unwrap() = Some(key);
    }
}

fn with_query_game(
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &AttachmentKey<u32>),
) {
    let key_slot = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "query_test",
        Box::new(QueryMod(key_slot.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let key = key_slot
                .lock()
                .unwrap()
                .clone()
                .expect("QueryMod registered");
            test(game, &key);
        });
    });
}

fn values(host: &dyn mod_api::Host, key: &AttachmentKey<u32>) -> Vec<u32> {
    let scope = host.attachment_read_scope().unwrap();
    let query = host.query(key).unwrap();
    let mut rows = query.iter();
    std::iter::from_fn(|| rows.try_next().unwrap())
        .map(|row| row.with_value(scope, |value| *value).unwrap())
        .collect()
}

#[test]
#[ignore = "daily"]
fn inspecting_a_loaded_chunk_does_not_create_or_dirty_attachment_storage() {
    with_query_game(|game, key| {
        let pos = [2, 3];
        let holder = mod_api::Holder::Chunk(pos);
        let coord = (pos[0], pos[1]);
        let column = Arc::new(Chunk::with_layout(
            pos[0],
            pos[1],
            game.world.shape.column(),
        ));
        game.world.chunks.insert(coord, column.clone());
        game.world.modified.clear();
        game.world.attachment_syncs.clear();
        assert_eq!(key.has(game, holder), Ok(false));
        assert_eq!(
            key.get_existing(game, holder, |value| value.copied()),
            Ok(None)
        );
        let kept = game.world.chunks.get(&coord).unwrap();
        assert!(Arc::ptr_eq(kept, &column));
        assert!(kept.saved.is_none());
        assert!(!game.world.modified.contains(&coord));
        assert!(game.world.attachment_syncs.is_empty());
        key.set(game, holder, 37).unwrap();
        assert!(game.world.modified.contains(&coord));
        assert_eq!(
            key.get_existing(game, holder, |value| value.copied()),
            Ok(Some(37))
        );
        assert!(
            game.world.attachment_syncs.is_empty(),
            "the unsynced key has no packet"
        );
    });
}

#[test]
#[ignore = "daily"]
fn query_snapshot_dimension_and_stale_writes() {
    with_query_game(|game, key| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        assert!(values(game, key).is_empty());
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        key.set(game, Holder::Chunk([2, 3]), 10).unwrap();
        let batch = game.query_mut(key).unwrap();
        let row = batch.iter().try_next().unwrap().unwrap();
        key.set(game, Holder::Chunk([2, 3]), 11).unwrap();
        assert_eq!(
            row.with_value(Default::default(), |value| *value).unwrap(),
            10
        );
        assert_eq!(values(game, key), [11]);
        game.enter_level(Dim::Nether).unwrap();
        assert!(values(game, key).is_empty());
        assert_eq!(
            row.set(game, 12),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        key.set(game, Holder::Chunk([2, 3]), 20).unwrap();
        assert_eq!(values(game, key), [20]);
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(values(game, key), [11]);
        row.set(game, 12).unwrap();
        assert_eq!(values(game, key), [12]);
        let retained = game.world.chunks.remove(&(2, 3)).unwrap();
        assert!(values(game, key).is_empty());
        assert_eq!(
            row.with_value(game.attachment_frame_borrow(), |value| *value)
                .unwrap(),
            10
        );
        assert_eq!(
            row.remove(game),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        game.world.chunks.insert((2, 3), retained);
        assert_eq!(
            row.set(game, 13),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        let fresh = game.query_mut(key).unwrap();
        let fresh_row = fresh.iter().try_next().unwrap().unwrap();
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        assert_eq!(
            fresh_row.set(game, 14),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        assert!(values(game, key).is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn query_writes_join_transaction_journal_without_implicit_commit() {
    with_query_game(|game, key| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        key.set(game, Holder::Chunk([2, 3]), 10).unwrap();
        let batch = game.query_mut(key).unwrap();
        let row = batch.iter().try_next().unwrap().unwrap();
        let aborted: Result<(), ()> = game.storage_transaction(|host, _| {
            row.set(host, 11).unwrap();
            assert_eq!(values(host, key), [11]);
            let current = host.query_mut(key).unwrap();
            current
                .iter()
                .try_next()
                .unwrap()
                .unwrap()
                .remove(host)
                .unwrap();
            assert!(values(host, key).is_empty());
            Err(())
        });
        assert_eq!(aborted, Err(()));
        assert_eq!(values(game, key), [10]);
        assert_eq!(
            row.with_value(game.attachment_frame_borrow(), |value| *value)
                .unwrap(),
            10
        );
        let committed: Result<(), ()> = game.storage_transaction(|host, _| {
            let current = host.query_mut(key).unwrap();
            current
                .iter()
                .try_next()
                .unwrap()
                .unwrap()
                .set(host, 15)
                .unwrap();
            Ok(())
        });
        assert_eq!(committed, Ok(()));
        drop(batch);
        assert_eq!(values(game, key), [15]);
    });
}

#[test]
#[ignore = "daily"]
fn rolled_back_attachment_queries_cannot_read_or_write_a_resident_holder() {
    use mod_api::AttachmentRowError::OutsideTransaction;

    with_query_game(|game, key| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        let holder = Holder::Chunk([2, 3]);
        key.set(game, holder, 10).unwrap();
        let before = game.query_mut(key).unwrap();
        let before_row = before.iter().try_next().unwrap().unwrap();
        let scoped_holder = before_row.holder();

        let reader = std::cell::OnceCell::new();
        let writer = std::cell::OnceCell::new();
        let (mut retained_iter, read_row, write_row) = game.storage_scope(|game, _| {
            key.set(game, holder, 11).unwrap();
            let reader = reader.get_or_init(|| game.query(key).unwrap());
            let writer = writer.get_or_init(|| game.query_mut(key).unwrap());
            let mut retained_iter = reader.iter();
            let read_row = retained_iter.try_next().unwrap().unwrap();
            let write_row = writer.iter().try_next().unwrap().unwrap();
            assert_eq!(
                read_row.with_value(Default::default(), |value| {
                    panic!("backend-less scope read ran: {value}")
                }),
                Err(OutsideTransaction)
            );
            let mut foreign = sunrise_mod_runtime::attachments::AttachmentQueryScopes::default();
            foreign.enter();
            assert_eq!(
                write_row.with_value(
                    mod_api::__private::game::attachment_read_scope(&foreign),
                    |value| { panic!("foreign scope read ran: {value}") },
                ),
                Err(OutsideTransaction)
            );
            let host: &dyn mod_api::ServerHost = game;
            let scope = host.attachment_read_scope().unwrap();
            assert_eq!(read_row.with_value(scope, |value| *value), Ok(11));
            assert_eq!(write_row.with_value(scope, |value| *value), Ok(11));
            assert_eq!(
                reader.get(scope, scoped_holder, |value| value.cloned()),
                Ok(Some(11))
            );
            assert_eq!(
                reader.get(
                    mod_api::__private::game::attachment_read_scope(&foreign),
                    scoped_holder,
                    |_| panic!("foreign read scope ran a keyed projection"),
                ),
                Err(OutsideTransaction)
            );
            game.storage_scope(|game, _| {
                assert_eq!(
                    read_row.with_value(game.attachment_frame_borrow(), |value| *value),
                    Err(OutsideTransaction)
                );
                ((), false)
            });
            assert_eq!(
                read_row.with_value(game.attachment_frame_borrow(), |value| *value),
                Ok(11)
            );
            ((retained_iter, read_row, write_row), false)
        });
        let reader = reader.get().unwrap();
        let writer = writer.get().unwrap();

        assert_eq!(values(game, key), [10]);
        assert_eq!(
            reader.get(game.attachment_frame_borrow(), scoped_holder, |_| {
                panic!("closed scope ran a keyed projection")
            }),
            Err(OutsideTransaction)
        );
        assert!(matches!(retained_iter.try_next(), Err(OutsideTransaction)));
        assert!(matches!(writer.iter().try_next(), Err(OutsideTransaction)));
        assert_eq!(
            read_row.with_value(game.attachment_frame_borrow(), |value| *value),
            Err(OutsideTransaction)
        );
        assert_eq!(
            write_row.with_value(game.attachment_frame_borrow(), |value| *value),
            Err(OutsideTransaction)
        );
        assert_eq!(write_row.set(game, 99), Err(OutsideTransaction));
        assert_eq!(write_row.remove(game), Err(OutsideTransaction));
        assert_eq!(values(game, key), [10]);
        before_row.set(game, 12).unwrap();
        assert_eq!(values(game, key), [12]);
    });
}
