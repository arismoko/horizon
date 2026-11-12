use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "disk_fixture",
    name: "value",
    default: || 0,
    codec: Some(horizon_mod_runtime::attachments::AttachmentCodec {
        encode: |value| Ok(Compound::from_iter([("value".into(), Tag::Int(*value))])),
        decode: |tag| match tag.get(&"value".into()) {
            Some(Tag::Int(value)) => Ok(*value),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

fn store(value: i32) -> Attachments {
    let mut store = Attachments::default();
    VALUE.set(&mut store, Holder::Level, value).unwrap();
    store
}
fn context(phase: ChunkDiskPhase) -> ChunkDiskContextWire {
    ChunkDiskContextWire::new(
        WorldEpoch::new(17),
        Identifier::parse("minecraft:overworld").unwrap(),
        [2, -3],
        ChunkDiskKind::Full,
        phase,
    )
}
fn read(entries: &Compound) -> i32 {
    VALUE
        .get(&mut Attachments::from_nbt(entries.clone()), Holder::Level)
        .unwrap()
}
fn run(job: &ChunkDiskJob) -> Result<Compound, ChunkDiskFailure> {
    job.run(&OwningWorldgen::new(&|_| None, &|_, _| None))
        .clone()
}
struct Listener {
    calls: Arc<AtomicUsize>,
    mode: u8,
}
#[crate::__subscriptions]
impl Hooks for Listener {
    fn chunk_data_save(&self, host: &dyn ChunkDiskHost, event: &mut ChunkDataSaveEvent<'_>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert!(host.backend().is_none());
        assert_eq!(event.data.get(&VALUE).unwrap(), 7);
        event.data.set(&VALUE, 8).unwrap();
        if self.mode == 1 {
            panic!("fixture unwind after tentative write");
        }
        if self.mode == 2 {
            horizon_mod_runtime::dispatch_failures::record_dispatch_failure();
        }
    }
    fn chunk_data_load(&self, _: &dyn ChunkDiskHost, event: &mut ChunkDataLoadEvent<'_>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let value = event.data.get(&VALUE).unwrap();
        event.data.set(&VALUE, value + 1).unwrap();
    }
}
fn with_listener(mode: u8, apply: impl FnOnce(Arc<AtomicUsize>)) {
    let calls = Arc::new(AtomicUsize::new(0));
    testing::with_dispatch(
        vec![ModEntry::new(
            "disk_fixture",
            Box::new(Listener {
                calls: calls.clone(),
                mode,
            }),
        )]
        .into(),
        || apply(calls),
    );
}

#[test]
fn capture_7_callback_8_live_9() {
    with_listener(0, |calls| {
        let mut live = store(7);
        let job = ChunkDiskJob::capture(&context(ChunkDiskPhase::Save), &live).unwrap();
        VALUE.set(&mut live, Holder::Level, 9).unwrap();
        assert_eq!(read(&run(&job).unwrap()), 8);
        assert_eq!(VALUE.get(&mut live, Holder::Level).unwrap(), 9);
        assert_eq!(read(&run(&job).unwrap()), 8);
        assert_eq!(read(job.captured()), 7);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    });
}

#[test]
fn load_disk_8_callback_increment_publishes_9_once() {
    with_listener(0, |calls| {
        let job = ChunkDiskJob::capture(&context(ChunkDiskPhase::Load), &store(8)).unwrap();
        assert_eq!(read(&run(&job).unwrap()), 9);
        assert_eq!(read(&run(&job).unwrap()), 9);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    });
}

#[test]
fn codec_error_unwind_or_trap_after_setting_8_refuses_partial_publication() {
    for mode in [1, 2] {
        with_listener(mode, |calls| {
            let job = ChunkDiskJob::capture(&context(ChunkDiskPhase::Save), &store(7)).unwrap();
            let refusal = run(&job).unwrap_err();
            assert_eq!(refusal.mod_id, "disk_fixture");
            assert_eq!(
                refusal.reason,
                if mode == 1 {
                    "native listener unwound"
                } else {
                    "wasm listener trapped or its Wire response was refused"
                }
            );
            assert_eq!(read(job.captured()), 7);
            assert_eq!(run(&job).unwrap_err(), refusal);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        });
    }
    let broken = AttachmentType {
        codec: Some(horizon_mod_runtime::attachments::AttachmentCodec {
            encode: |_| Err(AttachmentError::Encode),
            decode: |_| Ok(7),
        }),
        ..VALUE
    };
    let mut data = ChunkDiskData::from_nbt(store(7).to_nbt()).unwrap();
    assert!(data.set(&broken, 8).is_err());
    assert!(data.finish().is_err());
}

fn opaque(value: Tag) -> Compound {
    Compound::from_iter([(
        "unknown".into(),
        Tag::Compound(Compound::from_iter([(
            "value".into(),
            Tag::Compound(Compound::from_iter([("value".into(), value)])),
        )])),
    )])
}

#[test]
fn value_65537_depth_65_total_1048577_refused_without_truncation() {
    let oversize = opaque(Tag::ByteArray(vec![0; 65_537]));
    assert_eq!(
        ChunkDiskData::from_nbt(oversize).unwrap_err().reason,
        "encoded attachment exceeds 65536 bytes"
    );
    let mut deep = Tag::Int(8);
    for _ in 0..65 {
        deep = Tag::Compound(Compound::from_iter([("nested".into(), deep)]));
    }
    assert_eq!(
        ChunkDiskData::from_nbt(opaque(deep)).unwrap_err().reason,
        "attachment depth exceeds 64"
    );
    let mut names = Compound::new();
    for index in 0..18 {
        names.insert(
            format!("binding{index}").as_str().into(),
            Tag::Compound(Compound::from_iter([(
                "value".into(),
                Tag::ByteArray(vec![0; 60_000]),
            )])),
        );
    }
    let total = Compound::from_iter([("unknown".into(), Tag::Compound(names))]);
    assert_eq!(
        ChunkDiskData::from_nbt(total).unwrap_err().reason,
        "chunk attachments exceed 1048576 bytes"
    );
    let mut data = ChunkDiskData::from_nbt(opaque(Tag::Int(7))).unwrap();
    data.set(&VALUE, 8).unwrap();
    let result = data.finish().unwrap();
    assert_eq!(
        result.get(&"unknown".into()),
        opaque(Tag::Int(7)).get(&"unknown".into())
    );
}

#[test]
fn zero_subscribers_over_100_saves_creates_no_jobs_or_callbacks() {
    testing::with_dispatch(Vec::new().into(), || {
        let snapshot = store(7);
        let context = context(ChunkDiskPhase::Save);
        for _ in 0..100 {
            assert!(ChunkDiskJob::capture(&context, &snapshot).is_none());
        }
    });
}

#[test]
fn local_transaction_abort_preserves_prior_snapshot() {
    let mut data = ChunkDiskData::from_nbt(store(7).to_nbt()).unwrap();
    let result: Result<(), ()> = data.transaction(|data| {
        data.set(&VALUE, 8).unwrap();
        Err(())
    });
    assert_eq!(result, Err(()));
    assert_eq!(read(&data.finish().unwrap()), 7);
}

#[test]
fn type_without_persistence_codec_refuses_naming_type_and_capability() {
    let unsaved = AttachmentType {
        codec: None,
        ..VALUE
    };
    let mut data = ChunkDiskData::from_nbt(store(7).to_nbt()).unwrap();
    let refusal = data.get(&unsaved).unwrap_err();
    assert_eq!(refusal.key, "disk_fixture:value");
    assert_eq!(refusal.reason, "missing persistence codec");
    // The refusal poisons the whole job even if the listener ignores it.
    assert_eq!(data.finish().unwrap_err(), refusal);
}
