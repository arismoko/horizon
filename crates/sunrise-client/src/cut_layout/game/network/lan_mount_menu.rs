#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use sunrise_gameplay_common::game::tables::WorldTablesExt;
    #[test]
    #[ignore = "daily"]
    fn horse_inventory_click_keeps_complete_body_stack() {
        use sunrise_core::geometry::Vec3;
        use sunrise_items::items::ItemStack;
        use sunrise_sim::{menu::Input, mobs::Mob};
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            let mut mob = Mob::new(12, kind, Vec3::ZERO, 0.0);
            mob.extra.tame = true;
            let mut armor = ItemStack::new("minecraft:iron_horse_armor", 3);
            armor.damage = 7;
            armor
                .data_mut()
                .enchantments
                .push(("minecraft:unbreaking".into(), 2));
            mob.extra.set_body_stack(Some(armor.clone()));
            game.mobs.push(mob);
            let mut menu = game.mount_engine_menu(0, 0);
            assert_eq!(menu.slot(1), Some(&armor));
            let env = game.menu_env();
            let _ = menu.click(&env, 2, 0, Input::Pickup);
            game.lan_mount_store(0, 0, &menu);
            assert_eq!(game.mobs[0].equipment_stacks()[6], Some(armor));
        });
    }
}
