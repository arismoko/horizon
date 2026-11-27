//! Real region-file checks for the chunk worker's attachment boundary.
use super::*;
use mod_api::{ChunkDataLoadEvent, ChunkDataSaveEvent, ChunkDiskHost, Hooks, NbtDataType};
use std::sync::atomic::AtomicUsize;
mod fixture;
mod writer_stack;

#[derive(Default)]
struct Listener {
    saves: Arc<AtomicUsize>,
    loads: Arc<AtomicUsize>,
    value: Mutex<Option<mod_api::AttachmentKey<i32>>>,
}
#[mod_api::sunrise_mod]
impl Hooks for Listener {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        *self.value.lock().unwrap() = Some(fixture::register(r));
    }
    fn chunk_data_save(&self, host: &dyn ChunkDiskHost, event: &mut ChunkDataSaveEvent<'_>) {
        self.saves.fetch_add(1, Ordering::Relaxed);
        let names: &[String] = host.registry("minecraft:block").unwrap();
        assert_eq!(names, ["minecraft:stone", "minecraft:dirt"]);
        check_saved_nbt_upgrade(host);
        let key = self.value.lock().unwrap().clone().unwrap();
        fixture::save(&key, event.data);
    }
    fn chunk_data_load(&self, host: &dyn ChunkDiskHost, event: &mut ChunkDataLoadEvent<'_>) {
        self.loads.fetch_add(1, Ordering::Relaxed);
        check_saved_nbt_upgrade(host);
        let key = self.value.lock().unwrap().clone().unwrap();
        fixture::load(&key, host, event.data);
    }
}

fn check_saved_nbt_upgrade(host: &dyn ChunkDiskHost) {
    let state =
        |key: &str, name: &str| Compound::from_iter([(key.into(), Tag::String(name.into()))]);
    assert_eq!(
        host.upgrade_nbt(
            NbtDataType::BlockState,
            2586,
            state("Name", "minecraft:grass_path")
        ),
        Ok(state("id", "minecraft:dirt_path")),
    );
}

pub(super) fn context(save: &SaveDir) {
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(
            BTreeMap::from([(
                "minecraft:block".into(),
                vec!["minecraft:stone".into(), "minecraft:dirt".into()],
            )]),
            save.root.join("settings"),
        )),
        [],
    );
    save.set_chunk_disk_context(
        mod_api::WorldEpoch::new(17),
        mod_api::Identifier::parse("minecraft:overworld").unwrap(),
        Arc::new(load.worldgen_context([])),
    );
}
fn chunk(value: i32) -> Chunk {
    let mut chunk = Chunk::empty(2, -3);
    let mut extras = ChunkExtras::default();
    fixture::raw_value()
        .set(&mut extras.attachments, mod_api::Holder::Level, value)
        .unwrap();
    chunk.saved = Some(Arc::new(extras));
    chunk
}
fn value(chunk: &mut Chunk) -> i32 {
    fixture::raw_value()
        .get(
            &mut Arc::make_mut(chunk.saved.as_mut().unwrap()).attachments,
            mod_api::Holder::Level,
            Clone::clone,
        )
        .unwrap()
}
fn root() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "sunrise-chunk-disk-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}
pub(super) fn with_listener(run: impl FnOnce(Arc<AtomicUsize>, Arc<AtomicUsize>)) {
    let saves = Arc::new(AtomicUsize::new(0));
    let loads = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "chunk_disk_fixture",
        Box::new(Listener {
            saves: saves.clone(),
            loads: loads.clone(),
            value: Mutex::new(None),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        run(saves, loads);
    });
}

