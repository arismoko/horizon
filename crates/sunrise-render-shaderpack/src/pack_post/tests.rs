//! The unit tests of the pack's construction and types: its formats, samplers and images, its
//! pipelines made together, and its bind group layouts. They were the client `mod.rs`'s, where
//! one file held every name; the ones that build a pack's frame fixtures stay in the client.

use std::collections::HashMap;

use glam::DVec3;
use sunrise_shaders::{BUFFERS, Format, Group};
use wgpu::TextureFormat as T;

use super::*;

/// An adapter and device when the machine has one (a headless test machine may not): tests that
/// only contract the driver return then.
fn optional_device() -> Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let adapter = crate::test_gpu::adapter()?;
    let (device, queue) = crate::test_gpu::device(&adapter);
    Some((adapter, device, queue))
}

#[test]
fn shadow_cache_key_tracks_world_and_time_changes() {
    let key = ShadowCacheKey {
        sun_angle: 0.0,
        moon_angle: 180.0,
        snapped_camera: Some(DVec3::ZERO),
        shadow_settings_hash: 1,
        shadow_scene_revision: (2, 3),
        ticks: 4,
        day_time: 5,
        shadow_uniforms_hash: 6,
    };

    assert_ne!(
        key,
        ShadowCacheKey {
            shadow_scene_revision: (8, 3),
            ..key
        }
    );
    assert_ne!(key, ShadowCacheKey { day_time: 9, ..key });
    assert_ne!(
        key,
        ShadowCacheKey {
            shadow_uniforms_hash: 10,
            ..key
        }
    );
}

/// Two textures with a sampler each become two textures sharing one sampler variable.
#[test]
fn samplers_of_a_kind_share_one_variable() {
    let mut module = naga::front::wgsl::parse_str(
        "@group(0) @binding(1) var a: texture_2d<f32>;\n\
         @group(0) @binding(2) var sa: sampler;\n\
         @group(0) @binding(3) var b: texture_2d<f32>;\n\
         @group(0) @binding(4) var sb: sampler;\n\
         @fragment fn main() -> @location(0) vec4<f32> {\n\
         return textureSample(a, sa, vec2(0.5)) + textureSample(b, sb, vec2(0.5));\n}\n",
    )
    .unwrap();
    share_samplers(&mut module, &HashMap::from([(2, 2), (4, 2)]));
    let info = naga::valid::Validator::new(Default::default(), Default::default())
        .validate(&module)
        .unwrap();
    let used: Vec<u32> = module
        .global_variables
        .iter()
        .filter(|(h, _)| !info.get_entry_point(0)[*h].is_empty())
        .filter_map(|(_, g)| g.binding.as_ref().map(|b| b.binding))
        .collect();
    assert_eq!(used, [1, 2, 3]);
}

