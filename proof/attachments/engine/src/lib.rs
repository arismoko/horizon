use horizon_client::mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, Compound, Flow, Holder, Hooks, Host,
    ItemStack, Tag,
};

fn encode(n: &i32) -> Result<Compound, AttachmentError> {
    Ok([("count".into(), Tag::Int(*n))].into())
}
fn decode(data: &Compound) -> Result<i32, AttachmentError> {
    match data.get(&"count".into()) {
        Some(Tag::Int(n)) => Ok(*n),
        _ => Err(AttachmentError::Decode),
    }
}
static SAVED: AttachmentType<i32> = AttachmentType {
    namespace: "attachment_proof",
    name: "saved",
    default: || 0,
    codec: Some(AttachmentCodec { encode, decode }),
    copy_on_death: false,
};
static MEMORY: AttachmentType<i32> = AttachmentType {
    namespace: "attachment_proof",
    name: "memory",
    default: || 0,
    codec: None,
    copy_on_death: false,
};
static KEEP: AttachmentType<i32> = AttachmentType {
    namespace: "attachment_proof",
    name: "keep",
    default: || 0,
    codec: Some(AttachmentCodec { encode, decode }),
    copy_on_death: true,
};

fn increment(
    ty: &AttachmentType<i32>,
    host: &mut dyn Host,
    holder: Holder,
) -> Result<i32, AttachmentError> {
    let old = ty.get(host, holder)?;
    ty.set(host, holder, old + 1)?;
    Ok(old)
}

struct Proof;
impl Hooks for Proof {
    fn right_click_block(&self, host: &mut dyn Host, pos: [i32; 3], _face: [i32; 3]) -> Flow {
        if !host.block(pos).contains("chest") {
            return Flow::Continue;
        }
        let chunk = Holder::Chunk([pos[0] >> 4, pos[2] >> 4]);
        let block = Holder::BlockEntity(pos);
        let level = increment(&SAVED, host, Holder::Level);
        let chunk_count = increment(&SAVED, host, chunk);
        let block_count = increment(&SAVED, host, block);
        let memory = increment(&MEMORY, host, block);
        let player = host.acting_player();
        let keep = player
            .as_ref()
            .map(|p| increment(&KEEP, host, Holder::Player(p.uuid)));
        let discard = player
            .as_ref()
            .map(|p| increment(&SAVED, host, Holder::Player(p.uuid)));
        eprintln!(
            "ATTACH_PROOF chest={pos:?} level={level:?} chunk={chunk_count:?} block={block_count:?} memory={memory:?} player_keep={keep:?} player_drop={discard:?}"
        );
        Flow::Cancel
    }
    fn interact_entity(
        &self,
        host: &mut dyn Host,
        entity: i32,
        kind: &str,
        _stack: Option<&ItemStack>,
    ) -> Flow {
        let count = increment(&SAVED, host, Holder::Entity(entity as u32));
        eprintln!("ATTACH_PROOF entity={entity} kind={kind} saved={count:?}");
        Flow::Cancel
    }
}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Proof)
}
