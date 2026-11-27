#[cfg(test)]
pub(crate) mod tests {

    use crate::game::client_side::item_use_client::use_pose;

    use sunrise_client::first_person::ItemPose;

    use sunrise_core::use_animation::UseAnimation;

    #[test]
    fn a_use_animation_picks_the_hand_pose() {
        let pose = |animation| use_pose(animation, 12.0, 32.0, 0.5);
        assert!(matches!(
            pose(UseAnimation::Drink),
            ItemPose::Eat { remaining, duration, partial }
                if (remaining, duration, partial) == (12.0, 32.0, 0.5)
        ));
        // ticks used: 32 - (12 - 0.5 + 1).
        assert!(
            matches!(pose(UseAnimation::Bow), ItemPose::Bow { ticks_used } if ticks_used == 19.5)
        );
        assert!(matches!(
            pose(UseAnimation::Trident),
            ItemPose::Trident { ticks_used } if ticks_used == 19.5
        ));
        assert!(matches!(
            pose(UseAnimation::Brush),
            ItemPose::Brush { remaining, partial } if (remaining, partial) == (12.0, 0.5)
        ));
        for animation in [
            UseAnimation::None,
            UseAnimation::Block,
            UseAnimation::Spyglass,
        ] {
            assert!(matches!(pose(animation), ItemPose::Using));
        }
    }
}
