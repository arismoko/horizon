use std::sync::Arc;

use sunrise_core::{
    nbt::{Limits, NamedTag, Tag},
    position::ChunkPos,
};
use sunrise_world::region::RegionFile;

use super::{
    Chunk, ChunkExtras, Compression, Encoding, Folder, LevelShape, Record,
    compression_pool::{
        CompressionPool, compress_record, compress_records, compress_records_with_pool,
    },
    write_chunk_record, write_prepared_region_records,
    writes::QueuedWrite,
};

#[test]
fn full_record_overflow_discards_edits_without_losing_current_chunk_data() {
    use crate::containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields};
    use sunrise_core::nbt::{Compound, write_named};

    let (registry, biomes) = super::loot_tests::fixture();
    let mut chunk = Chunk::empty(0, 0);
    let pos = (1, 64, 1);
    chunk.block_entities.push((
        pos,
        BlockEntity::Mod(Box::new(ModBlockEntity::new(
            "test:machine".into(),
            ModBlockEntityFields {
                nbt: Compound::from([("".into(), Tag::Int(82)), ("stored".into(), Tag::Int(7))]),
                ..Default::default()
            },
        ))),
    ));
    let edited = Compound::from([
        ("".into(), Tag::Int(82)),
        ("edit".into(), Tag::ByteArray(vec![9; 64 * 1024])),
    ]);
    let limits = Limits {
        max_bytes: 96 * 1024,
        ..Limits::saved()
    };
    // The edit fits by itself. Combined with the rest of the captured record it does not.
    assert!(
        write_named(
            &NamedTag {
                name: "".into(),
                tag: Tag::Compound(edited.clone())
            },
            limits
        )
        .is_ok()
    );
    let unrelated = Tag::ByteArray(vec![11; 64 * 1024]);
    chunk.saved = Some(Arc::new(ChunkExtras {
        last_update: 37,
        root: Compound::from([("test:unrelated".into(), unrelated.clone())]),
        block_entity_save_edits: Some(Arc::new([(pos, edited)].into())),
        ..Default::default()
    }));
    let encoding = Arc::new(Encoding {
        registry: Box::leak(Box::new(registry)),
        biomes: Arc::new(biomes),
        items: Default::default(),
        tables: None,
        shape: LevelShape::OVERWORLD,
    });
    let record = Record::Chunk(Arc::new(chunk), encoding, None);
    let dir = tempfile::tempdir().unwrap();
    let mut region = RegionFile::open_writable(dir.path().join("r.0.0.mca"), true).unwrap();
    let region_pos = ChunkPos::new(0, 0);
    for (limits, compressed_overflow, admitted) in [
        (Limits::saved(), false, true),
        (limits, false, false),
        (Limits::saved(), true, false),
    ] {
        let (record, compression) = if compressed_overflow {
            let Record::Chunk(chunk, encoding, blending) = &record else {
                unreachable!()
            };
            let mut chunk = (**chunk).clone();
            let extras = Arc::make_mut(chunk.saved.as_mut().unwrap());
            let edits = Arc::make_mut(extras.block_entity_save_edits.as_mut().unwrap());
            edits
                .get_mut(&pos)
                .unwrap()
                .insert("edit".into(), Tag::ByteArray(vec![0; 64 * 1024 * 1024]));
            (
                Record::Chunk(Arc::new(chunk), encoding.clone(), blending.clone()),
                Compression::None,
            )
        } else {
            (record.clone(), Compression::Zlib)
        };
        let job = QueuedWrite::new((0, 0), record, 0);
        let pending = job.record.clone();
        let compressed = compress_record(&job.record, compression, 0, limits)
            .unwrap()
            .unwrap();
        assert_eq!(
            compressed.primary, admitted,
            "prepare the stored fallback before writing the region"
        );
        write_chunk_record(&mut region, region_pos, &job.record, compressed).unwrap();
        drop(job);
        let saved = region
            .read(region_pos)
            .unwrap()
            .unwrap()
            .nbt(limits)
            .unwrap();
        assert_eq!(pending.tag().unwrap().as_ref(), &saved);
        assert_eq!(saved.tag.get("LastUpdate"), Some(&Tag::Long(37)));
        assert_eq!(saved.tag.get("test:unrelated"), Some(&unrelated));
        let Some(Tag::List(entities)) = saved.tag.get("block_entities") else {
            panic!("block entities missing from the record");
        };
        assert_eq!(entities[0].get(""), Some(&Tag::Int(82)));
        assert_eq!(entities[0].get("edit").is_some(), admitted);
        assert_eq!(
            entities[0].get("stored"),
            if admitted { None } else { Some(&Tag::Int(7)) }
        );
    }
}

