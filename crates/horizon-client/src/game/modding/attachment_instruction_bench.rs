// game-module-cfg: test
use crate::game::native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game};
use horizon_client::mobs::Mob;
use horizon_core::{geometry::Vec3, position::ChunkPos};
static VALUE: mod_api::AttachmentType<u32> = mod_api::AttachmentType {
    namespace: "bench",
    name: "tick_state",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};
#[test]
#[ignore = "callgrind fixture; needs HORIZON_TEST_RESOURCES"]
fn attachment_instruction_count() {
    let Some(passes) = std::env::var("ATTACHMENT_PASSES")
        .ok()
        .and_then(|passes| passes.parse::<usize>().ok())
    else {
        eprintln!("refused, needs ATTACHMENT_PASSES; run by the parity harness");
        return;
    };
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        game.bind_dimension_attachments(game.dim);
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let mut holders = Vec::new();
        for id in 100..400 {
            game.mobs.push(Mob::new(id, kind.clone(), Vec3::ZERO, 0.0));
            holders.push(mod_api::Holder::Entity(id));
        }
        for z in (0..40).step_by(2) {
            for x in 0..20 {
                game.world
                    .block_entities
                    .insert((x, 201, z), crate::game::BlockEntity::Chest(vec![None; 5]));
                holders.push(mod_api::Holder::BlockEntity([x, 201, z]));
            }
        }
        for &holder in &holders {
            VALUE
                .set(game, holder, 0)
                .unwrap_or_else(|e| panic!("warm holder {holder:?}: {e:?}"));
        }
        assert_eq!(
            game.attachment_indexes[&game.dim]
                .lock()
                .unwrap()
                .snapshot_named("bench", "tick_state")
                .len(),
            700
        );
        if std::env::var("ATTACHMENT_HANDLE").is_ok() {
            let mut handles = ResolvedHandles::resolve(game, &holders);
            let valid = handles.token(0);
            for forged in [
                ResolvedHandle {
                    slot: usize::MAX,
                    ..valid
                },
                ResolvedHandle { column: 1, ..valid },
                ResolvedHandle {
                    generation: 0,
                    ..valid
                },
            ] {
                assert_eq!(handles.get(game, forged), Err(Refusal::InvalidHandle));
                assert_eq!(handles.set(game, forged, 99), Err(Refusal::InvalidHandle));
            }
            handles.server = !handles.server;
            assert_eq!(handles.get(game, valid), Err(Refusal::WrongSide));
            assert_eq!(handles.set(game, valid, 99), Err(Refusal::WrongSide));
            handles.server = !handles.server;
            let replacement = ResolvedHandles::resolve(game, &holders);
            assert_eq!(replacement.get(game, valid), Err(Refusal::InvalidHandle));
            attachment_handle_measured_passes(game, &handles, passes);
            // Retirement must refuse even though the physical map remains reachable.
            handles.records[0].registration.unregister();
            assert_eq!(
                handles.get(game, valid),
                Err(Refusal::Attachment(mod_api::AttachmentError::MissingHolder))
            );
            assert_eq!(
                handles.set(game, valid, 99),
                Err(Refusal::Attachment(mod_api::AttachmentError::MissingHolder))
            );
            // Unload the dimension's authority while its physical maps remain.
            game.attachment_indexes.remove(&game.dim);
            let other = handles.token(1);
            assert_eq!(
                handles.get(game, other),
                Err(Refusal::Attachment(mod_api::AttachmentError::MissingHolder))
            );
            assert_eq!(
                handles.set(game, other, 99),
                Err(Refusal::Attachment(mod_api::AttachmentError::MissingHolder))
            );
        } else {
            attachment_measured_passes(game, &holders, passes);
        }
        for &holder in &holders {
            assert_eq!(VALUE.get(game, holder).unwrap(), passes as u32);
        }
    });
}

#[inline(never)]
fn attachment_measured_passes(
    game: &mut crate::game::Game<'_>,
    holders: &[mod_api::Holder],
    passes: usize,
) {
    for _ in 0..passes {
        for &holder in holders {
            let value = VALUE.get(game, holder).unwrap();
            VALUE
                .set(game, holder, std::hint::black_box(value.wrapping_add(1)))
                .unwrap();
        }
    }
}

