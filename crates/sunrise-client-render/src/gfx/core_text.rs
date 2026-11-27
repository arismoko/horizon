//! A resource pack's `core/text` on world text, as vanilla 26.3's `TEXT`, `TEXT_POLYGON_OFFSET`
//! and `TEXT_SEE_THROUGH` draw signs, text displays, name tags and their backgrounds, and as
//! Improved Transparency's `OIT_TEXT` and `OIT_TEXT_POLYGON_OFFSET` draw the translucent ones
//! ([`OitPart`]). These paths and the GUI's text take the pack's shader; Sunrise's own world
//! canvas (`WORLD_TEXT_OVERLAY`, a native mod's) draws natively. Every pipeline vanilla requires
//! of the shader is checked when the packs load (`hud::pack_text::TEXT`).

use oit::OitPart;
use std::sync::Arc;
use sunrise_glsl::{Glue, Translated};

mod oit;

/// The entity vertices' light bytes are sky then block; vanilla's `UV2` is block then sky.
pub(crate) const WORLD_GLUE: Glue = Glue {
    inputs: &[
        ("Position", "vec3"),
        ("Color", "vec4"),
        ("UV0", "vec2"),
        ("UV2", "ivec2"),
    ],
    prelude: "layout(location = 0) in vec3 hz_pos;\n\
              layout(location = 1) in vec2 hz_uv;\n\
              layout(location = 3) in vec4 hz_color;\n\
              layout(location = 4) in uvec4 hz_light;",
    setup: "Position = hz_pos; Color = hz_color; UV0 = hz_uv; UV2 = ivec2(hz_light.y, hz_light.x);",
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
    fragment_finish: None,
    interface: sunrise_shaders::DEVICELESS_INTERFACE_LIMITS,
};

pub(crate) const SEE_THROUGH_GLUE: Glue = Glue {
    inputs: &[("Position", "vec3"), ("Color", "vec4"), ("UV0", "vec2")],
    prelude: "layout(location = 0) in vec3 hz_pos;\n\
              layout(location = 1) in vec2 hz_uv;\n\
              layout(location = 3) in vec4 hz_color;",
    setup: "Position = hz_pos; Color = hz_color; UV0 = hz_uv;",
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
    fragment_finish: None,
    interface: sunrise_shaders::DEVICELESS_INTERFACE_LIMITS,
};

#[derive(Clone, Default)]
pub struct TextShaders {
    pub gui: Option<Arc<Translated>>,
    pub world: Option<Arc<Translated>>,
    pub see_through: Option<Arc<Translated>>,
    /// `OIT_TEXT`'s programs, which `OIT_TEXT_POLYGON_OFFSET` shares: none unless all three
    /// stages' load. Boxed, as the game holds the atlas inline.
    pub oit: Option<Box<OitPrograms>>,
    /// Each program's Polytone expression uniforms (`shader_modifiers`), resolved when it
    /// loaded: the blocks its stages read, with their programs, bound after the pipeline's own
    /// (`expression_blocks`).
    pub expressions: TextExpressions,
}

/// [`TextShaders`]' expression uniforms, program by program.
#[derive(Clone, Debug, Default)]
pub struct TextExpressions {
    pub gui: Vec<crate::polytone_post::ExpressionUniform>,
    pub world: Vec<crate::polytone_post::ExpressionUniform>,
    pub see_through: Vec<crate::polytone_post::ExpressionUniform>,
}

/// `OIT_TEXT`'s program in each Improved Transparency stage (`OitStage`'s order), with its
/// Polytone expression uniforms.
#[derive(Clone)]
pub struct OitPrograms {
    pub programs: [Arc<Translated>; 3],
    pub expressions: [Vec<crate::polytone_post::ExpressionUniform>; 3],
}

#[derive(Clone, Copy)]
pub(super) struct PackBlocks {
    pub(super) globals: [u8; 48],
    pub(super) fog: [f32; 6],
}

impl super::Renderer {
    /// The packs' world and see-through text programs (`HudAtlas::text_shaders`), or the native
    /// world text; or the device's refusal of a pipeline vanilla requires, which fails the pack
    /// reload.
    pub fn set_world_text_shaders(&mut self, shaders: &TextShaders) -> Result<(), String> {
        let expressions = &shaders.expressions;
        let mut core = CoreText::new(
            &self.device,
            self.output.frame.format,
            (shaders.world.as_deref()).map(|p| (p, &expressions.world[..])),
            (shaders.see_through.as_deref()).map(|p| (p, &expressions.see_through[..])),
            &self.entity_atlas,
            (&self.lightmap_view, &self.lightmap_sampler),
        )?;
        // Improved Transparency's stages, where the device has it. They share the world text's
        // fallback: with no pack pipeline of the world's or the see-through text's, the
        // translucent world text draws natively in every mode.
        if let (Some(core), Some(programs)) = (core.as_mut(), shaders.oit.as_ref())
            && super::oit::supported(&self.device)
        {
            let programs =
                std::array::from_fn(|i| (&*programs.programs[i], &programs.expressions[i][..]));
            core.oit = OitPart::new(
                &self.device,
                programs,
                &self.entity_atlas,
                (&self.lightmap_view, &self.lightmap_sampler),
            )?;
        }
        self.core_text = core;
        Ok(())
    }

