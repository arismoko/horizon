use super::*;
use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
use sunrise_sim::{datapack_net, lan::Out};

struct Synced;
#[mod_api::__subscriptions]
impl Hooks for Synced {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.declare_datapack_registry(
            "spell",
            DatapackRegistrySync::Required,
            vec![DatapackField::required(
                "power",
                DatapackFieldKind::Int { min: 0, max: 100 },
            )],
        )
        .unwrap();
    }
}
fn apply_packets(
    rx: &std::sync::mpsc::Receiver<Out>,
    datapacks: &mut datapack_net::Receiver,
) -> usize {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(reason) => panic!("unexpected refusal: {reason:?}"),
        })
        .filter_map(|body| {
            let mut r = Reader::new(&body);
            let id = r.varint().unwrap();
            match Clientbound::decode(id, r.rest()).unwrap() {
                Clientbound::CustomPayload(p) if p.channel == datapack_net::CHANNEL => {
                    datapacks.accept_chunk(&p.data).unwrap();
                    Some(())
                }
                _ => None,
            }
        })
        .count()
}

#[test]
#[ignore = "daily"]
fn datapack_game_reload_delivers_only_successful_generations_and_disconnects_refused_guests() {
    // Both peers use process-wide snapshots; another fixture must not replace either one.
    if run_fresh_cases(
        "cut_layout::game::server_side::server_reload::tests::datapack_publication_tests::datapack_game_delivery_tests::datapack_game_reload_delivers_only_successful_generations_and_disconnects_refused_guests",
        "SUNRISE_DATAPACK_RELOAD_DELIVERY_CHILD",
        &["delivery"],
    ) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let pack = root.join("datapacks/base");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    let entry = pack.join("data/registry_test/registry_test/spell/fire.json");
    write(&entry, r#"{"power":7}"#);
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/base"],"disabled":[]}"#,
    );
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(Synced))].into(),
        || {
            with_game(|game| {
                game.load_world(request(root)).unwrap();
                let offer = datapack_net::offer()
                    .unwrap()
                    .expect("synchronized registry offer required");
                let mut datapacks = datapack_net::Receiver::default();
                let datapack_net::Step::Send(bytes) = datapack_net::server_step(true, Some(&offer))
                else {
                    panic!("initial snapshot required");
                };
                for chunk in datapack_net::chunks(&bytes).unwrap() {
                    datapacks.accept_chunk(&chunk).unwrap();
                }
                let first = datapacks.received().unwrap();
                assert_eq!(
                    first
                        .get("registry_test:spell", "registry_test:fire")
                        .unwrap()
                        .get("power"),
                    Some(&DatapackValue::Int(7))
                );
                let rx = sunrise_gameplay::game::server_side::lan::world_spawn_sync_tests::host_for(
                    game,
                );
                game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.brand = "sunrise".into();
                    g.sender.datapack_offer = Some(offer.into());
                });
                write(&entry, r#"{"power":9}"#);
                assert!(game.run_command("reload"));
                assert!(
                    apply_packets(&rx, &mut datapacks) > 0,
                    "the coordinator must send its committed set"
                );
                let received = datapacks.received().unwrap();
                assert_eq!(
                    received
                        .get("registry_test:spell", "registry_test:fire")
                        .unwrap()
                        .get("power"),
                    Some(&DatapackValue::Int(9))
                );
                write(&entry, "{broken");
                assert!(!game.run_command("reload"));
                assert_eq!(
                    apply_packets(&rx, &mut datapacks),
                    0,
                    "failed preparation must send no replacement"
                );
                assert_eq!(
                    datapacks
                        .received()
                        .unwrap()
                        .get("registry_test:spell", "registry_test:fire")
                        .unwrap()
                        .get("power"),
                    Some(&DatapackValue::Int(9))
                );
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.sender.datapack_offer = Some(Arc::from([255_u8])));
                write(&entry, r#"{"power":11}"#);
                assert!(game.run_command("reload"));
                assert!(rx.try_iter().any(|out| matches!(out, Out::Disconnect(sunrise_core::nbt::Tag::String(reason)) if String::from_utf16_lossy(&reason.0).contains("registries are unreadable"))));
            });
        },
    );
}
