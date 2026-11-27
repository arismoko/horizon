#[cfg(test)]
pub(crate) mod join_order_tests {

    use sunrise_gameplay_common::game::server_side::entity_server::join_order_tests::{Lists, at};

    use crate::game::client_side::entity_client::{DrawExtras, DrawFamily, DrawRun, draw_runs};

    use sunrise_gameplay_common::game::entity::EntityLists;

    use sunrise_gameplay_common::game::creatures::render_fixture::mob;

    use sunrise_sim::join_seq::{JoinList, JoinSeq};

    use sunrise_sim::mobs::Species;

    fn runs(
        lists: &Lists,
        local_player: Option<JoinSeq>,
    ) -> Vec<(DrawFamily, std::ops::Range<usize>)> {
        let mut runs = Vec::new();
        draw_runs(
            lists.view(),
            DrawExtras {
                local_player,
                ..Default::default()
            },
            &mut runs,
        );
        runs.into_iter().map(|r| (r.family, r.range)).collect()
    }
    /// The ordered draw takes the families' entities in join order, not family by family, and
    /// each run names the list range its family draws.
    #[test]
    fn draw_runs_interleave_families_in_join_order() {
        let mut lists = Lists::default();
        let cow = |id| mob(id, "minecraft:cow", Species::Cow, 0.9, 1.4);
        lists.mobs.push(cow(4));
        lists.mobs.push(cow(3));
        let local_player = JoinSeq::next();
        lists.mobs.push(cow(2));
        lists.trident(6);
        lists.rocket(8);
        lists.stand(5);
        lists.mobs.push(cow(1));
        lists.boat(7);
        lists.trident(9);
        lists.stand(11);
        lists.rocket(10);
        assert_eq!(
            runs(&lists, Some(local_player)),
            [
                (DrawFamily::Mob, 0..2),
                (DrawFamily::LocalPlayer, 0..1),
                (DrawFamily::Mob, 2..3),
                (DrawFamily::Trident, 0..1),
                (DrawFamily::Rocket, 0..1),
                (DrawFamily::ArmorStand, 0..1),
                (DrawFamily::Mob, 3..4),
                (DrawFamily::Boat, 0..1),
                (DrawFamily::Trident, 1..2),
                (DrawFamily::ArmorStand, 1..2),
                (DrawFamily::Rocket, 1..2),
            ]
        );
    }
    /// Entities of one family that joined back to back are one run.
    #[test]
    fn draw_runs_join_a_family_that_joined_back_to_back() {
        let mut lists = Lists::default();
        lists.stand(1);
        lists.stand(2);
        lists.boat(3);
        lists.stand(4);
        assert_eq!(
            runs(&lists, None),
            [
                (DrawFamily::ArmorStand, 0..2),
                (DrawFamily::Boat, 0..1),
                (DrawFamily::ArmorStand, 2..3),
            ]
        );
    }
    /// Entities of an undrawn family do not split a run.
    #[test]
    fn an_undrawn_family_splits_no_run() {
        use sunrise_sim::projectiles::{Projectile, ProjectileKind};
        let mut lists = Lists::default();
        lists.stand(1);
        let mut projectiles = JoinList::<Projectile, false>::default();
        projectiles.push(Projectile::new(ProjectileKind::DragonFireball, at(), None));
        lists.stand(2);
        let mut draw = Vec::new();
        draw_runs(
            EntityLists {
                projectiles: &projectiles,
                ..lists.view()
            },
            DrawExtras::default(),
            &mut draw,
        );
        assert_eq!(
            draw,
            [DrawRun {
                family: DrawFamily::ArmorStand,
                range: 0..2
            }]
        );
    }
}

#[cfg(test)]
pub(in crate::game) mod render_bounds_tests {
    use crate::game::client_side::entity_client::RenderEntityBounds;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::{
        boats_common::Boat,
        minecarts::{CartKind, Minecart},
    };
    use sunrise_sim::entities::ArmorStand;
    use sunrise_sim::entities::ItemFrame;

    #[test]
    fn armor_stand_bounds_include_raised_equipment_space() {
        let mut stand = ArmorStand::new(1, Vec3::new(4.0, 64.0, -2.0), 90.0);
        stand.show_arms = true;
        stand.pose[2] = [180.0, 0.0, 45.0];
        stand.equipment[0] = Some(sunrise_items::items::ItemStack {
            item: "minecraft:iron_sword".into(),
            count: 1,
            damage: 0,
            extra: None,
        });
        stand.equipment[5] = Some(sunrise_items::items::ItemStack {
            item: "minecraft:banner".into(),
            count: 1,
            damage: 0,
            extra: None,
        });
        let bounds = stand.render_bounds(0.5);
        assert!(bounds.min_position().x <= 2.0);
        assert!(bounds.max_position().x >= 6.0);
        assert!(bounds.min_position().y <= 63.0);
        assert!(bounds.max_position().y >= 69.0);
        assert!(bounds.min_position().z <= -4.0);
        assert!(bounds.max_position().z >= 0.0);
    }

    #[test]
    fn boat_bounds_cover_the_rotated_paddle_sweep() {
        let boat = Boat::new(2, "oak".into(), Vec3::new(0.0, 10.0, 0.0), 90.0);
        let bounds = boat.render_bounds(0.5);
        assert!(bounds.min_position().x <= -1.75);
        assert!(bounds.max_position().x >= 1.75);
        assert!(bounds.min_position().y <= 9.25);
        assert!(bounds.max_position().y >= 11.5);
        assert!(bounds.min_position().z <= -1.75);
        assert!(bounds.max_position().z >= 1.75);
    }

    #[test]
    fn item_frame_bounds_include_the_full_map_and_held_item_depth() {
        let frame = ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 3,
            pos: [0, 0, 0],
            facing: 5,
            glow: false,
            item: None,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            check_interval: 0,
            ticks: 0,
            custom_name: None,
            random: sunrise_core::random::LegacyRandom::new(0),
        };
        let bounds = frame.render_bounds(1.0);
        assert!(bounds.min_position().x <= -0.71875);
        assert!(bounds.max_position().x >= 0.78125);
        assert!(bounds.min_position().y <= 0.0);
        assert!(bounds.max_position().y >= 1.0);
        assert!(bounds.min_position().z <= 0.0);
        assert!(bounds.max_position().z >= 1.0);
    }

    #[test]
    fn minecart_bounds_follow_custom_display_offset() {
        let mut cart = Minecart::new(4, CartKind::Chest, Vec3::new(0.0, 10.0, 0.0));
        cart.custom_offset = Some(32);
        let bounds = cart.render_bounds(1.0);
        assert!(bounds.min_position().y <= 9.5);
        assert!(bounds.max_position().y >= 12.8);
    }
}
