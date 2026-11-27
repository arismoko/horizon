//! Shader packs: the one picked on Options > Sunrise Settings > Shaders, or
//! `SUNRISE_SHADER_PACK=<name>` (development; it wins over the pick), runs `shaderpacks/<name>`
//! (a folder or a zip beside `options.txt`) over the world. The programs of the current
//! dimension's folder are loaded when the dimension changes (spec 1.2). A pick in the removed
//! WGSL format is turned off with a note (`shader_pack_unsupported`).

use super::remote_camera::CameraFogEffects;
use sunrise_client::camera::CameraFluid;
use sunrise_client::gfx::pack_uniforms::{PackInputs, PlayerStatus, ViewEntity};
use sunrise_client::shader_options::ShaderOptions;
use sunrise_game_data::text::Text;
use sunrise_mod_package::mod_list::ModList;
use sunrise_shaders::{
    BlockIds, BlockLayers, Macros, Pack, Properties, Tonemap, dimension_folder_named, load_post,
};

use super::native_shaders::{fingerprint, outcome, refused, remember};
use super::{Dim, Screen};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use sunrise_client_render::render_geometry::EntitySpan;

/// The values the player chose for pack `name`'s options (`shaderpacks/<name>.txt`).
pub(in crate::game::client_side) fn chosen_shader_options(name: &str) -> BTreeMap<String, String> {
    let file = crate::game::ClientGame::shader_packs_dir().join(sunrise_shaders::chosen_file(name));
    std::fs::read_to_string(file)
        .map(|text| sunrise_shaders::parse_chosen(&text))
        .unwrap_or_default()
}

/// Pack `name`'s options, screens and names in `language`, with the values the player chose.
pub(in crate::game::client_side) fn read_shader_options(
    name: &str,
    language: &str,
    macros: &Macros,
) -> Option<ShaderOptions> {
    let pack = match Pack::open(&crate::game::ClientGame::shader_packs_dir().join(name)) {
        Ok(pack) => pack,
        Err(e) => {
            log::warn!("shader pack {name}: {e}");
            return None;
        }
    };
    let options = sunrise_shaders::options(&pack);
    let lang = sunrise_shaders::Lang::load(&pack, language);
    let chosen = chosen_shader_options(name);
    let pack = pack.with_chosen(chosen.clone());
    let text = pack.read_text("shaders.properties").unwrap_or_default();
    let properties = Properties::parse(&text, &sunrise_shaders::properties_macros(&pack, macros));
    let screens = sunrise_shaders::Screens::parse(&properties, &options);
    Some(ShaderOptions::new(
        name.to_owned(),
        options,
        screens,
        lang,
        chosen,
    ))
}

/// Saves the values chosen on the Shader Options page; none chosen removes the file.
/// Writes the chosen options of `options.pack`; whether they were written.
pub(in crate::game::client_side) fn write_shader_options(options: &ShaderOptions) -> bool {
    let file = crate::game::ClientGame::shader_packs_dir()
        .join(sunrise_shaders::chosen_file(&options.pack));
    let saved = if options.chosen.is_empty() {
        std::fs::remove_file(&file).or_else(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(e),
        })
    } else {
        std::fs::write(&file, sunrise_shaders::chosen_text(&options.chosen))
    };
    if let Err(e) = &saved {
        log::warn!("shader pack {}: {}: {e}", options.pack, file.display());
    }
    saved.is_ok()
}

/// The pack that runs over the world ("": none): `SUNRISE_SHADER_PACK`, else the pick
/// (`active_pack`).
pub(in crate::game::client_side) fn dev_pack() -> Option<&'static str> {
    static DEV_PACK: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    DEV_PACK
        .get_or_init(|| std::env::var("SUNRISE_SHADER_PACK").ok())
        .as_deref()
}

pub(in crate::game::client_side) fn picked_pack(
    settings: &sunrise_client::settings::Settings,
) -> &str {
    active_pack(
        dev_pack().unwrap_or(&settings.sunrise.shader_pack),
        &settings.mods,
    )
}

/// What the toggle key decided: the pack it turned off or asked for, a hint when there is
/// nothing (or nothing left) to resume, or nothing when the developer override wins.
/// Announcing "on" is the selection request: the loader still has to accept the pack (its
/// refusal path reports that), and a pack whose owning mod is off never starts.
pub(in crate::game) enum TogglePlan {
    /// Turned `name` off; the chat names it.
    Off(String),
    /// Asked for `name`; the chat names it.
    On(String),
    /// The last pick's owning mod is off: hint with its name, change nothing.
    HintModOff(String),
    /// Nothing to resume: clear `last` (a pack removed after it was turned off) and hint.
    HintClearLast,
    /// Nothing to resume; hint.
    Hint,
    /// The developer override wins; do nothing.
    Noop,
}

/// The toggle key's decision on the pick, without touching the game: `dev_override` is
/// `dev_pack()` (the `SUNRISE_SHADER_PACK` override wins over the pick), `present` says
/// whether the last pick is still installed, and `active` whether its owning mod lets it
/// run (`active_pack`). Pure so tests cover every branch without a game.
pub(in crate::game::client_side) fn plan_shader_toggle(
    pick: &str,
    last: &str,
    dev_override: bool,
    present: impl FnOnce(&str) -> bool,
    active: impl FnOnce(&str) -> bool,
) -> TogglePlan {
    if dev_override {
        return TogglePlan::Noop;
    }
    if !pick.is_empty() {
        return TogglePlan::Off(pick.to_owned());
    }
    if last.is_empty() {
        return TogglePlan::Hint;
    }
    if !present(last) {
        return TogglePlan::HintClearLast;
    }
    if !active(last) {
        return TogglePlan::HintModOff(last.to_owned());
    }
    TogglePlan::On(last.to_owned())
}

/// What the renderer made before any `Game` exists (the loading screen's) readies for the pick,
/// with `pick_hdr` the game then keeps (`NativeShaders::pick_hdr`), so its notes are logged once.
pub(in crate::game) fn startup_pack_shape(
    settings: &sunrise_client::settings::Settings,
    pick_hdr: &mut PickHdr,
) -> PickedPack {
    pack_shape(
        settings,
        super::native_shaders::linear_frame_switch(),
        pick_hdr,
    )
}

/// The `sunrise.hdr` request of pack `name` at `path` as the frame is picked for it: its
/// `shaders.properties` read with the standard macros and the pack's options as chosen, since
/// the renderer's macros come with the renderer the answer picks; and a note for each value
/// the request's keys don't take. Nothing when the pack can't be read.
fn read_pick_hdr(name: &str, path: &Path) -> (Option<Tonemap>, Vec<String>) {
    let Ok(pack) = Pack::open(path) else {
        return (None, Vec::new());
    };
    let pack = pack.with_chosen(chosen_shader_options(name));
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut Macros::standard());
    hdr_request(&properties)
}

/// The request in `properties` and its notes, as the log gives them.
fn hdr_request(properties: &Properties) -> (Option<Tonemap>, Vec<String>) {
    let (hdr, notes) = properties.sunrise_hdr();
    let notes = notes
        .into_iter()
        .map(|n| format!("shaders.properties:{}: {}", n.line, n.message))
        .collect();
    (hdr, notes)
}

/// The loader's read of pack `name`'s `shaders.properties` with `macros` (the standard set and
/// the renderer's), and what it logs about it, in order: each required feature Sunrise lacks,
/// each note of the file. Under the switch, also each note about the HDR request not already
/// logged for the pick (`pick_hdr`), and, when the request read here differs from `frame`, the
/// one the renderer's frame was made for (`Renderer::pack_hdr`), that the frame's answer counts.
/// A load reads the pack's files again: the pick's request is read again at the next frame.
fn loaded_properties(
    pack: &Pack,
    macros: &mut Macros,
    switch: bool,
    frame: Option<Tonemap>,
    name: &str,
    pick_hdr: &mut PickHdr,
) -> (Properties, Vec<String>) {
    let (properties, mut log) = sunrise_shaders::read_properties(pack, macros);
    log.extend(
        properties
            .notes
            .iter()
            .map(|n| format!("shaders.properties:{}: {}", n.line, n.message)),
    );
    if switch {
        let (loaded, notes) = hdr_request(&properties);
        pick_hdr.follow(name);
        log.extend(pick_hdr.unreported(notes));
        if loaded != frame {
            log.push(format!(
                "shaders.properties asks for Sunrise's HDR as {loaded:?} with the renderer's \
                 macros, but the frame was made for {frame:?}: the frame's answer counts"
            ));
        }
        pick_hdr.forget();
    }
    (properties, log)
}

/// The shader pack key (Sunrise Settings' Shaders page), by identity: the translated key's
/// vanilla name against the bound one, so a rebind never becomes another mapping's default
/// (Replay Studio's camera-keyframe shortcut stays physical K's).
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn shader_pack_key_press(&mut self, name: &str) -> bool {
        if self.client.screen != Screen::Playing
            || name == "key.keyboard.unknown"
            || self.client.settings.extras.shader_pack_key != name
        {
            return false;
        }
        self.toggle_shader_pack();
        true
    }

    /// What a renderer made now readies for the pick, with the pick's HDR request as last read
    /// (`NativeShaders::pick_hdr`).
    pub(in crate::game) fn picked_pack_shape(&self) -> PickedPack {
        pack_shape(
            &self.client.settings,
            super::native_shaders::linear_frame_switch(),
            &mut self.client.native_shaders.pick_hdr.borrow_mut(),
        )
    }

    /// `RenderAssets::shader_pack_block_id`: the running `shaders/` pack's `block.properties` id
    /// for the block state at registry index `state_idx`, or `None` while no pack runs (the
    /// panorama and a pack still loading included) or the pack gives it none.
    pub(in crate::game) fn active_block_state_id(&self, state_idx: usize) -> Option<i32> {
        running_block_state_id(
            self.shader_pack_active(),
            self.client
                .renderer
                .0
                .as_ref()
                .and_then(|r| r.shader_pack_kind()),
            self.client.pack_ids.as_ref(),
            state_idx,
        )
    }
}