/// `scale.composite2 = 0.25 0.5 0.25` on a 1280×720 picture: Iris 1.11.7 draws x 640..960
/// and GL y 180..360 (a probe pack of our own). Rows are GL's here, so that is rows 180..360.
#[test]
fn a_scaled_pass_draws_its_part_from_gl_s_corner() {
    assert_eq!(
        viewport([1280, 720], [0.25, 0.5, 0.25]),
        [640.0, 180.0, 320.0, 180.0]
    );
    assert_eq!(
        viewport([1280, 720], [0.5, 0.0, 0.0]),
        [0.0, 0.0, 640.0, 360.0]
    );
    // Whole pixels, at least one, and never past the buffer's edge.
    assert_eq!(viewport([5, 3], [0.5, 0.0, 0.0]), [0.0, 0.0, 2.0, 1.0]);
    assert_eq!(viewport([4, 4], [0.01, 0.0, 0.0]), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(
        viewport([100, 100], [1.0, 0.5, 0.75]),
        [50.0, 75.0, 50.0, 25.0]
    );
}

#[test]
fn pack_formats_map_to_renderable_ones() {
    let f = |name| texture_format(Format::parse(name).unwrap(), wgpu::Features::empty());
    assert_eq!(f("RGBA"), T::Rgba8Unorm);
    assert_eq!(f("RGB8"), T::Rgba8Unorm, "no three-channel formats in wgpu");
    assert_eq!(f("RG16F"), T::Rg16Float);
    assert_eq!(f("R32F"), T::R32Float);
    assert_eq!(f("RGB10_A2"), T::Rgb10a2Unorm);
    assert_eq!(f("R32UI"), T::R32Uint);
    assert_eq!(f("RGBA16"), T::Rgba16Float, "16-bit norm needs a feature");
    assert_eq!(f("RGBA8_SNORM"), T::Rgba16Float, "snorm is not renderable");
    assert_eq!(f("R11F_G11F_B10F"), T::Rgba16Float);
    let all = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
        | wgpu::Features::RG11B10UFLOAT_RENDERABLE;
    let g = |name| texture_format(Format::parse(name).unwrap(), all);
    assert_eq!(g("RGBA16"), T::Rgba16Unorm);
    assert_eq!(g("R11F_G11F_B10F"), T::Rg11b10Ufloat);
}

#[test]
fn float_targets_fall_back_when_not_renderable() {
    let all = |_| true;
    assert_eq!(renderable_float(T::Rgba32Float, all), T::Rgba32Float);
    // OpenGL without GL_EXT_color_buffer_float: only half floats render.
    let half_only = |f: T| matches!(f, T::R16Float | T::Rg16Float | T::Rgba16Float);
    assert_eq!(renderable_float(T::Rgba32Float, half_only), T::Rgba16Float);
    assert_eq!(renderable_float(T::Rg32Float, half_only), T::Rg16Float);
    assert_eq!(renderable_float(T::R32Float, half_only), T::R16Float);
    assert_eq!(renderable_float(T::Rgba16Float, half_only), T::Rgba16Float);
    // Neither renders: the 8-bit last resort.
    assert_eq!(renderable_float(T::Rgba32Float, |_| false), T::Rgba8Unorm);
    assert_eq!(renderable_float(T::Rg16Float, |_| false), T::Rgba8Unorm);
    // Other formats are never replaced.
    assert_eq!(renderable_float(T::Rgba8Uint, |_| false), T::Rgba8Uint);
    assert_eq!(
        renderable_float(T::Rgb10a2Unorm, |_| false),
        T::Rgb10a2Unorm
    );
}

#[test]
fn an_image_is_made_in_its_layout_qualifiers_format() {
    // `image.x = … RGB10_A2 …` read through `layout(rgb10_a2) image2D x`.
    for q in [
        "rgba32f",
        "rgba16f",
        "rg16f",
        "r11f_g11f_b10f",
        "r32f",
        "rgba16",
        "rgb10_a2",
        "rgba8",
        "rg8",
        "r16",
        "rgba16_snorm",
        "rgba8_snorm",
        "r8_snorm",
        "rgba32i",
        "r16i",
        "rgb10_a2ui",
        "rgba8ui",
        "r32ui",
    ] {
        let def = Format::parse(q).unwrap();
        assert_eq!(
            Some(crate::pack_images::format(def, true)),
            image_format(q),
            "{q}"
        );
    }
}

#[test]
fn samplers_read_custom_textures_of_their_type() {
    use wgpu::TextureSampleType as T;
    let float = T::Float { filterable: true };
    assert!(fits("sampler2D", float));
    assert!(fits("sampler3D", float));
    assert!(fits("isampler2D", T::Sint));
    assert!(fits("usampler1D", T::Uint));
    assert!(!fits("sampler2D", T::Uint));
    assert!(!fits("isampler2D", float));
    assert!(!fits("sampler2DShadow", float));
}

#[test]
fn sampler_names() {
    let main = |n| source(n, false, false, false);
    assert!(main("gaux1") == Source::Color(4));
    assert!(main("gdepthtex") == Source::Depth);
    assert!(main("depthtex1") == Source::Depth1);
    assert!(main("depthtex2") == Source::Depth2);
    assert!(main("normals") == Source::Normals);
    assert!(main("specular") == Source::Specular);
    assert!(main("noisetex") == Source::Noise);
    // Without the shadow pass, the shadow buffers are stand-ins.
    assert!(main("shadowcolor0") == Source::Dummy);
    assert!(main("shadowtex0") == Source::Dummy);
    // With it: shadowcolor n is buffer n of the shadow programs, `shadow` is shadowtex1
    // beside `watershadow` (6.3), and colortex is not theirs.
    assert!(source("shadowcolor", false, true, false) == Source::ShadowColor(0));
    assert!(source("shadowcolor1", true, true, false) == Source::Color(1));
    assert!(source("colortex1", true, true, false) == Source::Dummy);
    assert!(source("shadow", false, true, false) == Source::ShadowDepth(0, false));
    assert!(source("shadow", false, true, true) == Source::ShadowDepth(1, false));
    assert!(source("watershadow", false, true, true) == Source::ShadowDepth(0, false));
    assert!(source("shadowtex1HW", false, true, false) == Source::ShadowDepth(1, true));
}

/// A pack's pipelines are made together on several threads: each lands in its pass's slot,
/// and one wgpu refuses comes back by name (the pack is then built a program at a time).
#[test]
#[ignore = "daily"]
fn pipelines_compile_together_and_a_refused_one_is_named() {
    let _gpu = crate::test_gpu::lock();
    let Some((_adapter, device, _queue)) = optional_device() else {
        return;
    };
    let layout = device.create_pipeline_layout(&Default::default());
    let module = |source: &str| {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(source.into()),
        })
    };
    let job = |label: &str, module: wgpu::ShaderModule| {
        let slot = Slot::default();
        let job = ComputeJob {
            label: label.into(),
            layout: layout.clone(),
            module,
        };
        (slot.clone(), Job::Compute(slot, job))
    };
    let plain = "@compute @workgroup_size(1) fn main() {}";
    let go = std::sync::atomic::AtomicBool::new(false);
    let (slots, jobs): (Vec<_>, Vec<_>) = (0..12)
        .map(|i| job(&format!("composite{i}"), module(plain)))
        .unzip();
    // Cancelled before it starts: nothing is made (`new` then drops the pack).
    assert_eq!(compile(&device, &jobs, &true.into()), None);
    assert!(slots.iter().all(|s| s.get().is_none()));
    assert_eq!(compile(&device, &jobs, &go), None);
    assert!(slots.iter().all(|s| s.get().is_some()));
    // A binding the layout doesn't have: wgpu refuses the pipeline.
    let bound = "@group(0) @binding(0) var<storage, read_write> b: array<u32>;\n\
                 @compute @workgroup_size(1) fn main() { b[0] = 1u; }";
    let (_, bad) = job("deferred3", module(bound));
    let (slots, mut jobs): (Vec<_>, Vec<_>) =
        (0..4).map(|i| job(&format!("c{i}"), module(plain))).unzip();
    jobs.insert(2, bad);
    let (program, _why) = compile(&device, &jobs, &go).expect("the pipeline is refused");
    assert_eq!(program, "deferred3");
    // The refusal stops none of the others: they are all made.
    assert!(slots.iter().all(|s| s.get().is_some()));
}

