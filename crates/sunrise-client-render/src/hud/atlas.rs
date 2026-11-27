use super::*;

/// Border padding policy recorded for a packed sprite at atlas build time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasBorder {
    /// No border padding allocated around the sprite; uploads must stay inside `(x, y, w, h)`.
    None,
    /// 1-texel clamp-to-edge padding (`TextureAtlas` / `SpriteContents.AnimationState.drawToAtlas`).
    Clamp,
    /// 1-texel wrap-edge padding for repeating blurred sprites.
    Wrap,
}

/// Exact integer placement and border policy of an animated sprite inside `HudAtlas`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimatedPlacement {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub border: AtlasBorder,
}

pub struct HudAtlas {
    pub(super) pictures: std::sync::Mutex<pictures::Cache>,
    /// Identity of this packed atlas, used to invalidate cached model UV data
    /// even when a resource reload places sprites at the same coordinates.
    pub revision: u64,
    /// RGBA8, `width` by `height`, as built: empty once `pack_pixels` has
    /// compressed them (read them through `pixels()`).
    pub pixels: Vec<u8>,
    /// `pixels` compressed (lz4) on a thread of its own once the renderer
    /// has uploaded them: only rebuilding a renderer reads them again.
    pub(super) packed: Option<Arc<OnceLock<Vec<u8>>>>,
    /// The width each Standard Galactic letter uses in `font/ascii_sga`'s
    /// cell, by ASCII code (the enchanting table's text), read when built.
    pub galactic_widths: [u16; 128],
    pub width: u32,
    pub height: u32,
    pub sprites: HashMap<String, Rect>,
    /// The texel origin of each `sunrise/skin{n}` slot, by `n`: where a skin is written. Empty
    /// on an atlas without the slots.
    pub skin_origins: Vec<[u32; 2]>,
    /// The GUI atlas's own sprites by GUI id, each with an entry of its own: its rect, or none for
    /// a sprite its list removed or failed ([`HudAtlas::gui_lookup`]).
    pub gui: HashMap<String, GuiSprite>,
    /// The packed names no GUI lookup of an id the GUI list lacks answers with: the model atlases'
    /// sprites and the walked files a GUI entry stands on.
    pub non_gui: HashSet<String>,
    /// A registered atlas's sprites by their GUI names (`mymod:gui/atlases/icons/mymod/heart`),
    /// with UVs local to their atlas texture, distinct from model or vanilla GUI sprites.
    pub registered: HashMap<String, Rect>,
    pub(super) registered_owners: HashMap<String, usize>,
    pub(crate) registered_pages: Vec<RegisteredAtlasPage>,
    /// The dimension this generation was stitched against; absent on font-only or boot atlases.
    pub stitch_limit: Option<u32>,
    /// Resolved player-profile names and UUIDs for inline head components.
    pub object_skins: HashMap<String, String>,
    pub object_skins_revision: u64,
    /// Sprites with a 0 < alpha < 255 texel (`SpriteContents.transparency().hasTranslucent`),
    /// which puts their particles on vanilla's translucent layer.
    pub translucent: HashSet<String>,
    /// Sprites whose `.png.mcmeta` sets `blur` or `clamp`; the others
    /// sample as vanilla's defaults (nearest, repeat).
    pub flags: HashMap<String, TextureFlags>,
    /// Every animated block or item sprite the atlas packs: its atlas rect
    /// and the mcmeta-driven frame list, re-uploaded into the rect as the
    /// frames turn (vanilla's stitched atlas animates all of these).
    pub sprite_animations: Vec<(Rect, crate::sprites::SpriteAnimation)>,
    /// Exact integer placement and border policy for each entry in `sprite_animations`.
    pub sprite_animation_placements: Vec<AnimatedPlacement>,
    /// The default font (bitmap sheets in this atlas, unifont rasterized on demand).
    pub font: crate::font::Font,
    /// `misc/enchanted_glint_item` as its own image: vanilla's `GlintSampler`
    /// repeats it per pixel, which one sprite inside this atlas cannot.
    pub glint: (Vec<u8>, u32, u32),
    /// `misc/enchanted_glint_armor`, the armour layers' `GlintSampler`.
    pub glint_armor: (Vec<u8>, u32, u32),
    pub text_shaders: crate::gfx::core_text::TextShaders,
    /// A resource pack's own `core/rendertype_lines` shader for the block outline, translated;
    /// none draws the outline natively.
    pub lines_shader: Option<std::sync::Arc<sunrise_glsl::Translated>>,
    /// Its Polytone expression uniforms (`shader_modifiers`), resolved when it loaded.
    pub lines_expressions: Vec<crate::polytone_post::ExpressionUniform>,
    /// The packs' `core/sky` and `core/stars`, when a pack replaces them.
    pub sky_shader: Option<std::sync::Arc<sunrise_glsl::Translated>>,
    pub stars_shader: Option<std::sync::Arc<sunrise_glsl::Translated>>,
    /// Their Polytone expression uniforms, resolved when they loaded.
    pub sky_expressions: Vec<crate::polytone_post::ExpressionUniform>,
    pub stars_expressions: Vec<crate::polytone_post::ExpressionUniform>,
    /// The packs' `core/lightmap` (with `core/screenquad`), when a pack replaces either stage.
    pub lightmap_shader: Option<std::sync::Arc<sunrise_glsl::Translated>>,
    /// Its Polytone expression uniforms (`shader_modifiers`), resolved when it loaded: the
    /// blocks its stages read, with their programs.
    pub lightmap_expressions: Vec<crate::polytone_post::ExpressionUniform>,
    /// The packs' `core/clouds`, when a pack replaces either stage: outside and under Improved
    /// Transparency.
    pub clouds_shader: Option<std::sync::Arc<crate::clouds::pack::Programs>>,
    /// The packs' Polytone screen sprite modifiers.
    pub sprite_modifiers: std::sync::Arc<sprite_modifiers::SpriteModifiers>,
    /// The packs' OptiFine random entity textures, whose variants this atlas packs.
    pub random_textures: sunrise_optifine::random_textures::RandomTextures,
    /// Their emissive overlays (`_e`), packed here too.
    pub emissive: std::sync::Arc<sunrise_optifine::emissive::Emissive>,
    /// ETF's blinking frames (`_blink`), packed here too.
    pub blinks: sunrise_optifine::blink::Blinks,
    /// Of the shared atlas's textures, their LabPBR normal and specular maps as paged maps
    /// (`paged_maps.rs`), read while loading for shader packs (`sprites::labpbr_maps`).
    pub material_maps: std::sync::Arc<[Option<crate::paged_maps::PagedMap>; 2]>,
    /// The white texel flat fills sample (also `hud_kit::shapes`).
    pub(crate) white: Rect,
}
/// The CPU image and animations of one registered atlas, kept across renderer rebuilds.
#[derive(Clone)]
pub(crate) struct RegisteredAtlasPage {
    pub(crate) atlas: String,
    pub(crate) mod_id: Option<&'static str>,
    pub(crate) size: [u32; 2],
    pub(crate) pixels: Vec<u8>,
    pub(crate) animations: Vec<(Rect, crate::sprites::SpriteAnimation)>,
    pub(crate) animation_placements: Vec<AnimatedPlacement>,
}

