//! Custom names: `NameTagItem.interactLivingEntity` and the name tag
//! (`EntityRenderer.submitNameTag`): above the head, facing the camera,
//! on a translucent black background, shown when the name is visible or the
//! mob is under the crosshair within its name-tag distance (64 by default), as its team's name tag
//! visibility allows, in the team's colours, with its score in the
//! objective displayed below names. Other players' names (on a LAN host
//! or a server) show whenever `LivingEntityRenderer.shouldShowName` allows,
//! and the player's own in third person with the `ownNameTag` extra.
use super::billboard_axes;
use sunrise_client::{
    block_entity_render::vertex_light,
    hud::{HudAtlas, HudBatch, HudVertex},
    mesher::Vertex,
};
use sunrise_game_data::{scoreboard as sb, text::Text};
use sunrise_sim::{mobs::Mob, save::uuid_text};

/// The world size of one text pixel in a name tag: `Matrix4f.scale(0.025F, -0.025F, 0.025F)`.
pub(in crate::game) const NAME_TAG_UNIT: f32 = 0.025;
/// Options.getBackgroundOpacity(0.25) black, as ARGB.color.
pub(in crate::game) const BACKGROUND: u32 = 0x3F00_0000;

/// Local glyph quads only: camera transforms and lighting stay per frame.
/// Slots follow submission order; comparing the whole styled text also handles entity reorder,
/// team/score changes and native name hooks without keeping entity IDs alive.
#[derive(Default)]
pub(in crate::game) struct NameTagLayouts {
    pub(in crate::game) epoch: (u64, u64, bool),
    pub(in crate::game) next: usize,
    pub(in crate::game) lines: Vec<Option<NameTagLine>>,
}

pub(in crate::game) struct NameTagLine {
    pub(in crate::game) text: Text,
    pub(in crate::game) alpha: u32,
    pub(in crate::game) background: u32,
    pub(in crate::game) y: f32,
    pub(in crate::game) behind: usize,
    pub(in crate::game) vertices: Vec<HudVertex>,
}

impl NameTagLayouts {
    pub(in crate::game::client_side) fn begin_frame(&mut self, atlas: &HudAtlas) {
        let epoch = (
            atlas.revision,
            atlas.font.epoch(),
            sunrise_game_data::lang::bidirectional(),
        );
        if self.epoch != epoch {
            self.lines.clear();
            self.epoch = epoch;
        }
        self.next = 0;
    }

    pub(in crate::game::client_side) fn end_frame(&mut self) {
        self.lines.truncate(self.next);
    }