/// What a renderer readies for the pick, unless it was refused, is missing or its owner is off.
#[derive(Clone, Copy)]
pub(crate) struct PickedPack {
    /// It draws flat where the device can: the sections are laid out so.
    pub draws_flat: bool,
    /// Its passes read the solid quads' vertices: they start wide.
    pub starts_wide: bool,
    /// Under `SUNRISE_HDR_FRAME=float`, with no pack available or one that asks for Sunrise's
    /// HDR (`hdr`): the frame is the layered float one where HDR output is on (the renderer
    /// decides, `FrameRequest::hdr_float`).
    pub hdr_float: bool,
    /// Under the switch, the running pack's `sunrise.hdr` request (`PickHdr`); the renderer
    /// made for this pick keeps it (`Renderer::pack_hdr`).
    pub hdr: Option<Tonemap>,
}
impl PickedPack {
    /// What it asks of the renderer's frame.
    pub(crate) fn frame(self) -> sunrise_client::gfx::FrameRequest {
        sunrise_client::gfx::FrameRequest::new().with_hdr_float(self.hdr_float)
    }
}

/// The shape of `settings`' pick with `switch` for `SUNRISE_HDR_FRAME=float`, taking its HDR
/// request from `pick_hdr`.
fn pack_shape(
    settings: &sunrise_client::settings::Settings,
    switch: bool,
    pick_hdr: &mut PickHdr,
) -> PickedPack {
    let picked = picked_pack(settings);
    let dir = crate::game::ClientGame::shader_packs_dir();
    let path = dir.join(picked);
    let active = !picked.is_empty()
        && !sunrise_client::shader_pack_list::unsupported(&dir, picked)
        && (!switch || (path.exists() && outcome(&path) != Some(false)))
        && path
            .file_name()
            .is_none_or(|f| !active_pack(&f.to_string_lossy(), &settings.mods).is_empty());
    // Sunrise's HDR is asked for only where it can be had: under the switch, by a pack that runs.
    let hdr = if !switch {
        None
    } else if active {
        pick_hdr.answer(picked, &path)
    } else {
        // Not running: whatever runs it again reads its files as they are then.
        pick_hdr.forget();
        None
    };
    // A pack draws the terrain flat where the device can, and its passes read the solid quads'
    // vertices: the sections are laid out so.
    PickedPack {
        draws_flat: active,
        starts_wide: active,
        hdr_float: switch && (!active || hdr.is_some()),
        hdr,
    }
}

/// The pick's `sunrise.hdr` request under the switch. The frame follows the pick every frame
/// (`follow_frame`), and reading the request reads all the pack's sources for its options. It
/// is read again when the pick changes, after a frame where the pick doesn't run (off, refused,
/// missing, its mod off), and after each point where the pick's files or chosen options can
/// change (`forget`): a load of the pack, an options save, a resource reload, a scan of the
/// packs. Each frame otherwise compares the pick's name and reads nothing.
#[derive(Default)]
pub(crate) struct PickHdr {
    /// The pick (`picked_pack`) the rest is for.
    pick: String,
    /// Its request as last read; `None`: to be read.
    hdr: Option<Option<Tonemap>>,
    /// The notes about its request already in the log, so a read again doesn't repeat them.
    reported: Vec<String>,
}

impl PickHdr {
    /// The request of pack `picked` at `path`, read unless a read of that pick is kept. A note
    /// about an unknown value is logged once.
    fn answer(&mut self, picked: &str, path: &Path) -> Option<Tonemap> {
        self.follow(picked);
        if let Some(hdr) = self.hdr {
            return hdr;
        }
        let (hdr, notes) = read_pick_hdr(picked, path);
        for note in self.unreported(notes) {
            log::warn!("shader pack {picked}: {note}");
        }
        self.hdr = Some(hdr);
        hdr
    }

    /// The pick's files or chosen options may have changed: the next answer reads them again.
    pub(in crate::game) fn forget(&mut self) {
        self.hdr = None;
    }

    fn follow(&mut self, picked: &str) {
        if self.pick != picked {
            picked.clone_into(&mut self.pick);
            self.hdr = None;
            self.reported.clear();
        }
    }

    /// The notes of `notes` not yet in the log, now counted as in it.
    fn unreported(&mut self, notes: Vec<String>) -> Vec<String> {
        let new: Vec<String> = notes
            .into_iter()
            .filter(|n| !self.reported.contains(n))
            .collect();
        self.reported.extend(new.iter().cloned());
        new
    }
}

/// The pick's HDR request `picked` (`PickedPack::hdr`) where the frame's configuration was
/// looked at again: a frame that `follows` serves it from now on (`installed`,
/// `Renderer::pack_hdr`). Whether the frame is to be made again; until a new one is made, the
/// installed one keeps its own.
pub(in crate::game) fn reconcile_pack_hdr(
    follows: bool,
    installed: &mut Option<Tonemap>,
    picked: Option<Tonemap>,
) -> bool {
    if follows {
        *installed = picked;
    }
    !follows
}

// A native mod can bundle a pack as `native-<mod-id>`; its Mods toggle controls it too.
pub(in crate::game::client_side) fn active_pack<'a>(picked: &'a str, mods: &ModList) -> &'a str {
    if picked.strip_prefix("native-").is_some_and(|id| {
        mods.installed
            .iter()
            .any(|m| m.id == id && m.native.is_some() && mods.disabled.contains(id))
    }) {
        ""
    } else {
        picked
    }
}

/// A selected pack may read resource-pack material maps.
pub fn picks_material_pack(settings: &sunrise_client::settings::Settings) -> bool {
    let dev = std::env::var("SUNRISE_SHADER_PACK").ok();
    let picked = active_pack(
        dev.as_deref().unwrap_or(&settings.sunrise.shader_pack),
        &settings.mods,
    );
    if picked.is_empty() {
        return false;
    }
    let dir = crate::game::ClientGame::shader_packs_dir();
    if sunrise_client::shader_pack_list::unsupported(&dir, picked) {
        return false;
    }
    let path = dir.join(picked);
    path.file_name()
        .is_none_or(|f| !active_pack(&f.to_string_lossy(), &settings.mods).is_empty())
}

/// The id table is read only while a `shaders/` (Iris-format) pack is loaded and rendering:
/// `pack_ids` outlives the panorama and is set before the pack's pipelines finish building.
fn running_block_state_id(
    active: bool,
    kind: Option<&str>,
    ids: Option<&PackIds>,
    state_idx: usize,
) -> Option<i32> {
    if !active || kind != Some("iris") {
        return None;
    }
    ids?.block_state_id(state_idx)
}

/// The ids a pack gives entities and block entities (4.1, 5.3), for the spans of the entity
/// geometry.
#[cfg_attr(any(test, feature = "test-support"), derive(Default))]
pub struct PackIds {
    pub(in crate::game) entities: HashMap<String, i32>,
    pub(in crate::game) blocks: BlockIds,
    /// `block.properties`' `layer.*` rules (4.2), as the terrain meshes them.
    pub(in crate::game::client_side) layers: super::pack_layers::PackLayers,
    /// `terrain_state_ids` of the loaded pack: two ids per block state, the model's then the
    /// fluid's (the renderer holds the same table).
    pub(in crate::game) terrain_states: Vec<i32>,
    /// `blockEntityId` by block state, filled as states are drawn.
    pub(in crate::game) block_states: RefCell<HashMap<u32, i32>>,
    /// The id of a name the file lacks: 65535, or 0 without the file.
    pub(in crate::game) missing_entity: i32,
    pub(in crate::game) missing_block: i32,
    /// `item.properties` (4.1) and the id of a held item the file lacks, or of an empty hand:
    /// −1, or 0 without the file (5.3).
    pub(in crate::game) items: HashMap<String, i32>,
    pub(in crate::game) missing_item: i32,
    /// `currentRenderedItemId` of each block item (`StateFacts::item` of its block's default
    /// state): that state's `block.properties` id, or 0.
    pub(in crate::game) block_items: HashMap<String, i32>,
    /// Whether the pack has `item.properties` (`currentRenderedItemId` of an item it lacks).
    pub(in crate::game) item_file: bool,
    /// `oldHandLight` (3.2): the first held item and light are those of the brighter hand.
    pub(in crate::game) old_hand_light: bool,
    /// `vignette`: `false` hides the game's vignette over the pack's image.
    pub(in crate::game) vignette: bool,
    /// `oldLighting`: `true` keeps the game's face shading in the block colours.
    pub(in crate::game) old_lighting: bool,
    /// `separateAo`: `true` passes AO factor in vertex alpha and keeps pure tint in RGB.
    pub(in crate::game) separate_ao: bool,
}

impl PackIds {
    /// The pack's `block.properties` id for the block state at registry index `state`, the
    /// model id the terrain carries; `None` when the file gives it none or the index is past
    /// the registry.
    pub(in crate::game::client_side) fn block_state_id(&self, state: usize) -> Option<i32> {
        self.terrain_states
            .get(state.checked_mul(2)?)
            .copied()
            .filter(|id| *id >= 0)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl PackIds {
    pub fn with_entity_ids<'a>(ids: impl IntoIterator<Item = (&'a str, i32)>) -> Self {
        Self {
            entities: ids
                .into_iter()
                .map(|(name, id)| (name.into(), id))
                .collect(),
            ..Default::default()
        }
    }
}