/// Atlas rows reserved for glyphs rasterized at run time.
pub const GLYPH_ROWS: u32 = 512;
/// The largest atlas side: the 2D texture limit of the GPUs Sunrise runs on
/// (Apple silicon, RADV and Intel allow 16384; NVIDIA 32768).
pub use sunrise_level::pack_image::MAX_ATLAS_SIZE;
/// 128x128 atlas slots for map textures. A lobby paints its signs as walls of
/// framed maps, so the pool holds a wall; too few slots re-upload every map
/// every frame. 26.3's `MapTextureManager` keeps a texture per map instead.
pub const MAP_SLOTS: usize = 256;
/// The atlas sprite of map slot `slot` (`sunrise/map{slot}`), named once:
/// every map shown asks for its slot's each frame.
pub fn map_slot_sprite(slot: usize) -> Option<&'static str> {
    static NAMES: std::sync::LazyLock<Vec<String>> =
        std::sync::LazyLock::new(|| (0..MAP_SLOTS).map(|i| format!("sunrise/map{i}")).collect());
    NAMES.get(slot).map(String::as_str)
}
/// Atlas slots for downloaded player skins (`skins/<sha1>`), 64x64 each.
/// Vanilla registers a texture per skin. Here the slots are shared
/// (`skins::SkinSlots`): skins drawn in the last frame keep theirs, and a
/// skin past them shows the default skin until a slot frees up.
pub const SKIN_SLOTS: usize = 128;
/// Reused by the visible rows of the classic world selector.
pub const WORLD_ICON_SLOTS: usize = 32;
/// Atlas slots for the pack icons on the pack screen, one per icon drawn
/// (`PackSelectionScreen.loadPackIcon` registers a texture per pack): two lists
/// of 36-pixel rows at GUI scale 2 on a 4K screen still fit.
pub const PACK_ICON_SLOTS: usize = 64;
/// A pack icon slot's side: vanilla's own icons are 128x128.
pub const PACK_ICON_SIZE: u32 = 128;
/// Atlas slots for armour trim textures made at run time (`trims`), each the
/// size of the largest trim texture.
pub const TRIM_SLOTS: usize = 32;

