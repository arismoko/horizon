use super::*;
use sunrise_gameplay_common::game::remote::keep_client_state;

#[test]
fn a_beacon_update_tag_keeps_the_beam_the_client_walked() {
    let old = BlockEntity::Beacon(Box::new(Beacon {
        levels: 1,
        primary: Some("minecraft:speed".into()),
        sections: vec![(0xF9FFFE, 2)],
        checking: vec![(0xF9FFFE, 1)],
        last_check_y: 70,
        ..Default::default()
    }));
    // BeaconBlockEntity.getUpdateTag: saveCustomOnly.
    let tag = Tag::compound([
        ("id", Tag::String("minecraft:beacon".into())),
        ("x", Tag::Int(0)),
        ("y", Tag::Int(64)),
        ("z", Tag::Int(0)),
        ("primary_effect", Tag::String("minecraft:haste".into())),
        (
            "secondary_effect",
            Tag::String("minecraft:regeneration".into()),
        ),
        ("Levels", Tag::Int(4)),
    ]);
    let (_, mut new) = sunrise_sim::save::decode_block_entity(&tag).unwrap();
    keep_client_state(Some(&old), &mut new);
    let BlockEntity::Beacon(b) = new else {
        panic!("not a beacon");
    };
    assert_eq!(b.sections, vec![(0xF9FFFE, 2)]);
    assert_eq!(b.checking, vec![(0xF9FFFE, 1)]);
    assert_eq!(b.last_check_y, 70);
    // The powers are the tag's; the levels stay the client's own (not read).
    assert_eq!(b.primary.as_deref(), Some("minecraft:haste"));
    assert_eq!(b.secondary.as_deref(), Some("minecraft:regeneration"));
    assert_eq!(b.levels, 1);
}
