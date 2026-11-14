use super::*;

/// The guest's `fishing_medium`: it checks what crossed and answers a medium from it, with a
/// table only on the server.
pub(super) fn answer(
    host: mod_api::HostSide<'_>,
    event: &mut mod_api::FishingMediumEvent,
) -> Option<mod_api::FishingMedium> {
    assert_eq!(
        mod_api::__private::dimension_id(event.level.dimension()),
        Some("minecraft:overworld")
    );
    assert_eq!(event.bobber.id(), 41);
    assert_eq!(event.position, [5.5, 59.25, -0.5]);
    let server = matches!(host, mod_api::HostSide::Server(_));
    assert_eq!(event.owner.is_some(), server);
    if let Some(owner) = event.owner {
        assert_eq!((owner.entity().id(), owner.uuid()), (20, 7));
    }
    let feet = event.owner_position?;
    Some(mod_api::FishingMedium {
        surface: feet[1] - 4.0,
        loot_table: server.then(|| "test:gameplay/void".to_owned()),
        lure_ticks: Some([20, i32::MAX]),
    })
}

/// A wasm listener of `fishing_medium` reads the level, the bobber, its position, the caster
/// and where they stand on either side, and its `FishingMedium` comes back whole; no
/// `owner_position` is no answer.
#[test]
fn wire_loopback_fishing_medium_round_trips_the_answer() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    impl mod_api::ClientHost for World {}
    mod_api::impl_client_host_areas!(World);
    fn side(server: bool, world: &mut World) -> mod_api::HostSide<'_> {
        if server {
            mod_api::HostSide::Server(world)
        } else {
            mod_api::HostSide::Client(world)
        }
    }
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let epoch = mod_api::WorldEpoch::new(1);
    let level = mod_api::LevelRef::new(
        epoch,
        mod_api::__private::dimension_key("minecraft:overworld"),
    );
    let caster = mod_api::__private::game::player_ref(mod_api::EntityRef::new(epoch, 20), 7);
    for server in [true, false] {
        let mut event = mod_api::FishingMediumEvent {
            level,
            bobber: mod_api::EntityRef::new(epoch, 41),
            position: [5.5, 59.25, -0.5],
            owner: server.then_some(caster),
            owner_position: Some([0.5, 64.0, 0.5]),
        };
        assert_eq!(
            module.fishing_medium(side(server, &mut World), &mut event),
            Some(mod_api::FishingMedium {
                surface: 60.0,
                loot_table: server.then(|| "test:gameplay/void".to_owned()),
                lure_ticks: Some([20, i32::MAX]),
            })
        );
        event.owner_position = None;
        assert_eq!(
            module.fishing_medium(side(server, &mut World), &mut event),
            None
        );
    }
}