/// A texture name without vanilla's default namespace (`ResourceLocation`'s `minecraft:`), as the
/// atlas keeps its packed sprites and registered atlases' sprites.
fn default_namespace(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}

/// The prefix of the name a GUI atlas sprite is packed under when a model's sprite has its name.
pub(super) const GUI_KEY: &str = "#gui:";
/// The prefix of the name a registered atlas's sprite is packed under.
pub(super) const REGISTERED_KEY: &str = "#registered:";

/// `MissingTextureAtlasSprite`: what a sprite the atlas lacks draws as.
pub const MISSING_SPRITE: &str = "sunrise/missing";

/// `SpecialGlyphs.MISSING`: 5x8 opaque white border around a transparent interior.
pub(super) const MISSING_GLYPH_SPRITE: &str = "sunrise/missing_glyph";

pub(super) fn missing_glyph_rgba() -> Vec<u8> {
    let mut rgba = Vec::with_capacity(5 * 8 * 4);
    for y in 0..8u32 {
        for x in 0..5u32 {
            let edge = x == 0 || x == 4 || y == 0 || y == 7;
            rgba.extend_from_slice(if edge {
                &[255, 255, 255, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }
    rgba
}

/// The used width of each Standard Galactic letter (A-Z, a-z) in the
/// `font/ascii_sga` sheet: one past its last column with a texel that isn't
/// transparent.
pub(super) fn galactic_widths(
    sprites: &HashMap<String, Rect>,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> [u16; 128] {
    let mut widths = [0u16; 128];
    let Some(sheet) = sprites.get("font/ascii_sga") else {
        return widths;
    };
    let cell = sheet.w / 16;
    let (px, py) = (
        (sheet.u0 * width as f32) as u32,
        (sheet.v0 * height as f32) as u32,
    );
    for code in (0x41..=0x5Au32).chain(0x61..=0x7A) {
        let (cx, cy) = (px + code % 16 * cell, py + code / 16 * cell);
        let alpha = |x: u32, y: u32| {
            pixels
                .get(((y * width + x) * 4 + 3) as usize)
                .copied()
                .unwrap_or(0)
        };
        widths[code as usize] = (0..cell)
            .rev()
            .find(|&dx| (0..cell).any(|dy| alpha(cx + dx, cy + dy) != 0))
            .map_or(0, |dx| (dx + 1).min(u32::from(u16::MAX)) as u16);
    }
    widths
}
/// `TextureAtlas.dumpContents` (`F3 + S`, vanilla's "dynamic textures"): the atlas
/// written as `name.png` beside `name.txt`, every sprite in name order with its
/// rect in texels (`dumpSpriteNames`). Vanilla names each file after its atlas
/// with `/` and `:` as `_` (`Identifier.toDebugFileName`); Sunrise packs the GUI,
/// font, particle, map and skin sprites into one atlas, so one file stands in for
/// the atlases vanilla dumps apart.
pub fn dump_atlas(
    dir: &Path,
    name: &str,
    width: u32,
    height: u32,
    pixels: &[u8],
    sprites: &HashMap<String, Rect>,
) -> std::io::Result<PathBuf> {
    // A build whose pixels were freed (`fonts`) has the size and no texels:
    // write nothing rather than a file every reader would reject.
    if pixels.len() != (width as usize) * (height as usize) * 4 {
        return Err(std::io::Error::other(format!(
            "wrong data size, expected {} got {}",
            (width as usize) * (height as usize) * 4,
            pixels.len()
        )));
    }
    let png_path = write_dump_png(dir, name, width, height, pixels)?;
    let mut names = sprites.iter().collect::<Vec<_>>();
    names.sort_by(|a, b| a.0.cmp(b.0));
    let mut list = String::new();
    for (sprite, rect) in names {
        let (w, h) = rect.texels((width, height));
        let x = (rect.u0 * width as f32).round() as u32;
        let y = (rect.v0 * height as f32).round() as u32;
        list.push_str(&format!("{sprite}\tx={x}\ty={y}\tw={w}\th={h}\n"));
    }
    fs::write(dir.join(format!("{name}.txt")), list)?;
    Ok(png_path)
}

/// `NativeImage.writeToFile` for a dump: `pixels` (RGBA, `width * height * 4` bytes) written to
/// `dir/name.png`, `dir` made first.
pub(crate) fn write_dump_png(
    dir: &Path,
    name: &str,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let png_path = dir.join(format!("{name}.png"));
    let mut encoder = png::Encoder::new(fs::File::create(&png_path)?, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer
        .write_image_data(pixels)
        .map_err(std::io::Error::other)?;
    Ok(png_path)
}

impl HudAtlas {
    /// Builds the `HudRenderer::sprite_animations` entries from packed animations and placements.
    pub fn renderer_animations(
        &self,
    ) -> Vec<(
        AnimatedPlacement,
        crate::sprites::SpriteAnimation,
        (usize, usize, u32),
    )> {
        self.sprite_animation_placements
            .iter()
            .zip(&self.sprite_animations)
            .map(|(&placement, (_, animation))| (placement, animation.clone(), (usize::MAX, 0, 0)))
            .collect()
    }
    /// `TextureAtlas.dumpContents` for this atlas: `dir` is `screenshots/debug`,
    /// the folder `TextureUtil.getDebugTexturePath` names.
    pub fn dump_contents(&self, dir: &Path, name: &str) -> std::io::Result<PathBuf> {
        // `pixels()`: the copy `pack_pixels` compressed once the GPU had the
        // atlas, decompressed; vanilla reads the texture back from the GPU.
        let pixels = self.pixels();
        dump_atlas(dir, name, self.width, self.height, &pixels, &self.sprites)
    }

    pub(super) fn next_revision() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
    /// The atlas's pixels: as built, or uncompressed again after
    /// `pack_pixels` (a renderer being rebuilt).
    pub fn pixels(&self) -> std::borrow::Cow<'_, [u8]> {
        let Some(packed) = self.packed.as_ref().filter(|_| self.pixels.is_empty()) else {
            return std::borrow::Cow::Borrowed(&self.pixels);
        };
        let size = (self.width * self.height * 4) as usize;
        std::borrow::Cow::Owned(
            lz4_flex::decompress_size_prepended(packed.wait())
                .ok()
                .filter(|p| p.len() == size)
                .unwrap_or_else(|| vec![0; size]),
        )
    }
    /// Keep the pixels compressed from now on, once a renderer has them: the
    /// window's GPU holds the atlas, and a copy as built only serves a
    /// renderer made again (another window, a lost device, another graphics
    /// API). They are compressed on a thread of their own; `pixels()` waits
    /// for it.
    pub fn pack_pixels(&mut self) {
        // lz4_flex's packed representation prefixes the uncompressed length as u32.
        if self.pixels.is_empty()
            || self.packed.is_some()
            || u32::try_from(self.pixels.len()).is_err()
        {
            return;
        }
        let raw = Arc::new(std::mem::take(&mut self.pixels));
        let slot = Arc::new(OnceLock::new());
        let (job_raw, job_slot) = (raw.clone(), slot.clone());
        let spawned = std::thread::Builder::new()
            .name("Atlas packing".into())
            .spawn(move || {
                let _ = job_slot.set(lz4_flex::compress_prepend_size(&job_raw));
            });
        if spawned.is_err() {
            let _ = slot.set(lz4_flex::compress_prepend_size(&raw));
        }
        self.packed = Some(slot);
    }
    /// Compress raw atlas pixels synchronously so a cached test atlas holds lz4 bytes, not raw RGBA.
    #[cfg(any(test, feature = "test-support"))]
    pub fn pack_pixels_sync(&mut self) {
        if self.pixels.is_empty() || self.packed.is_some() {
            return;
        }
        let raw = std::mem::take(&mut self.pixels);
        let slot = Arc::new(OnceLock::new());
        let _ = slot.set(lz4_flex::compress_prepend_size(&raw));
        self.packed = Some(slot);
    }
    /// Clone atlas metadata while sharing packed pixels and Arc-backed shaders for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn clone_for_tests(&self) -> Self {
        Self {
            pictures: Default::default(),
            revision: Self::next_revision(),
            pixels: self.pixels.clone(),
            packed: self.packed.clone(),
            galactic_widths: self.galactic_widths,
            width: self.width,
            height: self.height,
            sprites: self.sprites.clone(),
            skin_origins: self.skin_origins.clone(),
            gui: self.gui.clone(),
            non_gui: self.non_gui.clone(),
            registered: self.registered.clone(),
            registered_owners: self.registered_owners.clone(),
            registered_pages: self.registered_pages.clone(),
            stitch_limit: self.stitch_limit,
            object_skins: self.object_skins.clone(),
            object_skins_revision: Self::next_revision(),
            translucent: self.translucent.clone(),
            flags: self.flags.clone(),
            sprite_animations: self.sprite_animations.clone(),
            sprite_animation_placements: self.sprite_animation_placements.clone(),
            font: self.font.clone_for_tests(),
            glint: self.glint.clone(),
            glint_armor: self.glint_armor.clone(),
            text_shaders: self.text_shaders.clone(),
            lines_shader: self.lines_shader.clone(),
            lines_expressions: self.lines_expressions.clone(),
            sky_shader: self.sky_shader.clone(),
            stars_shader: self.stars_shader.clone(),
            sky_expressions: self.sky_expressions.clone(),
            stars_expressions: self.stars_expressions.clone(),
            lightmap_shader: self.lightmap_shader.clone(),
            lightmap_expressions: self.lightmap_expressions.clone(),
            clouds_shader: self.clouds_shader.clone(),
            sprite_modifiers: self.sprite_modifiers.clone(),
            random_textures: self.random_textures.clone(),
            emissive: self.emissive.clone(),
            blinks: self.blinks.clone(),
            material_maps: self.material_maps.clone(),
            white: self.white,
        }
    }
    /// The sprite of the GUI id `id` (`minecraft:` dropped) that is there, with its scaling: the GUI list's own (a removed or failed entry ends the search); else, where
    /// the list says nothing of the id, a registered atlas's sprite of the name, and last a packed
    /// sprite no model or GUI entry owns.
    pub(super) fn gui_present(&self, id: &str) -> Option<(&Rect, Option<crate::ui::Scaling>)> {
        match self.gui.get(id) {
            Some(GuiSprite::Present { rect, scaling }) => Some((rect, *scaling)),
            Some(GuiSprite::Missing) => None,
            None => self
                .registered
                .get(id)
                .or_else(|| self.sprites.get(id).filter(|_| !self.non_gui.contains(id)))
                .map(|rect| (rect, None)),
        }
    }
    /// A GUI atlas sprite by its id (`minecraft:container/slot`, `container/slot`, a mod's
    /// `mymod:panel`; the two spellings of a vanilla id are one key): [`GuiSprite::Present`] for the
    /// GUI list's sprite, or for an unlisted id a registered atlas's sprite or a
    /// packed sprite no model or GUI entry owns, in that order; [`GuiSprite::Missing`] for an id the list removed or failed; none for an id the GUI
    /// atlas does not own. The sprite carries the scaling of the entry itself.
    pub fn gui_lookup(&self, id: &str) -> Option<GuiSprite> {
        let id = default_namespace(id);
        match self.gui_present(id) {
            Some((rect, scaling)) => Some(GuiSprite::Present {
                rect: *rect,
                scaling,
            }),
            None => self.gui.get(id).copied(),
        }
    }
    /// The rect a GUI draw uses for GUI sprite `id`: [`HudAtlas::gui_lookup`]'s, the missing
    /// sprite for an id the list removed or failed, none for an id the atlas does not own.
    pub fn gui_rect(&self, id: &str) -> Option<&Rect> {
        let id = default_namespace(id);
        match self.gui_present(id) {
            Some((rect, _)) => Some(rect),
            None => self
                .gui
                .contains_key(id)
                .then(|| self.sprites.get(MISSING_SPRITE))
                .flatten(),
        }
    }
    pub(crate) fn gui_atlas_page(&self, name: &str) -> Option<usize> {
        let name = default_namespace(name);
        if self.gui.contains_key(name) {
            return None;
        }
        self.registered_owners.get(name).copied()
    }
    /// Texture sampling flags for `name`, following [`HudAtlas::texture_rect`]'s `minecraft:` rule.
    pub fn texture_flags(&self, name: &str) -> Option<TextureFlags> {
        self.flags.get(default_namespace(name)).copied()
    }
    /// A raw texture uses its own page only if the base atlas has no sprite of that name. The
    /// name follows [`HudAtlas::texture_rect`]'s `minecraft:` rule.
    pub fn texture_atlas_page(&self, name: &str) -> Option<usize> {
        let name = default_namespace(name);
        if self.sprites.contains_key(name) {
            return None;
        }
        self.registered_owners.get(name).copied()
    }

    /// [`HudAtlas::gui_rect`] for a draw: an id the atlas lacks draws the missing texture, as vanilla's atlas does.
    pub fn gui_draw_rect(&self, id: &str) -> Option<&Rect> {
        if let Some(rect) = self.gui_rect(id) {
            return Some(rect);
        }
        // An atlas built with no resources has no GUI list, so nothing to miss against.
        if self.gui.is_empty() {
            return None;
        }
        missing_gui_sprite(id);
        self.sprites.get(MISSING_SPRITE)
    }
    /// The rect of a texture by its name: the packed sprite, else a registered atlas's sprite.
    /// One rule for both, vanilla's `ResourceLocation` default namespace: `minecraft:name` and
    /// `name` are the same texture. The raw lookup of the camera draw and a `Blit`, for the
    /// refusal and the draw alike; a GUI id goes through [`HudAtlas::gui_lookup`].
    pub fn texture_rect(&self, name: &str) -> Option<&Rect> {
        let name = default_namespace(name);
        self.sprites.get(name).or_else(|| self.registered.get(name))
    }
    /// [`HudAtlas::gui_rect`], or the missing sprite where the atlas has no such GUI sprite (an
    /// effect without its icon).
    pub fn gui_sprite_or_missing(&self, id: &str) -> Option<Rect> {
        self.gui_rect(id)
            .or_else(|| self.sprites.get(MISSING_SPRITE))
            .copied()
    }
    /// A sprite, or the missing one when the atlas has no such sprite (an
    /// effect without its icon), as a stitched atlas answers.
    pub fn sprite_or_missing(&self, name: &str) -> Option<Rect> {
        self.sprites
            .get(name)
            .or_else(|| self.sprites.get(MISSING_SPRITE))
            .copied()
    }
    pub fn clear_object_skins(&mut self) {
        self.object_skins.clear();
        self.object_skins_revision = Self::next_revision();
    }
    /// Keep a ready profile skin when another source only has its default.
    pub fn set_object_skin(&mut self, key: String, skin: String) {
        use std::collections::hash_map::Entry;
        if self.object_skins.len() >= 1024 && !self.object_skins.contains_key(&key) {
            self.clear_object_skins();
        }
        match self.object_skins.entry(key) {
            Entry::Occupied(entry)
                if skin.starts_with("entity/player/") && self.sprites.contains_key(entry.get()) => {
            }
            Entry::Occupied(mut entry) => {
                if entry.get() != &skin {
                    entry.insert(skin);
                    self.object_skins_revision = Self::next_revision();
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(skin);
                self.object_skins_revision = Self::next_revision();
            }
        }
    }
    /// A font glyph's atlas cell (8x8 GUI pixels) and advance.
    pub fn glyph(&self, ch: char) -> Option<(Rect, f32)> {
        let g = self.font.glyph(ch);
        g.rect.map(|rect| (rect, g.advance))
    }
    /// A font glyph in the default font: its atlas cell (none for blank
    /// glyphs such as the space), its box and advance.
    pub fn glyph_cell(&self, ch: char) -> crate::font::Glyph {
        self.font.glyph(ch)
    }
    pub fn build(root: &ResourceSource) -> Result<Self> {
        Self::build_over(root, root)
    }
    /// [`Self::build`] bounded by the GPU's `max_texture_dimension_2d` limit.
    pub fn build_with_limit(root: &ResourceSource, max_atlas_size: u32) -> Result<Self> {
        Self::build_over_and_limit(root, root, root, max_atlas_size)
    }
    /// The atlas of a resource root that layers packs over `defaults` (the
    /// game's own resources), whose textures give the sprites' GUI sizes.
    pub fn build_over(root: &ResourceSource, defaults: &ResourceSource) -> Result<Self> {
        Self::build_over_and_limit(root, defaults, defaults, MAX_ATLAS_SIZE)
    }
    /// [`Self::build_over`] bounded by the renderer device's actual
    /// `max_texture_dimension_2d` limit (`TextureAtlas.maxSupportedTextureSize` in vanilla 26.3).
    /// `game` is the game's own resources, without the packs: a
    /// pack's core shader (text, block outline, sky, stars, lightmap) is one that differs from
    /// the game's, so a `root` given as its own `game` loads none.
    pub fn build_over_and_limit(
        root: &ResourceSource,
        defaults: &ResourceSource,
        game: &ResourceSource,
        max_atlas_size: u32,
    ) -> Result<Self> {
        let mut atlas = Self::pack(root, defaults, WALKED_TEXTURE_FOLDERS, true, max_atlas_size)?;
        pack_text::load_shaders(&mut atlas, root, game)?;
        atlas.sprite_modifiers = std::sync::Arc::new(sprite_modifiers::SpriteModifiers::read(
            root,
            atlas.gui.keys(),
        ));
        Ok(atlas)
    }
    /// The packs' core shaders at `root` over the game's own resources `game`, as
    /// [`Self::build_over_and_limit`] loads them, for tests of the pipelines.
    #[cfg(test)]
    pub(crate) fn load_pack_shaders(&mut self, root: &ResourceSource, game: &ResourceSource) {
        pack_text::load_shaders(self, root, game).expect("the packs' required shaders");
    }
    /// Empty slots written at run time: map textures (game/blocks/maps/mod.rs),
    /// downloaded skins, world
    /// and pack icons and the on-screen keyboard's text. Their pixels are already zero in
    /// the destination atlas, so keep the source buffers empty and avoid duplicating the
    /// atlas-sized allocation during packing.
    pub(super) fn slots(images: &mut Vec<PackedImage>, trim: (u32, u32)) {
        for i in 0..MAP_SLOTS {
            let name = map_slot_sprite(i).unwrap_or_default();
            images.push((name.into(), 128, 128, Vec::new()));
        }
        // Downloaded skins (game/interface/scoreboard/mod.rs).
        for i in 0..SKIN_SLOTS {
            images.push((format!("sunrise/skin{i}"), 64, 64, Vec::new()));
        }
        for i in 0..TRIM_SLOTS {
            images.push((format!("sunrise/trim{i}"), trim.0, trim.1, Vec::new()));
        }
        for i in 0..WORLD_ICON_SLOTS {
            images.push((format!("sunrise/world-icon{i}"), 64, 64, Vec::new()));
        }
        for i in 0..PACK_ICON_SLOTS {
            images.push((
                format!("sunrise/pack-icon{i}"),
                PACK_ICON_SIZE,
                PACK_ICON_SIZE,
                Vec::new(),
            ));
        }
        // The on-screen keyboard's text and corner masks (ui_theme), rasterized at run time.
        images.push(("sunrise/ui-text".into(), 2048, 1024, Vec::new()));
    }
    /// Only the font's metrics, for a game that draws nothing (the
    /// integrated or dedicated server): text measures as with the full atlas,
    /// without packing textures or rasterizing glyphs.
    pub fn fonts(root: &ResourceSource) -> Result<Self> {
        let white = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 1,
            h: 1,
        };
        Ok(Self {
            pictures: Default::default(),
            skin_origins: Vec::new(),
            revision: Self::next_revision(),
            pixels: Vec::new(),
            packed: None,
            material_maps: Default::default(),
            galactic_widths: [0; 128],
            width: 1,
            height: 1,
            sprites: HashMap::new(),
            gui: HashMap::new(),
            non_gui: HashSet::new(),
            registered: HashMap::new(),
            registered_owners: HashMap::new(),
            registered_pages: Vec::new(),
            stitch_limit: None,
            object_skins: HashMap::new(),
            object_skins_revision: Self::next_revision(),
            translucent: HashSet::new(),
            flags: HashMap::new(),
            sprite_animations: Vec::new(),
            sprite_animation_placements: Vec::new(),
            font: crate::font::Font::metrics(root),
            glint: (Vec::new(), 1, 1),
            glint_armor: (Vec::new(), 1, 1),
            text_shaders: Default::default(),
            lines_shader: None,
            lines_expressions: Vec::new(),
            sky_shader: None,
            stars_shader: None,
            sky_expressions: Vec::new(),
            stars_expressions: Vec::new(),
            lightmap_shader: None,
            lightmap_expressions: Vec::new(),
            clouds_shader: None,
            sprite_modifiers: Default::default(),
            random_textures: Default::default(),
            emissive: Arc::default(),
            blinks: Default::default(),
            white,
        })
    }
    /// The boot screen's atlas, before any pack loads: the theme's run-time
    /// glyphs (`sunrise/ui-text`), a white sprite and the glyph rows over
    /// transparent black. Nothing from a pack: the boot screen draws with
    /// the theme's bundled faces, so the font here has no sheets to read.
    pub fn boot() -> Self {
        let (width, height) = (1024, 1024);
        let text_rows = 512;
        let glyph_top = text_rows + 9;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        // The white sprite the fills and the font's underline pass sample.
        for row in 0..4 {
            let at = ((text_rows + row) * width * 4) as usize;
            pixels[at..at + 16].fill(255);
        }
        // `SpecialGlyphs.MISSING` (5x8 outline) at (5, text_rows), leaving a 1-texel
        // transparent gutter around it before the run-time glyph region at `glyph_top`.
        let missing_glyph = missing_glyph_rgba();
        for row in 0..8u32 {
            let src = (row * 5 * 4) as usize;
            let dst = (((text_rows + row) * width + 5) * 4) as usize;
            pixels[dst..dst + 20].copy_from_slice(&missing_glyph[src..src + 20]);
        }
        let rect = |u0: u32, v0: u32, u1: u32, v1: u32, w: u32, h: u32| Rect {
            u0: u0 as f32 / width as f32,
            v0: v0 as f32 / height as f32,
            u1: u1 as f32 / width as f32,
            v1: v1 as f32 / height as f32,
            w,
            h,
        };
        let mut sprites = HashMap::new();
        sprites.insert(
            "sunrise/white".into(),
            rect(0, text_rows, 4, text_rows + 4, 4, 4),
        );
        sprites.insert(
            MISSING_GLYPH_SPRITE.into(),
            rect(5, text_rows, 10, text_rows + 8, 5, 8),
        );
        sprites.insert(
            "sunrise/ui-text".into(),
            rect(0, 0, width, text_rows, width, text_rows),
        );
        let white = sprites["sunrise/white"];
        Self {
            pictures: Default::default(),
            skin_origins: Vec::new(),
            revision: Self::next_revision(),
            font: crate::font::Font::load(
                &crate::resource_source::ResourceSource::default(),
                &sprites,
                &pixels,
                (width, height),
                (glyph_top, height - glyph_top),
                white,
            ),
            material_maps: Default::default(),
            galactic_widths: galactic_widths(&sprites, &pixels, width, height),
            pixels,
            packed: None,
            width,
            height,
            sprites,
            translucent: HashSet::new(),
            flags: HashMap::new(),
            gui: HashMap::new(),
            non_gui: HashSet::new(),
            registered: HashMap::new(),
            registered_owners: HashMap::new(),
            registered_pages: Vec::new(),
            stitch_limit: None,
            object_skins: HashMap::new(),
            object_skins_revision: Self::next_revision(),
            sprite_animations: Vec::new(),
            sprite_animation_placements: Vec::new(),
            glint: (vec![0; 4], 1, 1),
            glint_armor: (vec![0; 4], 1, 1),
            white,
            text_shaders: Default::default(),
            lines_shader: None,
            lines_expressions: Vec::new(),
            sky_shader: None,
            stars_shader: None,
            sky_expressions: Vec::new(),
            stars_expressions: Vec::new(),
            lightmap_shader: None,
            lightmap_expressions: Vec::new(),
            clouds_shader: None,
            sprite_modifiers: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
        }
    }
}