#[test]
fn an_encoder_panic_does_not_strand_serial_or_parallel_writes() {
    use super::{Folder, SaveDir};
    use std::{sync::mpsc, time::Duration};

    for count in [3, 4] {
        let dir = tempfile::tempdir().unwrap();
        let open = || SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
        let save = open();
        save.write_records("region", vec![((1, 0), Some(sample_tag(10)))], true)
            .unwrap();
        let records = (0..count)
            .map(|x| {
                let record = if x == 1 {
                    Record::TestEncode(Arc::new(|_| panic!("injected encoding panic")))
                } else {
                    Record::Tag(Arc::new(sample_tag(20 + x)))
                };
                ((x, 0), record)
            })
            .collect();
        let (send, receive) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            save.queue(Folder::Region, records, false).unwrap();
            save.flush().unwrap();
            send.send((save.record_save_failures(), save.active_chunk_writes()))
                .unwrap();
        });
        let (failures, active) = receive
            .recv_timeout(Duration::from_secs(1))
            .expect("flush waited indefinitely for the panicking batch");
        worker.join().unwrap();
        assert_eq!(failures, 1);
        assert_eq!(active, 0);
        let disk = open();
        for x in 0..count {
            let marker = if x == 1 { 10 } else { 20 + x };
            assert_eq!(
                disk.raw_chunk(x, 0).unwrap().tag.get("xPos"),
                Some(&Tag::Int(marker))
            );
        }
    }
}

#[test]
fn a_compressed_size_fallback_panic_is_rejected_before_region_writes() {
    let oversized = Arc::new(NamedTag {
        name: "".into(),
        tag: Tag::ByteArray(vec![0; 64 * 1024 * 1024]),
    });
    let record = Record::TestEncode(Arc::new(move |edits| {
        assert!(edits, "injected fallback encoder panic");
        Some(oversized.clone())
    }));
    assert!(compress_record(&record, Compression::None, 0, Limits::saved()).is_err());
}

#[test]
fn a_compressed_size_rejection_does_not_skip_healthy_records_in_either_region() {
    use super::SaveDir;
    use sunrise_core::nbt::Compound;

    const OVERSIZED: usize = 64 * 1024 * 1024 + 1;
    let (registry, biomes) = super::loot_tests::fixture();
    let encoding = Arc::new(Encoding {
        registry: Box::leak(Box::new(registry)),
        biomes: Arc::new(biomes),
        items: Default::default(),
        tables: None,
        shape: LevelShape::OVERWORLD,
    });
    for with_fallback in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let open = || SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
        let save = open();
        save.write_records("region", vec![((1, 0), Some(sample_tag(1)))], true)
            .unwrap();
        let old = open().raw_chunk(1, 0).unwrap();
        let mut chunk = Chunk::empty(1, 0);
        // Dropping the block-entity edits still leaves this oversized stored payload.
        chunk.saved = Some(Arc::new(ChunkExtras {
            root: Compound::from([("test:stored".into(), Tag::ByteArray(vec![0; OVERSIZED]))]),
            block_entity_save_edits: with_fallback.then(Arc::default),
            ..Default::default()
        }));
        let bad = Record::Chunk(Arc::new(chunk), encoding.clone(), None);
        let healthy = Record::Tag(Arc::new(sample_tag(11)));
        let neighbour = Record::Tag(Arc::new(sample_tag(33)));
        let later = Record::Tag(Arc::new(sample_tag(22)));
        let records = [
            ((0, 0), &healthy),
            ((1, 0), &bad),
            ((2, 0), &neighbour),
            ((32, 0), &later),
        ];
        let prepared = records
            .iter()
            .map(|(_, record)| compress_record(record, Compression::None, 0, Limits::saved()))
            .collect();
        let outcome = write_prepared_region_records(
            save.region_storage(Folder::Region),
            0,
            &records,
            prepared,
            &std::fs::File::sync_data,
        );
        assert!(outcome.sync_error.is_none());
        let failed = outcome.failed_records;
        let disk = open();
        assert_eq!(
            disk.raw_chunk(0, 0).unwrap().tag.get("xPos"),
            Some(&Tag::Int(11))
        );
        assert_eq!(
            disk.raw_chunk(2, 0).unwrap().tag.get("xPos"),
            Some(&Tag::Int(33))
        );
        assert_eq!(
            disk.raw_chunk(32, 0).unwrap().tag.get("xPos"),
            Some(&Tag::Int(22))
        );
        assert_eq!(disk.raw_chunk(1, 0), Some(old));
        assert_eq!(failed.keys().copied().collect::<Vec<_>>(), [(1, 0)]);
        let cause = failed[&(1, 0)].get_ref().unwrap();
        assert!(cause.is::<sunrise_world::region::CompressedChunkLimit>());
    }
}

fn sample_tag(i: i32) -> NamedTag {
    let payload: Vec<i64> = (0..((i * 131) % 1500).abs())
        .map(|v| (v as i64) * (i as i64 + 1))
        .collect();
    NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("xPos", Tag::Int(i)),
            ("zPos", Tag::Int(-i)),
            ("Data", Tag::LongArray(payload)),
        ]),
    }
}

