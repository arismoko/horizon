#[cfg(test)]
pub(in crate::game) mod layer_color_tests {
    use crate::game::client_side::armor_stands_client::{layer_color, preview_slot};
    use sunrise_items::items::ItemStack;
    #[test]
    fn the_smithing_preview_wears_armour_and_holds_the_rest_in_its_off_hand() {
        assert_eq!(preview_slot(Some("head")), 5);
        assert_eq!(preview_slot(Some("chest")), 4);
        assert_eq!(preview_slot(Some("legs")), 3);
        assert_eq!(preview_slot(Some("feet")), 2);
        // A netherite sword has no equippable slot; a saddle or a
        // main-hand item falls to the same default branch.
        for other in [
            None,
            Some("mainhand"),
            Some("offhand"),
            Some("body"),
            Some("saddle"),
        ] {
            assert_eq!(preview_slot(other), 1);
        }
    }

    #[test]
    fn a_dyed_stack_overrides_the_undyed_leather_colour() {
        let mut dyed = ItemStack::new("minecraft:leather_helmet", 1);
        dyed.data_mut().dyed_color = Some(0x1E_90FF);
        // Dyed blue, R and B swapped into the renderer's order, opaque.
        assert_eq!(layer_color(true, Some(0xA06540), Some(&dyed)), 0xFFFF_901E);
        // An undyed stack keeps the asset's brown.
        let plain = ItemStack::new("minecraft:leather_helmet", 1);
        assert_eq!(layer_color(true, Some(0xA06540), Some(&plain)), 0xFF40_65A0);
        // No dye and no undyed colour: white.
        assert_eq!(layer_color(false, None, Some(&plain)), 0xFFFF_FFFF);
        // A layer that isn't dyeable stays white even on a dyed stack.
        assert_eq!(layer_color(false, None, Some(&dyed)), 0xFFFF_FFFF);
    }
}

#[cfg(test)]
pub(in crate::game) mod setup_anim_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub(in crate::game) static HEARD: AtomicUsize = AtomicUsize::new(0);
    use sunrise_gameplay_common::game::armor_stands_common::TYPE_ID;

    pub(in crate::game) struct Listener;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listener {
        fn humanoid_setup_anim(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut mod_api::HumanoidSetupAnimEvent<'_>,
        ) {
            use mod_api::HumanoidPart;
            assert_eq!(event.entity_id, 73);
            assert_eq!(event.entity_type, TYPE_ID);
            assert_eq!(event.partial_tick, 0.5);
            assert_eq!(
                event.parts.get(HumanoidPart::Head).unwrap()[1],
                [30.0_f32.to_radians(), 0.0, 0.0]
            );
            let right = event.parts.get(HumanoidPart::RightArm).unwrap()[1];
            assert!((right[0] + std::f32::consts::PI / 5.0).abs() < 1e-6);
            assert_eq!(
                event.parts.get(HumanoidPart::RightLeg).unwrap()[1],
                [-1.413_716_7, std::f32::consts::PI / 10.0, 0.078_539_82]
            );
            assert!(!event.humanoid.is_using_item);
            for part in [
                HumanoidPart::Head,
                HumanoidPart::Body,
                HumanoidPart::LeftArm,
                HumanoidPart::RightArm,
                HumanoidPart::LeftLeg,
                HumanoidPart::RightLeg,
            ] {
                assert!(event.parts.set_rotation(part, [1.0, 2.0, 3.0]));
                assert!(event.parts.set_position(part, [7.0, 8.0, 9.0]));
            }
            HEARD.fetch_add(1, Ordering::Relaxed);
        }
    }
}
