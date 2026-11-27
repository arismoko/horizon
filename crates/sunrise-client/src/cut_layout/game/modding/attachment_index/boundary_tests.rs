use mod_api::AttachmentHolder;
use mod_api::{AttachmentDescriptor, AttachmentKey, AttachmentRowError, AttachmentWriteToken};
use std::sync::Arc;
use std::sync::Mutex;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::Dim;
use sunrise_mod_runtime::attachments::Holder;
use sunrise_mod_runtime::attachments::{AttachmentType, index::AttachmentIndex};

static VALUE: AttachmentType<u32> = AttachmentType {
    namespace: "boundary",
    name: "value",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync_codec: None,
};

struct BoundaryMod(Arc<Mutex<Option<AttachmentKey<u32>>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for BoundaryMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let key = r
            .register_attachment_type(AttachmentDescriptor::new("boundary:value", || 0u32))
            .unwrap();
        *self.0.lock().unwrap() = Some(key);
    }
}

fn capture(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    key: &AttachmentKey<u32>,
) -> Vec<(AttachmentHolder, u32, AttachmentWriteToken)> {
    let batch = mod_api::AttachmentQueries::capture(game, key).unwrap();
    let query = batch.query(key).unwrap();
    let mut rows = query.iter();
    std::iter::from_fn(|| rows.try_next().unwrap())
        .map(|row| {
            (
                row.holder,
                row.with_value(game.attachment_frame_borrow(), |value| *value)
                    .unwrap(),
                row.write_token(),
            )
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn current_dimension_capture_and_foreign_residency_validation() {
    let key_slot = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "boundary",
        Box::new(BoundaryMod(key_slot.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let key = key_slot
                .lock()
                .unwrap()
                .clone()
                .expect("BoundaryMod registered");
            VALUE
                .set(&mut game.world.attachments, Holder::Level, 7)
                .unwrap();
            game.native_level_opened(Dim::Overworld);
            let rows = capture(game, &key);
            let holder = rows[0].0;
            let token = rows[0].2.clone();
            assert_eq!(game.attachment_validate_write(token.clone()), Ok(()));
            assert_eq!(rows[0].1, 7);
            let epoch = game.native_lifecycle.epoch().unwrap();
            let b = AttachmentHolder::Entity(mod_api::EntityRef::new(epoch, 313));
            let mut local_map = sunrise_mod_runtime::attachments::Attachments::default();
            VALUE.set(&mut local_map, Holder::Entity(313), 8).unwrap();
            local_map.bind_index(&game.attachment_indexes[&game.dim], b);
            let token_b = capture(game, &key)
                .into_iter()
                .find_map(|(holder, _, token)| (holder == b).then_some(token))
                .unwrap();
            assert_eq!(game.attachment_validate_write(token_b), Ok(()));
            drop(local_map);

            let foreign = Arc::new(Mutex::new(AttachmentIndex::default()));
            let mut foreign_map = sunrise_mod_runtime::attachments::Attachments::default();
            VALUE.set(&mut foreign_map, Holder::Level, 99).unwrap();
            foreign_map.bind_index(&foreign, holder);
            let original = game
                .attachment_indexes
                .insert(Dim::Overworld, foreign)
                .unwrap();
            assert_eq!(
                game.attachment_validate_write(token.clone()),
                Err(AttachmentRowError::StaleHolder)
            );
            assert_eq!(capture(game, &key)[0].1, 99);
            game.attachment_indexes.insert(Dim::Overworld, original);
            assert_eq!(game.attachment_validate_write(token.clone()), Ok(()));

            game.dim = Dim::Nether;
            assert!(capture(game, &key).is_empty());
            assert_eq!(
                game.attachment_validate_write(token),
                Err(AttachmentRowError::StaleHolder)
            );
            game.dim = Dim::Overworld;
        });
    });
}