/// #3525: naga's SPIR-V writer panicked on Photon's gbuffers programs as the pipeline was
/// made. Such a panic skips the pass, as a validation error does, and the device goes on.
#[test]
#[ignore = "daily"]
fn a_shader_compiler_panic_skips_the_pass() {
    let _gpu = crate::test_gpu::lock();
    let Some((_adapter, device, _queue)) = optional_device() else {
        return;
    };
    let failed = checked(&device, || -> wgpu::Buffer {
        panic!("internal error: entered unreachable code: Expression [89] is not cached!")
    });
    let error = failed.expect_err("the panic comes back as an error");
    assert!(error.contains("Expression [89] is not cached!"), "{error}");
    let buffer = |usage| {
        checked(&device, || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage,
                mapped_at_creation: false,
            })
        })
    };
    // The device still makes things, and its errors still come back.
    assert!(buffer(wgpu::BufferUsages::UNIFORM).is_ok());
    assert!(buffer(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::MAP_WRITE).is_err());
}

/// `texture.composite.colortex0` stands in for buffer 0 in composite programs until one of
/// them flips that buffer; later ones, and `final`, read the buffer (Super Duper Vanilla,
/// Bliss, SEUS PTGI HRR). Another stage's flips don't release it.
#[test]
#[ignore = "daily"]
fn a_custom_texture_over_a_buffer_ends_when_its_stage_flips_it() {
    let _gpu = crate::test_gpu::lock();
    let Some((adapter, device, _queue)) = optional_device() else {
        return;
    };
    let textures = [crate::pack_textures::PackTexture {
        stage: "composite".into(),
        name: "colortex0".into(),
        dimension: wgpu::TextureViewDimension::D2,
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        blur: false,
        clamp: false,
        png: false,
        image: crate::pack_textures::Image::Atlas,
    }];
    let interface = sunrise_shaders::Interface {
        uniform_size: 48,
        initial_values: vec![0; 48],
        uniforms: Vec::new(),
        samplers: vec![sunrise_shaders::Sampler {
            name: "colortex0".into(),
            ty: "sampler2D".into(),
            binding: 1,
        }],
        outputs: vec![0],
        draw: false,
        images: Vec::new(),
        buffers: Vec::new(),
    };
    let buffers = crate::pack_images::Buffers::new(
        &device,
        Default::default(),
        Default::default(),
        wgpu::ShaderStages::FRAGMENT,
    );
    let stand_ins = std::cell::RefCell::new(Vec::new());
    let mismatches = std::cell::RefCell::new(std::collections::HashSet::new());
    let pack = Resources {
        textures: &textures,
        images: &[],
        left_out: &[],
        buffers: &buffers,
        indices: None,
        gpu: Gpu::of(&adapter, device.features()),
        storable: 0,
        rgb: 0,
        packed: [None; BUFFERS],
        released: std::cell::Cell::new(("", 0)),
        stand_ins: &stand_ins,
        shadow_linear: 0,
        jobs: None,
        mod_depth_targets: &[],
        mod_uniforms: &[],
        display_uniforms: false,
        mod_uniform_mismatches: &mismatches,
        mod_depth_target: std::cell::Cell::new(None),
        depth_compare: std::cell::Cell::new(wgpu::CompareFunction::LessEqual),
        mod_vertex: Default::default(),
    };
    let reads = |group: Group| {
        let (_, samplers, ..) = bindings(
            device.features(),
            &interface,
            "composite",
            group,
            &[1],
            None,
            &[T::Rgba8Unorm; BUFFERS],
            None,
            &mut Vec::new(),
            &pack,
        )
        .expect("the pass is kept");
        samplers[0].source
    };
    assert!(reads(Group::Composite) == Source::Atlas);
    pack.released.set(("deferred", 1));
    assert!(reads(Group::Composite) == Source::Atlas);
    pack.released.set(("composite", 1));
    assert!(reads(Group::Composite) == Source::Color(0));
    assert!(reads(Group::Final) == Source::Color(0));
}

