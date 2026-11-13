//! The same typed attachment job compiled natively and as a real wasm guest.
use mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, ChunkDiskData, ChunkDiskHost, Compound, Tag,
};

pub const VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "wasm_fixture",
    name: "disk_value",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |value| Ok(Compound::from_iter([("counter".into(), Tag::Int(*value))])),
        decode: |tag| match tag.get(&"counter".into()) {
            Some(Tag::Int(value)) => Ok(*value),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

pub fn save(host: &dyn ChunkDiskHost, data: &mut ChunkDiskData) {
    assert_eq!(data.get(&VALUE).unwrap(), 7);
    data.set(&VALUE, 8).unwrap();
    if host.job().pos == [99, 99] {
        panic!("guest trap after tentative write");
    }
}

pub fn load(_: &dyn ChunkDiskHost, data: &mut ChunkDiskData) {
    let value = data.get(&VALUE).unwrap();
    data.set(&VALUE, value + 1).unwrap();
}