impl crate::game::ClientGame<'_> {
    /// Loads the pack for the current dimension when it changed, and returns this frame's
    /// inputs to the pack's uniforms.
    pub(in crate::game::client_side) fn shader_pack_inputs(
        &mut self,
        camera: glam::DVec3,
        eye: glam::DVec3,
        partial: f32,
        sky: &sunrise_level::environment::Sky,
    ) -> PackInputs {
        // A `shaders/` pack lights the faces itself unless `oldLighting=true`: sections mesh
        // without the game's face shading (the pack loaded or dropped below shows next frame).
        let pack_lighting = self
            .client
            .pack_ids
            .as_ref()
            .is_some_and(|ids| !ids.old_lighting);
        if self.client.streaming.shader_pack_lighting != pack_lighting {
            self.client.streaming.shader_pack_lighting = pack_lighting;
            self.all_sections_changed();
        }
        // Any `shaders/` pack (its `oldLighting` too) draws fluid tops split as Iris does.
        let shader_pack = self.client.pack_ids.is_some();
        if self.client.streaming.shader_pack != shader_pack {
            self.client.streaming.shader_pack = shader_pack;
            self.all_sections_changed();
        }
        // The renderer keeps the meshes' sources while they carry them, and frees them after.
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.keep_pack_sources(shader_pack);
        }
        let separate_ao = self
            .client
            .pack_ids
            .as_ref()
            .is_some_and(|ids| ids.separate_ao);
        if self.client.streaming.separate_ao != separate_ao {
            self.client.streaming.separate_ao = separate_ao;
            self.all_sections_changed();
        }
        // Its `layer.*` rules (4.2), none without a pack: in the frame after it loads or drops, as
        // the flags above, so one meshing of every section takes them all.
        self.sync_pack_layers();
        let selected = picked_pack(&self.client.settings).to_owned();
        let dir = Self::shader_packs_dir();
        let path = dir.join(&selected);
        let unsupported = sunrise_client::shader_pack_list::unsupported(&dir, &selected);
        if unsupported {
            self.shader_pack_unsupported(&selected);
        }
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let picked = if selected.is_empty()
            || unsupported
            || active_pack(&file, &self.client.settings.mods).is_empty()
        {
            String::new()
        } else {
            path.to_string_lossy().into_owned()
        };
        if self.client.native_shaders.picked != picked {
            self.client.native_shaders.picked = picked.clone();
            self.set_translucent_layers(Vec::new());
            self.client.pack_ids = None;
            if let Some(renderer) = &mut self.client.renderer.0 {
                renderer.drop_shader_packs();
            }
        }
        // A pack whose pipelines were building runs once they are built, or is turned off.
        if let Some((dimension, name, started, report)) = self
            .client
            .renderer
            .0
            .as_mut()
            .and_then(|r| r.poll_shader_pack())
        {
            match report {
                Ok(Some((programs, notes))) => {
                    remember(&path, fingerprint(&path), true);
                    for note in notes {
                        log::warn!("shader pack {name}: {note}");
                    }
                    log::info!(
                        "shader pack {name}: {} programs ({}) in {} ms",
                        programs.len(),
                        programs.join(" "),
                        started.elapsed().as_millis()
                    );
                }
                Ok(None) => remember(&path, fingerprint(&path), false),
                // Stopped because it isn't wanted any more: not the pack's failure.
                Err(refused) if refused.is_cancelled() => {}
                Err(refused) => {
                    for note in refused.notes {
                        log::warn!("shader pack {name}: {note}");
                    }
                    self.shader_pack_failed(&name, dimension, &refused.reason);
                }
            }
        }
        if picked.is_empty() {
            return PackInputs::default();
        }
        if !sunrise_client::sprites::labpbr_maps() {
            sunrise_client::sprites::set_labpbr_maps(true);
            self.reload_packs();
            return PackInputs::default();
        }
        let name = selected.clone();
        let dimension = match self.common.dim {
            Dim::Overworld | Dim::Extra(_) => 0,
            Dim::Nether => -1,
            Dim::End => 1,
        };
        if let Some(renderer) = &mut self.client.renderer.0
            && renderer.shader_pack_dimension() != Some(dimension)
            && !(super::native_shaders::linear_frame_switch() && refused(&path, fingerprint(&path)))
        {
            let name = name.as_str();
            let started = std::time::Instant::now();
            // A pack is not the game's own code: one that makes the shader compiler (or our
            // translation of it) panic is turned off instead of crashing every world load (#3525).
            let loaded = sunrise_level::crash::contain(|| {
                match Pack::open(&path).map(|p| p.with_chosen(chosen_shader_options(name))) {
                    Ok(pack) => {
                        let mut macros = Macros::standard();
                        renderer.define_pack_macros(&mut macros);
                        // Its conditionals test the pack's options too.
                        let (properties, log) = loaded_properties(
                            &pack,
                            &mut macros,
                            super::native_shaders::linear_frame_switch(),
                            renderer.pack_hdr,
                            name,
                            &mut self.client.native_shaders.pick_hdr.borrow_mut(),
                        );
                        for line in log {
                            log::warn!("shader pack {name}: {line}");
                        }
                        let separate_ao = properties.separate_ao();
                        renderer.set_separate_ao(separate_ao);
                        // Lava's faces draw with the solid terrain's program.
                        renderer.set_pack_lava_layers(
                            ["lava_still", "lava_flow"]
                                .iter()
                                .filter_map(|f| {
                                    let name = format!("minecraft:block/{f}");
                                    self.client
                                        .client_pack
                                        .render
                                        .sprite_layers
                                        .get(&name)
                                        .map(|&l| l as u16)
                                })
                                .collect(),
                        );
                        let folder = dimension_folder_named(&pack, self.common.dim.id(), dimension);
                        // Its terrain programs read the section arena's pages, or else the
                        // normal stream, where they still fit the vertex stage with them.
                        renderer.set_pack_terrain_bindings(
                            sunrise_client::gfx::pack_atlas::TerrainBindings::of(&properties),
                        );
                        let inputs = renderer.pack_inputs();
                        let mut post = load_post(&pack, &folder, &properties, &macros, &inputs);
                        // Textures that name one of the game's: from the resource packs over
                        // the game's own resources.
                        let resources = self.common.pack.source.clone();
                        let read = |namespace: &str, path: &str| {
                            resources.read(&format!("assets/{namespace}/{path}")).ok()
                        };
                        for note in sunrise_shaders::resolve_resources(&mut post.textures, &read) {
                            log::warn!("shader pack {name}: {note}");
                        }
                        let biomes = world_biomes!(self.common);
                        let names: Vec<&str> =
                            biomes.list.iter().map(|b| b.name.as_str()).collect();
                        post.custom.set_biomes(&names);
                        let text = pack.read_text("block.properties").unwrap_or_default();
                        let block_properties = Properties::parse(&text, &macros);
                        let ids = BlockIds::parse(&block_properties);
                        let layers = BlockLayers::parse(&block_properties);
                        for note in ids.notes.iter().chain(&layers.notes) {
                            log::warn!("shader pack {name}: block.properties: {note}");
                        }
                        let text = pack.read_text("entity.properties");
                        let entities = BlockIds::parse_kind(
                            &Properties::parse(text.as_deref().unwrap_or_default(), &macros),
                            "entity",
                        );
                        let text = pack.read_text("item.properties");
                        let items = BlockIds::parse_kind(
                            &Properties::parse(text.as_deref().unwrap_or_default(), &macros),
                            "item",
                        );
                        let block_items = block_item_ids(
                            self.common.res.facts(),
                            self.common.pack.assets.registry,
                            &ids,
                        );
                        self.client.pack_ids = Some(PackIds {
                            block_items,
                            item_file: text.is_some(),
                            items: items.plain(),
                            missing_item: if text.is_some() { -1 } else { 0 },
                            // OQ-21: on unless the pack turns it off.
                            old_hand_light: properties
                                .get("oldHandLight")
                                .is_none_or(|v| v.trim() != "false"),
                            // On unless the pack turns it off (`vignette=false`).
                            vignette: properties
                                .get("vignette")
                                .is_none_or(|v| v.trim() != "false"),
                            // Off unless the pack turns it on (Iris shades no faces by default).
                            old_lighting: properties
                                .get("oldLighting")
                                .is_some_and(|v| v.trim() == "true"),
                            separate_ao,
                            entities: entities.plain(),
                            missing_entity: if text.is_some() { 65535 } else { 0 },
                            missing_block: if ids.is_empty() { 0 } else { 65535 },
                            blocks: ids.clone(),
                            layers: super::pack_layers::PackLayers::new(name, layers),
                            terrain_states: Vec::new(),
                            block_states: RefCell::default(),
                        });
                        let started_ids = std::time::Instant::now();
                        let table = terrain_state_ids(self.common.pack.assets.registry, &ids);
                        if let Some(pack_ids) = &mut self.client.pack_ids {
                            pack_ids.terrain_states.clone_from(&table);
                        }
                        log::info!(
                            "shader pack {name}: mc_Entity for {} of {} block states in {} ms",
                            table.chunks(2).filter(|p| p[0] != -1).count(),
                            table.len() / 2,
                            started_ids.elapsed().as_millis()
                        );
                        log::info!("shader pack {name}: folder `{folder}`, building its pipelines");
                        // The driver compiles the programs on a thread of their own; the game
                        // draws without a pack meanwhile, and the next frames pick it up.
                        renderer.load_shader_pack(dimension, post, table, name.to_owned(), started);
                    }
                    Err(e) => {
                        remember(&path, fingerprint(&path), false);
                        self.client.pack_ids = None;
                        log::warn!("shader pack {name}: {e}");
                        // Without a pack this can't fail.
                        let _ = renderer.set_shader_pack(dimension, None, &[]);
                    }
                }
                Ok(())
            });
            // A pack that can't run is turned off like one that panics.
            if let Err(error) = loaded.flatten() {
                self.shader_pack_failed(name, dimension, &error);
            }
        }
        let lerp = |(old, new): (f32, f32)| old + (new - old) * partial;
        let at = camera.floor().as_ivec3();
        let light = self.common.world.light(at.x, at.y, at.z);
        let biome = self
            .common
            .world
            .chunks
            .get(&(at.x >> 4, at.z >> 4))
            .map(|c| c.biome(at.x, at.y, at.z));
        let (held_items, held_light) = self.held_for_pack();
        let status = self.pack_player_status();
        let (end_flash, end_flash_direction) = self.end_flash_for_pack(partial);
        let info = biome.map(|b| world_biomes!(self).get(b));
        let dimension = self.client.remote.as_ref().map(|r| &r.dimension);
        PackInputs {
            // `Level.getDefaultClockTime`: 0 in a dimension type with no default clock (the
            // Nether's has none), as Iris 1.11.7 reads `worldTime` there (probe pack).
            day_time: if dimension.is_some_and(|d| d.default_clock.is_none()) {
                0
            } else {
                self.common.world.day_time
            },
            rain: lerp(self.common.world.weather.rain_level),
            thunder: lerp(self.common.world.weather.thunder_level),
            eye_in: match self.camera_fluid(camera.to_array()) {
                CameraFluid::None => 0,
                CameraFluid::Water => 1,
                CameraFluid::Lava => 2,
                CameraFluid::PowderSnow => 3,
            },
            blindness: f32::from(u8::from(
                self.common.potions.effects.get("blindness").is_some(),
            )),
            eye_light: [i32::from(light & 15), i32::from(light >> 4)],
            hide_gui: self.client.chat_ui.hide_gui,
            brightness: self.client.settings.gamma as f32,
            biome: info.map_or(-1, |b| {
                let names = world_biomes!(self).list.iter().map(|b| b.name.as_str());
                sunrise_shaders::biome_pack_id(&b.name, names)
            }),
            biome_known: biome.is_some(),
            biome_category: info.map_or(0, |b| sunrise_shaders::biome_category(&b.name)),
            biome_precipitation: info.map_or(0, |b| {
                sunrise_shaders::biome_precipitation(b.precipitation, b.temperature)
            }),
            temperature: info.map_or(0.0, |b| b.temperature),
            rainfall: info.map_or(0.0, |b| b.downfall),
            held_items,
            held_light,
            right_handed: !self.client.settings.main_hand_left,
            on_ground: self
                .common
                .player
                .as_ref()
                .is_some_and(|p| p.ground.on_ground),
            min_y: self.common.world.shape.min_y,
            height: self.common.world.shape.height,
            logical_height: self.common.world.shape.logical_height,
            has_skylight: self.common.world.shape.sky_light,
            has_ceiling: self.common.world.shape.has_ceiling,
            elytra_flying: self.common.player.as_ref().is_some_and(|p| p.fall_flying),
            cloud_height: sunrise_client::gfx::pack_uniforms::cloud_height(sky.cloud_height),
            // Vanilla's drift in float, times its 0.030000001F blocks a tick.
            cloud_time: sunrise_client::clouds::drift(
                self.common.world.game_time,
                f64::from(partial),
                self.client.client_pack.clouds.0,
            ) as f32
                * 0.030_000_001,
            status,
            end_flash,
            end_flash_direction,
            ambient_light: dimension.map_or(0.0, |d| d.ambient_light),
            view: self.pack_view_entity(eye, partial),
            still_ticks: self.common.clock.still().then(|| self.visual_ticks()),
            frame_timing: None,
        }
    }

    /// The camera entity's (`Minecraft.getCameraEntity`, [`Self::camera_subject`]) values for
    /// the uniforms tied to the view (#3948), at `eye`, the entity's eye
    /// `Camera.alignWithEntity` placed (the frame's `eyePosition`). The local player's are the
    /// ones the lightmap, the HUD and the cave ambience have; another living entity's are the
    /// effects the server sent for it (its `CameraFogEffects`, the same the fog reads); an
    /// entity that isn't living has none and looks nowhere.
    pub fn pack_view_entity(&self, eye: glam::DVec3, partial: f32) -> ViewEntity {
        let at = eye.floor().as_ivec3();
        let light = self.common.world.light(at.x, at.y, at.z);
        let eye_light = [i32::from(light & 15), i32::from(light >> 4)];
        let other = self.camera_subject().filter(|c| !c.is_local_player);
        let (living, effects, night_vision, mood) = match &other {
            Some(c) => {
                let fx = c.fog_effects;
                let night_vision = fx.night_vision.map_or(0.0, |duration| {
                    super::sculk::night_vision_scale(duration, partial)
                });
                (c.camera.living.is_some(), fx, night_vision, 0.0)
            }
            None => (
                true,
                CameraFogEffects::from_effects(
                    &self.common.potions.effects,
                    self.client.darkness_blend,
                ),
                self.night_vision_intensity(partial),
                self.client.ambience.moodiness,
            ),
        };
        // LivingEntity.getEffectBlendFactor: the fade only while the instance is there.
        let darkness = effects
            .darkness_blend
            .map_or(0.0, |blend| super::sculk::darkness_blend_at(blend, partial));
        ViewEntity {
            living,
            blindness: f32::from(u8::from(effects.blindness.is_some())),
            night_vision,
            darkness: darkness * self.darkness_effect_scale(),
            mood,
            eye_light,
        }
    }

    /// The window's player's state for the player booleans and the status uniforms: the six
    /// Iris also gives programs and the vitals read as the HUD reads them, and the eight only
    /// custom uniforms read (D-2 in `docs/cleanroom/decisions.md`), each as its vanilla method
    /// answers on a client. Asked only while a pack is picked ([`Self::shader_pack_inputs`]
    /// returns before it otherwise).
    pub fn pack_player_status(&mut self) -> PlayerStatus {
        let spectator = self.common.spectator();
        let survival = !self.common.creative && !spectator;
        let invisible = self.own_invisible();
        let riding = self.common.riding.is_some();
        let (glowing, ridden, wet) = self.pack_player_booleans();
        let Some(p) = &mut self.common.player else {
            return PlayerStatus::default();
        };
        PlayerStatus {
            survival,
            health: p.health.health(),
            max_health: p.attribute("minecraft:max_health").unwrap_or(20.0) as f32,
            food: p.health.food.food as f32,
            air: p.air as f32,
            armor: p
                .attribute("minecraft:armor")
                .or_else(|_| p.attribute("armor"))
                .unwrap_or(0.0) as f32,
            spectator,
            riding,
            feet_in_water: p.fluids.was_touching_water,
            swimming: p.pose == sunrise_sim::pose::Pose::Swimming,
            burning: p.is_on_fire() && !spectator,
            hurt: p.health.hurt_animation_time > 0,
            invisible,
            sneaking: p.pose == sunrise_sim::pose::Pose::Crouching,
            sprinting: p.sprinting,
            // `LivingEntity.isAlive`: the window's player is never removed while it is the
            // camera.
            alive: p.health.health() > 0.0,
            glowing,
            in_lava: p.is_in_lava(),
            ridden,
            wet,
        }
    }

    /// `Entity.isCurrentlyGlowing`, `Entity.isVehicle` and `Entity.isInWaterOrRain` of the
    /// window's player, as a client answers them, read without changing anything.
    pub(in crate::game::client_side) fn pack_player_booleans(&self) -> (bool, bool, bool) {
        let id = self.own_entity_id() as u32;
        // `Entity.isCurrentlyGlowing` on a client: shared flag 6 from a server, else the
        // Glowing effect `LivingEntity.isCurrentlyGlowing` sets it from.
        let glowing = self
            .remote_glowing(id)
            .unwrap_or_else(|| self.common.potions.effects.get("glowing").is_some());
        // `Entity.isVehicle`: the server's passenger list, else the passengers here.
        let ridden = self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.has_passengers(id));
        // `Entity.isInWaterOrRain`: `isInWater`, else `isInRain`.
        let wet = self.common.player.as_ref().is_some_and(|p| {
            let bp = sunrise_core::position::BlockPos::new(
                p.position.x.floor() as i32,
                p.position.y.floor() as i32,
                p.position.z.floor() as i32,
            );
            let up = sunrise_core::position::BlockPos::new(
                bp.x,
                p.bounds().max_position().y.floor() as i32,
                bp.z,
            );
            p.fluids.was_touching_water || self.raining_at(bp) || self.raining_at(up)
        });
        (glowing, ridden, wet)
    }

    /// The Shader Options page opened: the picked pack's options, screens and names.
    pub(in crate::game::client_side) fn open_shader_options(&mut self) {
        let mut macros = Macros::standard();
        if let Some(renderer) = &self.client.renderer.0 {
            renderer.define_pack_macros(&mut macros);
        }
        let name = &self.client.settings.sunrise.shader_pack;
        self.client.settings.shader_options =
            read_shader_options(name, &self.client.settings.language, &macros);
    }

    /// The Shader Options page closed with changes: they are saved, and the pack loads again.
    pub(in crate::game::client_side) fn save_shader_options(&mut self) {
        let Some(options) = self.client.settings.shader_options.take() else {
            return;
        };
        if write_shader_options(&options) {
            self.client.native_shaders.pick_hdr.get_mut().forget();
        }
        self.client.pack_ids = None;
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.drop_shader_packs();
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_open_shader_options(&mut self) {
        self.open_shader_options();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_save_shader_options(&mut self) {
        self.save_shader_options();
    }

    /// Turns off the pack `name` after loading it panicked or failed: nothing runs in its place,
    /// it is no longer the picked pack (so the next start does not load it again), and the chat
    /// says so.
    pub(in crate::game::client_side) fn shader_pack_failed(
        &mut self,
        name: &str,
        dimension: i32,
        error: &str,
    ) {
        let path = Self::shader_packs_dir().join(name);
        remember(&path, fingerprint(&path), false);
        log::error!("shader pack {name}: turned off: {error}");
        self.client.pack_ids = None;
        if let Some(renderer) = &mut self.client.renderer.0 {
            // Marks this dimension as loaded, without a pack, so it is not tried again (without
            // a pack this can't fail).
            let _ = renderer.set_shader_pack(dimension, None, &[]);
        }
        if self.client.settings.sunrise.shader_pack == name {
            self.client.settings.sunrise.shader_pack.clear();
            self.client
                .settings
                .save(&sunrise_gameplay_common::game::options_path());
        }
        self.client_system_message(Text::colored(
            format!("Shader pack {name} could not be loaded and was turned off"),
            0xFF5555,
        ));
    }

    /// Turns off the pick `name` in the removed WGSL format: no pack runs in its place, and the
    /// chat says so once. A saved pick falls back to no pack and is saved so; the developer
    /// override (`SUNRISE_SHADER_PACK`) stays as it is.
    fn shader_pack_unsupported(&mut self, name: &str) {
        if dev_pack().is_none() {
            self.client.settings.set_shader_pick("");
            self.client
                .settings
                .save(&sunrise_gameplay_common::game::options_path());
        }
        if self.client.native_shaders.noted != name {
            self.client.native_shaders.noted = name.to_owned();
            log::warn!("shader pack {name}: no longer supported (the WGSL format)");
            self.client_system_message(Text::tr(
                sunrise_client::shader_pack_list::UNSUPPORTED_NOTE,
                &[&name],
            ));
        }
    }

    /// Whether the game's vignette draws: the option, unless the loaded pack turns it off.
    pub(in crate::game::client_side) fn vignette_shown(&self) -> bool {
        self.client.settings.vignette
            && self.client.pack_ids.as_ref().is_none_or(|ids| ids.vignette)
    }

    /// `currentRenderedItemId` of a held `item`, [`rendered_item_id`]'s; 0 without a pack.
    pub(in crate::game::client_side) fn rendered_item_id(&self, item: &str) -> i32 {
        self.client.pack_ids.as_ref().map_or(0, |ids| {
            rendered_item_id(
                ids.block_items.get(item).copied(),
                ids.items.get(item).copied(),
                ids.item_file,
            )
        })
    }

    /// `heldItemId(2)` and `heldBlockLightValue(2)`: main hand, then off hand (5.2, 5.3).
    pub(in crate::game::client_side) fn held_for_pack(&mut self) -> ([i32; 2], [i32; 2]) {
        if self.client.pack_ids.is_none() {
            return ([0; 2], [0; 2]);
        }
        let lum = self.luminance();
        let Some(ids) = &self.client.pack_ids else {
            return ([0; 2], [0; 2]);
        };
        let hands = [
            self.common.selected,
            sunrise_gameplay_common::game::held_items::OFFHAND_SLOT,
        ]
        .map(|slot| {
            let item = self.common.stack_in(slot).map(|s| &*s.item);
            let id = item.and_then(|i| ids.items.get(i).copied());
            (
                id.unwrap_or(ids.missing_item),
                item.map_or(0, |i| i32::from(lum.of(i))),
            )
        });
        let first = if ids.old_hand_light && hands[1].1 > hands[0].1 {
            hands[1]
        } else {
            hands[0]
        };
        ([first.0, hands[1].0], [first.1, hands[1].1])
    }
}

impl crate::game::ClientGame<'_> {
    /// Starts a span of the entity geometry at vertex `start` for entity type `name`
    /// (`minecraft:zombie`; "" for one the pack cannot name). Nothing without a pack.
    pub(in crate::game) fn mark_entity(&self, start: usize, name: &str) {
        self.mark_entity_part(start, name, false);
    }

    /// Starts a span for entity geometry that blends (name tags, shadows).
    pub(in crate::game::client_side) fn mark_translucent_entity(&self, start: usize, name: &str) {
        self.mark_entity_part(start, name, true);
    }

    pub(in crate::game::client_side) fn mark_entity_part(
        &self,
        start: usize,
        name: &str,
        translucent: bool,
    ) {
        if let Some(span) = self.entity_part_span(start, name, translucent) {
            self.push_entity_span(span);
        }
    }

    pub(in crate::game) fn entity_part_span(
        &self,
        start: usize,
        name: &str,
        translucent: bool,
    ) -> Option<EntitySpan> {
        let id = match &self.client.pack_ids {
            Some(ids) => ids
                .entities
                .get(name)
                .copied()
                .unwrap_or(ids.missing_entity),
            None if native_spans() => name_id(name),
            None => return None,
        };
        Some(EntitySpan {
            start: start as u32,
            stage: sunrise_shaders::render_stage("ENTITIES"),
            id,
            translucent,
            ..Default::default()
        })
    }

    /// Starts a span for the block entity of block state `state`.
    pub(in crate::game::client_side) fn mark_block_entity(&self, start: usize, state: u32) {
        let Some(ids) = &self.client.pack_ids else {
            if native_spans() {
                self.push_span(start, "BLOCK_ENTITIES", state as i32);
            }
            return;
        };
        let id = *ids
            .block_states
            .borrow_mut()
            .entry(state)
            .or_insert_with(|| {
                let registry = self.common.pack.assets.registry;
                registry
                    .state(sunrise_world::registry::BlockStateId(state))
                    .and_then(|s| {
                        ids.blocks
                            .id(&s.name, |k| s.properties.get(k).map(String::as_str))
                    })
                    .unwrap_or(ids.missing_block)
            });
        self.push_span(start, "BLOCK_ENTITIES", id);
        let emission = self
            .common
            .res
            .facts()
            .get(sunrise_world::registry::BlockStateId(state))
            .emission;
        if let Some(last) = self.client.entity_spans.borrow_mut().last_mut() {
            last.emissive = emission > 0;
            if let Some(gpu) = self.client.skin_frame.borrow_mut().spans.last_mut() {
                gpu.emissive = emission > 0;
            }
        }
    }

    /// Starts a span for mod model copies (`WorldDrawContext::draw_model_instances`) under a
    /// pack: block entities, as `BlockEntityDrawContext::model` draws the same model, with the
    /// id of a block the pack doesn't list, since no block owns them. Nothing without a pack.
    pub(in crate::game) fn model_copy_span(&self, start: usize) -> Option<EntitySpan> {
        Some(EntitySpan {
            start: start as u32,
            stage: sunrise_shaders::render_stage("BLOCK_ENTITIES"),
            id: self.client.pack_ids.as_ref()?.missing_block,
            ..Default::default()
        })
    }

    /// Starts the span of the local player's model; `hidden` in first person, where only a
    /// pack's shadow pass draws it.
    pub(in crate::game::client_side) fn mark_player(&self, start: usize, hidden: bool) {
        self.mark_entity(start, "minecraft:player");
        if (self.client.pack_ids.is_some() || native_spans())
            && let Some(last) = self.client.entity_spans.borrow_mut().last_mut()
        {
            last.player = true;
            last.hidden = hidden;
            if let Some(gpu) = self.client.skin_frame.borrow_mut().spans.last_mut() {
                gpu.player = true;
                gpu.hidden = hidden;
            }
        }
    }

    /// Marks vertices `start..end` (one entity) as hidden from the camera, for an entity
    /// Sunrise's culling would drop but a pack's shadow pass still draws (`EntitySpan::hide`).
    pub(in crate::game::client_side) fn hide_spans_from(&self, start: usize, end: usize) -> bool {
        EntitySpan::hide(&mut self.client.entity_spans.borrow_mut(), start, end)
    }

    /// Starts a span of geometry that is not an entity (`PARTICLES`, `DESTROY`...).
    pub(in crate::game) fn mark_stage(&self, start: usize, stage: &str) {
        if self.client.pack_ids.is_some() || native_spans() {
            self.push_span(start, stage, 0);
        }
    }

    pub(in crate::game::client_side) fn push_span(&self, start: usize, stage: &str, id: i32) {
        self.push_entity_span(EntitySpan {
            start: start as u32,
            stage: sunrise_shaders::render_stage(stage),
            id,
            ..Default::default()
        });
    }

    fn push_entity_span(&self, span: EntitySpan) {
        let mut spans = self.client.entity_spans.borrow_mut();
        if self.client.pack_ids.is_some() && self.skinning() {
            let mut skin = self.client.skin_frame.borrow_mut();
            let gpu = EntitySpan {
                start: skin.vertices,
                ..span
            };
            match skin.spans.last_mut() {
                Some(last) if last.start == gpu.start => *last = gpu,
                _ => skin.spans.push(gpu),
            }
        }
        // A span that drew nothing gives way to the next.
        match spans.last_mut() {
            Some(last) if last.start == span.start => *last = span,
            _ => spans.push(span),
        }
    }
}