    pub(in crate::game::client_side) fn name_tag(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        text: &Text,
        y: f32,
        alpha: u32,
        background: u32,
    ) -> usize {
        // At most 512 * 512 HUD vertices (~10 MiB), plus short text keys. Longer tags and
        // excess lines still render normally. Animated/random glyphs and live inline skins
        // must go through the font each time, including legacy formatting supplied by hooks.
        const MAX_LINES: usize = 512;
        const MAX_VERTICES: usize = 512;
        let slot = self.next;
        self.next += 1;
        let stable =
            text.resolved().spans.iter().all(|span| {
                span.style.obfuscated != Some(true) && span.style.inline_object.is_none()
            }) && text.spans.iter().map(|s| s.text.len()).sum::<usize>() <= 1024;
        if slot < MAX_LINES
            && stable
            && let Some(Some(line)) = self.lines.get(slot)
            && line.text == *text
            && line.alpha == alpha
            && line.background == background
            && line.y == y
        {
            batch.vertices.extend_from_slice(&line.vertices);
            return line.behind;
        }
        let behind = batch.name_tag(atlas, text, y, alpha << 24 | 0xFF_FFFF, background);
        if slot < MAX_LINES {
            if self.lines.len() <= slot {
                self.lines.resize_with(slot + 1, || None);
            }
            self.lines[slot] =
                (stable && batch.vertices.len() <= MAX_VERTICES).then(|| NameTagLine {
                    text: text.clone(),
                    alpha,
                    background,
                    y,
                    behind,
                    vertices: batch.vertices.clone(),
                });
        }
        behind
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `Entity.belowNameDisplay`: the score holder's score in the objective
    /// shown below names, then a space and the objective's name.
    pub(in crate::game::client_side) fn below_name_text(&self, holder: &str) -> Option<Text> {
        let board = &self.common.score.board;
        let objective = board.display_objective(sb::SLOT_BELOW_NAME)?;
        let score = board.score(holder, &objective.name)?;
        // numberFormatOrDefault(StyledFormat.NO_STYLE).
        let format = score
            .number_format
            .clone()
            .or_else(|| objective.number_format.clone())
            .unwrap_or(sb::NumberFormat::Styled(serde_json::json!({})));
        let mut t = Text::empty();
        t.push(format.format(score.value));
        t.push(Text::literal(" "));
        t.push(objective.display_text());
        Some(t)
    }
    /// The other players' models and names: the players a server sends.
    pub(in crate::game::client_side) fn other_players(&self) -> Vec<(&Mob, String)> {
        let mut out = Vec::new();
        let indices = first_mob_indices(&self.common.mobs);
        for (id, name) in self.remote_player_names() {
            if let Some(&index) = indices.get(&id) {
                out.push((&self.common.mobs[index], name));
            }
        }
        out
    }
    /// Sunrise's own name tag extra (`ownNameTag`): the player's own model and
    /// name, while that model shows in third person. Vanilla never names the
    /// camera entity (`LivingEntityRenderer.shouldShowName`).
    pub(in crate::game::client_side) fn own_name_tag(&self) -> Option<(&Mob, String)> {
        if !self.client.settings.extras.own_name_tag
            || !self.controls_camera()
            || self.client.camera_type.is_first_person()
            || self.common.sleeping.is_some()
            || self.common.spectator()
            || self.own_invisible()
            || self.own_render_canceled()
        {
            return None;
        }
        let m = self.client.player_model.as_ref()?;
        Some((m, self.own_profile_name()))
    }
    /// The names the name tags show this frame (the control socket's status).
    pub fn shown_name_tags(&self, camera: glam::DVec3) -> Vec<String> {
        self.name_tags(camera, 1.0)
            .into_iter()
            .filter_map(|t| t.lines.last().map(|(t, _)| t.plain()))
            .collect()
    }
    /// Each name tag this frame.
    pub(in crate::game::client_side) fn name_tags(
        &self,
        camera: glam::DVec3,
        alpha: f64,
    ) -> Vec<NameTag> {
        self.name_tags_with_hooks(camera, alpha, !mod_api::__private::mods().is_empty())
    }
    pub(in crate::game::client_side) fn name_tags_with_hooks(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        hooks: bool,
    ) -> Vec<NameTag> {
        self.collect_name_tags(camera, alpha, hooks, None)
    }
    pub(in crate::game::client_side) fn collect_name_tags(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        hooks: bool,
        frustum: Option<&sunrise_client::gfx::JoinedFrusta>,
    ) -> Vec<NameTag> {
        let target = self.mob_hit().map(|(i, _, _)| self.common.mobs[i].id);
        // Asked once a frame: a mod loaded without this hook costs the tags nothing.
        let name_format = hooks && mod_api::__private::listened(mod_api::HookId::PlayerNameFormat);
        let viewer = self.client_player_team();
        let mut tags = Vec::new();
        let players = self.other_players();
        let player_ids = named_by_player_list(&players);
        let named = self.common.mobs.iter().filter_map(|m| {
            // Already named by the player list: a second tag would sit on the
            // first (`named_by_player_list`). A mob a mod's pre hook canceled has none.
            if player_ids.contains(&m.id) || self.render_canceled(m.id) {
                return None;
            }
            // A mannequin with no name still shows its description.
            let name = match &m.custom_name {
                Some(name) => name.clone(),
                None if hooks || m.species() == sunrise_sim::mobs::Species::Mannequin => {
                    String::new()
                }
                None => return None,
            };
            // The tag's per-entity distance (vanilla's default is 64) is
            // measured to the entity: copy the name only for a mob the frame
            // can show it on.
            let at = glam::DVec3::new(m.position.x, m.position.y, m.position.z);
            let max = m.name_tag_distance();
            (at.distance_squared(camera) < max * max).then_some((m, name, false))
        });
        let own = self.own_name_tag();
        let own_id = own.as_ref().map(|(m, _)| m.id);
        let players = players
            .into_iter()
            .filter(|(m, _)| !self.render_canceled(m.id))
            .chain(own)
            .map(|(m, name)| (m, name, true));
        // Who is riding whom: asked once per tag below, and rescanning every entity
        // for each tag made this quadratic in the number of loaded entities.
        let local_entity_id = self.client.remote.as_ref().map(|r| r.entity_id as u32);
        let ridden = collect_vehicle_ids(
            &self.common.mobs,
            self.remote_vehicle_ids(),
            self.common.riding,
            local_entity_id,
        );
        for (m, name, player) in named.chain(players) {
            if !m.alive() {
                continue;
            }
            // EntityAttachment.NAME_TAG: 0.5 above the box of the current pose.
            let p = name_tag_at(m, alpha);
            let feet = m.previous.lerp(m.position, alpha);
            let origin = glam::DVec3::new(feet.x, feet.y, feet.z);
            let kind = if player {
                TagKind::Fixed("minecraft:player")
            } else {
                TagKind::Mob(m.kind.clone())
            };
            // The name tag distance is measured to the entity.
            let distance_sqr =
                glam::DVec3::new(m.position.x, m.position.y, m.position.z).distance_squared(camera);
            // Match the model's renderer-specific box, not the tag anchor: the
            // name must survive when only the entity's feet are in view.
            let (lo, hi) = super::entity_culling::cull_box(m, alpha);
            let inflate = glam::DVec3::splat(0.5);
            if !name_tag_in_view(
                frustum,
                distance_sqr,
                (lo - camera - inflate).as_vec3(),
                (hi - camera + inflate).as_vec3(),
                m.name_tag_distance(),
            ) {
                continue;
            }
            // A player's scores and team go by name, a mob's by UUID.
            let holder = if player {
                Some(name.clone())
            } else {
                self.common.persist.uuids.get(&m.id).map(|u| uuid_text(*u))
            };
            let team = if player {
                self.common.score.board.players_team(&name)
            } else {
                self.client_mob_team_of(m)
            };
            // LivingEntityRenderer.shouldShowName (team rules included), then
            // MobRenderer: a visible name, or the mob under the crosshair.
            let invisible = sb::invisible_to(
                team,
                viewer,
                self.own_player_is_spectator(),
                m.is_invisible(),
            );
            // Entity.isDiscrete: a crouching player.
            let discrete = player && m.extra.crouching;
            let view = sb::NameView {
                discrete,
                distance_sqr,
                invisible_to_viewer: invisible,
                hud_hidden: self.client.chat_ui.hide_gui,
                is_camera: self.live_remote_camera_id() == Some(m.id),
                is_vehicle: ridden.contains(&m.id),
            };
            // MobRenderer.shouldShowName: a visible name, or the mob under the
            // crosshair; players (LivingEntityRenderer) need neither.
            // `ClientMannequin.belowNameDisplay`: the description is `scoreText`, drawn
            // within the below-name distance (10 by default) whether the name
            // shows or not.
            let description = (m.species() == sunrise_sim::mobs::Species::Mannequin
                && distance_sqr < m.below_name_distance().powi(2))
            .then(|| m.extra.description.component())
            .flatten()
            .map(|tag| sunrise_sim::entity_nbt::name_text(&tag).resolve_legacy());
            let is_own = Some(m.id) == own_id;
            let name_shown = sb::name_shown(team, viewer, view)
                && (!is_own || !view.is_vehicle)
                && (player || m.name_visible || Some(m.id) == target)
                && (player || m.custom_name.is_some());
            let mut visible = name_shown;
            let mut text = None;
            if hooks {
                self.native_render_name_tag_can_render(
                    m.id,
                    if player {
                        "minecraft:player"
                    } else {
                        &m.kind.id
                    },
                    name_shown,
                    &mut visible,
                    &mut text,
                );
            }
            if !visible {
                if let Some(text) = description {
                    let l = self.common.world.light(
                        m.position.x.floor() as i32,
                        (m.position.y + f64::from(m.height())).floor() as i32,
                        m.position.z.floor() as i32,
                    );
                    tags.push(NameTag {
                        at: p,
                        lines: vec![(text, 0.0)],
                        light: l,
                        discrete,
                        entity_id: m.id,
                        kind,
                        origin,
                        score: true,
                    });
                }
                continue;
            }
            // getDisplayName: the name in the team's prefix, suffix and colour.
            let member = if player {
                Text::untrusted(&name)
            } else {
                m.custom_name_text
                    .clone()
                    .unwrap_or_else(|| Text::untrusted(&name))
            };
            // StringDecomposer.iterateFormatted: `§` codes a server leaves
            // inside the team prefix or the name itself apply at draw time.
            let mut shown = match team {
                Some(t) => t.formatted_name(member),
                None => member,
            };
            // Every frame: build the username only for a mod that listens.
            if player
                && name_format
                && let Some(uuid) = self.remote_player_uuid(m.id)
            {
                shown = self.common.player_text(&name, uuid);
            }
            let shown = shown.resolve_legacy();
            let shown = text
                .map(|text| Text::untrusted(text).resolve_legacy())
                .unwrap_or(shown);
            let mut lines = Vec::new();
            // EntityRenderer.extractNameTags: the score shows within the
            // per-entity below-name distance (10 by default), under the name.
            let mut lift = 0.0;
            if m.species() == sunrise_sim::mobs::Species::Mannequin {
                if let Some(text) = description {
                    lines.push((text, 0.0));
                    lift = f64::from(9.0f32 * 1.15 * 0.025);
                }
            } else if distance_sqr < m.below_name_distance().powi(2)
                && let Some(score) = holder.and_then(|h| self.below_name_text(&h))
            {
                lines.push((score.resolve_legacy(), 0.0));
                lift = f64::from(9.0f32 * 1.15 * 0.025);
            }
            lines.push((shown, lift));
            let l = self.common.world.light(
                m.position.x.floor() as i32,
                (m.position.y + f64::from(m.height())).floor() as i32,
                m.position.z.floor() as i32,
            );
            let score = lines.len() == 2;
            tags.push(NameTag {
                at: p,
                lines,
                light: l,
                discrete,
                entity_id: m.id,
                kind,
                origin,
                score,
            });
        }
        drop(player_ids);
        // Armor stands name themselves: an invisible one with a visible name
        // is a hologram (its model is skipped, its name tag is not).
        for a in &self.common.armor_stands {
            if self.hides_as_camera(a.id) {
                continue;
            }
            if !hooks {
                let Some(_) = &a.custom_name else {
                    continue;
                };
                if !a.name_visible {
                    continue;
                }
            }
            let height = match (a.marker, a.small) {
                (true, _) => 0.0,
                (false, true) => 0.9875,
                (false, false) => 1.975,
            };
            let distance_sqr =
                glam::DVec3::new(a.position.x, a.position.y, a.position.z).distance_squared(camera);
            let position = a.render_position(alpha);
            // Same dimensions and dragon-head growth as armor_stand_geometry.
            let width = if a.marker {
                0.0
            } else if a.small {
                0.25
            } else {
                0.5
            };
            let dragon = a.equipment[5]
                .as_ref()
                .is_some_and(|i| &*i.item == "minecraft:dragon_head");
            let grow = if dragon { 1.0 } else { 0.5 };
            let half = glam::DVec3::new(width / 2.0 + grow, grow, width / 2.0 + grow);
            let origin = super::entity_culling::at(position) - camera;
            if !name_tag_in_view(
                frustum,
                distance_sqr,
                (origin - half).as_vec3(),
                (origin + glam::DVec3::new(half.x, height + grow, half.z)).as_vec3(),
                a.name_tag_distance(),
            ) {
                continue;
            }
            let team = self.client_mob_team(a.id);
            // `ArmorStandRenderer.shouldShowName` overrides the living
            // entity's rule with `isCustomNameVisible` alone: no team
            // visibility, no F1, no invisibility.
            let name_shown = a.custom_name.is_some() && a.name_visible;
            let mut visible = name_shown;
            let mut text = None;
            if hooks {
                self.native_render_name_tag_can_render(
                    a.id,
                    "minecraft:armor_stand",
                    name_shown,
                    &mut visible,
                    &mut text,
                );
            }
            if !visible {
                continue;
            }
            let name = a.custom_name.clone().unwrap_or_else(Text::empty);
            let shown = match team {
                Some(t) => t.formatted_name(name.clone()),
                None => name.clone(),
            }
            .resolve_legacy();
            let shown = text
                .map(|text| Text::untrusted(text).resolve_legacy())
                .unwrap_or(shown);
            let mut lines = Vec::new();
            let mut lift = 0.0;
            let holder = self.common.persist.uuids.get(&a.id).map(|u| uuid_text(*u));
            if distance_sqr < a.below_name_distance().powi(2)
                && let Some(score) = holder.and_then(|h| self.below_name_text(&h))
            {
                lines.push((score.resolve_legacy(), 0.0));
                lift = f64::from(9.0f32 * 1.15 * 0.025);
            }
            lines.push((shown, lift));
            // The tag rides with the body, between last tick's position and this one's.
            let at = a.render_position(alpha);
            let l = self.common.world.light(
                at.x.floor() as i32,
                (at.y + height).floor() as i32,
                at.z.floor() as i32,
            );
            let score = lines.len() == 2;
            tags.push(NameTag {
                at: glam::DVec3::new(at.x, at.y + height + 0.5, at.z),
                lines,
                light: l,
                discrete: false,
                entity_id: a.id,
                kind: TagKind::Fixed("minecraft:armor_stand"),
                origin: glam::DVec3::new(at.x, at.y, at.z),
                score,
            });
        }
        // ItemFrameRenderer.shouldShowName/getNameTag: only the targeted
        // frame's custom-named item, with the HUD visible (also glow frames).
        let targeted_frame = (hooks || !self.client.chat_ui.hide_gui)
            .then(|| self.frame_hit())
            .flatten()
            .filter(|(_, distance)| {
                self.before_block(*distance) && self.mob_hit().is_none_or(|(_, d, _)| d > *distance)
            })
            .map(|(index, _)| index);
        let frames = if hooks {
            0..self.common.item_frames.len()
        } else {
            targeted_frame.map_or(0..0, |index| index..index + 1)
        };
        for index in frames {
            let frame = &self.common.item_frames[index];
            let name = frame.item.as_ref().and_then(|item| item.custom_name());
            if !hooks && name.is_none() {
                continue;
            }
            let (lo, hi) = frame.bounds();
            let center = glam::DVec3::from_array(std::array::from_fn(|a| (lo[a] + hi[a]) / 2.0));
            if center.distance_squared(camera) < 64.0 * 64.0 {
                let name_shown = !self.client.chat_ui.hide_gui
                    && targeted_frame == Some(index)
                    && name.is_some();
                let mut visible = name_shown;
                let mut text = None;
                if hooks {
                    self.native_render_name_tag_can_render(
                        frame.id,
                        frame.kind(),
                        name_shown,
                        &mut visible,
                        &mut text,
                    );
                }
                if !visible {
                    continue;
                }
                // NAME_TAG is at the entity height (0.5), then the name
                // submitter lifts it 0.5. ItemFrameRenderer offsets rendering
                // 0.3 toward its facing and -0.25 vertically.
                let step = frame.step();
                let offset = |y| {
                    center
                        + glam::DVec3::new(
                            f64::from(step[0] as f32 * 0.3),
                            y,
                            f64::from(step[2] as f32 * 0.3),
                        )
                };
                tags.push(NameTag {
                    at: offset(0.75),
                    // ItemFrameRenderer.getRenderOffset: the pose the tag is drawn in.
                    origin: offset(-0.25),
                    entity_id: frame.id,
                    kind: TagKind::Fixed(frame.kind()),
                    score: false,
                    lines: vec![(
                        Text::untrusted(text.or(name).unwrap_or_default()).resolve_legacy(),
                        0.0,
                    )],
                    light: self.common.world.light(
                        center.x.floor() as i32,
                        center.y.floor() as i32,
                        center.z.floor() as i32,
                    ),
                    discrete: false,
                });
            }
        }
        // DisplayRenderer.submit -> EntityRenderer.shouldShowName: uses the same visibility
        // decision and per-entity partial tick as the display itself, within 64 blocks.
        let scale = self.view_scale();
        let now = std::time::Instant::now();
        for (&id, display) in &self.client.text_displays.0 {
            if !display.has_render_state() {
                continue;
            }
            let Some(name) = display.custom_name() else {
                continue;
            };
            let entity_partial = self.display_partial(id, alpha);
            if !self.display_visible_with_frustum(
                id,
                display,
                camera,
                entity_partial as f32,
                scale,
                now,
                frustum,
            ) {
                continue;
            }
            let pos = display.position();
            if pos.distance_squared(camera) >= 64.0 * 64.0 {
                continue;
            }
            let name_shown = display.custom_name_visible();
            let mut visible = name_shown;
            let mut text = None;
            if hooks {
                self.native_render_name_tag_can_render(
                    id as u32,
                    display.kind(),
                    name_shown,
                    &mut visible,
                    &mut text,
                );
            }
            if !visible {
                continue;
            }
            let at = display.render_position(entity_partial);
            let team = self.client_mob_team(id as u32);
            let shown = match team {
                Some(t) => t.formatted_name(name.clone()),
                None => name.clone(),
            }
            .resolve_legacy();
            let shown = text
                .map(|text| Text::untrusted(text).resolve_legacy())
                .unwrap_or(shown);
            let light = display.packed_light(&self.common.world, at);
            // Display is sized (0, 0): NAME_TAG attachment is (0, 0, 0), lifted 0.5 by submitNameTag.
            tags.push(NameTag {
                at: glam::DVec3::new(at.x, at.y + 0.5, at.z),
                lines: vec![(shown, 0.0)],
                light,
                discrete: display.is_discrete(),
                entity_id: id as u32,
                kind: TagKind::Fixed(display.kind()),
                origin: glam::DVec3::new(at.x, at.y, at.z),
                score: false,
            });
        }
        tags
    }
    /// `SubmitNodeCollection.submitNameTag`: a discrete name is drawn once,
    /// faded, on its background; any other in full over the world, its faded
    /// copy and background in `see_through`, over everything.
    pub fn name_tag_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        see_through: &mut super::text_displays::SeeThroughCollector,
    ) {
        let mut patches = Vec::new();
        let mut layouts = std::mem::take(&mut self.client.name_tag_layouts);
        layouts.begin_frame(&self.client.gui.atlas);
        let background = self.client.settings.text_background(0.25);
        let faded = self.client.settings.faded_text_alpha(0.25);
        let pack_text =
            (self.client.renderer.0.as_ref()).is_some_and(|r| r.draws_pack_world_text());
        let hooks = !mod_api::__private::mods().is_empty();
        // Asked once a frame: with no listener, no line is posed or copied.
        let do_render =
            hooks && mod_api::__private::listened(mod_api::HookId::RenderNameTagDoRender);
        let mut content = String::new();
        for mut tag in
            self.collect_name_tags(camera, alpha, hooks, self.client.view_frustum.as_ref())
        {
            if do_render {
                self.name_tag_do_render(&mut tag, camera, alpha, out, &mut content);
                if tag.lines.is_empty() {
                    continue;
                }
            }
            let o = tag.at - camera;
            // `gbuffers_entities` draws name tag text (2.3); a sneaking player's faded tag, on its
            // background, blends.
            if tag.discrete {
                self.mark_translucent_entity(out.len(), "minecraft:name_tag");
                let light = vertex_light(tag.light);
                patches.extend(self.styled_name_tag_quads(
                    o,
                    &tag.lines,
                    faded,
                    background,
                    name_tag_layer(pack_text, faded, background),
                    light,
                    NAME_TAG_UNIT,
                    out,
                    Some(&mut layouts),
                ));
                continue;
            }
            // LightCoordsUtil.lightCoordsWithEmission(lightCoords, 2).
            let lit = (tag.light >> 4).max(2) << 4 | (tag.light & 15).max(2);
            self.mark_entity(out.len(), "minecraft:name_tag");
            let light = vertex_light(lit);
            patches.extend(self.styled_name_tag_quads(
                o,
                &tag.lines,
                0xFF,
                0,
                name_tag_layer(pack_text, 0xFF, 0),
                light,
                NAME_TAG_UNIT,
                out,
                Some(&mut layouts),
            ));
            // RenderPipelines.TEXT_SEE_THROUGH: no depth test, no lightmap; sorted with all
            // see-through text submits by camera-relative pose distance.
            let st_start = see_through.start();
            patches.extend(self.styled_name_tag_quads(
                o,
                &tag.lines,
                faded,
                background,
                2,
                0,
                NAME_TAG_UNIT,
                see_through.buffer_mut(),
                Some(&mut layouts),
            ));
            let dist_sq =
                super::text_displays::submit_distance_sq(o.x as f32, o.y as f32, o.z as f32);
            see_through.finish(st_start, dist_sq);
        }
        layouts.end_frame();
        self.client.name_tag_layouts = layouts;
        self.client.map_patches.extend(patches);
    }
    /// `render_name_tag_do_render` for each line of `tag`, in vanilla's order (score, then name),
    /// keeping what the listeners draw and dropping the lines they cancel. The name keeps its lift
    /// when the score line is canceled (`EntityRenderer.submitNameDisplay` lifts outside the `if`).
    /// `content` is the frame's one buffer for the lines' text.
    fn name_tag_do_render(
        &self,
        tag: &mut NameTag,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        content: &mut String,
    ) {
        let start = out.len();
        // `NAME_TAG` attachment: `at` is 0.5 above it (`submitNameTag`).
        let attachment = tag.at - tag.origin - glam::DVec3::new(0.0, 0.5, 0.0);
        let mut first = true;
        tag.lines.retain(|(text, lift)| {
            let line = if std::mem::take(&mut first) && tag.score {
                mod_api::NameTagLine::Score
            } else {
                mod_api::NameTagLine::Name
            };
            content.clear();
            text.write_plain(content);
            !self.mod_name_tag_line(
                tag.entity_id,
                tag.kind.id(),
                line,
                content,
                tag.origin,
                *lift,
                attachment,
                alpha as f32,
                camera,
                tag.light,
                out,
            )
        });
        if out.len() > start {
            self.mark_entity(start, tag.kind.id());
        }
    }
    /// The lines of one name tag, `o` from the camera: the text at
    /// `text_alpha` in its layer, on the translucent black background
    /// (in `text_layer` like the glyphs, a hair behind) when `background`.
    /// Returns the glyph pixels the atlas still needs.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::client_side) fn name_tag_quads(
        &self,
        o: glam::DVec3,
        lines: &[(Text, f64)],
        text_alpha: u32,
        background: bool,
        text_layer: u32,
        light: u32,
        unit: f32,
        out: &mut Vec<Vertex>,
    ) -> Vec<sunrise_client::font::Patch> {
        self.styled_name_tag_quads(
            o,
            lines,
            text_alpha,
            if background { BACKGROUND } else { 0 },
            text_layer,
            light,
            unit,
            out,
            None,
        )
    }

    /// [`Self::name_tag_quads`] on a `background` of its own (ARGB, none when its alpha is 0).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::client_side) fn styled_name_tag_quads(
        &self,
        o: glam::DVec3,
        lines: &[(Text, f64)],
        text_alpha: u32,
        background: u32,
        text_layer: u32,
        light: u32,
        unit: f32,
        out: &mut Vec<Vertex>,
        mut layouts: Option<&mut NameTagLayouts>,
    ) -> Vec<sunrise_client::font::Patch> {
        // SubmitNodeCollection.submitNameTag turns the tag by the camera's orientation
        // (Camera.rotation()), turned round in third-person front view, so the text reads the
        // right way from where it is seen.
        let orientation = self.camera_orientation();
        let [right, up] = billboard_axes(orientation);
        let away = orientation * glam::Vec3::NEG_Z * 0.002;
        let mut batch = HudBatch::new(
            [1, 1],
            sunrise_client::hud::GuiScale {
                requested: 1,
                unicode: false,
            },
        );
        for (text, lift) in lines {
            let o = (o + glam::DVec3::new(0.0, *lift, 0.0)).as_vec3();
            let (r, u) = (right.as_vec3() * unit, up.as_vec3() * unit);
            // Text space: x right, y down (scale 0.025, -0.025).
            let at = |x: f32, y: f32| o + r * x - u * y;
            let y0 = if text.plain() == "deadmau5" {
                -10.0
            } else {
                0.0
            };
            batch.vertices.clear();
            // Font.prepareText, as any text: bold, italic, fonts and each
            // glyph's own box (accented.png's are 12 pixels tall).
            let behind = match layouts.as_deref_mut() {
                Some(layouts) => layouts.name_tag(
                    &mut batch,
                    &self.client.gui.atlas,
                    text,
                    y0,
                    text_alpha,
                    background,
                ),
                None => batch.name_tag(
                    &self.client.gui.atlas,
                    text,
                    y0,
                    text_alpha << 24 | 0xFF_FFFF,
                    background,
                ),
            };
            for (i, quad) in batch.vertices.as_chunks::<6>().0.iter().enumerate() {
                let shift = if i * 6 < behind {
                    away
                } else {
                    glam::Vec3::ZERO
                };
                // The corners from the bottom left, as the world draws quads.
                for v in [quad[5], quad[2], quad[1], quad[0]] {
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: (at(v.pos[0], v.pos[1]) + shift).to_array(),
                        uv: v.uv,
                        layer: text_layer,
                        color: v.color,
                        light,
                        glint_uv: [0.0, 0.0],
                    });
                }
            }
        }
        // Glyphs rasterized on first use (Unifont's) upload with the frame.
        batch.patches
    }
}

