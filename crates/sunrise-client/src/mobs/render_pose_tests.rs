fn pose_test_cube(x: f32) -> serde_json::Value {
    serde_json::json!({ "pose": [x, 0.0, 0.0], "polygons": [[[x, 0.0, 0.0, 0.0, 0.0], [x + 1.0, 0.0, 0.0, 1.0, 0.0], [x + 1.0, 1.0, 0.0, 1.0, 1.0], [x, 1.0, 0.0, 0.0, 1.0]]] })
}

fn quadruped_fixture() -> (Mob, ClientMobVisuals) {
    let cube = pose_test_cube;
    let mut head = cube(0.0);
    head["pose"] = serde_json::json!([3.5, -2.5, 1.0, 0.0, 0.0, 0.25, 2.0, 0.5, 1.0]);
    head["children"] = serde_json::json!({"jaw": cube(1.0)});
    let quadruped = serde_json::json!({ "children": {
        "head": head, "body": cube(0.0), "right_hind_leg": cube(-2.0),
        "left_hind_leg": cube(2.0), "right_front_leg": cube(-2.0), "left_front_leg": cube(2.0),
    }});
    let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
    kind.species = Species::Cow;
    kind.anim = super::super::Anim::Quadruped;
    let visuals = ClientMobVisuals {
        shadow: None,
        model: std::sync::Arc::new(Part::parse("root", &quadruped)),
        layers: Default::default(),
        texture: String::new(),
    };
    let mut cow = Mob::new(
        2,
        std::sync::Arc::new(kind),
        sunrise_core::geometry::Vec3::ZERO,
        0.0,
    );
    cow.age = 37;
    (cow, visuals)
}

#[test]
fn frame_model_lends_the_drawn_pose_without_changing_geometry() {
    use std::f32::consts::FRAC_PI_2;

    let (mut cow, cow_visuals) = quadruped_fixture();
    cow.head_pitch = 90.0;
    cow.previous_head_pitch = 90.0;
    let mut models = ClientMobModels::default();
    models.insert(cow.kind.id.clone(), cow_visuals);
    let data = ClientMobData {
        frame_models: BakedFrameModels::bake(models.types.values()),
        ..Default::default()
    };
    let source = ClientMobRender {
        models: &models,
        data: &data,
    };
    let cow_model = models.model(&cow.kind.id).unwrap();
    let atlas = |_: &str| Some([0.0, 0.0, 1.0, 1.0]);
    let check = |model: &dyn mod_api::FrameModel| {
        // The empty root still leads to its children. The head's pitch is animated on this
        // draw, while its offset, roll and scale are the fixture's literal local values.
        let root = model.root();
        assert_eq!((root.index, root.name, root.parent), (0, "root", None));
        assert_eq!((root.first_child, root.next_sibling), (Some(1), None));
        assert!(root.visible);
        let body = model.part_at(1).unwrap();
        assert_eq!(
            (body.name, body.parent, body.next_sibling),
            ("body", Some(0), Some(2))
        );
        let head = model.part("head").unwrap();
        assert_eq!(
            (head.index, head.parent, head.first_child, head.next_sibling),
            (2, Some(0), Some(3), Some(4))
        );
        let expected = [3.5, -2.5, 1.0, FRAC_PI_2, 0.0, 0.25, 2.0, 0.5, 1.0];
        assert_eq!(*head.local_pose, expected);
        let pose = head.pose();
        assert_eq!(
            [
                pose.x,
                pose.y,
                pose.z,
                pose.x_rot,
                pose.y_rot,
                pose.z_rot,
                pose.x_scale,
                pose.y_scale,
                pose.z_scale
            ],
            expected,
        );
        let jaw = model.part_at(3).unwrap();
        assert_eq!(
            (jaw.name, jaw.parent, jaw.first_child, jaw.next_sibling),
            ("jaw", Some(2), None, None)
        );
        assert_eq!(jaw.pose(), mod_api::PartPose::offset(1.0, 0.0, 0.0));
        assert!(jaw.visible);
        assert!(model.part("left_sleeve").is_none());
        assert!(model.part_at(8).is_none());
    };

    let cpu = |capture_model, layers_only| {
        let mut frame = FrameDraw::new(capture_model);
        let mut out = Vec::new();
        if layers_only {
            let extra = [(
                WornModel::Vanilla(cow_model),
                [0.0, 0.0, 1.0, 1.0],
                0xFFFF_FFFF,
                0,
                false,
            )];
            render_layers(
                &cow,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &source,
                &extra,
                &mut frame,
                &mut out,
            );
        } else {
            render_with_frame(
                &cow,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &source,
                &[],
                &mut frame,
                &mut out,
            );
        }
        (out, frame)
    };
    for layers_only in [false, true] {
        let (before, uncaptured) = cpu(false, layers_only);
        let (after, captured) = cpu(true, layers_only);
        assert!(uncaptured.model().is_none());
        assert!(!before.is_empty());
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&before),
            bytemuck::cast_slice::<_, u8>(&after)
        );
        check(captured.model().unwrap());
    }

    // The skinned path keeps the same local pose and submits the same GPU jobs and matrices.
    let gpu = |capture_model| {
        let mut skin_models = super::super::skin::SkinModels::default();
        let mut skin_frame = crate::gfx::entity_skin::SkinFrame::default();
        let mut skin = super::super::skin::Skin::new(&mut skin_models, &mut skin_frame);
        let mut frame = FrameDraw::new(capture_model);
        let mut out = Vec::new();
        render_skinned(
            &cow,
            0.5,
            glam::DVec3::ZERO,
            &atlas,
            0xf0f0,
            &source,
            &mut frame,
            &mut out,
            &mut skin,
        );
        assert!(out.is_empty());
        (skin_frame, frame)
    };
    let (before, uncaptured) = gpu(false);
    let (after, captured) = gpu(true);
    assert!(uncaptured.model().is_none());
    assert!(!before.jobs.is_empty(), "the cow takes the skinned path");
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&before.jobs),
        bytemuck::cast_slice::<_, u8>(&after.jobs)
    );
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&before.parts),
        bytemuck::cast_slice::<_, u8>(&after.parts)
    );
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&before.new_model),
        bytemuck::cast_slice::<_, u8>(&after.new_model)
    );
    check(captured.model().unwrap());
}

