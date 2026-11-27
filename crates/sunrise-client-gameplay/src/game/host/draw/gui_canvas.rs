//! Direct mod GUI drawing on the hook's HUD batch.

use super::text_layout::LabelLayout;
use crate::game::ClientGame;
use mod_api::{
    GuiBlitRegion, GuiCanvas, GuiDrawError, GuiSpriteRegion, GuiVertex, ItemStack, Text,
    TextLayoutSize, TextLayoutSpec,
};
use sunrise_client::{
    hud::{GuiSprite, HudAtlas, HudBatch},
    hud_layout::{
        Blend, IDENTITY, Pose, PoseOp, ResolvedGuiQuad, abgr, paint_fill_gradient,
        paint_player_face, pose_apply, pose_point,
    },
    items::ItemStack as EngineStack,
    skins::HeadSkinJournal,
};
use sunrise_gameplay_common::game::api_values::{TryIntoEngine, stack_value};

pub(in crate::game) struct GameGuiCanvas<'a, 'g> {
    batch: &'a mut HudBatch,
    atlas: &'a HudAtlas,
    /// Absent only for the renderer-only canvas fixture.
    game: Option<&'a ClientGame<'g>>,
    head_skins: &'a HeadSkinJournal,
    pose: Pose,
    /// Screen-space edges, not affected by future changes of pose.
    scissor: Option<[f32; 4]>,
    /// A decorator may paint the standard suffix, but never enter the decorator walk again.
    decorating: bool,
}

impl ClientGame<'_> {
    pub(in crate::game) fn draw_item_decorator(
        &self,
        batch: &mut HudBatch,
        stack: &EngineStack,
        x: f32,
        y: f32,
    ) {
        let _ = GameGuiCanvas::draw_root(
            batch,
            &self.client.gui.atlas,
            Some(self),
            None,
            true,
            &mut |draw| {
                self.native_item_decorations(stack, x, y, draw);
                Ok(())
            },
        );
    }
}