// Measurement-only handle table. This is not the public native/wasm API.
// The integers are checked before any physical map is accessed.
#[derive(Clone, Copy)]
struct ResolvedHandle {
    slot: usize,
    column: usize,
    generation: u64,
}
struct ResolvedRecord {
    holder: mod_api::Holder,
    registration: horizon_mod_runtime::attachments::AttachmentRegistration,
    generation: u64,
}
struct ResolvedHandles {
    records: Vec<ResolvedRecord>,
    key: horizon_mod_runtime::attachments::index::AttachmentKey,
    server: bool,
    reuse_entry: bool,
}
#[derive(Debug, PartialEq)]
enum Refusal {
    WrongSide,
    InvalidHandle,
    Attachment(mod_api::AttachmentError),
}
impl ResolvedHandles {
    fn resolve(game: &mut crate::game::Game<'_>, holders: &[mod_api::Holder]) -> Self {
        let server = game.native_authoritative();
        let records = holders
            .iter()
            .map(|&holder| {
                let map = mod_api::AttachmentHost::attachments(
                    game,
                    holder,
                    mod_api::AttachmentAccess::Existing,
                )
                .unwrap();
                ResolvedRecord {
                    holder,
                    registration: map
                        .resolved_registration()
                        .expect("warm-up bound residency"),
                    generation: {
                        static NEXT: std::sync::atomic::AtomicU64 =
                            std::sync::atomic::AtomicU64::new(1);
                        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    },
                }
            })
            .collect();
        Self {
            records,
            key: ("bench".into(), "tick_state".into()),
            server,
            reuse_entry: std::env::var_os("ATTACHMENT_ARC_REUSE").is_some(),
        }
    }
    fn token(&self, slot: usize) -> ResolvedHandle {
        ResolvedHandle {
            slot,
            column: 0,
            generation: self.records[slot].generation,
        }
    }
    fn map<'a>(
        &self,
        game: &'a mut crate::game::Game<'_>,
        handle: ResolvedHandle,
    ) -> Result<&'a mut mod_api::Attachments, Refusal> {
        if game.native_authoritative() != self.server {
            return Err(Refusal::WrongSide);
        }
        let record = self
            .records
            .get(handle.slot)
            .filter(|record| handle.column == 0 && handle.generation == record.generation)
            .ok_or(Refusal::InvalidHandle)?;
        if !record.registration.is_current() {
            return Err(Refusal::Attachment(mod_api::AttachmentError::MissingHolder));
        }
        // Same transaction and dirty/sync bookkeeping as the old access path;
        // physical map lookup does not consult the entity residency table.
        game.journal_attachments(record.holder);
        game.world.attachment_syncs.insert(record.holder);
        let map = match record.holder {
            mod_api::Holder::Entity(id) => game.entity_attachments.get_mut(&id),
            mod_api::Holder::BlockEntity([x, y, z]) => {
                game.world.modified.insert((x >> 4, z >> 4));
                game.world.block_entities.attachments_mut((x, y, z))
            }
            _ => None,
        }
        .ok_or(Refusal::Attachment(mod_api::AttachmentError::MissingHolder))?;
        if !map
            .resolved_registration()
            .is_some_and(|current| current.same_registration(&record.registration))
        {
            return Err(Refusal::Attachment(mod_api::AttachmentError::MissingHolder));
        }
        Ok(map)
    }
    fn get(
        &self,
        game: &mut crate::game::Game<'_>,
        handle: ResolvedHandle,
    ) -> Result<u32, Refusal> {
        self.map(game, handle)?
            .resolved_read(&self.key, &VALUE)
            .map_err(Refusal::Attachment)
    }
    fn set(
        &self,
        game: &mut crate::game::Game<'_>,
        handle: ResolvedHandle,
        value: u32,
    ) -> Result<(), Refusal> {
        let map = self.map(game, handle)?;
        if self.reuse_entry {
            map.resolved_write::<true, _>(&self.key, &VALUE, value)
        } else {
            map.resolved_write::<false, _>(&self.key, &VALUE, value)
        }
        .map_err(Refusal::Attachment)
    }
}
#[inline(never)]
fn attachment_handle_measured_passes(
    game: &mut crate::game::Game<'_>,
    handles: &ResolvedHandles,
    passes: usize,
) {
    for _ in 0..passes {
        for slot in 0..handles.records.len() {
            let handle = handles.token(slot);
            let value = handles.get(game, handle).unwrap();
            handles
                .set(game, handle, std::hint::black_box(value.wrapping_add(1)))
                .unwrap();
        }
    }
}