    /// Whether a pack's `core/text` draws the world's text (`TEXT`, `TEXT_POLYGON_OFFSET`): its
    /// pipelines were made for this renderer.
    pub fn draws_pack_world_text(&self) -> bool {
        self.core_text.as_ref().is_some_and(|c| c.world.is_some())
    }

    /// The vanilla `Globals` block a pack's core shaders read this frame.
    pub(super) fn pack_globals(&self, frame: &super::Frame) -> [u8; 48] {
        let globals = crate::post_chain::Globals {
            glint_alpha: frame.glint_alpha,
            ..frame.post_effects.globals
        };
        let size = [self.config.width, self.config.height];
        globals.bytes(size, frame.post_effects.menu_blur_radius as i32)
    }

    pub(super) fn pack_blocks(
        &self,
        frame: &super::Frame,
        fog: [f32; 4],
        ends: [f32; 4],
    ) -> PackBlocks {
        PackBlocks {
            globals: self.pack_globals(frame),
            fog: [fog[0], fog[1], fog[2], fog[3], ends[0], ends[1]],
        }
    }

    pub(super) fn bind_world_text(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        mode: WorldTextMode,
    ) -> bool {
        let pack = self.core_text.as_ref().and_then(|core| match mode {
            WorldTextMode::Normal => core.world.as_ref().map(|w| (&w.normal, &w.group)),
            WorldTextMode::PolygonOffset => {
                core.world.as_ref().map(|w| (&w.polygon_offset, &w.group))
            }
            WorldTextMode::SeeThrough => core.see_through.as_ref().map(|p| (&p.pipeline, &p.group)),
        });
        match pack {
            Some((pipeline, group)) => {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, group, &[]);
                true
            }
            None => {
                pass.set_bind_group(0, &self.entity_group, &[]);
                false
            }
        }
    }

    pub(super) fn bind_world_text_hdr_mark(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        mode: WorldTextMode,
    ) -> bool {
        let Some(core) = self.core_text.as_ref() else {
            return false;
        };
        let Some(world) = core.world.as_ref() else {
            return false;
        };
        let pipeline = match mode {
            WorldTextMode::Normal => world.mark_normal.as_ref(),
            WorldTextMode::PolygonOffset => world.mark_polygon_offset.as_ref(),
            WorldTextMode::SeeThrough => None,
        };
        let Some(pipeline) = pipeline else {
            return false;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &world.group, &[]);
        pass.set_bind_group(1, &core.hdr_mark_group, &[]);
        true
    }
}

/// Which world-text depth mode to bind for a draw call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorldTextMode {
    /// Normal depth testing and writes (`RenderPipelines.TEXT`, e.g. sign outline copies).
    Normal,
    /// Polygon-offset depth bias (`RenderPipelines.TEXT_POLYGON_OFFSET`, e.g. sign face glyphs).
    PolygonOffset,
    /// See-through text (`RenderPipelines.TEXT_SEE_THROUGH`).
    SeeThrough,
}

/// Byte offset of the 32-byte HDR tier-window uniform inside `CoreText::uniform` (256-byte
/// aligned, right after vanilla's 1024-byte blocks and before Polytone expression blocks).
const HDR_WINDOWS_OFFSET: u64 = 1024;
const EXTRA_BLOCKS_BASE: u64 = 1280;

/// The pack's world text pipelines (normal depth + polygon-offset depth, plus matching HDR mark
/// pipelines) sharing one bind group and Polytone expression blocks.
pub(super) struct WorldPart {
    pub(super) normal: wgpu::RenderPipeline,
    pub(super) polygon_offset: wgpu::RenderPipeline,
    pub(super) mark_normal: Option<wgpu::RenderPipeline>,
    pub(super) mark_polygon_offset: Option<wgpu::RenderPipeline>,
    pub(super) group: wgpu::BindGroup,
    extra: super::expression_blocks::ExpressionBlocks,
}

/// The pack's see-through text pipeline: uses `IS_SEE_THROUGH`, so it has its own layout and may
/// read different blocks from world text.
pub(super) struct Part {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) group: wgpu::BindGroup,
    /// The Polytone blocks its stages read (`PolyGlobals` and the packs' expression uniforms,
    /// `shader_modifiers`), in the shared buffer after vanilla's blocks.
    extra: super::expression_blocks::ExpressionBlocks,
}

/// The pack's world text pipelines over one uniform buffer: vanilla's four blocks once, HDR
/// tier windows at offset 1024, then the world text's Polytone blocks, then the see-through
/// text's.
pub(crate) struct CoreText {
    pub(super) world: Option<WorldPart>,
    pub(super) see_through: Option<Part>,
    /// `OIT_TEXT`'s stages, where the device has Improved Transparency; boxed, as the clouds'
    /// stages are, to keep the renderer the game holds inline small.
    pub(super) oit: Option<Box<OitPart>>,
    uniform: wgpu::Buffer,
    hdr_mark_group: wgpu::BindGroup,
}