/// `currentRenderedItemId` of an item (Iris 1.11.7, observed with probe packs): a block item
/// (`block`: its block's `block.properties` id, 0 when the file lacks it) reads that, whatever
/// `item.properties` says of it; any other item its `item.properties` id (`item`), and one the
/// file lacks 65535, or 0 when the pack has no `item.properties` (`item_file`).
pub(in crate::game::client_side) fn rendered_item_id(
    block: Option<i32>,
    item: Option<i32>,
    item_file: bool,
) -> i32 {
    block.or(item).unwrap_or(if item_file { 65535 } else { 0 })
}

/// The block items' `currentRenderedItemId`: for each block's default state, the item it gives
/// (`StateFacts::item`) takes the state's id in `ids`, or 0. A block named as its item wins over
/// another block giving that item.
pub(in crate::game::client_side) fn block_item_ids(
    facts: &sunrise_items::blocks::BlockFacts,
    registry: &sunrise_world::registry::BlockRegistry,
    ids: &BlockIds,
) -> HashMap<String, i32> {
    let mut out = HashMap::new();
    for (i, state) in facts.states.iter().enumerate() {
        let id = sunrise_world::registry::BlockStateId(i as u32);
        if &*state.item == "minecraft:air" || registry.default_state(&state.name) != Some(id) {
            continue;
        }
        let value = registry
            .state(id)
            .and_then(|s| ids.id(&s.name, |k| s.properties.get(k).map(String::as_str)))
            .unwrap_or(0);
        if state.item == state.name {
            out.insert(state.item.to_string(), value);
        } else {
            out.entry(state.item.to_string()).or_insert(value);
        }
    }
    out
}

