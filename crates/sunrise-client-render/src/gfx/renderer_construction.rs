use super::*;

impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        offscreen: bool,
        sprite_pixels: &[u8],
        sprite_size: u32,
        sprite_minimum_size: u32,
        layers: u32,
        sprite_mips: &[crate::mipmap::Settings],
        animations: &[SpriteAnimation],
        hud_atlas: (&[u8], u32, u32),
        hud_pages: Option<&crate::hud::HudAtlas>,
        glint: (&[u8], u32, u32),
        glint_armor: (&[u8], u32, u32),
        forcefield: (&[u8], u32),
        panorama_faces: Arc<panorama::Faces>,
        prepared: Option<PreparedTextures>,
        render_device: RendererDevice<'_>,
        api: GraphicsApi,
        translucent_batching: bool,
        (shader_pack, pack_starts_wide): (bool, bool),
        // The settings and the pack's `request` pick the frame (`output::resolve_frame`),
        // with what this window's display reported to the renderer before (`display`).
        settings: &crate::graphics::Graphics,
        request: output::FrameRequest,
        display: output::DisplayReading,
    ) -> Result<Self> {
        let previous = match &render_device {
            RendererDevice::Reuse(renderer) => Some(*renderer),
            _ => None,
        };
        let Opened {
            instance,
            surface,
            adapter,
            device,
            queue,
            device_lost,
            configured,
        } = match render_device {
            // Reload resources on the same device; replacing live Vulkan
            // devices can invalidate the driver's swapchain state on the same
            // window.
            RendererDevice::Reuse(renderer) => {
                describe_adapter(&renderer.adapter);
                check_layers(&renderer.adapter, layers)?;
                Opened {
                    surface: if offscreen {
                        None
                    } else {
                        let spare = renderer
                            .spare_surface
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        Some(match spare {
                            Some(surface) => surface,
                            None => renderer.instance.create_surface(window.clone())?,
                        })
                    },
                    instance: renderer.instance.clone(),
                    adapter: renderer.adapter.clone(),
                    device: renderer.device.clone(),
                    queue: renderer.queue.clone(),
                    device_lost: renderer.device_lost.clone(),
                    configured: None,
                }
            }
            RendererDevice::Opened(opened) => *opened,
            RendererDevice::New => Opened::for_window(&window, offscreen, api, layers).await?,
        };
        // Computed once: the sprite arrays, the shaders' constants and the
        // shader pack atlas all cut the sprites by it.
        let split = check_layers(&adapter, layers)?;
        let info = adapter.get_info();
        surface_setup::log_sprite_split(&info, split, layers);
        crate::graphics::suggest_reduced_far_detail(crate::graphics::lod_capped(&info));
        let size = window.inner_size();
        let max_side = device.limits().max_texture_dimension_2d;
        let configured = match (&surface, configured) {
            (Some(_), Some(configured)) => Some(configured),
            (Some(surface), None) => Some(set_up_surface(
                surface,
                &adapter,
                &device,
                &device_lost,
                surface.get_capabilities(&adapter),
                size,
                previous.map(|renderer| renderer.config.present_mode),
                previous.is_none(),
            )?),
            (None, _) => None,
        };
        let (config, present_modes, hdr_output) = if let Some((config, caps)) = configured {
            let hdr_output = hdr::output(&caps);
            if previous.is_none() {
                log::info!(
                    "Sunrise renderer: HDR output {}",
                    hdr_output.map_or("not offered by this display", hdr::Output::name)
                );
            }
            (config, caps.present_modes, hdr_output)
        } else {
            let format = wgpu::TextureFormat::Bgra8Unorm;
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                format,
                width: size.width.clamp(1, max_side),
                height: size.height.clamp(1, max_side),
                present_mode: wgpu::PresentMode::Immediate,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![format],
                color_space: wgpu::SurfaceColorSpace::default(),
            };
            let hdr_output = hdr::offscreen_output();
            if let Some(output) = hdr_output
                && previous.is_none()
            {
                log::info!(
                    "Sunrise renderer: HDR output {} offscreen (SUNRISE_HDR_OUTPUT)",
                    output.name()
                );
            }
            (config, Vec::new(), hdr_output)
        };
        let offscreen = surface
            .is_none()
            .then(|| Self::offscreen_target(&device, &config));
        let wide_ok = draws_wide_frame(
            device.features(),
            adapter.get_texture_format_features(WIDE_FRAME),
        );
        let float_ok = draws_frame(adapter.get_texture_format_features(LINEAR_FRAME));
        if request.linear_pack && !float_ok {
            log::warn!(
                "Sunrise renderer: the device can't draw a float frame; the shader API 0.2 pack runs on the 8-bit one"
            );
        }
        let coverage_ok = hud_coverage::supported(
            device.limits().max_color_attachments,
            adapter.get_downlevel_capabilities().flags,
            adapter.get_texture_format_features(hud_coverage::FORMAT),
        );
        let hdr_driver = surface.is_some().then(|| output::window_driver(&info));
        let output_caps = output::OutputCapabilities {
            hdr: hdr_output,
            wide_frame: wide_ok,
            float_frame: float_ok,
            hud_layer: coverage_ok,
            held: output::held(settings, hdr_driver.as_deref()),
        };
        let frame = output::resolve_frame(settings, request, output_caps, display, config.format);
        let (frame_format, hud_coverage, overlay_format) =
            (frame.format, frame.hud_layer, frame.overlay_format);
        let linear_light = frame.linear_light;
        if previous.is_none() {
            startup_step(format_args!("creating the pipelines"));
        }
        // A renderer on the same device takes the previous one's shader
        // modules, layouts and pipelines where they come out the same.
        let reuse = previous.map_or_else(Default::default, |r| r.reuse.clone());
        let generation = reuse.begin();
        let kept = reuse.on(&device);
        let pipelines_step = crate::spans::startup::step("renderer.pipelines_textures");
        let terrain_step = crate::spans::startup::step("renderer.terrain_pipelines");
        let gpu_timer = timing::GpuTimer::new(&device, &queue);
        let shader = kept.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sunrise terrain"),
            source: wgpu::ShaderSource::Wgsl(terrain_shader().into()),
        });
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let sprite_array = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        };
        let sprite_arrays = layers.div_ceil(split).max(1);
        if sprite_arrays > 1 {
            log::info!("Sunrise: {layers} block textures in {sprite_arrays} arrays");
        }
        let sprite_constants = [
            ("SPRITE_ARRAYS", f64::from(sprite_arrays)),
            ("SPRITE_SPLIT", f64::from(split)),
        ];
        let bindings = kept.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Sunrise terrain bindings"),
            entries: &[
                entry(
                    0,
                    vf,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                ),
                entry(
                    1 + 1,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    3,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    4,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(5, vf, sprite_array),
                entry(6, vf, sprite_array),
                entry(7, vf, sprite_array),
                entry(8, vf, sprite_array),
                entry(9, vf, sprite_array),
                entry(10, vf, sprite_array),
                entry(11, vf, sprite_array),
                entry(
                    12,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
            ],
        });
        let layout = kept.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Sunrise terrain layout"),
            bind_group_layouts: &[Some(&bindings)],
            ..Default::default()
        });
        let fluid_pipeline = fluid::pipeline(
            &device,
            Some(&layout),
            &shader,
            config.format.remove_srgb_suffix(),
            &sprite_constants,
        );
        let terrain_model_pipelines = terrain_model::pipelines(
            &device,
            Some(&layout),
            &shader,
            config.format.remove_srgb_suffix(),
            &sprite_constants,
        );
        let instance_attributes = wgpu::vertex_attr_array![5=>Float32x4];
        let terrain_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &TERRAIN_ATTRIBUTES,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 16,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attributes,
            }),
        ];
        let alpha_blend = wgpu::BlendState::ALPHA_BLENDING;
        let debug_line_attributes =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Unorm8x4,3=>Float32];
        let debug_line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 32,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &debug_line_attributes,
        })];
        // The solid terrain's fragment constants without HDR output's lighting, for the
        // frames HDR is off (`SolidPipeline`).
        let plain_constants = [
            sprite_constants[0],
            sprite_constants[1],
            ("HDR_LIGHTING", 0.0),
        ];
        // `hud`: the debug crosshair, drawn with the HUD in the overlays' format, with its
        // coverage or without.
        let pipeline_with = |constants: &[(&str, f64)],
                             layout: &wgpu::PipelineLayout,
                             label: &str,
                             vertex: &str,
                             fragment: &str,
                             buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                             topology: wgpu::PrimitiveTopology,
                             cull: Option<wgpu::Face>,
                             depth_write: bool,
                             depth_test: bool,
                             blend: Option<wgpu::BlendState>,
                             hud: Option<bool>| {
            let mut constants = constants.to_vec();
            // The debug crosshair draws with the HUD, in gamma.
            constants.extend(linear_blend::frame_constants(
                frame_format,
                linear_light && hud.is_none(),
            ));
            let desc = wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(if depth_test {
                        wgpu::CompareFunction::GreaterEqual
                    } else {
                        wgpu::CompareFunction::Always
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &constants,
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: if hud.is_some() {
                            overlay_format
                        } else {
                            frame_format
                        },
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            };
            if hud == Some(true) {
                hud_coverage::pipeline(
                    &device,
                    &desc,
                    wgpu::ShaderSource::Wgsl(terrain_shader().into()),
                )
                .expect("the debug crosshair's coverage")
            } else {
                kept.create_render_pipeline(&desc)
            }
        };
        let pipeline_in = |layout: &wgpu::PipelineLayout,
                           label: &str,
                           vertex: &str,
                           fragment: &str,
                           buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                           topology: wgpu::PrimitiveTopology,
                           cull: Option<wgpu::Face>,
                           depth_write: bool,
                           depth_test: bool,
                           blend: Option<wgpu::BlendState>| {
            pipeline_with(
                &sprite_constants,
                layout,
                label,
                vertex,
                fragment,
                buffers,
                topology,
                cull,
                depth_write,
                depth_test,
                blend,
                None,
            )
        };
        // A solid terrain pipeline (`cull` faces culled, depth written, no blend) without HDR
        // output's lighting, and with it where the display offers HDR output.
        let solid_in = |layout: &wgpu::PipelineLayout,
                        label: &str,
                        vertex: &str,
                        fragment: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        cull: wgpu::Face| {
            let build = |constants: &[(&str, f64)]| {
                pipeline_with(
                    constants,
                    layout,
                    label,
                    vertex,
                    fragment,
                    buffers,
                    wgpu::PrimitiveTopology::TriangleList,
                    Some(cull),
                    true,
                    true,
                    None,
                    None,
                )
            };
            SolidPipeline {
                plain: build(&plain_constants),
                hdr: hdr_output.is_some().then(|| build(&sprite_constants)),
            }
        };
        let pipeline = |label: &str,
                        vertex: &str,
                        fragment: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        topology: wgpu::PrimitiveTopology,
                        cull: Option<wgpu::Face>,
                        depth_write: bool,
                        depth_test: bool,
                        blend: Option<wgpu::BlendState>| {
            pipeline_in(
                &layout,
                label,
                vertex,
                fragment,
                buffers,
                topology,
                cull,
                depth_write,
                depth_test,
                blend,
            )
        };
        let tri = wgpu::PrimitiveTopology::TriangleList;
        let solid = solid_in(
            &layout,
            "Terrain solid",
            "vs_terrain",
            "fs_solid",
            &terrain_buffers,
            wgpu::Face::Back,
        );
        // `solid` in two passes (terrain.wgsl's `opaque_quad`, `mesher::Part`), for the
        // world pass without a shader pack: no cutout first, through a shader that never
        // discards, then the rest.
        let solid_opaque = solid_in(
            &layout,
            "Terrain solid opaque",
            "vs_terrain_opaque",
            "fs_opaque",
            &terrain_buffers,
            wgpu::Face::Back,
        );
        let solid_cutout = solid_in(
            &layout,
            "Terrain solid cutout",
            "vs_terrain_cutout",
            "fs_solid",
            &terrain_buffers,
            wgpu::Face::Back,
        );
        // The water mirror's camera sees the world from the other side of the plane, and a
        // mirrored world view's picture is flipped (`world_views::Flipped`): each turns the
        // winding round, so their terrain culls the front.
        let front = |label, fragment, blend| {
            pipeline(
                label,
                "vs_terrain",
                fragment,
                &terrain_buffers,
                tri,
                Some(wgpu::Face::Front),
                true,
                true,
                blend,
            )
        };
        let solid_mirror = front("Terrain solid (mirrored)", "fs_solid_mirror", None);
        let flipped = solid_in(
            &layout,
            "Terrain solid (flipped)",
            "vs_terrain",
            "fs_solid",
            &terrain_buffers,
            wgpu::Face::Front,
        );
        let alpha = Some(alpha_blend);
        let translucent_flipped = front("Terrain translucent (flipped)", "fs_translucent", alpha);
        let translucent = pipeline(
            "Terrain translucent",
            "vs_terrain",
            "fs_translucent",
            &terrain_buffers,
            tri,
            Some(wgpu::Face::Back),
            true,
            true,
            Some(alpha_blend),
        );
        let translucent_mark = pipeline(
            "Terrain translucent HDR mark",
            "vs_terrain",
            "fs_translucent_mark",
            &terrain_buffers,
            tri,
            Some(wgpu::Face::Back),
            false,
            true,
            Some(flat::ALPHA_ONLY),
        );
        let sky = pipeline(
            "Sky",
            "vs_sky",
            "fs_sky",
            &[],
            tri,
            None,
            false,
            false,
            None,
        );
        let sky_base = pipeline(
            "Sky base",
            "vs_sky",
            "fs_sky_base",
            &[],
            tri,
            None,
            false,
            false,
            None,
        );
        let sunrise_attributes = wgpu::vertex_attr_array![0=>Float32x4, 1=>Float32];
        let sunrise_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 20,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &sunrise_attributes,
        })];
        let sunrise_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            // The sky's HDR alpha is the destination's; the fan only tints it.
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let sunrise_pipeline = pipeline(
            "Sunrise fan",
            "vs_sunrise",
            "fs_sunrise",
            &sunrise_buffers,
            tri,
            None,
            false,
            false,
            Some(sunrise_blend),
        );
        let dark_disc = pipeline(
            "Dark disc",
            "vs_sky",
            "fs_dark_disc",
            &[],
            tri,
            None,
            false,
            false,
            Some(alpha_blend),
        );
        let depth_reset = kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Hand depth reset"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_depth_reset"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &sprite_constants,
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format: frame_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let lines = pipeline(
            "Block outline",
            "vs_line",
            "fs_line",
            &debug_line_buffers,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
            true,
            true,
            Some(alpha_blend),
        );
        // RenderTypes.secondaryBlockOutline: the black stroke under a high contrast outline.
        let lines_secondary = pipeline(
            "Block outline secondary",
            "vs_line",
            "fs_line",
            &debug_line_buffers,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
            false,
            true,
            Some(alpha_blend),
        );
        // RenderPipelines.LINES: DepthStencilState.DEFAULT, so an opaque hitbox or
        // chunk border keeps the water and clouds drawn after it behind it.
        let debug_lines_opaque = pipeline(
            "Debug lines opaque",
            "vs_debug_line",
            "fs_debug_line",
            &debug_line_buffers,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
            true,
            true,
            Some(alpha_blend),
        );
        let debug_lines = pipeline(
            "Debug lines",
            "vs_debug_line",
            "fs_debug_line",
            &debug_line_buffers,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
            false,
            true,
            Some(alpha_blend),
        );
        let debug_lines_on_top = pipeline(
            "Debug lines on top",
            "vs_debug_line",
            "fs_debug_line",
            &debug_line_buffers,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
            false,
            false,
            Some(alpha_blend),
        );
        // The debug crosshair with the HUD: with its coverage on the layered frame, and in the
        // overlays' format where that isn't the frame's.
        let crosshair = |coverage| {
            pipeline_with(
                &sprite_constants,
                &layout,
                "3d crosshair",
                "vs_debug_line",
                "fs_debug_line",
                &debug_line_buffers,
                wgpu::PrimitiveTopology::TriangleStrip,
                None,
                false,
                false,
                Some(alpha_blend),
                Some(coverage),
            )
        };
        let debug_crosshair = hud_coverage.then(|| crosshair(true));
        let overlay_crosshair =
            (overlay_format != frame_format || linear_light).then(|| crosshair(false));
        drop(terrain_step);
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Sunrise globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hand_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("First-person hand globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let crosshair_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Debug crosshair globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mirror_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Water mirror globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let prepared = match prepared {
            Some(prepared) => prepared.resplit(layers, split),
            None => PreparedTextures::with_split(
                sprite_pixels,
                sprite_size,
                layers,
                sprite_mips,
                forcefield,
                split,
            ),
        };
        let mut mip_levels = 1;
        let gl = adapter.get_info().backend == wgpu::Backend::Gl;
        // Keep every colour layer's companion, split by the same device limit. Native
        // material stages read the first array; terrain packs stitch all of them.
        let pbr_maps = prepared
            .pbr
            .map(|(normals, specular, levels, map_layers, size)| {
                let layer_bytes = normals.len() / map_layers as usize;
                (0..map_layers.div_ceil(split))
                    .map(|array| {
                        let first = array * split;
                        let used = (map_layers - first).min(split);
                        let count = array_layers(used, gl);
                        [("LabPBR normals", &normals), ("LabPBR specular", &specular)].map(
                            |(label, data)| {
                                let start = first as usize * layer_bytes;
                                let mut data =
                                    data[start..start + used as usize * layer_bytes].to_vec();
                                data.resize(layer_bytes * count as usize, 0);
                                device.create_texture_with_data(
                                    &queue,
                                    &wgpu::TextureDescriptor {
                                        label: Some(label),
                                        size: wgpu::Extent3d {
                                            width: size,
                                            height: size,
                                            depth_or_array_layers: count,
                                        },
                                        mip_level_count: levels,
                                        sample_count: 1,
                                        dimension: wgpu::TextureDimension::D2,
                                        format: wgpu::TextureFormat::Rgba8Unorm,
                                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                                            | wgpu::TextureUsages::COPY_SRC
                                            | wgpu::TextureUsages::COPY_DST,
                                        view_formats: &[],
                                    },
                                    wgpu::util::TextureDataOrder::LayerMajor,
                                    &data,
                                )
                            },
                        )
                    })
                    .collect()
            });
        let sprite_textures: Vec<wgpu::Texture> = prepared
            .sprites
            .into_iter()
            .enumerate()
            .map(|(array, (mut mips, levels))| {
                let first = array as u32 * split;
                let used = layers.saturating_sub(first).clamp(1, split);
                let count = array_layers(used, gl);
                // The extra layers are empty.
                mips.resize(mips.len() / used as usize * count as usize, 0);
                mip_levels = levels;
                device.create_texture_with_data(
                    &queue,
                    &wgpu::TextureDescriptor {
                        label: Some("Block sprites"),
                        size: wgpu::Extent3d {
                            width: sprite_size,
                            height: sprite_size,
                            depth_or_array_layers: count,
                        },
                        mip_level_count: levels,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_DST
                            // Shader packs' atlas copies them (pack_atlas.rs).
                            | wgpu::TextureUsages::COPY_SRC
                            | wgpu::TextureUsages::STORAGE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    &mips,
                )
            })
            .collect();
        let material_animations = animations
            .iter()
            .filter(|a| a.has_materials())
            .cloned()
            .collect();
        let animations = animate::SpriteAnimator::new(
            &device,
            animations,
            sprite_size,
            sprite_mips,
            &sprite_textures,
            split,
            animate::SpriteAnimator::compute(&adapter, &device),
        );
        let active_layer_words = animations.active_layer_words();
        let sprite_unused = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Block sprites (unused)"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: array_layers(1, gl),
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        let sprite_views = sprite_views(&sprite_textures, &sprite_unused, None);
        let sprite_sampler = device.create_sampler(&sprite_sampler_descriptor(1));
        let sprite_nearest = device.create_sampler(&SPRITE_NEAREST);
        let lightmap = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Lightmap"),
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            // A pack's `core/lightmap` draws into it, and its pixels are read back for F3.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let lightmap_view = lightmap.create_view(&Default::default());
        let lightmap_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Lightmap"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let emission_stats =
            hdr_stats::EmissionStats::new(&device, split, sprite_textures.len() as u32);
        let group = terrain_group(
            &device,
            "Sunrise terrain resources",
            &bindings,
            &globals,
            &sprite_views,
            &sprite_sampler,
            &lightmap_view,
            &lightmap_sampler,
            &emission_stats.view,
        );
        let fluid_sampler = device.create_sampler(&fluid::SAMPLER);
        let fluid_group = terrain_group(
            &device,
            "Still fluid resources",
            &bindings,
            &globals,
            &sprite_views,
            &fluid_sampler,
            &lightmap_view,
            &lightmap_sampler,
            &emission_stats.view,
        );
        let terrain_model_sampler = device.create_sampler(&terrain_model::MOVING_BLOCK_SAMPLER);
        let terrain_model_group = terrain_group(
            &device,
            "Native model resources",
            &bindings,
            &globals,
            &sprite_views,
            &terrain_model_sampler,
            &lightmap_view,
            &lightmap_sampler,
            &emission_stats.view,
        );
        let crosshair_group = terrain_group(
            &device,
            "Debug crosshair resources",
            &bindings,
            &crosshair_globals,
            &sprite_views,
            &sprite_sampler,
            &lightmap_view,
            &lightmap_sampler,
            &emission_stats.view,
        );
        let mirror_group = terrain_group(
            &device,
            "Water mirror resources",
            &bindings,
            &mirror_globals,
            &sprite_views,
            &sprite_sampler,
            &lightmap_view,
            &lightmap_sampler,
            &emission_stats.view,
        );
        let index_quads = 1 << 16;
        let indices = Self::quad_indices(&device, index_quads);
        let instance_capacity = 4096;
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Section origins"),
            size: instance_capacity * 16,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let prepared_instance_capacity = 64;
        let prepared_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Prepared model instances"),
            size: prepared_instance_capacity * std::mem::size_of::<PreparedModelInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let outline = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Outline"),
            size: 32 * 2 * MAX_OUTLINE_EDGES,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let debug_line_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Debug lines"),
            size: 32 * 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (atlas_pixels, atlas_width, atlas_height) = hud_atlas;
        let limit = device.limits().max_texture_dimension_2d;
        if atlas_width.max(atlas_height) > limit {
            return Err(format!(
                "HUD atlas {atlas_width}x{atlas_height} exceeds device texture limit {limit}"
            )
            .into());
        }
        let atlas = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("Shared atlas"),
                size: wgpu::Extent3d {
                    width: atlas_width,
                    height: atlas_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                // COPY_SRC: a shader pack's weather copies the rain and snow sprites out.
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            atlas_pixels,
        );
        let atlas_view = atlas.create_view(&Default::default());
        // vanilla's `GlintSampler`: enchanted_glint_item as its own texture,
        // repeated per pixel (`clamp: false`), linear without mipmaps
        // (`blur: true`).
        let (glint_pixels, glint_width, glint_height) = glint;
        let glint_texture = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("Enchanted glint"),
                size: wgpu::Extent3d {
                    width: glint_width,
                    height: glint_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            glint_pixels,
        );
        let glint_view = glint_texture.create_view(&Default::default());
        let glint_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Glint"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // ARMOR_ENTITY_GLINT_TEXTURING's sampler is vanilla's GlintSampler: the
        // same repeat and linear filtering as the item glint.
        let (glint_armor_pixels, glint_armor_width, glint_armor_height) = glint_armor;
        let glint_armor_texture = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("Enchanted armor glint"),
                size: wgpu::Extent3d {
                    width: glint_armor_width,
                    height: glint_armor_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            glint_armor_pixels,
        );
        let glint_armor_view = glint_armor_texture.create_view(&Default::default());
        let mut hud = HudRenderer::new(
            &device,
            overlay_format,
            &atlas_view,
            &glint_view,
            &glint_armor_view,
            &lightmap_view,
            &glint_sampler,
            hud_coverage,
            hud_pages.map_or_else(Vec::new, |atlas| {
                atlas
                    .registered_pages
                    .iter()
                    .map(|page| crate::hud::upload_atlas_page(&device, &queue, page))
                    .collect()
            }),
        )?;
        hud.atlas_texture = Some(atlas.clone());
        // On the same device the previous renderer's dynamic textures are still valid.
        if let Some(previous) = previous {
            hud.share_dynamic(previous.hud.dynamic().clone());
        }
        let entity_step = crate::spans::startup::step("renderer.entity_pipelines");
        let entity_shader = kept.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sunrise entities"),
            source: wgpu::ShaderSource::Wgsl(entity_shader().into()),
        });
        let static_entity_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sunrise static block entities"),
            source: wgpu::ShaderSource::Wgsl(static_entity_shader().into()),
        });
        let entity_bindings = kept.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Entity bindings"),
            entries: &entity_binding_entries(),
        });
        let static_entity_bindings =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Static block-entity transform"),
                entries: &[entry(
                    0,
                    wgpu::ShaderStages::VERTEX,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                )],
            });
        let weather = weather::Weather::new(
            &device,
            (frame_format, linear_light),
            &entity_bindings,
            &entity_shader,
        );
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Atlas"),
            ..Default::default()
        });
        let entity_group_for = |view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Entity resources"),
                layout: &entity_bindings,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: globals.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&lightmap_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&lightmap_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&glint_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::Sampler(&glint_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(&glint_armor_view),
                    },
                ],
            })
        };
        let entity_group = entity_group_for(&atlas_view);
        let dynamic_entity = dynamic_entity::DynamicEntity::new(
            &device,
            entity_bindings.clone(),
            globals.clone(),
            (lightmap_view.clone(), lightmap_sampler.clone()),
            (glint_view.clone(), glint_sampler.clone()),
            glint_armor_view.clone(),
        );
        let screen_page_groups = hud
            .atlas_page_views()
            .map(|view| view.map(entity_group_for))
            .collect();
        let static_entity_transform =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Static block-entity camera position"),
                contents: bytemuck::bytes_of(&StaticEntityUniform {
                    camera_block: [0; 4],
                    camera_fraction: [0.0; 4],
                }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let static_entity_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Static block-entity transform"),
            layout: &static_entity_bindings,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: static_entity_transform.as_entire_binding(),
            }],
        });
        let hand_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("First-person hand resources"),
            layout: &entity_bindings,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: hand_globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&lightmap_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&lightmap_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&glint_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&glint_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&glint_armor_view),
                },
            ],
        });
        let entity_layout = kept.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Entity layout"),
            bind_group_layouts: &[Some(&entity_bindings)],
            ..Default::default()
        });
        let static_entity_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Static block-entity layout"),
            bind_group_layouts: &[Some(&entity_bindings), Some(&static_entity_bindings)],
            ..Default::default()
        });
        let cloud_bindings = kept.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cloud bindings"),
            entries: &[entry(
                0,
                wgpu::ShaderStages::VERTEX,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            )],
        });
        let cloud_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cloud transforms and colors"),
            size: std::mem::size_of::<crate::clouds::Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cloud_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Cloud resources"),
            layout: &cloud_bindings,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cloud_uniform.as_entire_binding(),
            }],
        });
        let cloud_layout = kept.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cloud layout"),
            bind_group_layouts: &[Some(&entity_bindings), Some(&cloud_bindings)],
            ..Default::default()
        });
        let cloud_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cloud vertices"),
            size: 4 * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entity_vertex = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ENTITY_ATTRIBUTES,
        })];
        let static_entity_origin_attributes = wgpu::vertex_attr_array![7=>Sint32x3];
        let static_entity_vertex = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &ENTITY_ATTRIBUTES,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<[i32; 3]>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &static_entity_origin_attributes,
            }),
        ];
        let entity_pipeline_with = entity_pipeline_factory(
            &kept,
            &entity_layout,
            &cloud_layout,
            &entity_shader,
            &entity_vertex,
            frame_format,
            linear_light,
        );
        let entity_pipeline = |label, cull_mode, depth_write, compare, bias, blend, clouds| {
            entity_pipeline_with(EntityPipelineOptions {
                label,
                cull_mode,
                depth_write,
                compare,
                bias,
                blend,
                clouds,
                vertex: "vs_entity",
                fragment: "fs_entity",
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        // RenderPipelines.WATER_MASK: the boat's water patch writes depth only.
        let water_mask_pipeline = entity_pipeline_with(EntityPipelineOptions {
            label: "Water mask",
            cull_mode: None,
            depth_write: true,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias: Default::default(),
            blend: alpha_blend,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_water_mask",
            write_mask: wgpu::ColorWrites::empty(),
        });
        let crumbling_pipeline = entity_pipeline(
            "Breaking cracks",
            Some(wgpu::Face::Back),
            false,
            wgpu::CompareFunction::GreaterEqual,
            wgpu::DepthBiasState {
                constant: 10,
                slope_scale: 1.0,
                clamp: 0.0,
            },
            CRUMBLING_BLEND,
            false,
        );
        // RenderPipelines.ENTITY_SHADOW: translucent, back-face culled, depth
        // test without writes. The quads lie exactly on the block tops like
        // vanilla's, facing +Y so looking up from below culls them, but
        // terrain and shadows are separate draw paths, so their fp32 depth
        // interpolations disagree by a few dozen ulps of the fragment depth
        // and the shadow's rim z-fights with the block top (speckle that
        // re-rolls as the camera drifts a subpixel). A constant bias of 128
        // ulps covers that spread at any resolution while staying far below
        // the 0.001 a model's feet sit higher, so shadows never bleed onto
        // the feet.
        let shadow_blob_pipeline = entity_pipeline_with(EntityPipelineOptions {
            label: "Entity shadows",
            cull_mode: Some(wgpu::Face::Back),
            depth_write: false,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias: wgpu::DepthBiasState {
                constant: 128,
                slope_scale: 0.015,
                clamp: 0.0,
            },
            blend: alpha_blend,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_entity_shadow",
            write_mask: wgpu::ColorWrites::ALL,
        });
        // RenderPipelines.ITEM_TRANSLUCENT: blended, depth tested and, like
        // every vanilla pipeline that does not turn it off, depth writing.
        // `LevelRenderer.executeClassicTransparency` draws the translucent
        // features before the translucent terrain, so without the writes
        // ice or water behind a dropped slime block or glass item would blend
        // over it and the item would all but vanish (#7031, #7033).
        let translucent_item_pipeline = entity_pipeline(
            "Translucent items",
            None,
            true,
            wgpu::CompareFunction::GreaterEqual,
            Default::default(),
            alpha_blend,
            false,
        );
        let cloud_pipeline = entity_pipeline(
            "Clouds",
            Some(wgpu::Face::Back),
            true,
            wgpu::CompareFunction::GreaterEqual,
            Default::default(),
            alpha_blend,
            true,
        );
        let hdr_cloud_mark_pipeline = entity_pipeline_with(EntityPipelineOptions {
            label: "HDR cloud mark",
            cull_mode: Some(wgpu::Face::Back),
            depth_write: false,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias: Default::default(),
            blend: wgpu::BlendState::REPLACE,
            clouds: true,
            vertex: "vs_entity",
            fragment: "fs_hdr_cloud_mark",
            write_mask: wgpu::ColorWrites::ALPHA,
        });
        // RenderPipelines.TEXT_SEE_THROUGH: translucent, no depth test or
        // writes, culled like every world text pipeline (TEXT_SNIPPET keeps
        // RenderPipeline.Builder's default): a name tag or text display hides
        // its back.
        let see_through_pipeline = entity_pipeline_with(EntityPipelineOptions {
            label: "See-through text",
            cull_mode: Some(wgpu::Face::Back),
            depth_write: false,
            compare: wgpu::CompareFunction::Always,
            bias: Default::default(),
            blend: alpha_blend,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_entity_unmarked",
            write_mask: wgpu::ColorWrites::ALL,
        });
        // RenderPipelines.TEXT_POLYGON_OFFSET: a display's text/background or a sign's
        // outlined fill wins against the surface it lies on (bias 1, 10), from the front only.
        let text_bias = wgpu::DepthBiasState {
            constant: 10,
            slope_scale: 1.0,
            clamp: 0.0,
        };
        let text_polygon_offset_pipeline = entity_pipeline_with(EntityPipelineOptions {
            label: "Polygon offset text",
            cull_mode: Some(wgpu::Face::Back),
            depth_write: true,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias: text_bias,
            blend: alpha_blend,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_entity_text",
            write_mask: wgpu::ColorWrites::ALL,
        });
        // Marked HDR targets store a glowing glyph's tier in alpha. Match the colour pass's
        // exact depth, bias and culling in each mode so discarded or occluded glyphs never mark.
        let text_mark_pipelines = [wgpu::DepthBiasState::default(), text_bias].map(|bias| {
            entity_pipeline_with(EntityPipelineOptions {
                label: "HDR text mark",
                cull_mode: Some(wgpu::Face::Back),
                depth_write: false,
                compare: wgpu::CompareFunction::Equal,
                bias,
                blend: wgpu::BlendState::REPLACE,
                clouds: false,
                vertex: "vs_entity",
                fragment: "fs_text_hdr_mark",
                write_mask: wgpu::ColorWrites::ALPHA,
            })
        });
        let world_text_pipeline = world_text::pipeline(
            &device,
            Some(&entity_layout),
            &entity_shader,
            (frame_format, linear_light),
        );
        // RenderPipelines.ENTITY_SOLID: a surface of an entity that is coplanar with
        // terrain wins the tie in vanilla (the boat's floor over the trapdoor on the
        // ground it is stopped by, #5950). Terrain and entities are separate draw
        // paths here, so their fp32 depths of the same plane differ by a few dozen
        // ulps and the tie came out either way, as a jagged strip of the trapdoor
        // through the hull's floor. The same constant as the shadows' covers that
        // spread; a model's feet sit 0.001 above the ground, far beyond it. HDR output's
        // marks on what it draws take it too (`hdr_entity_mark_pipeline`).
        let entity_bias = wgpu::DepthBiasState {
            constant: 128,
            slope_scale: 0.0,
            clamp: 0.0,
        };
        let entity_pipeline = entity_pipeline(
            "Entities",
            None,
            true,
            wgpu::CompareFunction::GreaterEqual,
            entity_bias,
            alpha_blend,
            false,
        );
        // `RenderPipelines.BANNER_PATTERN`: blended without depth writes or alpha cutout.
        let banner_pattern_pipeline = entity_pipeline_with(EntityPipelineOptions {
            label: "Banner pattern",
            cull_mode: None,
            depth_write: false,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias: entity_bias,
            blend: alpha_blend,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_banner_pattern",
            write_mask: wgpu::ColorWrites::ALL,
        });
        // `RenderPipelines.GLINT`: equal depth against the entity pass, no depth writes,
        // `BlendFunction.GLINT` RGB addition (squared in `fs_glint`), dst alpha untouched.
        let glint_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let glint_only_pipeline = entity_pipeline_with(EntityPipelineOptions {
            label: "Glint overlay",
            cull_mode: None,
            depth_write: false,
            compare: wgpu::CompareFunction::Equal,
            bias: entity_bias,
            blend: glint_blend,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_glint",
            write_mask: wgpu::ColorWrites::ALL,
        });
        let particle_pipelines = particle_draws::ParticlePipelines::new(entity_pipeline_with);
        // The hand as `entity_pipeline` draws it, in the overlays' format.
        let overlay_hand = (overlay_format != frame_format || linear_light).then(|| {
            entity_pipeline_factory(
                &kept,
                &entity_layout,
                &cloud_layout,
                &entity_shader,
                &entity_vertex,
                overlay_format,
                false,
            )(EntityPipelineOptions {
                label: "First-person hand",
                cull_mode: None,
                depth_write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
                bias: entity_bias,
                blend: alpha_blend,
                clouds: false,
                vertex: "vs_entity",
                fragment: "fs_entity",
                write_mask: wgpu::ColorWrites::ALL,
            })
        });
        let overlay_hand_banner_pattern =
            (overlay_format != frame_format || linear_light).then(|| {
                entity_pipeline_factory(
                    &kept,
                    &entity_layout,
                    &cloud_layout,
                    &entity_shader,
                    &entity_vertex,
                    overlay_format,
                    false,
                )(EntityPipelineOptions {
                    label: "First-person hand banner pattern",
                    cull_mode: None,
                    depth_write: false,
                    compare: wgpu::CompareFunction::GreaterEqual,
                    bias: entity_bias,
                    blend: alpha_blend,
                    clouds: false,
                    vertex: "vs_entity",
                    fragment: "fs_banner_pattern",
                    write_mask: wgpu::ColorWrites::ALL,
                })
            });
        let overlay_hand_glint = (overlay_format != frame_format || linear_light).then(|| {
            entity_pipeline_factory(
                &kept,
                &entity_layout,
                &cloud_layout,
                &entity_shader,
                &entity_vertex,
                overlay_format,
                false,
            )(EntityPipelineOptions {
                label: "First-person hand glint",
                cull_mode: None,
                depth_write: false,
                compare: wgpu::CompareFunction::Equal,
                bias: entity_bias,
                blend: glint_blend,
                clouds: false,
                vertex: "vs_entity",
                fragment: "fs_glint",
                write_mask: wgpu::ColorWrites::ALL,
            })
        });
        let static_entity_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Static block entities"),
                layout: Some(&static_entity_layout),
                vertex: wgpu::VertexState {
                    module: &static_entity_shader,
                    entry_point: Some("vs_static_entity"),
                    compilation_options: Default::default(),
                    buffers: &static_entity_vertex,
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &static_entity_shader,
                    entry_point: Some("fs_entity_unmarked"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &linear_blend::frame_constants(frame_format, linear_light),
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: frame_format,
                        // fs_entity's discard handles binary cutout alpha. Any
                        // sprite with fractional alpha stays on the blended
                        // dynamic entity path.
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        let energy_swirl_pipeline = entity_pipeline_with(energy_swirl_options(false));
        // HDR output's mark on light-giving particles (entity.wgsl fs_hdr_mark).
        let hdr_mark_pipeline = entity_pipeline_with(EntityPipelineOptions {
            label: "HDR particle mark",
            cull_mode: None,
            depth_write: false,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias: Default::default(),
            blend: wgpu::BlendState::REPLACE,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_hdr_mark",
            write_mask: wgpu::ColorWrites::ALPHA,
        });
        let hdr_entity_mark_pipeline = entity_pipeline_with(EntityPipelineOptions {
            label: "HDR entity mark",
            cull_mode: None,
            depth_write: false,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias: entity_bias,
            blend: wgpu::BlendState::REPLACE,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_hdr_mark",
            write_mask: wgpu::ColorWrites::ALPHA,
        });
        let hdr_hand_unmark_pipeline = entity_pipeline_with(EntityPipelineOptions {
            label: "HDR hand unmark",
            cull_mode: None,
            depth_write: false,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias: entity_bias,
            blend: wgpu::BlendState::REPLACE,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_hdr_hand_unmark",
            write_mask: wgpu::ColorWrites::ALPHA,
        });
        // Attribute 7: 6 is the entity vertex buffer's glint UV.
        let lightning_attributes = wgpu::vertex_attr_array![7=>Float32x3];
        let lightning_vertex = [
            entity_vertex[0].clone(),
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<[f32; 3]>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &lightning_attributes,
            }),
        ];
        // BlendFunction.LIGHTNING: src alpha, one; no depth writes.
        let additive_descriptor = wgpu::RenderPipelineDescriptor {
            label: Some("Additive entities"),
            layout: Some(&entity_layout),
            vertex: wgpu::VertexState {
                module: &entity_shader,
                entry_point: Some("vs_entity"),
                compilation_options: Default::default(),
                buffers: &entity_vertex,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &entity_shader,
                entry_point: Some("fs_celestial"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &linear_blend::frame_constants(frame_format, linear_light),
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format: frame_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        };
        let additive_pipeline = kept.create_render_pipeline(&additive_descriptor);
        // `RenderPipelines.DRAGON_RAYS` uses DepthStencilState.DEFAULT:
        // unlike eyes, a ray hides the faces and rays behind it.
        let dragon_rays_pipeline = kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Dragon death rays"),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..additive_descriptor.primitive
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                depth_write_enabled: Some(true),
                ..additive_descriptor.depth_stencil.clone().unwrap()
            }),
            ..additive_descriptor.clone()
        });
        // HDR output's mark on lightning bolts: fs_hdr_mark over the bolts' own vertex layout,
        // as hdr_mark_pipeline marks entities and particles (only alpha written).
        let mark_targets = [Some(wgpu::ColorTargetState {
            format: frame_format,
            blend: Some(wgpu::BlendState::REPLACE),
            write_mask: wgpu::ColorWrites::ALPHA,
        })];
        let hdr_mark_lightning_pipeline =
            kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("HDR lightning mark"),
                vertex: wgpu::VertexState {
                    entry_point: Some("vs_lightning"),
                    buffers: &lightning_vertex,
                    ..additive_descriptor.vertex.clone()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &entity_shader,
                    entry_point: Some("fs_hdr_mark"),
                    compilation_options: Default::default(),
                    targets: &mark_targets,
                }),
                ..additive_descriptor.clone()
            });
        let lightning_pipeline = kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Lightning"),
            vertex: wgpu::VertexState {
                entry_point: Some("vs_lightning"),
                buffers: &lightning_vertex,
                ..additive_descriptor.vertex.clone()
            },
            ..additive_descriptor
        });
        let celestial = celestial::Celestial::new(
            &kept,
            &entity_layout,
            &entity_shader,
            &entity_vertex,
            (frame_format, linear_light),
        );
        drop(entity_step);
        // WorldBorderRenderer: misc/forcefield.png as a MipmappedTexture
        // (repeat, linear minification with mipmaps, nearest magnification).
        let forcefield_size = forcefield.1;
        let (forcefield_mips, forcefield_levels) = prepared.forcefield;
        let forcefield_texture = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("Forcefield"),
                size: wgpu::Extent3d {
                    width: forcefield_size,
                    height: forcefield_size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: forcefield_levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &forcefield_mips,
        );
        let forcefield_view = forcefield_texture.create_view(&Default::default());
        let forcefield_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Forcefield"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            min_filter: wgpu::FilterMode::Linear,
            mag_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let border_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("World border"),
            size: std::mem::size_of::<BorderUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let border_vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("World border vertices"),
            size: 16 * std::mem::size_of::<crate::border_render::BorderVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let border_bindings = kept.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("World border bindings"),
            entries: &[
                entry(
                    0,
                    vf,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    2,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    3,
                    vf,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let border_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("World border resources"),
            layout: &border_bindings,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&forcefield_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&forcefield_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: border_uniform.as_entire_binding(),
                },
            ],
        });
        let border_layout = kept.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("World border layout"),
            bind_group_layouts: &[Some(&border_bindings)],
            ..Default::default()
        });
        let border_shader = kept.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sunrise world border"),
            source: wgpu::ShaderSource::Wgsl(border_shader().into()),
        });
        let border_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2];
        // RenderPipelines.WORLD_BORDER: BlendFunction.OVERLAY, no culling,
        // depth writes with a polygon offset of 3 towards the camera.
        let border_pipeline = kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("World border"),
            layout: Some(&border_layout),
            vertex: wgpu::VertexState {
                module: &border_shader,
                entry_point: Some("vs_border"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<crate::border_render::BorderVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &border_attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 3,
                    slope_scale: 3.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &border_shader,
                entry_point: Some("fs_border"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &linear_blend::frame_constants(frame_format, linear_light),
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format: frame_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::Zero,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let screen_shader = kept.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sunrise screen effects"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../screen.wgsl").into()),
        });
        let screen_attributes =
            wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x2,2=>Float32x4,3=>Float32x4];
        let screen_vertex = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::screen_effects::ScreenVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &screen_attributes,
        })];
        let vignette_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::OneMinusSrc,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        // One per blend and `.png.mcmeta` sampling (screen_effects::SAMPLE_*):
        // the shader is specialised, so a pixel pays for none of the others.
        let screen_pipelines: [wgpu::RenderPipeline; 16] = std::array::from_fn(|i| {
            let blur = i as u32 & crate::screen_effects::SAMPLE_BLUR != 0;
            let (entry, blend) = match (i & 4 == 0, blur) {
                (true, false) => ("fs_screen", alpha_blend),
                (true, true) => ("fs_screen_blur", alpha_blend),
                (false, false) => ("fs_vignette", vignette_blend),
                (false, true) => ("fs_vignette_blur", vignette_blend),
            };
            let clamp = i as u32 & crate::screen_effects::SAMPLE_CLAMP != 0;
            let constants = [("CLAMP", f64::from(u8::from(clamp)))];
            let compilation_options = wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            };
            kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Screen effects"),
                layout: Some(&entity_layout),
                vertex: wgpu::VertexState {
                    module: &screen_shader,
                    entry_point: Some("vs_screen"),
                    compilation_options: compilation_options.clone(),
                    buffers: &screen_vertex,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &screen_shader,
                    entry_point: Some(entry),
                    compilation_options,
                    targets: &[Some(wgpu::ColorTargetState {
                        format: overlay_format,
                        blend: Some(blend),
                        // 8 and up: colour only, for a tint of the world in HDR output.
                        write_mask: if i & 8 == 0 {
                            wgpu::ColorWrites::ALL
                        } else {
                            wgpu::ColorWrites::COLOR
                        },
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let screen_capacity = 64;
        let screen_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Screen effect vertices"),
            size: screen_capacity
                * std::mem::size_of::<crate::screen_effects::ScreenVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let celestial_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Celestial vertices"),
            size: 2048 * 4 * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sunrise_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Sunrise fan vertices"),
            size: (sunrise_fan::VERTICES * 5 * std::mem::size_of::<f32>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entity_capacity = 1 << 14;
        let entity_skin = entity_skin::EntitySkin::new(
            &device,
            animate::SpriteAnimator::compute(&adapter, &device),
        );
        let entity_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Entity vertices"),
            size: entity_capacity * std::mem::size_of::<Vertex>() as u64,
            usage: entity_skin::EntitySkin::usage(entity_skin.is_some()),
            mapped_at_creation: false,
        });
        let static_entity_index_quads = 1 << 14;
        let static_entity_indices = Self::quad_indices(&device, static_entity_index_quads);
        let indirect_execution = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION);
        let flat_wanted = flat::wanted(&adapter, &device);
        // Metal draws solid terrain in two passes (`split_solid`): meshes put all
        // their opaque quads first, so each pass draws its part in a run or two.
        crate::mesher::group_parts(adapter.get_info().backend == wgpu::Backend::Metal);
        // Readable as storage where flat draws run, or while a `shaders/` pack's terrain programs
        // read them per section (`arena_storage`): it starts as for the picked pack, taken to
        // read them, so the pack loading keeps it (`follow_arena_layout`).
        // `SUNRISE_SHADER_PACK_WITHOUT=vertex-storage` takes reads away as for a pack
        // (`pack_post::Gpu::vertex_reads`).
        let terrain_pages = flat::pages_readable(&adapter, &device)
            && pack_post::Gpu::of(&adapter, device.features()).vertex_reads;
        let arena_storage = arena_storage(flat_wanted, pack_starts_wide && terrain_pages);
        // With a normal stream where the pack's per-section draws read no pages
        // (`follow_arena_layout`), as for the pack's taken to fit with it.
        let arena_normals = pack_starts_wide && !flat_wanted && !terrain_pages;
        let arena = arena::Arena::new(
            &device,
            std::mem::size_of::<TerrainVertex>() as u64,
            1 << 20,
            arena_storage,
        )
        .with_normals(arena_normals);
        // Sections as one index list per frame where multi-draw is emulated.
        let mut flat = flat_wanted.then(|| {
            let buffers = [Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &TERRAIN_ATTRIBUTES,
            })];
            let info = adapter.get_info();
            // Metal's per-section path drops cutout leaves (the canopy above trunks vanishes),
            // and cave culling can drop the listed count below MIN_SECTIONS on a normal view:
            // so Metal draws vanilla terrain flat at any count.
            let min_sections = if flat::is_broken_intel_vulkan(&info)
                || flat::is_adreno_dx12(&info)
                || info.backend == wgpu::Backend::Metal
                || std::env::var("SUNRISE_FLAT_DRAWS").as_deref() == Ok("1")
            {
                0
            } else if flat::for_vanilla(&info) {
                flat::MIN_SECTIONS
            } else {
                // Only a shader pack's terrain and shadow pass draw flat here.
                usize::MAX
            };
            flat::Flat::new(
                &device,
                &bindings,
                arena.page_vertices(),
                min_sections,
                indirect_execution,
                |layout, label, vertex, fragment, blend, cull, pulled| {
                    let buffers: &[Option<wgpu::VertexBufferLayout<'_>>] =
                        if pulled { &[] } else { &buffers };
                    pipeline_in(
                        layout, label, vertex, fragment, buffers, tri, cull, true, true, blend,
                    )
                },
                |layout, label, vertex, fragment, cull| {
                    solid_in(layout, label, vertex, fragment, &[], cull)
                },
            )
        });
        // With flat draws' page groups the per-section solid draws read their vertices from
        // the page too (`Flat::page_group`), in either layout of the solid quads
        // (`mesher::packed`): only the origins stay a vertex buffer, at slot 1.
        let (solid, solid_opaque, solid_cutout, solid_mirror, flipped) = match &flat {
            Some(f) => {
                let pulled_layout = kept.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Sunrise terrain layout (pulled)"),
                    bind_group_layouts: &[Some(&bindings), Some(&f.draw_layout)],
                    ..Default::default()
                });
                let origins = [None, terrain_buffers[1].clone()];
                let solid = |label: &str, vertex: &str, fragment: &str, cull| {
                    solid_in(&pulled_layout, label, vertex, fragment, &origins, cull)
                };
                let (back, front) = (wgpu::Face::Back, wgpu::Face::Front);
                (
                    solid("Terrain solid", "vs_section", "fs_solid", back),
                    solid(
                        "Terrain solid opaque",
                        "vs_section_opaque",
                        "fs_opaque",
                        back,
                    ),
                    solid(
                        "Terrain solid cutout",
                        "vs_section_cutout",
                        "fs_solid",
                        back,
                    ),
                    pipeline_in(
                        &pulled_layout,
                        "Terrain solid (mirrored)",
                        "vs_section",
                        "fs_solid_mirror",
                        &origins,
                        tri,
                        Some(front),
                        true,
                        true,
                        None,
                    ),
                    solid("Terrain solid (flipped)", "vs_section", "fs_solid", front),
                )
            }
            None => (solid, solid_opaque, solid_cutout, solid_mirror, flipped),
        };
        // The solid quads in the compact layout where every pass reads it; the first world
        // frames settle it for the picked pack (`follow_packed_layout`).
        let packed_quads = flat.is_some() && !pack_starts_wide && packed_wanted();
        packed::pack_quads(packed_quads);
        let visbuf = flat
            .as_ref()
            .filter(|_| visbuffer::wanted(&device))
            .map(|f| {
                log::info!(
                    "Sunrise: solid terrain through a visibility buffer (SUNRISE_VISBUFFER)"
                );
                visbuffer::Visbuffer::new(
                    &device,
                    &bindings,
                    f,
                    (frame_format, linear_light),
                    &sprite_constants,
                )
            });
        let multi_draw = device
            .features()
            .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
            && indirect_execution;
        let batchable = translucent_batchable(&adapter.get_info(), multi_draw);
        crate::graphics::offer_translucent_batching(batchable);
        let translucent_batching = batchable && translucent_batching;
        let gpu_culling_offer = gpu_culling::offer(
            &adapter.get_info(),
            gpu_culling::supported(
                &adapter.get_info(),
                device.features(),
                adapter.get_downlevel_capabilities().flags,
                &device.limits(),
                multi_draw,
                flat.as_ref().map(|f| f.min_sections),
            ),
            flat.is_some(),
        );
        // Not again after the device was lost in one of its frames, until Sunrise restarts.
        let gpu_culling_offer = if crate::graphics::gpu_culling_lost() {
            crate::graphics::GpuCullingOffer::DeviceLost
        } else {
            gpu_culling_offer
        };
        crate::graphics::offer_gpu_culling(gpu_culling_offer);
        // A new renderer runs no shader pack: the game loads the picked one on its
        // first world frame (`shader_pack`: the picked pack draws flat, a `shaders/`
        // pack or one of Sunrise's own with `[shadow]`), and the frames before it, the
        // level loading screen's among them, keep the layout the sections upload in
        // (`draw_frame`). It starts as that pack's (`pack_starts_wide`: a `shaders/` pack
        // taken to draw terrain), so the pack loading keeps it.
        let (sorted_indices, translucent_arena) = arena::Arena::translucent_arenas(
            &device,
            std::mem::size_of::<TerrainVertex>() as u64,
            flat.is_some(),
            translucent_split(
                translucent_batching,
                flat.is_some(),
                pack_starts_wide && terrain_pages,
                shader_pack,
                false,
            ),
            arena_normals,
        );
        if let (Some(flat), Some(arena)) = (&mut flat, &translucent_arena) {
            flat.set_translucent_page_vertices(arena.page_vertices());
        }
        // Without indirect draws (OpenGL before 4.0 without indirect execution, or
        // before 4.2 + GL_ARB_shader_draw_parameters without indirect first instance)
        // even creating an indirect buffer fails; one draw per section reads
        // `draws` on the CPU instead.
        let indirect = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Section draws"),
            size: 1024 * 20,
            usage: if multi_draw {
                wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST
            } else {
                wgpu::BufferUsages::COPY_DST
            },
            mapped_at_creation: false,
        });
        // SUNRISE_GPU_CULL=0 turns occlusion culling off, `all` culls every
        // frame however few its draws and little it hides, `shader` tests in
        // the shader as on Metal (A/B checks).
        let cull_env = std::env::var("SUNRISE_GPU_CULL");
        let adapter_info = adapter.get_info();
        let cull = cull::Cull::supported(&adapter, &device)
            .filter(|_| occlusion_culling_requested(&adapter_info, cull_env.as_deref().ok()))
            .map(|depth_test| match cull_env.as_deref() {
                Ok("shader") => cull::DepthTest::InShader,
                _ => depth_test,
            })
            .map(|depth_test| {
                let mut cull = cull::Cull::new(&device, frame_format, depth_test);
                cull.always = cull_env.as_deref() == Ok("all");
                if !cull.always && adapter_info.backend == wgpu::Backend::Metal {
                    cull.min_sections = cull::TILED_MIN_SECTIONS;
                }
                cull
            });
        let depth = Self::depth(&device, &config);
        let world_post = world_post::Post::new(&device, frame_format);
        let uploads = upload::Uploads::new(&device);
        let section_uploads = upload::Uploads::with_chunk(&device, SECTION_UPLOAD_CHUNK);
        let stars = stars::Stars::new(&device, (frame_format, linear_light));
        let custom_sky = custom_sky::CustomSky::new(&device, (frame_format, linear_light));
        let entity_outline = entity_outline::EntityOutline::new(
            &device,
            (&entity_shader, &entity_layout, &entity_vertex),
            overlay_format,
        );
        let panorama = panorama::Panorama::new(&device, frame_format, panorama_faces);
        drop(pipelines_step);
        reuse.finish(generation);
        if previous.is_some() {
            log::info!(
                "Sunrise renderer: rebuilt on the same device ({} shader modules, layouts and pipelines kept)",
                reuse.len()
            );
        }
        if previous.is_none() {
            startup_step(format_args!(
                "pipelines ready (multi-draw {}, occlusion culling {}, GPU culling {}, GPU timer {})",
                on_off(multi_draw),
                on_off(cull.is_some()),
                if gpu_culling_offer == crate::graphics::GpuCullingOffer::Offered {
                    "available"
                } else {
                    "unavailable"
                },
                on_off(gpu_timer.is_some()),
            ));
        }
        let mut world_primitives =
            world_primitives::WorldPrimitives::new(&device, (frame_format, linear_light));
        world_primitives.reserve(
            &device,
            &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes),
            world_primitives::DEFAULT_CAPACITY,
            world_primitives::DEFAULT_CAPACITY,
        );
        let core_particles =
            core_particles::CoreParticles::new(&device, &queue, (frame_format, linear_light));
        let views = world_views::Kept::new(flipped, translucent_flipped, entity_pipeline_with);
        let mod_gpu = mod_gpu::ModGpuResources::new(
            adapter.clone(),
            device.clone(),
            queue.clone(),
            hud.dynamic().clone(),
            mod_gpu::ModFrameState {
                entity_atlas: atlas_view.clone(),
                lightmap: lightmap_view.clone(),
                world_depth: depth.clone(),
                entity_quad_capacity: EntityQuadCapacity::from_limits(
                    &device.limits(),
                    entity_skin.is_some(),
                ),
                pack_iris: false,
            },
        );
        Ok(Self {
            display: None,
            reading: display,
            display_fresh: false,
            instance,
            adapter,
            surface: surface.map(Arc::new),
            spare_surface: std::sync::Mutex::new(None),
            offscreen,
            in_flight: std::collections::VecDeque::new(),
            first_frame: if previous.is_none() {
                FirstFrame::Acquire
            } else {
                FirstFrame::Drawn
            },
            gpu_timer,
            gpu_times: Vec::new(),
            lightmap_chart: core_lightmap::LightmapChart::new(),
            last_wait: std::time::Duration::ZERO,
            last_encode: std::time::Duration::ZERO,
            window,
            ray_scene: surface_setup::ray_query::ray_scene::RayScene::new(&device),
            device,
            reuse,
            queue: submit::Queue::new(queue, submit::wanted()),
            deferred: None,
            device_lost,
            config,
            solid,
            solid_opaque,
            solid_cutout,
            translucent,
            translucent_mark,
            flat_on: flat.is_some(),
            packed_quads,
            packed_on: packed_wanted(),
            cull_on: cull.is_some(),
            sides_on: true,
            gpu_culling: None,
            sourced: Default::default(),
            gpu_sourced_frame: false,
            gpu_culling_offer,
            gpu_frame: false,
            #[cfg(debug_assertions)]
            gpu_checks: 0,
            cull,
            flat,
            visbuf,
            sky,
            sky_base,
            sunrise_pipeline,
            sunrise_buffer,
            dark_disc,
            depth_reset,
            lines,
            lines_secondary,
            core_lines: None,
            core_text: None,
            core_sky: None,
            core_particles,
            core_lightmap: None,
            core_clouds: None,
            debug_lines_opaque,
            debug_lines,
            debug_lines_on_top,
            debug_crosshair,
            overlay_crosshair,
            overlay_hand,
            overlay_hand_banner_pattern,
            overlay_hand_glint,
            debug_line_buffer,
            debug_line_capacity: 4096,
            crosshair_group,
            crosshair_globals,
            solid_mirror,
            mirror_globals,
            mirror_group,
            mirror: None,
            world_views: views,
            world_view_side: max_side,
            entity_pipeline,
            banner_pattern_pipeline,
            glint_only_pipeline,
            particle_pipelines,
            explicit_opaque_particles: Default::default(),
            explicit_translucent_particles: Default::default(),
            explicit_additive_particles: Default::default(),
            static_entity_pipeline,
            hdr_mark_pipeline,
            hdr_entity_mark_pipeline,
            hdr_hand_unmark_pipeline,
            hdr_mark_lightning_pipeline,
            weather,
            crumbling_pipeline,
            shadow_blob_pipeline,
            translucent_item_pipeline,
            water_mask_pipeline,
            cloud_pipeline,
            hdr_cloud_mark_pipeline,
            cloud_mesh: Default::default(),
            cloud_buffer,
            cloud_capacity: 4,
            cloud_uniform,
            cloud_group,
            additive_pipeline,
            dragon_rays_pipeline,
            energy_swirl_pipeline,
            lightning_pipeline,
            lightning: lightning::Renderer::new(),
            see_through_pipeline,
            world_primitives: Some(world_primitives),
            mod_gpu,
            transient_targets: std::sync::Mutex::new(
                mod_frame_graph::TransientTargetPool::default(),
            ),
            text_polygon_offset_pipeline,
            world_text_pipeline,
            text_mark_pipelines,
            fluid_pipeline,
            terrain_model_pipelines,
            celestial,
            celestial_buffer,
            end_sky: end_sky::EndSky::default(),
            stars,
            custom_sky,
            custom_sky_draws: Vec::new(),
            entity_outline,
            panorama,
            border_pipeline,
            border_group,
            border_uniform,
            border_vertices,
            screen_pipelines,
            screen_page_groups,
            screen_buffer,
            screen_capacity,
            entity_group,
            entity_bindings,
            prepared_models: None,
            prepared_draws: Vec::new(),
            prepared_cuts: Vec::new(),
            prepared_ranges: Vec::new(),
            prepared_uploaded: Vec::new(),
            prepared_instance_buffer,
            prepared_instance_capacity,
            static_entity_group,
            static_entity_transform,
            static_entity_camera: None,
            static_entity_catalog_revision: 0,
            static_entity_frame_stats: Default::default(),
            hand_group,
            hand_globals,
            entity_buffer,
            entity_capacity,
            static_entity_sections: HashMap::default(),
            static_entity_indices,
            static_entity_index_quads,
            entity_batch: EntityBatch::default(),
            dynamic_entity,
            entity_order: entity_skin::OrdinaryOrder::default(),
            entity_skin,
            group,
            fluid_group,
            fluid_sampler,
            terrain_model_group,
            terrain_model_sampler,
            globals,
            pbr_maps,
            lightmap,
            sprite_textures,
            sprite_unused,
            sprite_views,
            mip_levels,
            mipmaps: mip_levels - 1,
            terrain_layout: bindings,
            terrain_sprites: sprite_constants.map(|(_, v)| v),
            sprite_split: split,
            sprite_sampler,
            sprite_nearest,
            texture_filtering: 0,
            max_anisotropy: 1,
            lightmap_view,
            lightmap_sampler,
            animations,
            material_animations,
            active_animated_layers: vec![0; active_layer_words],
            uploads,
            section_uploads,
            section_copies: None,
            section_copy_bytes: 0,
            shrink_gate: ShrinkGate::default(),
            lightmap_uploaded: false,
            sprite_minimum_size,
            sprite_native: sprite_mips.iter().map(|s| s.native_size).collect(),
            mipmap_option: 4,
            indices,
            index_quads,
            instances,
            instance_capacity,
            outline,
            depth,
            sections: resident::Resident::default(),
            lods: FxHashMap::default(),
            lod_vertex_bytes: 0,
            lod_cells: lod::TileGrid::default(),
            lod_cull_free: Vec::new(),
            lod_cull_next: 0,
            lod_cutout: Default::default(),
            lod_retiring: Default::default(),
            lod_leaving: Vec::new(),
            lod_pending: None,
            lod_settled: None,
            detail_columns: Default::default(),
            detail_revision: 0,
            arena,
            translucent_arena,
            translucent_batching,
            sections_dropped: false,
            sorted_indices,
            graveyard: Default::default(),
            translucent_sorter: translucency::Sorter::default(),
            translucency_cursor: 0,
            indirect,
            indirect_capacity: 1024,
            multi_draw,
            visible: Vec::new(),
            visible_lods: Vec::new(),
            visible_renderable_sections: Vec::new(),
            visible_renderable_scratch: Vec::new(),
            hook_geometry: Default::default(),
            hook_entities_scratch: Vec::new(),
            hook_see_through_scratch: Vec::new(),
            translucent_lods: Vec::new(),
            shadow_visible: Vec::new(),
            translucent_draws: Vec::new(),
            translucent_tests: Vec::new(),
            translucent_marks: Vec::new(),
            phase2_upload: Vec::new(),
            origins: Vec::new(),
            draws: Vec::new(),
            draw_pages: Vec::new(),
            phase2_draws: Vec::new(),
            phase2_pages: Vec::new(),
            solid_runs: page_runs::PageRuns::default(),
            phase2_runs: page_runs::PageRuns::default(),
            shadow_runs: page_runs::PageRuns::default(),
            external: None,
            block_entities: Default::default(),
            block_entity_section_revisions: Default::default(),
            block_entity_membership_revisions: Default::default(),
            section_uploaded: Default::default(),
            newest_upload: None,
            newest_lod_fade: None,
            block_entity_revision: next_block_entity_revision(),
            light_blocks: Default::default(),
            light_revision: 0,
            occlusion: occlusion::Occlusion::new(crate::chunk::MIN_Y, crate::chunk::HEIGHT / 16),
            layout: crate::chunk::ColumnLayout::VANILLA,
            world_post,
            frame_gen: frame_gen::FrameGen::new(),
            entity_atlas: atlas_view.clone(),
            hdr: None,
            hdr_lighting_setting: true,
            emission_stats,
            output_caps,
            output_rejected: None,
            auto_off_since: None,
            // Nothing is presented as HDR yet: `update_output` applies the settings.
            output: output::resolve_output(settings, output_caps, display, frame, false),
            frame_kept_logged: false,
            hdr_marker: None,
            hdr_driver,
            pack_post: None,
            pack_loading: None,
            pack_generation: 0,
            native: None,
            pass_timing: false,
            pass_detail: false,
            pass_timing_gap: std::time::Duration::ZERO,
            keep_timed_frames: false,
            own_timing: None,
            pack_entity_rest: Vec::new(),
            cloud_bindings,
            pack_weather: None,
            pack_atlas: None,
            pack_material_atlases: None,
            shared_material_maps: [None, None],
            pack_terrain_flat: false,
            terrain_pages,
            pack_terrain_pages: false,
            pack_terrain_normals: false,
            pack_terrain_bindings: Default::default(),
            arena_storage,
            separate_ao: false,
            pack_pages: Vec::new(),
            pack_block_ids: None,
            pack_sources_on: false,
            outline_edges_scratch: Vec::new(),
            outline_lines_scratch: Vec::new(),
            debug_lines_scratch: Vec::new(),
            screen_vertices_scratch: Vec::new(),
            lod_leaving_scratch: crate::fast_hash::FxHashSet::default(),
            shadow_boxed_scratch: Vec::new(),
            shadow_seen_scratch: crate::fast_hash::FxHashSet::default(),
            shadow_translucent_scratch: Vec::new(),
            pack_lava_layers: Vec::new(),
            hud,
            menu_blur: None,
            post_effects: None,
            readback: readback::Readback::default(),
            info,
            suspended: !drawable_size(size.width, size.height, max_side),
            acquire_validation_failures: 0,
            surface_unconfigured: false,
            drawn_sections: 0,
            section_draws: 0,
            section_draw_calls: 0,
            frame_serial: 0,
            stats_frame: 0,
            stats_halves: Default::default(),
            uploaded_bytes: 0,
            started: Instant::now(),
            screenshot: None,
            screenshot_result: None,
            screenshot_readbacks: screenshot_readback::Readbacks::default(),
            screenshot_quality: 1,
            screenshot_halvings: 0,
            gamma: 0.5,
            improved_transparency: false,
            oit: None,
            fullbright: false,
            animated_textures: true,
            present_modes,
            capture_copies: std::env::var_os("SUNRISE_RENDER_CAPTURE")
                .is_some()
                .then(capture::Copies::default),
            capture: None,
            capture_result: None,
        })
    }
}
