use super::*;

pub(in crate::gfx) struct PostPassArgs<'a> {
    pub(in crate::gfx) frame: &'a Frame<'a>,
    pub(in crate::gfx) scaled: Option<(wgpu::TextureView, wgpu::TextureView)>,
    pub(in crate::gfx) view_target: wgpu::TextureView,
    pub(in crate::gfx) target_texture: wgpu::Texture,
    pub(in crate::gfx) surface_frame: Option<&'a wgpu::SurfaceTexture>,
    pub(in crate::gfx) fg_frames: Option<&'a (wgpu::Texture, wgpu::Texture)>,
    pub(in crate::gfx) pack_post: bool,
    pub(in crate::gfx) native_post: bool,
    pub(in crate::gfx) after_pack_sky: bool,
    pub(in crate::gfx) after_pack_features: bool,
    pub(in crate::gfx) after_pack_custom_geometry: bool,
    pub(in crate::gfx) pack_draws_weather: bool,
    pub(in crate::gfx) pack_draws_terrain: bool,
    pub(in crate::gfx) pack_draws_shadow: bool,
    pub(in crate::gfx) pack_draws_outline: bool,
    pub(in crate::gfx) pack_draws_clouds: bool,
    pub(in crate::gfx) pack_draws_hand: bool,
    pub(in crate::gfx) flat_frame: bool,
    pub(in crate::gfx) outline_edges: usize,
    pub(in crate::gfx) cloud_quads: u32,
    pub(in crate::gfx) solid_draws: u32,
    pub(in crate::gfx) translucent_draws: u32,
    pub(in crate::gfx) shadow_first: u32,
    pub(in crate::gfx) shadow_solid: u32,
    pub(in crate::gfx) shadow_translucent: u32,
    pub(in crate::gfx) size: [u32; 2],
    pub(in crate::gfx) jittered: Mat4,
    pub(in crate::gfx) view_proj: Mat4,
    pub(in crate::gfx) hand_view_proj: Mat4,
    pub(in crate::gfx) hand_quads: u32,
    pub(in crate::gfx) made_hand_vertex: i32,
    pub(in crate::gfx) crosshair: u32,
    pub(in crate::gfx) debug_depth: u32,
    pub(in crate::gfx) debug_top: u32,
    pub(in crate::gfx) globals: Globals,
    pub(in crate::gfx) entity_quads: u32,
    pub(in crate::gfx) hand_in_world: bool,
    pub(in crate::gfx) world_quads: u32,
    pub(in crate::gfx) additive_quads: u32,
    pub(in crate::gfx) culling: bool,
    pub(in crate::gfx) gpu_frame: bool,
    pub(in crate::gfx) serial: u64,
    pub(in crate::gfx) cpu: [usize; 3],
    pub(in crate::gfx) sourced_frame: bool,
    pub(in crate::gfx) sourced_cpu: [usize; 3],
    pub(in crate::gfx) lifts_world: bool,
    pub(in crate::gfx) frame_graph: Option<mod_frame_graph::FrameGraphRunner>,
}

pub(in crate::gfx) struct PostPassResult {
    pub(in crate::gfx) readback: Option<(ScreenshotRequest, wgpu::Buffer)>,
    pub(in crate::gfx) target_texture: wgpu::Texture,
    pub(in crate::gfx) fg_made: bool,
}