/// Spans for a native mod (`native_render::RenderNeeds::entity_spans`), without a pack.
pub(in crate::game::client_side) fn native_spans() -> bool {
    sunrise_client_render::render_slots::needs().entity_spans
}

/// A span's id for entity type `name` without a pack: stable, so a mod can match runs from one
/// frame to the next.
pub(in crate::game::client_side) fn name_id(name: &str) -> i32 {
    // FNV-1a.
    let mut hash: u32 = 0x811c_9dc5;
    for byte in name.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    hash as i32
}

/// Model/fluid material ids by the original state, independent of sprite sharing and
/// multipart model selection. Waterlogged model faces retain their model id; emitted
/// fluid faces use the fluid's state (the source state itself for liquid blocks).
pub(in crate::game::client_side) fn terrain_state_ids(
    registry: &sunrise_world::registry::BlockRegistry,
    ids: &BlockIds,
) -> Vec<i32> {
    use sunrise_world::{
        fluid::{FluidKind, FluidState},
        registry::BlockStateId,
    };
    let material = |state: &sunrise_world::chunk::BlockState| {
        ids.id(&state.name, |k| state.properties.get(k).map(String::as_str))
            .unwrap_or(-1)
    };
    (0..registry.len())
        .flat_map(|i| {
            let Some(state) = registry.state(BlockStateId(i as u32)) else {
                return [-1, -1];
            };
            let fluid = FluidState::from_block(state).unwrap_or(FluidState::EMPTY);
            let name = match fluid.kind() {
                FluidKind::Water | FluidKind::FlowingWater => "minecraft:water",
                FluidKind::Lava | FluidKind::FlowingLava => "minecraft:lava",
                FluidKind::Empty => return [material(state), -1],
                FluidKind::Mod(_) | FluidKind::ModFlowing(_) => {
                    return [material(state), material(state)];
                }
            };
            let fluid = if state.name == name {
                Some(state)
            } else {
                registry
                    .default_state(name)
                    .and_then(|id| registry.state(id))
            };
            [material(state), fluid.map_or(-1, material)]
        })
        .collect()
}