#[test]
fn a_disk_encoder_panic_leaves_the_original_readable_and_siblings_durable() {
    use std::{sync::mpsc, time::Duration};

    let (registry, biomes) = super::loot_tests::fixture();
    let encoding = Arc::new(Encoding {
        registry: Box::leak(Box::new(registry)),
        biomes: Arc::new(biomes),
        items: Default::default(),
        tables: None,
        shape: LevelShape::OVERWORLD,
    });
    let dir = tempfile::tempdir().unwrap();
    let open = || SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
    with_listener(|_, _| {
        let save = open();
        context(&save);
        let key = (2, -3);
        let mut wrapped = save
            .capture_disk_record(Record::Chunk(Arc::new(chunk(0)), encoding, None))
            .expect("disk capture must succeed");
        let Record::Disk(disk) = &mut wrapped else {
            panic!("save callback must be captured")
        };
        let marker = |i| NamedTag {
            name: "".into(),
            tag: Tag::compound([("marker", Tag::Int(i))]),
        };
        let original = Arc::new(marker(42));
        Arc::get_mut(disk).unwrap().original = Record::TestEncode(Arc::new({
            let original = original.clone();
            move |edits| {
                assert!(!edits, "injected disk encoder panic");
                Some(original.clone())
            }
        }));
        let neighbor = Arc::new(marker(7));
        let borrowed = wrapped.clone();
        save.queue(
            Folder::Region,
            vec![(key, wrapped), ((3, -3), Record::Tag(neighbor.clone()))],
            false,
        )
        .unwrap();
        let (send, receive) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            send.send(borrowed.tag()).unwrap();
        });
        save.flush().unwrap();
        assert_eq!(save.record_save_failures(), 1);
        let pending = receive
            .recv_timeout(Duration::from_secs(1))
            .expect("disk reader did not finish");
        reader.join().unwrap();
        assert_eq!(pending.as_deref(), Some(original.as_ref()));
        assert!(save.raw_chunk(key.0, key.1).is_none());
        let disk = open();
        assert_eq!(disk.raw_chunk(3, -3).as_ref(), Some(neighbor.as_ref()));
        assert!(disk.raw_chunk(key.0, key.1).is_none());
    });
}

#[test]
fn capture_7_callback_8_live_9_round_trips_through_region_writer() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = root();
    with_listener(|saves, _| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        let mut live = chunk(7);
        save.save_later(&[Arc::new(live.clone())], registry, &biomes)
            .unwrap();
        fixture::raw_value()
            .set(
                &mut Arc::make_mut(live.saved.as_mut().unwrap()).attachments,
                mod_api::Holder::Level,
                9,
            )
            .unwrap();
        save.flush().unwrap();
        assert_eq!(saves.load(Ordering::Relaxed), 1);
        assert_eq!(value(&mut live), 9);
        let disk = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        assert_eq!(value(&mut disk.load(2, -3, registry, &biomes).unwrap()), 8);
    });
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn load_disk_8_callback_increment_publishes_9_once_through_decode() {
    let (registry, biomes) = super::loot_tests::fixture();
    let path = root();
    mod_platform::testing::with_mods(Vec::new().into(), || {
        SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        )
        .save(&[Arc::new(chunk(8))], &registry, &biomes)
        .unwrap();
    });
    with_listener(|_, loads| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        assert_eq!(value(&mut save.load(2, -3, &registry, &biomes).unwrap()), 9);
        assert_eq!(loads.load(Ordering::Relaxed), 1);
    });
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn full_and_proto_writer_records_each_invoke_once_before_encode() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = root();
    with_listener(|saves, _| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        save.save_later(&[Arc::new(chunk(7))], registry, &biomes)
            .unwrap();
        save.save_protos(
            vec![SavedProto {
                column: chunk(7),
                decorated: true,
                block_entities: Vec::new(),
                entities: Vec::new(),
            }],
            registry,
            &biomes,
        )
        .unwrap();
        save.flush().unwrap();
        assert_eq!(saves.load(Ordering::Relaxed), 2);
        assert_eq!(
            value(
                &mut SaveDir::new(path.clone(), LevelShape::OVERWORLD, None)
                    .load_proto(2, -3, registry, &biomes)
                    .unwrap()
                    .column
            ),
            8
        );
    });
    fs::remove_dir_all(path).unwrap();
}

/// Records what each save callback saw, in the order the writer ran them.
// Its own module: `sunrise_mod` defines one `hooks` function per module.
mod echo {
    use super::*;
    #[derive(Default)]
    pub struct Echo {
        pub seen: Arc<Mutex<Vec<(mod_api::ChunkDiskKind, i32)>>>,
        pub value: Mutex<Option<mod_api::AttachmentKey<i32>>>,
    }
    #[mod_api::sunrise_mod]
    impl Hooks for Echo {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            *self.value.lock().unwrap() = Some(fixture::register(r));
        }
        fn chunk_data_save(&self, host: &dyn ChunkDiskHost, event: &mut ChunkDataSaveEvent<'_>) {
            // A worker callback has no live world: the old handle's path refuses.
            assert!(host.backend().is_none());
            let key = self.value.lock().unwrap().clone().unwrap();
            let value = event.data.get(&key).unwrap();
            self.seen.lock().unwrap().push((host.job().kind, value));
        }
    }
}
use echo::Echo;
fn with_echo(run: impl FnOnce(Arc<Mutex<Vec<(mod_api::ChunkDiskKind, i32)>>>)) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "chunk_disk_fixture",
        Box::new(Echo {
            seen: seen.clone(),
            value: Mutex::new(None),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        run(seen);
    });
}
fn disk_value(path: &Path, registry: &BlockRegistry, biomes: &Biomes) -> Option<i32> {
    SaveDir::new(
        path.to_path_buf(),
        LevelShape::OVERWORLD,
        Some(Default::default()),
    )
    .load(2, -3, registry, biomes)
    .map(|mut chunk| value(&mut chunk))
}