#[test]
fn empty_and_single_chunk_compression() {
    assert!(compress_records(&[], 100).is_empty());

    let tag = Arc::new(sample_tag(42));
    let records = [((42, -42), Record::Tag(Arc::clone(&tag)))];
    let refs: Vec<_> = records.iter().map(|(k, r)| (*k, r)).collect();
    let compressed = compress_records(&refs, 555);
    assert_eq!(compressed.len(), 1);
    let record = &compressed[0].as_ref().unwrap().as_ref().unwrap().record;
    assert_eq!(record.timestamp, 555);
    let loaded = record.nbt(Limits::saved()).unwrap();
    assert_eq!(loaded, *tag);
}

#[test]
fn parallel_compression_matches_single_threaded_fidelity() {
    let records: Vec<((i32, i32), Record)> = (0..64i32)
        .map(|i| {
            let record = if i % 5 == 2 {
                Record::Delete
            } else {
                Record::Tag(Arc::new(sample_tag(i)))
            };
            ((i, -i), record)
        })
        .collect();
    let refs: Vec<_> = records.iter().map(|(k, r)| (*k, r)).collect();
    let pool_compressed = compress_records(&refs, 9999);
    assert_eq!(pool_compressed.len(), records.len());

    let single_pool = CompressionPool::new(0);
    let single_compressed = compress_records_with_pool(&refs, 9999, &single_pool);

    for (i, (got_pool, got_single)) in pool_compressed
        .into_iter()
        .zip(single_compressed)
        .enumerate()
    {
        let rec_pool = got_pool.unwrap().map(|r| r.record);
        let rec_single = got_single.unwrap().map(|r| r.record);
        match (rec_pool, rec_single) {
            (None, None) => assert_eq!(i % 5, 2),
            (Some(p), Some(s)) => {
                assert_eq!(p.compression, s.compression);
                assert_eq!(p.timestamp, s.timestamp);
                assert_eq!(p.compressed, s.compressed);
                let tag_p = p.nbt(Limits::saved()).unwrap();
                let tag_s = s.nbt(Limits::saved()).unwrap();
                assert_eq!(tag_p, tag_s);
            }
            _ => panic!("mismatched option variant at index {i}"),
        }
    }
}

#[test]
fn custom_worker_pool_sizes_all_produce_identical_output() {
    let records: Vec<((i32, i32), Record)> = (0..27i32)
        .map(|i| ((i, i * 2), Record::Tag(Arc::new(sample_tag(i)))))
        .collect();
    let refs: Vec<_> = records.iter().map(|(k, r)| (*k, r)).collect();

    let pool_1 = CompressionPool::new(1);
    let pool_2 = CompressionPool::new(2);
    let pool_4 = CompressionPool::new(4);

    let res_1 = compress_records_with_pool(&refs, 12345, &pool_1);
    let res_2 = compress_records_with_pool(&refs, 12345, &pool_2);
    let res_4 = compress_records_with_pool(&refs, 12345, &pool_4);

    for idx in 0..records.len() {
        let r1 = &res_1[idx].as_ref().unwrap().as_ref().unwrap().record;
        let r2 = &res_2[idx].as_ref().unwrap().as_ref().unwrap().record;
        let r4 = &res_4[idx].as_ref().unwrap().as_ref().unwrap().record;
        assert_eq!(r1.compressed, r2.compressed);
        assert_eq!(r1.compressed, r4.compressed);
        assert_eq!(r1.timestamp, r2.timestamp);
        assert_eq!(r1.timestamp, r4.timestamp);
    }
}

#[test]
fn compressed_chunks_written_to_region_file_reload_with_full_integrity() {
    let dir = std::env::temp_dir().join(format!("sunrise-chunk-save-pool-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let region_path = dir.join("r.0.0.mca");
    let mut region = RegionFile::open_writable(&region_path, true).unwrap();

    let count = 32i32;
    let records: Vec<((i32, i32), Record)> = (0..count)
        .map(|i| ((i, i), Record::Tag(Arc::new(sample_tag(i)))))
        .collect();
    let refs: Vec<_> = records.iter().map(|(k, r)| (*k, r)).collect();

    let compressed = compress_records(&refs, 77777);
    assert_eq!(compressed.len(), count as usize);

    for (idx, ((x, z), _)) in records.iter().enumerate() {
        let record = &compressed[idx].as_ref().unwrap().as_ref().unwrap().record;
        region.write(ChunkPos::new(*x, *z), record).unwrap();
    }
    region.sync().unwrap();
    drop(region);

    let mut reopened = RegionFile::open(&region_path).unwrap();
    for ((x, z), record) in &records {
        let original_tag = record.tag().unwrap();
        let read_record = reopened
            .read(ChunkPos::new(*x, *z))
            .unwrap()
            .expect("chunk must exist");
        assert_eq!(read_record.timestamp, 77777);
        let read_tag = read_record.nbt(Limits::saved()).unwrap();
        assert_eq!(read_tag, *original_tag);
    }

    let _ = std::fs::remove_dir_all(&dir);
}