#[cfg(test)]
pub(in crate::game) mod terrain_source_tests {
    use crate::game::client_side::shader_packs::*;

    #[test]
    fn terrain_source_cpu_state_filters_fluid_owners_and_pack_reload() {
        let rows = serde_json::json!({"defaults":{"minecraft:air":0,"fixture:signal":1,"minecraft:water":3,"minecraft:lava":5},"states":[
            ["",true,false,"","minecraft:air"],
            ["",false,false,"",{"id":"fixture:signal","properties":{"lit":"false","waterlogged":"false"}}],
            ["",false,true,"",{"id":"fixture:signal","properties":{"lit":"true","waterlogged":"true"}}],
            ["",false,true,"",{"id":"minecraft:water","properties":{"level":"0"}}],
            ["",false,true,"",{"id":"minecraft:water","properties":{"level":"1"}}],
            ["",false,true,"",{"id":"minecraft:lava","properties":{"level":"0"}}]
        ]});
        let registry =
            sunrise_world::registry::BlockRegistry::from_facts_json(rows.to_string().as_bytes())
                .unwrap();
        let parse = |text| BlockIds::parse(&Properties::parse(text, &Macros::standard()));
        let ids = parse(
            "block.31=fixture:signal:lit=false\nblock.47=fixture:signal:lit=true\nblock.59=minecraft:water:level=0\nblock.61=minecraft:water:level=1\nblock.73=minecraft:lava\n",
        );
        assert_eq!(
            terrain_state_ids(&registry, &ids),
            [-1, -1, 31, -1, 47, 59, 59, 59, 61, 61, 73, 73]
        );
        let changed = parse("block.211=fixture:signal:lit=true\nblock.223=minecraft:water\n");
        assert_eq!(
            terrain_state_ids(&registry, &changed),
            [-1, -1, -1, -1, 211, 223, 223, 223, 223, 223, -1, -1]
        );
        assert_eq!(terrain_state_ids(&registry, &BlockIds::default()), [-1; 12]);

        let pack = |ids: &BlockIds| PackIds {
            terrain_states: terrain_state_ids(&registry, ids),
            ..Default::default()
        };
        let signal = pack(&ids);
        // The model's id, not the fluid's: state 3 is water level 0, state 4 level 1.
        assert_eq!(signal.block_state_id(1), Some(31));
        assert_eq!(signal.block_state_id(2), Some(47));
        assert_eq!(signal.block_state_id(3), Some(59));
        assert_eq!(signal.block_state_id(4), Some(61));
        assert_eq!(signal.block_state_id(5), Some(73));
        assert_eq!(signal.block_state_id(0), None);
        assert_eq!(signal.block_state_id(6), None);
        assert_eq!(signal.block_state_id(usize::MAX), None);
        assert_eq!(pack(&BlockIds::default()).block_state_id(1), None);
        assert_eq!(PackIds::default().block_state_id(1), None);
    }

    #[test]
    fn running_block_state_id_needs_a_running_iris_pack() {
        let ids = PackIds {
            terrain_states: vec![-1, -1, 31, -1],
            ..Default::default()
        };
        assert_eq!(
            running_block_state_id(true, Some("iris"), Some(&ids), 1),
            Some(31)
        );
        // Loaded but suppressed (title screen) or not yet rendering.
        assert_eq!(
            running_block_state_id(false, Some("iris"), Some(&ids), 1),
            None
        );
        assert_eq!(running_block_state_id(true, None, Some(&ids), 1), None);
        assert_eq!(
            running_block_state_id(true, Some("native"), Some(&ids), 1),
            None
        );
        assert_eq!(running_block_state_id(true, Some("iris"), None, 1), None);
    }
}

#[cfg(test)]
pub(in crate::game) mod rendered_item_tests {
    use super::rendered_item_id;

    /// Iris 1.11.7's values (probe packs of ours): a block item reads its block's id, and 0 when
    /// `block.properties` lacks it even if `item.properties` names it (oak planks as item 15);
    /// other items read their `item.properties` id, or 65535 when the file lacks them, and 0
    /// when the pack has no such file.
    #[test]
    fn a_held_item_reads_its_block_id_or_item_id() {
        // Stone: block 21 and item 12.
        assert_eq!(rendered_item_id(Some(21), Some(12), true), 21);
        assert_eq!(rendered_item_id(Some(0), Some(15), true), 0);
        assert_eq!(rendered_item_id(Some(0), None, true), 0);
        assert_eq!(rendered_item_id(Some(0), None, false), 0);
        assert_eq!(rendered_item_id(None, Some(11), true), 11);
        assert_eq!(rendered_item_id(None, None, true), 65535);
        assert_eq!(rendered_item_id(None, None, false), 0);
    }
}

#[cfg(test)]
pub(in crate::game) mod native_pack_tests {
    use super::{
        PickHdr, active_pack, loaded_properties, pack_shape, reconcile_pack_hdr, startup_pack_shape,
    };
    use sunrise_mod_package::mod_list::{Installed, ModList, NativePage};
    use sunrise_shaders::Tonemap;