impl CoreText {
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        world: Option<(&Translated, &[crate::polytone_post::ExpressionUniform])>,
        see_through: Option<(&Translated, &[crate::polytone_post::ExpressionUniform])>,
        atlas: &wgpu::TextureView,
        lightmap: (&wgpu::TextureView, &wgpu::Sampler),
    ) -> Result<Option<Self>, String> {
        if world.is_none() && see_through.is_none() {
            return Ok(None);
        }
        let depth = |write, compare, bias| wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: Default::default(),
            bias,
        };
        let offset = wgpu::DepthBiasState {
            constant: 10,
            slope_scale: 1.0,
            clamp: 0.0,
        };
        // Vanilla's blocks first, HDR tier windows at 1024, then each pipeline's Polytone blocks.
        let mut base = EXTRA_BLOCKS_BASE;
        let world_extra = world.map(|(core, expressions)| {
            let extra = super::expression_blocks::ExpressionBlocks::new(core, expressions, base);
            base = extra.size();
            extra
        });
        let see_through_extra = see_through.map(|(core, expressions)| {
            let extra = super::expression_blocks::ExpressionBlocks::new(core, expressions, base);
            base = extra.size();
            extra
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Resource pack world text uniforms"),
            size: base,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hdr_mark_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Resource pack world text HDR mark"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(32),
                },
                count: None,
            }],
        });
        let hdr_mark_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Resource pack world text HDR mark"),
            layout: &hdr_mark_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform,
                    offset: HDR_WINDOWS_OFFSET,
                    size: std::num::NonZeroU64::new(32),
                }),
            }],
        });
        let glyphs = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Resource pack world text glyphs"),
            ..Default::default()
        });
        let textures = [(4, atlas, &glyphs), (8, lightmap.0, lightmap.1)];
        let text = &crate::hud::pack_text::TEXT;
        // `TEXT` and `TEXT_POLYGON_OFFSET` are one program, named by the first: a program past
        // the device's limits draws natively (`sunrise_glsl::PastDeviceLimit`), and the device
        // refusing a pipeline fails the load, named by it.
        let world_pipeline = text[crate::hud::pack_text::TEXT_WORLD].pipeline;
        let world = world.filter(|(program, _)| {
            crate::core_shader::within_device_limits(device, program, world_pipeline, &[format])
        });
        let world_part = match (world, world_extra) {
            (Some((program, _)), Some(extra)) => {
                let part = crate::core_shader::scoped(device, world_pipeline, || {
                    Self::world_part(
                        device,
                        format,
                        program,
                        extra,
                        (
                            depth(
                                true,
                                wgpu::CompareFunction::GreaterEqual,
                                Default::default(),
                            ),
                            depth(true, wgpu::CompareFunction::GreaterEqual, offset),
                        ),
                        (
                            depth(false, wgpu::CompareFunction::Equal, Default::default()),
                            depth(false, wgpu::CompareFunction::Equal, offset),
                        ),
                        (&uniform, &textures),
                        &hdr_mark_layout,
                    )
                })?;
                Some(part)
            }
            _ => None,
        };
        let see_through_pipeline = text[crate::hud::pack_text::TEXT_SEE_THROUGH].pipeline;
        let see_through = see_through.filter(|(program, _)| {
            crate::core_shader::within_device_limits(
                device,
                program,
                see_through_pipeline,
                &[format],
            )
        });
        let see_through_part = match (see_through, see_through_extra) {
            (Some((program, _)), Some(extra)) => {
                let part = crate::core_shader::scoped(device, see_through_pipeline, || {
                    Self::part(
                        device,
                        format,
                        "Resource pack see-through text",
                        program,
                        extra,
                        depth(false, wgpu::CompareFunction::Always, Default::default()),
                        (&uniform, &textures),
                    )
                })?;
                Some(part)
            }
            _ => None,
        };
        Ok(Some(Self {
            world: world_part,
            see_through: see_through_part,
            oit: None,
            uniform,
            hdr_mark_group,
        }))
    }

    fn bind_group(
        device: &wgpu::Device,
        label: &str,
        program: &Translated,
        extra: &super::expression_blocks::ExpressionBlocks,
        (uniform, textures): (
            &wgpu::Buffer,
            &[(u32, &wgpu::TextureView, &wgpu::Sampler); 2],
        ),
    ) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
        let blocks = super::expression_blocks::vanilla_blocks(program);
        let (mut entries, mut resources) =
            super::expression_blocks::bindings(&blocks, extra, uniform);
        let (layout, group) = super::expression_blocks::sampler_bindings(program, textures);
        entries.extend(layout);
        resources.extend(group);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &entries,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &layout,
            entries: &resources,
        });
        (layout, group)
    }

    #[expect(clippy::too_many_arguments)]
    fn world_part(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        program: &Translated,
        extra: super::expression_blocks::ExpressionBlocks,
        (normal_depth, offset_depth): (wgpu::DepthStencilState, wgpu::DepthStencilState),
        (mark_normal_depth, mark_offset_depth): (wgpu::DepthStencilState, wgpu::DepthStencilState),
        resources: (
            &wgpu::Buffer,
            &[(u32, &wgpu::TextureView, &wgpu::Sampler); 2],
        ),
        hdr_mark_layout: &wgpu::BindGroupLayout,
    ) -> WorldPart {
        let label = "Resource pack world text";
        let (layout, group) = Self::bind_group(device, label, program, &extra, resources);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::mesher::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let module = |label, module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let (vertex, fragment) = (
            module(label, &program.vertex),
            module(label, &program.fragment),
        );
        let build = |label,
                     pipeline_layout: &wgpu::PipelineLayout,
                     vertex: &wgpu::ShaderModule,
                     fragment: &wgpu::ShaderModule,
                     depth,
                     blend,
                     write_mask| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(pipeline_layout),
                vertex: wgpu::VertexState {
                    module: vertex,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(depth),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: fragment,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let normal = build(
            "Resource pack world text (normal)",
            &pipeline_layout,
            &vertex,
            &fragment,
            normal_depth,
            wgpu::BlendState::ALPHA_BLENDING,
            wgpu::ColorWrites::ALL,
        );
        let polygon_offset = build(
            "Resource pack world text (polygon offset)",
            &pipeline_layout,
            &vertex,
            &fragment,
            offset_depth,
            wgpu::BlendState::ALPHA_BLENDING,
            wgpu::ColorWrites::ALL,
        );
        let (mark_normal, mark_polygon_offset) = if let Some((mark_vs, mark_fs)) =
            wrap_program_for_hdr_mark(program)
        {
            let mark_label = "Resource pack world text HDR mark";
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
            let mark_pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(mark_label),
                    bind_group_layouts: &[Some(&layout), Some(hdr_mark_layout)],
                    ..Default::default()
                });
            let (mark_vertex, mark_fragment) =
                (module(mark_label, &mark_vs), module(mark_label, &mark_fs));
            let normal_mark = build(
                "Resource pack world text HDR mark (normal)",
                &mark_pipeline_layout,
                &mark_vertex,
                &mark_fragment,
                mark_normal_depth,
                wgpu::BlendState::REPLACE,
                wgpu::ColorWrites::ALPHA,
            );
            let offset_mark = build(
                "Resource pack world text HDR mark (polygon offset)",
                &mark_pipeline_layout,
                &mark_vertex,
                &mark_fragment,
                mark_offset_depth,
                wgpu::BlendState::REPLACE,
                wgpu::ColorWrites::ALPHA,
            );
            let internal = pollster::block_on(internal.pop());
            match internal.or(pollster::block_on(validation.pop())) {
                Some(e) => {
                    log::warn!("the resource pack's world text HDR mark shader is not used: {e}");
                    (None, None)
                }
                None => (Some(normal_mark), Some(offset_mark)),
            }
        } else {
            (None, None)
        };
        WorldPart {
            normal,
            polygon_offset,
            mark_normal,
            mark_polygon_offset,
            group,
            extra,
        }
    }

    /// One pipeline of `program` over the shared `uniform` buffer: each block and sampler bound
    /// only to the stages that read it (vanilla's blocks, then `extra`'s; the glyphs at 4 and 5,
    /// the lightmap at 8 and 9, `textures`).
    fn part(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        label: &str,
        program: &Translated,
        extra: super::expression_blocks::ExpressionBlocks,
        depth: wgpu::DepthStencilState,
        resources: (
            &wgpu::Buffer,
            &[(u32, &wgpu::TextureView, &wgpu::Sampler); 2],
        ),
    ) -> Part {
        let (layout, group) = Self::bind_group(device, label, program, &extra, resources);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::mesher::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let module = |module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let (vertex, fragment) = (module(&program.vertex), module(&program.fragment));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(depth),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Part {
            pipeline,
            group,
            extra,
        }
    }

    /// Vanilla's blocks (`data`, as `core_lines::uniforms` lays them out) once into the shared
    /// buffer, then HDR tier windows, then each pipeline's Polytone blocks; the Improved
    /// Transparency stages' only when the frame draws them (`improved_transparency`).
    pub(super) fn prepare(
        &self,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        data: &[u8; 1024],
        (hdr_windows, improved_transparency): ([[f32; 4]; 2], bool),
    ) {
        uploads.buffer(encoder, &self.uniform, 0, data);
        uploads.buffer(
            encoder,
            &self.uniform,
            HDR_WINDOWS_OFFSET,
            bytemuck::bytes_of(&hdr_windows),
        );
        if let Some(world) = &self.world {
            world.extra.upload(uploads, encoder, &self.uniform);
        }
        if let Some(see_through) = &self.see_through {
            see_through.extra.upload(uploads, encoder, &self.uniform);
        }
        if let Some(oit) = self.oit.as_ref().filter(|_| improved_transparency) {
            oit.prepare(uploads, encoder, data);
        }
    }

    /// The pack's `OIT_TEXT` (or `OIT_TEXT_POLYGON_OFFSET`, the same program) for `stage` with its
    /// group, when the pack's text draws under Improved Transparency and [`CoreText::bind_oit`]
    /// made the groups; else nothing is set and the native pipeline draws.
    pub(super) fn bind_oit_stage(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        stage: super::oit::Stage,
    ) -> bool {
        let Some(oit) = self.oit.as_ref() else {
            return false;
        };
        let Some((_, groups)) = oit.groups.as_ref() else {
            return false;
        };
        pass.set_pipeline(&oit.stages[stage as usize].pipeline);
        pass.set_bind_group(0, &groups[stage as usize], &[]);
        true
    }

    /// The Improved Transparency stages' groups for `samples`' targets, made again only when
    /// those targets were.
    pub(super) fn bind_oit(
        &mut self,
        device: &wgpu::Device,
        samples: &super::oit::CloudSamples<'_>,
    ) {
        if let Some(oit) = self.oit.as_mut() {
            oit.bind(device, samples);
        }
    }

    /// The world's, the see-through text's and the Improved Transparency stages' Polytone blocks.
    pub(super) fn expression_blocks(
        &mut self,
    ) -> impl Iterator<Item = &mut super::expression_blocks::ExpressionBlocks> {
        let oit = (self.oit.iter_mut()).flat_map(|o| o.stages.iter_mut().map(|s| &mut s.extra));
        [
            self.world.as_mut().map(|w| &mut w.extra),
            self.see_through.as_mut().map(|p| &mut p.extra),
        ]
        .into_iter()
        .flatten()
        .chain(oit)
    }
}

