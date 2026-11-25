#[path = "frame/shadow.rs"]
mod shadow;
#[path = "frame/sky.rs"]
mod sky;
#[path = "frame/submit.rs"]
mod submit_frame;
use submit_frame::SubmitFrameArgs;
#[path = "frame/acquire.rs"]
mod acquire_frame;
use acquire_frame::AcquiredFrame;
#[path = "frame/post.rs"]
mod post;
#[path = "frame/water_mirror.rs"]
mod water_mirror;
use super::*;
use post::{PostPassArgs, PostPassResult};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SkyStageOwner {
    Engine,
    Inline,
    ClassicPack,
}

impl Renderer {
    /// `checks`: the error scopes of a scaled screenshot's frame, checked before its PNG is written.
    /// `output`: what this frame is drawn for, the window's output or a screenshot's.
    pub(super) fn draw_frame(
        &mut self,
        frame: &Frame,
        checks: Option<GpuChecks>,
        output: output::EffectiveOutput,
        mut hooks: Option<&mut dyn world_primitives::FrameHooks>,
    ) -> Result<bool> {
        debug_assert_eq!(output.encoding, self.hdr.as_ref().map(|hdr| hdr.output));
        // Its steps as spans (SUNRISE_SPIKES): each lap names the step that just ended.
        let mut laps = crate::spans::laps();
        let landing = Instant::now();
        self.land_frame();
        self.refresh_mod_frame_views();
        if let Some(world_primitives) = self.world_primitives.as_mut() {
            world_primitives.pipelines.begin_frame();
        }
        // The frame's one poll takes in what earlier frames read back (GPU times, GPU
        // culling's statistics, occlusion results, a shader pack's centre depth); their
        // collectors don't poll. It, or the previous frame's submission, is often where a
        // device loss shows: this frame then touches the device no more and holds no surface
        // texture, and `render` leaves recovery to its caller.
        let polled =
            super::in_flight::poll_live(&self.device, &self.device_lost, wgpu::PollType::Poll);
        if self.device_lost() {
            return Err(super::in_flight::LOST.into());
        }
        if polled.is_ok()
            && let Some(timer) = &mut self.gpu_timer
        {
            timer.collect(&mut self.gpu_times);
        }
        // Waiting for the previous frame (offscreen: at most two on the GPU), not work.
        let landed = landing.elapsed();
        self.queue.take_waited();
        // World frames alone follow the shader pack: the game loads the picked one
        // before it draws one. Title frames draw no terrain and keep the layout
        // through a pack dropped (Shader Options) or not loaded yet (`Renderer::new`),
        // so the next world's sections, which the level loading screen uploads, stay.
        if !frame.title {
            self.follow_arena_layout();
            self.follow_translucent_layout();
            self.follow_packed_layout();
        }
        // No world runs on a title or loading frame: the text's expression uniforms and
        // `PolyGlobals` are 0 there, whatever the game last gave them.
        if frame.title {
            self.zero_text_blocks();
        }
        let released = self.arena.trim();
        self.graveyard.add(released);
        let left = self.compact_arena();
        if let Some(arena) = &mut self.translucent_arena {
            self.graveyard.add(arena.trim());
        }
        self.graveyard.add(self.sorted_indices.trim());
        arena::free_later(self.graveyard.advance());
        self.shrink_translucent(left);
        if let Some(flat) = &mut self.flat {
            let translucent = self.translucent_arena.as_ref();
            flat.trim(|key| match key.checked_sub(flat::TRANSLUCENT_KEYS) {
                Some(page) => translucent.is_some_and(|a| a.has_page(page)),
                None => self.arena.has_page(key),
            });
        }
        if self.suspended {
            return Ok(false);
        }
        let started = Instant::now();
        let Some(AcquiredFrame {
            fg_frames,
            fg,
            surface_frame,
            reconfigure,
            target_texture,
        }) = self.acquire_frame_surface(frame, landed, &mut laps)?
        else {
            return Ok(false);
        };

        let size = [self.config.width, self.config.height];
        // One command buffer for the frame, after the section copies staged
        // since the last one: Metal commits every command buffer wgpu-core
        // makes (one per run of copies and two per pass) on its own.
        let section_copies = self.section_copies.take();
        let has_section_copies = section_copies.is_some();
        let mut encoder = section_copies.unwrap_or_else(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Sunrise frame"),
                })
        });
        // Sunrise's own passes are timed like a shader pack's, from the frame's first pass.
        if self.own_timing.is_some() != self.pass_timing {
            self.own_timing = self
                .pass_timing
                .then(|| pack_timing::PassTiming::new(&self.device, &self.queue))
                .flatten();
        }
        if let Some(t) = &mut self.own_timing {
            t.gap = self.pass_timing_gap;
            t.keep_frames(self.keep_timed_frames);
            t.set_detail(self.pass_detail);
            // The calibration draw's wait is the one that can find a loss here; the acquired
            // surface texture goes back undrawn and unpresented (`render` leaves recovery to
            // its caller).
            t.begin(
                &self.device,
                &self.queue,
                &self.device_lost,
                super::in_flight::poll_live,
            )?;
        }
        if self.animated_textures {
            self.hud.animate(
                &mut self.uploads,
                &mut encoder,
                frame.ticks,
                &self.animations,
            );
        }
        self.update_lightmap(&mut encoder, frame);
        let views = self.views(frame);
        let pose = frame.posed_bob() * frame.spin;
        let transforms = views.frame_transforms(pose, frame.view_transform);
        let view_proj = transforms.draw_view_proj;
        laps.lap("r_acquire");
        // TAA runs where the world post pass does, not under a shader pack's own.
        let taa = !frame.title
            && self.pack_post.as_ref().is_none_or(|p| p.1.is_none())
            && self.native.as_ref().is_none_or(|n| n.post.is_none());
        let hdr_lighting = self.shader_pack_kind().is_none();
        if let Some(hdr) = &mut self.hdr {
            hdr.lighting = hdr_lighting && hdr.look;
        }
        let hdr_gain = no_pack_hdr_gain(
            self.hdr.as_ref().is_some_and(|h| h.layered()),
            self.shader_pack_kind().is_some(),
            !frame.title,
        );
        let (hdr_settings, hdr_gains) = self
            .hdr
            .as_ref()
            .map_or(([0.0; 4], [0.0; 4]), |h| h.draw_params(!frame.title));
        self.world_post.baked_light(
            hdr_gain,
            hdr_settings,
            hdr_gains,
            self.hdr.as_ref().map_or([[0.0; 4]; 2], hdr::Hdr::windows),
        );
        let jitter = self.world_post.jitter(
            [self.config.width, self.config.height],
            taa,
            view_proj,
            frame.camera,
        );
        let jittered = jitter * view_proj;
        if frame.title {
            self.panorama.prepare(
                &self.device,
                &self.queue,
                self.config.width,
                self.config.height,
                frame.yaw,
                frame.pitch,
            );
        } else {
            self.panorama.release();
        }
        let view = views.primary().view;
        // The hand's bob and view (`hand_globals`).
        let hand_view = frame.posed_bob() * view;
        let sun_angle = frame.sky.sun_angle.to_radians();
        // Celestial bodies rotate around the X axis; angle 0 = noon.
        let sun = Vec3::new(0.0, sun_angle.cos(), -sun_angle.sin());
        let (fog_color, fog) = if let Some(f) = frame.fog {
            (
                f.color,
                [
                    f.environmental_start,
                    f.environmental_end,
                    f.render_distance_start,
                    f.render_distance_end,
                ],
            )
        } else if frame.underwater {
            (
                [
                    frame.water_fog[0],
                    frame.water_fog[1],
                    frame.water_fog[2],
                    1.0,
                ],
                [-8.0, 96.0, -8.0, 96.0],
            )
        } else {
            let d = frame.distance;
            (
                [
                    frame.sky.fog_color[0],
                    frame.sky.fog_color[1],
                    frame.sky.fog_color[2],
                    1.0,
                ],
                // FogRenderer: render-distance fog starts at max(d - 16, 0)... d.
                {
                    let (start, end) = frame.environment_fog.unwrap_or((f32::MAX, f32::MAX));
                    [start, end, (d - d.min(64.0) * 0.25).max(0.0), d]
                },
            )
        };
        let globals = Globals {
            view_proj: jittered.to_cols_array_2d(),
            fog_color,
            fog,
            sky_color: [
                frame.sky.sky_color[0],
                frame.sky.sky_color[1],
                frame.sky.sky_color[2],
                1.0,
            ],
            misc: [
                1.0 - frame.sky.sky_light_factor,
                frame.game_time,
                views.primary().aspect,
                views.primary().tan_half_fov,
            ],
            inverse_view: view.inverse().to_cols_array_2d(),
            sunrise: frame.sky.sunrise_color,
            sun: [sun.x, sun.y, sun.z, frame.sky.star_brightness],
            fog_ends: [
                frame.fog.map_or(frame.distance.min(512.0), |f| f.sky_end),
                frame.fog.map_or(2048.0, |f| f.cloud_end),
                if frame.dark_disc { 1.0 } else { 0.0 },
                frame.glint_alpha,
            ],
            end_portal_sprites: frame.end_portal_sprites,
            crumbling_sprite: frame.crumbling_sprite,
            time: [
                frame.shader_game_time,
                f32::from(frame.nether_lighting),
                frame.dynamic_lights.len().min(crate::dynamic_light::MAX) as f32,
                frame.glint_ticks,
            ],
            outline_params: outline_params(self.config.width, self.config.height),
            dynamic_lights: {
                let mut lights = [[0.0; 4]; crate::dynamic_light::MAX];
                for (to, from) in lights.iter_mut().zip(frame.dynamic_lights) {
                    *to = *from;
                }
                lights
            },
            colored_lights: {
                let mut lights = [[[0.0; 4]; 2]; crate::dynamic_light::MAX];
                for (to, from) in lights.iter_mut().zip(frame.colored_lights) {
                    *to = *from;
                }
                lights
            },
            colored_count: [
                frame.colored_lights.len().min(crate::dynamic_light::MAX) as u32,
                // PvP & HUD's hit colour: entity.wgsl's hurt texel, 0 for vanilla's.
                frame.pvp.hurt_texel,
                // PvP & HUD's glint colour: entity.wgsl's `tinted_glint`, 0 for vanilla's.
                frame.pvp.glint,
                0,
            ],
            emissive_layers: {
                let mut layers = [[0; 4]; crate::native_emissive::MAX];
                for (to, from) in layers.iter_mut().zip(frame.emissive_layers) {
                    *to = *from;
                }
                layers
            },
            emissive_count: [
                frame.emissive_layers.len().min(crate::native_emissive::MAX) as u32,
                0,
                0,
                0,
            ],
            sunrise_fan: sunrise_fan::normals(frame.sky.sunrise_color[3], sun),
            filtering: [
                self.texture_filtering as f32,
                self.sprite_textures[0].width() as f32,
                // The views' last level (`rebuild_terrain_group`), when a sprite is one texel
                // there: RGSS takes one tap from it on (terrain.wgsl's `rotated_grid`).
                if self.sprite_textures[0].width() >> self.mipmaps == 1 {
                    self.mipmaps as f32
                } else {
                    0.0
                },
                0.0,
            ],
            hdr_windows: self.hdr.as_ref().map_or([[0.0; 4]; 2], hdr::Hdr::windows),
            hdr_light: [
                0.0,
                0.0,
                // Only HDR output reads the solid terrain's lighting alpha.
                if self.hdr.is_some() { 1.0 } else { 0.0 },
                f32::from(u8::from(hdr_gain)),
            ],
            hdr_settings,
            hdr_gains,
            view_origin: [0.0; 4],
            world_view: [0.0; 4],
        };
        self.uploads
            .buffer(&mut encoder, &self.globals, 0, bytemuck::bytes_of(&globals));
        // The sunrise fan as sixteen triangles. Its clip positions use the sky's
        // own view (no TAA jitter, bob or spin: `fs_sky`'s `view_ray` has none),
        // and its apex weight interpolates the barycentric the shader used to
        // compute for every sky pixel.
        if frame.sky.sunrise_color[3] > 0.001 && !frame.title && frame.skybox {
            let sky_view_proj = views.primary().view_proj(Mat4::IDENTITY);
            let mut mesh = [[0.0f32; 5]; sunrise_fan::VERTICES];
            for (to, from) in mesh
                .iter_mut()
                .zip(sunrise_fan::mesh(frame.sky.sunrise_color[3], sun))
            {
                let clip = sky_view_proj * glam::Vec4::new(from[0], from[1], from[2], 1.0);
                *to = [clip.x, clip.y, clip.z, clip.w, from[3]];
            }
            self.uploads.buffer(
                &mut encoder,
                &self.sunrise_buffer,
                0,
                bytemuck::cast_slice(&mesh),
            );
        }
        laps.lap("r_targets");
        // GameRenderer.render3dHud: the hand's projection at hudFov, with the
        // bob in its pose rather than the projection (same product).
        let hand_projection = Mat4::from_cols_array(&crate::camera::hand_projection(
            frame.hand_fov_degrees,
            self.config.width as f32,
            self.config.height.max(1) as f32,
            frame.depth_far,
        ));
        let hand_view_proj = hand_projection * frame.posed_bob() * view;
        let hand_globals = Globals {
            view_proj: hand_view_proj.to_cols_array_2d(),
            ..globals
        };
        self.uploads.buffer(
            &mut encoder,
            &self.hand_globals,
            0,
            bytemuck::bytes_of(&hand_globals),
        );
        // GameRenderer.render3dHud: DebugCrosshairRenderer draws with the
        // hand's projection but no bob, and FogMode.NONE (no fog colour).
        let crosshair_globals = Globals {
            view_proj: (hand_projection * view).to_cols_array_2d(),
            fog_color: [0.0; 4],
            ..globals
        };
        self.uploads.buffer(
            &mut encoder,
            &self.crosshair_globals,
            0,
            bytemuck::bytes_of(&crosshair_globals),
        );
        // Frustum culling and camera-relative origins.
        let cull_proj = transforms.cull_view_proj;
        let view = transforms.cull_frustum;
        let camera = frame.camera;
        let camera_section = camera_section(camera, self.layout.min_y());
        // The water mirror draws the same lists: they take in what its camera sees too
        // (above the screen's top, the far shore, the trees against the sky).
        let mirror_plane = frame
            .water_plane
            .filter(|_| !frame.title && self.wants_water_plane());
        let mirror = mirror_plane.map(|plane| {
            Frustum::from_planes(frustum(view::mirror_view_projection(
                cull_proj,
                plane - frame.camera.y as f32,
            )))
        });
        // Solid quads facing away from the eye are left out: the bob moves
        // the eye off `camera`, so it is found from the composed draw projection.
        let rendered_eye = transforms.rendered_eye;
        let eye = rendered_eye.filter(|_| self.sides_on);
        let mirror_low = mirror_plane
            .zip(eye)
            .map(|(plane, eye)| 2.0 * (plane - frame.camera.y as f32) - eye.y);
        let bounded = !frame.title;
        if bounded {
            crate::spans::time("occlusion", || {
                self.start_occlusion(camera, frame.detail_distance, frame.cave_culling)
            });
        }
        // A shader pack's shadow pass draws the sections in its camera's box, all of them (the
        // occlusion graph follows the eye, not the light), within shadowDistanceRenderMul; for
        // an Iris-format pack, only those whose shadow can fall into the camera's view.
        let state = external::FrameState {
            frame,
            output: &output,
        };
        if let Some(pipeline) = self.external.as_mut().filter(|_| !frame.title) {
            pipeline.begin_frame(&state);
        }
        let shadow_view = self
            .pack_post
            .as_ref()
            .and_then(|p| p.1.as_ref())
            .filter(|_| !frame.title)
            .and_then(|p| p.shadow_view(frame))
            .or_else(|| {
                // One of Sunrise's own packs with `[shadow]` uses the same lists.
                let native = self.native.as_ref().filter(|_| !frame.title)?;
                native.shadow.as_ref()?;
                native.frame.shadow_view(frame)
            })
            .or_else(|| {
                self.external
                    .as_ref()
                    .filter(|_| !frame.title)?
                    .shadow_request(&state)
            });
        let pack_terrain = !frame.title
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_terrain());
        // `LevelRenderer.extractSectionDrawGroups`' fade-in time, for the instances.
        let fade_ms = fade_millis(frame.chunk_fade);
        // GPU culling's fading sections, decided at `fades.now`: before the gate, the record
        // check and `prepare`, which sends their flags, and no later than the instances' `now`,
        // so a section the pass draws whole is whole there too. Title frames draw no terrain
        // (a loading screen's fade-in time is 0): they leave them as they are.
        let fades = FadeInputs {
            now: Instant::now(),
            fade_ms,
            uploaded: &self.section_uploaded,
            retiring: &self.lod_retiring,
        };
        if !frame.title
            && let Some(gpu) = &mut self.gpu_culling
        {
            let (sections, rows) = (&mut self.sections, self.layout.sections());
            mark_fades(sections, &mut gpu.records, &mut gpu.fades, rows, fades);
        }
        // The near solid sections culled and drawn on the GPU (`gpu_culling`): never where a
        // shader pack's terrain, a shadow pass or the water mirror draws the CPU's lists, nor
        // under a native pack's `[displacement]`, whose grid takes its quads from the walk's
        // `visible`. Decided before the walk, so that a pass whose buffers no longer fit can
        // still switch GPU culling off.
        let radius = (frame.detail_distance / 16.0) as i32;
        let displaced = self
            .native
            .as_ref()
            .is_some_and(|n| n.displacement.is_some());
        let mut gpu_frame = self.gpu_culling.is_some()
            && !frame.title
            && !pack_terrain
            && shadow_view.is_none()
            && mirror_plane.is_none()
            && !displaced;
        #[cfg(debug_assertions)]
        if gpu_frame {
            if self.gpu_checks.is_multiple_of(64) {
                self.check_gpu_records(fades);
            }
            self.gpu_checks = self.gpu_checks.wrapping_add(1);
        }
        self.frame_serial += 1;
        let serial = self.frame_serial;
        // GPU frames' statistics come back a frame or a few later: the near table's
        // (`GpuCulling::collect`) and sourced geometry's, which drew only in frames the
        // near pass drew, added to the same frame's counts. The halves land in different
        // polls when one pass's ring held no readback free for the frame, and half a
        // frame's counts undercount it, so they publish together (`collect_stats`).
        if let Some(gpu) = &mut self.gpu_culling {
            let near = gpu.collect();
            let sourced = self.sourced.collect();
            if let Some((frame, stats)) = self.collect_stats(near, sourced) {
                self.show_stats(frame, stats);
            }
        }
        // The walk reads the graph GPU culling is given (checked after it).
        #[cfg(debug_assertions)]
        let graph_given = self.occlusion.generation();
        let sourced_frame;
        (gpu_frame, sourced_frame) = self.prepare_gpu_culling(
            gpu_frame,
            camera,
            &view,
            radius,
            eye,
            jittered,
            bounded,
            &mut encoder,
        );
        self.gpu_frame = gpu_frame;
        self.gpu_sourced_frame = sourced_frame;
        let reachable = &self.occlusion;
        let cutout = &self.lod_cutout;
        // Sections fading out (`leave_section`) are past the render distance:
        // drawn there until they have, or they popped out at once.
        self.lod_leaving_scratch.clear();
        self.lod_leaving_scratch
            .extend(self.lod_leaving.iter().map(|&((x, _, z), _)| (x, z)));
        let leaving = &self.lod_leaving_scratch;
        let mut visible = std::mem::take(&mut self.visible);
        visible.clear();
        let lend_sections = !frame.title && hooks.is_some();
        self.visible_renderable_scratch.clear();
        crate::spans::time("visibility", || {
            // GPU culling draws the solid quads of the near whole sections it has records of:
            // within the render distance the walk lists only those with translucent quads,
            // without a record or fading (`SectionGpu::listed`).
            self.sections.walk(
                camera,
                radius,
                &Frusta(view, mirror),
                |k| !bounded || reachable.contains(k),
                |x, z| {
                    (!cutout.is_empty() && cutout.contains(&(x, z)))
                        || (!leaving.is_empty() && leaving.contains(&(x, z)))
                },
                gpu_frame,
                &mut visible,
                lend_sections.then_some(resident::PrimarySelection {
                    frustum: &view,
                    detail_columns: &self.detail_columns,
                    out: &mut self.visible_renderable_scratch,
                }),
            )
        });
        #[cfg(debug_assertions)]
        assert!(
            !gpu_frame || self.occlusion.generation() == graph_given,
            "the walk reads the graph GPU culling was given"
        );
        // Lend the primary selection made by the resident walk; submit before the world passes.
        let hook_oit = !frame.title && self.oit_active_for(self.improved_transparency);
        if !frame.title
            && let Some(hooks) = hooks.as_deref_mut()
        {
            self.visible_renderable_sections.clear();
            self.visible_renderable_sections
                .extend(
                    self.visible_renderable_scratch
                        .iter()
                        .map(|&(_, (x, y, z), slot)| {
                            let origin = [x * 16, self.sections.min_y() + y * 16, z * 16];
                            let [x, y, z] = origin.map(f64::from);
                            mod_api::RenderableSection::new(
                                origin,
                                [[x, y, z], [x + 16.0, y + 16.0, z + 16.0]],
                                slot.is_none_or(|slot| {
                                    let mesh = self.sections.get(slot);
                                    mesh.solid == 0 && mesh.translucent == 0
                                }),
                            )
                        }),
                );
            hooks.on_custom_geometry(&self.visible_renderable_sections, hook_oit);
            hooks.on_frame_graph_setup(&self.visible_renderable_sections, hook_oit);
        }
        let now = std::time::Instant::now();
        let mut visible_lods = std::mem::take(&mut self.visible_lods);
        visible_lods.clear();
        // The animated layers this frame draws, marked as each list is made.
        let mut animated = self
            .animated_textures
            .then_some(&mut self.active_animated_layers);
        if let Some(active) = &mut animated {
            active.fill(0);
        }
        if frame.title {
            visible.clear();
        } else {
            // Far tiles take the frustum test a cell at a time: a cell out of
            // view leaves every tile in it out, and one wholly in view leaves
            // them only their distance. The sort below is the same either way.
            // The walk's complete set drives the water's re-sort, whatever solid
            // draws the GPU owns; a frame the sourced-geometry pass draws in has
            // the CPU draw and instance only what it left it.
            let lods = &self.lods;
            let mut far = std::mem::take(&mut self.translucent_lods);
            far.clear();
            let mut list = |tile| far.push(tile);
            sourced_geometry::gpu::walk_tiles(
                &self.lod_cells,
                lods,
                camera,
                &view,
                now,
                animated,
                &mut list,
            );
            far.sort_unstable_by_key(|v| (v.distance, v.key));
            if sourced_frame {
                visible_lods.extend(far.iter().filter(|v| self.sourced.lists(v.key)).cloned());
                // The filter keeps the walk's order.
            } else {
                visible_lods.append(&mut far);
            }
            self.translucent_lods = far;
        }
        let mut shadow_visible = std::mem::take(&mut self.shadow_visible);
        shadow_visible.clear();
        if let Some(v) = &shadow_view {
            let mut boxed = std::mem::take(&mut self.shadow_boxed_scratch);
            let mut seen = std::mem::take(&mut self.shadow_seen_scratch);
            crate::spans::time("shadow visibility", || {
                collect_shadow_sections(
                    &self.sections,
                    camera,
                    frame.shadow_distance,
                    frame.detail_distance,
                    v,
                    cull_proj,
                    &mut shadow_visible,
                    &mut boxed,
                    &mut seen,
                );
            });
            self.shadow_boxed_scratch = boxed;
            self.shadow_seen_scratch = seen;
            // Nearest the light first, so the depth test rejects what lies under it.
            let light = v.light;
            shadow_visible.sort_unstable_by(|a, b| {
                let height = |s: &resident::Visible| glam::Vec3::from(s.origin).dot(light);
                height(b).total_cmp(&height(a))
            });
            for v in &shadow_visible {
                self.sort_unseen(&mut encoder, v.slot, v.key, camera);
            }
        }
        crate::spans::time("world view visibility", || {
            self.walk_world_views(&mut encoder, frame, radius)
        });
        // A `shaders/` pack that draws the terrain, or its shadow, sees it as Iris does.
        let shaders_pack = !frame.title
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_terrain() || p.has_shadow());
        if shaders_pack {
            let from = pack_terrain_camera(camera);
            let lists = self.world_views.sections.sections_mut();
            for v in visible
                .iter_mut()
                .chain(shadow_visible.iter_mut())
                .chain(lists)
            {
                v.origin = resident::origin(v.key, self.sections.min_y(), from)
                    .as_vec3()
                    .to_array();
            }
        }
        if self.animated_textures {
            // The far tiles' layers were marked as they were listed.
            let lists = self.world_views.sections.sections();
            for v in visible.iter().chain(&shadow_visible).chain(lists) {
                let s = self.sections.get(v.slot);
                mark_layers(&mut self.active_animated_layers, &s.animated_layers);
            }
            // Visibility is collected before terrain draw commands are recorded,
            // so upload the current mask here and keep animation updates ahead
            // of every draw in this command buffer. A GPU frame's walk leaves out
            // the sections GPU culling draws: every layer animates.
            self.animate(&mut encoder, frame.ticks, !frame.title && !gpu_frame);
        }
        let listed = visible.len() + visible_lods.len();
        let viewed = listed + shadow_visible.len();
        let instances = viewed + self.world_views.sections.sections().count();
        if instances as u64 > self.instance_capacity {
            self.instance_capacity = (instances as u64).next_power_of_two();
            self.instances = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Section origins"),
                size: self.instance_capacity * 16,
                usage: wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
        }
        self.ray_prepare(&mut encoder, camera, frame.title);
        self.origins.clear();
        let (sections, retiring) = (&self.sections, &self.lod_retiring);
        laps.lap("r_cameras");
        // `LevelRenderer.extractSectionDrawGroups`: each section's `getVisibility`. Once
        // nothing is fading (the usual frame), every section is whole without a lookup.
        let fading_in = self.sections_fading_in(fade_ms, now);
        let fading_over_lods = self.sections_fading_over_lods(now);
        let fade_code = move |section: &SectionGpu, key: &SectionKey| {
            if fading_over_lods {
                section.fade(retiring.contains(key))
            } else {
                0
            }
        };
        let uploaded = &self.section_uploaded;
        let visibility = move |key: SectionKey| {
            if !fading_in {
                return 1.0;
            }
            section_visibility(uploaded.get(&key), fade_ms, now)
        };
        self.origins.extend(
            visible
                .iter()
                .map(|v| {
                    let code = fade_code(sections.get(v.slot), &v.key);
                    section_instance(v.origin, code, visibility(v.key))
                })
                .chain(visible_lods.iter().map(|v| lod::instance(v.origin, v.fade)))
                .chain(
                    shadow_visible
                        .iter()
                        .map(|v| section_instance(v.origin, 0, visibility(v.key))),
                )
                .chain(self.world_views.sections.sections().map(|v| {
                    let code = fade_code(sections.get(v.slot), &v.key);
                    section_instance(v.origin, code, visibility(v.key))
                })),
        );
        if !self.origins.is_empty() {
            self.uploads.buffer(
                &mut encoder,
                &self.instances,
                0,
                bytemuck::cast_slice(&self.origins),
            );
        }
        // `[displacement]`: the solid quads of the sections it reaches, nearest first.
        if let Some(d) = self.native.as_mut().and_then(|n| n.displacement.as_mut()) {
            let near: smallvec::SmallVec<[displace::Near<'_>; displace::MAX_SECTIONS]> =
                if frame.title {
                    smallvec::SmallVec::new()
                } else {
                    visible
                        .iter()
                        .filter(|v| {
                            let o = glam::Vec3::from(v.origin);
                            let closest = glam::Vec3::ZERO.clamp(o, o + 16.0);
                            closest.length() < d.distance
                        })
                        .filter_map(|v| {
                            let s = self.sections.get(v.slot);
                            (s.solid > 0).then(|| displace::Near {
                                buffer: self.arena.buffer(s.page),
                                base: s.solid_base(),
                                quads: s.solid,
                                origin: v.origin,
                            })
                        })
                        .collect()
                };
            d.prepare(&self.device, &self.queue, &mut encoder, &near, view_proj);
        }
        // Solid sections front to back, then translucent ones back to front,
        // as one list of indirect draws over the section arena.
        self.draws.clear();
        self.draw_pages.clear();
        // Occlusion culling leaves the sections its last result did not see
        // to phase 2, after the others, with instance counts of 0. Whether
        // the frame has draws enough goes by the last one's.
        // A shader pack drawing terrain draws all of it: occlusion culling reads the depth of
        // Sunrise's own terrain draw.
        let mut cull = self
            .cull
            .as_mut()
            .filter(|c| {
                self.cull_on
                    && !frame.title
                    && !pack_terrain
                    && self.multi_draw
                    // Occlusion is found from the camera, not the mirror's eye.
                    && mirror.is_none()
                    // It tests the CPU's lists, which leave out GPU culling's sections.
                    && !gpu_frame
                    && visible.len() >= c.min_sections
            })
            .and_then(|cull| cull.begin().then_some(cull));
        self.phase2_draws.clear();
        self.phase2_pages.clear();
        self.solid_runs.clear();
        self.phase2_runs.clear();
        // On Metal, phase 1 goes the opaque quads of every list, then the cutout
        // ones (`Part`), each solid pass drawing its own: `[..opaque_draws]`,
        // then the rest; phase 2 likewise. A fading list's quads all draw in the
        // cutout pass.
        let split_solid = self.info.backend == wgpu::Backend::Metal;
        let parts = |fading: bool| -> &'static [(u8, Quads)] {
            match (split_solid, fading) {
                (false, _) => &[(0, Quads::All)],
                (true, false) => &[(0, Quads::Opaque), (1, Quads::Cutout)],
                (true, true) => &[(1, Quads::All)],
            }
        };
        let (mut opaque_draws, mut phase2_opaque) = (0, 0);
        debug_assert!(
            !(split_solid && gpu_frame),
            "GPU culling is never offered on Metal"
        );
        // In a GPU frame the GPU draws the solid quads of the whole sections within the
        // render distance that have records (`SectionGpu::gpu_draws_solid`), and the walk
        // listed those among them with translucent quads. Here stay those fading in or out
        // (`gpu_fading`), those without a record and those past the distance (far terrain's
        // cutout, and columns leaving).
        let from = resident::camera_column(camera);
        // Listed sections whose solid quads the pass draws: it counts them among the
        // sections it kept.
        let mut gpu_solid = 0;
        self.translucent_tests.clear();
        self.translucent_tests.resize(visible.len(), cull::NO_TEST);
        for (i, v) in visible.iter().enumerate() {
            let s = self.sections.get(v.slot);
            if gpu_frame
                && s.gpu_draws_solid(crate::streaming::in_view_distance(
                    from, radius, v.key.0, v.key.2,
                ))
            {
                gpu_solid += 1;
                continue;
            }
            let first_phase = cull.as_ref().is_none_or(|c| c.first(v.slot));
            let (draws, pages) = if first_phase {
                (&mut self.draws, &mut self.draw_pages)
            } else {
                (&mut self.phase2_draws, &mut self.phase2_pages)
            };
            let instances = u32::from(first_phase);
            let mut indices = 0;
            let parts = parts(fade_code(s, &v.key) != 0);
            // Each part's (group, first draw, draws).
            let mut runs = [(0, 0, 0); 2];
            for (run, &(group, part)) in runs.iter_mut().zip(parts) {
                let start = draws.len();
                facing_runs_from(&s.sides, eye, mirror_low, v.origin, part, |first, quads| {
                    indices += quads * 6;
                    let base = s.solid_base() + first * 4;
                    draws.push([quads * 6, instances, 0, base, i as u32]);
                    pages.push((s.page, None));
                });
                *run = (group, start, draws.len() - start);
            }
            let runs = &runs[..parts.len()];
            // The test's weight is the translucent quads' indices too: they are the work a
            // hidden section saves.
            let translucent_indices = if s.sorted_indices.is_some() {
                s.translucent * 6
            } else {
                0
            };
            if first_phase {
                for &(group, start, len) in runs {
                    if group == 0 {
                        opaque_draws += len;
                    }
                    self.solid_runs
                        .push_in(group, v.distance, s.page, start, len, [0; 3]);
                }
                if let Some(c) = &mut cull {
                    c.drawn(
                        i,
                        v.slot,
                        indices + translucent_indices,
                        cull::section_box(self.sections.bounds(v.slot)),
                    );
                }
            } else if let Some(c) = &mut cull {
                // One test sees the section's draws in both passes, and its translucent ones.
                let total: usize =
                    runs.iter().map(|r| r.2).sum::<usize>() + usize::from(translucent_indices > 0);
                if let Some(test) = c.deferred(
                    i,
                    v.slot,
                    total,
                    indices + translucent_indices,
                    cull::section_box(self.sections.bounds(v.slot)),
                ) {
                    if translucent_indices > 0 {
                        self.translucent_tests[i] = test;
                    }
                    for &(group, start, len) in runs {
                        if group == 0 {
                            phase2_opaque += len;
                        }
                        let tag = [test, u32::from(group), 0];
                        self.phase2_runs
                            .push_in(group, v.distance, s.page, start, len, tag);
                    }
                }
            }
        }
        // Distant terrain tiles are occlusion tested as sections are, with their meshes'
        // bounds, but for those fading or with a stand-in (`lod::Tile::cull_bounds`).
        // The sourced-geometry pass draws the solid quads of the whole meshes it has records
        // of; listed for their water, their instance stays.
        let mut sourced_gpu_solid = 0;
        for (i, v) in visible_lods.iter().enumerate() {
            if sourced_frame && self.sourced.draws_solid(v.key) {
                sourced_gpu_solid += 1;
                continue;
            }
            let tile = &self.lods[&v.key];
            let s = tile.drawn();
            let index = visible.len() + i;
            let tested = cull.as_ref().and(tile.cull_bounds(v.fade));
            let slot = cull::TILE | tile.cull_slot;
            let first_phase = tested.is_none() || cull.as_ref().is_some_and(|c| c.first(slot));
            let (draws, pages) = if first_phase {
                (&mut self.draws, &mut self.draw_pages)
            } else {
                (&mut self.phase2_draws, &mut self.phase2_pages)
            };
            let instances = u32::from(first_phase);
            let mut indices = 0;
            let parts = parts(v.fade != 0);
            let mut runs = [(0, 0, 0); 2];
            for (run, &(group, part)) in runs.iter_mut().zip(parts) {
                let start = draws.len();
                facing_runs_from(&s.sides, eye, mirror_low, v.origin, part, |first, quads| {
                    indices += quads * 6;
                    let base = s.solid_base() + first * 4;
                    draws.push([quads * 6, instances, 0, base, index as u32]);
                    pages.push((s.page, None));
                });
                *run = (group, start, draws.len() - start);
            }
            let runs = &runs[..parts.len()];
            let (Some(bounds), Some(c)) = (tested, &mut cull) else {
                for &(group, start, len) in runs {
                    if group == 0 {
                        opaque_draws += len;
                    }
                    self.solid_runs
                        .push_in(group, v.distance, s.page, start, len, [0; 3]);
                }
                continue;
            };
            if first_phase {
                for &(group, start, len) in runs {
                    if group == 0 {
                        opaque_draws += len;
                    }
                    self.solid_runs
                        .push_in(group, v.distance, s.page, start, len, [0; 3]);
                }
                c.drawn(index, slot, indices, bounds);
            } else {
                let total: usize = runs.iter().map(|r| r.2).sum();
                if let Some(test) = c.deferred(index, slot, total, indices, bounds) {
                    for &(group, start, len) in runs {
                        if group == 0 {
                            phase2_opaque += len;
                        }
                        let tag = [test, u32::from(group), 0];
                        self.phase2_runs
                            .push_in(group, v.distance, s.page, start, len, tag);
                    }
                }
            }
        }
        // Phase 2's draws are tested where grouping left them.
        let runs = self
            .phase2_runs
            .regroup(&mut self.phase2_draws, &mut self.phase2_pages, 0);
        if let Some(c) = &mut cull {
            for r in runs {
                c.place(r.tag[0], r.tag[1], r.start, r.len);
            }
        }
        self.solid_runs
            .regroup(&mut self.draws, &mut self.draw_pages, 0);
        let phase1_draws = self.draws.len() as u32;
        // Draws `[..opaque_end]` hold opaque quads, `[cutout_start..phase1_draws]` cutout ones.
        let (opaque_end, cutout_start) = if split_solid {
            (opaque_draws, opaque_draws)
        } else {
            (phase1_draws as usize, 0)
        };
        self.draws.extend_from_slice(&self.phase2_draws);
        self.draw_pages.extend_from_slice(&self.phase2_pages);
        let culling = cull.is_some();
        let solid_draws = self.draws.len() as u32;
        // Phase 2's draws `[phase1_draws..phase2_cutout]` hold opaque quads, the rest
        // cutout ones (without the split, all of them go through one pipeline).
        let phase2_cutout = phase1_draws + phase2_opaque as u32;
        laps.lap("r_section_groups");
        // LevelRenderer: nearby sections each frame, plus a rotating eighth
        // (at least 15) of the visible list. Cached views avoid needless sorts.
        self.translucent_draws.clear();
        // Only the camera's own pass takes a section's translucent order with the quads that face
        // away left out: a shadow, a mirrored view or a pack's own pipelines draw them all.
        // The eye it keeps them for is the projection's (`rendered_eye`), not the camera: the bob
        // moves it, and the water mirror draws no translucent terrain.
        let cull_back = shadow_view.is_none()
            && rendered_eye.is_some()
            && !pack_terrain
            && self.native.is_none()
            && self.world_views.sections.sections().next().is_none();
        // The re-sort's own complete set: the near sections and every far tile in view,
        // so its budget and rotation are the same whichever solid draws the GPU owns.
        let far = if sourced_frame {
            &self.translucent_lods
        } else {
            &visible_lods
        };
        let rotation = Rotation::new(visible.len(), far.len(), self.translucency_cursor);
        let eye = camera + rendered_eye.map_or(DVec3::ZERO, |e| e.as_dvec3());
        for (i, v) in visible.iter().enumerate().rev() {
            let s = self.sections.get_mut(v.slot);
            if let Some((index_page, index_start)) = s.sorted_indices {
                let nearby = (v.key.0 - camera_section.0).abs() <= 2
                    && (v.key.1 - camera_section.1).abs() <= 2
                    && (v.key.2 - camera_section.2).abs() <= 2;
                let scheduled = rotation.near(i);
                let origin = [
                    v.key.0 * 16,
                    self.layout.min_y() + v.key.1 * 16,
                    v.key.2 * 16,
                ];
                if (!s.sort.is_sorted() || nearby || scheduled)
                    && let Some(indices) = s.sort.update(
                        camera.to_array(),
                        eye.to_array(),
                        origin,
                        cull_back,
                        &mut self.translucent_sorter,
                    )
                {
                    self.uploads.buffer(
                        &mut encoder,
                        self.sorted_indices.buffer(index_page),
                        index_start * 4,
                        bytemuck::cast_slice(indices),
                    );
                }
                let (page, first) = s.translucent_at;
                let pov = translucency::viewpoint(eye.to_array(), origin);
                let (skip, quads) = s.sort.drawn(s.translucent, cull_back, pov);
                if quads > 0 {
                    self.translucent_draws.push((
                        v.distance,
                        i,
                        [
                            quads * 6,
                            1,
                            (index_start + u64::from(skip) * 6) as u32,
                            first as u32,
                            i as u32,
                        ],
                        (page, Some(index_page)),
                    ));
                }
            }
        }
        // Sections went back to front (`visible` is sorted by distance, then
        // key): tiles' water only needs sorting in when there is some. The walk's
        // complete set is the rotation's; a listed tile's instance slot is its
        // place in what the CPU draws (`visible_lods`), which the tiles whose
        // solid quads the pass drew whole leave out of.
        let section_translucent = self.translucent_draws.len();
        let far_water = FarWater::new(far, &visible_lods, rotation);
        for (i, instance, scheduled) in far_water.entries() {
            let v = &far[i];
            let tile = self.lods.get_mut(&v.key).expect("visible resident tile");
            let s = tile.drawn_mut();
            if let Some((index_page, index_start)) = s.sorted_indices {
                if (!s.sort.is_sorted() || scheduled)
                    && let Some(indices) = s.sort.update_lod(
                        camera.to_array(),
                        v.key.origin(),
                        &mut self.translucent_sorter,
                    )
                {
                    self.uploads.buffer(
                        &mut encoder,
                        self.sorted_indices.buffer(index_page),
                        index_start * 4,
                        bytemuck::cast_slice(indices),
                    );
                }
                let (page, first) = s.translucent_at;
                self.translucent_draws.push((
                    v.distance,
                    instance,
                    [
                        s.translucent * 6,
                        1,
                        index_start as u32,
                        first as u32,
                        instance as u32,
                    ],
                    (page, Some(index_page)),
                ));
            }
        }
        // Near water and distant water share one back-to-front draw order.
        if self.translucent_draws.len() > section_translucent {
            // Distant water goes after the near chunks', and replaced tiles' after all: each is a
            // hair behind the one before, so depth leaves it only where that one has none. Back to
            // front together, a far tile's water under a near chunk's (one fading in, or brought
            // into view by a longer render distance) blended twice and looked opaque. The cost: far
            // water just behind near water shows no surface of its own there.
            self.translucent_draws
                .sort_unstable_by_key(|&(distance, instance, ..)| {
                    far_water.order(distance, instance)
                });
        }
        self.translucent_marks.clear();
        for (k, &(_, instance, draw, pages)) in self.translucent_draws.iter().enumerate() {
            if let Some(&test) = self.translucent_tests.get(instance)
                && test != cull::NO_TEST
            {
                self.translucent_marks.push((k as u32, test));
            }
            self.draws.push(draw);
            self.draw_pages.push(pages);
        }
        self.translucency_cursor = rotation.next();
        let translucent_draws = self.draws.len() as u32 - solid_draws;
        let section_draws = self.draws.len();
        // The shadow camera's sections after the camera's: quads that can face the light (all of
        // them for `every_face`), then translucent ones (in their last sorted order; depth
        // decides there).
        let shadow_first = self.draws.len() as u32;
        let mut shadow_translucent = std::mem::take(&mut self.shadow_translucent_scratch);
        shadow_translucent.clear();
        self.shadow_runs.clear();
        if let Some(view) = &shadow_view {
            let base = listed;
            // How far below the nearest section to the light (squared, as bits, as `PageRuns`
            // takes distances): each band's draws go by page, a call per page and band.
            let height = |v: &resident::Visible| glam::Vec3::from(v.origin).dot(view.light);
            let top = shadow_visible.first().map_or(0.0, height);
            for (k, v) in shadow_visible.iter().enumerate() {
                let s = self.sections.get(v.slot);
                let instance = (base + k) as u32;
                let depth = f64::from(top - height(v)).max(0.0);
                let distance = (depth * depth).to_bits();
                let before = self.draws.len();
                shadow_runs(&s.sides, view, v.origin, |first, quads| {
                    let start = s.solid_base() + first * 4;
                    self.draws.push([quads * 6, 1, 0, start, instance]);
                    self.draw_pages.push((s.page, None));
                });
                let len = self.draws.len() - before;
                self.shadow_runs.push(distance, s.page, before, len, [0; 3]);
                if let Some((index_page, index_start)) = s.sorted_indices {
                    let (page, start) = s.translucent_at;
                    shadow_translucent.push((
                        distance,
                        [
                            s.translucent * 6,
                            1,
                            index_start as u32,
                            start as u32,
                            instance,
                        ],
                        (page, Some(index_page)),
                    ));
                }
            }
        }
        let shadow_solid = self.draws.len() as u32 - shadow_first;
        self.shadow_runs
            .regroup(&mut self.draws, &mut self.draw_pages, shadow_first as usize);
        self.shadow_runs.clear();
        for (distance, draw, pages) in shadow_translucent.drain(..) {
            self.shadow_runs
                .push(distance, pages.0, self.draws.len(), 1, [0; 3]);
            self.draws.push(draw);
            self.draw_pages.push(pages);
        }
        self.shadow_translucent_scratch = shadow_translucent;
        self.shadow_runs.regroup(
            &mut self.draws,
            &mut self.draw_pages,
            (shadow_first + shadow_solid) as usize,
        );
        let shadow_translucent = self.draws.len() as u32 - shadow_first - shadow_solid;
        // Each world view's draws after them (`view_sections`).
        let (lists, draws) = (&mut self.world_views.sections, &mut self.draws);
        lists.push_draws(&self.sections, draws, &mut self.draw_pages, viewed as u32);
        laps.lap("r_nearby");
        // Metal replays a multi-draw as one draw call per entry.
        let mut section_draw_calls = if self.multi_draw && self.info.backend != wgpu::Backend::Metal
        {
            (0..section_draws)
                .filter(|&i| {
                    i == 0
                        || i == phase1_draws as usize
                        || i == solid_draws as usize
                        || self.draw_pages[i] != self.draw_pages[i - 1]
                })
                .count()
        } else {
            section_draws
        };
        // GPU culling's draw calls, one per page; its draws come back with its statistics
        // (`GpuCulling::copy_stats`).
        if gpu_frame && let Some(gpu) = &self.gpu_culling {
            section_draw_calls += gpu.draw_calls();
        }
        // Sourced draw calls are counted here; its kept meshes and solid draws are read back
        // separately (`Sourced::collect`).
        if sourced_frame {
            section_draw_calls += self.sourced.draw_calls();
        }
        let debug_depth = frame.debug_lines.len() as u32;
        let debug_opaque = frame
            .debug_lines
            .iter()
            .filter(|l| debug_line_opaque(l))
            .count() as u32;
        let debug_top = frame.debug_lines_on_top.len() as u32;
        let crosshair = frame.debug_crosshair.len() as u32;
        if debug_depth + debug_top + crosshair > 0 {
            let needed = u64::from(debug_depth + debug_top + crosshair);
            if needed > self.debug_line_capacity {
                self.debug_line_capacity = needed.next_power_of_two();
                self.debug_line_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Debug lines"),
                    size: 32 * self.debug_line_capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            let lines = &mut self.debug_lines_scratch;
            lines.clear();
            lines.extend(
                debug_lines_opaque_first(frame.debug_lines)
                    .chain(frame.debug_lines_on_top)
                    .chain(frame.debug_crosshair)
                    .copied(),
            );
            self.uploads.buffer(
                &mut encoder,
                &self.debug_line_buffer,
                0,
                bytemuck::cast_slice(lines),
            );
        }
        self.weather.prepare(
            &self.device,
            &mut encoder,
            &mut self.uploads,
            frame.weather,
            frame.weather_time,
        );
        let live_hooks = !frame.title && hooks.is_some();
        let pack = self.pack_blocks(frame, fog, globals.fog_ends);
        // Vanilla's four blocks as the pack's world text and GPU particles read them (the block
        // outline's layout), made once a frame and only when one of them asks.
        let vanilla_blocks = std::cell::OnceCell::new();
        let vanilla_blocks = || {
            *vanilla_blocks.get_or_init(|| {
                let view = views.primary();
                core_lines::uniforms(pack.globals, view, pose, jitter, 1.0, fog_color, pack.fog)
            })
        };
        // The pack's world text: vanilla's blocks, HDR tier windows, then its expression uniforms.
        if let Some(core) = &self.core_text {
            core.prepare(
                &mut self.uploads,
                &mut encoder,
                &vanilla_blocks(),
                globals.hdr_windows,
            );
        }
        let outline_edges = frame.outline.len().min(MAX_OUTLINE_EDGES as usize);
        if outline_edges > 0 {
            // The same per-line colour and pixel width layout as the gizmo lines. A high
            // contrast outline is two strokes: the black one first, the bright one after it.
            outline_vertices_into(
                &frame.outline[..outline_edges],
                camera,
                &mut self.outline_edges_scratch,
            );
            let width = outline_width(self.config.width);
            let edges = &self.outline_edges_scratch;
            let lines = &mut self.outline_lines_scratch;
            lines.clear();
            let stroke = |color: u32, width: f32, lines: &mut Vec<[f32; 8]>| {
                for e in edges {
                    let mut line = [0.0; 8];
                    line[..6].copy_from_slice(e);
                    line[6] = f32::from_bits(color);
                    line[7] = width;
                    lines.push(line);
                }
            };
            // A custom outline (PvP & HUD) takes the bright stroke's colour, scales both widths.
            let (custom, scale) = frame.pvp.outline.map_or((None, 1.0), |(c, s)| (Some(c), s));
            if frame.outline_high_contrast {
                stroke(0xff00_0000, 7.0 * scale, lines);
                stroke(custom.unwrap_or(0xff57_ffe1), width * scale, lines);
            } else {
                stroke(custom.unwrap_or(0x6600_0000), width * scale, lines);
            }
            self.uploads
                .buffer(&mut encoder, &self.outline, 0, bytemuck::cast_slice(lines));
            if let Some(core) = &self.core_lines {
                let data = core_lines::uniforms(
                    pack.globals,
                    views.primary(),
                    pose,
                    jitter,
                    globals.outline_params[3],
                    fog_color,
                    pack.fog,
                );
                core.upload(&mut self.uploads, &mut encoder, &data);
            }
        }
        // A pack's core/sky and core/stars: the disc with the sky colour, the stars with their
        // brightness, both turned with the camera only (SkyRenderer's model view).
        if let Some(core) = &self.core_sky
            && !frame.title
        {
            let (block, fog) = (pack.globals, pack.fog);
            let view = views.primary();
            // Far enough for the disc's rim, 512 out: the sky draws without a depth test.
            let fov = (2.0 * view.tan_half_fov.atan()).to_degrees();
            let projection = jitter * world_projection(fov, view.aspect, 2048.0) * pose;
            let [r, g, b, ..] = frame.sky.sky_color;
            let sky =
                core_sky::uniforms(block, projection, view.view, [r, g, b, 1.0], fog_color, fog);
            let stars = frame.stars.map(|(rotation, brightness)| {
                let b = brightness.clamp(0.0, 1.0);
                core_sky::uniforms(
                    block,
                    projection,
                    view.view * rotation,
                    [b; 4],
                    fog_color,
                    fog,
                )
            });
            core.prepare(&mut self.uploads, &mut encoder, &sky, stars.as_deref());
        }
        let view_target = target_texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.output.frame.format),
            ..Default::default()
        });
        // The world's targets: the screen, or the scaled target of `post`.
        let pack_post = !frame.title && self.pack_post.as_ref().is_some_and(|p| p.1.is_some());
        // One of Sunrise's own packs reads the same world target.
        let native_post = !frame.title && self.native.as_ref().is_some_and(|n| n.post.is_some());
        let pack_reads_world = pack_post || native_post;
        let pack_draws_sky = pack_post
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_sky());
        let pack_draws_celestial = pack_post
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_celestial());
        // The pack's `sky`, `stars`, `sun` and `moon`: the parts no pack program draws are the
        // game's, which leaves the same ones out.
        let sky_switches = self
            .pack_post
            .as_ref()
            .and_then(|p| p.1.as_ref())
            .map(|p| p.sky_switches())
            .unwrap_or_default();
        let scaled = self.world_post.prepare(
            &self.device,
            [self.config.width, self.config.height],
            pack_reads_world || fg,
        );
        let pack_world_runs = pack_post && scaled.is_some();
        let after_pack_features = live_hooks
            && pack_world_runs
            && self
                .pack_post
                .as_ref()
                .and_then(|(_, post)| post.as_ref())
                .is_some_and(|post| post.owns_feature_stages());
        // Notification follows the last owner of the logical sky pass, independent of
        // whether any of that owner's geometry is visible. Drawing flags stay separate:
        // a celestial-only pack leaves the engine's disc, sunset, and stars intact.
        let pack_owns_sky_stage = pack_world_runs
            && (pack_draws_sky || pack_draws_celestial)
            && (!frame.end_sky || end_fog_sky(frame));
        let wants_after_sky = live_hooks
            && hooks
                .as_deref()
                .is_some_and(|hooks| hooks.wants_after_sky());
        let sky_stage = if !wants_after_sky || !frame.sky_pass {
            None
        } else if pack_owns_sky_stage {
            Some(SkyStageOwner::ClassicPack)
        } else if pack_draws_sky {
            Some(SkyStageOwner::Inline)
        } else {
            Some(SkyStageOwner::Engine)
        };
        let pack_sky_clear = pack_draws_sky
            .then(|| self.pack_post.as_ref().and_then(|p| p.1.as_ref()))
            .flatten()
            .and_then(|p| p.sky_clear());
        let clear = if let Some([r, g, b, a]) = pack_sky_clear {
            wgpu::Color {
                r: f64::from(r),
                g: f64::from(g),
                b: f64::from(b),
                a: f64::from(a),
            }
        } else if frame.title || (frame.end_sky && !frame.underwater && sky_open(frame)) {
            wgpu::Color::BLACK
        } else {
            wgpu::Color {
                r: f64::from(fog_color[0]),
                g: f64::from(fog_color[1]),
                b: f64::from(fog_color[2]),
                a: 1.0,
            }
        };
        let clear = linear_blend::stored_clear(clear, self.output.frame.linear_light);
        let sky_visible = sky_open(frame);
        if let Some((rotation, brightness)) = frame.stars {
            self.stars.prepare(
                &mut self.uploads,
                &mut encoder,
                jittered * rotation,
                brightness,
                &globals,
            );
        }
        self.custom_sky.prepare(
            &mut self.uploads,
            &mut encoder,
            jittered,
            &self.custom_sky_draws,
            &globals,
        );
        let celestial_vertices = frame.celestial.len().min(2048 * 4);
        if frame.end_sky && !frame.title {
            self.end_sky.prepare(&self.device, frame.end_sky_sprite);
        }
        if celestial_vertices > 0 {
            self.uploads.buffer(
                &mut encoder,
                &self.celestial_buffer,
                0,
                bytemuck::cast_slice(&frame.celestial[..celestial_vertices]),
            );
        }
        // One of Sunrise's own packs: this frame's inputs; in a timed frame its sky draws in a
        // pass of its own, so that it has a time of its own.
        let mut native_sky_pass = false;
        if !frame.title
            && let Some(native) = &mut self.native
        {
            let size = self
                .world_post
                .size()
                .filter(|_| scaled.is_some())
                .unwrap_or([self.config.width, self.config.height]);
            native
                .frame
                .write(&self.queue, frame, size, output.display());
            native.frame.block_light.update(
                &self.queue,
                frame.camera,
                &self.light_blocks,
                self.light_revision,
            );
            if let Some(t) = &mut native.timing {
                t.begin(
                    &self.device,
                    &self.queue,
                    &self.device_lost,
                    super::in_flight::poll_live,
                )?;
                native_sky_pass =
                    native.sky.is_some() && t.recording() && sky_visible && frame.skybox;
            }
        }
        // Complete the separate engine sky pass before buffer preparation only when a
        // listener subscribes to `RenderLevelStage::AfterSky`. The main world pass
        // loads that colour and starts its own depth as usual. A classic pack's replacement
        // sky draws and notifies after the engine world import in PackPost instead.
        let separate_sky =
            wants_after_sky && sky_visible && (frame.skybox || frame.end_sky) && !pack_draws_sky;
        if separate_sky {
            if let Some(timer) = &mut self.gpu_timer {
                timer.begin(&mut encoder);
            }
            let (color, depth) = match &scaled {
                Some((color, depth)) => (color, depth),
                None => (&view_target, &self.depth),
            };
            if native_sky_pass {
                Self::draw_timed_native_sky(
                    &mut self.native,
                    &self.device,
                    &self.group,
                    &mut encoder,
                    (color, depth, self.output.frame.format),
                    clear,
                );
            }
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Sunrise sky"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: if native_sky_pass {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(clear)
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            let mut pass = pack_timing::CountedPass::plain(pass);
            pass.set_bind_group(0, &self.group, &[]);
            self.draw_sky_pass_content(
                &mut pass,
                frame,
                sky_visible,
                false,
                native_sky_pass,
                celestial_vertices,
                pack_draws_celestial,
                sky_switches,
                hdr_gain,
            );
        }
        self.hook_geometry.entities.clear();
        self.hook_geometry.entity_spans.clear();
        self.hook_geometry.see_through.clear();
        self.hook_geometry.prepared_draws.clear();
        self.hook_geometry.text_patches.clear();
        if live_hooks && let Some(hooks) = hooks.as_deref_mut() {
            if sky_stage == Some(SkyStageOwner::Engine) {
                hooks.on_after_sky(&self.visible_renderable_sections, hook_oit);
            }
            hooks.on_prepare_render_buffers(
                &self.visible_renderable_sections,
                hook_oit,
                &mut self.hook_geometry,
            );
            self.prepared_draws
                .append(&mut self.hook_geometry.prepared_draws);
        }
        let mut frame_graph = if live_hooks
            && let Some(hooks) = hooks.as_deref_mut()
            && let Some(recorded) = hooks.take_frame_graph()
        {
            let mut warned = self
                .mod_gpu
                .warned_frame_graph_cycles
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            mod_frame_graph::FrameGraphRunner::prepare(recorded, hook_oit, &mut warned)
        } else {
            None
        };
        // Upload both mods' stage batches and deferred geometry through the same path, before
        // the first consuming pass. Stage listeners may only reference these prepared ranges.
        let world_primitives = !frame.title && {
            let batch = hooks
                .as_deref()
                .map_or(frame.world_primitives, |hooks| hooks.prepared_primitives());
            let mod_blocks = (!batch.pipelines.runs.is_empty()
                || !batch.pipelines.draws.is_empty()
                || frame_graph.is_some())
            .then(vanilla_blocks);
            // Mods' queued storage buffer writes land in call order before anything draws.
            self.flush_mod_storage();
            let prepared = self
                .world_primitives
                .as_mut()
                .expect("world primitives")
                .prepare(
                    &self.device,
                    &mut |buffer, bytes| self.uploads.buffer(&mut encoder, buffer, 0, bytes),
                    &batch,
                    jittered,
                    [self.config.width, self.config.height],
                    &self.entity_atlas,
                    mod_blocks.as_ref(),
                );
            let released = self.take_released_mod_storage();
            self.world_primitives
                .as_mut()
                .expect("world primitives")
                .forget_storage(&released);
            let released_targets = self.take_released_mod_render_targets();
            self.world_primitives
                .as_mut()
                .expect("world primitives")
                .forget_render_targets(&released_targets);
            let bound = self
                .pack_post
                .as_ref()
                .and_then(|(_, post)| post.as_ref())
                .map_or(&[][..], |post| post.bound_mod_slots());
            self.world_primitives
                .as_mut()
                .expect("world primitives")
                .set_bound_mod_slots(bound);
            prepared || live_hooks
        };
        if frame_graph.is_some() {
            post::execute_pack_fg_slot(
                &mut frame_graph,
                mod_frame_graph::FrameGraphSlot::BeforeWorld,
                &self.device,
                &self.queue,
                &mut encoder,
                self.world_primitives.as_mut(),
                &self.mod_gpu.render_targets,
                &self.transient_targets,
            );
            if sky_stage == Some(SkyStageOwner::Engine) {
                post::execute_pack_fg_slot(
                    &mut frame_graph,
                    mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterSky,
                    ),
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    self.world_primitives.as_mut(),
                    &self.mod_gpu.render_targets,
                    &self.transient_targets,
                );
            }
        }
        // Polytone GPU particles with their own shader: the vanilla blocks as the block outline's
        // (a particle's ModelViewMat is the camera's rotation, its positions camera relative).
        self.core_particles
            .prepare_frame(&self.device, &self.queue, frame.title, vanilla_blocks);
        let skin = frame.skin.filter(|skin| match &mut self.entity_skin {
            Some(skinner) => skinner.register(&self.device, &mut self.uploads, &mut encoder, skin),
            None => false,
        });
        let pack_skin = !frame.title
            && skin.is_some()
            && self
                .pack_post
                .as_ref()
                .and_then(|(_, post)| post.as_ref())
                .is_some_and(|post| post.skins_entities());
        // While a shader pack draws entities, the runs of `entity_spans` its programs draw are
        // left out of Sunrise's own entity draw.
        let merged_entities: &[Vertex] = if self.hook_geometry.entities.is_empty() {
            frame.entities
        } else {
            self.hook_entities_scratch.clear();
            self.hook_entities_scratch.extend_from_slice(frame.entities);
            self.hook_entities_scratch
                .append(&mut self.hook_geometry.entities);
            &self.hook_entities_scratch
        };
        let merged_see_through: &[Vertex] = if self.hook_geometry.see_through.is_empty() {
            frame.see_through
        } else {
            self.hook_see_through_scratch.clear();
            self.hook_see_through_scratch
                .extend_from_slice(frame.see_through);
            self.hook_see_through_scratch
                .append(&mut self.hook_geometry.see_through);
            &self.hook_see_through_scratch
        };
        let mut entities = merged_entities;
        let mut pack_entities = false;
        if !frame.title
            && let Some((_, Some(post))) = &mut self.pack_post
            && post.has_entities()
        {
            pack_entities = true;
            let spans = if self.hook_geometry.entity_spans.is_empty() {
                frame.entity_spans
            } else {
                for span in &mut self.hook_geometry.entity_spans {
                    span.start += frame.entities.len() as u32;
                }
                let before_hooks = frame
                    .entity_spans
                    .partition_point(|s| s.start < frame.entities.len() as u32);
                drop(
                    self.hook_geometry
                        .entity_spans
                        .splice(..0, frame.entity_spans[..before_hooks].iter().copied()),
                );
                &self.hook_geometry.entity_spans
            };
            post.prepare_entities(
                &self.device,
                &self.queue,
                merged_entities,
                spans,
                frame.crumbling_sprite,
                frame.end_portal_sprites[1],
                &mut self.pack_entity_rest,
                skin.filter(|_| pack_skin),
                (frame.entity_motion, frame.entity_motion_posed),
            );
            // `RenderTypes.entitySolid` of a mod's texture: the pack's program draws it with that
            // texture bound as the entity texture.
            post.resolve_entity_textures(|handle| {
                self.dynamic_entity.view(
                    &self.device,
                    &mut self.uploads,
                    &mut encoder,
                    self.hud.dynamic(),
                    handle,
                )
            });
            entities = &self.pack_entity_rest;
        }
        self.hook_geometry.entity_spans.clear();
        // GameRenderer.useImprovedTransparency. A shader pack keeps its own
        // path.
        let oit_on = !frame.title && self.oit_active_for(self.improved_transparency);
        // The posed models' vertices come first: the ordinary entities' draw takes them in.
        let posed = if pack_skin {
            0
        } else {
            skin.map_or(0, |s| s.vertices)
        };
        let outlined = skin.map_or(0, |s| s.outline_vertices);
        let entity_quad_capacity = self.entity_quad_capacity();
        let reserved_skin_quads = (posed / 4) as usize + (outlined / 4) as usize;
        let combined_quad_budget = entity_quad_capacity
            .max_quads()
            .saturating_sub(reserved_skin_quads);
        let reserved_halfway_hand_quads = if fg {
            (frame.hand.len() / 4).min(combined_quad_budget)
        } else {
            0
        };
        let batch_quad_budget = combined_quad_budget.saturating_sub(reserved_halfway_hand_quads);
        let batch_counts = self.entity_batch.update_within(
            entities,
            frame.additive,
            frame.hand,
            merged_see_through,
            frame.shadow_only,
            oit_on,
            batch_quad_budget,
        );
        let [
            entity_quads,
            text_quads,
            crumbling_quads,
            translucent_crumbling_quads,
            water_mask_quads,
            swirl_quads,
            shadow_quads,
            translucent_particle_quads,
            translucent_item_quads,
            oit_entity_quads,
            fluid_quads,
            world_text_quads,
            world_text_offset_quads,
            native_solid_quads,
            native_cutout_quads,
            native_cutout_mipped_quads,
            native_translucent_quads,
            banner_pattern_quads,
            glint_only_quads,
            text_offset_solid_quads,
            text_normal_solid_quads,
            dynamic_solid_quads,
            dynamic_cutout_quads,
            dynamic_translucent_quads,
        ] = batch_counts.materials;
        let dynamic_quads = dynamic_solid_quads + dynamic_cutout_quads + dynamic_translucent_quads;
        let entities = &entities[..batch_counts.admitted_entity_vertices];
        let additive_quads = batch_counts.additive;
        let hand_quads = batch_counts.hand;
        let see_through_quads = batch_counts.see_through;
        let shadow_only_quads = batch_counts.shadow_only;
        let ordinary_skin = skin.filter(|_| !pack_skin);
        self.entity_order.prepare(entities, ordinary_skin, oit_on);
        let entity_quads = entity_quads + posed / 4;
        let weather_quads = frame.weather.len() as u32 / 4;
        let text_solid_quads = text_offset_solid_quads + text_normal_solid_quads;
        let world_quads = entity_quads
            + text_quads
            + crumbling_quads
            + translucent_crumbling_quads
            + water_mask_quads
            + swirl_quads
            + shadow_quads
            + translucent_particle_quads
            + translucent_item_quads
            + oit_entity_quads
            + fluid_quads
            + world_text_quads
            + world_text_offset_quads
            + native_solid_quads
            + native_cutout_quads
            + native_cutout_mipped_quads
            + native_translucent_quads
            + dynamic_quads
            + banner_pattern_quads
            + glint_only_quads
            + text_solid_quads;
        laps.lap("r_draw_lists");
        // Where the tail of materials the classic and the improved passes
        // both draw ends: the shadows, particles and items count back from it.
        let base_quads = world_quads
            - oit_entity_quads
            - fluid_quads
            - world_text_quads
            - world_text_offset_quads
            - native_solid_quads
            - native_cutout_quads
            - native_cutout_mipped_quads
            - native_translucent_quads
            - dynamic_quads
            - banner_pattern_quads
            - glint_only_quads
            - text_solid_quads;
        // Dynamic-texture quads sit after the terrain models and before banner patterns: their
        // first quad in the entity buffer, and their runs with one bind group each.
        let dynamic_first = world_quads
            - text_solid_quads
            - glint_only_quads
            - banner_pattern_quads
            - dynamic_quads;
        if dynamic_quads > 0 {
            let section_start = (dynamic_first - posed / 4) as usize * 4;
            self.dynamic_entity.prepare(
                &self.device,
                &mut self.uploads,
                &mut encoder,
                self.hud.dynamic(),
                &self.entity_batch.combined
                    [section_start..section_start + dynamic_quads as usize * 4],
                dynamic_first,
            );
        }
        let native_clouds = self.native.as_ref().is_none_or(|n| n.vanilla_clouds);
        // A pack's core/clouds, decided before either mesh is prepared: its faces, or none and
        // the native mesh in this same frame.
        let pack_clouds = core_clouds::CloudsFrame {
            pack_shader: self.core_clouds.as_ref().is_some_and(|c| c.classic()),
            pack_oit_shader: (self.core_clouds.as_ref()).is_some_and(|c| c.improved_transparency()),
            clouds: frame.clouds.is_some(),
            native_clouds,
            improved_transparency: oit_on,
            shader_pack: self.pack_post.as_ref().is_some_and(|p| p.1.is_some()),
            title: frame.title,
            cloud_marks: light_marks(self.output.frame.format, self.hdr.is_some())
                && self.hdr.as_ref().is_some_and(|h| h.lighting),
        }
        .pack_draws();
        let pack_cloud_quads = match (self.core_clouds.as_mut(), frame.clouds.as_ref()) {
            (Some(core), Some(input)) if pack_clouds => {
                let view = views.primary();
                let vanilla = core_lines::uniforms(
                    pack.globals,
                    view,
                    pose,
                    jitter,
                    1.0,
                    fog_color,
                    pack.fog,
                );
                core.prepare(
                    &self.device,
                    &mut self.uploads,
                    &mut encoder,
                    &vanilla,
                    input,
                )
                .map_err(|e| {
                    log::warn!(
                        "the resource pack's clouds are drawn natively this frame: their faces \
                             take {} bytes, more than the {} this device binds",
                        e.bytes,
                        e.limit
                    )
                })
                .ok()
            }
            _ => None,
        };
        let native_input = frame
            .clouds
            .as_ref()
            .filter(|_| core_clouds::native_mesh(native_clouds, pack_cloud_quads));
        let cloud_quads = if let Some(input) = native_input {
            let (rebuilt, uniform) =
                crate::spans::time("clouds", || self.cloud_mesh.prepare(input));
            let vertices = self.cloud_mesh.vertices.len() as u64;
            if rebuilt && vertices > 0 {
                if vertices > self.cloud_capacity {
                    self.cloud_capacity = vertices.next_power_of_two();
                    self.cloud_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Cloud vertices"),
                        size: self.cloud_capacity * std::mem::size_of::<Vertex>() as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                }
                self.uploads.buffer(
                    &mut encoder,
                    &self.cloud_buffer,
                    0,
                    bytemuck::cast_slice(&self.cloud_mesh.vertices),
                );
            }
            self.uploads.buffer(
                &mut encoder,
                &self.cloud_uniform,
                0,
                bytemuck::bytes_of(&uniform),
            );
            (vertices / 4) as u32
        } else {
            0
        };
        self.lightning.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            &frame.lightning,
            camera,
        );
        // Frame generation's made frame draws its own hand, halfway, after the others.
        let made_hand_vertex = (posed as usize + self.entity_batch.combined.len()) as i32;
        if fg {
            let halfway_hand = &frame.hand[..hand_quads as usize * 4];
            self.frame_gen
                .halfway_hand(halfway_hand, hand_view, &mut self.entity_batch.combined);
            self.frame_gen.track_entities(
                &self.device,
                &self.entity_atlas,
                frame.entities,
                frame.entity_motion,
                frame.camera,
            );
        }
        let all_entities = &self.entity_batch.combined;
        // Glowing entities' silhouettes posed on the GPU go last, past every
        // material (the outline pass draws them from there).
        let outline_base = posed + all_entities.len() as u32;
        let entity_vertices = u64::from(outline_base) + u64::from(outlined);
        if entity_vertices > self.entity_capacity {
            self.entity_capacity =
                entity_buffer_vertices(entity_vertices, entity_quad_capacity.vertex_bytes);
            self.entity_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Entity vertices"),
                size: self.entity_capacity * std::mem::size_of::<Vertex>() as u64,
                usage: entity_skin::EntitySkin::usage(self.entity_skin.is_some()),
                mapped_at_creation: false,
            });
        }
        for (dst, range) in self.entity_order.cpu_uploads(all_entities.len() as u32) {
            self.uploads.buffer(
                &mut encoder,
                &self.entity_buffer,
                u64::from(dst) * std::mem::size_of::<Vertex>() as u64,
                bytemuck::cast_slice(&all_entities[range.start as usize..range.end as usize]),
            );
        }
        if pack_entities {
            // The pack replaced `frame.entities`. A staged cut indexes the original stream,
            // and the game does not stage one while that pack is loaded.
            self.prepared_draws.clear();
        }
        let prepared_count = self.prepared_draws.len() as u64;
        let stride = std::mem::size_of::<PreparedModelInstance>() as u64;
        let slots = prepared_model::instance_slot_capacity(
            prepared_count,
            stride,
            self.device.limits().max_buffer_size,
        )?;
        if slots > self.prepared_instance_capacity {
            let checks = GpuChecks::push(&self.device);
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Prepared model instances"),
                size: slots * stride,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            if let Some(error) = checks.pop() {
                return Err(error.to_string().into());
            }
            self.prepared_instance_buffer = buffer;
            self.prepared_instance_capacity = slots;
        }
        if prepared_count > 0 {
            self.prepared_uploaded.clear();
            self.prepared_uploaded
                .extend(self.prepared_draws.iter().map(|draw| draw.instance));
            self.uploads.buffer(
                &mut encoder,
                &self.prepared_instance_buffer,
                0,
                bytemuck::cast_slice(&self.prepared_uploaded),
            );
        }
        self.prepared_ranges.clear();
        if prepared_count > 0 {
            entity_batch::ordinary_vertices_at_cuts(
                entities,
                self.prepared_draws.iter().map(|draw| draw.cpu_vertices),
                oit_on,
                &mut self.prepared_cuts,
            );
            for cut in &mut self.prepared_cuts {
                *cut = self.entity_order.prepared_cut(*cut);
            }
            prepared_model::fill_instance_ranges(
                entity_quads * 4,
                &self.prepared_draws,
                &self.prepared_cuts,
                0,
                &mut self.prepared_ranges,
            );
        }
        if let (Some(skinner), Some(skin)) = (&mut self.entity_skin, skin) {
            if pack_skin {
                let target = self
                    .pack_post
                    .as_ref()
                    .and_then(|(_, post)| post.as_ref())
                    .and_then(|post| post.skin_target())
                    .expect("GPU pack entity target");
                skinner.dispatch_pack(
                    &self.device,
                    &mut self.uploads,
                    &mut encoder,
                    skin,
                    target,
                    &self.entity_buffer,
                    outline_base,
                );
            } else {
                skinner.dispatch(
                    &self.device,
                    &mut self.uploads,
                    &mut encoder,
                    skin,
                    &self.entity_buffer,
                    self.entity_order.posed_jobs(skin),
                    outline_base,
                );
            }
        }
        let static_sections = frame.static_block_entities;
        let mut static_bytes_uploaded = 0;
        let catalog_revision = frame.static_block_entity_stats.snapshot_revision;
        if catalog_revision != self.static_entity_catalog_revision {
            self.static_entity_sections.retain(|section, _| {
                static_sections
                    .binary_search_by_key(section, |snapshot| snapshot.section)
                    .is_ok()
            });
            for snapshot in static_sections {
                let vertices = snapshot.vertices.len() as u64;
                let gpu = self
                    .static_entity_sections
                    .entry(snapshot.section)
                    .or_insert_with(|| {
                        let capacity = vertices.next_power_of_two().max(4);
                        let origin_buffer =
                            self.device
                                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("Static block-entity section origin"),
                                    contents: bytemuck::bytes_of(&snapshot.origin),
                                    usage: wgpu::BufferUsages::VERTEX,
                                });
                        static_bytes_uploaded += std::mem::size_of_val(&snapshot.origin) as u64;
                        GpuStaticEntitySection {
                            buffer: self.device.create_buffer(&wgpu::BufferDescriptor {
                                label: Some("Static block-entity section vertices"),
                                size: capacity * std::mem::size_of::<Vertex>() as u64,
                                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                                mapped_at_creation: false,
                            }),
                            origin_buffer,
                            origin: snapshot.origin,
                            capacity,
                            revision: 0,
                            vertices: 0,
                        }
                    });
                if gpu.origin != snapshot.origin {
                    gpu.origin_buffer =
                        self.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("Static block-entity section origin"),
                                contents: bytemuck::bytes_of(&snapshot.origin),
                                usage: wgpu::BufferUsages::VERTEX,
                            });
                    gpu.origin = snapshot.origin;
                    static_bytes_uploaded += std::mem::size_of_val(&snapshot.origin) as u64;
                }
                if gpu.revision == snapshot.revision {
                    continue;
                }
                if vertices > gpu.capacity {
                    gpu.capacity = vertices.next_power_of_two();
                    gpu.buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Static block-entity section vertices"),
                        size: gpu.capacity * std::mem::size_of::<Vertex>() as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                }
                self.uploads.buffer(
                    &mut encoder,
                    &gpu.buffer,
                    0,
                    bytemuck::cast_slice(&snapshot.vertices),
                );
                static_bytes_uploaded += vertices * std::mem::size_of::<Vertex>() as u64;
                gpu.revision = snapshot.revision;
                gpu.vertices = vertices as u32;
            }
            self.static_entity_catalog_revision = catalog_revision;
        }
        let static_entity_quads = frame
            .visible_static_block_entity_sections
            .iter()
            .filter_map(|section| self.static_entity_sections.get(section))
            .map(|section| section.vertices / 4)
            .sum::<u32>();
        let max_static_section_quads = frame
            .visible_static_block_entity_sections
            .iter()
            .filter_map(|section| self.static_entity_sections.get(section))
            .map(|section| section.vertices / 4)
            .max()
            .unwrap_or(0);
        if max_static_section_quads > self.static_entity_index_quads {
            self.static_entity_index_quads = max_static_section_quads.next_power_of_two();
            self.static_entity_indices =
                Self::quad_indices(&self.device, self.static_entity_index_quads);
        }
        if !frame.visible_static_block_entity_sections.is_empty()
            && self.static_entity_camera != Some(frame.camera)
        {
            let camera_floor = [
                frame.camera.x.floor() as i32,
                frame.camera.y.floor() as i32,
                frame.camera.z.floor() as i32,
            ];
            let transform = StaticEntityUniform {
                camera_block: [camera_floor[0], camera_floor[1], camera_floor[2], 0],
                camera_fraction: [
                    (frame.camera.x - f64::from(camera_floor[0])) as f32,
                    (frame.camera.y - f64::from(camera_floor[1])) as f32,
                    (frame.camera.z - f64::from(camera_floor[2])) as f32,
                    0.0,
                ],
            };
            self.uploads.buffer(
                &mut encoder,
                &self.static_entity_transform,
                0,
                bytemuck::bytes_of(&transform),
            );
            static_bytes_uploaded += std::mem::size_of::<StaticEntityUniform>() as u64;
            self.static_entity_camera = Some(frame.camera);
        }
        self.static_entity_frame_stats = frame.static_block_entity_stats;
        self.static_entity_frame_stats.static_bytes_uploaded = static_bytes_uploaded;
        self.entity_outline.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            frame.glowing,
            (outline_base, outlined / 4),
            [self.config.width, self.config.height],
        );
        let particle_limit = self.device.limits().max_buffer_size;
        let (opaque, translucent, additive) =
            frame
                .explicit_particles
                .map_or((&[][..], &[][..], &[][..]), |quads| {
                    (
                        quads.opaque.as_slice(),
                        quads.translucent.as_slice(),
                        quads.additive.as_slice(),
                    )
                });
        let opaque_draw = self.explicit_opaque_particles.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            opaque,
            particle_limit,
        );
        let translucent_draw = self.explicit_translucent_particles.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            translucent,
            particle_limit,
        );
        let additive_draw = self.explicit_additive_particles.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            additive,
            particle_limit,
        );
        let max_quads =
            (world_quads + additive_quads + hand_quads + see_through_quads + shadow_only_quads)
                .max(cloud_quads)
                .max(pack_cloud_quads.unwrap_or(0))
                .max(if frame.lightning.bolts.is_empty() {
                    0
                } else {
                    lightning::QUADS
                })
                .max(weather_quads)
                .max(frame.glowing.len() as u32 / 4)
                .max(outlined / 4)
                .max(opaque_draw)
                .max(translucent_draw)
                .max(additive_draw);
        let needed_index_quads = max_quads.min(entity_quad_capacity.max_index_quads());
        if needed_index_quads > self.index_quads {
            self.index_quads = entity_quad_capacity.index_buffer_quads(needed_index_quads);
            self.indices = Self::quad_indices(&self.device, self.index_quads);
        }
        if let Some(b) = &frame.border {
            if b.rebuilt {
                self.uploads.buffer(
                    &mut encoder,
                    &self.border_vertices,
                    0,
                    bytemuck::cast_slice(&b.vertices),
                );
            }
            let t = &b.transforms;
            let uniform = BorderUniform {
                color: t.color,
                offset: [
                    t.model_offset[0],
                    t.model_offset[1],
                    t.model_offset[2],
                    t.texture_offset,
                ],
            };
            self.uploads.buffer(
                &mut encoder,
                &self.border_uniform,
                0,
                bytemuck::bytes_of(&uniform),
            );
        }
        laps.lap("r_materials");
        // Every upload before the frame's first pass: a pass ends wgpu-core's
        // command buffer, so copies between passes would need one more.
        if !frame.screen.is_empty() {
            let vertices = &mut self.screen_vertices_scratch;
            vertices.clear();
            vertices.extend(frame.screen.iter().flat_map(|q| q.vertices));
            if vertices.len() as u64 > self.screen_capacity {
                self.screen_capacity = (vertices.len() as u64).next_power_of_two();
                self.screen_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Screen effect vertices"),
                    size: self.screen_capacity
                        * std::mem::size_of::<crate::screen_effects::ScreenVertex>() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            self.uploads.buffer(
                &mut encoder,
                &self.screen_buffer,
                0,
                bytemuck::cast_slice(vertices),
            );
        }
        self.hud.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            frame.hud,
            [self.config.width, self.config.height],
        );
        if !self.hook_geometry.text_patches.is_empty() {
            self.hud.upload_glyph_patches(
                &mut self.uploads,
                &mut encoder,
                &self.hook_geometry.text_patches,
            );
            self.hook_geometry.text_patches.clear();
        }
        // The translucent terrain pass reads its draws from phase 2's buffer, where the tests of
        // the sections left to phase 2 give them their instance. Where it draws from
        // `self.indirect` (flat draws, improved transparency, a pack's material), or no test
        // runs (no solid draws), every translucent draw stays.
        let flat_frame_possible = self.flat_on
            && self
                .flat
                .as_ref()
                .is_some_and(|f| listed >= f.min_sections || pack_terrain || shadow_view.is_some());
        let translucent_culled = culling
            && solid_draws > 0
            && !flat_frame_possible
            && !oit_on
            && self.native.is_none()
            && !self.translucent_marks.is_empty();
        let phase2_end = if translucent_culled {
            let (skip, tail) = (solid_draws - phase1_draws, solid_draws + translucent_draws);
            self.phase2_upload.clear();
            self.phase2_upload
                .extend_from_slice(&self.draws[phase1_draws as usize..tail as usize]);
            if let Some(cull) = &mut self.cull {
                for &(k, test) in &self.translucent_marks {
                    self.phase2_upload[(skip + k) as usize][1] = 0;
                    cull.place_translucent(test, skip + k);
                }
            }
            tail
        } else {
            solid_draws
        };
        if culling && let Some(cull) = &mut self.cull {
            cull.upload(
                &self.device,
                &mut self.uploads,
                &mut encoder,
                if translucent_culled {
                    &self.phase2_upload
                } else {
                    &self.draws[phase1_draws as usize..phase2_end as usize]
                },
                &self.instances,
                &self.globals,
            );
        }
        let flat_frame = match &mut self.flat {
            Some(flat)
                if self.flat_on
                    // They copy the CPU's lists alone (GPU culling is never offered
                    // where vanilla frames draw flat).
                    && !gpu_frame
                    && !self.draws.is_empty()
                    && (listed >= flat.min_sections
                        || pack_terrain
                        || shadow_view.is_some()) =>
            {
                // The visibility buffer's phase 2 IDs need each call's page (`visbuffer`).
                flat.page_instances = self.visbuf.is_some()
                    && !pack_terrain
                    && shadow_view.is_none()
                    && self.native.as_ref().is_none_or(|n| n.material.is_none());
                let sections = &self.sections;
                let lods = &self.lods;
                let (sorted, lists) = (&self.sorted_indices, &self.world_views.sections);
                let phase2 = self.cull.as_ref().filter(|_| culling).map(|c| &c.phase2);
                crate::spans::time("flatten", || {
                    flat.prepare(
                        &self.device,
                        &mut encoder,
                        &mut self.uploads,
                        &self.draws,
                        &self.draw_pages,
                        phase1_draws as usize,
                        solid_draws as usize,
                        lists.marks([shadow_first, shadow_first + shadow_solid]),
                        [opaque_end, cutout_start, phase2_cutout as usize],
                        phase2,
                        self.translucent_arena.is_some(),
                        visible
                            .iter()
                            .chain(&shadow_visible)
                            .chain(lists.sections())
                            .filter_map(|v| {
                                let section = sections.get(v.slot);
                                let code = fade_code(section, &v.key);
                                Some((
                                    section.flat_id?,
                                    section_instance(v.origin, code, visibility(v.key)),
                                ))
                            })
                            .chain(visible_lods.iter().filter_map(|v| {
                                let id = lods[&v.key].drawn().flat_id?;
                                Some((id, lod::instance(v.origin, v.fade)))
                            })),
                        |page| sorted.buffer(page).clone(),
                        self.own_timing.as_mut(),
                    )
                })
            }
            _ => false,
        };
        // Each page's group binds the page for the solid draws, flat or not.
        if let Some(flat) = &mut self.flat {
            let arena = &self.arena;
            flat.bind_pages(&self.device, |page| {
                arena.has_page(page).then(|| arena.buffer(page))
            });
        }
        if flat_frame {
            section_draw_calls = self.flat.as_ref().map_or(0, |f| f.shadow_calls[0]);
        } else if self.multi_draw && !self.draws.is_empty() {
            if self.draws.len() as u64 > self.indirect_capacity {
                self.indirect_capacity = (self.draws.len() as u64).next_power_of_two();
                self.indirect = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Section draws"),
                    size: self.indirect_capacity * 20,
                    usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            self.uploads.buffer(
                &mut encoder,
                &self.indirect,
                0,
                bytemuck::cast_slice(&self.draws),
            );
        }
        if !separate_sky && let Some(timer) = &mut self.gpu_timer {
            timer.begin(&mut encoder);
        }
        // The frame's statistics (`show_stats`): a CPU frame's at once; a GPU frame's when the
        // pass's counts come back, with the CPU's part of it: the sections the walk listed but
        // those whose solid quads the pass drew (it counts them), its draws and every call. A
        // frame the sourced-geometry pass draws in counts the far tiles with the near part out:
        // they go with its own counts, its kept records and their solid draws (the tiles whose
        // water the CPU still draws are its kept records too, once).
        let cpu = [
            (if sourced_frame { visible.len() } else { listed }) - gpu_solid,
            section_draws,
            section_draw_calls,
        ];
        let sourced_cpu = [visible_lods.len() - sourced_gpu_solid, 0, 0];
        if gpu_frame && let Some(gpu) = &mut self.gpu_culling {
            gpu.dispatch(&mut encoder, self.own_timing.as_mut());
        } else {
            self.show_stats(serial, cpu);
        }
        if sourced_frame {
            self.sourced
                .dispatch(&mut encoder, self.own_timing.as_mut());
        }
        if let Some(timer) = Self::pass_timer(&mut self.pack_post, &mut self.native)
            && timer.is_some() != self.pass_timing
        {
            *timer = self
                .pass_timing
                .then(|| pack_timing::PassTiming::new(&self.device, &self.queue))
                .flatten();
        }
        if let Some(Some(t)) = Self::pass_timer(&mut self.pack_post, &mut self.native) {
            t.gap = self.pass_timing_gap;
            t.keep_frames(self.keep_timed_frames);
        }
        laps.lap("r_uploads");
        // Phase 2 of occlusion culling draws over the depth, and a shader pack or TAA reads it.
        // Post effects (a resource pack's chains, `use_depth_buffer`) read the world's depth
        // after the world pass, as `GameRenderer.render` hands them the main target's.
        let post_reads_depth = !frame.title && !frame.post_effects.effects.is_empty();
        let keep_depth = pack_reads_world
            || post_reads_depth
            || (taa && self.world_post.taa)
            || oit_on
            || native_render::needs().world_depth
            || fg;
        // The pack's program reads the vertices this frame's draws bind.
        let pack_draws_terrain = pack_terrain && flat_frame == self.pack_terrain_flat;
        // The shadow camera's lists, drawn by the pack's shadow programs.
        let pack_draws_shadow = shadow_view.is_some() && flat_frame == self.pack_terrain_flat;
        let pack_draws_water = pack_draws_terrain
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_water());
        let pack_draws_clouds = pack_post
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_clouds());
        let pack_draws_outline = pack_post
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_lines());
        let pack_draws_hand = pack_post
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_hand());
        let pack_draws_weather = pack_post
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_weather());
        let pack_colortex0_blendable = pack_world_runs
            && self
                .pack_post
                .as_ref()
                .and_then(|(_, post)| post.as_ref())
                .is_some_and(|post| post.colortex0_blendable(&self.device));
        let after_pack_custom_geometry =
            live_hooks && world_primitives && pack_draws_terrain && pack_colortex0_blendable;
        if oit_on {
            let size = self
                .world_post
                .size()
                .filter(|_| scaled.is_some())
                .unwrap_or([self.config.width, self.config.height]);
            let (device, layout, sprites) =
                (&self.device, &self.terrain_layout, self.terrain_sprites);
            let clouds = [0, 1].map(|i| self.cloud_pipeline.get_bind_group_layout(i));
            let format = self.output.frame.format;
            let flat = self
                .flat
                .as_ref()
                .map(|f| f.translucent.get_bind_group_layout(1));
            let features = [
                self.entity_pipeline.get_bind_group_layout(0),
                self.weather.pipeline().get_bind_group_layout(1),
                self.border_pipeline.get_bind_group_layout(0),
            ];
            let oit = match &mut self.oit {
                Some(oit) => oit,
                None => {
                    let oit = oit::Oit::new(
                        device,
                        layout,
                        flat.as_ref(),
                        [&features[0], &features[1], &features[2]],
                        [&clouds[0], &clouds[1]],
                        sprites,
                        format,
                    );
                    self.world_primitives
                        .as_mut()
                        .expect("world primitives")
                        .enable_oit(device, oit.stage_layouts());
                    self.oit.insert(oit)
                }
            };
            oit.prepare(
                &self.device,
                &self.queue,
                size,
                &views.primary().projection,
                [
                    water_mask_quads > 0,
                    cloud_quads > 0 || pack_cloud_quads.is_some_and(|q| q > 0),
                ],
            );
            // A pack's clouds sample the targets just made.
            if let (Some(core), Some(samples)) = (
                self.core_clouds
                    .as_mut()
                    .filter(|_| pack_cloud_quads.is_some()),
                self.oit.as_ref().and_then(|o| o.cloud_samples()),
            ) {
                core.bind_oit(&self.device, &samples);
            }
        }
        laps.lap("r_world_pass");
        self.draw_shadow_passes(
            &mut encoder,
            frame,
            shadow_view.as_ref(),
            flat_frame,
            shadow_first,
            shadow_solid,
            shadow_translucent,
            entity_quads,
            shadow_only_quads,
            outline_base,
        );
        laps.lap("r_pack_inputs");
        let reflection = self.draw_water_mirror(
            &mut encoder,
            frame,
            globals,
            view_proj,
            scaled.as_ref(),
            mirror_plane,
            clear,
            sky_visible,
            flat_frame,
            solid_draws,
        );
        let (world_view, world_depth) = match &scaled {
            Some((color, depth)) => (color, depth),
            None => (&view_target, &self.depth),
        };
        if culling && let Some(cull) = &mut self.cull {
            cull.bind_depth(&self.device, world_depth);
        }
        let oit = self.oit.as_ref().filter(|_| oit_on);
        let cull = self.cull.as_ref().filter(|_| culling && solid_draws > 0);
        // The hand draws at the end of the world pass, after a depth reset, when nothing runs
        // between them (a scaled world's post pass, HDR's world copy, a native mod's
        // `world_drawn`): one render pass fewer, ~10 µs of wgpu's encoding a frame.
        let hand_in_world = (frame.hand.len() / 4) > 0
            && !pack_draws_hand
            && scaled.is_none()
            && self.hdr.is_none()
            // Frame generation draws the hand over the real and the made frame after the world.
            && fg_frames.is_none()
            && !live_hooks
            && !native_render::world_drawn_hooked();
        if native_sky_pass && !separate_sky {
            Self::draw_timed_native_sky(
                &mut self.native,
                &self.device,
                &self.group,
                &mut encoder,
                (world_view, world_depth, self.output.frame.format),
                clear,
            );
        }
        if let Some(native) = &mut self.native {
            let reflection =
                reflection.map(|view| native.frame.linear_mirror(&self.device, &mut encoder, view));
            native
                .frame
                .set_reflection(&self.device, reflection.as_ref());
        }
        // The material stage's copies of the world before translucent terrain, the size of
        // this frame's targets.
        if let Some(native) = &mut self.native
            && native.material.is_some()
            && !frame.title
        {
            let color = scaled.as_ref().map(|(c, _)| c.texture());
            native
                .frame
                .fit_opaque(&self.device, world_depth.texture(), color);
        }
        if let Some(native) = self.native.as_mut().filter(|_| !frame.title) {
            let writes = native.material.as_ref().is_some_and(|m| m.writes_surface);
            let depth = world_depth.texture();
            let size = if writes {
                [depth.width(), depth.height()]
            } else {
                [1, 1]
            };
            native.frame.fit_surface(&self.device, size);
        }
        laps.lap("r_mirror");
        // One of Sunrise's own packs with a material stage: terrain draws with its pipelines,
        // which read the pack's frame at group 3.
        let material = self
            .native
            .as_ref()
            .filter(|_| !frame.title)
            .and_then(|n| Some((n.material.as_ref()?, &n.frame.group)));
        // Without HDR output, the solid shaders without its lighting (`SolidPipeline`).
        let hdr = self.hdr.is_some();
        let solid_pipeline = material.map_or(self.solid.get(hdr), |(m, _)| &m.solid);
        // Two solid passes only where a discard costs the GPU its hidden surface
        // removal: Apple's tiled GPUs, behind Metal. Elsewhere the second pass
        // would double the solid draw calls and vertex work for nothing.
        let split_solid = self.info.backend == wgpu::Backend::Metal;
        let translucent_pipeline = material.map_or(&self.translucent, |(m, _)| &m.translucent);
        let pack_flat = material.and_then(|(m, _)| m.flat.as_ref());
        // A material stage's translucent pass reads a copy of the depth the solid terrain
        // left: it is stored across the pass that ends before the copy.
        let keep_depth = keep_depth || material.is_some();
        // GPU culling's occlusion test splits the world pass as cull.rs's does.
        let gpu_occluding = gpu_frame && self.gpu_culling.as_ref().is_some_and(|g| g.occluding());
        let pack_frame = |pass: &mut wgpu::RenderPass<'_>| {
            if let Some((_, group)) = material {
                pass.set_bind_group(sunrise_shader_api::stages::GROUP, group, &[]);
            }
        };
        // A pack's solid terrain that calls `set_surface` also writes the surface target, in
        // passes of its own between the sky and everything else.
        let surface = self
            .native
            .as_ref()
            .filter(|n| {
                !frame.title
                    && !pack_draws_terrain
                    && n.material.as_ref().is_some_and(|m| m.writes_surface)
            })
            .map(|n| n.frame.surface_view.clone());
        // Solid terrain through the visibility buffer (`visbuffer`): flat frames without a
        // pack drawing terrain, a material stage, a surface target or a shadow pass.
        let vis_frame = flat_frame
            && !frame.title
            && !pack_terrain
            && !pack_draws_terrain
            && material.is_none()
            && surface.is_none()
            && shadow_view.is_none()
            && self
                .arena
                .page_buffers()
                .all(|(page, _)| page < visbuffer::MAX_PAGES)
            && self.visbuf.is_some();
        if vis_frame && let (Some(vis), Some(flat)) = (&mut self.visbuf, &self.flat) {
            let depth = world_depth.texture();
            vis.fit(&self.device, [depth.width(), depth.height()], flat);
        }
        {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: world_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: if native_sky_pass || separate_sky {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(clear)
                    },
                    store: wgpu::StoreOp::Store,
                },
            })];
            let world_pixels = pixel_count(world_depth);
            let first_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Sunrise world"),
                timestamp_writes: self.own_timing.as_mut().and_then(|t| t.writes("World")),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: world_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        // The visibility buffer's passes, and those after an occlusion test,
                        // draw on after it.
                        store: if cull.is_some() || gpu_occluding || keep_depth || vis_frame {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            let mut pass = pack_timing::counted(
                self.own_timing.as_mut(),
                first_pass,
                world_pixels,
                "before translucent",
            );
            pass.set_bind_group(0, &self.group, &[]);
            pack_frame(&mut pass);
            if !separate_sky {
                if frame.title || (sky_visible && (frame.skybox || frame.end_sky)) {
                    pass.stage(self.own_timing.as_mut(), "sky");
                }
                self.draw_sky_pass_content(
                    &mut pass,
                    frame,
                    sky_visible,
                    pack_draws_sky,
                    native_sky_pass,
                    celestial_vertices,
                    pack_draws_celestial,
                    sky_switches,
                    hdr_gain,
                );
                // The inline sky boundary does not lend the world pass to mods.
                if sky_stage == Some(SkyStageOwner::Inline)
                    && let Some(hooks) = hooks.as_deref_mut()
                {
                    hooks.on_after_sky(&self.visible_renderable_sections, hook_oit);
                }
            }
            if sky_stage == Some(SkyStageOwner::Inline)
                && frame_graph.as_ref().is_some_and(|fg| {
                    fg.has_slot(mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterSky,
                    ))
                })
            {
                drop(pass);
                post::execute_pack_fg_slot(
                    &mut frame_graph,
                    mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterSky,
                    ),
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    self.world_primitives.as_mut(),
                    &self.mod_gpu.render_targets,
                    &self.transient_targets,
                );
                let mut resumed =
                    after_culling_pass(&mut encoder, world_view, world_depth, true, None)
                        .forget_lifetime();
                resumed.set_bind_group(0, &self.group, &[]);
                pack_frame(&mut resumed);
                pass = pack_timing::CountedPass::plain(resumed);
            }
            // Keep the cursor for engine-supplied test batches. Mod stage callbacks draw only
            // explicitly selected prepared ranges at their real boundaries.
            let mut primitive_stage = 0;
            let terrain_pass =
                |encoder: &mut wgpu::CommandEncoder, surface: &wgpu::TextureView, clear: bool| {
                    // Its lifetime forgotten: a closure cannot tie it to `encoder`'s.
                    let mut pass = surface_pass(encoder, world_view, surface, world_depth, clear)
                        .forget_lifetime();
                    pass.set_bind_group(0, &self.group, &[]);
                    pack_frame(&mut pass);
                    pack_timing::CountedPass::plain(pass)
                };
            // The occlusion test's pipeline draws into the world's colour alone.
            let plain_pass = |encoder: &mut wgpu::CommandEncoder| {
                let mut pass = after_culling_pass(encoder, world_view, world_depth, true, None)
                    .forget_lifetime();
                pass.set_bind_group(0, &self.group, &[]);
                pack_frame(&mut pass);
                pack_timing::CountedPass::plain(pass)
            };
            if let Some(surface) = &surface {
                drop(pass);
                pass = terrain_pass(&mut encoder, surface, true);
            }
            match self
                .flat
                .as_ref()
                .filter(|_| flat_frame && !pack_draws_terrain)
            {
                _ if pack_draws_terrain => {
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
                // The visibility buffer: the sky stays in the world pass's colour; the solid
                // calls draw their IDs and depth (phase 1, the occlusion test, phase 2), then
                // the resolve shades them, and the world pass goes on over both.
                Some(flat) if vis_frame => {
                    let vis = self.visbuf.as_ref().expect("a visibility buffer frame");
                    let vertices = flat_vertices(&self.arena, self.translucent_arena.as_ref());
                    drop(pass);
                    let mut ids =
                        vis.id_pass(&mut encoder, world_depth, true, self.own_timing.as_mut());
                    ids.set_bind_group(0, &self.group, &[]);
                    vis.draw_ids(&mut ids, flat, vertices);
                    drop(ids);
                    if let Some(cull) = cull {
                        // The test's pipeline draws into the world's colour.
                        let mut test = plain_pass(&mut encoder);
                        cull.test(&mut test);
                        drop(test);
                        cull.test_pass(&mut encoder, self.own_timing.as_mut());
                        flat.flatten_phase2(&mut encoder, self.own_timing.as_mut());
                        let mut ids =
                            vis.id_pass(&mut encoder, world_depth, false, self.own_timing.as_mut());
                        ids.set_bind_group(0, &self.group, &[]);
                        vis.draw_ids_phase2(&mut ids, flat, vertices);
                    }
                    vis.resolve(
                        &self.device,
                        &mut encoder,
                        world_view,
                        &self.group,
                        flat,
                        &self.arena,
                        self.own_timing.as_mut(),
                    );
                    pass = world_pass_after_culling(
                        &mut encoder,
                        world_view,
                        world_depth,
                        keep_depth,
                        self.own_timing.as_mut(),
                        "World after visibility buffer",
                        world_pixels,
                    );
                    pass.set_bind_group(0, &self.group, &[]);
                    pack_frame(&mut pass);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
                Some(flat) => {
                    let vertices = flat_vertices(&self.arena, self.translucent_arena.as_ref());
                    // Without a pack, on Metal, the opaque pass, then the cutout one
                    // (terrain.wgsl's `opaque_quad`), each over phase 1's and phase 2's
                    // calls with quads for it (phase 2's counted from its first).
                    let [opaque, cutout, phase2_split] = flat.pass_calls;
                    let phase2_split = phase2_split - flat.phase1_calls;
                    let phase2_calls = flat.solid_calls - flat.phase1_calls;
                    type Calls = std::ops::Range<usize>;
                    let passes: &[(&wgpu::RenderPipeline, Calls, Calls)] = match pack_flat {
                        Some(p) => &[(&p.0, 0..flat.phase1_calls, 0..phase2_calls)],
                        None if split_solid => &[
                            (flat.solid_opaque.get(hdr), 0..opaque, 0..phase2_split),
                            (
                                flat.solid_cutout.get(hdr),
                                cutout..flat.phase1_calls,
                                phase2_split..phase2_calls,
                            ),
                        ],
                        None => &[(flat.solid.get(hdr), 0..flat.phase1_calls, 0..phase2_calls)],
                    };
                    let draw = |pass: &mut wgpu::RenderPass<'_>| {
                        for (pipeline, calls, _) in passes {
                            pass.set_pipeline(pipeline);
                            flat.draw(pass, &flat.calls[calls.clone()], vertices);
                        }
                    };
                    pass.stage(self.own_timing.as_mut(), "solid terrain");
                    match cull {
                        // As below: phase 1, the test, then phase 2's draws
                        // that the test saw, copied after it.
                        Some(cull) => {
                            draw(&mut pass);
                            if surface.is_some() {
                                drop(pass);
                                pass = plain_pass(&mut encoder);
                            }
                            pass.split(self.own_timing.as_mut(), "occlusion boxes");
                            pass.stage(self.own_timing.as_mut(), "occlusion boxes");
                            cull.test(&mut pass);
                            drop(pass);
                            cull.test_pass(&mut encoder, self.own_timing.as_mut());
                            flat.flatten_phase2(&mut encoder, self.own_timing.as_mut());
                            pass = match &surface {
                                Some(surface) => terrain_pass(&mut encoder, surface, false),
                                None => world_pass_after_culling(
                                    &mut encoder,
                                    world_view,
                                    world_depth,
                                    keep_depth,
                                    self.own_timing.as_mut(),
                                    "World after culling",
                                    world_pixels,
                                ),
                            };
                            pass.set_bind_group(0, &self.group, &[]);
                            pack_frame(&mut pass);
                            pass.stage(self.own_timing.as_mut(), "solid terrain phase 2");
                            for (pipeline, _, calls) in passes {
                                pass.set_pipeline(pipeline);
                                flat.draw_phase2(&mut pass, calls.clone(), vertices);
                            }
                        }
                        // Without culling, phase 1 is all the solid calls.
                        None => draw(&mut pass),
                    }
                    pass.set_bind_group(0, &self.group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
                None => {
                    // Without a native material, on Metal, the opaque pass, then the
                    // cutout one (terrain.wgsl's `opaque_quad`), each over phase 1's
                    // and phase 2's lists with quads for it.
                    let (opaque, cutout) = (opaque_end as u32, cutout_start as u32);
                    type Draws = std::ops::Range<u32>;
                    let passes: &[(&wgpu::RenderPipeline, Draws, Draws)] = match material {
                        None if split_solid => &[
                            (
                                self.solid_opaque.get(hdr),
                                0..opaque,
                                phase1_draws..phase2_cutout,
                            ),
                            (
                                self.solid_cutout.get(hdr),
                                cutout..phase1_draws,
                                phase2_cutout..solid_draws,
                            ),
                        ],
                        _ => &[(solid_pipeline, 0..phase1_draws, phase1_draws..solid_draws)],
                    };
                    // The solid pipelines, a native material's too, read the page themselves.
                    let pulled = self.flat.is_some();
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.set_vertex_buffer(1, self.instances.slice(..));
                    pass.stage(self.own_timing.as_mut(), "solid terrain");
                    match cull {
                        Some(cull) => {
                            // Phase 1 and the boxes against its depth (in
                            // its pass, or one of their own on Metal), then
                            // what they found in sight, in a pass that loads
                            // both targets (it reads the test's results).
                            for (pipeline, draws, _) in passes {
                                pass.set_pipeline(pipeline);
                                self.draw_sections_from(
                                    &mut pass,
                                    &self.indirect,
                                    0,
                                    draws.start,
                                    draws.len() as u32,
                                    pulled,
                                );
                            }
                            if surface.is_some() {
                                drop(pass);
                                pass = plain_pass(&mut encoder);
                            }
                            pass.split(self.own_timing.as_mut(), "occlusion boxes");
                            pass.stage(self.own_timing.as_mut(), "occlusion boxes");
                            cull.test(&mut pass);
                            drop(pass);
                            cull.test_pass(&mut encoder, self.own_timing.as_mut());
                            pass = match &surface {
                                Some(surface) => terrain_pass(&mut encoder, surface, false),
                                None => world_pass_after_culling(
                                    &mut encoder,
                                    world_view,
                                    world_depth,
                                    keep_depth,
                                    self.own_timing.as_mut(),
                                    "World after culling",
                                    world_pixels,
                                ),
                            };
                            pass.set_bind_group(0, &self.group, &[]);
                            pack_frame(&mut pass);
                            pass.set_index_buffer(
                                self.indices.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.set_vertex_buffer(1, self.instances.slice(..));
                            pass.stage(self.own_timing.as_mut(), "solid terrain phase 2");
                            for (pipeline, _, draws) in passes {
                                pass.set_pipeline(pipeline);
                                self.draw_sections_from(
                                    &mut pass,
                                    &cull.phase2,
                                    phase1_draws,
                                    draws.start,
                                    draws.len() as u32,
                                    pulled,
                                );
                            }
                        }
                        // Without culling, phase 1 is all the solid draws; GPU culling's
                        // own test (`GpuCulling::occluding`) leaves the CPU's to phase 2.
                        None => {
                            let gpu = self.gpu_culling.as_ref().filter(|_| gpu_frame);
                            let occluding = gpu.is_some_and(|g| g.occluding());
                            // The sourced-geometry pass's draws after every near one and the
                            // CPU's: it lies past the render distance (`sourced_geometry::gpu`).
                            let sourced = sourced_frame.then_some(&self.sourced);
                            let page_group = |page: u32| {
                                pulled
                                    .then(|| self.flat.as_ref()?.page_group(page))
                                    .flatten()
                            };
                            for (pipeline, draws, _) in passes {
                                pass.set_pipeline(pipeline);
                                // GPU culling's draws first, all within the render distance.
                                if let Some(gpu) = gpu {
                                    gpu.draw(&mut pass, &self.arena, page_group, 0);
                                    pass.set_vertex_buffer(1, self.instances.slice(..));
                                }
                                if !occluding {
                                    self.draw_sections_from(
                                        &mut pass,
                                        &self.indirect,
                                        0,
                                        draws.start,
                                        draws.len() as u32,
                                        pulled,
                                    );
                                    if let Some(sourced) = sourced {
                                        sourced.draw(&mut pass, &self.arena, page_group);
                                        pass.set_vertex_buffer(1, self.instances.slice(..));
                                    }
                                }
                            }
                            // As cull.rs's arm: the boxes against phase 1's depth, then
                            // phase 2 and the CPU's draws in a pass that loads both targets.
                            if let Some(gpu) = gpu.filter(|_| occluding) {
                                if surface.is_some() {
                                    drop(pass);
                                    pass = plain_pass(&mut encoder);
                                }
                                pass.split(self.own_timing.as_mut(), "occlusion boxes");
                                pass.stage(self.own_timing.as_mut(), "occlusion boxes");
                                gpu.test(&mut pass);
                                drop(pass);
                                gpu.compact(&mut encoder, self.own_timing.as_mut());
                                pass = match &surface {
                                    Some(surface) => terrain_pass(&mut encoder, surface, false),
                                    None => world_pass_after_culling(
                                        &mut encoder,
                                        world_view,
                                        world_depth,
                                        keep_depth,
                                        self.own_timing.as_mut(),
                                        "World after culling",
                                        world_pixels,
                                    ),
                                };
                                pass.set_bind_group(0, &self.group, &[]);
                                pack_frame(&mut pass);
                                pass.set_index_buffer(
                                    self.indices.slice(..),
                                    wgpu::IndexFormat::Uint32,
                                );
                                pass.stage(self.own_timing.as_mut(), "solid terrain phase 2");
                                for (pipeline, draws, _) in passes {
                                    pass.set_pipeline(pipeline);
                                    gpu.draw(&mut pass, &self.arena, page_group, 1);
                                    pass.set_vertex_buffer(1, self.instances.slice(..));
                                    self.draw_sections_from(
                                        &mut pass,
                                        &self.indirect,
                                        0,
                                        draws.start,
                                        draws.len() as u32,
                                        pulled,
                                    );
                                    if let Some(sourced) = sourced {
                                        sourced.draw(&mut pass, &self.arena, page_group);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // `[displacement]`'s grid over the flat quads it took over, writing its depth.
            if let Some(n) = self
                .native
                .as_ref()
                .filter(|_| !frame.title && !pack_draws_terrain)
                && let Some(d) = &n.displacement
                && let Some(pipeline) = n.material.as_ref().and_then(|m| m.displaced.as_ref())
            {
                pass.stage(self.own_timing.as_mut(), "displacement");
                pass.set_bind_group(0, &self.group, &[]);
                pack_frame(&mut pass);
                d.draw(&mut pass, &d.view, pipeline);
            }
            if surface.is_some() {
                drop(pass);
                pass = plain_pass(&mut encoder);
            }
            // RenderLevelStageEvent.AfterOpaqueBlocks: after solid and cutout terrain, before
            // entities.
            if world_primitives && !pack_draws_terrain {
                let drew = if let Some(hooks) = hooks.as_deref_mut() {
                    let data = hooks.on_render_stage(
                        mod_api::RenderLevelStage::AfterOpaqueBlocks,
                        &self.visible_renderable_sections,
                        oit_on,
                    );
                    self.world_primitives
                        .as_ref()
                        .expect("world primitives")
                        .draw_prepared(&mut pass, None, data)
                } else {
                    self.world_primitives
                        .as_ref()
                        .expect("world primitives")
                        .draw_through(
                            &mut pass,
                            &mut primitive_stage,
                            world_primitives::STAGE_AFTER_OPAQUE_BLOCKS,
                        );
                    true
                };
                if drew {
                    pass.set_bind_group(0, &self.group, &[]);
                    pack_frame(&mut pass);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
            }
            if !pack_draws_terrain
                && frame_graph.as_ref().is_some_and(|fg| {
                    fg.has_slot(mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterOpaqueBlocks,
                    ))
                })
            {
                drop(pass);
                post::execute_pack_fg_slot(
                    &mut frame_graph,
                    mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterOpaqueBlocks,
                    ),
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    self.world_primitives.as_mut(),
                    &self.mod_gpu.render_targets,
                    &self.transient_targets,
                );
                pass = plain_pass(&mut encoder);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            if static_entity_quads > 0 {
                pass.stage(self.own_timing.as_mut(), "static block entities");
                pass.set_pipeline(&self.static_entity_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_bind_group(1, &self.static_entity_group, &[]);
                pass.set_index_buffer(
                    self.static_entity_indices.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                for section in frame.visible_static_block_entity_sections {
                    let Some(gpu) = self.static_entity_sections.get(section) else {
                        continue;
                    };
                    let quads = gpu.vertices / 4;
                    if quads == 0 {
                        continue;
                    }
                    pass.set_vertex_buffer(0, gpu.buffer.slice(..));
                    pass.set_vertex_buffer(1, gpu.origin_buffer.slice(..));
                    pass.draw_indexed(0..quads * 6, 0, 0..1);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            if entity_quads > 0 || !self.prepared_draws.is_empty() {
                pass.stage(self.own_timing.as_mut(), "entities");
                pass.set_pipeline(&self.entity_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                if self.prepared_draws.is_empty() {
                    pass.draw_indexed(0..entity_quads * 6, 0, 0..1);
                    if light_marks(self.output.frame.format, self.hdr.is_some()) {
                        pass.set_pipeline(&self.hdr_entity_mark_pipeline);
                        pass.draw_indexed(0..entity_quads * 6, 0, 0..1);
                    }
                } else {
                    prepared_model::draw_ordered(
                        &mut pass,
                        &prepared_model::EntityRangePass {
                            pipeline: &self.entity_pipeline,
                            hdr_pipeline: &self.hdr_entity_mark_pipeline,
                            vertices: &self.entity_buffer,
                            indices: &self.indices,
                            hdr: light_marks(self.output.frame.format, self.hdr.is_some()),
                        },
                        self.prepared_models.as_ref(),
                        &self.prepared_instance_buffer,
                        &self.prepared_draws,
                        &self.prepared_ranges,
                    );
                }
                pass.set_bind_group(0, &self.group, &[]);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            if banner_pattern_quads > 0 {
                pass.stage(self.own_timing.as_mut(), "banner patterns");
                pass.set_pipeline(&self.banner_pattern_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..banner_pattern_quads * 6,
                    ((world_quads - text_solid_quads - glint_only_quads - banner_pattern_quads) * 4)
                        as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            if glint_only_quads > 0 {
                pass.stage(self.own_timing.as_mut(), "glint overlay");
                pass.set_pipeline(&self.glint_only_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..glint_only_quads * 6,
                    ((world_quads - text_solid_quads - glint_only_quads) * 4) as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            if text_solid_quads > 0 {
                pass.stage(self.own_timing.as_mut(), "entity text solid");
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                // Draw glowing outline copies with NORMAL depth before coplanar
                // POLYGON_OFFSET sign glyph faces and inline sprites.
                let marked = light_marks(self.output.frame.format, self.hdr.is_some());
                for (mode, pipeline, mark_pipeline, count, first) in [
                    (
                        core_text::WorldTextMode::Normal,
                        &self.world_text_pipeline,
                        &self.text_mark_pipelines[0],
                        text_normal_solid_quads,
                        world_quads - text_normal_solid_quads,
                    ),
                    (
                        core_text::WorldTextMode::PolygonOffset,
                        &self.text_polygon_offset_pipeline,
                        &self.text_mark_pipelines[1],
                        text_offset_solid_quads,
                        world_quads - text_solid_quads,
                    ),
                ] {
                    if count == 0 {
                        continue;
                    }
                    pass.set_pipeline(pipeline);
                    let pack_drawn = self.bind_world_text(&mut pass, mode);
                    pass.draw_indexed(0..count * 6, (first * 4) as i32, 0..1);
                    if marked {
                        if pack_drawn {
                            if self.bind_world_text_hdr_mark(&mut pass, mode) {
                                pass.draw_indexed(0..count * 6, (first * 4) as i32, 0..1);
                            }
                        } else {
                            pass.set_pipeline(mark_pipeline);
                            pass.set_bind_group(0, &self.entity_group, &[]);
                            pass.draw_indexed(0..count * 6, (first * 4) as i32, 0..1);
                        }
                    }
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            if self.explicit_opaque_particles.any() {
                pass.stage(self.own_timing.as_mut(), "opaque provider particles");
                pass.set_pipeline(&self.particle_pipelines.opaque);
                pass.set_bind_group(0, &self.entity_group, &[]);
                self.explicit_opaque_particles.draw(&mut pass);
                if light_marks(self.output.frame.format, self.hdr.is_some()) {
                    pass.set_pipeline(&self.particle_pipelines.mark);
                    self.explicit_opaque_particles.draw(&mut pass);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            // Native terrain models use the block sprite arrays with their own
            // alpha/mipmap policies and ordinary opaque depth. Storage follows
            // the existing materials without moving their indices or frame tails.
            let mut first = world_quads
                - text_solid_quads
                - glint_only_quads
                - banner_pattern_quads
                - dynamic_quads
                - native_solid_quads
                - native_cutout_quads
                - native_cutout_mipped_quads
                - native_translucent_quads;
            for (i, count) in [
                native_solid_quads,
                native_cutout_quads,
                native_cutout_mipped_quads,
            ]
            .into_iter()
            .enumerate()
            {
                if count > 0 {
                    pass.stage(self.own_timing.as_mut(), "terrain models");
                    pass.set_pipeline(&self.terrain_model_pipelines[i]);
                    pass.set_bind_group(0, &self.terrain_model_group, &[]);
                    pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                    pass.draw_indexed(0..count * 6, (first * 4) as i32, 0..1);
                }
                first += count;
            }
            pass.set_bind_group(0, &self.group, &[]);
            // `RenderTypes.entitySolid` and `entityCutout` of a mod's dynamic texture
            // (`BlockEntityDrawContext::textured_quad`): the entity pipeline, one bind group a run.
            for (overlay, count) in [
                (DYNAMIC_TEXTURE_SOLID_OVERLAY, dynamic_solid_quads),
                (DYNAMIC_TEXTURE_CUTOUT_OVERLAY, dynamic_cutout_quads),
            ] {
                if count > 0 {
                    pass.stage(self.own_timing.as_mut(), "dynamic texture quads");
                    pass.set_pipeline(&self.entity_pipeline);
                    pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                    self.dynamic_entity.draw(&mut pass, overlay);
                    pass.set_bind_group(0, &self.group, &[]);
                }
            }
            // RenderLevelStageEvent.AfterOpaqueFeatures: after the last opaque feature, including
            // the terrain-material model group. This pass owns those draws even when a pack
            // owns the blocks.
            if world_primitives && !after_pack_features {
                let drew = if let Some(hooks) = hooks.as_deref_mut() {
                    let data = hooks.on_render_stage(
                        mod_api::RenderLevelStage::AfterOpaqueFeatures,
                        &self.visible_renderable_sections,
                        oit_on,
                    );
                    self.world_primitives
                        .as_ref()
                        .expect("world primitives")
                        .draw_prepared(&mut pass, None, data)
                } else {
                    self.world_primitives
                        .as_ref()
                        .expect("world primitives")
                        .draw_through(
                            &mut pass,
                            &mut primitive_stage,
                            world_primitives::STAGE_AFTER_OPAQUE_FEATURES,
                        );
                    true
                };
                if drew {
                    pass.set_bind_group(0, &self.group, &[]);
                    pack_frame(&mut pass);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
            }
            if !after_pack_features
                && frame_graph.as_ref().is_some_and(|fg| {
                    fg.has_slot(mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterOpaqueFeatures,
                    ))
                })
            {
                drop(pass);
                post::execute_pack_fg_slot(
                    &mut frame_graph,
                    mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterOpaqueFeatures,
                    ),
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    self.world_primitives.as_mut(),
                    &self.mod_gpu.render_targets,
                    &self.transient_targets,
                );
                pass = plain_pass(&mut encoder);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            // FeatureRenderDispatcher.PreparedFrame.executeTranslucent: the shadows
            // come before the translucent items, which then cover them (#7026).
            if shadow_quads > 0 && !oit_on {
                pass.stage(self.own_timing.as_mut(), "shadows");
                pass.set_pipeline(&self.shadow_blob_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..shadow_quads * 6,
                    ((base_quads
                        - translucent_item_quads
                        - translucent_particle_quads
                        - shadow_quads)
                        * 4) as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            if text_quads > 0 && !oit_on {
                pass.stage(self.own_timing.as_mut(), "entity text");
                pass.set_pipeline(&self.text_polygon_offset_pipeline);
                self.bind_world_text(&mut pass, core_text::WorldTextMode::PolygonOffset);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(0..text_quads * 6, (entity_quads * 4) as i32, 0..1);
                pass.set_bind_group(0, &self.group, &[]);
            }
            if translucent_item_quads > 0 && !oit_on {
                pass.stage(self.own_timing.as_mut(), "translucent items");
                pass.set_pipeline(&self.translucent_item_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..translucent_item_quads * 6,
                    ((base_quads - translucent_item_quads) * 4) as i32,
                    0..1,
                );
                if light_marks(self.output.frame.format, self.hdr.is_some()) {
                    // A glowing item in the world (Item sheet quads), as entities are.
                    pass.set_pipeline(&self.hdr_mark_pipeline);
                    pass.draw_indexed(
                        0..translucent_item_quads * 6,
                        ((base_quads - translucent_item_quads) * 4) as i32,
                        0..1,
                    );
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            // Translucent moving blocks (`RenderTypes.translucentMovingBlock`): alpha blended,
            // depth written, quads far to near, before terrain transparency like fluids.
            if native_translucent_quads > 0 {
                pass.stage(self.own_timing.as_mut(), "translucent terrain models");
                pass.set_pipeline(&self.terrain_model_pipelines[3]);
                pass.set_bind_group(0, &self.terrain_model_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..native_translucent_quads * 6,
                    ((world_quads
                        - text_solid_quads
                        - glint_only_quads
                        - banner_pattern_quads
                        - dynamic_quads
                        - native_translucent_quads)
                        * 4) as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            // `RenderTypes.entityTranslucent` of a dynamic texture: blended and depth writing, as
            // the translucent items are, quads far to near.
            if dynamic_translucent_quads > 0 {
                pass.stage(
                    self.own_timing.as_mut(),
                    "translucent dynamic texture quads",
                );
                pass.set_pipeline(&self.translucent_item_pipeline);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                self.dynamic_entity
                    .draw(&mut pass, DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY);
                pass.set_bind_group(0, &self.group, &[]);
            }
            // Still-fluid block entities draw as their own ordinary sorted
            // translucent material, before terrain transparency. This material
            // always writes depth and never joins the improved-transparency queues.
            if fluid_quads > 0 {
                pass.stage(self.own_timing.as_mut(), "fluid block entities");
                pass.set_pipeline(&self.fluid_pipeline);
                pass.set_bind_group(0, &self.fluid_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..fluid_quads * 6,
                    ((base_quads + oit_entity_quads) * 4) as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            // Cracks multiply the existing light; scalar OIT coverage cannot carry that factor.
            if crumbling_quads > 0 && (!oit_on || hdr_gain) {
                pass.stage(self.own_timing.as_mut(), "block breaking");
                pass.set_pipeline(&self.crumbling_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..crumbling_quads * 6,
                    ((entity_quads + text_quads) * 4) as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            // BoatRenderer's water patch (RenderTypes.waterMask): depth only,
            // before translucent terrain, so no water shows inside the boat.
            if water_mask_quads > 0 && !oit_on {
                pass.stage(self.own_timing.as_mut(), "boat water mask");
                pass.set_pipeline(&self.water_mask_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..water_mask_quads * 6,
                    ((entity_quads + text_quads + crumbling_quads + translucent_crumbling_quads)
                        * 4) as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            if !frame.lightning.bolts.is_empty() && !oit_on {
                pass.stage(self.own_timing.as_mut(), "lightning");
                pass.set_pipeline(&self.lightning_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                self.lightning.draw(&mut pass, &frame.lightning);
                if light_marks(self.output.frame.format, self.hdr.is_some()) {
                    pass.set_pipeline(&self.hdr_mark_lightning_pipeline);
                    self.lightning.draw(&mut pass, &frame.lightning);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            if swirl_quads > 0 && !oit_on {
                pass.stage(self.own_timing.as_mut(), "energy swirl");
                pass.set_pipeline(&self.energy_swirl_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..swirl_quads * 6,
                    ((entity_quads
                        + text_quads
                        + crumbling_quads
                        + translucent_crumbling_quads
                        + water_mask_quads)
                        * 4) as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            if additive_quads > 0 && !oit_on {
                pass.stage(self.own_timing.as_mut(), "additive entities");
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                for (rays, indices) in classic_additive_runs(frame.additive, additive_quads) {
                    pass.set_pipeline(if rays {
                        &self.dragon_rays_pipeline
                    } else {
                        &self.additive_pipeline
                    });
                    pass.draw_indexed(indices, (world_quads * 4) as i32, 0..1);
                }
                if light_marks(self.output.frame.format, self.hdr.is_some()) {
                    // The dragon's eyes and death rays, marked as entities are.
                    pass.set_pipeline(&self.hdr_mark_pipeline);
                    pass.draw_indexed(0..additive_quads * 6, (world_quads * 4) as i32, 0..1);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            if outline_edges > 0 && !frame.outline_after_terrain && !pack_draws_outline {
                pass.stage(self.own_timing.as_mut(), "block outline");
                self.draw_outline(&mut pass, outline_edges as u32, frame.outline_high_contrast);
            }
            // DrawableGizmoPrimitives' opaque group writes depth, the translucent one does not.
            if debug_opaque > 0 {
                pass.stage(self.own_timing.as_mut(), "debug lines");
                pass.set_pipeline(&self.debug_lines_opaque);
                pass.set_vertex_buffer(0, self.debug_line_buffer.slice(..));
                pass.draw(0..4, 0..debug_opaque);
            }
            if debug_depth > debug_opaque {
                pass.stage(self.own_timing.as_mut(), "debug lines");
                pass.set_pipeline(&self.debug_lines);
                pass.set_vertex_buffer(0, self.debug_line_buffer.slice(..));
                pass.draw(0..4, debug_opaque..debug_depth);
            }
            if debug_top > 0 {
                pass.stage(self.own_timing.as_mut(), "debug lines");
                pass.set_pipeline(&self.debug_lines_on_top);
                pass.set_vertex_buffer(0, self.debug_line_buffer.slice(..));
                pass.draw(0..4, debug_depth..debug_depth + debug_top);
            }
            // One of Sunrise's own packs reads the world before translucent terrain
            // (`opaque_depth`, `opaque_color`): the pass ends, the targets are copied, and a
            // pass that loads both carries on.
            if material.is_some()
                && let Some(native) = self.native.as_ref().filter(|n| n.frame.reads_opaque)
            {
                drop(pass);
                let depth = world_depth.texture();
                encoder.copy_texture_to_texture(
                    depth.as_image_copy(),
                    native.frame.opaque_depth.as_image_copy(),
                    depth.size(),
                );
                if let Some((color, _)) = &scaled {
                    native.frame.copy_opaque(&self.device, &mut encoder, color);
                }
                pass = world_pass_after_culling(
                    &mut encoder,
                    world_view,
                    world_depth,
                    keep_depth,
                    self.own_timing.as_mut(),
                    "World after copy",
                    world_pixels,
                );
                pass.set_bind_group(0, &self.group, &[]);
                pack_frame(&mut pass);
            }
            // RenderLevelStageEvent.AfterTranslucentFeatures: after translucent features, right
            // before translucent terrain (classic path).
            if world_primitives && !oit_on && !after_pack_features {
                let drew = if let Some(hooks) = hooks.as_deref_mut() {
                    let data = hooks.on_render_stage(
                        mod_api::RenderLevelStage::AfterTranslucentFeatures,
                        &self.visible_renderable_sections,
                        oit_on,
                    );
                    self.world_primitives
                        .as_ref()
                        .expect("world primitives")
                        .draw_prepared(&mut pass, None, data)
                } else {
                    self.world_primitives
                        .as_ref()
                        .expect("world primitives")
                        .draw_through(
                            &mut pass,
                            &mut primitive_stage,
                            world_primitives::STAGE_AFTER_TRANSLUCENT_FEATURES,
                        );
                    true
                };
                if drew {
                    pass.set_bind_group(0, &self.group, &[]);
                    pack_frame(&mut pass);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
            }
            if !oit_on
                && !after_pack_features
                && frame_graph.as_ref().is_some_and(|fg| {
                    fg.has_slot(mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterTranslucentFeatures,
                    ))
                })
            {
                drop(pass);
                post::execute_pack_fg_slot(
                    &mut frame_graph,
                    mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterTranslucentFeatures,
                    ),
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    self.world_primitives.as_mut(),
                    &self.mod_gpu.render_targets,
                    &self.transient_targets,
                );
                pass = plain_pass(&mut encoder);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            match self.flat.as_ref().filter(|_| flat_frame) {
                // The pack draws translucent terrain after its deferred programs.
                _ if pack_draws_water => {}
                // LevelRenderer.executeOit in place of the classic blend; what
                // follows draws over its composite in a pass of its own.
                flat if let Some(oit) = oit => {
                    drop(pass);
                    // LevelRenderer.executeOitWaterMask and CloudRenderer.renderOit.
                    let mask = |pass: &mut wgpu::RenderPass<'_>| {
                        pass.set_pipeline(&self.water_mask_pipeline);
                        pass.set_bind_group(0, &self.entity_group, &[]);
                        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                        pass.draw_indexed(
                            0..water_mask_quads * 6,
                            ((entity_quads
                                + text_quads
                                + crumbling_quads
                                + translucent_crumbling_quads)
                                * 4) as i32,
                            0..1,
                        );
                    };
                    let clouds = |pass: &mut wgpu::RenderPass<'_>, stage: oit::Stage| {
                        pass.set_pipeline(oit.clouds(stage));
                        pass.set_bind_group(0, &self.entity_group, &[]);
                        pass.set_bind_group(1, &self.cloud_group, &[]);
                        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.set_vertex_buffer(0, self.cloud_buffer.slice(..));
                        pass.draw_indexed(0..cloud_quads * 6, 0, 0..1);
                    };
                    // "Features, World Border, Weather": what follows the opaque
                    // world in the classic pass, one stage at a time.
                    let mut wp = self.world_primitives.take().expect("world primitives");
                    wp.prepare_oit_mod_pipelines(&self.device, oit.cloud_samples());
                    let mut features = |pass: &mut wgpu::RenderPass<'_>, stage: oit::Stage| {
                        let f = oit.features();
                        let s = stage as usize;
                        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.set_bind_group(0, &self.entity_group, &[]);
                        pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                        // Quads of a material, `first` quads into the buffer.
                        let quads = |pass: &mut wgpu::RenderPass<'_>,
                                     pipeline: &wgpu::RenderPipeline,
                                     count: u32,
                                     first: u32| {
                            if count > 0 {
                                pass.set_pipeline(pipeline);
                                pass.draw_indexed(0..count * 6, (first * 4) as i32, 0..1);
                            }
                        };
                        quads(pass, &f.text[s], text_quads, entity_quads);
                        if !hdr_gain {
                            quads(
                                pass,
                                &f.crumbling[s],
                                crumbling_quads,
                                entity_quads + text_quads,
                            );
                            quads(
                                pass,
                                &f.crumbling[s],
                                translucent_crumbling_quads,
                                entity_quads + text_quads + crumbling_quads,
                            );
                        }
                        quads(
                            pass,
                            &f.swirl[s],
                            swirl_quads,
                            entity_quads
                                + text_quads
                                + crumbling_quads
                                + translucent_crumbling_quads
                                + water_mask_quads,
                        );
                        quads(
                            pass,
                            &f.shadow[s],
                            shadow_quads,
                            base_quads
                                - translucent_item_quads
                                - translucent_particle_quads
                                - shadow_quads,
                        );
                        quads(
                            pass,
                            &f.entity[s],
                            translucent_particle_quads,
                            base_quads - translucent_item_quads - translucent_particle_quads,
                        );
                        if self.explicit_translucent_particles.any() {
                            pass.set_pipeline(&f.entity[s]);
                            self.explicit_translucent_particles.draw(pass);
                            pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                        }
                        quads(
                            pass,
                            &f.entity[s],
                            translucent_item_quads,
                            base_quads - translucent_item_quads,
                        );
                        // Translucent, emissive, eyes and breeze wind models:
                        // the same fragment, their light and shading are in
                        // the vertices.
                        quads(pass, &f.entity[s], oit_entity_quads, base_quads);
                        if !frame.lightning.bolts.is_empty() {
                            pass.set_pipeline(&f.lightning[s]);
                            self.lightning.draw(pass, &frame.lightning);
                            pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                        }
                        quads(pass, &f.additive[s], additive_quads, world_quads);
                        self.weather
                            .draw_with(pass, &f.weather[s], &self.entity_group);
                        if let Some(b) = frame.border.as_ref().filter(|_| !frame.title) {
                            pass.set_pipeline(&f.border[s]);
                            pass.set_bind_group(0, &self.border_group, &[]);
                            pass.set_vertex_buffer(0, self.border_vertices.slice(..));
                            for &side in &b.sides {
                                let first = side as u32 * 6;
                                pass.draw_indexed(first..first + 6, 0, 0..1);
                            }
                        }
                        if world_primitives {
                            if let Some(hooks) = hooks.as_deref_mut() {
                                let api_stage = match stage {
                                    oit::Stage::DepthBounds => mod_api::OitStage::DepthBounds,
                                    oit::Stage::Transmittance => mod_api::OitStage::Transmittance,
                                    oit::Stage::Accumulate => mod_api::OitStage::Accumulate,
                                };
                                let data = hooks.on_render_stage(
                                    mod_api::RenderLevelStage::OitTranslucent(api_stage),
                                    &self.visible_renderable_sections,
                                    oit_on,
                                );
                                wp.draw_prepared(pass, Some(stage), data);
                            } else {
                                wp.draw_oit(pass, stage);
                            }
                        }
                    };
                    // A pack's `OIT_CLOUDS` in their place.
                    let pack_clouds = (self.core_clouds.as_ref())
                        .zip(frame.clouds.as_ref())
                        .filter(|_| pack_cloud_quads.is_some_and(|q| q > 0));
                    let indices = &self.indices;
                    let pack_clouds = pack_clouds.map(|(core, input)| {
                        move |pass: &mut wgpu::RenderPass<'_>, stage: oit::Stage| {
                            core.draw_oit(pass, stage, !input.fast, indices);
                        }
                    });
                    let mut extras = oit::Extras {
                        water_mask: (water_mask_quads > 0).then_some(&mask as _),
                        clouds: core_clouds::oit_clouds::<
                            dyn Fn(&mut wgpu::RenderPass<'_>, oit::Stage),
                        >(
                            pack_clouds.as_ref().map(|p| p as _), &clouds, cloud_quads
                        ),
                        features: Some(&mut features),
                    };
                    // The draw closure below borrows all of `self`, so the timer is taken out.
                    let mut own_timing = self.own_timing.take();
                    oit.run(
                        &mut encoder,
                        world_view,
                        world_depth,
                        &|pass, stage| {
                            pass.set_bind_group(0, &self.group, &[]);
                            // Flat section draws take the flat pipelines.
                            if let Some((flat, pipeline)) = flat.zip(oit.flat(stage)) {
                                pass.set_pipeline(pipeline);
                                flat.draw(
                                    pass,
                                    flat.translucent_calls(),
                                    flat_vertices(&self.arena, self.translucent_arena.as_ref()),
                                );
                                return;
                            }
                            pass.set_pipeline(oit.terrain(stage));
                            pass.set_index_buffer(
                                self.indices.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.set_vertex_buffer(1, self.instances.slice(..));
                            self.draw_sections(
                                pass,
                                &self.indirect,
                                0,
                                solid_draws,
                                translucent_draws,
                            );
                        },
                        &mut extras,
                        own_timing.as_mut(),
                    );
                    drop(features);
                    self.world_primitives = Some(wp);
                    self.own_timing = own_timing;
                    for oit_stage in [
                        mod_api::OitStage::DepthBounds,
                        mod_api::OitStage::Transmittance,
                        mod_api::OitStage::Accumulate,
                    ] {
                        post::execute_pack_fg_slot(
                            &mut frame_graph,
                            mod_frame_graph::FrameGraphSlot::AfterStage(
                                mod_api::RenderLevelStage::OitTranslucent(oit_stage),
                            ),
                            &self.device,
                            &self.queue,
                            &mut encoder,
                            self.world_primitives.as_mut(),
                            &self.mod_gpu.render_targets,
                            &self.transient_targets,
                        );
                    }
                    pass = world_pass_after_culling(
                        &mut encoder,
                        world_view,
                        world_depth,
                        true,
                        self.own_timing.as_mut(),
                        "World after OIT",
                        world_pixels,
                    );
                    pass.set_bind_group(0, &self.group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.stage(self.own_timing.as_mut(), "OIT marks");
                    if cloud_quads > 0 && !pack_draws_clouds {
                        self.mark_clouds(&mut pass, cloud_quads);
                    }
                    // HDR output's mark on what shines in the stages, as the classic
                    // pass marks it after drawing it: eyes, the dragon's eyes and rays,
                    // firework sparks and lightning.
                    if light_marks(self.output.frame.format, self.hdr.is_some()) {
                        pass.set_pipeline(&self.hdr_mark_pipeline);
                        pass.set_bind_group(0, &self.entity_group, &[]);
                        pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                        for (quads, first) in [
                            (oit_entity_quads, base_quads),
                            (additive_quads, world_quads),
                            (translucent_item_quads, base_quads - translucent_item_quads),
                            (
                                translucent_particle_quads,
                                base_quads - translucent_item_quads - translucent_particle_quads,
                            ),
                        ] {
                            if quads > 0 {
                                pass.draw_indexed(0..quads * 6, (first * 4) as i32, 0..1);
                            }
                        }
                        if self.explicit_translucent_particles.any() {
                            pass.set_pipeline(&self.particle_pipelines.mark);
                            self.explicit_translucent_particles.draw(&mut pass);
                        }
                        if !frame.lightning.bolts.is_empty() {
                            pass.set_pipeline(&self.hdr_mark_lightning_pipeline);
                            self.lightning.draw(&mut pass, &frame.lightning);
                        }
                        pass.set_bind_group(0, &self.group, &[]);
                    }
                    self.mark_translucent_terrain(
                        &mut pass,
                        flat,
                        (&self.indirect, 0),
                        solid_draws,
                        translucent_draws,
                    );
                }
                Some(flat) => {
                    pass.split(self.own_timing.as_mut(), "translucent terrain");
                    pass.stage(self.own_timing.as_mut(), "translucent terrain");
                    pass.set_pipeline(pack_flat.map_or(&flat.translucent, |p| &p.1));
                    flat.draw(
                        &mut pass,
                        flat.translucent_calls(),
                        flat_vertices(&self.arena, self.translucent_arena.as_ref()),
                    );
                    pass.set_bind_group(0, &self.group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    self.mark_translucent_terrain(
                        &mut pass,
                        Some(flat),
                        (&self.indirect, 0),
                        solid_draws,
                        translucent_draws,
                    );
                    pass.split(self.own_timing.as_mut(), "after translucent");
                }
                None => {
                    pass.split(self.own_timing.as_mut(), "translucent terrain");
                    pass.stage(self.own_timing.as_mut(), "translucent terrain");
                    pass.set_pipeline(translucent_pipeline);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.set_vertex_buffer(1, self.instances.slice(..));
                    let from = match &self.cull {
                        Some(cull) if translucent_culled => (&cull.phase2, phase1_draws),
                        _ => (&self.indirect, 0),
                    };
                    self.draw_sections(&mut pass, from.0, from.1, solid_draws, translucent_draws);
                    self.mark_translucent_terrain(
                        &mut pass,
                        None,
                        from,
                        solid_draws,
                        translucent_draws,
                    );
                    pass.split(self.own_timing.as_mut(), "after translucent");
                }
            }
            // RenderLevelStageEvent.AfterTranslucentBlocks: after translucent terrain (classic
            // path).
            if world_primitives && !pack_draws_water && !pack_draws_terrain && !oit_on {
                let drew = if let Some(hooks) = hooks.as_deref_mut() {
                    let data = hooks.on_render_stage(
                        mod_api::RenderLevelStage::AfterTranslucentBlocks,
                        &self.visible_renderable_sections,
                        oit_on,
                    );
                    self.world_primitives
                        .as_ref()
                        .expect("world primitives")
                        .draw_prepared(&mut pass, None, data)
                } else {
                    self.world_primitives
                        .as_ref()
                        .expect("world primitives")
                        .draw_through(
                            &mut pass,
                            &mut primitive_stage,
                            world_primitives::STAGE_AFTER_TRANSLUCENT_BLOCKS,
                        );
                    true
                };
                if drew {
                    pass.set_bind_group(0, &self.group, &[]);
                    pack_frame(&mut pass);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
            }
            if !pack_draws_water
                && !pack_draws_terrain
                && !oit_on
                && frame_graph.as_ref().is_some_and(|fg| {
                    fg.has_slot(mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterTranslucentBlocks,
                    ))
                })
            {
                drop(pass);
                post::execute_pack_fg_slot(
                    &mut frame_graph,
                    mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterTranslucentBlocks,
                    ),
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    self.world_primitives.as_mut(),
                    &self.mod_gpu.render_targets,
                    &self.transient_targets,
                );
                pass = plain_pass(&mut encoder);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            // LevelRenderer.executeTranslucentAfterTerrain: vanilla's
            // translucent particle layer draws after translucent terrain —
            // the quads write depth, so before it they would hide the glass
            // or water behind them.
            if translucent_particle_quads > 0 && !oit_on {
                pass.stage(self.own_timing.as_mut(), "translucent particles");
                pass.set_pipeline(&self.entity_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..translucent_particle_quads * 6,
                    ((base_quads - translucent_item_quads - translucent_particle_quads) * 4) as i32,
                    0..1,
                );
                if light_marks(self.output.frame.format, self.hdr.is_some()) {
                    // Fireworks: their sparks are translucent-layer particles.
                    pass.set_pipeline(&self.hdr_entity_mark_pipeline);
                    pass.draw_indexed(
                        0..translucent_particle_quads * 6,
                        ((base_quads - translucent_item_quads - translucent_particle_quads) * 4)
                            as i32,
                        0..1,
                    );
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            if self.explicit_translucent_particles.any() && !oit_on {
                pass.stage(self.own_timing.as_mut(), "translucent provider particles");
                pass.set_pipeline(&self.particle_pipelines.translucent);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                self.explicit_translucent_particles.draw(&mut pass);
                if light_marks(self.output.frame.format, self.hdr.is_some()) {
                    pass.set_pipeline(&self.particle_pipelines.mark);
                    self.explicit_translucent_particles.draw(&mut pass);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            if self.explicit_additive_particles.any() {
                pass.stage(self.own_timing.as_mut(), "additive provider particles");
                pass.set_pipeline(&self.particle_pipelines.additive);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                self.explicit_additive_particles.draw(&mut pass);
                if light_marks(self.output.frame.format, self.hdr.is_some()) {
                    pass.set_pipeline(&self.particle_pipelines.mark);
                    self.explicit_additive_particles.draw(&mut pass);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            // RenderLevelStageEvent.AfterTranslucentParticles: after translucent particles
            // (classic path).
            if world_primitives && !oit_on {
                let drew = if let Some(hooks) = hooks.as_deref_mut() {
                    let data = hooks.on_render_stage(
                        mod_api::RenderLevelStage::AfterTranslucentParticles,
                        &self.visible_renderable_sections,
                        oit_on,
                    );
                    self.world_primitives
                        .as_ref()
                        .expect("world primitives")
                        .draw_prepared(&mut pass, None, data)
                } else {
                    self.world_primitives
                        .as_ref()
                        .expect("world primitives")
                        .draw_through(
                            &mut pass,
                            &mut primitive_stage,
                            world_primitives::STAGE_AFTER_TRANSLUCENT_PARTICLES,
                        );
                    true
                };
                if drew {
                    pass.set_bind_group(0, &self.group, &[]);
                    pack_frame(&mut pass);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
            }
            if !oit_on
                && frame_graph.as_ref().is_some_and(|fg| {
                    fg.has_slot(mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterTranslucentParticles,
                    ))
                })
            {
                drop(pass);
                post::execute_pack_fg_slot(
                    &mut frame_graph,
                    mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterTranslucentParticles,
                    ),
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    self.world_primitives.as_mut(),
                    &self.mod_gpu.render_targets,
                    &self.transient_targets,
                );
                pass = plain_pass(&mut encoder);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            // Native world text is an ordinary depth-tested overlay, after
            // particles (also after the OIT composite) and before the hand.
            // Its own geometry is independent of block-entity lifetime.
            if world_text_quads + world_text_offset_quads > 0 {
                pass.stage(self.own_timing.as_mut(), "world text");
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                let first = base_quads + oit_entity_quads + fluid_quads;
                for (pipeline, count, offset) in [
                    (&self.world_text_pipeline, world_text_quads, first),
                    (
                        &self.text_polygon_offset_pipeline,
                        world_text_offset_quads,
                        first + world_text_quads,
                    ),
                ] {
                    if count > 0 {
                        pass.set_pipeline(pipeline);
                        pass.draw_indexed(0..count * 6, (offset * 4) as i32, 0..1);
                    }
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            // LevelRenderer.executeClassicTransparency: translucent targets'
            // outlines follow terrain, so its blending cannot wash them out.
            if outline_edges > 0 && frame.outline_after_terrain && !pack_draws_outline {
                pass.stage(self.own_timing.as_mut(), "block outline");
                self.draw_outline(&mut pass, outline_edges as u32, frame.outline_high_contrast);
            }
            if translucent_crumbling_quads > 0 && (!oit_on || hdr_gain) {
                pass.stage(self.own_timing.as_mut(), "block breaking");
                pass.set_pipeline(&self.crumbling_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..translucent_crumbling_quads * 6,
                    ((entity_quads + text_quads + crumbling_quads) * 4) as i32,
                    0..1,
                );
            }
            // Under Improved Transparency a pack's clouds drew in its stages.
            if let (Some(core), Some(input)) = (&self.core_clouds, frame.clouds.as_ref())
                && pack_cloud_quads.is_some_and(|quads| {
                    core_clouds::classic_pass_draws(quads, pack_draws_clouds, oit_on)
                })
            {
                pass.stage(self.own_timing.as_mut(), "clouds");
                core.draw(&mut pass, !input.fast, &self.indices);
                pass.set_bind_group(0, &self.group, &[]);
            }
            if core_clouds::classic_pass_draws(cloud_quads, pack_draws_clouds, oit_on) {
                pass.stage(self.own_timing.as_mut(), "clouds");
                pass.set_pipeline(&self.cloud_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_bind_group(1, &self.cloud_group, &[]);
                pass.set_vertex_buffer(0, self.cloud_buffer.slice(..));
                pass.draw_indexed(0..cloud_quads * 6, 0, 0..1);
                self.mark_clouds(&mut pass, cloud_quads);
            }
            // LevelRenderer.executeClassicTransparency: weather after translucent
            // terrain and clouds, so the water behind the rain is drawn first.
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            if !pack_draws_weather && !oit_on {
                pass.stage(self.own_timing.as_mut(), "weather");
                let native_weather = self
                    .native
                    .as_ref()
                    .filter(|_| !frame.title)
                    .and_then(|n| Some((n.weather.as_ref()?, &n.frame.group)));
                self.weather
                    .draw(&mut pass, &self.entity_group, native_weather);
                pass.set_bind_group(0, &self.group, &[]);
            }
            // RenderLevelStageEvent.AfterWeather: after the rain and snow this pass drew.
            if world_primitives && !pack_draws_weather && !oit_on {
                let drew = if let Some(hooks) = hooks.as_deref_mut() {
                    let data = hooks.on_render_stage(
                        mod_api::RenderLevelStage::AfterWeather,
                        &self.visible_renderable_sections,
                        oit_on,
                    );
                    self.world_primitives
                        .as_ref()
                        .expect("world primitives")
                        .draw_prepared(&mut pass, None, data)
                } else {
                    self.world_primitives
                        .as_ref()
                        .expect("world primitives")
                        .draw_through(
                            &mut pass,
                            &mut primitive_stage,
                            world_primitives::STAGE_AFTER_WEATHER,
                        );
                    true
                };
                if drew {
                    pass.set_bind_group(0, &self.group, &[]);
                    pack_frame(&mut pass);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
            }
            if !pack_draws_weather
                && !oit_on
                && frame_graph.as_ref().is_some_and(|fg| {
                    fg.has_slot(mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterWeather,
                    ))
                })
            {
                drop(pass);
                post::execute_pack_fg_slot(
                    &mut frame_graph,
                    mod_frame_graph::FrameGraphSlot::AfterStage(
                        mod_api::RenderLevelStage::AfterWeather,
                    ),
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    self.world_primitives.as_mut(),
                    &self.mod_gpu.render_targets,
                    &self.transient_targets,
                );
                pass = plain_pass(&mut encoder);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            // LevelRenderer: the world border after translucent terrain and weather.
            if let Some(b) = frame.border.as_ref().filter(|_| !frame.title && !oit_on) {
                pass.stage(self.own_timing.as_mut(), "world border");
                pass.set_pipeline(&self.border_pipeline);
                pass.set_bind_group(0, &self.border_group, &[]);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, self.border_vertices.slice(..));
                for &side in &b.sides {
                    let first = side as u32 * 6;
                    pass.draw_indexed(first..first + 6, 0, 0..1);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            // LevelRenderer.executeSeeThrough, after the rest of the world.
            if see_through_quads > 0 {
                pass.stage(self.own_timing.as_mut(), "see-through");
                pass.set_pipeline(&self.see_through_pipeline);
                self.bind_world_text(&mut pass, core_text::WorldTextMode::SeeThrough);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..see_through_quads * 6,
                    ((world_quads + additive_quads + hand_quads) * 4) as i32,
                    0..1,
                );
            }
            // Polytone GPU particles with their own shader (wiki "GPU Particles"): after the
            // world, the clouds and the weather, which would otherwise cover the ones that write
            // no depth.
            if self.core_particles_drawing() {
                pass.stage(self.own_timing.as_mut(), "polytone particles");
                self.draw_core_particles(&mut pass);
                pass.set_bind_group(0, &self.group, &[]);
                pack_frame(&mut pass);
            }
            // Native mods' deferred custom geometry (`submit_custom_geometry` and debug renderers):
            // drawn at the end of the world pass before the hand, or inside `PackPost` when an
            // active shader pack owns terrain rendering. Live stage hooks never relocate skipped
            // stages here.
            if world_primitives && !after_pack_custom_geometry {
                if live_hooks {
                    primitive_stage = world_primitives::STAGE_CUSTOM_GEOMETRY;
                }
                pass.stage(self.own_timing.as_mut(), "mod primitives");
                self.world_primitives
                    .as_ref()
                    .expect("world primitives")
                    .draw_through(
                        &mut pass,
                        &mut primitive_stage,
                        world_primitives::STAGE_CUSTOM_GEOMETRY,
                    );
            }
            if hand_in_world {
                pass.stage(self.own_timing.as_mut(), "hand");
                pass.set_pipeline(&self.depth_reset);
                pass.set_bind_group(0, &self.group, &[]);
                pass.draw(0..3, 0..1);
                self.draw_hand(
                    &mut pass,
                    frame.hand,
                    hand_quads,
                    world_quads + additive_quads,
                );
            }
        }
        // RenderLevelStageEvent.AfterLevel: after the world finishes and before the engine
        // hand and overlays. A classic pack finishes its world inside PackPost.
        if !pack_world_runs {
            if live_hooks && let Some(hooks) = hooks.as_deref_mut() {
                hooks.on_after_level(&self.visible_renderable_sections, oit_on);
            }
            post::execute_pack_fg_slot(
                &mut frame_graph,
                mod_frame_graph::FrameGraphSlot::AfterStage(mod_api::RenderLevelStage::AfterLevel),
                &self.device,
                &self.queue,
                &mut encoder,
                self.world_primitives.as_mut(),
                &self.mod_gpu.render_targets,
                &self.transient_targets,
            );
        }
        let lifts_world = output::lifts_world(
            frame.title,
            self.output.frame.format == LINEAR_FRAME,
            self.shader_pack_kind() == Some("iris"),
            self.native.as_ref().map(|n| n.frame.api),
        );
        let (
            encoder,
            PostPassResult {
                readback,
                target_texture,
                fg_made,
            },
        ) = self.run_post_and_overlay(
            encoder,
            PostPassArgs {
                frame,
                scaled,
                view_target,
                target_texture,
                surface_frame: surface_frame.as_ref(),
                fg_frames: fg_frames.as_ref(),
                pack_post,
                native_post,
                after_pack_sky: sky_stage == Some(SkyStageOwner::ClassicPack),
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
                frame_graph,
            },
            if pack_world_runs { hooks } else { None },
        )?;
        laps.lap("r_passes");
        // A loss the passes' waits found (a shader pack's timing, its image dump): no submit.
        if self.device_lost() {
            return Err(super::in_flight::LOST.into());
        }
        self.submit_and_present_frame(
            encoder,
            SubmitFrameArgs {
                frame,
                checks,
                surface_frame,
                reconfigure,
                readback,
                target_texture: &target_texture,
                fg_frames,
                fg_made,
                has_section_copies,
                lifts_world,
                started,
                visible,
                visible_lods,
                shadow_visible,
            },
        )
    }
}

/// Shadow caster walk radius in chunks, with an optional override of the effective eye distance.
pub(super) fn shadow_walk_radius(shadow_distance: Option<f32>, detail_distance: f32) -> i32 {
    (shadow_distance.unwrap_or(detail_distance) / 16.0) as i32
}

/// Collects shadow caster sections using the shadow walk radius (separate from eye distance)
/// and the pack's light-volume, view-caster, and reach tests.
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_shadow_sections<T>(
    sections: &resident::Resident<T>,
    camera: DVec3,
    shadow_distance: Option<f32>,
    detail_distance: f32,
    v: &external::ShadowView,
    cull_proj: Mat4,
    shadow_visible: &mut Vec<resident::Visible>,
    boxed: &mut Vec<resident::Visible>,
    seen: &mut crate::fast_hash::FxHashSet<SectionKey>,
) {
    let radius = shadow_walk_radius(shadow_distance, detail_distance);
    let light = ShadowCull::new(shadow_box(v))
        .with_casters(v.view_casters.then(|| shadow_casters(cull_proj, v.light)))
        .with_reach(v.distance);
    // Within the unculled distance every section, then those in the light's box.
    if let Some(d) = v.unculled {
        let all = ShadowCull::new(Frustum::from_planes([glam::Vec4::W; 6])).with_reach(v.distance);
        let near = radius.min((d / 16.0).ceil() as i32);
        sections.visible(camera, near, &all, |_| true, |_, _| false, shadow_visible);
    }
    boxed.clear();
    sections.visible(camera, radius, &light, |_| true, |_, _| false, boxed);
    seen.clear();
    seen.extend(shadow_visible.iter().map(|s| s.key));
    shadow_visible.extend(boxed.drain(..).filter(|s| !seen.contains(&s.key)));
}

/// Marks `layers` in `active`, a bit per animated sprite layer (`Renderer::animate`).
pub(super) fn mark_layers(active: &mut [u64], layers: &[u16]) {
    for &layer in layers {
        let layer = usize::from(layer);
        active[layer / u64::BITS as usize] |= 1 << (layer % u64::BITS as usize);
    }
}

/// With OIT off, the additive vertices uploaded by `EntityBatch` are exactly this prefix.
pub(super) fn classic_additive_runs(
    additive: &[Vertex],
    admitted_quads: u32,
) -> impl Iterator<Item = (bool, std::ops::Range<u32>)> {
    let rays = |quad: &[Vertex; 4]| quad[0].layer & 0xffff == DRAGON_RAYS_OVERLAY;
    let mut first = 0;
    additive[..admitted_quads as usize * 4]
        .as_chunks::<4>()
        .0
        .chunk_by(move |a, b| rays(a) == rays(b))
        .map(move |run| {
            let end = first + run.len() as u32 * 6;
            let indices = first..end;
            first = end;
            (rays(&run[0]), indices)
        })
}

/// The vertices the entity buffer grows to for a frame of `vertices`: the next power of two,
/// but no more than a buffer of `max_buffer_size` bytes holds unless the frame itself needs it.
pub(super) fn entity_buffer_vertices(vertices: u64, max_buffer_size: u64) -> u64 {
    let most = max_buffer_size / std::mem::size_of::<Vertex>() as u64;
    vertices.next_power_of_two().min(most).max(vertices)
}

/// A frame's re-sort rotation over the near sections' and the far tiles' combined translucent
/// list (`LevelRenderer`: a rotating eighth of it, at least 15, besides the nearby sections). A
/// far tile's place is past the near sections', at its index in the walk's complete set, so the
/// window is the same whichever solid draws the GPU owns.
#[derive(Clone, Copy, Debug)]
pub(super) struct Rotation {
    near: usize,
    count: usize,
    budget: usize,
    cursor: usize,
}

impl Rotation {
    /// The rotation over `near` sections and `far` tiles, from the last frame's `cursor`.
    pub(super) fn new(near: usize, far: usize, cursor: usize) -> Self {
        let count = near + far;
        Self {
            near,
            count,
            budget: (count / 8).max(15).min(count),
            cursor: cursor % count.max(1),
        }
    }

    /// Whether the near section at `index` of the visible list re-sorts this frame.
    pub(super) fn near(&self, index: usize) -> bool {
        self.window(index)
    }

    /// The cursor the next frame starts from.
    pub(super) fn next(&self) -> usize {
        self.cursor + self.budget
    }

    fn window(&self, combined: usize) -> bool {
        (combined + self.count - self.cursor) % self.count < self.budget
    }
}

/// A frame's far water: the walk's complete set (`far`) and what the CPU draws of it (`listed`,
/// which keeps the walk's order and leaves out the tiles whose solid quads the pass drew
/// whole), with the frame's re-sort rotation. The rotation reads a tile's place in `far`, its
/// instance slot and its blend tier read `listed`.
pub(super) struct FarWater<'a> {
    far: &'a [lod::Visible],
    listed: &'a [lod::Visible],
    rotation: Rotation,
}

impl<'a> FarWater<'a> {
    pub(super) fn new(
        far: &'a [lod::Visible],
        listed: &'a [lod::Visible],
        rotation: Rotation,
    ) -> Self {
        Self {
            far,
            listed,
            rotation,
        }
    }

    /// Each listed tile, as its index in `far`, its instance slot past the near sections', and
    /// whether the rotation re-sorts it this frame.
    pub(super) fn entries(&self) -> impl Iterator<Item = (usize, usize, bool)> + '_ {
        let mut slot = 0usize;
        let near = self.rotation.near;
        self.far.iter().enumerate().filter_map(move |(i, v)| {
            if self.listed.get(slot).is_some_and(|l| l.key == v.key) {
                slot += 1;
                Some((i, near + slot - 1, self.rotation.window(near + i)))
            } else {
                None
            }
        })
    }

    /// A translucent draw's place in the one back-to-front order of near and far water: its
    /// blend tier first (the near sections' water 0, the far tiles' 1, a tile standing in for
    /// its replacement 2, over them), then the farthest, then the highest instance.
    pub(super) fn order(
        &self,
        distance: u64,
        instance: usize,
    ) -> (u32, std::cmp::Reverse<u64>, std::cmp::Reverse<usize>) {
        let tier = match instance.checked_sub(self.rotation.near) {
            None => 0,
            Some(slot) if self.listed[slot].old => 2,
            Some(_) => 1,
        };
        (
            tier,
            std::cmp::Reverse(distance),
            std::cmp::Reverse(instance),
        )
    }
}