/// Iris 1.11.7 (a probe pack of ours, #9064) didn't bind a PNG under
/// `texture.<stage>.<name>` when `name` was a name of the pack's own, no sampler of the
/// game's; a raw texture there, a PNG over `gaux2` or `noisetex`, and a `customTexture` PNG
/// were bound.
#[test]
#[ignore = "daily"]
fn a_png_under_a_name_of_its_own_is_not_bound() {
    let _gpu = crate::test_gpu::lock();
    let Some((adapter, device, _queue)) = optional_device() else {
        return;
    };
    let texture = |stage: &str, name: &str, png: bool| crate::pack_textures::PackTexture {
        stage: stage.into(),
        name: name.into(),
        dimension: wgpu::TextureViewDimension::D2,
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        blur: !png,
        clamp: !png,
        png,
        image: crate::pack_textures::Image::Atlas,
    };
    let textures = [
        texture("composite", "tp", true),
        texture("composite", "tr", false),
        texture("composite", "gaux2", true),
        texture("composite", "noisetex", true),
        texture(sunrise_shaders::EVERY_STAGE, "cp", true),
    ];
    let names = ["tp", "tr", "gaux2", "noisetex", "cp"];
    let interface = sunrise_shaders::Interface {
        uniform_size: 48,
        uniforms: Vec::new(),
        samplers: names
            .iter()
            .zip(1..)
            .map(|(name, k)| sunrise_shaders::Sampler {
                name: (*name).into(),
                ty: "sampler2D".into(),
                binding: 2 * k - 1,
            })
            .collect(),
        outputs: vec![0],
        draw: false,
        images: Vec::new(),
        buffers: Vec::new(),
        initial_values: vec![0; 48],
    };
    let buffers = crate::pack_images::Buffers::new(
        &device,
        Default::default(),
        Default::default(),
        wgpu::ShaderStages::FRAGMENT,
    );
    let stand_ins = std::cell::RefCell::new(Vec::new());
    let mismatches = std::cell::RefCell::new(std::collections::HashSet::new());
    let pack = Resources {
        textures: &textures,
        images: &[],
        left_out: &[],
        buffers: &buffers,
        indices: None,
        gpu: Gpu::of(&adapter, device.features()),
        storable: 0,
        rgb: 0,
        released: std::cell::Cell::new(("", 0)),
        stand_ins: &stand_ins,
        shadow_linear: 0,
        jobs: None,
        packed: [None; BUFFERS],
        mod_depth_targets: &[],
        mod_uniforms: &[],
        display_uniforms: false,
        mod_uniform_mismatches: &mismatches,
        mod_depth_target: std::cell::Cell::new(None),
        depth_compare: std::cell::Cell::new(wgpu::CompareFunction::LessEqual),
        mod_vertex: Default::default(),
    };
    let (_, samplers, ..) = bindings(
        device.features(),
        &interface,
        "final",
        Group::Final,
        &[0],
        None,
        &[T::Rgba8Unorm; BUFFERS],
        None,
        &mut Vec::new(),
        &pack,
    )
    .expect("the pass is kept");
    let read: Vec<Source> = samplers.iter().map(|b| b.source).collect();
    assert!(
        read == [
            Source::Dummy,
            Source::Atlas,
            Source::Atlas,
            Source::Atlas,
            Source::Atlas
        ]
    );
}

