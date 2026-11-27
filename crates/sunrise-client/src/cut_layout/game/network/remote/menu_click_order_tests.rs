use sunrise_gameplay_common::game::remote::inventory::open_hash_order;

/// The vanilla 26.3 client's ServerboundContainerClickPacket for a shift-click from hotbar slot 37
/// into slot 10 listed 37 first (wire stream harness, `inventory`). The key 0 always leads.
#[test]
fn changed_slots_go_out_in_the_order_vanilla_s_hash_map_iterates() {
    assert_eq!(open_hash_order(&[10, 37]), [37, 10]);
    assert_eq!(open_hash_order(&[0, 1, 2, 36, 44])[0], 0);
    // Past 24 keys the table grows; every key is still listed once.
    let all: Vec<i16> = (0..46).collect();
    let mut out = open_hash_order(&all);
    out.sort_unstable();
    assert_eq!(out, all);
}
