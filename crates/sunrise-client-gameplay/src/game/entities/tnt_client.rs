use crate::game::ClientGame;
use sunrise_client::mesher::Vertex;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::entity::Entity as _;
use sunrise_gameplay_common::game::tnt_common::PrimedTnt;

impl ClientGame<'_> {
    /// `TntRenderer`: swelling over the last 10 ticks and flashing white every 5 ticks.
    pub fn tnt_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        run: std::ops::Range<usize>,
        sprites: Option<[Option<sunrise_client_render::hud::Rect>; 6]>,
    ) {
        let Some(sprites) = sprites else {
            return;
        };
        let range = super::entity_culling::default_range(
            glam::DVec3::splat(PrimedTnt::SIZE),
            self.view_scale(),
        );
        for t in &self.common.tnts[run] {
            // LevelExtractor: the camera entity is not drawn from its own eyes.
            if self.hides_as_camera(t.id) {
                continue;
            }
            // EntityRenderer.shouldRender.
            if !self.standing_entity_in_view(
                camera,
                super::entity_culling::at(t.previous),
                super::entity_culling::at(t.position),
                alpha,
                PrimedTnt::SIZE,
                PrimedTnt::SIZE,
                range,
                0.0,
            ) {
                continue;
            }
            self.mark_motion(
                out.len(),
                sunrise_gameplay_common::game::MOTION_OBJECT | u64::from(t.id),
            );
            let fuse = t.fuse as f32 - alpha as f32 + 1.0;
            if let Some(resolved) = super::native_entity_renderers::tnt_renderer() {
                let partial = alpha as f32;
                let at = glam::DVec3::new(
                    t.previous.x + (t.position.x - t.previous.x) * alpha,
                    t.previous.y + (t.position.y - t.previous.y) * alpha,
                    t.previous.z + (t.position.z - t.previous.z) * alpha,
                );
                let vanilla = t.vanilla_dimensions().expect("primed TNT dimensions");
                let [width, height, eye_height] = t
                    .size_answer
                    .filter(|s| s.vanilla == vanilla)
                    .and_then(|s| s.size)
                    .unwrap_or(vanilla);
                let entity: mod_api::EntityRenderState =
                    mod_api::__private::built::EntityRenderState {
                        id: t.id,
                        partial,
                        age: t.ticks as f32 + partial,
                        yaw: t.rotation[0],
                        pitch: t.rotation[1],
                        position: at.to_array(),
                        size: [width, height],
                        eye_height,
                        distance_to_camera_sq: (at - camera).length_squared(),
                        living: None,
                        tnt_fuse: Some(fuse),
                        orb_value: None,
                        painting: None,
                        item_frame: None,
                        attachments: None,
                    }
                    .into();
                let light = self.common.world.light(
                    t.position.x.floor() as i32,
                    (t.position.y + 0.5).floor() as i32,
                    t.position.z.floor() as i32,
                );
                let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
                let block = [
                    at.x.floor() as i32,
                    at.y.floor() as i32,
                    at.z.floor() as i32,
                ];
                if self.mod_entity_renderer_geometry(
                    &resolved,
                    &entity,
                    "minecraft:tnt",
                    at - camera,
                    Some(at),
                    block,
                    packed,
                    out,
                ) {
                    continue;
                }
            }
            let scale = if fuse < 10.0 {
                let g = (1.0 - fuse / 10.0).clamp(0.0, 1.0);
                1.0 + g * g * g * g * 0.3
            } else {
                1.0
            };
            let lit = fuse >= 0.0 && (fuse / 5.0) as i32 % 2 == 0;
            let base = glam::DVec3::new(
                t.previous.x + (t.position.x - t.previous.x) * alpha,
                t.previous.y + (t.position.y - t.previous.y) * alpha + 0.5,
                t.previous.z + (t.position.z - t.previous.z) * alpha,
            ) - camera;
            let o = base.as_vec3();
            let light = self.common.world.light(
                t.position.x.floor() as i32,
                (t.position.y + 0.5).floor() as i32,
                t.position.z.floor() as i32,
            );
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let v = |x: f32, y: f32, z: f32| o + glam::Vec3::new(x - 0.5, y - 0.5, z - 0.5) * scale;
            let faces = [
                (
                    [v(0., 1., 1.), v(1., 1., 1.), v(1., 1., 0.), v(0., 1., 0.)],
                    1.0,
                ),
                (
                    [v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)],
                    0.5,
                ),
                (
                    [v(1., 0., 0.), v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.)],
                    0.8,
                ),
                (
                    [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
                    0.8,
                ),
                (
                    [v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.), v(0., 1., 0.)],
                    0.6,
                ),
                (
                    [v(1., 0., 1.), v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.)],
                    0.6,
                ),
            ];
            for ((corners, shade), sprite) in faces.into_iter().zip(sprites) {
                let Some(r) = sprite else { continue };
                let c = (255.0 * shade) as u32;
                let color = 0xFF00_0000 | c << 16 | c << 8 | c;
                for (p, uv) in
                    corners
                        .iter()
                        .zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]])
                {
                    // Overlay 5: OverlayTexture white flash.
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: p.to_array(),
                        uv,
                        layer: if lit { 5 } else { 0 },
                        color,
                        light: packed,

                        glint_uv: [0.0, 0.0],
                    });
                }
            }
        }
    }
    /// The sprites of the TNT block's six faces (up, down, north, south, west, east).
    pub fn tnt_face_sprites(&self) -> Option<[Option<sunrise_client_render::hud::Rect>; 6]> {
        let state = self
            .common
            .pack
            .assets
            .registry
            .default_state("minecraft:tnt")?;
        let Ok(models) = self
            .client
            .client_pack
            .render
            .model_catalog
            .collect(state, BlockPos::new(0, 0, 0))
        else {
            return None;
        };
        let mut sprites = [None; 6];
        for face in models.iter().flat_map(|(_, m)| m.faces.iter()) {
            use sunrise_assets::model::Direction as D;
            let index = match face.quad.facing {
                D::Up => 0,
                D::Down => 1,
                D::North => 2,
                D::South => 3,
                D::West => 4,
                D::East => 5,
            };
            if sprites[index].is_none() {
                sprites[index] = self
                    .client
                    .client_pack
                    .layer_names
                    .get(&face.layer)
                    .and_then(|name| self.client.gui.atlas.sprites.get(name).copied());
            }
        }
        Some(sprites)
    }
}