/// Iris 1.11.7 runs a program whose image no `image.*` gives, and its loads read zero
/// (observed with a probe pack). Its pass is kept, the image bound to a stand-in of its
/// declared format, zero again before each pass even after one stored into it.
#[test]
#[ignore = "daily"]
fn an_image_without_an_entry_reads_zero() {
    let _gpu = crate::test_gpu::lock();
    use wgpu::TextureViewDimension as D;
    let Some((adapter, device, queue)) = optional_device() else {
        return;
    };
    let image = |name: &str, ty: &str, format: &str, binding| sunrise_shaders::Image {
        name: name.into(),
        ty: ty.into(),
        format: format.into(),
        access: String::new(),
        binding,
        before: None,
    };
    let interface = sunrise_shaders::Interface {
        uniform_size: 48,
        initial_values: vec![0; 48],
        uniforms: Vec::new(),
        samplers: Vec::new(),
        outputs: vec![0],
        draw: false,
        images: vec![
            image("mask", "uimage3D", "r32ui", 1),
            image("other", "uimage3D", "r32ui", 2),
            image("flat", "image2D", "rgba8", 3),
        ],
        buffers: Vec::new(),
    };
    let buffers = crate::pack_images::Buffers::new(
        &device,
        Default::default(),
        Default::default(),
        wgpu::ShaderStages::FRAGMENT,
    );
    let stand_ins = std::cell::RefCell::new(Vec::new());
    let mismatches = std::cell::RefCell::new(std::collections::HashSet::new());
    let pack = Resources {
        textures: &[],
        images: &[],
        left_out: &[],
        buffers: &buffers,
        indices: None,
        gpu: Gpu::of(&adapter, device.features()),
        storable: 0,
        rgb: 0,
        packed: [None; BUFFERS],
        released: std::cell::Cell::new(("", 0)),
        stand_ins: &stand_ins,
        shadow_linear: 0,
        jobs: None,
        mod_depth_targets: &[],
        mod_uniforms: &[],
        display_uniforms: false,
        mod_uniform_mismatches: &mismatches,
        mod_depth_target: std::cell::Cell::new(None),
        depth_compare: std::cell::Cell::new(wgpu::CompareFunction::LessEqual),
        mod_vertex: Default::default(),
    };
    let mut notes = Vec::new();
    let (.., images) = bindings(
        device.features(),
        &interface,
        "composite",
        Group::Composite,
        &[0],
        None,
        &[T::Rgba8Unorm; BUFFERS],
        None,
        &mut notes,
        &pack,
    )
    .expect("the pass is kept");
    assert!(
        images
            == [
                (1, Image::StandIn(T::R32Uint, D::D3)),
                (2, Image::StandIn(T::R32Uint, D::D3)),
                (3, Image::StandIn(T::Rgba8Unorm, D::D2)),
            ]
    );
    assert_eq!(notes.len(), 3, "{notes:?}");
    // One stand-in for each format and dimension.
    let made = crate::pack_images::stand_ins(&device, stand_ins.take());
    assert_eq!(made.len(), 2);
    let mask = crate::pack_images::stand_in(&made, T::R32Uint, D::D3).expect("made");
    let texture = mask.view.texture();
    queue.write_texture(
        texture.as_image_copy(),
        &7u32.to_le_bytes(),
        wgpu::TexelCopyBufferLayout::default(),
        texture.size(),
    );
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    mask.clear(&mut encoder);
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout::default(),
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    assert_eq!(&read.slice(..).get_mapped_range().unwrap()[..], &[0; 4]);
}