    #[test]
    fn unknown_pack_is_optimistic_but_refusal_owner_and_presence_override_it() {
        use crate::game::native_shaders::{fingerprint, linear_frame_switch, remember};
        use sunrise_client::settings::Settings;
        let mut settings = Settings::default();
        settings.sunrise.hdr = sunrise_client_render::graphics::HdrMode::On;
        settings.mods.installed.push(Installed {
            id: "frame_owner".into(),
            title: "Frame Owner".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(NativePage::default()),
        });
        let root =
            std::env::temp_dir().join(format!("sunrise-frame-policy-{}", std::process::id()));
        let path = root.join("native-frame_owner");
        std::fs::create_dir_all(path.join("shaders")).unwrap();
        settings.sunrise.shader_pack = path.to_string_lossy().into_owned();
        let switch = linear_frame_switch();
        // A pack nobody answered for draws; it asks no float frame.
        let shape = startup_pack_shape(&settings, &mut PickHdr::default());
        assert!(shape.draws_flat && shape.starts_wide);
        assert!(!shape.hdr_float);
        remember(&path, fingerprint(&path), true);
        assert!(!startup_pack_shape(&settings, &mut PickHdr::default()).hdr_float);
        // Refused, the frame is the one no pack asks for.
        remember(&path, fingerprint(&path), false);
        let shape = startup_pack_shape(&settings, &mut PickHdr::default());
        assert_eq!(shape.draws_flat, !switch);
        assert_eq!(shape.hdr_float, switch);
        remember(&path, fingerprint(&path), true);
        // Its mod off, it does not run.
        settings.mods.disabled.insert("frame_owner".into());
        let shape = startup_pack_shape(&settings, &mut PickHdr::default());
        assert!(!shape.draws_flat && !shape.starts_wide);
        assert_eq!(shape.hdr_float, switch);
        settings.mods.disabled.clear();
        assert!(startup_pack_shape(&settings, &mut PickHdr::default()).draws_flat);
        // Gone, it does not run under the switch.
        std::fs::remove_dir_all(root).unwrap();
        let missing = startup_pack_shape(&settings, &mut PickHdr::default());
        assert_eq!(missing.draws_flat, !switch);
        assert_eq!(missing.hdr_float, switch);
    }