#[test]
fn revisions_10_then_11_queued_together_leave_disk_at_11() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = root();
    with_echo(|seen| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        save.save_later(&[Arc::new(chunk(10))], registry, &biomes)
            .unwrap();
        save.save_later(&[Arc::new(chunk(11))], registry, &biomes)
            .unwrap();
        save.flush().unwrap();
        // Each captured record had its callback, whether or not its write was
        // coalesced away; only the newest revision is on disk.
        let seen: Vec<i32> = seen.lock().unwrap().iter().map(|(_, v)| *v).collect();
        assert_eq!(seen, [10, 11]);
        assert_eq!(disk_value(&path, registry, &biomes), Some(11));
    });
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn every_save_kind_runs_exactly_one_callback_per_captured_record() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = root();
    with_echo(|seen| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        // Asynchronous full saves: explicit, forced, late and unload all queue this way.
        save.save_later(&[Arc::new(chunk(1))], registry, &biomes)
            .unwrap();
        save.save_later(&[Arc::new(chunk(2))], registry, &biomes)
            .unwrap();
        save.flush().unwrap();
        // The synchronous route used by closing and forced saves.
        save.save(&[Arc::new(chunk(3))], registry, &biomes).unwrap();
        // Proto chunks.
        save.save_protos(
            vec![SavedProto {
                column: chunk(4),
                decorated: true,
                block_entities: Vec::new(),
                entities: Vec::new(),
            }],
            registry,
            &biomes,
        )
        .unwrap();
        save.flush().unwrap();
        use mod_api::ChunkDiskKind::{Full, Proto};
        assert_eq!(
            *seen.lock().unwrap(),
            [(Full, 1), (Full, 2), (Full, 3), (Proto, 4)]
        );
    });
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn no_worker_context_keeps_the_ordinary_record_writer_available() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = tempfile::tempdir().unwrap();
    let open = || {
        SaveDir::new(
            path.path().to_path_buf(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        )
    };
    with_echo(|seen| {
        let save = open();
        save.save_later(&[Arc::new(chunk(7))], registry, &biomes)
            .unwrap();
        save.flush().unwrap();
        assert_eq!(
            value(&mut open().load(2, -3, registry, &biomes).unwrap()),
            7
        );
        save.save(&[Arc::new(chunk(9))], registry, &biomes).unwrap();
        assert_eq!(
            value(&mut open().load(2, -3, registry, &biomes).unwrap()),
            9
        );
        assert!(seen.lock().unwrap().is_empty());
    });
}