/// The vertex layer for a name tag's quads (`RenderTypes.text`) when a shader pack draws the
/// world's text (`pack_text`). `text_alpha` is 0 to 255, `background` ARGB.
pub(in crate::game) fn name_tag_layer(pack_text: bool, text_alpha: u32, background: u32) -> u32 {
    if !pack_text {
        0
    } else if text_alpha == 0xFF && background >> 24 == 0 {
        sunrise_client::gfx::NAME_TAG_SOLID_OVERLAY
    } else {
        sunrise_client::gfx::NAME_TAG_OVERLAY
    }
}

/// Use the renderer's camera-relative clip test before measuring or emitting text.
pub(in crate::game::client_side) fn name_tag_in_view(
    frustum: Option<&impl sunrise_client::gfx::Cull>,
    distance_sqr: f64,
    min: glam::Vec3,
    max: glam::Vec3,
    max_distance: f64,
) -> bool {
    distance_sqr < max_distance * max_distance
        && frustum.is_none_or(|view| view.intersects(min, max))
}

pub(in crate::game::client_side) fn first_mob_indices(
    mobs: &[Mob],
) -> std::collections::HashMap<u32, usize> {
    let mut indices = std::collections::HashMap::with_capacity(mobs.len());
    for (index, mob) in mobs.iter().enumerate() {
        // Match iter().find even if a transient snapshot contains duplicate IDs.
        indices.entry(mob.id).or_insert(index);
    }
    indices
}