    /// A pack folder under a fresh `root` with `properties` as its `shaders.properties`.
    fn hdr_pack(root: &std::path::Path, name: &str, properties: &str) -> std::path::PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(path.join("shaders")).unwrap();
        std::fs::write(path.join("shaders/shaders.properties"), properties).unwrap();
        path
    }

    fn hdr_settings() -> sunrise_client::settings::Settings {
        let mut settings = sunrise_client::settings::Settings::default();
        settings.sunrise.hdr = sunrise_client_render::graphics::HdrMode::On;
        settings
    }

    /// `pack_shape` for the pick `path`, with `state` as the game's kept request.
    fn shape_of(
        settings: &mut sunrise_client::settings::Settings,
        state: &mut PickHdr,
        path: &std::path::Path,
        switch: bool,
    ) -> (bool, Option<Tonemap>) {
        settings.sunrise.shader_pack = path.to_string_lossy().into_owned();
        let shape = pack_shape(settings, switch, state);
        assert!(shape.draws_flat, "{path:?} runs");
        (shape.hdr_float, shape.hdr)
    }

    /// Under `SUNRISE_HDR_FRAME=float`, a running pack whose `shaders.properties` asks for
    /// Sunrise's HDR (with its options as chosen) gets the float frame; one that doesn't gets
    /// today's pack frame, and without the switch nothing is read.
    #[test]
    fn a_pack_asking_for_hdr_gets_the_float_frame_under_the_switch() {
        let root = std::env::temp_dir().join(format!("sunrise-hdr-request-{}", std::process::id()));
        let plain = hdr_pack(&root, "plain", "separateAo=true\n");
        let asks = hdr_pack(&root, "asks", "sunrise.hdr = true\n");
        let curve = hdr_pack(
            &root,
            "curve",
            "sunrise.hdr = true\nsunrise.hdr.tonemap = sunrise\n",
        );
        let unknown = hdr_pack(&root, "unknown", "sunrise.hdr = on\n");
        let mut settings = hdr_settings();
        let mut state = PickHdr::default();
        let mut shape = |path: &std::path::Path, switch: bool| {
            shape_of(&mut settings, &mut state, path, switch)
        };
        assert_eq!(shape(&asks, false), (false, None));
        assert_eq!(shape(&plain, true), (false, None));
        assert_eq!(shape(&unknown, true), (false, None));
        assert_eq!(shape(&curve, true), (true, Some(Tonemap::Sunrise)));
        assert_eq!(shape(&asks, true), (true, Some(Tonemap::Pack)));
        // Without the switch nothing is read: the kept read is still `asks`'.
        assert_eq!(shape(&plain, false), (false, None));
        std::fs::write(asks.join("shaders/shaders.properties"), "x=1\n").unwrap();
        assert_eq!(shape(&asks, true), (true, Some(Tonemap::Pack)));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The kept request is read again when the pick changes and at each boundary that can
    /// change the pick's files or options (`PickHdr::forget`), not before: the same pack replaced
    /// under its name, and its options saved, give the new answer there.
    #[test]
    fn the_kept_hdr_request_is_read_again_at_its_boundaries() {
        let root = std::env::temp_dir().join(format!("sunrise-hdr-kept-{}", std::process::id()));
        let asks = hdr_pack(&root, "asks", "sunrise.hdr = true\n");
        let plain = hdr_pack(&root, "plain", "separateAo=true\n");
        let mut settings = hdr_settings();
        let mut state = PickHdr::default();
        let mut shape = |state: &mut PickHdr, path: &std::path::Path| {
            shape_of(&mut settings, state, path, true)
        };
        // The pick turned off, as the toggle does: no pack runs, under the switch.
        let settings_off = |state: &mut PickHdr| {
            let mut off = hdr_settings();
            off.sunrise.shader_pack.clear();
            assert!(!pack_shape(&off, true, state).draws_flat);
        };
        assert_eq!(shape(&mut state, &asks), (true, Some(Tonemap::Pack)));
        // Replaced under the same name: kept until a boundary, read there.
        std::fs::write(
            asks.join("shaders/shaders.properties"),
            "sunrise.hdr=false\n",
        )
        .unwrap();
        assert_eq!(shape(&mut state, &asks), (true, Some(Tonemap::Pack)));
        state.forget();
        assert_eq!(shape(&mut state, &asks), (false, None));
        // Another pick reads its own; back to this one, it is read again.
        std::fs::write(
            asks.join("shaders/shaders.properties"),
            "sunrise.hdr=true\n",
        )
        .unwrap();
        assert_eq!(shape(&mut state, &plain), (false, None));
        assert_eq!(shape(&mut state, &asks), (true, Some(Tonemap::Pack)));

        // Turned off (the toggle's empty pick), replaced under its name, turned on again: read
        // again.
        settings_off(&mut state);
        std::fs::write(asks.join("shaders/shaders.properties"), "x=1\n").unwrap();
        assert_eq!(shape(&mut state, &asks), (false, None));
        std::fs::write(
            asks.join("shaders/shaders.properties"),
            "sunrise.hdr=true\n",
        )
        .unwrap();
        settings_off(&mut state);
        assert_eq!(shape(&mut state, &asks), (true, Some(Tonemap::Pack)));

        // Behind the pack's own switch, off by default: asked once the saved options turn it
        // on, and off again when they turn it off, each at the save.
        let option = hdr_pack(
            &root,
            "option",
            "#ifdef HDR_OUTPUT\nsunrise.hdr=true\n#endif\n",
        );
        std::fs::write(
            option.join("shaders/settings.glsl"),
            "//#define HDR_OUTPUT\n#ifdef HDR_OUTPUT\n#endif\n",
        )
        .unwrap();
        assert_eq!(shape(&mut state, &option), (false, None));
        let chosen = root.join("option.txt");
        std::fs::write(&chosen, "HDR_OUTPUT=true\n").unwrap();
        assert_eq!(shape(&mut state, &option), (false, None));
        state.forget();
        assert_eq!(shape(&mut state, &option), (true, Some(Tonemap::Pack)));
        std::fs::write(&chosen, "HDR_OUTPUT=false\n").unwrap();
        state.forget();
        assert_eq!(shape(&mut state, &option), (false, None));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The loader's read of the properties (`loaded_properties`, what a load logs), with the
    /// renderer's macros, after the frame was made for the pick's request. Where a renderer
    /// macro changes the request it says the frame's answer counts; an unknown value only the
    /// renderer's macros reach is noted once, with its line, key and value, and one the frame's
    /// read already noted is not noted again. Without the switch it logs what the file's own
    /// notes are, as before. The load reads the pack's files again at the next frame.
    #[test]
    fn the_loader_reads_the_request_again_with_the_renderers_macros() {
        use sunrise_shaders::{Macros, Pack};
        let root = std::env::temp_dir().join(format!("sunrise-hdr-loader-{}", std::process::id()));
        let renderer_curve = hdr_pack(
            &root,
            "renderer_curve",
            "sunrise.hdr=true\n#ifdef MC_GL_RENDERER_OTHER\nsunrise.hdr.tonemap=sunrise\n#endif\n",
        );
        let renderer_bad = hdr_pack(
            &root,
            "renderer_bad",
            "sunrise.hdr=true\n#ifdef MC_GL_RENDERER_OTHER\nsunrise.hdr.tonemap=maybe\n#endif\n",
        );
        let bad = hdr_pack(&root, "bad", "a=1\nsunrise.hdr = yes\n");
        let mut settings = hdr_settings();
        let mut state = PickHdr::default();
        let load = |state: &mut PickHdr, path: &std::path::Path, switch: bool, frame| {
            let pack = Pack::open(path).unwrap();
            let mut macros = Macros::standard();
            macros.define_renderer(4);
            let name = path.to_string_lossy();
            loaded_properties(&pack, &mut macros, switch, frame, &name, state).1
        };
        let names_value = |line: &str, at: u32, key: &str, value: &str| {
            line.starts_with(&format!("shaders.properties:{at}: "))
                && line.contains(&format!("`{key}`"))
                && line.contains(&format!("`{value}`"))
        };

        // The frame is made for the standard macros' answer: the pack's curve.
        let frame = shape_of(&mut settings, &mut state, &renderer_curve, true).1;
        assert_eq!(frame, Some(Tonemap::Pack));
        let warned = load(&mut state, &renderer_curve, true, frame);
        assert_eq!(
            warned,
            [
                "shaders.properties asks for Sunrise's HDR as Some(Sunrise) with the renderer's \
                 macros, but the frame was made for Some(Pack): the frame's answer counts"
            ]
        );
        assert!(load(&mut state, &renderer_curve, false, frame).is_empty());
        // A frame made for the loader's answer: nothing to say.
        assert!(load(&mut state, &renderer_curve, true, Some(Tonemap::Sunrise)).is_empty());

        // An unknown value behind a renderer macro: the frame's read can't see it, the load
        // notes it, once.
        let frame = shape_of(&mut settings, &mut state, &renderer_bad, true).1;
        assert_eq!(frame, Some(Tonemap::Pack));
        let noted = load(&mut state, &renderer_bad, true, frame);
        assert_eq!(noted.len(), 2, "{noted:?}");
        assert!(names_value(&noted[0], 3, "sunrise.hdr.tonemap", "maybe"));
        assert_eq!(load(&mut state, &renderer_bad, true, frame).len(), 1);
        assert!(load(&mut state, &renderer_bad, false, frame).is_empty());

        // One the frame's read noted is not noted again by the load.
        let frame = shape_of(&mut settings, &mut state, &bad, true).1;
        assert_eq!(frame, None);
        assert!(load(&mut state, &bad, true, frame).is_empty());

        // Without the switch the load logs what it logged before: each required feature
        // Sunrise lacks, then each note of the file, in that order; with it, the request's note
        // after them.
        let others = hdr_pack(
            &root,
            "others",
            "iris.features.required = SSBO NO_SUCH_FEATURE\nnot a pair\n\
             sunrise.hdr = maybe\n",
        );
        let expected = [
            "the pack requires `NO_SUCH_FEATURE`, which is not supported yet",
            "shaders.properties:2: not a key=value line",
        ];
        assert_eq!(load(&mut state, &others, false, None), expected);
        let with_switch = load(&mut state, &others, true, None);
        assert_eq!(with_switch[..2], expected);
        assert_eq!(with_switch.len(), 3, "{with_switch:?}");
        assert!(names_value(&with_switch[2], 3, "sunrise.hdr", "maybe"));

        // The load reads the files again: the pack replaced under its name gives its new
        // request at the next frame.
        let frame = shape_of(&mut settings, &mut state, &renderer_curve, true);
        assert_eq!(frame, (true, Some(Tonemap::Pack)));
        std::fs::write(
            renderer_curve.join("shaders/shaders.properties"),
            "separateAo=true\n",
        )
        .unwrap();
        assert_eq!(
            shape_of(&mut settings, &mut state, &renderer_curve, true),
            frame
        );
        load(&mut state, &renderer_curve, true, frame.1);
        assert_eq!(
            shape_of(&mut settings, &mut state, &renderer_curve, true),
            (false, None)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A frame that keeps serving takes the pick's new request; one made again keeps the one
    /// it was made for until the new one is made (`build_renderer` sets it then).
    #[test]
    fn a_kept_frame_takes_the_new_request_and_a_rebuild_keeps_the_old() {
        let mut installed = None;
        assert!(!reconcile_pack_hdr(
            true,
            &mut installed,
            Some(Tonemap::Pack)
        ));
        assert_eq!(installed, Some(Tonemap::Pack));
        assert!(!reconcile_pack_hdr(
            true,
            &mut installed,
            Some(Tonemap::Sunrise)
        ));
        assert_eq!(installed, Some(Tonemap::Sunrise));
        assert!(reconcile_pack_hdr(false, &mut installed, None));
        assert_eq!(installed, Some(Tonemap::Sunrise));
        assert!(!reconcile_pack_hdr(true, &mut installed, None));
        assert_eq!(installed, None);
    }

    #[test]
    fn native_shader_follows_its_mod_without_affecting_other_packs() {
        let mut mods = ModList::default();
        mods.installed.push(Installed {
            id: "sunrise_better_visuals".into(),
            title: "Better Visuals".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(NativePage::default()),
        });
        let owned = "native-sunrise_better_visuals";
        assert_eq!(active_pack(owned, &mods), owned);
        mods.disabled.insert("sunrise_better_visuals".into());
        assert_eq!(active_pack(owned, &mods), "");
        assert_eq!(active_pack("golden_hour", &mods), "golden_hour");
        mods.installed[0].native = None;
        assert_eq!(active_pack(owned, &mods), owned);
    }
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn projectile_body_uses_its_own_shader_entity_id() {
    use sunrise_sim::projectiles::{Projectile, ProjectileKind};

    super::native_entity_refs::tests::with_client_game(|game| {
        game.client.pack_ids = Some(PackIds::with_entity_ids([
            ("minecraft:snowball", 41),
            ("minecraft:ominous_item_spawner", 42),
            ("minecraft:arrow", 43),
            ("minecraft:spectral_arrow", 44),
        ]));
        let arrow = |effect| ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect,
        };
        for kind in [
            ProjectileKind::Snowball,
            arrow(None),
            arrow(Some(("minecraft:glowing", 200))),
        ] {
            game.creatures.projectiles.push(Projectile::new(
                kind,
                sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
                None,
            ));
        }
        game.entity_geometry(
            glam::DVec3::new(0.5, 64.0, -4.5),
            1.0,
            None,
            Vec::new(),
            &mut Vec::new(),
        );
        let ids: Vec<_> = game
            .client
            .entity_spans
            .borrow()
            .windows(2)
            .filter(|span| span[0].start < span[1].start && (41..=44).contains(&span[0].id))
            .map(|span| span[0].id)
            .collect();
        assert_eq!(ids, [41, 43, 44]);
    });
}

#[cfg(test)]
pub(in crate::game) mod toggle_tests {
    use super::{TogglePlan, plan_shader_toggle};
    use sunrise_client::settings::Settings;

    fn plan(pick: &str, last: &str, present: bool, active: bool) -> String {
        match plan_shader_toggle(pick, last, false, |_| present, |_| active) {
            TogglePlan::Off(name) => format!("off {name}"),
            TogglePlan::On(name) => format!("on {name}"),
            TogglePlan::HintModOff(name) => format!("mod-off {name}"),
            TogglePlan::HintClearLast => "hint-clear".into(),
            TogglePlan::Hint => "hint".into(),
            TogglePlan::Noop => "noop".into(),
        }
    }

    /// The key turns the picked pack off and remembers it, resumes it, and hints when there
    /// is nothing to resume; a removed pack clears the memory instead of failing, and a pack
    /// whose owning mod is off is reported instead of announced.
    #[test]
    fn the_toggle_turns_off_resumes_and_hints() {
        assert_eq!(plan("A.zip", "", true, true), "off A.zip");
        assert_eq!(plan("", "A.zip", true, true), "on A.zip");
        assert_eq!(plan("", "", true, true), "hint");
        assert_eq!(plan("", "A.zip", false, true), "hint-clear");
        assert_eq!(
            plan("", "native-frame_owner", true, false),
            "mod-off native-frame_owner"
        );
    }

    /// The developer override wins over the pick: the key does nothing.
    #[test]
    fn the_developer_override_makes_the_toggle_a_noop() {
        for (pick, last) in [("A.zip", ""), ("", "A.zip"), ("", "")] {
            assert!(
                matches!(
                    plan_shader_toggle(pick, last, true, |_| true, |_| true),
                    TogglePlan::Noop
                ),
                "pick {pick:?}, last {last:?}"
            );
        }
    }

    /// One shared pick: the menu's choice and the key's write the same fields, so turning
    /// off by either keeps the last non-empty pick for the key to resume.
    #[test]
    fn picking_and_unpicking_share_the_picks_memory() {
        let mut s = Settings::default();
        let pick_of = |s: &Settings| {
            (
                s.sunrise.shader_pack.clone(),
                s.extras.last_shader_pack.clone(),
            )
        };
        s.set_shader_pick("A.zip");
        assert_eq!(pick_of(&s), ("A.zip".into(), "A.zip".into()));
        // Picking another pack while turned off by the key forgets the older one.
        s.set_shader_pick("");
        s.set_shader_pick("B.zip");
        assert_eq!(pick_of(&s), ("B.zip".into(), "B.zip".into()));
        // Unpicking by hand keeps the memory, as the key's turning off does.
        s.set_shader_pick("");
        assert_eq!(pick_of(&s), ("".into(), "B.zip".into()));
    }

    /// A rapid double press ends where a single one does: off keeps the memory, on takes it
    /// back, and the loader (not the key) owns reinstalling the pack.
    #[test]
    fn a_rapid_double_press_restores_the_pick() {
        let mut s = Settings::default();
        s.set_shader_pick("sunrise:warm");
        s.set_shader_pick("");
        assert_eq!(
            (
                s.sunrise.shader_pack.as_str(),
                s.extras.last_shader_pack.as_str()
            ),
            ("", "sunrise:warm")
        );
        s.set_shader_pick(&s.extras.last_shader_pack.clone());
        assert_eq!(
            (
                s.sunrise.shader_pack.as_str(),
                s.extras.last_shader_pack.as_str()
            ),
            ("sunrise:warm", "sunrise:warm")
        );
    }
}

use sunrise_gameplay_common::game::weather::WeatherGameExt as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