/// Wraps a pack's world text program so the HDR mark pass executes the pack's own vertex and
/// fragment shaders: any discard or zero-alpha fragment in the pack shader also skips marking,
/// and marked pixels match the colour pass's exact clip depth and brightness.
pub(super) fn wrap_program_for_hdr_mark(
    program: &Translated,
) -> Option<(naga::Module, naga::Module)> {
    let (vertex, tier_location) = wrap_vertex_for_hdr_mark(&program.vertex)?;
    let fragment = wrap_fragment_for_hdr_mark(&program.fragment, tier_location)?;
    Some((vertex, fragment))
}

fn wrap_vertex_for_hdr_mark(source: &naga::Module) -> Option<(naga::Module, u32)> {
    use naga::{Expression as E, Statement as S, TypeInner as T};
    let mut module = source.clone();
    let span = naga::Span::UNDEFINED;
    let entry_index = module
        .entry_points
        .iter()
        .position(|e| e.stage == naga::ShaderStage::Vertex && e.name == "main")?;
    if module.entry_points[entry_index]
        .function
        .arguments
        .iter()
        .any(|a| matches!(a.binding, Some(naga::Binding::Location { location: 2, .. })))
    {
        return None;
    }
    let result = module.entry_points[entry_index].function.result.clone()?;
    let mut next_location = 0u32;
    match &module.types[result.ty].inner {
        T::Struct { members, .. } => {
            for member in members {
                if let Some(naga::Binding::Location { location, .. }) = member.binding {
                    next_location = next_location.max(location.checked_add(1)?);
                }
            }
        }
        _ => {
            if let Some(naga::Binding::Location { location, .. }) = result.binding {
                next_location = next_location.max(location.checked_add(1)?);
            }
        }
    }
    let u32_ty = module.types.insert(
        naga::Type {
            name: None,
            inner: T::Scalar(naga::Scalar::U32),
        },
        span,
    );
    let mut original = std::mem::take(&mut module.entry_points[entry_index].function);
    let orig_arg_count = original.arguments.len();
    let mut entry_args = original.arguments.clone();
    for arg in &mut original.arguments {
        arg.binding = None;
    }
    original.result.as_mut()?.binding = None;
    original.name = Some("hz_pack_text_vs".into());
    let function = module.functions.append(original, span);

    entry_args.push(naga::FunctionArgument {
        name: Some("hz_overlay".into()),
        ty: u32_ty,
        binding: Some(naga::Binding::Location {
            location: 2,
            interpolation: None,
            sampling: None,
            blend_src: None,
            per_primitive: false,
        }),
    });

    let mut f = naga::Function {
        name: Some("main".into()),
        arguments: entry_args,
        ..Default::default()
    };
    let args = (0..orig_arg_count)
        .map(|i| f.expressions.append(E::FunctionArgument(i as u32), span))
        .collect();
    let overlay_arg = f
        .expressions
        .append(E::FunctionArgument(orig_arg_count as u32), span);
    let call = f.expressions.append(E::CallResult(function), span);
    f.body.push(
        S::Call {
            function,
            arguments: args,
            result: Some(call),
        },
        span,
    );
    let shift_16 = f
        .expressions
        .append(E::Literal(naga::Literal::U32(16)), span);
    let mask_ff = f
        .expressions
        .append(E::Literal(naga::Literal::U32(0xff)), span);
    let start = f.expressions.len();
    let shifted = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::ShiftRight,
            left: overlay_arg,
            right: shift_16,
        },
        span,
    );
    let tier = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::And,
            left: shifted,
            right: mask_ff,
        },
        span,
    );
    let (mut members, mut values, offset) = match &module.types[result.ty].inner {
        T::Struct {
            members,
            span: size,
        } => {
            let (members, values): (Vec<_>, Vec<_>) = members
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let value = f.expressions.append(
                        E::AccessIndex {
                            base: call,
                            index: i as u32,
                        },
                        span,
                    );
                    (m.clone(), value)
                })
                .unzip();
            (members, values, *size)
        }
        _ => (
            vec![naga::StructMember {
                name: Some("position".into()),
                ty: result.ty,
                binding: result.binding,
                offset: 0,
            }],
            vec![call],
            16,
        ),
    };
    let tier_offset = offset.next_multiple_of(4);
    members.push(naga::StructMember {
        name: Some("hz_tier".into()),
        ty: u32_ty,
        binding: Some(naga::Binding::Location {
            location: next_location,
            interpolation: Some(naga::Interpolation::Flat),
            sampling: None,
            blend_src: None,
            per_primitive: false,
        }),
        offset: tier_offset,
    });
    let out_ty = module.types.insert(
        naga::Type {
            name: Some("HzTextHdrMarkVsOut".into()),
            inner: T::Struct {
                members,
                span: (tier_offset + 4).next_multiple_of(16),
            },
        },
        span,
    );
    values.push(tier);
    let out_value = f.expressions.append(
        E::Compose {
            ty: out_ty,
            components: values,
        },
        span,
    );
    f.result = Some(naga::FunctionResult {
        ty: out_ty,
        binding: None,
    });
    f.body.push(S::Emit(f.expressions.range_from(start)), span);
    f.body.push(
        S::Return {
            value: Some(out_value),
        },
        span,
    );
    module.entry_points[entry_index].function = f;
    Some((module, next_location))
}