impl Renderer {
    pub(in crate::gfx) fn run_post_and_overlay(
        &mut self,
        mut encoder: wgpu::CommandEncoder,
        args: PostPassArgs<'_>,
        mut hooks: Option<&mut dyn world_primitives::FrameHooks>,
    ) -> std::result::Result<(wgpu::CommandEncoder, PostPassResult), String> {
        let PostPassArgs {
            frame,
            scaled,
            view_target,
            target_texture,
            surface_frame,
            fg_frames,
            pack_post,
            native_post,
            after_pack_sky,
            after_pack_features,
            after_pack_custom_geometry,
            pack_draws_weather,
            pack_draws_terrain,
            pack_draws_shadow,
            pack_draws_outline,
            pack_draws_clouds,
            pack_draws_hand,
            flat_frame,
            outline_edges,
            cloud_quads,
            solid_draws,
            translucent_draws,
            shadow_first,
            shadow_solid,
            shadow_translucent,
            size,
            jittered,
            view_proj,
            hand_view_proj,
            hand_quads,
            made_hand_vertex,
            crosshair,
            debug_depth,
            debug_top,
            globals,
            entity_quads,
            hand_in_world,
            world_quads,
            additive_quads,
            culling,
            gpu_frame,
            serial,
            cpu,
            sourced_frame,
            sourced_cpu,
            lifts_world,
            mut frame_graph,
        } = args;

        let mut fg_made = false;

        if let Some((color, depth)) = &scaled
            && pack_post
            && let Some((dimension, Some(mut post))) = self.pack_post.take()
        {
            let format = post.formats[0];
            // The standard primitive blend cannot target an integer or unblendable colortex0.
            // On that route the pack world stages have no compatible pass, rather than firing
            // early.
            let pack_blendable = post.colortex0_blendable(&self.device);
            let pack_features = after_pack_features && pack_blendable;
            let pack_opaque_blocks = hooks.is_some() && pack_draws_terrain && pack_blendable;
            let pack_translucent_blocks = hooks.is_some() && pack_draws_terrain && pack_blendable;
            let pack_weather = hooks.is_some() && pack_draws_weather && pack_blendable;
            let pack_custom_geometry = after_pack_custom_geometry && pack_blendable;
            if pack_features
                || pack_opaque_blocks
                || pack_translucent_blocks
                || pack_weather
                || pack_custom_geometry
            {
                self.world_primitives
                    .as_mut()
                    .expect("world primitives")
                    .prepare_pack_pass(
                        &self.device,
                        format,
                        post.rgb & 1 != 0,
                        self.output.frame.linear_light,
                    );
            }

            let size = self.world_post.size().unwrap_or([1, 1]);
            let weather = match &self.hud.atlas_texture {
                Some(atlas) if pack_draws_weather => self
                    .pack_weather
                    .get_or_insert_with(|| pack_weather::PackWeather::new(&self.device))
                    .prepare(
                        &self.device,
                        &mut encoder,
                        |enc, buf, offset, data| self.uploads.buffer(enc, buf, offset, data),
                        atlas,
                        frame.weather,
                        frame.weather_time,
                        &self.indices,
                    ),
                _ => None,
            };
            post.set_weather(weather);
            let flat = self.flat.as_ref().filter(|_| flat_frame);
            self.pack_pages.clear();
            // The pack's terrain programs read their pages: flat, or per section where vertex
            // shaders read storage (`pack_terrain_draws`).
            let reads_pages = flat.is_some() || self.pack_terrain_draws().reads_pages();
            if reads_pages && (pack_draws_terrain || pack_draws_shadow) {
                self.pack_pages.extend(
                    self.arena
                        .page_buffers()
                        .map(|(p, b)| (p, b.clone(), self.arena.sources(p).cloned())),
                );
            }
            let pages = &self.pack_pages;
            // The pack's terrain programs read the normal stream beside each run's vertices
            // (`pack_atlas::TerrainDraws::Normals`).
            let normals = self.pack_terrain_draws() == pack_atlas::TerrainDraws::Normals;
            // Out of `self` for the pass: its boundary closure runs mod frame-graph nodes beside
            // the terrain closure's borrow of the renderer.
            let mut world_primitives = self.world_primitives.take();
            let mut terrain =
                |rp: &mut wgpu::RenderPass<'_>,
                 list: pack_post::TerrainList,
                 page: &dyn Fn(&mut wgpu::RenderPass<'_>, u32),
                 geometry: Option<(u32, &wgpu::BindGroupLayout)>| {
                    use pack_post::TerrainList as L;
                    let shadow = matches!(list, L::ShadowSolid | L::ShadowTranslucent);
                    if !(if shadow {
                        pack_draws_shadow
                    } else {
                        pack_draws_terrain
                    }) {
                        return;
                    }
                    match flat {
                        Some(flat) => {
                            let [shadow, shadow_translucent] = flat.shadow_calls;
                            let calls = match list {
                                L::Solid => &flat.calls[..flat.solid_calls],
                                L::Translucent => flat.translucent_calls(),
                                L::ShadowSolid => &flat.calls[shadow..shadow_translucent],
                                L::ShadowTranslucent => flat.shadow_translucent_calls(),
                            };
                            match geometry {
                                Some((instances, layout)) => {
                                    let indices =
                                        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                            label: Some("Shader pack geometry indices"),
                                            layout,
                                            entries: &[wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: flat.indices.as_entire_binding(),
                                            }],
                                        });
                                    rp.set_bind_group(5, &indices, &[]);
                                    let vertices =
                                        flat_vertices(&self.arena, self.translucent_arena.as_ref());
                                    flat.draw_geometry(rp, calls, vertices, page, instances);
                                }
                                None => flat.draw_with(
                                    rp,
                                    calls,
                                    flat_vertices(&self.arena, self.translucent_arena.as_ref()),
                                    page,
                                ),
                            }
                        }
                        // A geometry stage reads flat draws' indices; without them it draws nothing.
                        None if geometry.is_some() => {}
                        None => {
                            rp.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                            rp.set_vertex_buffer(1, self.instances.slice(..));
                            let (first, count) = match list {
                                L::Solid => (0, solid_draws),
                                L::Translucent => (solid_draws, translucent_draws),
                                L::ShadowSolid => (shadow_first, shadow_solid),
                                L::ShadowTranslucent => {
                                    (shadow_first + shadow_solid, shadow_translucent)
                                }
                            };
                            if normals {
                                let bind = |rp: &mut wgpu::RenderPass<'_>, key: u32| {
                                    rp.set_vertex_buffer(
                                        2,
                                        section_upload::normals_binding_in(
                                            &self.arena,
                                            &self.translucent_arena,
                                            key,
                                        ),
                                    );
                                    page(rp, key);
                                };
                                self.draw_sections_paged(rp, &self.indirect, first, count, &bind);
                            } else {
                                self.draw_sections_paged(rp, &self.indirect, first, count, page);
                            }
                        }
                    }
                };
            // Both strokes of a high contrast outline, the black one first.
            post.set_lines(pack_draws_outline.then(|| pack_post::Lines {
                edges: self.outline.clone(),
                count: outline_edges as u32,
                strokes: if frame.outline_high_contrast { 2 } else { 1 },
            }));
            post.set_clouds(pack_draws_clouds.then(|| pack_post::Clouds {
                vertices: self.cloud_buffer.clone(),
                group: self.cloud_group.clone(),
                indices: self.indices.clone(),
                quads: cloud_quads,
            }));
            if let Some(wp) = world_primitives.as_ref() {
                for &(name, value) in wp.pack_uniforms() {
                    post.set_mod_uniform(name, crate::mod_pipelines::mod_uniform_value(value));
                }
                post.fill_mod_draws(|bound_slots, out| {
                    wp.pack_bound_chunks_into(bound_slots, out);
                });
            } else {
                post.fill_mod_draws(|_, _| {});
            }
            // Fails only on a wait that found the device lost: the frame stops here, with
            // the pack kept for the next one and nothing more encoded.
            let ran = post.run(
                &self.device,
                &self.queue,
                &mut encoder,
                color,
                depth,
                &view_target,
                frame,
                &frame.pack,
                size,
                [self.config.width, self.config.height],
                pages,
                &mut terrain,
                pack_post::WorldBoundaries {
                    sky: after_pack_sky,
                    opaque_blocks: pack_opaque_blocks,
                    features: pack_features,
                    translucent_blocks: pack_translucent_blocks,
                    weather: pack_weather,
                    custom_geometry: pack_custom_geometry,
                },
                |encoder, boundary| {
                    use pack_post::WorldBoundary;
                    if let WorldBoundary::CustomGeometry { color, depth } = boundary {
                        return world_primitives
                            .as_ref()
                            .expect("world primitives")
                            .draw_pack_custom(encoder, color, depth);
                    }
                    if let Some(hooks) = hooks.as_deref_mut() {
                        let sections = &self.visible_renderable_sections;
                        let (stage, color, depth) = match boundary {
                            WorldBoundary::AfterSky => {
                                hooks.on_after_sky(sections, false);
                                execute_pack_fg_slot(
                                    &mut frame_graph,
                                    mod_frame_graph::FrameGraphSlot::AfterStage(
                                        mod_api::RenderLevelStage::AfterSky,
                                    ),
                                    &self.device,
                                    &self.queue,
                                    encoder,
                                    world_primitives.as_mut(),
                                    &self.mod_gpu.render_targets,
                                    &self.transient_targets,
                                );
                                return false;
                            }
                            WorldBoundary::AfterLevel => {
                                hooks.on_after_level(sections, false);
                                execute_pack_fg_slot(
                                    &mut frame_graph,
                                    mod_frame_graph::FrameGraphSlot::AfterStage(
                                        mod_api::RenderLevelStage::AfterLevel,
                                    ),
                                    &self.device,
                                    &self.queue,
                                    encoder,
                                    world_primitives.as_mut(),
                                    &self.mod_gpu.render_targets,
                                    &self.transient_targets,
                                );
                                return false;
                            }
                            WorldBoundary::AfterOpaqueBlocks { color, depth } => {
                                (mod_api::RenderLevelStage::AfterOpaqueBlocks, color, depth)
                            }
                            WorldBoundary::Features {
                                translucent,
                                color,
                                depth,
                            } => (
                                if translucent {
                                    mod_api::RenderLevelStage::AfterTranslucentFeatures
                                } else {
                                    mod_api::RenderLevelStage::AfterOpaqueFeatures
                                },
                                color,
                                depth,
                            ),
                            WorldBoundary::AfterTranslucentBlocks { color, depth } => (
                                mod_api::RenderLevelStage::AfterTranslucentBlocks,
                                color,
                                depth,
                            ),
                            WorldBoundary::AfterWeather { color, depth } => {
                                (mod_api::RenderLevelStage::AfterWeather, color, depth)
                            }
                            WorldBoundary::CustomGeometry { .. } => unreachable!(),
                        };
                        let draws = hooks.on_render_stage(stage, sections, false);
                        let drew = world_primitives
                            .as_ref()
                            .expect("world primitives")
                            .draw_pack_prepared(encoder, color, depth, draws);
                        execute_pack_fg_slot(
                            &mut frame_graph,
                            mod_frame_graph::FrameGraphSlot::AfterStage(stage),
                            &self.device,
                            &self.queue,
                            encoder,
                            world_primitives.as_mut(),
                            &self.mod_gpu.render_targets,
                            &self.transient_targets,
                        );
                        return drew;
                    }
                    false
                },
                &self.device_lost,
                super::super::in_flight::poll_live,
            );
            self.world_primitives = world_primitives;
            // Keep the capacity, not the buffers: a page the arena frees must not stay alive
            // here until the next shader-pack frame (or forever once the pack is off).
            self.pack_pages.clear();
            self.pack_post = Some((dimension, Some(post)));
            ran?;
        } else if let Some((color, depth)) = &scaled
            && native_post
            && let Some(native) = &mut self.native
            && let Some(post) = &mut native.post
        {
            let size = self.world_post.size().unwrap_or([1, 1]);
            let history_reset = native.frame.take_history_reset();
            post.run(
                &self.device,
                &mut encoder,
                (color, depth, &native.frame.surface_view),
                &view_target,
                size,
                history_reset,
                native.timing.as_mut(),
            );
        } else if let (Some((_, generated)), Some((_, depth))) = (fg_frames, &scaled) {
            let camera = frame_gen::Camera {
                jittered,
                view_proj,
                depth: depth.clone(),
                render_size: self.world_post.size().unwrap_or(size),
                jitter: self.world_post.jitter_pixels(),
                reproject: self.frame_gen.camera_motion(view_proj, frame.camera),
                near: NEAR_PLANE,
                far: frame.depth_far,
                fov_y: frame.fov_degrees.to_radians(),
            };
            let overlay = Overlay {
                hand_quads: hand_quads * u32::from(!pack_draws_hand),
                first_hand_vertex: made_hand_vertex,
                crosshair,
                first_crosshair: debug_depth + debug_top,
                world_depth: false,
                real: false,
                hand_drawn: !pack_draws_hand,
                hand_view_proj,
            };
            let onto = (&view_target, &target_texture, generated);
            fg_made =
                self.draw_generated(&mut encoder, frame, &camera, &overlay, onto, lifts_world);
        } else if scaled.is_some() {
            self.world_post.finish(
                &self.queue,
                &mut encoder,
                &view_target,
                self.own_timing.as_mut(),
            );
        }
        if !frame.title {
            self.draw_world_views(&mut encoder, frame, globals, flat_frame, entity_quads);
            let (depth, depth_size) = match &scaled {
                Some((_, depth)) => (depth, self.world_post.size().unwrap_or(size)),
                None => (&self.depth, size),
            };
            native_render::world_drawn(&mut native_render::WorldDrawn {
                device: &self.device,
                queue: &self.queue,
                encoder: &mut encoder,
                color: &target_texture,
                color_view: &view_target,
                color_format: self.output.frame.overlay_format,
                size,
                depth,
                depth_size,
                view_proj,
                jittered,
                camera: frame.camera,
                entities: frame.entities,
                spans: frame.entity_spans,
                motion: frame.entity_motion,
                atlas: self.hud.atlas_texture.as_ref(),
                quad_indices: &self.indices,
                block_sprites: native_render::BlockSpriteArrays {
                    views: &self.sprite_views,
                    split: self.sprite_split,
                    size: self.sprite_textures[0].width(),
                    nearest: &self.sprite_nearest,
                    filtered: &self.sprite_sampler,
                },
            });
        }
        // A 0.2 pack in SDR: its picture finishes on 8 bits before the hand, the camera overlays
        // and the HUD draw over it (`overlay_format`).
        let finished = (self.output.frame.overlay_format != self.output.frame.format)
            .then(|| {
                let image = surface_frame.as_ref().map(|f| &f.texture);
                image.or(self.offscreen.as_ref())
            })
            .flatten()
            .and_then(|image| {
                let screenshot = self.screenshot.is_some();
                self.readback
                    .finish(&self.device, &mut encoder, image, screenshot)
            });
        let (target_texture, view_target, overlay_format) = match finished {
            Some(texture) => {
                let view = texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(self.output.frame.overlay_format),
                    ..Default::default()
                });
                (texture, view, self.output.frame.overlay_format)
            }
            None => (target_texture, view_target, self.output.frame.format),
        };
        let overlay = Overlay {
            hand_quads: hand_quads * u32::from(!pack_draws_hand && !hand_in_world),
            first_hand_vertex: match fg_made && frame_gen::generated_only() && self.hdr.is_some() {
                true => made_hand_vertex,
                false => ((world_quads + additive_quads) * 4) as i32,
            },
            crosshair,
            first_crosshair: debug_depth + debug_top,
            world_depth: scaled.is_none(),
            real: true,
            hand_drawn: !pack_draws_hand,
            hand_view_proj,
        };
        let readback =
            self.draw_overlay(&mut encoder, frame, &view_target, &target_texture, &overlay);
        if culling && let Some(cull) = &mut self.cull {
            cull.end(&self.device, &mut encoder);
        }
        // GPU culling's counts once its world passes are done (phase 2's too), and sourced
        // geometry's beside them; whether the frame drew sourced geometry rides the near
        // copy, so its half waits for sourced geometry's (`in_flight::collect_stats`).
        if gpu_frame && let Some(gpu) = &mut self.gpu_culling {
            gpu.copy_stats(&mut encoder, serial, cpu, sourced_frame);
        }
        if sourced_frame {
            self.sourced.copy_stats(&mut encoder, serial, sourced_cpu);
        }
        if let Some(runner) = frame_graph {
            let mut pool = self
                .transient_targets
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            runner.finish(&mut pool);
        }
        if let Some(timer) = &mut self.gpu_timer {
            timer.end(&self.device, &mut encoder, (&view_target, overlay_format));
        }

        Ok((
            encoder,
            PostPassResult {
                readback,
                target_texture,
                fg_made,
            },
        ))
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_pack_fg_slot(
    frame_graph: &mut Option<mod_frame_graph::FrameGraphRunner>,
    slot: mod_frame_graph::FrameGraphSlot,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    world_primitives: Option<&mut world_primitives::WorldPrimitives>,
    mod_render_targets: &std::sync::Mutex<mod_render_targets::ModRenderTargets>,
    transient_targets: &std::sync::Mutex<mod_frame_graph::TransientTargetPool>,
) {
    if let Some(runner) = frame_graph.as_mut()
        && runner.has_slot(slot)
        && let Some(wp) = world_primitives
    {
        let targets = mod_render_targets.lock().unwrap_or_else(|e| e.into_inner());
        let mut pool = transient_targets.lock().unwrap_or_else(|e| e.into_inner());
        runner.execute_slot(slot, device, queue, encoder, wp, &targets, &mut pool);
    }
}
