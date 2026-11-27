use super::*;
use mod_api::*;
use sunrise_protocol::{
    frame,
    packets::{configuration as cfg, play},
};

fn declaration() -> DatapackRegistryDeclaration {
    DatapackRegistryDeclaration {
        mod_id: "large".into(),
        key: "large:blob".into(),
        fields: vec![DatapackField::required(
            "text",
            DatapackFieldKind::Text { max_len: 4 << 20 },
        )],
        sync: DatapackRegistrySync::Required,
    }
}
fn snapshot(text: String) -> DatapackSnapshot {
    DatapackSnapshot {
        registries: vec![DatapackSnapshotRegistry {
            key: "large:blob".into(),
            fields: declaration().fields,
            tags: Default::default(),
            entries: vec![DatapackSnapshotEntry {
                id: "large:entry".into(),
                values: vec![DatapackValue::Text(text)],
            }],
        }],
    }
}

#[test]
#[ignore = "daily"]
fn incompressible_large_registry_snapshot_fits_each_frame_and_activates_only_when_complete() {
    struct Large;
    #[mod_api::__subscriptions]
    impl Hooks for Large {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let d = declaration();
            r.declare_datapack_registry("blob", d.sync, d.fields)
                .unwrap();
        }
    }
    let mods = vec![ModEntry::new("large", Box::new(Large))];
    mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
    mod_platform::testing::with_mods(mods.into(), || {
        let mut receiver = Receiver::default();
        for chunk in chunks(&snapshot("old".into()).encode().unwrap()).unwrap() {
            receiver.accept_chunk(&chunk).unwrap();
        }
        let mut seed = 0x1a2b3c4d5e6f_u64;
        let text: String = (0..(3 << 20))
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                char::from(b'!' + (seed & 63) as u8)
            })
            .collect();
        let bytes = snapshot(text.clone()).encode().unwrap();
        assert!(bytes.len() > frame::MAX_FRAME);
        assert!(
            frame::encode(&bytes, Some(256)).is_err(),
            "the unsplit incompressible snapshot must exceed the real frame limit"
        );
        let packets: Vec<_> = chunks(&bytes).unwrap().collect();
        assert!(packets.len() > 1);
        for (i, data) in packets.iter().enumerate() {
            assert!(
                data.len() <= 1 << 20,
                "vanilla unknown custom payload bound"
            );
            if i + 1 < packets.len() {
                assert_eq!(data.len(), 1 << 20, "the header is inside that bound");
            }
            let body = play::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: CHANNEL.into(),
                data: data.clone(),
            })
            .encode();
            assert!(body.len() < frame::MAX_UNCOMPRESSED);
            frame::encode(&body, None).unwrap();
            frame::encode(&body, Some(256)).unwrap();
            receiver.accept_chunk(data).unwrap();
            assert_eq!(receiver.finish_transfer().is_ok(), i + 1 == packets.len());
            let expected = if i + 1 == packets.len() {
                text.as_str()
            } else {
                "old"
            };
            let current = receiver.received().unwrap();
            let Some(DatapackValue::Text(actual)) = current
                .get("large:blob", "large:entry")
                .unwrap()
                .get("text")
            else {
                panic!("the accepted payload must be readable");
            };
            assert_eq!(actual, expected);
        }
        // A completed configuration event retains the decoded text, not just its inline receiver.
        assert!(receiver.owned_heap_bytes() >= text.len());
        let set = receiver.received().unwrap();
        assert_eq!(
            set.get("large:blob", "large:entry").unwrap().get("text"),
            Some(&DatapackValue::Text(text))
        );
        receiver.finish_transfer().unwrap();
    });
}

#[test]
fn registry_fragments_refuse_oversize_reordered_duplicate_and_truncated_transfers() {
    let data = vec![1; CHUNK_DATA + 3];
    let parts: Vec<_> = chunks(&data).unwrap().collect();
    let mut assembly = Assembly::default();
    assert!(assembly.push(&parts[1]).is_err());
    assert!(assembly.push(&parts[0]).unwrap().is_none());
    assert!(assembly.push(&parts[0]).is_err());
    assert!(assembly.pending.is_none());
    assert_eq!(parts[0].len(), 1 << 20);
    let mut at_limit = parts[0].clone();
    at_limit[4..8].copy_from_slice(&(128_u32 << 20).to_le_bytes());
    assert!(Assembly::default().push(&at_limit).unwrap().is_none());
    let mut oversized = parts[0][..HEADER + 1].to_vec();
    oversized[4..8].copy_from_slice(&((128_u32 << 20) + 1).to_le_bytes());
    assert!(
        assembly
            .push(&oversized)
            .unwrap_err()
            .contains("transport limit")
    );
    let mut overlong_fragment = parts[0].clone();
    overlong_fragment.push(0);
    assert!(assembly.push(&overlong_fragment).is_err());
    let mut truncated = parts[0].clone();
    truncated.pop();
    assert!(assembly.push(&truncated).is_err());
    assert!(assembly.pending.is_none());
    assert!(assembly.push(&parts[0]).unwrap().is_none());
    assert_eq!(assembly.push(&parts[1]).unwrap(), Some(data));
    assert!(
        assembly.push(&parts[1]).is_err(),
        "a completed transfer cannot be replayed from its tail"
    );
    let larger = vec![2; 2 * CHUNK_DATA + 3];
    let mut changed: Vec<_> = chunks(&larger).unwrap().collect();
    assert!(assembly.push(&changed[0]).unwrap().is_none());
    changed[1][4..8].copy_from_slice(&((larger.len() + 1) as u32).to_le_bytes());
    assert!(
        assembly.push(&changed[1]).is_err(),
        "the declared total cannot change mid-transfer"
    );
}