/// `blend.<program>` of a composite program blends its output onto what the buffer's side
/// holds (Iris 1.11.7, probed: 0.25 over 0.75 with SRC_ALPHA ONE_MINUS_SRC_ALPHA ONE ZERO and
/// a source alpha of 0.25 reads 0.625).
#[test]
#[ignore = "daily"]
fn a_composite_program_blends_onto_its_buffer() {
    let _gpu = crate::test_gpu::lock();
    let Some(adapter) = crate::test_gpu::adapter() else {
        return;
    };
    // A pass's pipeline layout has six bind group slots.
    let desc = wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    };
    let Some((device, queue)) = crate::test_gpu::optional_device_with(&adapter, &desc) else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("hz-composite-blend-{}", std::process::id()));
    let shaders = dir.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    let file = |name: &str, text: &str| std::fs::write(shaders.join(name), text).unwrap();
    file(
        "composite.vsh",
        "#version 120\nvoid main() { gl_Position = ftransform(); }\n",
    );
    file(
        "composite.fsh",
        "#version 120\n/* DRAWBUFFERS:3 */\n\
         void main() { gl_FragData[0] = vec4(0.25, 0.25, 1.0, 0.25); }\n",
    );
    let pack = sunrise_shaders::Pack::open(&dir).unwrap();
    let properties = sunrise_shaders::Properties::parse(
        "blend.composite = SRC_ALPHA ONE_MINUS_SRC_ALPHA ONE ZERO\n",
        &sunrise_shaders::Macros::default(),
    );
    let mut post = sunrise_shaders::load_post(
        &pack,
        "",
        &properties,
        &sunrise_shaders::Macros::standard(),
        &Default::default(),
    );
    std::fs::remove_dir_all(&dir).unwrap();
    let program = post.programs.remove(0);
    let buffers = crate::pack_images::Buffers::new(
        &device,
        Default::default(),
        Default::default(),
        wgpu::ShaderStages::FRAGMENT,
    );
    let stand_ins = std::cell::RefCell::new(Vec::new());
    let mismatches = std::cell::RefCell::new(std::collections::HashSet::new());
    let pack = Resources {
        textures: &[],
        images: &[],
        left_out: &[],
        buffers: &buffers,
        indices: None,
        gpu: Gpu::of(&adapter, device.features()),
        storable: 0,
        rgb: 0,
        packed: [None; BUFFERS],
        released: std::cell::Cell::new(("", 0)),
        stand_ins: &stand_ins,
        shadow_linear: 0,
        jobs: None,
        mod_depth_targets: &[],
        mod_uniforms: &[],
        display_uniforms: false,
        mod_uniform_mismatches: &mismatches,
        mod_depth_target: std::cell::Cell::new(None),
        depth_compare: std::cell::Cell::new(wgpu::CompareFunction::LessEqual),
        mod_vertex: Default::default(),
    };
    let draws = crate::pack_draw::DrawUniforms::new(&device);
    let mut notes = Vec::new();
    let pass = build_pass(
        &device,
        program,
        &[T::Rgba8Unorm; BUFFERS],
        None,
        T::Rgba8Unorm,
        &mut notes,
        None,
        &[],
        &draws.layout,
        &pack,
        false,
        false,
    )
    .unwrap_or_else(|| panic!("the pass builds: {notes:?}"));
    // colortex3 holds (0.75, 0.5, 0.25, 0.5) before the pass.
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 64,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: T::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let head = [64.0f32, 1.0, 1.0 / 64.0, 1.0];
    queue.write_buffer(&pass.ubo, 0, bytemuck::cast_slice(&head));
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pass.layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: pass.ubo.as_entire_binding(),
        }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let before = wgpu::Color {
        r: 0.75,
        g: 0.5,
        b: 0.25,
        a: 0.5,
    };
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[attach(
            &view,
            wgpu::Operations {
                load: wgpu::LoadOp::Clear(before),
                store: wgpu::StoreOp::Store,
            },
        )],
        ..Default::default()
    });
    {
        // Loaded, as `PackPost::render` attaches a buffer a composite pass writes.
        let load = wgpu::Operations {
            load: wgpu::LoadOp::Load,
            store: wgpu::StoreOp::Store,
        };
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[attach(&view, load)],
            ..Default::default()
        });
        rp.set_pipeline(pass.render());
        rp.set_bind_group(0, &group, &[]);
        rp.draw(0..6, 0..1);
    }
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout::default(),
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let texel: Vec<u8> = read.slice(..).get_mapped_range().unwrap()[..4].to_vec();
    // rgb = 0.25 × src + 0.75 × dst, alpha = the source's.
    let expected = [0.625, 0.4375, 0.4375, 0.25].map(|v: f32| v * 255.0);
    for (got, want) in texel.iter().zip(expected) {
        assert!(
            (f32::from(*got) - want).abs() <= 1.0,
            "{texel:?} vs {expected:?}"
        );
    }
}

