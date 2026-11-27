#![cfg(test)]
use mod_api::{
    Billboard, DisplayKind, DisplayRef, EntityRef, ItemDisplayContext, TextAlignment,
    Transformation,
};
use std::sync::Arc;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_items::items::ItemStack;

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use mod_api::WorldEpoch;
    use sunrise_core::{geometry::Vec3, nbt::Tag};
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_gameplay_common::game::commands_common::OtherEntity;
    use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;

    fn push(game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32, kind: &str) -> DisplayRef {
        game.options.dedicated = Some(Vec::new());
        let display = game
            .read_display_nbt(kind, &Tag::Compound(Default::default()))
            .map(Arc::new);
        game.server.others.0.push(OtherEntity {
            size_answer: None,
            attachment_lifetime: Default::default(),
            knot_check: None,
            eye: None,
            join: Default::default(),
            display,
            interaction: None,
            id,
            kind: kind.into(),
            position: Vec3::new(1.0, 70.0, 1.0),
            yaw: 0.0,
            pitch: 0.0,
        });
        let entity = game.entity(id).unwrap();
        game.display(entity).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn displays_narrow_by_kind_and_share_the_display_state() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let text = push(game, 21, "minecraft:text_display");
            let block = push(game, 22, "minecraft:block_display");
            let item = push(game, 23, "minecraft:item_display");
            assert_eq!(text.kind(game), Some(DisplayKind::Text));
            assert_eq!(block.kind(game), Some(DisplayKind::Block));
            assert_eq!(item.kind(game), Some(DisplayKind::Item));
            assert_eq!(text.entity(), game.entity(21).unwrap());
            game.server.others.0.push(OtherEntity {
                size_answer: None,
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                display: None,
                interaction: None,
                id: 24,
                kind: "minecraft:marker".into(),
                position: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
            });
            assert!(game.display(game.entity(24).unwrap()).is_none());

            assert_eq!(text.transformation(game), Some(Transformation::IDENTITY));
            let moved = Transformation {
                translation: [0.0, 1.5, 0.0],
                left_rotation: [0.0, 0.000_000_1, 0.0, 0.0],
                scale: [2.0, 2.0, 2.0],
                right_rotation: [0.0; 4],
            };
            // `Display.setTransformation` stores the parts as-is, not via the NBT codec.
            assert!(text.set_transformation(game, moved));
            assert_eq!(text.transformation(game), Some(moved));

            assert_eq!(item.billboard(game), Some(Billboard::Fixed));
            assert!(item.set_billboard(game, Billboard::Center));
            assert_eq!(item.billboard(game), Some(Billboard::Center));
            assert_eq!(block.brightness(game), None);
            assert!(block.set_brightness(game, Some((15, 3))));
            assert_eq!(block.brightness(game), Some((15, 3)));
            assert!(!block.set_brightness(game, Some((16, 0))));
            assert!(block.set_brightness(game, None));
            assert_eq!(block.brightness(game), None);
            assert_eq!(block.view_range(game), Some(1.0));
            assert!(block.set_view_range(game, 4.0));
            assert_eq!(block.view_range(game), Some(4.0));
            assert!(block.set_view_range(game, f32::INFINITY));
            assert_eq!(block.view_range(game), Some(f32::INFINITY));
            assert!(block.set_shadow_radius(game, 0.5));
            assert!(block.set_shadow_strength(game, 0.25));
            assert!(block.set_width(game, 2.0));
            assert!(block.set_height(game, 3.0));
            assert!(block.set_glow_color_override(game, 0xff00ff));
            assert_eq!(
                (
                    block.shadow_radius(game),
                    block.shadow_strength(game),
                    block.width(game),
                    block.height(game),
                    block.glow_color_override(game)
                ),
                (Some(0.5), Some(0.25), Some(2.0), Some(3.0), Some(0xff00ff))
            );
            assert!(item.set_interpolation_duration(game, 8));
            assert!(item.set_start_interpolation(game, -1));
            assert!(item.set_teleport_duration(game, 99));
            assert_eq!(
                (
                    item.interpolation_duration(game),
                    item.start_interpolation(game),
                    item.teleport_duration(game)
                ),
                (Some(8), Some(-1), Some(59))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn display_kinds_keep_their_own_state_and_refuse_the_others() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let text = push(game, 21, "minecraft:text_display");
            let block = push(game, 22, "minecraft:block_display");
            let item = push(game, 23, "minecraft:item_display");
            assert_eq!(text.line_width(game), Some(200));
            assert_eq!(text.background(game), Some(0x4000_0000));
            assert_eq!(text.text_opacity(game), Some(-1));
            assert_eq!(text.alignment(game), Some(TextAlignment::Center));
            assert!(
                text.set_text(game, mod_api::Text::literal("hello"))
                    .unwrap()
            );
            assert!(text.set_line_width(game, 80));
            assert!(text.set_background(game, 0x20ff0000));
            assert!(text.set_text_opacity(game, 100));
            assert!(text.set_text_shadow(game, true));
            assert!(text.set_see_through(game, true));
            assert!(text.set_default_background(game, true));
            assert!(text.set_alignment(game, TextAlignment::Right));
            assert_eq!(
                text.text(game).unwrap().unwrap(),
                mod_api::Text::literal("hello")
            );
            assert_eq!(
                (
                    text.line_width(game),
                    text.background(game),
                    text.text_opacity(game),
                    text.text_shadow(game),
                    text.see_through(game),
                    text.default_background(game),
                    text.alignment(game)
                ),
                (
                    Some(80),
                    Some(0x20ff0000),
                    Some(100),
                    Some(true),
                    Some(true),
                    Some(true),
                    Some(TextAlignment::Right)
                )
            );
            assert!(text.set_alignment(game, TextAlignment::Left));
            assert!(text.set_text_shadow(game, false));
            assert_eq!(text.alignment(game), Some(TextAlignment::Left));
            assert_eq!(text.text_shadow(game), Some(false));
            assert_eq!(text.see_through(game), Some(true));

            let stone = game
                .block_state_id("minecraft:stone")
                .unwrap()
                .try_into_api()
                .unwrap();
            let stairs = game
                .block_state_id("minecraft:oak_stairs[facing=east]")
                .unwrap()
                .try_into_api()
                .unwrap();
            let air = game
                .block_state_id("minecraft:air")
                .unwrap()
                .try_into_api()
                .unwrap();
            assert_eq!(block.block_state(game), Some(air));
            assert!(block.set_block_state(game, stone));
            assert_eq!(block.block_state(game), Some(stone));
            assert!(block.set_block_state(game, stairs));
            assert_eq!(block.block_state(game), Some(stairs));
            let invalid = mod_api::__private::block_state_from_ordinal(stairs.block(), u16::MAX);
            assert!(!block.set_block_state(game, invalid));
            assert_eq!(block.block_state(game), Some(stairs));

            let sword = ItemStack {
                item: "minecraft:diamond_sword".into(),
                count: 1,
                damage: 3,
                extra: None,
            };
            assert_eq!(
                item.with_item(game, |_| unreachable!("empty display")),
                None::<()>
            );
            assert_eq!(item.item_context(game), Some(ItemDisplayContext::None));
            let sword = sword.try_into_api().unwrap();
            assert_eq!(item.set_item(game, Some(sword.clone())), Ok(true));
            assert!(item.set_item_context(game, ItemDisplayContext::Head));
            assert_eq!(
                item.with_item(game, |stack| (stack.count(), stack.damage())),
                Some((1, 3))
            );
            assert_eq!(
                item.with_item(game, |stack| stack.project()).flatten(),
                Some(sword)
            );
            assert_eq!(item.item_context(game), Some(ItemDisplayContext::Head));
            assert_eq!(item.set_item(game, None), Ok(true));
            assert_eq!(
                item.with_item(game, |_| unreachable!("empty display")),
                None::<()>
            );

            // The wrong kind reads None and writes nothing.
            assert_eq!(block.text(game).unwrap(), None);
            assert!(!block.set_text(game, mod_api::Text::literal("x")).unwrap());
            assert!(!text.set_block_state(game, stone));
            assert_eq!(text.block_state(game), None);
            assert!(!block.set_item_context(game, ItemDisplayContext::Gui));
            assert!(!item.set_alignment(game, TextAlignment::Left));
            assert_eq!(item.line_width(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn display_writes_reach_entity_data_and_the_saved_entity() {
        use sunrise_game_data::scoreboard::{comp_text, tag_to_json};
        use sunrise_protocol::entity_data::{Value, serializer};

        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let text = push(game, 21, "minecraft:text_display");
            // No display fields in the raw record. An unrelated key still survives a save.
            game.persist
                .raw
                .insert(21, Tag::compound([("keep", Tag::Int(7))]));
            assert!(
                text.set_text(game, mod_api::Text::literal("saved"))
                    .unwrap()
            );
            assert!(text.set_billboard(game, Billboard::Vertical));
            assert!(text.set_view_range(game, 2.5));
            assert!(text.set_start_interpolation(game, 3));
            assert_eq!(
                text.text(game).unwrap(),
                Some(mod_api::Text::literal("saved"))
            );
            assert_eq!(text.view_range(game), Some(2.5));
            assert!(game.persist.raw[&21].get("text").is_none());
            let o = game.server.others.0.iter().find(|o| o.id == 21).unwrap();
            let mut metadata = Vec::new();
            o.display
                .as_ref()
                .unwrap()
                .append_metadata(None, false, |_| None, &mut metadata);
            assert!(metadata.contains(&(15, serializer::BYTE, Value::Byte(1))));
            assert!(metadata.contains(&(17, serializer::FLOAT, Value::Float(2.5))));
            assert!(metadata.contains(&(
                23,
                serializer::COMPONENT,
                Value::Text(Tag::String("saved".into())),
            )));
            let saved = game.other_to_nbt(o, [1, 2, 3, 4]);
            assert_eq!(saved.get("keep"), Some(&Tag::Int(7)));
            assert_eq!(
                saved.get("billboard"),
                Some(&Tag::String("vertical".into()))
            );
            assert_eq!(saved.get("view_range"), Some(&Tag::Float(2.5f32.to_bits())));
            assert!(saved.get("start_interpolation").is_none());
            assert_eq!(
                comp_text(&tag_to_json(saved.get("text").unwrap())).plain(),
                "saved"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn each_start_delay_write_forces_a_send_even_when_equal() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let text = push(game, 21, "minecraft:text_display");
            let other = push(game, 22, "minecraft:item_display");
            assert!(game.persist.forced_meta.is_empty());
            for _ in 0..2 {
                assert!(text.set_start_interpolation(game, 0));
                assert_eq!(text.start_interpolation(game), Some(0));
                assert_eq!(game.persist.forced_meta.get(&21), Some(&[1 << 8, 0, 0, 0]));
                // The tracking pass spends the force; the next write sets it again.
                game.spend_forced_meta();
            }
            // Other writes, and writes that fail, force nothing.
            assert!(other.set_view_range(game, 3.0));
            assert!(game.persist.forced_meta.is_empty());
            assert!(text.entity().discard(game));
            assert!(!text.set_start_interpolation(game, 0));
            assert!(game.persist.forced_meta.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn stale_removed_and_client_displays_do_nothing() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let text = push(game, 21, "minecraft:text_display");
            let stale = DisplayRef::new(EntityRef::new(
                WorldEpoch::new(text.entity().epoch().get() + 1),
                21,
            ));
            assert_eq!(stale.kind(game), None);
            assert_eq!(stale.view_range(game), None);
            assert!(!stale.set_view_range(game, 2.0));
            assert!(!stale.set_text(game, mod_api::Text::literal("x")).unwrap());
            assert!(text.entity().discard(game));
            assert_eq!(text.kind(game), None);
            assert!(!text.set_width(game, 1.0));
        });
        #[cfg(feature = "client")]
        {
            use super::super::native_entity_refs::tests::with_client_game;
            use sunrise_client_gameplay::game::ClientGame;
            use sunrise_protocol::{
                connection::Pipe,
                packets::play::{AddEntity, Clientbound},
            };

            // Implementing ServerHost on ClientGame would make this trait inference ambiguous.
            trait WithoutServerWrites<Marker> {
                fn check() {}
            }
            impl<T> WithoutServerWrites<()> for T {}
            impl<T: mod_api::ServerHost> WithoutServerWrites<u8> for T {}
            let _ = <ClientGame<'_> as WithoutServerWrites<_>>::check;

            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "DisplayHandleTest").unwrap();
                let remote = game.client.remote.as_mut().unwrap();
                remote.logged_in = true;
                let kind = remote
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id("minecraft:text_display")
                    .unwrap();
                let position = [1.0, 70.0, 1.0];
                game.handle_packet(Clientbound::AddEntity(AddEntity {
                    id: 21,
                    uuid: 21,
                    kind,
                    position,
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                }))
                .unwrap();
                let entity = game.entity(21).expect("the client tracks the display");
                assert_eq!(entity.position(game), Some(position));
                let text = DisplayRef::new(entity);
                assert_eq!(text.kind(game), None);
                assert_eq!(text.view_range(game), None);
                assert!(matches!(
                    text.text(game),
                    Err(mod_api::text::TextFormError::Unavailable)
                ));
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn text_display_preserves_keybind_component_across_load_read_save_and_pairing() {
        use mod_api::text::{TextContent, TextHover};
        use sunrise_protocol::{
            buf::Reader,
            entity_data::{Value, serializer},
            packets::play::{Clientbound, SetEntityData},
        };

        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let keybind = Tag::compound([
                ("keybind", Tag::String("key.jump".into())),
                (
                    "hover_event",
                    Tag::compound([
                        ("action", Tag::String("show_item".into())),
                        ("id", Tag::String("minecraft:diamond".into())),
                        (
                            "components",
                            Tag::compound([(
                                "minecraft:custom_data",
                                Tag::compound([("marker", Tag::Byte(2))]),
                            )]),
                        ),
                    ]),
                ),
            ]);
            let tag = Tag::compound([
                ("Pos", Tag::List(vec![Tag::Double(0.0f64.to_bits()); 3])),
                ("Rotation", Tag::List(vec![Tag::Float(0.0f32.to_bits()); 2])),
                ("text", keybind.clone()),
            ]);
            fixture_resident_column(&mut game.world, (0, 0));
            let id = game
                .load_other_transferred(tag, "minecraft:text_display", None)
                .unwrap();
            let display = game.display(game.entity(id).unwrap()).unwrap();
            let projected = display.text(game).unwrap().unwrap();
            assert_eq!(projected.content, TextContent::Keybind("key.jump".into()));
            assert!(matches!(
                &projected.style.hover,
                Some(TextHover::Item { id, .. }) if id == "minecraft:diamond"
            ));

            let entity = game.server.others.0.iter().find(|o| o.id == id).unwrap();
            let saved = game.other_to_nbt(entity, [1, 2, 3, 4]);
            assert_eq!(saved.get("text"), Some(&keybind));

            let mut metadata = Vec::new();
            entity
                .display
                .as_ref()
                .unwrap()
                .append_metadata(None, false, |_| None, &mut metadata);
            let packet = Clientbound::SetEntityData(SetEntityData {
                id: id as i32,
                values: metadata,
            })
            .encode();
            struct ItemNames<'a>(&'a sunrise_items::items::Items);
            impl sunrise_protocol::item::Names for ItemNames<'_> {
                fn name(&self, _: &str, _: i32) -> Option<String> {
                    None
                }
                fn contains_entry(&self, registry: &str, name: &str) -> bool {
                    registry == "minecraft:item" && self.0.get(name).is_some()
                }
            }
            let names = ItemNames(game.res.items());
            let mut reader = Reader::new(&packet).with_names(&names);
            let packet_id = reader.varint().unwrap();
            let Clientbound::SetEntityData(decoded) =
                Clientbound::decode_reader(packet_id, reader).unwrap()
            else {
                panic!("expected display metadata packet");
            };
            assert!(
                decoded
                    .values
                    .contains(&(23, serializer::COMPONENT, Value::Text(keybind)))
            );

            let sprint = mod_api::Text::new(TextContent::Keybind("key.sprint".into()));
            assert!(display.set_text(game, sprint.clone()).unwrap());
            assert_eq!(display.text(game).unwrap(), Some(sprint));
            let entity = game.server.others.0.iter().find(|o| o.id == id).unwrap();
            let saved = game.other_to_nbt(entity, [1, 2, 3, 4]);
            assert_eq!(
                saved.get("text"),
                Some(&Tag::compound([(
                    "keybind",
                    Tag::String("key.sprint".into()),
                )]))
            );
        });
    }
}
