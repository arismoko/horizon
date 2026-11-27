//! The attachment and its save/load edits used by the region writer tests.
use mod_api::{
    AttachmentCodec, AttachmentDescriptor, AttachmentError, AttachmentKey, ChunkDiskData,
    ChunkDiskHost, Compound, Registrar, Tag,
};
use sunrise_mod_runtime::attachments::{AttachmentType, CheckedAttachmentType};

const CODEC: AttachmentCodec<i32> = AttachmentCodec {
    encode: |value| Ok(Compound::from_iter([("counter".into(), Tag::Int(*value))])),
    decode: |tag| match tag.get(&"counter".into()) {
        Some(Tag::Int(value)) => Ok(*value),
        _ => Err(AttachmentError::Decode),
    },
};

pub fn register(registrar: &mut dyn Registrar) -> AttachmentKey<i32> {
    registrar
        .register_attachment_type(
            AttachmentDescriptor::new("chunk_disk_fixture:disk_value", || 0).serialize(CODEC),
        )
        .unwrap()
}

/// Direct storage descriptor for test setup and assertions outside a mod installation.
pub fn raw_value() -> CheckedAttachmentType<i32> {
    CheckedAttachmentType::new(AttachmentType {
        namespace: "chunk_disk_fixture",
        name: "disk_value",
        default: || 0,
        codec: Some(CODEC),
        copy_on_death: false,
        sync_codec: None,
    })
    .unwrap()
}

/// Only the chunk that the test fills with a value needs editing.
pub fn save(key: &AttachmentKey<i32>, data: &mut ChunkDiskData) {
    let value = data.get(key).unwrap();
    if value == 0 {
        return;
    }
    assert_eq!(value, 7);
    data.set(key, 8).unwrap();
}

pub fn load(key: &AttachmentKey<i32>, _: &dyn ChunkDiskHost, data: &mut ChunkDiskData) {
    let value = data.get(key).unwrap();
    if value != 0 {
        data.set(key, value + 1).unwrap();
    }
}
