// This row is present only in the installed-client daily. Startup has loaded the native mods
// and installed their patches before it reaches this point, but has not opened a window yet.
fn client_draw() {
    use std::sync::Arc;
    use sunrise_client::gfx::entity_skin::SkinFrame;
    use sunrise_client::mobs::{Mob, MobTypes, Part, Polygon, render, skin};
    use sunrise_client::mobs_render::{
        ClientMobData, ClientMobModels, ClientMobRender, ClientMobVisuals,
    };
    use sunrise_core::geometry::Vec3;

    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_FNR_LOWER_DRAW").unwrap());
    let attributes = sunrise_world::attributes::entity::Catalog::from_json(&serde_json::json!({
        "data_version": sunrise_world::DATA_VERSION,
        "attributes": [],
        "entities": {},
    }))
    .unwrap();
    let kind = Arc::new(
        MobTypes::load_for_server(&root, &attributes)
            .unwrap()
            .types
            .remove("minecraft:wolf")
            .unwrap(),
    );
    let names = ["body", "head", "left_arm", "right_arm", "right_leg"];
    let corners = [
        [0.0, 0.0, 0.0],
        [0.0, 12.0, 0.0],
        [0.0, 12.0, 4.0],
        [0.0, 0.0, 4.0],
    ];
    let children = names
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            let x = i as f32;
            Part {
                name: name.into(),
                pose: [x, 2.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                polygons: vec![Polygon {
                    normal: [1.0, 0.0, 0.0],
                    vertices: corners.map(|[px, py, pz]| [px, py, pz, (x + pz) / 16.0, py / 16.0]),
                    cull_back: false,
                }],
                children: Vec::new(),
                layer: None,
            }
        })
        .collect();
    // A wet wolf supplies a non-white body tint and can use the GPU. Both synthetic
    // models have identical parts; only the layer name the installed rows see differs.
    let mut model = Part {
        name: "root".into(),
        pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        polygons: Vec::new(),
        children,
        layer: Some("minecraft:wolf#main".into()),
    };
    let mut control_models = ClientMobModels::default();
    control_models.insert(
        "minecraft:wolf".into(),
        ClientMobVisuals {
            shadow: None,
            model: Arc::new(model.clone()),
            layers: Default::default(),
            texture: String::new(),
        },
    );
    let mut control = Mob::new(1, kind.clone(), Vec3::ZERO, 0.0);
    control.hurt_time = 4;
    control.extra.flag2 = true;
    model.layer = Some("minecraft:player#main".into());
    let mut selected_models = ClientMobModels::default();
    selected_models.insert(
        "minecraft:wolf".into(),
        ClientMobVisuals {
            shadow: None,
            model: Arc::new(model),
            layers: Default::default(),
            texture: String::new(),
        },
    );
    let mut selected = Mob::new(1, kind, Vec3::ZERO, 0.0);
    selected.hurt_time = control.hurt_time;
    selected.extra.flag2 = control.extra.flag2;
    let data = ClientMobData::default();
    let control_render = ClientMobRender {
        models: &control_models,
        data: &data,
    };
    let selected_render = ClientMobRender {
        models: &selected_models,
        data: &data,
    };
    let uv = |_: &str| Some([0.125, 0.25, 0.75, 0.875]);
    let light = 0x70a0;
    let camera = glam::DVec3::ZERO;
    let skinned = |mob: &Mob, render_data: &ClientMobRender<'_>| {
        let mut frame = SkinFrame::default();
        let mut models = skin::SkinModels::default();
        let mut vertices = Vec::new();
        render::render_skinned(
            mob,
            0.5,
            camera,
            &uv,
            light,
            render_data,
            &mut render::FrameDraw::default(),
            &mut vertices,
            &mut skin::Skin::new(&mut models, &mut frame),
        );
        (vertices, frame.vertices)
    };
    let (plain_cpu, plain_gpu) = skinned(&control, &control_render);
    assert!(
        plain_cpu.is_empty(),
        "the unselected model should remain GPU-posed"
    );
    assert_eq!(plain_gpu, 20);
    let (bent, bent_gpu) = skinned(&selected, &selected_render);
    assert_eq!(bent_gpu, 0, "the taken model must leave the GPU");
    assert_eq!(
        bent.len(),
        24,
        "two quads from the right arm instead of one"
    );

    let mut plain = Vec::new();
    render::render_with_frame(
        &control,
        0.5,
        camera,
        &uv,
        light,
        &control_render,
        &[],
        &mut render::FrameDraw::default(),
        &mut plain,
    );
    assert_eq!(plain.len(), 20);
    let bytes = |vertices: &[sunrise_client::mesher::Vertex]| {
        bytemuck::cast_slice::<_, u8>(vertices).to_vec()
    };
    assert_eq!(
        bytes(&bent[..12]),
        bytes(&plain[..12]),
        "body, head and left arm"
    );
    assert_eq!(bytes(&bent[20..]), bytes(&plain[16..]), "right leg");
    let arm = &plain[12..16];
    assert_eq!(arm[0].color, 0xFFBF_BFBF, "the wet-shade tint is exercised");
    assert_eq!(arm[0].light, light);
    assert_ne!(arm[0].layer, 0, "the hurt overlay is actually exercised");
    for segment in [&bent[12..16], &bent[16..20]] {
        for (v, a) in segment.iter().zip(arm) {
            assert_eq!(
                (v.uv, v.color, v.light, v.layer, v.normal, v.glint_uv),
                (a.uv, a.color, a.light, a.layer, a.normal, a.glint_uv),
                "both arm segments keep the original quad's paint and normal"
            );
        }
    }
    // In right-arm coordinates the shoulder is at the quad's origin and the elbow is
    // six pixels down. Predict both emitted segments from this hinge and the original quad.
    let shoulder =
        render::part_transform(&selected, 0.5, camera, &selected_render, "right_arm").unwrap();
    let half = glam::Mat4::from_scale(glam::Vec3::new(1.0, 0.5, 1.0));
    let elbow = shoulder * glam::Mat4::from_translation(glam::Vec3::Y * (6.0 / 16.0));
    for (segment, transform) in [
        (&bent[12..16], shoulder * half),
        (
            &bent[16..20],
            elbow * glam::Mat4::from_rotation_x(0.8) * half,
        ),
    ] {
        for (v, corner) in segment.iter().zip(corners) {
            assert_eq!(
                v.pos.map(f32::to_bits),
                transform
                    .transform_point3(glam::Vec3::from_array(corner) / 16.0)
                    .to_array()
                    .map(f32::to_bits)
            );
        }
    }
    fnr_lower::note("installed client draw passed");
    std::process::exit(0);
}