/// Collect entity IDs of all vehicles: entities that have passengers
/// (`passenger_vehicles`: what every entity the server sent rides, text displays,
/// armor stands, item displays and interactions included; vanilla's `isVehicle` is
/// any passenger at all).
///
/// On multiplayer servers (e.g. `mcpvp.club`), server plugins hang custom nametags
/// (ranks, prefixes, stats) over players using `TextDisplay` or `ArmorStand` passengers.
/// In vanilla `LivingEntityRenderer.shouldShowName`, a vehicle does not show its default
/// name tag (`!entity.isVehicle()`).
///
/// If the local player's remote entity ID is in the set, ID 0 (`player_model`) is also
/// added so that Sunrise's own name tag extra (`ownNameTag`) recognizes that the player
/// is ridden and suppresses the duplicate tag (#9477).
pub(in crate::game::client_side) fn collect_vehicle_ids(
    mobs: &[Mob],
    passenger_vehicles: impl IntoIterator<Item = u32>,
    local_riding: Option<u32>,
    local_entity_id: Option<u32>,
) -> std::collections::HashSet<u32> {
    let mut ridden: std::collections::HashSet<u32> = mobs
        .iter()
        .filter_map(|o| o.extra.mount)
        .chain(mobs.iter().filter_map(|m| m.extra.rider.map(|_| m.id)))
        .chain(passenger_vehicles)
        .collect();
    if let Some(riding) = local_riding {
        ridden.insert(riding);
    }
    if let Some(id) = local_entity_id
        && ridden.contains(&id)
    {
        // The local player's model has id 0 (player_model).
        ridden.insert(0);
    }
    ridden
}

