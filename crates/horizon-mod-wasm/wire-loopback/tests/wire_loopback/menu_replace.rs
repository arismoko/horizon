use super::*;

#[test]
fn menu_replace_round_trips_both_choices() {
    for (choice, ordinal) in [
        (mod_api::MenuReplace::CloseFirst, 0_u32),
        (mod_api::MenuReplace::InPlace, 1_u32),
    ] {
        let mut out = Writer::new();
        out.put(&choice);
        assert_eq!(out.bytes(), ordinal.to_le_bytes());
        assert_eq!(
            Reader::decode::<mod_api::MenuReplace>(out.bytes()).unwrap(),
            choice
        );
    }
}
