use super::*;
use std::sync::OnceLock;
use std::thread::{self, ThreadId};

#[derive(Default)]
struct ThreadProbe {
    load: Arc<OnceLock<ThreadId>>,
    save: Arc<OnceLock<ThreadId>>,
}

#[mod_api::sunrise_mod]
impl Hooks for ThreadProbe {
    fn chunk_data_load(&self, _: &dyn ChunkDiskHost, _: &mut ChunkDataLoadEvent<'_>) {
        self.load.set(thread::current().id()).unwrap();
    }
    fn chunk_data_save(&self, _: &dyn ChunkDiskHost, _: &mut ChunkDataSaveEvent<'_>) {
        self.save.set(thread::current().id()).unwrap();
    }
}

#[test]
fn chunk_disk_hooks_run_on_owner_while_region_io_runs_on_writer() {
    let (registry, biomes) = super::super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = tempfile::tempdir().unwrap();
    let caller = thread::current().id();
    let probe = ThreadProbe::default();
    let (on_load, on_save) = (probe.load.clone(), probe.save.clone());
    let mods = vec![mod_api::ModEntry::new("stack_probe", Box::new(probe))].into();
    mod_platform::testing::with_mods(mods, || {
        let save = SaveDir::new(
            path.path().to_path_buf(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        let writer_id = Arc::new(OnceLock::new());
        // `sync_data` runs inside each region write; `flush` forces regions on its caller.
        *save.writer.lock().unwrap() = Some(save.start_writer(
            {
                let writer_id = writer_id.clone();
                move |file| {
                    let _ = writer_id.set(thread::current().id());
                    #[cfg(target_os = "linux")]
                    {
                        let bounds = sunrise_threads::unwind::StackBounds::current().unwrap();
                        assert!(bounds.hi - bounds.lo >= sunrise_threads::GAME_THREAD_STACK_BYTES);
                    }
                    file.sync_data()
                }
            },
            RegionFile::sync_all,
        ));
        context(&save);
        save.save_later(&[Arc::new(chunk(7))], registry, &biomes)
            .unwrap();
        assert_eq!(on_save.get(), Some(&caller));
        save.flush().unwrap();
        assert_ne!(*writer_id.get().unwrap(), caller);
        let mut decoded = thread::scope(|scope| {
            scope
                .spawn(|| save.try_load(2, -3, registry, &biomes).unwrap().unwrap())
                .join()
                .unwrap()
        });
        assert!(on_load.get().is_none());
        save.load_disk_attachments(&mut decoded, mod_api::ChunkDiskKind::Full);
        assert_eq!(on_load.get(), Some(&caller));
        assert_eq!(value(&mut decoded), 7);
    });
}
