//! A separate test binary owns this allocator. The unique tag isolates the
//! retained values from the harness and the allocations made by census() itself.
use horizon_core::nbt::{JavaString, Tag};
use horizon_protocol::retained::OwnedHeap;
use horizon_threads::{CensusAllocator, heap_census};
use std::collections::BTreeMap;

#[global_allocator]
static ALLOCATOR: CensusAllocator = CensusAllocator;
const TAG: &str = "protocol retained map census";

fn live_bytes() -> usize {
    let bytes = heap_census::census()
        .into_iter()
        .find_map(|(name, bytes)| (name == TAG).then_some(bytes))
        .unwrap_or(0);
    usize::try_from(bytes).expect("tag's live allocation count must not be negative")
}

fn assert_covers_live_layout<T: OwnedHeap>(case: &str, build: impl FnOnce() -> T) {
    assert_covers_charge(case, build, OwnedHeap::owned_heap_bytes);
}

fn assert_covers_charge<T>(
    case: &str,
    build: impl FnOnce() -> T,
    charge: impl FnOnce(&T) -> usize,
) {
    assert_charge(case, build, charge, None);
}

fn assert_charge<T>(
    case: &str,
    build: impl FnOnce() -> T,
    charge: impl FnOnce(&T) -> usize,
    expected_live: Option<usize>,
) {
    assert_eq!(live_bytes(), 0, "previous case leaked tagged storage");
    let value = {
        // Direct tag() is unconditional: the alloc-census feature is unnecessary
        // when explicitly installing CensusAllocator instead of the game's allocator.
        let _tag = heap_census::tag(TAG);
        build()
    };
    // The tag guard is gone before census() allocates its own result vector.
    // CensusAllocator records Layout::size(), excluding its own header and the
    // backing allocator's metadata/rounding, exactly the accounting boundary here.
    let observed = live_bytes();
    let charged = charge(&value);
    if let Some(expected) = expected_live {
        assert_eq!(observed, expected, "{case}: live retained storage");
        assert_eq!(charged, expected, "{case}: exact empty-storage charge");
    } else {
        assert!(
            observed > 0,
            "{case}: census must observe real retained storage"
        );
        assert!(
            charged >= observed,
            "{case}: charge {charged} undercounts {observed} live requested bytes"
        );
    }
    drop(value);
    assert_eq!(
        live_bytes(),
        0,
        "{case}: dropping the value must release every tagged allocation"
    );
}

#[test]
fn opaque_map_charges_cover_independently_observed_live_layouts() {
    assert_charge(
        "fresh empty NBT compound",
        horizon_core::nbt::Compound::new,
        OwnedHeap::owned_heap_bytes,
        Some(0),
    );
    assert_charge(
        "unused entry keeps the empty compound allocation-free",
        || {
            let mut map = horizon_core::nbt::Compound::new();
            drop(map.entry(JavaString::from("unused")));
            map
        },
        OwnedHeap::owned_heap_bytes,
        Some(0),
    );
    assert_covers_live_layout("populated NBT compound", || {
        (0..4096)
            .map(|i| (JavaString::from(i.to_string().as_str()), Tag::Int(i)))
            .collect::<horizon_core::nbt::Compound>()
    });
    assert_covers_live_layout("nested maps and spare child capacities", || {
        let mut outer = horizon_core::nbt::Compound::new();
        for i in 0..64 {
            let mut inner = horizon_core::nbt::Compound::new();
            for j in 0..31 {
                let mut units = Vec::with_capacity(100);
                units.push(j);
                let mut bytes = Vec::with_capacity(1000);
                bytes.push(42);
                let mut children = Vec::with_capacity(20);
                children.push(Tag::String(JavaString(units)));
                children.push(Tag::ByteArray(bytes));
                inner.insert(
                    JavaString::from(j.to_string().as_str()),
                    Tag::List(children),
                );
            }
            outer.insert(
                JavaString::from(i.to_string().as_str()),
                Tag::Compound(inner),
            );
        }
        Tag::Compound(outer)
    });
    assert_covers_live_layout("insert/remove churn", || {
        let mut map = horizon_core::nbt::Compound::new();
        for i in (0..4096).rev() {
            map.insert(JavaString::from(i.to_string().as_str()), Tag::Int(i));
        }
        for i in 0..4096 {
            if i % 11 != 0 {
                map.remove(&JavaString::from(i.to_string().as_str()));
            }
        }
        map
    });
    assert_covers_live_layout("allocated then emptied NBT compound", || {
        let mut map = horizon_core::nbt::Compound::new();
        for i in 0..128 {
            map.insert(JavaString::from(i.to_string().as_str()), Tag::Int(i));
        }
        map.clear();
        map
    });
    assert_covers_live_layout("retained empty root", || {
        let mut map = BTreeMap::<u32, [u64; 32]>::new();
        for i in 0..1000 {
            map.insert(i, [0; 32]);
        }
        for i in 0..1000 {
            map.remove(&i);
        }
        map
    });

    use horizon_protocol::{
        packets::configuration::{RegistryData, RegistryEntry, RegistryTags, TagEntry, UpdateTags},
        registry::Dynamic,
        retained::fresh_hash_table_bytes,
    };
    use std::collections::HashSet;
    #[repr(align(128))]
    #[derive(Hash, PartialEq, Eq)]
    struct Aligned(u32);
    for requested in [1, 2, 3, 7, 14, 15, 28, 56, 1000] {
        assert_covers_charge(
            "fresh aligned set table",
            || HashSet::<Aligned>::with_capacity(requested),
            |set| fresh_hash_table_bytes::<Aligned>(set.capacity()),
        );
        // Small/ZST elements exercise hashbrown's minimum table sizes too.
        assert_covers_charge(
            "fresh zero-sized set table",
            || HashSet::<()>::with_capacity(requested),
            |set| fresh_hash_table_bytes::<()>(set.capacity()),
        );
    }
    assert_covers_charge(
        "fresh dynamic snapshot with replacement",
        || {
            let mut dynamic = Dynamic::default();
            for round in 0..2 {
                for i in 0..40 {
                    let mut entries = Vec::with_capacity(100);
                    let mut nbt = horizon_core::nbt::Compound::new();
                    nbt.insert(
                        JavaString::from("nested"),
                        Tag::ByteArray(Vec::with_capacity(3000)),
                    );
                    let mut name = String::with_capacity(400);
                    name.push_str("test:entry");
                    entries.push(RegistryEntry {
                        id: name,
                        data: Some(Tag::Compound(nbt)),
                    });
                    dynamic.add(RegistryData {
                        registry: format!("test:registry_{i}"),
                        entries,
                    });
                    let tags = (0..(20 + round))
                        .map(|j| {
                            let mut ids = Vec::with_capacity(500);
                            ids.push(j);
                            TagEntry {
                                name: format!("test:tag_{j}"),
                                ids,
                            }
                        })
                        .collect();
                    dynamic.add_tags(&UpdateTags {
                        registries: vec![RegistryTags {
                            registry: format!("test:registry_{i}"),
                            tags,
                        }],
                    });
                }
            }
            dynamic
        },
        Dynamic::fresh_owned_heap_bytes,
    );
}