#[cfg(test)]
mod physics_tests {
    use super::PrimedTnt;
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    #[cfg(test)]
    /// `Entity.collectColliders` (#7080): the world border's wall stops primed
    /// TNT. A border of size 10 around (20, 20) has its east wall at x = 25;
    /// the TNT starts at x = 24 on the test scene's flat ground, 1.5 east.
    #[test]
    fn the_world_border_stops_primed_tnt() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = sunrise_level::assets::Assets::load(&root).unwrap();
        let render = sunrise_client_render::assets::RenderAssets::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            &assets,
        )
        .unwrap();
        let facts = sunrise_items::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = sunrise_sim::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = sunrise_level::collisions::load(&root, assets.registry).unwrap();
        let world = sunrise_client_render::assets::test_scene_world(&assets, &render, 3).unwrap();
        let border =
            sunrise_core::geometry::border::BorderGeometry::new([20.0, 20.0], 10.0, 29_999_984)
                .unwrap();
        for bordered in [false, true] {
            let at = Vec3::new(24.0, 4.0, 20.5);
            let mut tnt = PrimedTnt {
                size_answer: None,
                attachment_lifetime: Default::default(),
                rotation: [0.0, 0.0],
                join: Default::default(),
                no_gravity: false,
                id: 0,
                position: at,
                previous: at,
                velocity: Vec3::new(1.5, 0.0, 0.0),
                fuse: 80,
                ticks: 0,
                ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
                fall_distance: 0.0,
                stuck: Vec3::ZERO,
                in_water: false,
                owner: None,
                guest: None,
                power: 4.0,
                used_portal: false,
                block_state: None,
            };
            let border = bordered.then_some(&border);
            tnt.tick(
                &world,
                &collision,
                &assets,
                &facts,
                &fluids,
                &sunrise_gameplay_common::game::test_server_load()
                    .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
                border,
                None,
            )
            .unwrap();
            let east = tnt.bounds().max_position().x;
            if bordered {
                assert!(
                    (east - 25.0).abs() < 1.0e-7,
                    "stops on the wall, got {east}"
                );
            } else {
                assert!(east > 25.0, "no border: goes on, got {east}");
            }
        }
    }

    #[cfg(test)]
    /// `Entity.move`'s last step, `getBlockSpeedFactor`: primed TNT sliding
    /// over soul sand or honey keeps 40% of the horizontal speed it keeps over
    /// stone. Skips quietly without the vanilla data (CI has none).
    #[test]
    fn primed_tnt_on_soul_sand_or_honey_is_slowed_by_the_move() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = sunrise_level::assets::Assets::load(&root).unwrap();
        let render = sunrise_client_render::assets::RenderAssets::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            &assets,
        )
        .unwrap();
        let facts = sunrise_items::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = sunrise_sim::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = sunrise_level::collisions::load(&root, assets.registry).unwrap();
        let motion = sunrise_sim::motion::MotionResources::load(&root, &assets).unwrap();
        let mut after = Vec::new();
        // The tops of the blocks: stone 1, soul sand 7/8, honey 15/16.
        for (block, top) in [
            ("minecraft:stone", 4.0),
            ("minecraft:soul_sand", 3.875),
            ("minecraft:honey_block", 3.9375),
        ] {
            let mut world =
                sunrise_client_render::assets::test_scene_world(&assets, &render, 3).unwrap();
            let id = assets.registry.default_state(block).unwrap();
            for x in 18..=23 {
                for z in 19..=21 {
                    world.set(x, 3, z, id).unwrap();
                }
            }
            let at = Vec3::new(19.5, top, 20.5);
            let mut tnt = PrimedTnt {
                size_answer: None,
                attachment_lifetime: Default::default(),
                rotation: [0.0, 0.0],
                join: Default::default(),
                no_gravity: false,
                id: 0,
                position: at,
                previous: at,
                velocity: Vec3::new(0.2, 0.0, 0.0),
                fuse: 80,
                ticks: 0,
                ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
                fall_distance: 0.0,
                stuck: Vec3::ZERO,
                in_water: false,
                owner: None,
                guest: None,
                power: 4.0,
                used_portal: false,
                block_state: None,
            };
            tnt.tick(
                &world,
                &collision,
                &assets,
                &facts,
                &fluids,
                &sunrise_gameplay_common::game::test_server_load()
                    .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
                None,
                Some(&motion.blocks),
            )
            .unwrap();
            after.push(tnt.velocity.x);
        }
        let (stone, soul, honey) = (after[0], after[1], after[2]);
        assert!(stone > 0.0);
        assert!(
            (soul - stone * f64::from(0.4_f32)).abs() < 1.0e-9,
            "soul sand {soul} vs stone {stone}"
        );
        assert!(
            (honey - stone * f64::from(0.4_f32)).abs() < 1.0e-9,
            "honey {honey} vs stone {stone}"
        );
    }

    #[cfg(test)]
    #[test]
    fn primed_tnt_follows_water_currents_after_moving() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = sunrise_level::assets::Assets::load(&root).unwrap();
        let render = sunrise_client_render::assets::RenderAssets::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            &assets,
        )
        .unwrap();
        let facts = sunrise_items::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = sunrise_sim::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = sunrise_level::collisions::load(&root, assets.registry).unwrap();
        let mut world =
            sunrise_client_render::assets::test_scene_world(&assets, &render, 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let mut state = assets.registry.state(water).unwrap().clone();
        state.properties.insert("level".into(), "1".into());
        let flowing = assets.registry.resolve(&state).unwrap();
        for x in 19..=22 {
            for z in 19..=21 {
                world.set(x, 3, z, stone).unwrap();
            }
        }
        world.set(20, 4, 20, water).unwrap();
        world.set(21, 4, 20, flowing).unwrap();
        let at = Vec3::new(20.5, 4.0, 20.5);
        let mut tnt = PrimedTnt {
            size_answer: None,
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            fuse: 80,
            ticks: 0,
            ground: Default::default(),
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            in_water: false,
            owner: None,
            guest: None,
            power: 4.0,
            used_portal: false,
            block_state: None,
        };
        tnt.tick(
            &world,
            &collision,
            &assets,
            &facts,
            &fluids,
            &sunrise_gameplay_common::game::test_server_load()
                .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            tnt.position.x, at.x,
            "current is applied after motion, not before"
        );
        assert!(
            (tnt.velocity.x - 0.014).abs() < 1e-12,
            "water must push primed TNT east: {:?}",
            tnt.velocity
        );
        assert_eq!(tnt.fuse, 79);
        tnt.tick(
            &world,
            &collision,
            &assets,
            &facts,
            &fluids,
            &sunrise_gameplay_common::game::test_server_load()
                .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
            None,
            None,
        )
        .unwrap();
        assert!((tnt.position.x - at.x - 0.014).abs() < 1e-12);
        assert!(tnt.ground.on_ground);
        tnt.position = at;
        tnt.velocity = Vec3::ZERO;
        tnt.fuse = 1;
        tnt.tick(
            &world,
            &collision,
            &assets,
            &facts,
            &fluids,
            &sunrise_gameplay_common::game::test_server_load()
                .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
            None,
            None,
        )
        .unwrap();
        assert_eq!(tnt.fuse, 0);
        assert_eq!(
            tnt.velocity.x, 0.0,
            "expired TNT must not receive a current"
        );
        world.set(21, 4, 20, water).unwrap();
        tnt.position = at;
        tnt.velocity = Vec3::ZERO;
        tnt.fuse = 80;
        tnt.tick(
            &world,
            &collision,
            &assets,
            &facts,
            &fluids,
            &sunrise_gameplay_common::game::test_server_load()
                .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            tnt.velocity.x, 0.0,
            "still water must not add horizontal motion"
        );
        world
            .set(
                20,
                3,
                20,
                assets.registry.default_state("minecraft:air").unwrap(),
            )
            .unwrap();
        tnt.ground.on_ground = false;
        tnt.velocity = Vec3::ZERO;
        tnt.tick(
            &world,
            &collision,
            &assets,
            &facts,
            &fluids,
            &sunrise_gameplay_common::game::test_server_load()
                .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
            None,
            None,
        )
        .unwrap();
        assert!(
            tnt.position.y < at.y && tnt.velocity.y < 0.0,
            "TNT sinks; water does not add buoyancy"
        );
    }

    #[cfg(test)]
    #[test]
    fn a_soul_sand_bubble_column_lifts_primed_tnt() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = sunrise_level::assets::Assets::load(&root).unwrap();
        let render = sunrise_client_render::assets::RenderAssets::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            &assets,
        )
        .unwrap();
        let facts = sunrise_items::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = sunrise_sim::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = sunrise_level::collisions::load(&root, assets.registry).unwrap();
        let mut world =
            sunrise_client_render::assets::test_scene_world(&assets, &render, 3).unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let soul_sand = assets
            .registry
            .default_state("minecraft:soul_sand")
            .unwrap();
        // Soul sand makes an up-pushing column (`drag=false`); the block's
        // default state is the down-dragging one.
        let drag_down = assets
            .registry
            .default_state("minecraft:bubble_column")
            .unwrap();
        let mut up = assets.registry.state(drag_down).unwrap().clone();
        up.properties.insert("drag".into(), "false".into());
        let column = assets.registry.resolve(&up).unwrap();
        for y in 3..=8 {
            world.set(20, y, 20, column).unwrap();
        }
        world.set(20, 2, 20, soul_sand).unwrap();
        world.set(20, 9, 20, water).unwrap();
        // BubbleColumnBlock.entityInside: the columns its box touches lift it
        // (Entity.handleOnInsideBubbleColumn / handleOnAboveBubbleColumn).
        let at = Vec3::new(20.5, 6.0, 20.5);
        let mut tnt = PrimedTnt {
            size_answer: None,
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            fuse: 80,
            ticks: 0,
            ground: Default::default(),
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            in_water: false,
            owner: None,
            guest: None,
            power: 4.0,
            used_portal: false,
            block_state: None,
        };
        for _ in 0..10 {
            tnt.tick(
                &world,
                &collision,
                &assets,
                &facts,
                &fluids,
                &sunrise_gameplay_common::game::test_server_load()
                    .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
                None,
                None,
            )
            .unwrap();
        }
        assert!(
            tnt.velocity.y > 0.0,
            "soul sand lifts primed TNT, got {:?}",
            tnt.velocity
        );
        assert!(
            tnt.position.y > at.y,
            "the column carries it up, got {}",
            tnt.position.y
        );
        // Without the column the same TNT only falls.
        for y in 3..=8 {
            world.set(20, y, 20, water).unwrap();
        }
        tnt.position = at;
        tnt.velocity = Vec3::ZERO;
        tnt.ground.on_ground = false;
        for _ in 0..4 {
            tnt.tick(
                &world,
                &collision,
                &assets,
                &facts,
                &fluids,
                &sunrise_gameplay_common::game::test_server_load()
                    .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
                None,
                None,
            )
            .unwrap();
        }
        assert!(
            tnt.position.y < at.y,
            "water alone does not lift it, got {}",
            tnt.position.y
        );
    }

    #[cfg(test)]
    /// `Entity.onAboveBubbleColumn` sends the surface particles only from the top of
    /// a column: `tick` names the column block it is at the top of, and none while
    /// the column has water above it.
    #[test]
    fn primed_tnt_reports_the_surface_of_a_bubble_column() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = sunrise_level::assets::Assets::load(&root).unwrap();
        let render = sunrise_client_render::assets::RenderAssets::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            &assets,
        )
        .unwrap();
        let facts = sunrise_items::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = sunrise_sim::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = sunrise_level::collisions::load(&root, assets.registry).unwrap();
        let mut world =
            sunrise_client_render::assets::test_scene_world(&assets, &render, 3).unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let soul_sand = assets
            .registry
            .default_state("minecraft:soul_sand")
            .unwrap();
        let default = assets
            .registry
            .default_state("minecraft:bubble_column")
            .unwrap();
        let mut up = assets.registry.state(default).unwrap().clone();
        up.properties.insert("drag".into(), "false".into());
        let column = assets.registry.resolve(&up).unwrap();
        for y in 3..=8 {
            world.set(20, y, 20, column).unwrap();
        }
        world.set(20, 2, 20, soul_sand).unwrap();
        let at = Vec3::new(20.5, 6.0, 20.5);
        let primed = || PrimedTnt {
            size_answer: None,
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            fuse: 80,
            ticks: 0,
            ground: Default::default(),
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            in_water: false,
            owner: None,
            guest: None,
            power: 4.0,
            used_portal: false,
            block_state: None,
        };
        let mut tops = Vec::new();
        let mut tnt = primed();
        for _ in 0..30 {
            tops.extend(
                tnt.tick(
                    &world,
                    &collision,
                    &assets,
                    &facts,
                    &fluids,
                    &sunrise_gameplay_common::game::test_server_load()
                        .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
                    None,
                    None,
                )
                .unwrap(),
            );
        }
        assert!(!tops.is_empty(), "it reaches the top of the column");
        assert!(
            tops.iter().all(|p| *p == BlockPos::new(20, 8, 20)),
            "only the surface block, got {tops:?}"
        );
        // Water above the top block: it is no longer the surface.
        world.set(20, 9, 20, water).unwrap();
        let mut tnt = primed();
        for _ in 0..10 {
            assert!(
                tnt.tick(
                    &world,
                    &collision,
                    &assets,
                    &facts,
                    &fluids,
                    &sunrise_gameplay_common::game::test_server_load()
                        .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
                    None,
                    None,
                )
                .unwrap()
                .is_empty()
            );
        }
    }
}
