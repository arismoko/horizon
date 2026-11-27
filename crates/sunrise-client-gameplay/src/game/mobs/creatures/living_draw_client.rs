use super::*;
use crate::game::{mob_blocks::MobView, stock_layer_mask::StockLayerMask};
use sunrise_client::mobs::render::FrameDraw;
use sunrise_gameplay_common::game::creatures::living_draw::LivingFrame;

impl crate::game::ClientGame<'_> {
    /// Draws `mob` into `out` as the frame's mob pass does, or nothing when it is out of range,
    /// out of view, hidden or culled.
    pub(in crate::game) fn living_geometry<const PRECHECK: bool>(
        &self,
        mob: &Mob,
        frame: &LivingFrame<'_>,
        out: &mut Vec<Vertex>,
    ) {
        let LivingFrame {
            camera,
            alpha,
            scale,
            viewer,
            skinning,
            listened,
            now,
        } = *frame;
        let atlas = &self.client.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        // LevelRenderer: the entity the view looks out of is not drawn
        // in first person.
        if self.hides_as_camera(mob.id) {
            return;
        }
        if !self.entity_section_visible(
            glam::DVec3::new(mob.position.x, mob.position.y, mob.position.z),
            now,
        ) {
            return;
        }
        let is_player = &*mob.kind.id == "minecraft:player";
        let d = glam::DVec3::new(mob.position.x, mob.position.y, mob.position.z) - camera;
        // Entity.shouldRenderAtSqrDistance: 64 times the bounding box's
        // mean edge, scaled by the view scale. RemotePlayer's box counts
        // ten times.
        let (w, h, _) = mob.dims();
        let edge = (w + h + w) / 3.0 * if is_player { 10.0 } else { 1.0 };
        let range = edge * 64.0 * scale;
        if d.length_squared() >= range * range {
            return;
        }
        let beam = sunrise_sim::mobs::guardian::beam(mob, alpha as f32);
        // EntityRenderer.shouldRender: the renderer's interpolated culling box,
        // grown by 0.5 (GuardianRenderer also draws while its beam is out).
        if let Some(view) = &self.client.view_frustum
            && beam.is_none()
        {
            let (lo, hi) = super::super::entity_culling::cull_box(mob, alpha);
            let (lo, hi) = (
                (lo - camera - glam::DVec3::splat(0.5)).as_vec3(),
                (hi - camera + glam::DVec3::splat(0.5)).as_vec3(),
            );
            if !view.intersects(lo, hi) {
                return;
            }
        }
        let outline = self.mob_outline(mob);
        // Entity.DATA_SHARED_FLAGS_ID invisible (the flag the invisibility
        // effect sets): only the name tag draws (and the outline of a
        // glowing mob, LivingEntityRenderer.getRenderType). A player keeps
        // its armour, elytra and held items (`LivingEntityRenderer.submit`
        // skips only the body).
        let invisible = mob.is_invisible();
        // Entity.isInvisibleTo(viewer): a spectator sees it anyway, the
        // body forced translucent (`LivingEntityRenderer.submit`).
        let seen_invisible = invisible
            && !sunrise_game_data::scoreboard::invisible_to(
                self.client_mob_team_of(mob),
                viewer,
                self.own_player_is_spectator(),
                true,
            );
        let layers_only = invisible && outline.is_none() && is_player && !seen_invisible;
        if invisible && outline.is_none() && !layers_only && !seen_invisible {
            return;
        }
        let culling = if PRECHECK
            && outline.is_none()
            && !is_player
            && mob.kind.id.starts_with("minecraft:")
            && (0..6).all(|slot| mob.extra.slot_stack(slot).is_none())
            && beam.is_none()
        {
            let Some(bounds) = super::super::entity_client::mob_render_bounds(
                mob,
                self.client_res().client_mob_models(),
                alpha,
            ) else {
                return;
            };
            self.entity_cull_decision_in_bounds(mob, bounds)
        } else {
            EntityCullDecision::CheckVertices
        };
        if culling.hides_entity() {
            return;
        }
        let entity_start = out.len();
        self.mark_motion(entity_start, (1 << 32) | u64::from(mob.id));
        self.mark_entity(entity_start, &mob.kind.id);
        self.client
            .rendered_mobs
            .set(self.client.rendered_mobs.get() + 1);
        let packed = self.mob_packed_light(mob, alpha);
        // The mods' pre hooks (a player's `render_player_pre`, then `render_living_pre`),
        // before any of the vanilla model's parts (a sulfur cube's swallowed block) are
        // drawn. A cancel skips the rest of the entity's drawing and its post hooks; the
        // frame's later name tag and leash passes leave it out too, its flames and shadow stay.
        let who = is_player
            .then(|| self.remote_player_name(mob.id))
            .flatten()
            .map(|name| PlayerRender { name, local: false });
        let mut frame_draw = FrameDraw::default();
        if !self.mod_render_pre(
            mob,
            who,
            listened,
            alpha as f32,
            camera,
            packed,
            &mut frame_draw,
            out,
        ) {
            // What the mods drew in its place is culled as the entity's own would be.
            self.cull_entity_vertices(out, entity_start);
            return;
        }
        // A mod's renderer for the type (`register_entity_renderers`) draws in place of its
        // model and vanilla layers; the mods' layers and post hooks still follow.
        let modded =
            self.mod_living_renderer_geometry(mob, alpha as f32, camera, packed, true, out);
        // AvatarRenderer.shouldRenderLayers: a spectator draws no layers.
        let layers = sunrise_client::mobs::render::layers_shown(mob);
        let resolved = self.entity_layers(mob);
        let hidden = if modded {
            StockLayerMask::EMPTY
        } else {
            resolved.hidden
        };
        if mob.species() == Species::SulfurCube && !mob.is_invisible() && !modded {
            self.sulfur_cube_body_geometry(
                mob,
                alpha as f32,
                sunrise_client::mobs::render::base_transform(mob, alpha as f32, camera),
                packed,
                out,
            );
        }
        // Other players on a server: PlayerRenderer's HumanoidArmorLayer.
        let armor = if is_player {
            self.worn_armor_layers(mob, hidden)
        } else {
            Vec::new()
        };
        let body = out.len();
        // The GPU path: an ordinary mob, no layers the CPU path changes after drawing them
        // (a player's, the HDR tiers below, the sulfur cube's shell). A glowing mob's
        // silhouette is posed too (`SkinFrame::outline_since`), unless it is invisible:
        // then only the outline draws.
        let silhouette_on_gpu = outline.is_none() || !invisible;
        let posable = skinning
            && !(is_player || modded)
            && !layers_only
            && !seen_invisible
            && silhouette_on_gpu
            && armor.is_empty()
            && !matches!(
                mob.species(),
                Species::Blaze | Species::MagmaCube | Species::GlowSquid | Species::SulfurCube
            );
        let added = if layers { resolved.layers } else { None };
        let data = super::super::humanoid_setup_anim::HiddenLayersData {
            game: self,
            hidden_layers: hidden,
        };
        let mut skinned = None;
        frame_draw.capture_model = !modded
            && (added.is_some() || (listened && self.mob_render_post_listened(who.is_some())));
        if posable {
            let mut models = self.client.skin_models.0.borrow_mut();
            let mut frame = self.client.skin_frame.borrow_mut();
            let mark = frame.mark();
            let mut skin = sunrise_client::mobs::skin::Skin::new(&mut models, &mut frame);
            sunrise_client::mobs::render::render_skinned(
                mob,
                alpha as f32,
                camera,
                &lookup,
                packed,
                &data,
                &mut frame_draw,
                out,
                &mut skin,
            );
            skinned = Some((mark, skin.bounds));
        } else if !modded {
            if seen_invisible {
                sunrise_client::mobs::render::render_seen_invisible(
                    mob,
                    alpha as f32,
                    camera,
                    &lookup,
                    packed,
                    &data,
                    &armor,
                    &mut frame_draw,
                    out,
                );
            } else if layers_only {
                sunrise_client::mobs::render::render_layers(
                    mob,
                    alpha as f32,
                    camera,
                    &lookup,
                    packed,
                    &data,
                    &armor,
                    &mut frame_draw,
                    out,
                );
            } else {
                sunrise_client::mobs::render::render_with_frame(
                    mob,
                    alpha as f32,
                    camera,
                    &lookup,
                    packed,
                    &data,
                    &armor,
                    &mut frame_draw,
                    out,
                );
            }
        }
        let model = frame_draw.model();
        // HDR output: mobs that are light shine by their kind, as blocks do (entity.wgsl
        // fs_hdr_mark); a tier's window lifts only their bright texels.
        if let Some(tier) = match mob.species() {
            Species::Blaze => Some(sunrise_client::mesher::BRIGHT),
            Species::MagmaCube | Species::GlowSquid => Some(sunrise_client::mesher::LOW),
            _ => None,
        } {
            for v in &mut out[body..] {
                v.layer |= u32::from(tier) << 16;
            }
        }
        if is_player && layers {
            // PlayerRenderer's CapeLayer, WingsLayer, ParrotOnShoulderLayer
            // and SpinAttackEffectLayer. Only the cape hides with the body;
            // the others have no `isInvisible` check.
            let chest = mob.extra.slot_id(sunrise_items::equipment::CHEST);
            // `CapeLayer` and `Deadmau5EarsLayer`: `!state.isInvisible`, whoever sees the body.
            if !invisible && !hidden.contains(mod_api::StockLayer::Cape) {
                self.player_cape_geometry(mob, chest, camera, alpha as f32, packed, out);
            }
            if !invisible
                && !hidden.contains(mod_api::StockLayer::Deadmau5Ears)
                && let Some(who) = who
            {
                self.player_ears_geometry(mob, who.name, camera, alpha as f32, packed, out);
            }
            if !hidden.contains(mod_api::StockLayer::Wings) {
                self.player_elytra_geometry(mob, chest, camera, alpha as f32, packed, out);
            }
            if !hidden.contains(mod_api::StockLayer::ParrotOnShoulder) {
                self.shoulder_parrots_geometry(mob, camera, alpha as f32, packed, out);
            }
            if !hidden.contains(mod_api::StockLayer::SpinAttackEffect) {
                self.spin_attack_geometry(mob, camera, alpha as f32, packed, out);
            }
            let view = MobView {
                camera,
                alpha,
                light: packed,
            };
            self.stuck_in_body_geometry(mob, mob.id as i32, view, hidden, out);
        }
        // CustomHeadLayer (a pumpkin, skull or head item) included.
        if layers {
            if !modded {
                // Held items join the GPU path's draws, their silhouette too.
                let skin = (skinning && silhouette_on_gpu).then_some(&mut skinned);
                let view = MobView {
                    camera,
                    alpha,
                    light: packed,
                };
                self.mob_block_geometry(mob, view, hidden, skin, out);
            }
            let stock_ctx = super::super::native_entity_layers::StockLayerContext {
                is_player,
                local_player: false,
                invisible,
                in_world: true,
                player_name: who.map(|w| w.name),
                alpha,
                camera,
                light: packed,
                supported: if modded {
                    StockLayerMask::EMPTY
                } else {
                    resolved.supported
                },
                hidden,
            };
            self.mod_layer_geometry(
                mob,
                who,
                model,
                added,
                &stock_ctx,
                alpha as f32,
                camera,
                packed,
                out,
            );
        }
        self.mod_render_post(mob, who, model, alpha as f32, camera, packed, out);
        if let Some(rgb) = outline {
            self.mark_outline(&out[entity_start..], rgb);
            if let Some(((first, _, _), _)) = skinned {
                let color = sunrise_client::mobs::render::abgr(rgb);
                self.client
                    .skin_frame
                    .borrow_mut()
                    .outline_since(first, color);
            }
            if invisible && !seen_invisible {
                out.truncate(entity_start);
                return;
            }
        }
        // Glowing outlines and a guardian's beam remain visible independently
        // of whether the body is hidden behind terrain.
        if outline.is_none() && culling.checks_vertices() && beam.is_none() {
            // What the GPU draws of the mob counts by its bounds' corners.
            self.cull_skinned(out, entity_start, skinned);
        }
        // GuardianRenderer: the laser to the locked target.
        if let Some((time, scale)) = beam
            && !modded
            && let Some(target) = self.beam_target(mob.ai.target, alpha)
            && let Some(uv) = lookup("entity/guardian/guardian_beam")
        {
            let a = alpha;
            let eye = glam::DVec3::new(
                mob.previous.x + (mob.position.x - mob.previous.x) * a,
                mob.previous.y + (mob.position.y - mob.previous.y) * a + mob.eye_height(),
                mob.previous.z + (mob.position.z - mob.previous.z) * a,
            ) - camera;
            sunrise_client::mobs_render::guardian_beam_geometry(
                eye.as_vec3(),
                (target - camera).as_vec3(),
                time,
                scale,
                uv,
                out,
            );
        }
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;