/// The entity ids the player-list pass names, which the custom-name pass must
/// therefore skip.
///
/// Vanilla draws one tag per entity, and names a player from its profile:
/// `Player.getName` overrides `Entity.getName` and ignores `customName`. So a
/// server that sets a custom name on a player entity -- a rank, a nickname, a
/// prefix -- must not get a tag of its own drawn over the one the player list
/// already draws.
///
/// Built whether or not native mods are loaded. It was briefly built only with
/// them, as a fast path, and without them both tags were drawn on top of each
/// other on every server that names its players this way. The set is small --
/// the players in view -- and `other_players` has already allocated a name
/// string for each, so skipping it saved very little.
pub(in crate::game::client_side) fn named_by_player_list(
    players: &[(&Mob, String)],
) -> std::collections::HashSet<u32> {
    players.iter().map(|(player, _)| player.id).collect()
}
/// `EntityAttachment.NAME_TAG` of the entity's current pose
/// (`Mob::name_tag_height`), which `SubmitNodeCollector.submitNameTag` lifts
/// 0.5 more, between the last two tick positions. The player box is
/// `Pose::dimensions(1.0)`.
/// shortcut: the scale attribute is not in the tag height, add `model_scale`
/// when tags on scaled players matter.
pub fn name_tag_at(m: &Mob, alpha: f64) -> glam::DVec3 {
    glam::DVec3::new(
        m.previous.x + (m.position.x - m.previous.x) * alpha,
        m.previous.y + (m.position.y - m.previous.y) * alpha + m.name_tag_height() + 0.5,
        m.previous.z + (m.position.z - m.previous.z) * alpha,
    )
}

