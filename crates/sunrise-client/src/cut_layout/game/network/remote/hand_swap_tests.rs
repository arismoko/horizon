use sunrise_core::geometry::Vec3;
use sunrise_items::equipment::{MAINHAND, OFFHAND};
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::play::{Clientbound as C, EntityEvent};
use sunrise_sim::mobs::Mob;
use {
    crate::game::load_resources,
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    crate::settings::Settings, sunrise_client_lib::audio::Audio,
    sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    sunrise_gameplay_common::game::execution_role::ExecutionRole,
};

/// `LivingEntity.handleEntityEvent(55)` is `swapHandItems`: the whole
/// stacks trade hands, not just their ids.
#[test]
#[ignore = "daily"]
fn the_hand_swap_event_keeps_whole_stacks() {
    with_client_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.damage = 40;
        sword.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("Biter".into()));
        let mut shield = ItemStack::new("minecraft:shield", 1);
        shield.damage = 12;
        let mut mob = Mob::new(31, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        mob.extra.set_slot_stack(MAINHAND, Some(sword.clone()));
        mob.extra.set_slot_stack(OFFHAND, Some(shield.clone()));
        game.mobs.push(mob);
        let swap = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
            game.handle_packet(C::EntityEvent(EntityEvent { id: 31, event: 55 }))
                .unwrap();
        };
        swap(game);
        let m = game.mobs.iter().find(|m| m.id == 31).unwrap();
        assert_eq!(m.extra.slot_stack(MAINHAND), Some(&shield));
        assert_eq!(m.extra.slot_stack(OFFHAND), Some(&sword));
        swap(game);
        let m = game.mobs.iter().find(|m| m.id == 31).unwrap();
        assert_eq!(m.extra.slot_stack(MAINHAND), Some(&sword));
        assert_eq!(m.extra.slot_stack(OFFHAND), Some(&shield));
    });
}

#[test]
#[ignore = "daily"]
fn joining_a_server_empties_the_off_hand_too() {
    // ClientPacketListener.handleLogin makes a new LocalPlayer, whose whole Inventory is
    // empty: a server that never sends slot 45 must not leave the last server's item.
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        &loaded.resources,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();
    let (first, _first_server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(first, "HopTest").unwrap();
    let stack = || Some(ItemStack::new("minecraft:shield", 1));
    game.inventory[0] = stack();
    game.armor[0] = stack();
    game.offhand = stack();
    game.equipment.set(sunrise_items::equipment::BODY, stack());
    game.equipment
        .set(sunrise_items::equipment::SADDLE, stack());
    let (second, _second_server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(second, "HopTest").unwrap();
    assert!(game.inventory[0].is_none() && game.armor[0].is_none());
    assert!(game.offhand.is_none());
    assert!(game.equipment.get(sunrise_items::equipment::BODY).is_none());
    assert!(
        game.equipment
            .get(sunrise_items::equipment::SADDLE)
            .is_none()
    );

    // Replacing the remote LocalPlayer (handleRespawn / handleLogin) also resets extra equipment.
    game.equipment.set(sunrise_items::equipment::BODY, stack());
    game.equipment
        .set(sunrise_items::equipment::SADDLE, stack());
    game.drop_remote_player();
    assert!(game.equipment.get(sunrise_items::equipment::BODY).is_none());
    assert!(
        game.equipment
            .get(sunrise_items::equipment::SADDLE)
            .is_none()
    );
}