#[test]
fn unload_then_reload_reads_saved_8_in_a_fresh_holder() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = root();
    with_listener(|_, _| {
        let save = SaveDir::new(
            path.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        save.save_later(&[Arc::new(chunk(7))], registry, &biomes)
            .unwrap();
        save.flush().unwrap();
    });
    // No listener on reload: the stored value is exactly what the save callback wrote.
    mod_platform::testing::with_mods(Vec::new().into(), || {
        assert_eq!(disk_value(&path, registry, &biomes), Some(8));
    });
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn invalid_attachment_edits_keep_prior_values_and_publish_terrain_and_siblings() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = tempfile::tempdir().unwrap();
    let open = || {
        SaveDir::new(
            path.path().to_path_buf(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        )
    };
    let mut original = chunk(7);
    let saved_key = Tag::compound([("value", Tag::compound([("prior", Tag::Int(33))]))]);
    let Tag::Compound(raw) = Tag::compound([(
        "chunk_disk_fixture",
        Tag::compound([("invalid", saved_key.clone())]),
    )]) else {
        unreachable!()
    };
    Arc::make_mut(original.saved.as_mut().unwrap())
        .attachments
        .read_saved_over(raw);
    mod_platform::testing::with_mods(Vec::new().into(), || {
        open()
            .save(&[Arc::new(original)], registry, &biomes)
            .unwrap();
    });
    let read_disk = || {
        RegionFile::open(path.path().join("region/r.0.-1.mca"))
            .unwrap()
            .read(ChunkPos::new(2, -3))
            .unwrap()
            .unwrap()
            .nbt(Limits::saved())
            .unwrap()
    };
    let check_prior = |record: &NamedTag| {
        assert_eq!(
            record
                .tag
                .get(ATTACHMENTS_KEY)
                .unwrap()
                .get("chunk_disk_fixture")
                .unwrap()
                .get("invalid"),
            Some(&saved_key)
        );
    };
    check_prior(&read_disk());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "chunk_disk_fixture",
        Box::new(invalid_nbt::InvalidNbt::with_seen(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        // A named End and a locally writable but unreadably deep payload both refuse in the
        // callback; each proposed replacement is based on the previous readable region slot.
        for (bad, synchronous, x) in [(9, false, 0), (10, true, 1)] {
            let save = open();
            context(&save);
            let mut replacement = save.load(2, -3, registry, &biomes).unwrap();
            replacement.set(x, 0, 0, 1);
            fixture::raw_value()
                .set(
                    &mut Arc::make_mut(replacement.saved.as_mut().unwrap()).attachments,
                    mod_api::Holder::Level,
                    bad,
                )
                .unwrap();
            let chunks = [Arc::new(replacement)];
            if synchronous {
                save.save(&chunks, registry, &biomes).unwrap();
            } else {
                save.save_later(&chunks, registry, &biomes).unwrap();
                save.flush().unwrap();
            }
            check_prior(&read_disk());
            let mut reloaded = open().load(2, -3, registry, &biomes).unwrap();
            assert_eq!(reloaded.get(x, 0, 0), 1);
            assert_eq!(value(&mut reloaded), bad + 100);
            assert_eq!(save.record_save_failures(), 0);
        }
    });
    assert_eq!(*seen.lock().unwrap(), [9, 10]);
}

mod invalid_nbt {
    use super::*;

    #[derive(Default)]
    pub struct InvalidNbt {
        pub seen: Arc<Mutex<Vec<i32>>>,
        value: Mutex<Option<mod_api::AttachmentKey<i32>>>,
        invalid: Mutex<Option<mod_api::AttachmentKey<i32>>>,
    }

    impl InvalidNbt {
        pub(super) fn with_seen(seen: Arc<Mutex<Vec<i32>>>) -> Self {
            Self {
                seen,
                ..Default::default()
            }
        }
    }

    #[mod_api::sunrise_mod]
    impl Hooks for InvalidNbt {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            *self.value.lock().unwrap() = Some(fixture::register(r));
            let descriptor = mod_api::AttachmentDescriptor::new("chunk_disk_fixture:invalid", || 0)
                .serialize(mod_api::AttachmentCodec {
                    encode: |kind| {
                        let mut payload = Tag::End;
                        if *kind == 10 {
                            payload = Tag::Int(1);
                            for _ in 0..Limits::saved().max_depth {
                                payload = Tag::List(vec![payload]);
                            }
                        }
                        Ok(Compound::from([("bad".into(), payload)]))
                    },
                    decode: |_| Ok(33),
                });
            *self.invalid.lock().unwrap() = Some(r.register_attachment_type(descriptor).unwrap());
        }
        fn chunk_data_save(&self, _: &dyn ChunkDiskHost, event: &mut ChunkDataSaveEvent<'_>) {
            let key = self.value.lock().unwrap().clone().unwrap();
            let value = event.data.get(&key).unwrap();
            self.seen.lock().unwrap().push(value);
            if matches!(value, 9 | 10) {
                let invalid = self.invalid.lock().unwrap().clone().unwrap();
                let refusal = event.data.set(&invalid, value).unwrap_err();
                assert_eq!(refusal.key, "chunk_disk_fixture:invalid");
                assert_eq!(
                    refusal.reason,
                    mod_api::ChunkDiskFailureKind::InvalidSavedNbt
                );
                event.data.set(&key, value + 100).unwrap();
            }
        }
    }
}