/// The GLSL image format qualifiers (GLSL 4.60, 4.4.6.2).
const QUALIFIERS: [&str; 39] = [
    "rgba32f",
    "rgba16f",
    "rg32f",
    "rg16f",
    "r11f_g11f_b10f",
    "r32f",
    "r16f",
    "rgba16",
    "rgb10_a2",
    "rgba8",
    "rg16",
    "rg8",
    "r16",
    "r8",
    "rgba16_snorm",
    "rgba8_snorm",
    "rg16_snorm",
    "rg8_snorm",
    "r16_snorm",
    "r8_snorm",
    "rgba32i",
    "rgba16i",
    "rgba8i",
    "rg32i",
    "rg16i",
    "rg8i",
    "r32i",
    "r16i",
    "r8i",
    "rgba32ui",
    "rgba16ui",
    "rgb10_a2ui",
    "rgba8ui",
    "rg32ui",
    "rg16ui",
    "rg8ui",
    "r32ui",
    "r16ui",
    "r8ui",
];

/// A 16-bit normalized qualifier binds the 16-bit float texture of its channels in that
/// texture's format (#548, decision D-3), as `r11f_g11f_b10f` binds the `RGBA16F` stand-in;
/// every other differing pair of a qualifier and a texture format a qualifier names is refused.
#[test]
fn only_16_bit_normalized_qualifiers_bind_their_float_texture() {
    let formats: Vec<T> = QUALIFIERS
        .iter()
        .map(|q| image_format(q).expect(q))
        .collect();
    let mut taken = Vec::new();
    for (q, &declared) in QUALIFIERS.iter().zip(&formats) {
        for &texture in &formats {
            if declared != texture && rebinds(declared, texture) {
                taken.push((*q, texture));
            }
        }
    }
    assert_eq!(
        taken,
        [
            ("r11f_g11f_b10f", T::Rgba16Float),
            ("rgba16", T::Rgba16Float),
            ("rg16", T::Rg16Float),
            ("r16", T::R16Float),
            ("rgba16_snorm", T::Rgba16Float),
            ("rg16_snorm", T::Rg16Float),
            ("r16_snorm", T::R16Float),
        ]
    );
    // Never the reverse, nor between unorm and snorm.
    assert!(!rebinds(T::Rgba16Float, T::Rgba16Unorm));
    assert!(!rebinds(T::Rgba16Unorm, T::Rgba16Snorm));
}