impl GameGuiCanvas<'_, '_> {
    pub(in crate::game) fn draw(
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        game: Option<&ClientGame<'_>>,
        scissor: Option<[f32; 4]>,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError> {
        Self::draw_root(batch, atlas, game, scissor, false, draw)
    }

    fn draw_root(
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        game: Option<&ClientGame<'_>>,
        scissor: Option<[f32; 4]>,
        decorating: bool,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError> {
        let head_skins = HeadSkinJournal::default();
        GameGuiCanvas {
            batch,
            atlas,
            game,
            head_skins: &head_skins,
            pose: IDENTITY,
            scissor,
            decorating,
        }
        .group(draw)?;
        if let Some(game) = game
            && !head_skins.is_empty()
        {
            game.commit_head_skin_journal(head_skins);
        }
        Ok(())
    }

    fn group(
        self,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError> {
        let GameGuiCanvas {
            batch,
            atlas,
            game,
            head_skins,
            pose,
            scissor,
            decorating,
        } = self;
        head_skins.scope(|| {
            batch.draw_scope(&atlas.font, |batch| {
                let start = batch.vertices.len();
                draw(&mut GameGuiCanvas {
                    batch,
                    atlas,
                    game,
                    head_skins,
                    pose,
                    scissor,
                    decorating,
                })?;
                if let Some([left, top, right, bottom]) = scissor {
                    batch.clip_since(start, left, top, right, bottom);
                }
                Ok(())
            })
        })
    }

    /// Validate the posed slot before asking its model or bar hooks to run. Oversized models
    /// and count text may extend beyond the slot; `paint` validates their emitted vertices.
    fn check_item_geometry(&self, [x, y]: [f32; 2]) -> Result<(), GuiDrawError> {
        let [right, bottom] = rect_end([x, y, 16.0, 16.0])?;
        if self.pose == IDENTITY
            || [[x, y], [right, y], [right, bottom], [x, bottom]]
                .into_iter()
                .flat_map(|point| pose_point(self.pose, point))
                .all(f32::is_finite)
        {
            Ok(())
        } else {
            Err(GuiDrawError::NotFinite)
        }
    }

    /// An individual call can be refused even when the caller chooses to commit its group.
    fn paint(&mut self, emit: impl FnOnce(&mut HudBatch, &HudAtlas)) -> Result<(), GuiDrawError> {
        self.paint_in_pose(self.pose, |batch, atlas| {
            emit(batch, atlas);
            Ok(())
        })
    }

    fn paint_in_pose<T>(
        &mut self,
        pose: Pose,
        emit: impl FnOnce(&mut HudBatch, &HudAtlas) -> Result<T, GuiDrawError>,
    ) -> Result<T, GuiDrawError> {
        let atlas = self.atlas;
        self.batch.draw_scope(&atlas.font, |batch| {
            let start = batch.vertices.len();
            let output = emit(batch, atlas)?;
            for vertex in &mut batch.vertices[start..] {
                if pose != IDENTITY {
                    vertex.pos = pose_point(pose, vertex.pos);
                }
                if !vertex.pos.into_iter().chain(vertex.uv).all(f32::is_finite) {
                    return Err(GuiDrawError::NotFinite);
                }
            }
            Ok(output)
        })
    }
}

/// Multiplies the vertices from `start` on by `tint` (packed ABGR), as `item_tinted` does. An
/// OptiFine CIT glint layer's vertices (`Blend::CitGlint`) carry the layer's slot in red and its
/// intensity in alpha, so only their alpha is multiplied, by the tint's alpha: the slot stays,
/// and the layer fades with the item.
fn tint_vertices(batch: &mut HudBatch, start: usize, tint: u32) {
    let cit: Vec<std::ops::Range<usize>> = batch
        .blends
        .iter()
        .filter(|(range, blend)| {
            matches!(blend, sunrise_client::hud::Blend::CitGlint(_)) && range.end as usize > start
        })
        .map(|(range, _)| range.start as usize..range.end as usize)
        .collect();
    let alpha = tint | 0x00FF_FFFF;
    for (i, vertex) in batch.vertices.iter_mut().enumerate().skip(start) {
        let tint = if cit.iter().any(|range| range.contains(&i)) {
            alpha
        } else {
            tint
        };
        vertex.color = multiply_abgr(vertex.color, tint);
    }
}

/// Two packed colours multiplied channel by channel, as a shader multiplies a vertex colour by
/// `ColorModulator`: `u32::MAX` leaves `color` unchanged.
fn multiply_abgr(color: u32, tint: u32) -> u32 {
    let channel = |shift: u32| {
        let product = ((color >> shift) & 0xFF) * ((tint >> shift) & 0xFF);
        // Rounded division by 255, exact for every product of two bytes.
        ((product + 128 + ((product + 128) >> 8)) >> 8) << shift
    };
    channel(0) | channel(8) | channel(16) | channel(24)
}

/// The opposite corner of a finite `[x, y, w, h]`, including finite endpoint arithmetic.
fn rect_end([x, y, w, h]: [f32; 4]) -> Result<[f32; 2], GuiDrawError> {
    let end = [x + w, y + h];
    if [x, y].into_iter().chain(end).all(f32::is_finite) {
        Ok(end)
    } else {
        Err(GuiDrawError::NotFinite)
    }
}

/// The checks `blit` and `texture` share, before either looks its texture up: a finite
/// destination rectangle and `uv`, and no zero `tex` side (the divisor of the UVs).
fn check_blit_region(dst: [f32; 4], uv: [f32; 2], tex: [i32; 2]) -> Result<(), GuiDrawError> {
    rect_end(dst)?;
    if !uv.into_iter().all(f32::is_finite) {
        return Err(GuiDrawError::NotFinite);
    }
    if tex.contains(&0) {
        return Err(GuiDrawError::BadBlitRegion);
    }
    Ok(())
}

/// The API count is nonzero, but vanilla also treats air as an empty stack.
fn drawable_stack(stack: &ItemStack) -> Result<Option<EngineStack>, GuiDrawError> {
    let stack = stack_value(stack).map_err(GuiDrawError::Stack)?;
    Ok((&*stack.item != "minecraft:air").then_some(stack))
}

impl GuiCanvas for GameGuiCanvas<'_, '_> {
    fn size(&self) -> [f32; 2] {
        [self.batch.width, self.batch.height]
    }

    fn scope(
        &mut self,
        ops: &[PoseOp],
        scissor: Option<[f32; 4]>,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError> {
        let mut pose = self.pose;
        for &op in ops {
            let finite = match op {
                PoseOp::Translate(x, y) | PoseOp::Scale(x, y) => x.is_finite() && y.is_finite(),
                PoseOp::Rotate(angle) => angle.is_finite(),
            };
            if !finite {
                return Err(GuiDrawError::NotFinite);
            }
            pose_apply(&mut pose, op);
            if !pose.into_iter().all(f32::is_finite) {
                return Err(GuiDrawError::NotFinite);
            }
        }
        let clip = if let Some(rect @ [x, y, w, h]) = scissor {
            let end = rect_end(rect)?;
            let [x, y] = pose_point(pose, [x, y]);
            let [right, bottom] = pose_point(pose, end);
            // ScreenRectangle transforms the two ordered corners, not an enclosing box;
            // it floors the origin and extents separately.
            let [left, top] = [x.floor(), y.floor()];
            let [width, height] = [(right - x).floor(), (bottom - y).floor()];
            let mut clip = [left, top, left + width, top + height];
            if !clip.into_iter().all(f32::is_finite) {
                return Err(GuiDrawError::NotFinite);
            }
            let [x0, y0, x1, y1] =
                self.scissor
                    .unwrap_or([0.0, 0.0, self.batch.width, self.batch.height]);
            clip = [
                clip[0].max(x0),
                clip[1].max(y0),
                clip[2].min(x1),
                clip[3].min(y1),
            ];
            if w <= 0.0 || h <= 0.0 || clip[2] <= clip[0] || clip[3] <= clip[1] {
                clip = [0.0; 4];
            }
            Some(clip)
        } else {
            self.scissor
        };
        GameGuiCanvas {
            batch: &mut *self.batch,
            pose,
            scissor: clip,
            ..*self
        }
        .group(draw)
    }

    fn fill(&mut self, rect @ [x, y, ..]: [f32; 4], argb: u32) -> Result<(), GuiDrawError> {
        let [x1, y1] = rect_end(rect)?;
        let (left, right) = (x.min(x1), x.max(x1));
        let (top, bottom) = (y.min(y1), y.max(y1));
        self.paint(|batch, atlas| {
            batch.fill_quad(
                atlas,
                [[left, top], [right, top], [right, bottom], [left, bottom]],
                abgr(argb),
            );
        })
    }

    fn fill_gradient(&mut self, rect: [f32; 4], top: u32, bottom: u32) -> Result<(), GuiDrawError> {
        rect_end(rect)?;
        self.paint(|batch, atlas| paint_fill_gradient(batch, atlas, rect, top, bottom))
    }

    fn triangles(&mut self, triangles: &[[GuiVertex; 3]]) -> Result<(), GuiDrawError> {
        // `paint` refuses a corner that is not finite once posed, the identity pose included.
        self.paint(|batch, atlas| {
            for triangle in triangles {
                batch.fill_triangle_shaded(
                    atlas,
                    triangle.map(|corner| corner.pos),
                    triangle.map(|corner| abgr(corner.argb)),
                );
            }
        })
    }

    fn text(
        &mut self,
        text: &Text,
        at @ [x, y]: [f32; 2],
        argb: u32,
        shadow: bool,
    ) -> Result<(), GuiDrawError> {
        if !at.into_iter().all(f32::is_finite) {
            return Err(GuiDrawError::NotFinite);
        }
        let text = text.try_into_engine(&()).map_err(GuiDrawError::Text)?;
        // GuiGraphicsExtractor.text skips extraction when the default alpha is zero.
        if argb >> 24 == 0 {
            return Ok(());
        }
        self.paint(|batch, atlas| {
            batch.text_styled(atlas, &text, x, y, abgr(argb), shadow);
        })
    }

    fn text_with_word_wrap(
        &mut self,
        text: &Text,
        at: [f32; 2],
        spec: TextLayoutSpec,
        argb: u32,
        shadow: bool,
    ) -> Result<TextLayoutSize, GuiDrawError> {
        let layout = LabelLayout::try_from(spec).map_err(GuiDrawError::TextLayout)?;
        if !at.into_iter().all(f32::is_finite) {
            return Err(GuiDrawError::NotFinite);
        }
        let mut pose = self.pose;
        pose_apply(&mut pose, PoseOp::Translate(at[0], at[1]));
        pose_apply(&mut pose, PoseOp::Scale(spec.scale, spec.scale));
        if !pose.into_iter().all(f32::is_finite) {
            return Err(GuiDrawError::NotFinite);
        }
        let text = text.try_into_engine(&()).map_err(GuiDrawError::Text)?;
        self.paint_in_pose(pose, |batch, atlas| {
            layout
                .visit_visual(&atlas.font, &text, |line, [x, y]| {
                    if argb >> 24 != 0 {
                        batch.text_visual(atlas, line, x, y, abgr(argb), shadow);
                    }
                })
                .map_err(GuiDrawError::TextLayout)
        })
    }

    fn item(&mut self, stack: &ItemStack, at: [f32; 2], seed: i32) -> Result<(), GuiDrawError> {
        let Some(stack) = drawable_stack(stack)? else {
            return Ok(());
        };
        self.check_item_geometry(at)?;
        let game = self.game.ok_or(GuiDrawError::Unavailable)?;
        let head_skins = self.head_skins;
        head_skins.scope(|| {
            self.paint(|batch, _| {
                game.draw_item_model_with_skins(batch, &stack, at, seed, Some(head_skins));
            })
        })
    }

    fn item_tinted(
        &mut self,
        stack: &ItemStack,
        at: [f32; 2],
        seed: i32,
        argb: u32,
    ) -> Result<(), GuiDrawError> {
        if argb >> 24 == 0 {
            return Ok(());
        }
        let start = self.batch.vertices.len();
        self.item(stack, at, seed)?;
        let tint = abgr(argb);
        if tint != u32::MAX {
            tint_vertices(self.batch, start, tint);
        }
        Ok(())
    }

    fn item_decorations(
        &mut self,
        stack: &ItemStack,
        at @ [x, y]: [f32; 2],
        count: Option<&str>,
    ) -> Result<(), GuiDrawError> {
        let Some(stack) = drawable_stack(stack)? else {
            return Ok(());
        };
        self.check_item_geometry(at)?;
        let game = self.game.ok_or(GuiDrawError::Unavailable)?;
        self.paint(|batch, _| {
            game.draw_vanilla_item_decorations(batch, &stack, x, y, count);
        })?;
        // Its draws already use this pose and scissor. Only the vanilla suffix above needs
        // transforming, not the whole decorator call.
        if !self.decorating {
            game.native_item_decorations(
                &stack,
                x,
                y,
                &mut GameGuiCanvas {
                    batch: &mut *self.batch,
                    decorating: true,
                    ..*self
                },
            );
        }
        Ok(())
    }

    fn blit_sprite(
        &mut self,
        name: &str,
        dst: [f32; 4],
        region: Option<GuiSpriteRegion>,
        argb: u32,
        blend: Blend,
    ) -> Result<(), GuiDrawError> {
        rect_end(dst)?;
        if matches!(blend, Blend::Vignette | Blend::Nausea) {
            return Err(GuiDrawError::UnsupportedSpriteBlend);
        }
        let region = if let Some(GuiSpriteRegion {
            size: [w, h],
            offset: [u, v],
        }) = region
        {
            if w <= 0 || h <= 0 {
                return Err(GuiDrawError::BadSpriteRegion);
            }
            Some([w, h, u, v])
        } else {
            None
        };
        let Some(GuiSprite::Present { rect, .. }) = self.atlas.gui_lookup(name) else {
            return Err(GuiDrawError::UnknownSprite);
        };
        let quad = ResolvedGuiQuad {
            rect,
            dst,
            color: argb,
        };
        self.paint(|batch, atlas| quad.paint_sprite(batch, atlas, name, region, blend))
    }

    fn atlas_sprite(&mut self, name: &str, dst: [f32; 4], argb: u32) -> Result<(), GuiDrawError> {
        rect_end(dst)?;
        let rect = *self
            .atlas
            .sprites
            .get(name)
            .ok_or(GuiDrawError::UnknownSprite)?;
        let quad = ResolvedGuiQuad {
            rect,
            dst,
            color: argb,
        };
        self.paint(|batch, _| quad.paint(batch))
    }

    fn blit(
        &mut self,
        texture: &str,
        dst: [f32; 4],
        GuiBlitRegion { uv, src, tex }: GuiBlitRegion,
        argb: u32,
        blend: Blend,
    ) -> Result<(), GuiDrawError> {
        check_blit_region(dst, uv, tex)?;
        let rect = *self
            .atlas
            .texture_rect(texture)
            .ok_or(GuiDrawError::UnknownTexture)?;
        let quad = ResolvedGuiQuad {
            rect,
            dst,
            color: argb,
        };
        self.paint(|batch, atlas| {
            quad.paint_blit(
                batch,
                atlas.texture_atlas_page(texture),
                uv,
                src,
                tex,
                blend,
            )
        })
    }

    fn texture(
        &mut self,
        id: &mod_api::Identifier,
        dst: [f32; 4],
        GuiBlitRegion { uv, src, tex }: GuiBlitRegion,
        argb: u32,
    ) -> Result<(), GuiDrawError> {
        check_blit_region(dst, uv, tex)?;
        // A live dynamic texture first, then the resource stack, as one id space; an id in
        // neither is refused here, and only one released after this call draws the missing texture.
        let dynamic = self
            .game
            .and_then(|game| game.client.dynamic_textures.get(id));
        if dynamic.is_none() && !self.atlas.has_texture(id) {
            return Err(GuiDrawError::UnknownTexture);
        }
        self.paint(|batch, atlas| match dynamic {
            Some((handle, size)) => {
                batch.dynamic_texture(handle, size, dst, uv, [src, tex], abgr(argb))
            }
            None => batch.texture(atlas, id, dst, uv, [src, tex], abgr(argb)),
        })
    }

    fn player_face(
        &mut self,
        uuid: u128,
        at: [f32; 2],
        size: f32,
        hat: bool,
        flip: bool,
        argb: u32,
    ) -> Result<(), GuiDrawError> {
        let dst = [at[0], at[1], size, size];
        rect_end(dst)?;
        let game = self.game.ok_or(GuiDrawError::Unavailable)?;
        let head_skins = self.head_skins;
        head_skins.scope(|| {
            let mut skin_name = String::new();
            let skin = game.player_face_skin_with_journal(uuid, Some(head_skins), &mut skin_name);
            self.paint(|batch, atlas| {
                paint_player_face(batch, atlas, skin, dst, hat, flip, argb);
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use sunrise_assets::resource_source::ResourceSource;
    use sunrise_client::hud::{Blend as HudBlend, Rect};

    /// The multiply rounds to the nearest byte for every pair of channels, and white is neutral.
    #[test]
    fn multiplying_packed_colours_rounds_each_channel() {
        for a in 0..=255u32 {
            for b in 0..=255u32 {
                let want = ((a * b) as f64 / 255.0).round() as u32;
                assert_eq!(multiply_abgr(a, b), want, "{a} * {b}");
                assert_eq!(multiply_abgr(a << 24, b << 24), want << 24, "{a} * {b}");
            }
        }
        assert_eq!(multiply_abgr(0x8040_20FF, u32::MAX), 0x8040_20FF);
        assert_eq!(multiply_abgr(0xFFFF_FFFF, 0x80FF_0000), 0x80FF_0000);
    }

    /// A CIT glint layer's vertices keep their slot (red) and take only the tint's alpha on their
    /// intensity; every other vertex is multiplied by the whole tint.
    #[test]
    fn a_tint_keeps_a_cit_glint_layers_slot() {
        let mut batch = HudBatch::scratch([320, 240]);
        let quad = |batch: &mut HudBatch, color: u32| {
            batch.quad_glint(
                [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                [[0.0; 2]; 4],
                [[0.0; 2]; 4],
                color,
            );
        };
        // Drawn before the item: not tinted.
        quad(&mut batch, 0xFF00_00FF);
        let start = batch.vertices.len();
        quad(&mut batch, u32::MAX);
        let cit = batch.vertices.len();
        // Slot 7 at intensity 200: red 7, alpha 200 (packed ABGR).
        quad(&mut batch, 0xC800_0007);
        batch.blend_since(cit, HudBlend::CitGlint(0));
        let tint = abgr(0x80FF_4000);
        tint_vertices(&mut batch, start, tint);
        let colors: Vec<u32> = batch.vertices.iter().map(|v| v.color).collect();
        assert!(colors[..start].iter().all(|&c| c == 0xFF00_00FF));
        assert!(colors[start..cit].iter().all(|&c| c == tint));
        // 200 * 128 / 255 rounds to 100; the slot stays 7.
        assert!(
            colors[cit..].iter().all(|&c| c == 0x6400_0007),
            "{colors:x?}"
        );
    }

    /// `item_tinted` multiplies every vertex `item` draws, the enchantment glint's included (the
    /// glint shader multiplies by its white vertex colour): opaque white draws `item` itself and
    /// a zero alpha draws nothing.
    #[test]
    #[ignore = "daily"]
    fn a_tinted_item_multiplies_the_vertices_the_item_draws() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            // The glint options a frame captures (full strength), as a frame drawing items has.
            game.client.glint_frame = sunrise_client::item_render::GlintFrame {
                millis: 0,
                speed: 1.0,
                alpha: 1.0,
                sprite: game
                    .client
                    .gui
                    .atlas
                    .sprites
                    .contains_key("misc/enchanted_glint_item"),
            };
            assert!(
                game.client.glint_frame.sprite,
                "the glint's sprite is in the atlas"
            );
            // A plain item, and one that always has the glint.
            for (item, foil) in [
                ("minecraft:diamond_sword", false),
                ("minecraft:enchanted_golden_apple", true),
            ] {
                let stack = mod_api::ItemStack::new(mod_api::__private::item_key(item).unwrap(), 1)
                    .unwrap();
                let draw = |argb: Option<u32>| {
                    let mut batch = HudBatch::scratch([320, 240]);
                    GameGuiCanvas::draw(
                        &mut batch,
                        &game.client.gui.atlas,
                        Some(&*game),
                        None,
                        &mut |root| match argb {
                            None => root.item(&stack, [4.0, 4.0], 0),
                            Some(argb) => root.item_tinted(&stack, [4.0, 4.0], 0, argb),
                        },
                    )
                    .unwrap();
                    batch
                        .vertices
                        .iter()
                        .map(|v| (v.pos, v.uv, v.color))
                        .collect::<Vec<_>>()
                };
                let plain = draw(None);
                assert!(!plain.is_empty());
                assert_eq!(draw(Some(0xFFFF_FFFF)), plain);
                assert!(draw(Some(0x00FF_FFFF)).is_empty());
                let tint = 0x80FF_4000;
                let tinted = draw(Some(tint));
                assert_eq!(tinted.len(), plain.len());
                for ((pos, uv, color), (plain_pos, plain_uv, plain_color)) in
                    tinted.iter().zip(&plain)
                {
                    assert_eq!((pos, uv), (plain_pos, plain_uv));
                    assert_eq!(*color, multiply_abgr(*plain_color, abgr(tint)));
                }
                // The glint's quads (atlas U at 2 or more, the shader's glint marker) are drawn white,
                // so tinted they carry the tint itself, which the glint shader multiplies by.
                let glint: Vec<u32> = tinted
                    .iter()
                    .filter(|(_, uv, _)| uv[0] >= 2.0)
                    .map(|&(_, _, color)| color)
                    .collect();
                assert_eq!(!glint.is_empty(), foil, "{item}");
                assert!(glint.iter().all(|&color| color == abgr(tint)), "{item}");
            }
        });
    }

    /// `blit` and `texture` refuse a bad region the same way and before their lookup (#5994),
    /// and `blit` refuses a name outside the atlas, which `texture` is for, as documented.
    #[test]
    fn blit_and_texture_share_their_region_checks() {
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let id = mod_api::Identifier::parse("test:dynamic/panel").unwrap();
        let region = |uv, tex| GuiBlitRegion {
            uv,
            src: [4, 4],
            tex,
        };
        let cases = [
            (
                [0.0, 0.0, f32::NAN, 4.0],
                region([0.0, 0.0], [4, 4]),
                GuiDrawError::NotFinite,
            ),
            (
                [0.0; 4],
                region([f32::INFINITY, 0.0], [4, 4]),
                GuiDrawError::NotFinite,
            ),
            (
                [0.0; 4],
                region([0.0, 0.0], [4, 0]),
                GuiDrawError::BadBlitRegion,
            ),
            (
                [0.0; 4],
                region([0.0, 0.0], [4, 4]),
                GuiDrawError::UnknownTexture,
            ),
        ];
        for (dst, region, error) in cases {
            let mut batch = HudBatch::scratch([320, 240]);
            let blit = GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
                root.blit("test:dynamic/panel", dst, region, u32::MAX, Blend::Textured)
            });
            assert_eq!(blit, Err(error.clone()), "blit {dst:?}");
            let texture = GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
                root.texture(&id, dst, region, u32::MAX)
            });
            assert_eq!(texture, Err(error), "texture {dst:?}");
        }
    }

    #[test]
    fn gui_groups_clip_posed_leaves_and_restore_vertices_and_blends_on_abort() {
        let mut atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        atlas.sprites.insert(
            "test:canvas".into(),
            Rect {
                u0: 0.25,
                v0: 0.125,
                u1: 0.75,
                v1: 0.625,
                w: 4,
                h: 4,
            },
        );
        let crosshair = |canvas: &mut dyn GuiCanvas, dst| {
            canvas.blit(
                "test:canvas",
                dst,
                GuiBlitRegion {
                    uv: [0.0, 0.0],
                    src: [4, 4],
                    tex: [4, 4],
                },
                0x8011_2233,
                Blend::Crosshair,
            )
        };
        let mut batch = HudBatch::scratch([320, 240]);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            crosshair(root, [2.0, 3.0, 2.0, 2.0])
        })
        .unwrap();

        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            assert_eq!(root.size(), [320.0, 240.0]);
            root.scope(
                &[PoseOp::Translate(10.75, 20.25)],
                Some([0.0, 0.0, 5.75, 5.75]),
                &mut |parent| {
                    parent.scope(
                        &[PoseOp::Translate(1.0, 1.0), PoseOp::Scale(2.0, 3.0)],
                        Some([0.25, 0.25, 3.0, 3.0]),
                        &mut |child| crosshair(child, [0.0, 0.0, 4.0, 4.0]),
                    )?;
                    for angle in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                        assert_eq!(
                            parent.scope(
                                &[PoseOp::Translate(2.0, 0.0), PoseOp::Rotate(angle)],
                                None,
                                &mut |_| panic!("a refused rotation must not invoke the callback"),
                            ),
                            Err(GuiDrawError::NotFinite),
                        );
                    }
                    assert_eq!(
                        parent.scope(
                            &[PoseOp::Scale(f32::MAX, 1.0), PoseOp::Scale(2.0, 1.0)],
                            None,
                            &mut |_| {
                                panic!("a scope with a non-finite composed pose must not run")
                            }
                        ),
                        Err(GuiDrawError::NotFinite)
                    );
                    assert_eq!(
                        parent.scope(&[], Some([f32::MAX, 0.0, f32::MAX, 1.0]), &mut |_| {
                            panic!("a scope with a non-finite clip edge must not run")
                        }),
                        Err(GuiDrawError::NotFinite)
                    );

                    // Both discarded scopes append with the existing blend, extending its run.
                    assert_eq!(
                        parent.scope(&[], None, &mut |child| {
                            crosshair(child, [0.0, 0.0, 1.0, 1.0])?;
                            Err(GuiDrawError::Abort)
                        }),
                        Err(GuiDrawError::Abort)
                    );
                    let unwind = catch_unwind(AssertUnwindSafe(|| {
                        parent.scope(&[PoseOp::Translate(1.0, 0.0)], None, &mut |child| {
                            crosshair(child, [0.0, 0.0, 1.0, 1.0])?;
                            panic!("discard on unwind")
                        })
                    }));
                    assert!(unwind.is_err());

                    // Catch a leaf refusal instead of discarding its entire scope.
                    parent.scope(&[PoseOp::Scale(2.0, 1.0)], None, &mut |child| {
                        assert_eq!(
                            crosshair(child, [f32::MAX, 0.0, 1.0, 1.0]),
                            Err(GuiDrawError::NotFinite)
                        );
                        Ok(())
                    })?;
                    parent.fill_gradient([2.0, 2.0, 1.0, 1.0], 0x80AA_1234, 0xFF00_0000)
                },
            )?;
            root.fill([30.0, 40.0, 2.0, 2.0], 0xFFFF_FFFF)
        })
        .unwrap();
        assert_eq!(batch.vertices.len(), 24);
        assert_eq!(batch.vertices[0].pos, [2.0, 3.0]);
        assert_eq!(batch.vertices[6].pos, [11.75, 21.25]);
        assert_eq!(batch.vertices[8].pos, [19.75, 33.25]);
        assert_eq!(batch.vertices[6].uv, [0.25, 0.125]);
        assert_eq!(batch.vertices[6].color, 0x8033_2211);
        assert_eq!(batch.vertices[12].pos, [12.75, 22.25]);
        assert_eq!(batch.vertices[12].color, 0x8034_12AA);
        assert_eq!(batch.vertices[14].color, 0xFF00_0000);
        assert_eq!(batch.vertices[18].pos, [30.0, 40.0]);
        assert_eq!(batch.blends, [(0..12, HudBlend::Invert)]);

        let scissors = [
            (0..6, None),
            (6..12, Some([12.0, 22.0, 15.0, 25.0])),
            (12..18, Some([10.0, 20.0, 15.0, 25.0])),
            (18..24, None),
        ];
        assert_eq!(batch.scissors_in(0..24).collect::<Vec<_>>(), scissors);
        let before = bytemuck::cast_slice::<_, u8>(&batch.vertices).to_vec();
        assert_eq!(
            GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
                root.scope(&[], Some([4.0, 7.0, 1.0, 1.0]), &mut |child| {
                    crosshair(child, [4.0, 7.0, 1.0, 1.0])
                })?;
                Err(GuiDrawError::Abort)
            }),
            Err(GuiDrawError::Abort)
        );
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&batch.vertices),
            before.as_slice()
        );
        assert_eq!(batch.blends, [(0..12, HudBlend::Invert)]);
        assert_eq!(batch.scissors_in(0..24).collect::<Vec<_>>(), scissors);
    }

    #[test]
    fn a_refused_blit_leaves_the_batch_as_it_was_before_the_group() {
        const DST: [f32; 4] = [10.0, 20.0, 4.0, 4.0];
        const WHITE: u32 = u32::MAX;
        const REGION: GuiBlitRegion = GuiBlitRegion {
            uv: [0.0, 0.0],
            src: [4, 4],
            tex: [4, 4],
        };
        let mut atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let rect = Rect {
            u0: 0.25,
            v0: 0.5,
            u1: 0.5,
            v1: 0.75,
            w: 4,
            h: 4,
        };
        atlas.gui.insert(
            "test:present".into(),
            GuiSprite::Present {
                rect,
                scaling: None,
            },
        );
        atlas.gui.insert("test:removed".into(), GuiSprite::Missing);
        atlas.sprites.insert("test:removed".into(), rect);
        atlas.sprites.insert("test:live".into(), rect);

        let mut batch = HudBatch::scratch([320, 240]);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            root.atlas_sprite("test:removed", DST, WHITE)?;
            root.blit_sprite("test:present", DST, None, WHITE, Blend::Crosshair)
        })
        .unwrap();
        assert_eq!(batch.vertices.len(), 12);
        assert_eq!(batch.vertices[0].uv, [0.25, 0.5]);
        assert_eq!(batch.vertices[6].uv, [0.25, 0.5]);
        assert_eq!(batch.blends, [(6..12, HudBlend::Invert)]);
        let before = bytemuck::cast_slice::<_, u8>(&batch.vertices).to_vec();

        let mut refuses = |draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
                           expected| {
            assert_eq!(
                GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
                    root.scope(&[PoseOp::Translate(2.0, 0.0)], None, &mut |child| {
                        child.blit("test:live", DST, REGION, WHITE, Blend::Crosshair)
                    })?;
                    draw(root)
                }),
                Err(expected)
            );
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&batch.vertices),
                before.as_slice()
            );
            assert_eq!(batch.blends, [(6..12, HudBlend::Invert)]);
        };
        for name in [
            "test:unknown",
            "test:removed",
            "mob_effect/test_missing",
            "test:mob_effect/missing",
        ] {
            refuses(
                &mut |gui| gui.blit_sprite(name, DST, None, WHITE, Blend::Textured),
                GuiDrawError::UnknownSprite,
            );
        }
        refuses(
            &mut |gui| gui.atlas_sprite("test:unknown", DST, WHITE),
            GuiDrawError::UnknownSprite,
        );
        for blend in [Blend::Nausea, Blend::Vignette] {
            refuses(
                &mut |gui| gui.blit_sprite("test:present", DST, None, WHITE, blend),
                GuiDrawError::UnsupportedSpriteBlend,
            );
        }
        for (size, blend) in [
            ([0, 4], Blend::Textured),
            ([4, 0], Blend::Textured),
            ([-1, 4], Blend::Crosshair),
        ] {
            let region = Some(GuiSpriteRegion {
                size,
                offset: [0; 2],
            });
            refuses(
                &mut |gui| gui.blit_sprite("test:present", DST, region, WHITE, blend),
                GuiDrawError::BadSpriteRegion,
            );
        }
        refuses(
            &mut |gui| gui.blit("test:unknown", DST, REGION, WHITE, Blend::Vignette),
            GuiDrawError::UnknownTexture,
        );
        let standalone = mod_api::Identifier::parse("test:textures/gui/standalone.png").unwrap();
        refuses(
            &mut |gui| gui.texture(&standalone, DST, REGION, WHITE),
            GuiDrawError::UnknownTexture,
        );
        for (region, error) in [
            (
                GuiBlitRegion {
                    tex: [0, 4],
                    ..REGION
                },
                GuiDrawError::BadBlitRegion,
            ),
            (
                GuiBlitRegion {
                    uv: [f32::NAN, 0.0],
                    ..REGION
                },
                GuiDrawError::NotFinite,
            ),
        ] {
            refuses(
                &mut |gui| gui.blit("test:live", DST, region, WHITE, Blend::Textured),
                error.clone(),
            );
            refuses(
                &mut |gui| gui.texture(&standalone, DST, region, WHITE),
                error,
            );
        }
    }

    #[test]
    fn raw_blit_negative_source_extent_mirrors_the_texture_region() {
        let mut atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        atlas.sprites.insert(
            "test:mirror".into(),
            Rect {
                u0: 0.25,
                v0: 0.125,
                u1: 0.75,
                v1: 0.625,
                w: 8,
                h: 8,
            },
        );
        let mut batch = HudBatch::scratch([320, 240]);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |canvas| {
            canvas.blit(
                "test:mirror",
                [10.0, 20.0, 4.0, 4.0],
                GuiBlitRegion {
                    uv: [4.0, 0.0],
                    src: [-4, 4],
                    tex: [8, 8],
                },
                u32::MAX,
                Blend::Textured,
            )
        })
        .unwrap();
        assert_eq!(
            batch
                .vertices
                .iter()
                .map(|v| (v.pos, v.uv))
                .collect::<Vec<_>>(),
            [
                ([10.0, 20.0], [0.50, 0.125]),
                ([14.0, 20.0], [0.25, 0.125]),
                ([14.0, 24.0], [0.25, 0.375]),
                ([10.0, 20.0], [0.50, 0.125]),
                ([14.0, 24.0], [0.25, 0.375]),
                ([10.0, 24.0], [0.50, 0.375]),
            ]
        );
    }

    #[test]
    fn scoped_text_keeps_its_style_and_a_late_conversion_failure_discards_it() {
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let mut styled = Text::literal("A");
        styled.style.color = Some("#112233".into());
        styled.style.underlined = Some(true);
        let mut invalid = Text::literal("Not representable");
        invalid.metadata =
            mod_api::Tag::compound([("test:nonfinite", mod_api::Tag::Double(f64::NAN.to_bits()))]);
        let at = [10.0, 20.0];
        let mut batch = HudBatch::scratch([320, 240]);
        assert_eq!(
            GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
                root.text(&styled, at, u32::MAX, false)?;
                root.text(&invalid, at, u32::MAX, false)
            }),
            Err(GuiDrawError::Text(mod_api::text::TextFormError::NonFinite))
        );
        assert!(batch.vertices.is_empty());
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            root.text(&styled, at, u32::MAX, false)
        })
        .unwrap();
        assert!(!batch.vertices.is_empty());
        assert!(
            batch
                .vertices
                .iter()
                .all(|vertex| vertex.color == 0xFF33_2211)
        );
    }

    #[test]
    fn wrapped_text_refuses_nonfinite_scaled_geometry_without_visiting_or_painting() {
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        for (label, argb, scale) in [
            ("\n", u32::MAX, f32::MAX),
            ("A\nA", 0, f32::MAX / 16.0),
            ("A\nA", u32::MAX, f32::MAX / 16.0),
        ] {
            let text = Text::literal(label);
            let spec = TextLayoutSpec {
                max_width: 100.0,
                align: mod_api::TextAlign::Left,
                scale,
            };
            let layout = LabelLayout::try_from(spec).unwrap();
            assert_eq!(
                layout.visit_components(&atlas.font, &text, &mut |_| {
                    panic!("an invalid label must not publish even its first line")
                }),
                Err(mod_api::TextLayoutError::GeometryNotFinite)
            );

            let mut batch = HudBatch::scratch([320, 240]);
            GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
                assert_eq!(
                    root.text_with_word_wrap(&text, [0.0; 2], spec, argb, false),
                    Err(GuiDrawError::TextLayout(
                        mod_api::TextLayoutError::GeometryNotFinite
                    ))
                );
                root.fill([1.0, 2.0, 3.0, 4.0], u32::MAX)
            })
            .unwrap();
            assert_eq!(batch.vertices.len(), 6);
            assert_eq!(batch.vertices[0].pos, [1.0, 2.0]);
            assert!(batch.patches.is_empty());
        }
    }

    #[test]
    fn transparent_styled_text_does_not_extract_inline_profiles_or_glyphs() {
        use mod_api::text::{TextContent, TextObject, TextProfile};
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let mut profile = TextProfile::default();
        profile.name = Some("CanvasHead".into());
        let mut text = Text::new(TextContent::Object(TextObject::Player {
            profile,
            hat: true,
            fallback: None,
        }));
        text.children.push(Text::literal("Visible only with alpha"));
        let mut batch = HudBatch::scratch([320, 240]);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            root.text(&text, [10.0, 20.0], 0x00FF_FFFF, false)
        })
        .unwrap();
        assert!(batch.vertices.is_empty());
        assert!(batch.inline_profiles.is_empty());
        assert!(batch.patches.is_empty());
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            root.text(&text, [10.0, 20.0], u32::MAX, false)
        })
        .unwrap();
        assert!(!batch.vertices.is_empty());
        assert_eq!(batch.inline_profiles.len(), 1);
    }

    #[test]
    fn gui_scissors_are_empty_on_reflection_inversion_and_disjoint_intersection() {
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let mut batch = HudBatch::scratch([320, 240]);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            let mut over_scissor =
                |canvas: &mut dyn GuiCanvas| canvas.fill([-32.0, -32.0, 64.0, 64.0], 0xFFFF_FFFF);
            root.scope(
                &[PoseOp::Scale(-1.0, 1.0)],
                Some([1.0, 1.0, 4.0, 4.0]),
                &mut over_scissor,
            )?;
            root.scope(
                &[PoseOp::Scale(-1.0, 1.0)],
                Some([5.0, 1.0, -4.0, 4.0]),
                &mut over_scissor,
            )?;
            root.scope(&[], Some([0.0, 0.0, 4.0, 4.0]), &mut |parent| {
                parent.scope(&[], Some([6.0, 6.0, 4.0, 4.0]), &mut over_scissor)
            })?;
            // A singular, finite pose is allowed. A scissor with zero transformed width is empty.
            root.scope(
                &[PoseOp::Scale(0.0, 1.0)],
                Some([0.0, 0.0, 4.0, 4.0]),
                &mut over_scissor,
            )
        })
        .unwrap();
        assert_eq!(batch.vertices.len(), 24);
        assert_eq!(
            batch.scissors_in(0..24).collect::<Vec<_>>(),
            [(0..24, Some([0.0; 4]))]
        );
        assert!(batch.blends.is_empty());
    }

    #[test]
    fn player_face_rejects_non_finite_geometry_and_missing_game() {
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let mut batch = HudBatch::scratch([320, 240]);
        let uuid = 0x853c_80ef_3c37_49fd_aa49_938b_674a_dae6;
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            assert_eq!(
                root.player_face(uuid, [f32::NAN, 0.0], 8.0, true, false, 0xFFFF_FFFF),
                Err(GuiDrawError::NotFinite)
            );
            assert_eq!(
                root.player_face(uuid, [f32::MAX, 0.0], f32::MAX, true, false, 0xFFFF_FFFF),
                Err(GuiDrawError::NotFinite)
            );
            assert_eq!(
                root.player_face(uuid, [4.0, 4.0], 8.0, true, false, 0xFFFF_FFFF),
                Err(GuiDrawError::Unavailable)
            );
            Ok(())
        })
        .unwrap();
        assert!(batch.vertices.is_empty());
    }

    #[test]
    fn triangles_are_posed_with_their_corner_colours_and_a_non_finite_corner_draws_none() {
        let atlas = HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ))
        .unwrap();
        let mut batch = HudBatch::scratch([320, 240]);
        let corner = |pos, argb| GuiVertex { pos, argb };
        let white = |pos| corner(pos, 0xFFFF_FFFF);
        GameGuiCanvas::draw(&mut batch, &atlas, None, None, &mut |root| {
            root.scope(
                &[PoseOp::Translate(10.0, 20.0), PoseOp::Scale(2.0, 2.0)],
                None,
                &mut |panel| {
                    panel.triangles(&[[
                        corner([0.0, 0.0], 0xFF11_2233),
                        corner([4.0, 0.0], 0x8044_5566),
                        corner([0.0, 3.0], 0x0077_8899),
                    ]])
                },
            )?;
            // One bad corner refuses the whole call, the finite triangle before it included.
            assert_eq!(
                root.triangles(&[
                    [white([0.0, 0.0]), white([1.0, 0.0]), white([0.0, 1.0])],
                    [white([f32::NAN, 0.0]), white([1.0, 0.0]), white([0.0, 1.0])],
                ]),
                Err(GuiDrawError::NotFinite)
            );
            assert_eq!(
                root.scope(&[PoseOp::Scale(f32::MAX, 1.0)], None, &mut |wide| {
                    wide.triangles(&[[white([0.0, 0.0]), white([2.0, 0.0]), white([0.0, 1.0])]])
                }),
                Err(GuiDrawError::NotFinite)
            );
            Ok(())
        })
        .unwrap();
        let corners = batch.vertices.iter().map(|v| (v.pos, v.color));
        assert_eq!(
            corners.collect::<Vec<_>>(),
            [
                ([10.0, 20.0], 0xFF33_2211),
                ([18.0, 20.0], 0x8066_5544),
                ([10.0, 26.0], 0x0099_8877),
                ([10.0, 20.0], 0xFF33_2211),
                ([10.0, 26.0], 0x0099_8877),
                ([10.0, 26.0], 0x0099_8877),
            ]
        );
    }
}