#[test]
fn a_coalesced_record_runs_its_callback_without_writing_its_invalid_nbt() {
    use std::time::Duration;

    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "chunk_disk_fixture",
        Box::new(invalid_nbt::InvalidNbt::with_seen(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let save = SaveDir::new(
            dir.path().to_path_buf(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        let (entered, wait_for_encoder) = mpsc::channel();
        let (release, wait_for_release) = mpsc::channel();
        let wait_for_release = Mutex::new(wait_for_release);
        let blocker = Record::TestEncode(Arc::new(move |_| {
            entered.send(()).unwrap();
            wait_for_release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(1))
                .unwrap();
            None
        }));
        save.queue(Folder::Region, vec![((0, 0), blocker)], false)
            .unwrap();
        wait_for_encoder
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        // Enqueue both revisions before the writer can drain its next pass.
        save.save_later(&[Arc::new(chunk(9))], registry, &biomes)
            .unwrap();
        save.save_later(&[Arc::new(chunk(11))], registry, &biomes)
            .unwrap();
        release.send(()).unwrap();
        save.flush().unwrap();
        assert_eq!(save.record_save_failures(), 0);
        assert_eq!(*seen.lock().unwrap(), [9, 11]);
        assert_eq!(disk_value(dir.path(), registry, &biomes), Some(11));
    });
}

#[test]
fn a_biome_admission_refusal_keeps_the_rest_of_the_batch_writable() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry: &'static BlockRegistry = Box::leak(Box::new(registry));
    let names = (0..u16::MAX)
        .map(|i| format!("test:biome_{i}"))
        .collect::<Vec<_>>();
    let exhausted = biomes
        .with_packs(
            names
                .iter()
                .map(|name| (name.as_str(), &serde_json::Value::Null)),
        )
        .unwrap();
    assert_eq!(exhausted.saved_id("absent:last"), u16::MAX);
    exhausted.saved_id("absent:overflow");
    assert!(exhausted.ensure_saveable().is_err());
    let biomes = Arc::new(biomes);
    let encoding = |biomes| {
        Arc::new(Encoding {
            registry,
            biomes,
            items: Default::default(),
            tables: None,
            shape: LevelShape::OVERWORLD,
        })
    };
    let record = |chunk, biomes| Record::Chunk(Arc::new(chunk), encoding(biomes), None);
    let refused = record(chunk(9), Arc::new(exhausted));
    let mut healthy = chunk(11);
    healthy.x = 3;
    let healthy = record(healthy, biomes.clone());
    let run = || {
        let dir = tempfile::tempdir().unwrap();
        let open = || SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
        let save = open();
        context(&save);
        save.queue(
            Folder::Region,
            vec![((2, -3), refused.clone()), ((3, -3), healthy.clone())],
            true,
        )
        .unwrap();
        assert_eq!(save.record_save_failures(), 1);
        assert!(open().raw_chunk(2, -3).is_none());
        assert_eq!(
            value(&mut open().load(3, -3, registry, &biomes).unwrap()),
            11
        );
    };
    mod_platform::testing::with_mods(Vec::new().into(), run);
    with_echo(|seen| {
        run();
        let seen: Vec<i32> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, value)| *value)
            .collect();
        assert_eq!(seen, [9, 11]);
    });
}

#[test]
fn an_unencodable_record_without_a_listener_does_not_skip_other_regions() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let path = tempfile::tempdir().unwrap();
        let open = || SaveDir::new(path.path().to_path_buf(), LevelShape::OVERWORLD, None);
        let save = open();
        let key = (2, -3);
        let neighbour = (3, -3);
        let other_region = (32, -3);
        let invalid = (0..=sunrise_core::nbt::DISK_WRITE_NESTING_GUARD)
            .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
        let record = |key, tag| {
            (
                key,
                Some(NamedTag {
                    name: "".into(),
                    tag,
                }),
            )
        };
        let valid = |marker| {
            Tag::compound([
                ("DataVersion", Tag::Int(DATA_VERSION)),
                ("marker", Tag::Int(marker)),
            ])
        };
        save.write_records("region", vec![record(key, valid(1))], true)
            .unwrap();
        save.write_records(
            "region",
            vec![
                record(key, invalid),
                record(neighbour, valid(3)),
                record(other_region, valid(4)),
            ],
            true,
        )
        .unwrap();
        let disk = open();
        for ((x, z), marker) in [(key, 1), (neighbour, 3), (other_region, 4)] {
            assert_eq!(
                disk.raw_chunk(x, z).unwrap().tag.get("marker"),
                Some(&Tag::Int(marker))
            );
        }
        assert_eq!(save.record_save_failures(), 1);
        save.wait_for_chunk_writes_below(0).unwrap();
    });
}