fn wrap_fragment_for_hdr_mark(source: &naga::Module, tier_location: u32) -> Option<naga::Module> {
    use naga::{Expression as E, Statement as S, TypeInner as T};
    let mut module = source.clone();
    let span = naga::Span::UNDEFINED;
    let entry_index = module
        .entry_points
        .iter()
        .position(|e| e.stage == naga::ShaderStage::Fragment && e.name == "main")?;
    if module.entry_points[entry_index]
        .function
        .arguments
        .iter()
        .any(|a| {
            matches!(
                a.binding,
                Some(naga::Binding::Location { location, .. }) if location == tier_location
            )
        })
    {
        return None;
    }
    let result = module.entry_points[entry_index].function.result.clone()?;
    let is_vec4_f32 = |ty| {
        matches!(
            module.types[ty].inner,
            T::Vector {
                size: naga::VectorSize::Quad,
                scalar: naga::Scalar::F32,
            }
        )
    };
    let color_member_index = match &module.types[result.ty].inner {
        T::Struct { members, .. } => {
            let idx = members.iter().position(|m| {
                matches!(m.binding, Some(naga::Binding::Location { location: 0, .. }))
            })?;
            if !is_vec4_f32(members[idx].ty) {
                return None;
            }
            Some(idx)
        }
        _ => {
            if !matches!(
                result.binding,
                Some(naga::Binding::Location { location: 0, .. })
            ) || !is_vec4_f32(result.ty)
            {
                return None;
            }
            None
        }
    };

    let u32_ty = module.types.insert(
        naga::Type {
            name: None,
            inner: T::Scalar(naga::Scalar::U32),
        },
        span,
    );
    let vec3f_ty = module.types.insert(
        naga::Type {
            name: None,
            inner: T::Vector {
                size: naga::VectorSize::Tri,
                scalar: naga::Scalar::F32,
            },
        },
        span,
    );
    let vec4f_ty = module.types.insert(
        naga::Type {
            name: None,
            inner: T::Vector {
                size: naga::VectorSize::Quad,
                scalar: naga::Scalar::F32,
            },
        },
        span,
    );
    let hdr_windows_ty = module.types.insert(
        naga::Type {
            name: Some("HzHdrWindows".into()),
            inner: T::Struct {
                members: vec![
                    naga::StructMember {
                        name: Some("start".into()),
                        ty: vec4f_ty,
                        binding: None,
                        offset: 0,
                    },
                    naga::StructMember {
                        name: Some("full".into()),
                        ty: vec4f_ty,
                        binding: None,
                        offset: 16,
                    },
                ],
                span: 32,
            },
        },
        span,
    );
    let hdr_windows_gv = module.global_variables.append(
        naga::GlobalVariable {
            name: Some("hz_hdr_windows".into()),
            space: naga::AddressSpace::Uniform,
            binding: Some(naga::ResourceBinding {
                group: 1,
                binding: 0,
            }),
            ty: hdr_windows_ty,
            init: None,
            memory_decorations: Default::default(),
        },
        span,
    );

    let mut original = std::mem::take(&mut module.entry_points[entry_index].function);
    let orig_arg_count = original.arguments.len();
    let mut entry_args = original.arguments.clone();
    for arg in &mut original.arguments {
        arg.binding = None;
    }
    original.result.as_mut()?.binding = None;
    original.name = Some("hz_pack_text_fs".into());
    let function = module.functions.append(original, span);

    entry_args.push(naga::FunctionArgument {
        name: Some("hz_tier".into()),
        ty: u32_ty,
        binding: Some(naga::Binding::Location {
            location: tier_location,
            interpolation: Some(naga::Interpolation::Flat),
            sampling: None,
            blend_src: None,
            per_primitive: false,
        }),
    });

    let mut f = naga::Function {
        name: Some("main".into()),
        arguments: entry_args,
        ..Default::default()
    };
    let args = (0..orig_arg_count)
        .map(|i| f.expressions.append(E::FunctionArgument(i as u32), span))
        .collect();
    let tier_arg = f
        .expressions
        .append(E::FunctionArgument(orig_arg_count as u32), span);
    let gv_expr = f
        .expressions
        .append(E::GlobalVariable(hdr_windows_gv), span);
    let call = f.expressions.append(E::CallResult(function), span);
    f.body.push(
        S::Call {
            function,
            arguments: args,
            result: Some(call),
        },
        span,
    );

    let u_0 = f
        .expressions
        .append(E::Literal(naga::Literal::U32(0)), span);
    let u_1 = f
        .expressions
        .append(E::Literal(naga::Literal::U32(1)), span);
    let u_3 = f
        .expressions
        .append(E::Literal(naga::Literal::U32(3)), span);
    let u_4 = f
        .expressions
        .append(E::Literal(naga::Literal::U32(4)), span);
    let f_0 = f
        .expressions
        .append(E::Literal(naga::Literal::F32(0.0)), span);
    let f_1 = f
        .expressions
        .append(E::Literal(naga::Literal::F32(1.0)), span);
    let f_half = f
        .expressions
        .append(E::Literal(naga::Literal::F32(0.5)), span);
    let f_gamma = f
        .expressions
        .append(E::Literal(naga::Literal::F32(2.2)), span);
    let f_luma_r = f
        .expressions
        .append(E::Literal(naga::Literal::F32(0.2126)), span);
    let f_luma_g = f
        .expressions
        .append(E::Literal(naga::Literal::F32(0.7152)), span);
    let f_luma_b = f
        .expressions
        .append(E::Literal(naga::Literal::F32(0.0722)), span);
    let f_band = f
        .expressions
        .append(E::Literal(naga::Literal::F32(0.175)), span);
    let f_lift_span = f
        .expressions
        .append(E::Literal(naga::Literal::F32(0.165)), span);

    let start_check = f.expressions.len();
    let color = match color_member_index {
        Some(idx) => f.expressions.append(
            E::AccessIndex {
                base: call,
                index: idx as u32,
            },
            span,
        ),
        None => call,
    };
    let color_a = f.expressions.append(
        E::AccessIndex {
            base: color,
            index: 3,
        },
        span,
    );
    let tier_zero = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::Equal,
            left: tier_arg,
            right: u_0,
        },
        span,
    );
    let alpha_empty = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::LessEqual,
            left: color_a,
            right: f_0,
        },
        span,
    );
    let skip = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::LogicalOr,
            left: tier_zero,
            right: alpha_empty,
        },
        span,
    );
    f.body
        .push(S::Emit(f.expressions.range_from(start_check)), span);
    f.body.push(
        S::If {
            condition: skip,
            accept: naga::Block::from_vec(vec![S::Kill]),
            reject: naga::Block::new(),
        },
        span,
    );

    let start_mark = f.expressions.len();
    let color_rgb = f.expressions.append(
        E::Swizzle {
            size: naga::VectorSize::Tri,
            vector: color,
            pattern: [
                naga::SwizzleComponent::X,
                naga::SwizzleComponent::Y,
                naga::SwizzleComponent::Z,
                naga::SwizzleComponent::X,
            ],
        },
        span,
    );
    let zero_rgb = f.expressions.append(
        E::Splat {
            size: naga::VectorSize::Tri,
            value: f_0,
        },
        span,
    );
    let gamma_rgb = f.expressions.append(
        E::Splat {
            size: naga::VectorSize::Tri,
            value: f_gamma,
        },
        span,
    );
    let clamped_rgb = f.expressions.append(
        E::Math {
            fun: naga::MathFunction::Max,
            arg: color_rgb,
            arg1: Some(zero_rgb),
            arg2: None,
            arg3: None,
        },
        span,
    );
    let linear_rgb = f.expressions.append(
        E::Math {
            fun: naga::MathFunction::Pow,
            arg: clamped_rgb,
            arg1: Some(gamma_rgb),
            arg2: None,
            arg3: None,
        },
        span,
    );
    let luma_weights = f.expressions.append(
        E::Compose {
            ty: vec3f_ty,
            components: vec![f_luma_r, f_luma_g, f_luma_b],
        },
        span,
    );
    let luma = f.expressions.append(
        E::Math {
            fun: naga::MathFunction::Dot,
            arg: linear_rgb,
            arg1: Some(luma_weights),
            arg2: None,
            arg3: None,
        },
        span,
    );
    let lr = f.expressions.append(
        E::AccessIndex {
            base: linear_rgb,
            index: 0,
        },
        span,
    );
    let lg = f.expressions.append(
        E::AccessIndex {
            base: linear_rgb,
            index: 1,
        },
        span,
    );
    let lb = f.expressions.append(
        E::AccessIndex {
            base: linear_rgb,
            index: 2,
        },
        span,
    );
    let max_gb = f.expressions.append(
        E::Math {
            fun: naga::MathFunction::Max,
            arg: lg,
            arg1: Some(lb),
            arg2: None,
            arg3: None,
        },
        span,
    );
    let max_rgb = f.expressions.append(
        E::Math {
            fun: naga::MathFunction::Max,
            arg: lr,
            arg1: Some(max_gb),
            arg2: None,
            arg3: None,
        },
        span,
    );
    let sum = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::Add,
            left: luma,
            right: max_rgb,
        },
        span,
    );
    let bright = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::Multiply,
            left: f_half,
            right: sum,
        },
        span,
    );
    let y = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::Multiply,
            left: bright,
            right: color_a,
        },
        span,
    );
    let t_clamped = f.expressions.append(
        E::Math {
            fun: naga::MathFunction::Clamp,
            arg: tier_arg,
            arg1: Some(u_1),
            arg2: Some(u_4),
            arg3: None,
        },
        span,
    );
    let t = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::Subtract,
            left: t_clamped,
            right: u_1,
        },
        span,
    );
    let start_ptr = f.expressions.append(
        E::AccessIndex {
            base: gv_expr,
            index: 0,
        },
        span,
    );
    let full_ptr = f.expressions.append(
        E::AccessIndex {
            base: gv_expr,
            index: 1,
        },
        span,
    );
    let start_vec = f.expressions.append(E::Load { pointer: start_ptr }, span);
    let full_vec = f.expressions.append(E::Load { pointer: full_ptr }, span);
    let w_start = f.expressions.append(
        E::Access {
            base: start_vec,
            index: t,
        },
        span,
    );
    let w_full = f.expressions.append(
        E::Access {
            base: full_vec,
            index: t,
        },
        span,
    );
    let lift = f.expressions.append(
        E::Math {
            fun: naga::MathFunction::SmoothStep,
            arg: w_start,
            arg1: Some(w_full),
            arg2: Some(y),
            arg3: None,
        },
        span,
    );
    let inv_t_u = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::Subtract,
            left: u_3,
            right: t,
        },
        span,
    );
    let inv_t_f = f.expressions.append(
        E::As {
            expr: inv_t_u,
            kind: naga::ScalarKind::Float,
            convert: Some(4),
        },
        span,
    );
    let band = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::Multiply,
            left: f_band,
            right: inv_t_f,
        },
        span,
    );
    let inv_lift = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::Subtract,
            left: f_1,
            right: lift,
        },
        span,
    );
    let lift_term = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::Multiply,
            left: f_lift_span,
            right: inv_lift,
        },
        span,
    );
    let mark = f.expressions.append(
        E::Binary {
            op: naga::BinaryOperator::Add,
            left: band,
            right: lift_term,
        },
        span,
    );
    let out_color = f.expressions.append(
        E::Compose {
            ty: vec4f_ty,
            components: vec![f_0, f_0, f_0, mark],
        },
        span,
    );
    let (out_ty, out_binding, out_value) = match &module.types[result.ty].inner {
        T::Struct {
            members,
            span: size,
        } => {
            let (new_members, values): (Vec<_>, Vec<_>) = members
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    !matches!(
                        m.binding,
                        Some(naga::Binding::Location { location, .. }) if location != 0
                    )
                })
                .map(|(i, m)| {
                    let value =
                        if matches!(m.binding, Some(naga::Binding::Location { location: 0, .. })) {
                            out_color
                        } else {
                            f.expressions.append(
                                E::AccessIndex {
                                    base: call,
                                    index: i as u32,
                                },
                                span,
                            )
                        };
                    (m.clone(), value)
                })
                .unzip();
            let struct_ty = module.types.insert(
                naga::Type {
                    name: Some("HzTextHdrMarkFsOut".into()),
                    inner: T::Struct {
                        members: new_members,
                        span: *size,
                    },
                },
                span,
            );
            let composed = f.expressions.append(
                E::Compose {
                    ty: struct_ty,
                    components: values,
                },
                span,
            );
            (struct_ty, None, composed)
        }
        _ => (vec4f_ty, result.binding, out_color),
    };
    f.result = Some(naga::FunctionResult {
        ty: out_ty,
        binding: out_binding,
    });
    f.body
        .push(S::Emit(f.expressions.range_from(start_mark)), span);
    f.body.push(
        S::Return {
            value: Some(out_value),
        },
        span,
    );
    module.entry_points[entry_index].function = f;
    Some(module)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "core_text_expression_tests.rs"]
mod expression_tests;
