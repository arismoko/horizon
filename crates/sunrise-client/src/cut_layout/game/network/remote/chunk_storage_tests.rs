#[test]
fn the_chunk_storage_takes_any_radius_and_centre_a_server_sends() {
    use sunrise_level::chunk_map::ClientChunkRange;
    let in_storage_range =
        |distance, center, x, z| ClientChunkRange::new(center, distance).contains((x, z));
    // Vanilla's storage: radius at least 2, plus 3, each way from the centre.
    assert!(in_storage_range(10, (0, 0), 13, -13));
    assert!(!in_storage_range(10, (0, 0), 14, 0));
    assert!(in_storage_range(-5, (100, 100), 105, 95));
    assert!(!in_storage_range(0, (100, 100), 106, 100));
    // Out of i32's range, as a server may send: no overflow, and the sums a wrap got wrong.
    assert!(in_storage_range(i32::MAX, (0, 0), i32::MAX, i32::MIN + 1));
    assert!(!in_storage_range(10, (i32::MIN, 0), i32::MAX, 0));
    assert!(in_storage_range(
        10,
        (i32::MIN, i32::MAX),
        i32::MIN + 13,
        i32::MAX - 13
    ));
}

#[test]
fn the_cache_indexes_negative_slots_on_both_axes() {
    use std::sync::Arc;
    use sunrise_level::{
        chunk::Chunk,
        chunk_map::{ChunkMap, ClientChunkRange},
    };

    let mut chunks = ChunkMap::default();
    let old = Arc::new(Chunk::empty(2, 3));
    chunks.insert((2, 3), old.clone());
    let (hidden, shown) = chunks.set_client_chunk_range(ClientChunkRange::new((-9, -8), 2));
    assert_eq!(hidden, [(2, 3)]);
    assert!(shown.is_empty());
    assert_eq!(chunks.client_chunk_alias((-9, -8)), Some((2, 3)));
    assert_eq!(chunks.client_chunk_alias((-9, -7)), None);
    assert!(chunks.remove(&(-9, -8)).is_none());
    assert!(Arc::ptr_eq(chunks.resident_get(&(2, 3)).unwrap(), &old));
}

/// A column a centre move hid comes back when the server keeps it past the view (`far`), and is
/// handed back to be drawn again; a later centre move leaves it visible.
#[test]
fn keeping_a_hidden_column_far_shows_it_again() {
    use std::sync::Arc;
    use sunrise_level::{
        chunk::Chunk,
        chunk_map::{ChunkMap, ClientChunkRange},
    };

    let mut chunks = ChunkMap::default();
    chunks.insert((2, 3), Arc::new(Chunk::empty(2, 3)));
    let (hidden, _) = chunks.set_client_chunk_range(ClientChunkRange::new((-9, -8), 2));
    assert_eq!(hidden, [(2, 3)]);
    assert!(!chunks.contains_key(&(2, 3)));
    assert_eq!(chunks.keep_far(&[(2, 3)]), [(2, 3)]);
    assert!(chunks.contains_key(&(2, 3)));
    let (hidden, _) = chunks.set_client_chunk_range(ClientChunkRange::new((-40, -8), 2));
    assert!(hidden.is_empty() && chunks.contains_key(&(2, 3)));
}