#[test]
fn pre_lends_the_bind_pose_and_edits_more_than_eight_parts_for_one_draw() {
    use std::f32::consts::FRAC_PI_2;

    let (mut cow, mut cow_visuals) = quadruped_fixture();
    let mut head = pose_test_cube(0.0);
    head["children"] = serde_json::json!({"jaw": pose_test_cube(0.0)});
    let mut children = serde_json::Map::new();
    children.insert("head".into(), head);
    for i in 0..10 {
        let mut part = pose_test_cube(0.0);
        if i >= 8 {
            part["children"] = serde_json::json!({"tip": pose_test_cube(i as f32)});
        }
        children.insert(format!("named_part_{i}"), part);
    }
    cow_visuals.model = std::sync::Arc::new(Part::parse(
        "root",
        &serde_json::json!({"children": children}),
    ));
    cow.head_pitch = 90.0;
    cow.previous_head_pitch = 90.0;
    let mut models = ClientMobModels::default();
    models.insert(cow.kind.id.clone(), cow_visuals);
    let data = ClientMobData {
        frame_models: BakedFrameModels::bake(models.types.values()),
        ..Default::default()
    };
    let source = ClientMobRender {
        models: &models,
        data: &data,
    };
    let atlas = |_: &str| Some([0.0, 0.0, 1.0, 1.0]);
    let draw = |frame: &mut FrameDraw<'_>| {
        let mut out = Vec::new();
        render_with_frame(
            &cow,
            0.5,
            glam::DVec3::ZERO,
            &atlas,
            0xf0f0,
            &source,
            &[],
            frame,
            &mut out,
        );
        out.len()
    };

    let mut frame = FrameDraw::new(true);
    let pre = frame.prepare(&cow, &data, &models).unwrap();
    let head = pre.part("head").unwrap();
    let (head_index, jaw_index) = (head.index, head.first_child.unwrap());
    assert_eq!(head.pose().x_rot, 0.0, "Pre sees bind pose, not setupAnim");
    assert!(pre.part("missing").is_none());
    assert!(pre.part_at(usize::MAX).is_none());
    assert!(!pre.hide(usize::MAX));
    assert!(pre.part_at(jaw_index).unwrap().visible);
    let first_tip = pre.part("tip").unwrap().index;
    let second_tip = pre.part("named_part_9").unwrap().first_child.unwrap();
    assert_ne!(first_tip, second_tip);
    assert_eq!(pre.part_at(second_tip).unwrap().pose().x, 9.0);
    assert!(pre.hide(first_tip));
    assert!(
        pre.part_at(second_tip).unwrap().visible,
        "indices distinguish duplicate names"
    );
    for i in 0..9 {
        let index = pre.part(&format!("named_part_{i}")).unwrap().index;
        assert!(pre.hide(index));
        assert!(!pre.part_at(index).unwrap().visible);
    }
    assert!(pre.hide(head_index));
    assert!(
        !pre.part_at(jaw_index).unwrap().visible,
        "the parent is hidden"
    );
    assert_eq!(draw(&mut frame), 8, "only named_part_9 and its tip remain");
    let post = frame.model().unwrap();
    assert_eq!(post.part("head").unwrap().pose().x_rot, FRAC_PI_2);
    assert!(!post.part_at(jaw_index).unwrap().visible);
    drop(frame);

    let mut fresh = FrameDraw::new(true);
    let pre = fresh.prepare(&cow, &data, &models).unwrap();
    assert!(pre.part_at(jaw_index).unwrap().visible);
    assert_eq!(
        draw(&mut fresh),
        56,
        "the ten named parts, two tips, head and jaw all draw"
    );
    assert!(fresh.model().unwrap().part_at(jaw_index).unwrap().visible);
}