/// A name tag: where it hangs, its lines (text, lift), the packed light at
/// the entity and whether it is discrete.
pub(in crate::game) struct NameTag {
    pub(in crate::game) at: glam::DVec3,
    pub(in crate::game) lines: Vec<(Text, f64)>,
    pub(in crate::game) light: u8,
    pub(in crate::game) discrete: bool,
    pub(in crate::game) entity_id: u32,
    pub(in crate::game) kind: TagKind,
    pub(in crate::game) origin: glam::DVec3,
    pub(in crate::game) score: bool,
}

/// The type id of a name tag's entity, borrowed when a hook is asked.
pub(in crate::game) enum TagKind {
    Mob(std::sync::Arc<sunrise_sim::mobs::MobType>),
    Fixed(&'static str),
}

impl TagKind {
    pub(in crate::game) fn id(&self) -> &str {
        match self {
            Self::Mob(kind) => &kind.id,
            Self::Fixed(id) => id,
        }
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::name_component::name_component;
    use sunrise_sim::mobs::{Anim, Brain, MobType, Species};
    use sunrise_sim::pose::Pose;

    #[test]
    fn a_mobs_name_is_sent_with_its_style() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        let Ok(types) = sunrise_sim::mobs::MobTypes::load(&root) else {
            return;
        };
        let cow = Arc::new(types.types["minecraft:cow"].clone());
        let mut m = Mob::new(1, cow, Vec3::new(0.0, 64.0, 0.0), 0.0);
        m.custom_name = Some("RED NAME".into());
        let plain = sunrise_protocol::text::literal("RED NAME");
        assert_eq!(name_component(&m, "RED NAME"), plain);
        let styled = sunrise_core::nbt::Tag::compound([
            ("text", sunrise_core::nbt::Tag::String("RED NAME".into())),
            ("color", sunrise_core::nbt::Tag::String("red".into())),
        ]);
        m.custom_name_text = Some(sunrise_sim::entity_nbt::name_text(&styled));
        let sent = sunrise_sim::entity_nbt::name_text(&name_component(&m, "RED NAME"));
        assert_eq!(sent, sunrise_sim::entity_nbt::name_text(&styled));
        assert_ne!(sent, sunrise_sim::entity_nbt::name_text(&plain));
    }