#[test]
fn region_io_failure_is_reported_without_stopping_healthy_regions() {
    let dir = tempfile::tempdir().unwrap();
    let open = || SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
    let save = open();
    for folder in ["region", "entities", "poi"] {
        let blocked = dir.path().join(folder).join("r.0.0.mca");
        // A directory in place of the file fails even for a privileged user.
        fs::create_dir_all(blocked).unwrap();
        let marker = || NamedTag {
            name: "".into(),
            tag: Tag::compound([("marker", Tag::Int(7))]),
        };
        let before = save.record_save_failures();
        save.write_records(
            folder,
            vec![
                ((0, 0), Some(marker())),
                ((1, 0), None),
                ((32, 0), Some(marker())),
            ],
            true,
        )
        .unwrap();
        assert_eq!(save.record_save_failures() - before, 2);
        assert_eq!(open().read_record_raw(folder, 32, 0), Some(marker()));
        // The clear-only path must report a broken file, not mistake it for a missing file.
        save.write_records(folder, vec![((0, 0), None), ((32, 0), None)], true)
            .unwrap();
        assert_eq!(save.record_save_failures() - before, 3);
        assert!(open().read_record_raw(folder, 32, 0).is_none());
    }
}

/// A panicking load listener does not suppress a separate save listener on any save path.
#[test]
fn panicking_load_listener_keeps_saved_value_and_does_not_disable_save_hook() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    for (load_only, sync) in [(true, true), (true, false), (false, true), (false, false)] {
        let path = root();
        mod_platform::testing::with_mods(Vec::new().into(), || {
            SaveDir::new(
                path.clone(),
                LevelShape::OVERWORLD,
                Some(Default::default()),
            )
            .save(&[Arc::new(chunk(8))], registry, &biomes)
            .unwrap();
        });
        let saves = Arc::new(AtomicUsize::new(0));
        let listener: Box<dyn Hooks> = if load_only {
            Box::new(breaker::Breaker)
        } else {
            Box::new(breaker_both::BreakerBoth {
                saves: saves.clone(),
            })
        };
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("chunk_disk_fixture", listener)].into();
        mod_platform::testing::with_mods(mods, || {
            let save = SaveDir::new(
                path.clone(),
                LevelShape::OVERWORLD,
                Some(Default::default()),
            );
            context(&save);
            let mut loaded = save
                .load(2, -3, registry, &biomes)
                .expect("the chunk loads");
            // The stored value is still there.
            assert_eq!(value(&mut loaded), 8);
            let chunks = [Arc::new(loaded)];
            if sync {
                save.save(&chunks, registry, &biomes).unwrap();
            } else {
                save.save_later(&chunks, registry, &biomes).unwrap();
                save.flush().unwrap();
            }
            assert_eq!(save.record_save_failures(), 0);
        });
        assert_eq!(saves.load(Ordering::Relaxed), usize::from(!load_only));
        mod_platform::testing::with_mods(Vec::new().into(), || {
            assert_eq!(disk_value(&path, registry, &biomes), Some(8));
        });
        fs::remove_dir_all(path).unwrap();
    }
}

// Its own module: `sunrise_mod` defines one `hooks` function per module.
mod breaker {
    use super::*;
    #[derive(Default)]
    pub struct Breaker;
    #[mod_api::sunrise_mod]
    impl Hooks for Breaker {
        fn chunk_data_load(&self, _: &dyn ChunkDiskHost, _: &mut ChunkDataLoadEvent<'_>) {
            panic!("load listener fails");
        }
    }
}
mod breaker_both {
    use super::*;
    #[derive(Default)]
    pub struct BreakerBoth {
        pub saves: Arc<AtomicUsize>,
    }
    #[mod_api::sunrise_mod]
    impl Hooks for BreakerBoth {
        fn chunk_data_load(&self, _: &dyn ChunkDiskHost, _: &mut ChunkDataLoadEvent<'_>) {
            panic!("load listener fails");
        }
        fn chunk_data_save(&self, _: &dyn ChunkDiskHost, _: &mut ChunkDataSaveEvent<'_>) {
            self.saves.fetch_add(1, Ordering::Relaxed);
        }
    }
}