fn hide_in_draw<'m>(
    frame: &mut FrameDraw<'m>,
    mob: &'m Mob,
    data: &ClientMobData,
    models: &'m ClientMobModels,
    names: &[&str],
) {
    let model = frame.prepare(mob, data, models).unwrap();
    for name in names {
        let index = model.part(name).unwrap().index;
        assert!(model.hide(index));
    }
}

#[test]
fn named_hidden_parts_leave_the_body_and_its_overlays_but_not_armour() {
    let (cow, cow_visuals) = quadruped_fixture();
    let cube = pose_test_cube;
    let mut hat = cube(0.0);
    hat["children"] = serde_json::json!({"hat_rim": cube(0.0)});
    let mut head = cube(0.0);
    head["children"] = serde_json::json!({"hat": hat, "nose": cube(0.0)});
    let tree = serde_json::json!({"children": {"head": head, "body": cube(0.0)}});
    let villager_model = std::sync::Arc::new(Part::parse("root", &tree));
    let mut kind = (*cow.kind).clone();
    kind.id = "minecraft:villager".into();
    kind.species = Species::Villager;
    kind.anim = super::super::Anim::Villager;
    let villager_kind = std::sync::Arc::new(kind);
    let villager = Mob::new(
        3,
        villager_kind.clone(),
        sunrise_core::geometry::Vec3::ZERO,
        0.0,
    );
    let mut layered = Mob::new(4, villager_kind, sunrise_core::geometry::Vec3::ZERO, 0.0);
    let mut models = ClientMobModels::default();
    models.insert(cow.kind.id.clone(), cow_visuals);
    models.insert(
        "minecraft:villager".into(),
        ClientMobVisuals {
            shadow: None,
            model: villager_model.clone(),
            layers: Default::default(),
            texture: String::new(),
        },
    );
    let mut layered_models = ClientMobModels::default();
    layered_models.insert(
        "minecraft:villager".into(),
        ClientMobVisuals {
            shadow: None,
            model: villager_model.clone(),
            layers: [("villager#main".into(), villager_model.clone())]
                .into_iter()
                .collect(),
            texture: String::new(),
        },
    );
    let data = ClientMobData {
        frame_models: BakedFrameModels::bake(
            models.types.values().chain(layered_models.types.values()),
        ),
        ..Default::default()
    };
    let atlas = |_: &str| Some([0.0, 0.0, 1.0, 1.0]);
    let draw = |mob: &Mob,
                mob_models: &ClientMobModels,
                names: &[&str],
                gpu: bool,
                extra: &[WornLayer<'_>]| {
        let mut out = Vec::new();
        let mut skin_models = super::super::skin::SkinModels::default();
        let mut skin_frame = crate::gfx::entity_skin::SkinFrame::default();
        let mut skin = super::super::skin::Skin::new(&mut skin_models, &mut skin_frame);
        let mut frame = FrameDraw::default();
        hide_in_draw(&mut frame, mob, &data, mob_models, names);
        let source = ClientMobRender {
            models: mob_models,
            data: &data,
        };
        render_visible(
            mob,
            0.5,
            None,
            [glam::Mat4::IDENTITY],
            &atlas,
            0xf0f0,
            &source,
            extra,
            Some(body_color(mob, 0.5)),
            &mut frame,
            &mut out,
            gpu.then_some(&mut skin),
        );
        (
            out.len() as u32,
            skin_frame.jobs.iter().map(|job| job.count).sum::<u32>(),
        )
    };
    let armour = [(WornModel::Vanilla(&villager_model), [0.0, 0.0, 1.0, 1.0], 0xf0f0, 0, false)];
    let cpu = |mob: &Mob, mob_models: &ClientMobModels, names: &[&str]| {
        draw(mob, mob_models, names, false, &[]).0
    };
    let gpu = |mob: &Mob, mob_models: &ClientMobModels, names: &[&str]| {
        let (cpu, gpu) = draw(mob, mob_models, names, true, &[]);
        assert_eq!(cpu, 0, "the whole body takes the GPU path");
        gpu
    };
    for route in [
        &cpu as &dyn Fn(&Mob, &ClientMobModels, &[&str]) -> u32,
        &gpu,
    ] {
        let whole = route(&cow, &models, &[]);
        assert!(whole > 0);
        assert_eq!(route(&cow, &models, &["right_hind_leg"]), whole - 4);
        let whole = route(&villager, &models, &[]);
        assert_eq!(route(&villager, &models, &["hat_rim", "nose"]), whole - 8);
        assert_eq!(
            route(&villager, &models, &["hat"]),
            whole - 8,
            "the hat rim goes with it"
        );
    }
    for gpu_path in [false, true] {
        let draw_armoured = |names: &[&str], extra: &[WornLayer<'_>]| {
            let (cpu, gpu) = draw(&villager, &models, names, gpu_path, extra);
            cpu + gpu
        };
        let bare = draw_armoured(&[], &[]);
        let armoured = draw_armoured(&[], &armour);
        assert!(armoured > bare);
        assert_eq!(
            draw_armoured(&["hat_rim", "nose"], &armour),
            armoured - 8,
            "armour retains every part on CPU and GPU"
        );
    }
    // VillagerProfessionLayer draws the type, profession and level on `getParentModel()`:
    // a part hidden on the body leaves them too.
    let table = super::super::villager::ResolvedProfessions::vanilla();
    super::villager_client::install(&table, &Default::default());
    layered.extra.profession = super::super::villager::ProfessionState::Vanilla(5);
    assert_eq!(layers(&layered, 0.0).len(), 3, "type, profession and level");
    assert_eq!(
        cpu(&layered, &layered_models, &["nose"]),
        cpu(&layered, &layered_models, &[]) - 16
    );
    assert_eq!(
        gpu(&layered, &layered_models, &["nose"]),
        gpu(&layered, &layered_models, &[]) - 16
    );
    // A full profession hat makes the type overlay use the no-hat poses.
    // Its head (including nose) is absent even before the mod hides the nose;
    // the body, profession and level each lose one four-vertex polygon.
    use sunrise_assets::resource_source::{Resource, ResourceSource};
    let source = ResourceSource::indexed(std::collections::BTreeMap::from([
        (
            "assets/minecraft/textures/entity/villager/profession/farmer.png".into(),
            Resource::bytes(vec![]),
        ),
        (
            "assets/minecraft/textures/entity/villager/profession/farmer.png.mcmeta".into(),
            Resource::bytes(br#"{"villager":{"hat":"full"}}"#.to_vec()),
        ),
    ]));
    super::villager_client::install(
        &table,
        &super::super::villager_hats::VillagerHats::load(&source),
    );
    assert!(!super::villager_client::type_hat_shown(
        &layered.extra.profession,
        false,
        layered.extra.variant
    ));
    assert_eq!(
        cpu(&layered, &layered_models, &["nose"]),
        cpu(&layered, &layered_models, &[]) - 12
    );
    assert_eq!(
        gpu(&layered, &layered_models, &["nose"]),
        gpu(&layered, &layered_models, &[]) - 12
    );
    super::villager_client::install(&table, &Default::default());
}

#[test]
fn equipment_model_override_draws_with_body_poses_and_inventory_does_not_dispatch_it() {
    struct Source {
        models: ClientMobModels,
        data: ClientMobData,
        model: std::sync::Arc<EquipmentModel>,
        calls: std::cell::Cell<usize>,
    }
    impl RenderDataSource for Source {
        fn mob_tables(&self) -> ClientMobRender<'_> {
            ClientMobRender {
                models: &self.models,
                data: &self.data,
            }
        }
        fn equipment_layers(&self, _: &Mob) -> Vec<Layer> {
            self.calls.set(self.calls.get() + 1);
            let mut layer = Layer::equipment(
                "no_vanilla_layer",
                "test:equipment",
                0xFFFFFFFF,
                ARMOR_GLINT,
            );
            layer.equipment_model = Some((self.model.clone(), ArmorSlot::Chest));
            layer.equipment_uv = Some([0.25, 0.25, 0.5, 0.5]);
            vec![layer]
        }
        fn equipment_inventory(&self, _: &Mob) -> Vec<Layer> {
            Vec::new()
        }
    }
    let (mob, models, data) = humanoid_zombie();
    let mut reference = (**models.model(&mob.kind.id).unwrap()).clone();
    reference.children.retain(|node| node.name == "body");
    let reference = std::sync::Arc::new(reference);
    let baked = EquipmentModel::bake(&crate::mobs::model_layers::from_part(&reference)).unwrap();
    let source = Source {
        models,
        model: std::sync::Arc::new(baked),
        data,
        calls: std::cell::Cell::new(0),
    };
    let uv = [0.25, 0.25, 0.5, 0.5];
    let atlas = |name: &str| {
        assert_ne!(
            name, "test:equipment",
            "final UVs must not consult the live atlas"
        );
        Some([0.0, 0.0, 1.0, 1.0])
    };
    assert!(
        !drawn_textures(&mob, &source)
            .iter()
            .any(|name| name == "test:equipment")
    );
    assert_eq!(
        source.calls.get(),
        0,
        "inventory must not invoke frame providers"
    );
    let mut actual = Vec::new();
    render_with(
        &mob,
        0.5,
        glam::DVec3::ZERO,
        &atlas,
        0xF0F0,
        &source,
        &[],
        &mut actual,
    );
    let mut expected = Vec::new();
    let plain = source.mob_tables();
    render_with(
        &mob,
        0.5,
        glam::DVec3::ZERO,
        &atlas,
        0xF0F0,
        &plain,
        &[(
            WornModel::Vanilla(&reference),
            uv,
            0xFFFFFFFF,
            ARMOR_GLINT,
            false,
        )],
        &mut expected,
    );
    assert_eq!(source.calls.get(), 1);
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&actual),
        bytemuck::cast_slice::<_, u8>(&expected)
    );
}
