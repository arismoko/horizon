#[test]
#[ignore = "daily"]
fn held_durability_retains_the_stack_hand_and_explicit_break_slot() {
    use sunrise_core::equipment_slot::EquipmentSlot;
    use sunrise_core::geometry::Vec3;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Hand, sunrise_gameplay_common::game::Options,
        sunrise_gameplay_common::game::Progress,
    };
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.0, 70.0, 0.0)).unwrap());
    game.set_mode_id(0);
    // Stage an acting guest: break events are queued instead of rendering local crumbs.
    game.server.lan_acting = Some(1);
    game.hand = Hand::Off;
    let main = ItemStack::new("minecraft:stick", 3);
    game.common.inventory[game.common.selected] = Some(main.clone());
    let max = game
        .items()
        .get("minecraft:flint_and_steel")
        .unwrap()
        .max_damage;
    game.offhand = Some(ItemStack::new("minecraft:flint_and_steel", 1));
    game.damage_held_in_slot(1, EquipmentSlot::OffHand);
    assert_eq!(game.offhand.as_ref().unwrap().damage, 1);
    assert!(game.server.guest_equipment_breaks.is_empty());
    for (explicit, event) in [(true, 48), (false, 47)] {
        let mut tool = ItemStack::new("minecraft:flint_and_steel", 1);
        tool.damage = max - 1;
        game.offhand = Some(tool);
        if explicit {
            game.damage_held_in_slot(1, EquipmentSlot::OffHand);
        } else {
            game.damage_held(1);
        }
        assert!(game.offhand.is_none());
        assert_eq!(game.inventory[game.selected], Some(main.clone()));
        assert_eq!(game.server.guest_equipment_breaks.last().unwrap(), &event);
    }
    game.set_mode_id(1);
    game.offhand = Some(ItemStack::new("minecraft:flint_and_steel", 1));
    game.damage_held_in_slot(1, EquipmentSlot::OffHand);
    assert_eq!(game.offhand.as_ref().unwrap().damage, 0);
}
#[cfg(test)]
use sunrise_gameplay_common::game::tables::GameItems as _;
