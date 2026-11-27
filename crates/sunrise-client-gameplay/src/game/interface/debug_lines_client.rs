use crate::game::ClientGame;
use sunrise_client::{
    debug_entries::{self, ENTRIES},
    debug_overlay as overlay,
    debug_overlay::Displayer,
};
use sunrise_core::java_number;
use sunrise_gameplay_common::game::debug_lines::LOOKING_AT_RANGE;
use sunrise_gameplay_common::game::debug_lines::SERVER_ENTRIES;
use sunrise_world::heightmap::HeightmapType;

#[derive(Clone, Debug, Default)]
pub(in crate::game) struct FormattedServerDebug {
    /// The server chunk's heightmaps at the feet (`keepAfterWorldgen`).
    pub heights: Option<[i32; 4]>,
    pub difficulty: Option<String>,
    pub spawn_counts: Option<String>,
    pub chunks: Option<String>,
}

#[derive(Default)]
pub(in crate::game) struct DebugLinesCache {
    revision: Option<u64>,
    lines: FormattedServerDebug,
}

impl ClientGame<'_> {
    pub(in crate::game::client_side) fn reset_server_debug_lines(&mut self) {
        *self.client.debug_server_lines.get_mut() = Default::default();
    }

    /// `Minecraft.crosshairPickEntity`'s type: the nearest entity under the
    /// crosshair before any block.
    pub(in crate::game::client_side) fn crosshair_entity_type(&self) -> Option<String> {
        let mut best: Option<(f64, String)> = None;
        let mut offer = |hit: Option<(f64, String)>| {
            if let Some((distance, kind)) = hit
                && self.before_block(distance)
                && best.as_ref().is_none_or(|b| distance < b.0)
            {
                best = Some((distance, kind));
            }
        };
        offer(
            self.mob_hit()
                .map(|(i, d, _)| (d, self.common.mobs[i].kind.id.to_string())),
        );
        offer(
            self.boat_hit()
                .map(|(i, d)| (d, self.common.boats[i].entity_type())),
        );
        offer(
            self.cart_hit()
                .map(|(i, d)| (d, self.common.carts[i].kind.id().to_owned())),
        );
        offer(
            self.armor_stand_hit()
                .map(|(_, d, _)| (d, "minecraft:armor_stand".to_owned())),
        );
        offer(
            self.frame_hit()
                .map(|(i, d)| (d, self.common.item_frames[i].kind().to_owned())),
        );
        offer(
            self.painting_hit()
                .map(|(_, d)| (d, "minecraft:painting".to_owned())),
        );
        offer(
            self.dragon_hit()
                .map(|(_, d)| (d, "minecraft:ender_dragon".to_owned())),
        );
        best.map(|b| b.1)
    }
    /// `DebugEntryBiome`: the client level's biome at the feet, inside the
    /// build height.
    pub(in crate::game::client_side) fn debug_biome_line(&self, d: &mut Displayer) {
        let Some([x, y, z]) = self.debug_feet() else {
            return;
        };
        if y < self.common.world.bottom_y() || y >= self.common.world.top_y() {
            return;
        }
        if let Some(chunk) = self.common.world.chunks.get(&(x >> 4, z >> 4))
            && let Some(biome) = self
                .common
                .res
                .biomes()
                .list
                .get(chunk.biome(x, y, z) as usize)
        {
            d.line(format!("Biome: {}", biome.name));
        }
    }
    /// `Entity.pick(20, 0, false)` from the camera: the block outline hit.
    pub(in crate::game::client_side) fn debug_block_hit(&self) -> Option<[i32; 3]> {
        let (origin, dir) = self.aim()?;
        Some(self.clip_ray(origin, dir, LOOKING_AT_RANGE)?.pos)
    }
    /// `LevelExtractor.sectionStatistics`: the sections drawn of the view
    /// area's (smart culling is always on), the render distance, and
    /// `SectionRenderDispatcher.getStats` (the meshes queued; Sunrise has
    /// no buffer pool, so no buffer is ever free).
    pub(in crate::game::client_side) fn debug_chunk_render_line(&self, d: &mut Displayer) {
        let Some(r) = self.client.renderer.0.as_ref() else {
            return;
        };
        let distance = self.client.settings.render_distance;
        let side = (distance * 2 + 1) as usize;
        let total = side * side * (self.common.world.layout().height() / 16) as usize;
        d.line(format!(
            "C: {}/{total} (s) D: {distance}, pC: {:03}, aB: {:02}",
            r.drawn_sections,
            self.client.streaming.pending(),
            0
        ));
    }
    /// `DebugEntryChunkSourceStats`: `ClientLevel.gatherChunkSourceStats`
    /// (the chunk storage's size and its loaded chunks, then the entities,
    /// the sections holding them and the chunks that tick them), and the
    /// server level's when this game runs it.
    pub(in crate::game::client_side) fn debug_chunk_source_lines(
        &self,
        d: &mut Displayer,
        server: Option<&FormattedServerDebug>,
    ) {
        if self.common.player.is_none() {
            return;
        }
        let range = self
            .client
            .remote
            .as_ref()
            .map_or(self.client.settings.render_distance, |r| r.view_distance);
        let side = (2 * (range.max(2) + 3) + 1) as usize;
        let loaded = self.common.world.chunks.len();
        d.line(format!(
            "Chunks[C] W: {}, {loaded} E: {},{},{loaded}",
            side * side,
            self.debug_entity_count(),
            self.debug_entity_sections()
        ));
        if let Some(line) = server.and_then(|s| s.chunks.clone()) {
            d.line(line);
        }
    }
    /// `LevelExtractor.entityStatistics`: the mobs drawn last frame of the
    /// level's entities, and the server's simulation distance.
    pub(in crate::game::client_side) fn debug_entity_render_line(&self, d: &mut Displayer) {
        if self.common.player.is_none() {
            return;
        }
        let simulation = self
            .client
            .remote
            .as_ref()
            .map_or(self.client.settings.simulation_distance, |r| {
                r.simulation_distance
            });
        d.line(format!(
            "E: {}/{}, SD: {simulation}",
            self.client.rendered_mobs.get(),
            self.debug_entity_count()
        ));
    }
    /// Every enabled entry's lines, by identifier.
    pub fn debug_entry_lines(&self, d: &mut Displayer, framebuffer: [i32; 2]) {
        let reduced = self.reduced_debug_info();
        let mut paths: Vec<&str> = ENTRIES
            .iter()
            .map(|e| e.path)
            .filter(|path| {
                self.client.debug_entries.0.enabled(
                    &self.client.registered_debug_entries.0,
                    path,
                    self.client.debug,
                    reduced,
                )
            })
            .collect();
        paths.sort_unstable();
        let server = self.server_debug(paths.iter().any(|p| SERVER_ENTRIES.contains(p)));
        for path in paths {
            match path {
                "biome" => self.debug_biome_line(d),
                "chunk_render_stats" => self.debug_chunk_render_line(d),
                "chunk_source_stats" => self.debug_chunk_source_lines(d, server.as_ref()),
                "day_count" => {
                    if self.common.player.is_some() {
                        d.line(format!("Day #{}", self.world.day_time.div_euclid(24000)));
                    }
                }
                "entity_render_stats" => self.debug_entity_render_line(d),
                "entity_spawn_counts" => {
                    if let Some(line) = server.as_ref().and_then(|s| s.spawn_counts.clone()) {
                        d.line(line);
                    }
                }
                "fps" => self.debug_fps_line(d),
                "game_version" => d.priority(format!(
                    "Minecraft {v} ({v}/sunrise)",
                    v = sunrise_sim::save::VERSION_NAME
                )),
                "gpu_utilization" => d.line(overlay::gpu_line(self.client.gpu_utilization)),
                "heightmap" => self.debug_heightmap_lines(d, server.as_ref()),
                "light_levels" => {
                    if let Some(feet) = self.debug_feet() {
                        let light = self.common.world.light(feet[0], feet[1], feet[2]);
                        d.group("light", [overlay::light_line(light >> 4, light & 15)]);
                    }
                }
                "local_difficulty" => {
                    if let Some(line) = server.as_ref().and_then(|s| s.difficulty.clone()) {
                        d.line(line);
                    }
                }
                "looking_at_block_state" | "looking_at_block_tags" => {
                    let lines = self.debug_looking_at_block(path == "looking_at_block_state");
                    d.group("looking_at_block", lines);
                }
                "looking_at_entity" | "looking_at_entity_tags" => {
                    let lines = self.crosshair_entity_type().map_or_else(Vec::new, |kind| {
                        if path == "looking_at_entity" {
                            vec![format!("{}Targeted Entity", overlay::UNDERLINE), kind]
                        } else {
                            overlay::tag_lines(
                                self.items().entity_type_tags.members().iter(),
                                &kind,
                            )
                        }
                    });
                    d.group("looking_at_entity", lines);
                }
                "looking_at_fluid_state" | "looking_at_fluid_tags" => {
                    let lines = self.debug_looking_at_fluid(path == "looking_at_fluid_state");
                    d.group("looking_at_fluid", lines);
                }
                "memory" => {
                    if let Some(lines) = self.client.memory_meter.borrow_mut().lines() {
                        d.group("memory", lines);
                    }
                }
                "particle_render_stats" => d.line(format!("P: {}", self.client.particles.0.len())),
                "player_position" | "player_section_position" => {
                    if let Some(cam) = self.camera_subject() {
                        let mut lines = overlay::position(
                            cam.camera.position,
                            cam.camera.y_rot,
                            cam.camera.x_rot,
                            self.common.dim.id(),
                            self.client.streaming.forced.len(),
                        );
                        let section = lines.pop().unwrap_or_default();
                        if path == "player_position" {
                            d.group("position", lines);
                        } else {
                            d.group("position", [section]);
                        }
                    }
                }
                "player_speed" => {
                    if self.common.player.is_some() {
                        let speed = self.known_player_movement();
                        d.group(
                            "position",
                            [format!(
                                "Speed: {} blocks/tick",
                                java_number::format_fixed(speed.length(), 3)
                            )],
                        );
                    }
                }
                "post_effects" => {
                    let effects = self.requested_post_effects();
                    if !effects.is_empty() {
                        d.line(format!("Post: {}", effects.join(", ")));
                    }
                }
                "simple_performance_impactors" => self.debug_performance_lines(d),
                "sound_cache" => {
                    let (buffers, bytes) = self.client.audio.0.cache_stats();
                    d.line(overlay::sound_cache_line(buffers, bytes));
                }
                "sound_mood" => {
                    if self.common.player.is_some() {
                        d.line(overlay::sound_mood_line(
                            self.client.audio.0.engine.used_channels(),
                            [
                                sunrise_client::sound_engine::STATIC_CHANNELS,
                                sunrise_client::sound_engine::STREAMING_CHANNELS,
                            ],
                            self.client.ambience.moodiness,
                        ));
                    }
                }
                "system_specs" => {
                    self.debug_system_lines(d, framebuffer);
                    // Sunrise's own: the shader pack's GPU time per pass (an
                    // empty group would move others).
                    let pass_lines = self.client.renderer.0.as_ref().map_or_else(Vec::new, |r| {
                        overlay::shader_pass_lines(&r.shader_pass_times())
                    });
                    if self.client.debug && !pass_lines.is_empty() {
                        d.group("shader pack", pass_lines);
                    }
                    // Sunrise's own passes, in a group of their own.
                    let own_lines = self.client.renderer.0.as_ref().map_or_else(Vec::new, |r| {
                        overlay::labelled_pass_lines("Sunrise GPU", &r.own_pass_times())
                    });
                    if self.client.debug && !own_lines.is_empty() {
                        d.group("Sunrise passes", own_lines);
                    }
                    let detail_lines = self.client.renderer.0.as_ref().map_or_else(Vec::new, |r| {
                        let support = r.pass_detail_support().filter(|_| r.pass_detail());
                        support.map_or_else(Vec::new, |support| {
                            overlay::pass_detail_lines(
                                &r.own_pass_parts(),
                                &r.own_pass_counters(),
                                support,
                            )
                        })
                    });
                    if self.client.debug && !detail_lines.is_empty() {
                        d.group("Sunrise pass detail", detail_lines);
                    }
                }
                "tps" => self.debug_tps_line(d),
                // DebugEntryChunkGeneration, where the world is generated
                // here: the window's server thread publishes its generators.
                "chunk_generation_stats" => {
                    if let Some(feet) = self.debug_feet() {
                        let terrain = self
                            .client
                            .integrated
                            .as_ref()
                            .and_then(|server| server.terrain(self.common.dim.id()));
                        let dim = self.common.dim.id();
                        let mut cache = self.client.debug_density.borrow_mut();
                        let line = cache.line(
                            dim,
                            feet,
                            terrain.map(|t| move || t.density_debug_line(feet)),
                        );
                        if let Some(line) = line {
                            d.group("chunk_generation", [line.to_owned()]);
                        }
                    }
                }
                // detailed_memory: the JVM's memory pools, which Sunrise
                // has none of. The rest are renderers (debug_renderers.rs).
                _ => {}
            }
        }
        // A native mod's entries come after vanilla's, in registration order.
        for (group, lines) in debug_entries::mod_lines(
            &self.client.registered_debug_entries.0,
            self,
            &self.client.debug_entries.0,
            self.client.debug,
            reduced,
        ) {
            d.group(group, lines);
        }
    }
    /// The camera's feet (`Entity.blockPosition`).
    pub(in crate::game::client_side) fn debug_feet(&self) -> Option<[i32; 3]> {
        let pos = self.camera_entity_position()?;
        Some(pos.map(|v| v.floor() as i32))
    }
    /// `Entity.pick(20, 0, true)` (`ClipContext.Fluid.ANY`): the first fluid
    /// along the view, under its surface, before the block outline hit.
    pub(in crate::game::client_side) fn debug_fluid_hit(&self) -> Option<[i32; 3]> {
        let (origin, dir) = self.aim()?;
        let block = self.clip_ray(origin, dir, LOOKING_AT_RANGE);
        let reach = block.map_or(LOOKING_AT_RANGE, |h| {
            (0..3)
                .map(|i| (h.point[i] - origin[i]).powi(2))
                .sum::<f64>()
                .sqrt()
        });
        let mut t = 0.0;
        while t <= reach {
            let p: [f64; 3] = std::array::from_fn(|i| origin[i] + dir[i] * t);
            let b = p.map(|v| v.floor() as i32);
            if let Some((height, _)) = self.common.fluid_height_of(b[0], b[1], b[2], None)
                && p[1] - f64::from(b[1]) <= f64::from(height)
            {
                return Some(b);
            }
            t += 0.05;
        }
        block.map(|h| h.pos)
    }
    /// `DebugEntryHeightmap`: the client chunk's heightmaps at the feet,
    /// and the server chunk's.
    pub(in crate::game::client_side) fn debug_heightmap_lines(
        &self,
        d: &mut Displayer,
        server: Option<&FormattedServerDebug>,
    ) {
        let Some([x, _, z]) = self.debug_feet() else {
            return;
        };
        if !self.common.world.loaded(x, z) {
            return;
        }
        let heights = server.and_then(|s| s.heights);
        let server_height = |kind: HeightmapType| {
            let i = [
                HeightmapType::WorldSurface,
                HeightmapType::OceanFloor,
                HeightmapType::MotionBlocking,
                HeightmapType::MotionBlockingNoLeaves,
            ]
            .iter()
            .position(|k| *k == kind)
            .unwrap_or(0);
            heights.map_or(0, |h| h[i])
        };
        let lines = overlay::heightmap_lines(
            |kind| self.common.column_height(kind, x, z),
            heights
                .is_some()
                .then_some(&server_height as &dyn Fn(_) -> i32),
        );
        d.group("heightmaps", lines);
    }
    /// `DebugEntryLookingAt.BlockStateInfo` or `BlockTagInfo`.
    pub(in crate::game::client_side) fn debug_looking_at_block(&self, state: bool) -> Vec<String> {
        let Some(pos) = self.debug_block_hit() else {
            return Vec::new();
        };
        let id = self.common.world.block(pos[0], pos[1], pos[2]);
        let Some(s) = self.common.pack.assets.registry.state(id) else {
            return Vec::new();
        };
        if state {
            overlay::state_lines(
                "Targeted Block",
                pos,
                &s.name,
                s.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            )
        } else {
            overlay::tag_lines(&self.items().block_tags, &s.name)
        }
    }
    /// `DebugEntryLookingAt.FluidStateInfo` or `FluidTagInfo`.
    pub(in crate::game::client_side) fn debug_looking_at_fluid(&self, state: bool) -> Vec<String> {
        let Some(pos) = self.debug_fluid_hit() else {
            return Vec::new();
        };
        let (name, properties) = self.fluid_state(pos);
        if state {
            overlay::state_lines(
                "Targeted Fluid",
                pos,
                &name,
                properties.iter().map(|(k, v)| (*k, v.as_str())),
            )
        } else {
            overlay::tag_lines(&self.items().fluid_tags, &name)
        }
    }
    /// `FluidState`'s registered name and properties at `pos`: a source is
    /// `water`/`lava` with `falling`, a flowing one `flowing_*` with `level`.
    pub(in crate::game) fn fluid_state(
        &self,
        pos: [i32; 3],
    ) -> (String, Vec<(&'static str, String)>) {
        use sunrise_items::blocks::Fluid;
        let f = self
            .common
            .res
            .facts()
            .get(self.common.world.block(pos[0], pos[1], pos[2]));
        let kind = match f.fluid {
            Fluid::None => return ("minecraft:empty".into(), Vec::new()),
            Fluid::Water => "water",
            Fluid::Lava => "lava",
            Fluid::Modded => {
                let state = self.common.world.block(pos[0], pos[1], pos[2]);
                let name = self.common.res.behaviours().fluid(state).kind().name();
                return (
                    name.to_owned(),
                    vec![("falling", f.fluid_falling.to_string())],
                );
            }
        };
        let falling = ("falling", f.fluid_falling.to_string());
        if f.fluid_level == 8 && !f.fluid_falling {
            (format!("minecraft:{kind}"), vec![falling])
        } else {
            (
                format!("minecraft:flowing_{kind}"),
                vec![falling, ("level", f.fluid_level.to_string())],
            )
        }
    }
    /// The integrated server's lines when an entry wants them; the request
    /// (the dimension and the feet) goes with it, for the next server tick.
    pub(in crate::game::client_side) fn server_debug(
        &self,
        wanted: bool,
    ) -> Option<FormattedServerDebug> {
        let server = self.client.integrated.as_ref()?;
        let at = self
            .debug_feet()
            .filter(|_| wanted)
            .map(|feet| (self.common.dim, feet));
        server.request_debug(at);
        at?;
        let (revision, sample) = server.debug();
        let mut cache = self.client.debug_server_lines.borrow_mut();
        if cache.revision != Some(revision) {
            cache.lines = FormattedServerDebug {
                heights: sample.heights,
                difficulty: sample.difficulty.map(overlay::local_difficulty_line),
                spawn_counts: sample
                    .spawn_counts
                    .map(|(chunks, counts)| overlay::spawn_counts_line(chunks, &counts)),
                chunks: sample.chunks.map(|(loaded, entities, sections)| {
                    format!("Chunks[S] W: {loaded} E: {entities},{entities},{sections},{loaded},{loaded},0,0")
                }),
            };
            cache.revision = Some(revision);
        }
        Some(cache.lines.clone())
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
