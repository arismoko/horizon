use crate::game::client_side::villager_client_client::receive;
use std::sync::Arc;
use sunrise_client::mobs::render;
use sunrise_gameplay_common::game::lan_meta_fixture::profession_hub;
use sunrise_gameplay_common::game::villager_client::{
    Builtin, Mob, ResolvedProfessions, Value, server_meta,
};
use sunrise_sim::mobs::MobTypes;
fn table() -> ResolvedProfessions {
    ordered_table(&["test:engineer"])
}
fn ordered_table(names: &[&str]) -> ResolvedProfessions {
    let id = |name| mod_api::Identifier::parse(name).unwrap();
    ResolvedProfessions::prepare(
        names
            .iter()
            .map(|name| {
                (
                    "test".into(),
                    mod_api::ProfessionDefinition {
                        key: mod_api::ProfessionKey(id(name)),
                        workstations: vec![],
                        held_poi: vec![],
                        acquirable_poi: vec![],
                        requested_items: vec![],
                        secondary_blocks: vec![],
                        work_sound: None,
                        trades: vec![],
                    },
                )
            })
            .collect(),
        |_, _| true,
    )
    .unwrap()
}
fn roundtrip(kind: &str, custom: bool, texture: &str) {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources"),
    );
    let types = MobTypes::load(&root).unwrap();
    let kind = Arc::new(types.types.get(kind).unwrap().clone());
    let mut server = Mob::new(1, kind.clone(), sunrise_core::geometry::Vec3::ZERO, 0.0);
    let server_table = if custom {
        table()
    } else {
        ResolvedProfessions::vanilla()
    };
    let client_table = if custom {
        table()
    } else {
        ResolvedProfessions::vanilla()
    };
    let profession = if custom {
        "test:engineer"
    } else {
        "minecraft:farmer"
    };
    let slot = server_table
        .resolve(profession)
        .expect("registered profession");
    server.extra.profession = server_table.state(slot).unwrap();
    let hub = profession_hub("ProfessionClient", true);
    let (mut meta, mut defaults) = (Vec::new(), Vec::new());
    server_meta(
        &server,
        &hub,
        None,
        0,
        &mut meta,
        &mut defaults,
        &server_table,
    );
    let mut client = Mob::new(2, kind, sunrise_core::geometry::Vec3::ZERO, 0.0);
    encoded_receive(&mut client, meta, &client_table);
    render::villager_client::install(&client_table, &Default::default());
    assert_eq!(
        client.extra.profession.id(),
        if custom {
            "test:engineer"
        } else {
            "minecraft:farmer"
        }
    );
    assert!(
        render::layers(&client, 0.0)
            .iter()
            .any(|layer| layer.texture == texture)
    );
}
#[test]
#[ignore = "daily"]
fn engineer_metadata_and_villager_texture() {
    roundtrip(
        "minecraft:villager",
        true,
        "test:entity/villager/profession/engineer",
    );
}
#[test]
#[ignore = "daily"]
fn engineer_metadata_and_zombie_texture() {
    roundtrip(
        "minecraft:zombie_villager",
        true,
        "test:entity/zombie_villager/profession/engineer",
    );
}
#[test]
#[ignore = "daily"]
fn vanilla_farmer_metadata_and_texture() {
    roundtrip(
        "minecraft:villager",
        false,
        "entity/villager/profession/farmer",
    );
}

fn encoded_receive(client: &mut Mob, values: Vec<(u8, i32, Value)>, table: &ResolvedProfessions) {
    use sunrise_protocol::{
        buf::Reader,
        packets::play::{Clientbound, SetEntityData},
    };
    let bytes = Clientbound::SetEntityData(SetEntityData { id: 99, values }).encode();
    let mut reader = Reader::new(&bytes);
    let id = reader.varint().unwrap();
    let Clientbound::SetEntityData(packet) =
        Clientbound::decode(id, &bytes[bytes.len() - reader.remaining()..]).unwrap()
    else {
        panic!("entity metadata");
    };
    receive(
        client,
        &packet.values,
        &Builtin::default(),
        packet.id,
        table,
    );
}

#[test]
#[ignore = "daily"]
fn villager_client_packet_slots_and_unresolved_identity() {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let types = MobTypes::load(&root).unwrap();
    let server_table = ordered_table(&["test:surveyor", "test:engineer"]);
    let client_table = ordered_table(&["test:engineer", "test:surveyor"]);
    let hub = profession_hub("ProfessionPacket", true);
    for species in ["minecraft:villager", "minecraft:zombie_villager"] {
        let kind = Arc::new(types.types[species].clone());
        for (name, expected_slot, expected_id, layer) in [
            ("minecraft:weaponsmith", 14, "minecraft:weaponsmith", true),
            ("test:engineer", 15, "test:engineer", true),
            ("test:surveyor", 16, "test:surveyor", true),
            ("missing:retired", 0, "minecraft:none", false),
        ] {
            let mut server = Mob::new(1, kind.clone(), sunrise_core::geometry::Vec3::ZERO, 0.0);
            server.extra.profession = server_table
                .resolve(name)
                .and_then(|slot| server_table.state(slot))
                .unwrap_or_else(|| sunrise_sim::mobs::villager::ProfessionState::from_id(name));
            let mut client = Mob::new(2, kind.clone(), sunrise_core::geometry::Vec3::ZERO, 0.0);
            let (mut meta, mut defaults) = (Vec::new(), Vec::new());
            server_meta(
                &server,
                &hub,
                None,
                0,
                &mut meta,
                &mut defaults,
                &server_table,
            );
            let field = if species == "minecraft:villager" {
                19
            } else {
                20
            };
            assert!(meta.iter().any(|(index, _, value)| *index == field
                && matches!(value, Value::Ids(ids) if ids == &[0, expected_slot, 1])));
            encoded_receive(&mut client, meta, &client_table);
            assert_eq!(client.extra.profession.id(), expected_id);
            assert_eq!(
                client_table.state_index(&client.extra.profession),
                Some(expected_slot as usize)
            );
            match &client.extra.profession {
                sunrise_sim::mobs::villager::ProfessionState::Registered { slot, .. } => {
                    assert_eq!(*slot, expected_slot as u16);
                }
                state => assert_eq!(state.index(), Some(expected_slot as u8)),
            }
            assert_eq!(server.extra.profession.id(), name);
            let saved = sunrise_sim::mobs::villager::professions::save::villager_data(&server);
            assert_eq!(
                saved.get("profession"),
                Some(&sunrise_core::nbt::Tag::String(name.into()))
            );
            render::villager_client::install(&client_table, &Default::default());
            assert_eq!(
                render::layers(&client, 0.0)
                    .iter()
                    .any(|layer| layer.texture.contains("/profession/")),
                layer
            );
        }
    }
}
