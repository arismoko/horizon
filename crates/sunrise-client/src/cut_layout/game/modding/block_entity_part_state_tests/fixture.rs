//! A block entity Part whose tick exercises the Part state accessors.
use mod_api::*;

fn i32_codec() -> AttachmentCodec<i32> {
    AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |saved| match saved.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }
}

/// Each tick does what its block entity's first data slot says:
/// - 0: an aborted edit, a refused recursive edit, then a committed count;
/// - 1: an edit of an undecodable saved value is refused before its closure runs, then `mark`
///   is set to 1.
pub struct Counter {
    count: AttachmentKey<i32>,
    /// Set to 1 after the undecodable-value check, so the game sees it ran to the end.
    mark: AttachmentKey<i32>,
}
#[mod_api::sunrise_part]
impl BlockPart for Counter {}
#[mod_api::sunrise_part]
impl BlockEntityPart for Counter {
    fn block_entity_tick(&self, event: &mut BlockEntityTickEvent<'_>) {
        let (entity, mode) = event
            .subject
            .with_view(|entity| (entity.handle(), entity.data_slots()[0]))
            .unwrap();
        let host = event.subject.host();
        match mode {
            1 => {
                let refused = self.edit_state(host, entity, |_, _tx, _| panic!("ran"));
                assert_eq!(
                    refused,
                    Err(PartStateError::Attachment {
                        namespace: "part_state_fixture",
                        name: "part_state_count",
                        error: AttachmentError::Decode,
                    })
                );
                assert_eq!(
                    edit_part_state(host, entity, &self.mark, |_, _tx, m| *m = 1),
                    Ok(())
                );
            }
            _ => {
                let aborted: Result<(), mod_api::TransactionError<()>> =
                    host.transaction(|host, _| {
                        assert_eq!(self.edit_state(host, entity, |_, _tx, n| *n = 100), Ok(()));
                        Err(())
                    });
                assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
                let recursive = self.edit_state(host, entity, |host, _tx, n| {
                    *n += 1;
                    self.edit_state(host, entity, |_, _tx, n| *n = 50)
                });
                assert_eq!(recursive, Ok(Err(PartStateError::Reentrant)));
            }
        }
    }
}
impl BlockEntityPartState for Counter {
    type Data = i32;
    fn state_key(&self) -> &AttachmentKey<i32> {
        &self.count
    }
}

pub(crate) fn register(registrar: &mut dyn Registrar) {
    let count = registrar
        .register_attachment_type(
            AttachmentDescriptor::new("part_state_fixture:part_state_count", || 0_i32)
                .serialize(i32_codec()),
        )
        .unwrap();
    let mark = registrar
        .register_attachment_type(
            AttachmentDescriptor::new("part_state_fixture:part_state_mark", || 0_i32)
                .serialize(i32_codec()),
        )
        .unwrap();
    let kind = registrar
        .block_entity_type("part_state")
        .blocks(&["part_state_fixture:part_state"])
        .build();
    registrar
        .block_entity_part_with_state(&kind, Counter { count, mark })
        .unwrap();
}

/// The block the state's block entity stands in, so a save keeps it.
pub(crate) fn blocks(event: &mut BlocksEvent<'_>) {
    event
        .register(Block::new("part_state_fixture:part_state", "minecraft:barrel").with_item(false))
        .unwrap();
}
