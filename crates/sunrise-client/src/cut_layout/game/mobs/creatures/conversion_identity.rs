#![cfg(test)]
#[cfg(test)]
mod tests {
    use mod_api::Holder;
    use sunrise_core::{geometry::Vec3, nbt::Tag};
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_mod_runtime::attachments::{AttachmentCodec, AttachmentError, AttachmentType};
    use sunrise_sim::mobs::{Mob, conversion::Params};

    static MARK: AttachmentType<i32> = AttachmentType {
        namespace: "conversion_test",
        name: "mark",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
            decode: |tag| match tag.get(&"n".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: false,
        sync_codec: None,
    };
    static LOOSE: AttachmentType<i32> = AttachmentType {
        namespace: "conversion_test",
        name: "loose",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync_codec: None,
    };

    /// A villager struck into a witch keeps the attachment that has a codec and leaves the
    /// one without behind (`copyAttachmentsFrom(villager, false)`).
    #[test]
    #[ignore = "daily"]
    fn a_converted_mob_keeps_its_serializable_attachments() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            let villager = Mob::new(500, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            game.mobs.push(villager.clone());
            MARK.set(game, Holder::Entity(500), 7).unwrap();
            LOOSE.set(game, Holder::Entity(500), 3).unwrap();
            let params = Params {
                keep_equipment: false,
                preserve_can_pick_up_loot: false,
            };
            let witch = game
                .convert_to(&villager, "minecraft:witch", params, false, |_, _| {})
                .unwrap();
            assert_eq!(MARK.get_existing(game, Holder::Entity(witch)), Ok(Some(7)));
            assert_eq!(LOOSE.get_existing(game, Holder::Entity(witch)), Ok(None));
        });
    }
}