/// The image `rebinds` binds as its float texture declares that format; one beside it of the
/// same declared type bound in its own format keeps it, and a refused pair is never retyped.
#[test]
fn a_rebound_image_declares_its_textures_format_alone() {
    let mut module = naga::front::glsl::Frontend::default()
        .parse(
            &naga::front::glsl::Options::from(naga::ShaderStage::Compute),
            "#version 450\nlayout(local_size_x = 1) in;\n\
             layout(set = 0, binding = 1, rgba16) uniform image2D a;\n\
             layout(set = 0, binding = 2, rgba16) uniform image2D b;\n\
             layout(set = 0, binding = 3, rgba8) uniform image2D c;\n\
             layout(set = 0, binding = 4, rg16_snorm) uniform image2D d;\n\
             void main() {\n\
             imageStore(a, ivec2(0), imageLoad(b, ivec2(0)) + imageLoad(c, ivec2(0)) \
             + imageLoad(d, ivec2(0)));\n\
             imageStore(d, ivec2(0), vec4(2.0));\n}\n",
        )
        .unwrap();
    let entry = |binding, format| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::ReadWrite,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    };
    storage_formats_of(
        &mut module,
        &[
            entry(1, T::Rgba16Float),
            entry(2, T::Rgba16Unorm),
            entry(3, T::Rgba16Float),
            entry(4, T::Rg16Float),
        ],
    );
    let declared = |binding| {
        let var = module
            .global_variables
            .iter()
            .find(|(_, v)| v.binding.is_some_and(|b| b.binding == binding))
            .expect("declared")
            .1;
        match module.types[var.ty].inner {
            naga::TypeInner::Image {
                class: naga::ImageClass::Storage { format, .. },
                ..
            } => format,
            ref other => panic!("{other:?}"),
        }
    };
    assert_eq!(declared(1), naga::StorageFormat::Rgba16Float);
    assert_eq!(declared(2), naga::StorageFormat::Rgba16Unorm);
    assert_eq!(declared(3), naga::StorageFormat::Rgba8Unorm);
    assert_eq!(declared(4), naga::StorageFormat::Rg16Float);
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("the retyped module is valid");
}

/// A rebound image gets a note naming its program, the image, its qualifier and the format it
/// binds; an image in its qualifier's format gets none; a refused pair refuses the program.
#[test]
fn a_rebound_image_is_noted_and_an_exact_one_is_not() {
    let rebound = image_binding(
        "composite70",
        "colorimg8",
        "rgba16",
        T::Rgba16Unorm,
        T::Rgba16Float,
    )
    .expect("bound")
    .expect("noted");
    for name in ["composite70", "`colorimg8`", "rgba16", "Rgba16Float"] {
        assert!(rebound.contains(name), "{name} in {rebound}");
    }
    assert_eq!(
        image_binding(
            "composite70",
            "colorimg8",
            "rgba16",
            T::Rgba16Unorm,
            T::Rgba16Unorm
        ),
        Ok(None)
    );
    assert_eq!(
        image_binding(
            "composite70",
            "colorimg8",
            "rgba16f",
            T::Rgba16Float,
            T::Rgba16Float
        ),
        Ok(None)
    );
    let refused = image_binding(
        "composite70",
        "colorimg5",
        "rgba8",
        T::Rgba8Unorm,
        T::Rgba16Unorm,
    )
    .expect_err("refused");
    assert!(refused.contains("`colorimg5`"), "{refused}");
}

/// A sampler of another class than its image's texture refuses the program with a note naming
/// the program, the image, the sampler's type and the texture's format (#5223, decision D-6);
/// one of the texture's class reads it.
#[test]
fn a_sampler_of_another_class_than_its_image_refuses_the_program() {
    let float = wgpu::TextureSampleType::Float { filterable: false };
    let uint = wgpu::TextureSampleType::Uint;
    let refused = sampled_image(
        "composite73_a",
        "img_rtwImportance2D",
        "usampler2D",
        "rtwImportance2D",
        T::R32Float,
        float,
    )
    .expect_err("refused");
    for name in [
        "composite73_a",
        "`img_rtwImportance2D`",
        "usampler2D",
        "R32Float",
    ] {
        assert!(refused.contains(name), "{name} in {refused}");
    }
    let read = |ty, format, sample_type| {
        sampled_image("composite73_a", "img", ty, "img", format, sample_type)
    };
    assert_eq!(read("sampler2D", T::R32Float, float), Ok(()));
    assert_eq!(read("usampler2D", T::R32Uint, uint), Ok(()));
    assert!(read("sampler2D", T::R32Uint, uint).is_err());
    assert!(read("isampler2D", T::R32Uint, uint).is_err());
}