    fn tag_font() -> HudAtlas {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "sunrise-tag-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let textures = root.join("assets/minecraft/textures/font");
        std::fs::create_dir_all(root.join("assets/minecraft/font")).unwrap();
        std::fs::create_dir_all(&textures).unwrap();
        std::fs::write(root.join("assets/minecraft/font/default.json"),
            r#"{"providers":[{"type":"bitmap","file":"minecraft:font/cache.png","ascent":7,"chars":["ab"]}]}"#).unwrap();
        let file = std::fs::File::create(textures.join("cache.png")).unwrap();
        let mut encoder = png::Encoder::new(file, 16, 8);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let pixels: Vec<u8> = (0..128)
            .flat_map(|i| [255, 255, 255, if i % 8 < 4 { 255 } else { 0 }])
            .collect();
        writer.write_image_data(&pixels).unwrap();
        drop(writer);
        let mut atlas = HudAtlas::boot();
        atlas.font = sunrise_client::font::Font::load(
            &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
            &atlas.sprites,
            &[],
            (1024, 1024),
            (512, 512),
            atlas.sprites["sunrise/white"],
        );
        std::fs::remove_dir_all(root).unwrap();
        atlas
    }

    fn tag_bytes(
        cache: &mut NameTagLayouts,
        atlas: &HudAtlas,
        text: &Text,
        alpha: u32,
        background: bool,
    ) -> Vec<u8> {
        cache.begin_frame(atlas);
        let mut batch = HudBatch::scratch([1, 1]);
        let backdrop = if background { BACKGROUND } else { 0 };
        let behind = cache.name_tag(&mut batch, atlas, text, 0.0, alpha, backdrop);
        cache.end_frame();
        let mut fresh = HudBatch::scratch([1, 1]);
        assert_eq!(
            behind,
            fresh.name_tag(
                atlas,
                text,
                0.0,
                alpha << 24 | 0xFF_FFFF,
                if background { 0x3F00_0000 } else { 0 }
            )
        );
        let bytes: &[u8] = bytemuck::cast_slice(&batch.vertices);
        assert_eq!(bytes, bytemuck::cast_slice::<_, u8>(&fresh.vertices));
        bytes.to_vec()
    }

    #[test]
    fn cached_tag_quads_follow_text_style_pass_and_font_changes() {
        let mut atlas = tag_font();
        let mut cache = NameTagLayouts::default();
        let a = Text::literal("a");
        let first = tag_bytes(&mut cache, &atlas, &a, 255, false);
        assert!(!first.is_empty());
        assert_eq!(tag_bytes(&mut cache, &atlas, &a, 255, false), first);
        for text in [
            Text::literal("b"),
            a.clone().bold(),
            Text::colored("a", 0x12AB34),
        ] {
            assert_ne!(tag_bytes(&mut cache, &atlas, &text, 255, false), first);
            tag_bytes(&mut cache, &atlas, &text, 255, false);
        }
        assert_ne!(tag_bytes(&mut cache, &atlas, &a, 127, true), first);
        tag_bytes(&mut cache, &atlas, &a, 127, true);
        tag_bytes(&mut cache, &atlas, &a, 255, false);
        // Reset and occupy the first glyph slot with b: a's old UV is now wrong.
        atlas.font.reset_cache();
        atlas.font.glyph('b');
        assert_ne!(tag_bytes(&mut cache, &atlas, &a, 255, false), first);
        // Font options may change without replacing the atlas.
        atlas.font.uniform = true;
        tag_bytes(&mut cache, &atlas, &a, 255, false);
    }

    #[test]
    fn obfuscated_tag_glyphs_keep_animating_after_cache_warmup() {
        let atlas = tag_font();
        let mut cache = NameTagLayouts::default();
        let text = Text::legacy("§kaaaaaaaaaaaaaaaa");
        let mut images = std::collections::HashSet::new();
        for seed in 0..8 {
            cache.begin_frame(&atlas);
            atlas.font.reseed(seed);
            let mut batch = HudBatch::scratch([1, 1]);
            cache.name_tag(&mut batch, &atlas, &text, 0.0, 255, 0);
            cache.end_frame();
            let bytes = bytemuck::cast_slice::<_, u8>(&batch.vertices).to_vec();
            atlas.font.reseed(seed);
            let mut fresh = HudBatch::scratch([1, 1]);
            fresh.name_tag(&atlas, &text, 0.0, u32::MAX, 0);
            assert_eq!(bytes, bytemuck::cast_slice::<_, u8>(&fresh.vertices));
            images.insert(bytes);
        }
        assert!(images.len() > 1, "the fixture must actually animate");
    }

    #[test]
    fn inline_tag_objects_follow_live_sprite_updates() {
        let mut atlas = HudAtlas::boot();
        let mut sprite = atlas.sprites["sunrise/white"];
        atlas.sprites.insert("item/probe".into(), sprite);
        let text = sunrise_game_data::component::to_text(
            &serde_json::json!({"object":"atlas", "sprite":"minecraft:item/probe"}),
            &mut |_| Text::empty(),
        );
        let mut cache = NameTagLayouts::default();
        let first = tag_bytes(&mut cache, &atlas, &text, 255, false);
        assert!(!first.is_empty());
        sprite.u0 += 0.1;
        atlas.sprites.insert("item/probe".into(), sprite);
        assert_ne!(tag_bytes(&mut cache, &atlas, &text, 255, false), first);
    }

    #[test]
    fn name_tag_culling_keeps_intersecting_entity_boxes_within_distance() {
        // A 90-degree camera looking down -Z, with near/far at 1/100.
        pub struct View;
        impl sunrise_client::gfx::Cull for View {
            fn intersects(&self, lo: glam::Vec3, hi: glam::Vec3) -> bool {
                lo.z <= -1.0
                    && hi.z >= -100.0
                    && lo.x + lo.z <= 0.0
                    && hi.x - lo.z >= 0.0
                    && lo.y + lo.z <= 0.0
                    && hi.y - lo.z >= 0.0
            }
            fn contains(&self, lo: glam::Vec3, hi: glam::Vec3) -> bool {
                self.intersects(lo, hi)
            }
        }
        // The head/anchor at y=12 is outside the top plane, but the
        // entity's feet are inside: vanilla still submits its name.
        assert!(name_tag_in_view(
            Some(&View),
            181.0,
            glam::Vec3::new(-0.8, 8.5, -10.8),
            glam::Vec3::new(0.8, 11.3, -9.2),
            64.0
        ));
        for (offset, distance, expected) in [
            ([0.0, 0.0, -10.0], 100.0, true),
            ([0.0, 0.0, 10.0], 100.0, false),
            ([12.0, 0.0, -10.0], 244.0, false),
            ([0.0, 12.0, -10.0], 244.0, false),
            ([0.0, 0.0, -65.0], 4225.0, false),
            ([0.0, 0.0, -64.0], 4096.0, false),
            ([0.0, 0.0, -63.0], 3969.0, true),
        ] {
            assert_eq!(
                name_tag_in_view(
                    Some(&View),
                    distance,
                    glam::Vec3::from_array(offset) - glam::Vec3::splat(0.5),
                    glam::Vec3::from_array(offset) + glam::Vec3::splat(0.5),
                    64.0
                ),
                expected
            );
        }
        // A server-synced `minecraft:name_tag_distance` of 5 hides a tag
        // six blocks away even though the vanilla default reaches 64.
        assert!(!name_tag_in_view(
            Some(&View),
            36.0,
            glam::Vec3::splat(-0.5),
            glam::Vec3::splat(0.5),
            5.0
        ));
    }

    #[test]
    fn name_tag_id_lookup_matches_scan_on_randomized_duplicate_lists() {
        let mut seed = 4664_u32;
        for len in [0, 1, 10, 150, 300] {
            let mut mobs = Vec::new();
            for _ in 0..len {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let mut mob = player_mob();
                mob.id = seed % 80;
                mobs.push(mob);
            }
            let indices = first_mob_indices(&mobs);
            for id in 0..100 {
                assert_eq!(
                    indices.get(&id).copied(),
                    mobs.iter().position(|m| m.id == id)
                );
            }
        }
    }

    /// The player model `perspective::player_type` builds from the prepared
    /// resources, without them: its box only needs the id and the pointer.
    fn player_mob() -> Mob {
        let kind = MobType {
            type_key: Default::default(),
            id: "minecraft:player".into(),
            species: Species::Zombie,
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            category: "misc".into(),
            max_health: 20.0,
            speed: 0.1,
            flying_speed: 0.02,
            attack_damage: 1.0,
            attack_knockback: 0.0,
            follow_range: 0.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Default::default(),
            item_stack_limits: Default::default(),
            attribute_bounds: Default::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Default::default(),
            brain: Brain::Passive,
            anim: Anim::Humanoid { zombie_arms: false },
            sounds: [
                "entity.player.hurt",
                "entity.player.hurt",
                "entity.player.death",
            ],
            sound_overrides: Default::default(),
            food: Default::default(),
            tempt: Default::default(),
            passenger_attachments: Vec::new(),
        };
        Mob::new(0, Arc::new(kind), Vec3::ZERO, 0.0)
    }

    /// A server that names its players -- a rank prefix, a nickname -- sets a
    /// custom name on the player entity. The player list already names that
    /// player, so the custom-name pass has to skip it or the two tags are drawn
    /// on top of each other. The set of already-named players was briefly built
    /// only when native mods were loaded, so without mods every such server
    /// showed the pair overlapping.
    ///
    /// An entity the player list does not name keeps its own tag: that is how a
    /// shopkeeper NPC spawned as a player entity is named at all.
    #[test]
    fn a_listed_player_is_named_once_whatever_custom_name_a_server_sets() {
        let mut listed = player_mob();
        listed.id = 7;
        listed.custom_name = Some("[MVP+] Steve".to_owned());
        let mut unlisted = player_mob();
        unlisted.id = 9;
        unlisted.custom_name = Some("Shopkeeper".to_owned());
        // The player list names 7 and knows nothing of 9.
        let players = vec![(&listed, "Steve".to_owned())];
        let named = named_by_player_list(&players);
        assert!(
            named.contains(&listed.id),
            "the player list names 7, so its custom name must not add a second tag"
        );
        assert!(
            !named.contains(&unlisted.id),
            "9 is named by nothing else, so its custom name is its only tag"
        );
    }

    /// #4129: the tag hangs from the box of the pose (0.6 swimming, 1.5
    /// crouching, 1.8 standing) and never from the sneak flag, which a
    /// sprint-swimming or gliding player still has set. Today's
    /// `if player && m.extra.crouching { 1.5 }` gives 2.0 for the first.
    #[test]
    fn a_name_tag_hangs_from_the_pose_box_not_the_sneak_flag() {
        let mut m = player_mob();
        let at_y = |m: &Mob| name_tag_at(m, 1.0).y;
        m.extra.crouching = true;
        m.extra.avatar_pose = Pose::Swimming;
        assert!((at_y(&m) - 1.1).abs() < 1e-6, "0.6 + 0.5, not 2.0");
        // A crouching player's tag keeps the height it had: 1.5 + 0.5.
        m.extra.avatar_pose = Pose::Crouching;
        assert!((at_y(&m) - 2.0).abs() < 1e-6, "1.5 + 0.5, as before");
        m.extra.avatar_pose = Pose::Standing;
        m.extra.crouching = false;
        assert!((at_y(&m) - 2.3).abs() < 1e-6, "1.8 + 0.5");
    }

    /// #9477: When server plugins attach a TextDisplay or ArmorStand to a player
    /// (e.g. rank prefixes on multiplayer lobbies), vanilla's `LivingEntityRenderer.shouldShowName`
    /// hides the default nametag because `entity.isVehicle()` is true.
    ///
    /// For the local player with `ownNameTag: true`, `player_model` has ID 0 while
    /// the server sends passengers for `local_entity_id`. `collect_vehicle_ids`
    /// maps `local_entity_id -> 0` so that `ownNameTag` is not drawn twice over the
    /// server's rider nametag.
    #[test]
    fn vehicle_with_text_display_or_armor_stand_passenger_is_marked_ridden() {
        let mut remote_player = player_mob();
        remote_player.id = 42;
        let mobs = vec![remote_player];

        // 1. TextDisplay riding remote player 42
        let ridden = collect_vehicle_ids(&mobs, vec![42], None, Some(100));
        assert!(
            ridden.contains(&42),
            "remote player 42 is ridden by text display"
        );
        assert!(!ridden.contains(&100), "local player 100 has no passenger");
        assert!(!ridden.contains(&0), "local model 0 has no passenger");

        // 2. ArmorStand riding local player 100 -> model 0 must be in ridden
        let ridden_local = collect_vehicle_ids(&mobs, vec![100], None, Some(100));
        assert!(
            ridden_local.contains(&100),
            "local player 100 is ridden by armor stand"
        );
        assert!(
            ridden_local.contains(&0),
            "local player model (id 0) must be mapped to vehicle so ownNameTag is suppressed (#9477)"
        );

        // 3. Name tag is suppressed for vehicles when team is None
        let view_vehicle = sb::NameView {
            is_vehicle: true,
            ..Default::default()
        };
        assert!(
            !sb::name_shown(None, None, view_vehicle),
            "vanilla shouldShowName hides tag for vehicles with no team"
        );
    }
}
