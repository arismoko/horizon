//! Client-only mob presentation tables, entity-model loading, and non-sim render helpers.

pub use crate::mobs::{
    ModelNormals, Part, Polygon, cem, cem_anim, model_layers, random_textures, render, skin,
    villager_hats,
};

use crate::fast_hash::FxHashMap;
use crate::mesher::Vertex;
use crate::mobs::{Mob, anim};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::f32::consts::PI;
use std::fs;
use std::path::Path;
use std::sync::{Arc, RwLock};
use sunrise_assets::resource_source::ResourceRoot;
use sunrise_core::geometry::trig::Mth;

/// The models that draw with `RenderTypes::entityCutoutCull`.
fn culls_back_faces(key: &str) -> bool {
    matches!(key, "minecraft:bat#main" | "minecraft:turtle_baby#main")
}

/// Client-only visuals removed from `mobs::MobType`, keyed by `MobType.id` inside `ClientMobModels`.
#[derive(Clone, Debug)]
pub struct ClientMobVisuals {
    /// Resolved content renderer shadow; `None` uses the built-in shadow table (`shadows::shadow_params`).
    pub shadow: Option<(f32, f32)>,
    pub model: Arc<Part>,
    /// Every baked layer of this entity's models keyed by `model#layer`
    /// (e.g. `sheep#wool`, `zombie_baby#main`, `player_slim#main`, `player#cape`, `player#ears`).
    pub layers: FxHashMap<String, Arc<Part>>,
    pub texture: String,
}

impl ClientMobVisuals {
    pub fn layer(&self, key: &str) -> Option<&Arc<Part>> {
        self.layers.get(key)
    }

    /// Exact max absolute scale axis of `self.model.pose[6..]`, folded from `1.0`
    /// (e.g. `2.35` for `minecraft:elder_guardian#main`, `1.0` for unscaled models).
    pub fn root_scale(&self) -> f32 {
        self.model.pose[6..]
            .iter()
            .fold(1.0_f32, |scale, axis| scale.max(axis.abs()))
    }
}

/// Client-only lookup table parallel to `HashMap<String, Arc<MobType>>`.
/// Never constructed on a dedicated server.
#[derive(Clone, Debug, Default)]
pub struct ClientMobModels {
    pub types: HashMap<String, ClientMobVisuals>,
}

impl ClientMobModels {
    pub fn load(root: &Path) -> crate::Result<Self> {
        let document = EntityModelDocument::load(root)?;
        Self::load_with_document(root, &document)
    }

    /// Parses `entity-types.json` + `document` using `mobs::species_entries`, populates vanilla
    /// species and content-mod template species whose main model key exists in `document`
    /// (applying `culls_back_faces` and `shadows::content_shadow`), and inserts `"minecraft:player"`
    /// via `player_visuals(document)` when `minecraft:player#main` exists.
    pub fn load_with_document(root: &Path, document: &EntityModelDocument) -> crate::Result<Self> {
        let bytes = fs::read(root.join("entity-types.json"))?;
        let types: serde_json::Map<String, Value> = serde_json::from_slice(&bytes)?;
        let mut out = HashMap::new();
        for (id, def, t) in crate::mobs::species_entries(&types) {
            let model_key = format!("minecraft:{}", def.model);
            let Some(mut part) = document.model(&model_key) else {
                continue;
            };
            let mut layers = FxHashMap::default();
            for prefix in def.layer_prefixes() {
                let start = format!("minecraft:{prefix}#");
                for (k, v) in document.0.iter().filter(|(k, _)| k.starts_with(&start)) {
                    let mut layer = v.clone();
                    if culls_back_faces(k) {
                        layer.set_cull_back();
                    }
                    layers.insert(
                        k.trim_start_matches("minecraft:").to_owned(),
                        Arc::new(layer),
                    );
                }
            }
            if culls_back_faces(&model_key) {
                part.set_cull_back();
            }
            out.insert(
                id,
                ClientMobVisuals {
                    shadow: crate::shadows::content_shadow(t),
                    model: Arc::new(part),
                    layers,
                    texture: def.texture.to_owned(),
                },
            );
        }
        if let Some(player) = Self::player_visuals(document) {
            out.insert("minecraft:player".to_owned(), player);
        }
        Ok(Self { types: out })
    }

    /// Builds the `"minecraft:player"` entry (`player#main`, layers `player_slim#main`,
    /// `player#cape`, `player#ears`, texture `"entity/player/wide/steve"`, `shadow: None`)
    /// when `minecraft:player#main` exists in `document`.
    pub fn player_visuals(document: &EntityModelDocument) -> Option<ClientMobVisuals> {
        let model = Arc::new(document.model("minecraft:player#main")?);
        let layers = ["player_slim#main", "player#cape", "player#ears"]
            .into_iter()
            .filter_map(|key| {
                let part = document.model(&format!("minecraft:{key}"))?;
                Some((key.to_owned(), Arc::new(part)))
            })
            .collect();
        Some(ClientMobVisuals {
            shadow: None,
            model,
            layers,
            texture: "entity/player/wide/steve".into(),
        })
    }

    pub fn get(&self, id: &str) -> Option<&ClientMobVisuals> {
        self.types.get(id)
    }

    pub fn model(&self, id: &str) -> Option<&Arc<Part>> {
        self.get(id).map(|visuals| &visuals.model)
    }

    pub fn layer(&self, id: &str, key: &str) -> Option<&Arc<Part>> {
        self.get(id).and_then(|visuals| visuals.layer(key))
    }

    /// Scans all loaded entries for a baked layer key.
    pub fn layer_any(&self, key: &str) -> Option<&Arc<Part>> {
        self.types.values().find_map(|visuals| visuals.layer(key))
    }

    pub fn texture(&self, id: &str) -> Option<&str> {
        self.get(id).map(|visuals| visuals.texture.as_str())
    }

    pub fn shadow(&self, id: &str) -> Option<(f32, f32)> {
        self.get(id).and_then(|visuals| visuals.shadow)
    }

    /// `self.get(id).map(ClientMobVisuals::root_scale)` — returns `None` when the visual is absent.
    pub fn root_scale(&self, id: &str) -> Option<f32> {
        self.get(id).map(ClientMobVisuals::root_scale)
    }

    pub fn insert(&mut self, id: String, visuals: ClientMobVisuals) {
        self.types.insert(id, visuals);
    }
}

/// Client-only presentation tables removed from `mobs::MobData`.
/// Never constructed on a dedicated server.
#[derive(Default)]
pub struct ClientMobData {
    pub animations: HashMap<String, crate::mobs::keyframes::Animation>,
    /// Baked vanilla frame layouts.
    pub frame_models: render::BakedFrameModels,
    /// Replaced when the resource packs change.
    pub villager_hats: RwLock<villager_hats::VillagerHats>,
    /// The packs' custom entity models (OptiFine CEM); replaced when the resource packs change.
    pub cem: RwLock<Arc<cem::CemModels>>,
    /// The packs' random entity textures; replaced when the resource packs change.
    pub random_textures: RwLock<sunrise_optifine::random_textures::RandomTextures>,
    /// The packs' emissive overlays of entity textures; replaced when the resource packs change.
    pub emissive: RwLock<Arc<sunrise_optifine::emissive::Emissive>>,
    /// ETF's blinking textures; replaced when the resource packs change.
    pub blinks: RwLock<sunrise_optifine::blink::Blinks>,
}

impl ClientMobData {
    /// Loads client keyframe animations, resource-pack entity overrides, CEM models, and
    /// baked vanilla frame layouts as one unit.
    pub fn load(root: &ResourceRoot, models: &ClientMobModels) -> crate::Result<Self> {
        let source = root.source();
        let prepared = root.prepared();
        let animations = Self::load_animations(prepared)?;
        let villager_hats = RwLock::new(villager_hats::VillagerHats::load(source));
        let random = sunrise_optifine::random_textures::RandomTextures::scan(source);
        let blinks = sunrise_optifine::blink::Blinks::scan(source, &random);
        let mut emissive = sunrise_optifine::emissive::Emissive::scan(source, &random);
        emissive.pair_beside(source, blinks.files());
        let cem = RwLock::new(Arc::new(cem::CemModels::load(source, models)));
        let frame_models = render::BakedFrameModels::bake(models.types.values());
        Ok(Self {
            animations,
            frame_models,
            villager_hats,
            cem,
            random_textures: RwLock::new(random),
            emissive: RwLock::new(Arc::new(emissive)),
            blinks: RwLock::new(blinks),
        })
    }

    /// Only the client jar's keyframe animation definitions.
    pub fn load_animations(
        root: &Path,
    ) -> std::io::Result<HashMap<String, crate::mobs::keyframes::Animation>> {
        crate::mobs::keyframes::load_animations(root)
    }
}

/// Standalone `RenderDataSource` borrowing `ClientMobModels` and `ClientMobData`
/// (used by tests, probes, and baseline draws without `Game`).
#[derive(Clone, Copy)]
pub struct ClientMobRender<'a> {
    pub models: &'a ClientMobModels,
    pub data: &'a ClientMobData,
}

impl render::RenderDataSource for ClientMobRender<'_> {
    fn mob_tables(&self) -> ClientMobRender<'_> {
        *self
    }
}

/// Model geometry for a mob, posed for its animation state, as camera-relative
/// vertices textured from the shared atlas.
pub fn render_single_mob(
    mob: &Mob,
    models: &ClientMobModels,
    alpha: f32,
    camera: glam::DVec3,
    uv: [f32; 4],
    light: u32,
    out: &mut Vec<Vertex>,
) {
    render::render_single(mob, models, alpha, camera, uv, light, out);
}

/// A parsed `entity-models.json` document, shared while loading model batches.
/// Each lookup still constructs an independent part tree for the caller.
#[derive(Clone)]
pub struct EntityModelDocument(Arc<BTreeMap<String, Part>>);

#[derive(Debug)]
pub(crate) enum EntityModelValidationError {
    NotNonemptyObject(String),
    InvalidLayer { id: String, detail: String },
}

impl std::fmt::Display for EntityModelValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotNonemptyObject(detail) | Self::InvalidLayer { detail, .. } => {
                f.write_str(detail)
            }
        }
    }
}

impl std::error::Error for EntityModelValidationError {}

impl EntityModelDocument {
    pub fn load(root: &Path) -> crate::Result<Self> {
        let path = root.join("entity-models.json");
        let layers: Value = serde_json::from_slice(&fs::read(&path)?)?;
        let Value::Object(layers) = layers else {
            return Err(EntityModelValidationError::NotNonemptyObject(format!(
                "{}: expected a top-level JSON object",
                path.display()
            ))
            .into());
        };
        // EntityModelSet.vanilla builds a populated set from LayerDefinitions.createRoots.
        if layers.is_empty() {
            return Err(EntityModelValidationError::NotNonemptyObject(format!(
                "{}: no entity model definitions",
                path.display()
            ))
            .into());
        }
        let mut models = BTreeMap::new();
        for (key, model) in layers {
            if !model_layers::is_layer_id(&key) {
                return Err(EntityModelValidationError::InvalidLayer {
                    detail: format!("{}: invalid model layer key {key:?}", path.display()),
                    id: key,
                }
                .into());
            }
            let mut part = Part::decode("root", model).map_err(|error| {
                EntityModelValidationError::InvalidLayer {
                    id: key.clone(),
                    detail: format!("{}: layer {key}: {error}", path.display()),
                }
            })?;
            part.layer = Some(key.as_str().into());
            models.insert(key, part);
        }
        Ok(Self(Arc::new(models)))
    }

    pub fn model(&self, key: &str) -> Option<Part> {
        self.0.get(key).cloned()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }
}

/// Load one baked model layer from `entity-models.json` (e.g. `minecraft:chest#main`).
/// Batch loaders should share an `EntityModelDocument` instead.
pub fn load_model(root: &Path, key: &str) -> Option<Part> {
    EntityModelDocument::load(root).ok()?.model(key)
}

/// ModelPart.render: translate, rotate ZYX, scale, then children.
pub fn render_model(
    part: &Part,
    base: glam::Mat4,
    pose_for: &dyn Fn(&Part) -> [f32; 9],
    uv: [f32; 4],
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    render_model_filtered(
        part,
        (base, ModelNormals::new(base)),
        &|p| Some(pose_for(p)),
        uv,
        light,
        overlay,
        out,
    );
}

/// Draw a posed model, skipping a part and its descendants when its pose is `None`. Each part's
/// own geometry goes through the step every baked model's draw takes (`render::compile_part`
/// when a mod replaced it), with the model's layer and empty `Poses`: these draws are posed by
/// `pose_for`, not by `anim`.
pub fn render_model_filtered(
    part: &Part,
    base: (glam::Mat4, ModelNormals),
    pose_for: &impl Fn(&Part) -> Option<[f32; 9]>,
    uv: [f32; 4],
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    let paint = skin::Paint {
        uv,
        scroll: 0.0,
        color: 0xFFFF_FFFF,
        light,
        overlay,
    };
    let compile = render::compiler();
    // Built only for a replacement to read: empty maps allocate nothing.
    let poses = anim::Poses::default();
    let draw = Draw {
        layer: part.layer.as_deref(),
        compile,
        paint,
        poses: &poses,
    };
    visit(
        part,
        base.0,
        base.1,
        pose_for,
        &draw,
        &mut render::Quads::new(out),
    );
    /// What every part of one draw shares.
    struct Draw<'a> {
        layer: Option<&'a str>,
        compile: Option<render::CompilePart>,
        paint: skin::Paint,
        poses: &'a anim::Poses,
    }
    fn visit(
        part: &Part,
        parent: glam::Mat4,
        normals: ModelNormals,
        pose_for: &impl Fn(&Part) -> Option<[f32; 9]>,
        draw: &Draw<'_>,
        quads: &mut render::Quads<'_>,
    ) {
        let Some(p) = pose_for(part) else {
            return;
        };
        let m = parent
            * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
            * glam::Mat4::from_quat(glam::Quat::from_euler(
                glam::EulerRot::ZYX,
                p[5],
                p[4],
                p[3],
            ))
            * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]));
        let normals = normals.part(p);
        render::emit_part(
            draw.compile,
            draw.layer,
            part,
            (m, normals),
            draw.paint,
            draw.poses,
            quads,
        );
        for c in &part.children {
            visit(c, m, normals, pose_for, draw, quads);
        }
    }
}

/// `GuardianRenderer.renderBeam` from `eye` to `target` (world minus
/// camera): the four rotating sides and the end cap. The texture's V
/// repeats along the beam, so the sides are cut at each repeat.
pub fn guardian_beam_geometry(
    eye: glam::Vec3,
    target: glam::Vec3,
    time: f32,
    scale: f32,
    uv: [f32; 4],
    out: &mut Vec<Vertex>,
) {
    let v = target - eye;
    let length = v.length() + 1.0;
    let dir = v.normalize_or_zero();
    let x_rot = dir.y.clamp(-1.0, 1.0).acos();
    let y_rot = std::f32::consts::FRAC_PI_2 - dir.z.atan2(dir.x);
    let pose = glam::Mat4::from_translation(eye)
        * glam::Mat4::from_rotation_y(y_rot)
        * glam::Mat4::from_rotation_x(x_rot);
    let rot = time * 0.05 * -1.5;
    let cs = scale * scale;
    let (r, g, b) = (
        64 + (cs * 191.0) as u32,
        32 + (cs * 191.0) as u32,
        128u32.saturating_sub((cs * 64.0) as u32),
    );
    let color = 0xFF00_0000 | (b.min(255) << 16) | (g.min(255) << 8) | r.min(255);
    let at = |a: f32, radius: f32| (a.mcos() * radius, a.msin() * radius);
    let quarter = std::f32::consts::FRAC_PI_4;
    let (wx, wz) = at(rot + PI, 0.2);
    let (ex, ez) = at(rot, 0.2);
    let (nx, nz) = at(rot + PI / 2.0, 0.2);
    let (sx, sz) = at(rot + PI * 1.5, 0.2);
    let (wnx, wnz) = at(rot + quarter * 3.0, 0.282);
    let (enx, enz) = at(rot + quarter, 0.282);
    let (wsx, wsz) = at(rot + quarter * 5.0, 0.282);
    let (esx, esz) = at(rot + quarter * 7.0, 0.282);
    let tex_off = (time * 0.5) % 1.0;
    let min_v = -1.0 + tex_off;
    let max_v = min_v + length * 2.5;
    let atlas = |u: f32, v: f32| [uv[0] + (uv[2] - uv[0]) * u, uv[1] + (uv[3] - uv[1]) * v];
    let mut push = |p: [f32; 3], t: [f32; 2]| {
        out.push(Vertex {
            normal: [0; 4],
            pos: pose.transform_point3(glam::Vec3::from(p)).to_array(),
            uv: t,
            layer: 0,
            color,
            light: 240 | 240 << 8,

            glint_uv: [0.0, 0.0],
        });
    };
    // Sides: one quad per texture repeat.
    let mut k = min_v.floor();
    while k < max_v {
        let (v0, v1) = (min_v.max(k), max_v.min(k + 1.0));
        let (y0, y1) = (
            (v0 - min_v) / (max_v - min_v) * length,
            (v1 - min_v) / (max_v - min_v) * length,
        );
        let (t0, t1) = (v0 - k, v1 - k);
        for (a, c) in [((wx, wz), (ex, ez)), ((nx, nz), (sx, sz))] {
            push([a.0, y1, a.1], atlas(0.4999, t1));
            push([a.0, y0, a.1], atlas(0.4999, t0));
            push([c.0, y0, c.1], atlas(0.0, t0));
            push([c.0, y1, c.1], atlas(0.0, t1));
        }
        k += 1.0;
    }
    let base = if (time.floor() as i32) % 2 == 0 {
        0.5
    } else {
        0.0
    };
    push([wnx, length, wnz], atlas(0.5, base + 0.5));
    push([enx, length, enz], atlas(1.0, base + 0.5));
    push([esx, length, esz], atlas(1.0, base));
    push([wsx, length, wsz], atlas(0.5, base));
}

#[cfg(test)]
#[path = "mobs/lighting_tests.rs"]
mod lighting_tests;

#[cfg(test)]
mod entity_sounds {
    use crate::mobs::entity_sounds::{Selection, Sounds, declared_step, full_id};
    use std::borrow::Cow;

    mod tests {
        include!("mobs/entity_sounds_tests.rs");
    }
}

#[cfg(test)]
mod model_document_sharing_tests {
    use super::*;
    use serde_json::json;

    fn part_dump(part: &Part) -> String {
        format!("{part:?}")
    }

    /// The startup reads `entity-models.json` once and hands the same document
    /// to every model loader; reading it again per loader must build the same
    /// visuals, model for model and layer for layer.
    #[test]
    fn a_shared_document_builds_the_same_types() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-models.json").exists() {
            return;
        }
        let document = EntityModelDocument::load(&root).unwrap();
        let shared = ClientMobModels::load_with_document(&root, &document).unwrap();
        let independent = ClientMobModels::load(&root).unwrap();
        assert_eq!(shared.types.len(), independent.types.len());
        for (id, independent) in &independent.types {
            let shared = shared.types.get(id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(independent.shadow, shared.shadow, "{id} shadow");
            assert_eq!(independent.texture, shared.texture, "{id} texture");
            assert_eq!(
                part_dump(&independent.model),
                part_dump(&shared.model),
                "{id} model"
            );
            let mut independent_layers: Vec<_> = independent.layers.iter().collect();
            let mut shared_layers: Vec<_> = shared.layers.iter().collect();
            independent_layers.sort_by_key(|(key, _)| key.as_str());
            shared_layers.sort_by_key(|(key, _)| key.as_str());
            assert_eq!(independent_layers.len(), shared_layers.len(), "{id} layers");
            for ((independent_key, independent_part), (shared_key, shared_part)) in
                independent_layers.into_iter().zip(shared_layers)
            {
                assert_eq!(independent_key, shared_key, "{id} layer key");
                assert_eq!(
                    part_dump(independent_part),
                    part_dump(shared_part),
                    "{id} layer {independent_key}"
                );
            }
        }
    }

    #[test]
    fn client_mob_data_load_requires_mob_animations() {
        let folder = tempfile::tempdir().unwrap();
        let root = ResourceRoot::directory(folder.path().to_path_buf());
        assert!(ClientMobData::load(&root, &ClientMobModels::default()).is_err());
    }

    #[test]
    fn content_template_visuals_inherit_template_texture_and_model() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(
            folder.path().join("entity-types.json"),
            serde_json::to_vec(&json!({
                "minecraft:skeleton": {"width": 0.6, "height": 1.99},
                "ruby:archer": {"template": "minecraft:skeleton"},
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            folder.path().join("entity-models.json"),
            serde_json::to_vec(&json!({
                "minecraft:skeleton#main": {"children": {"head": {}}},
                "minecraft:skeleton#helmet": {"children": {"head": {}}},
            }))
            .unwrap(),
        )
        .unwrap();
        let models = ClientMobModels::load(folder.path()).unwrap();
        let vanilla = models.get("minecraft:skeleton").unwrap();
        let archer = models.get("ruby:archer").unwrap();
        assert_eq!(archer.texture, vanilla.texture);
        assert_eq!(part_dump(&archer.model), part_dump(&vanilla.model));
        assert!(archer.layer("skeleton#helmet").is_some());
    }

    #[test]
    fn summoned_age_selects_the_baby_model_and_texture() {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:rabbit".into();
        kind.species = crate::mobs::Species::Rabbit;
        kind.anim = crate::mobs::Anim::Rabbit;
        let kind = Arc::new(kind);
        let model = Arc::new(Part::parse(
            "root",
            &json!({"children": {"head": {}, "left_hind_foot": {}}}),
        ));
        let mut layers = FxHashMap::default();
        layers.insert("rabbit_baby#main".into(), model.clone());
        let visuals = ClientMobVisuals {
            shadow: None,
            model,
            layers,
            texture: "entity/rabbit/rabbit_brown".into(),
        };
        let types = HashMap::from([(kind.id.clone(), kind)]);
        for (age, baby) in [(-24000, true), (-1, true), (0, false), (6000, false)] {
            let (tag, _) = sunrise_core::nbt::parse_snbt(&format!(
                "{{id:\"minecraft:rabbit\",Pos:[0.0d,0.0d,0.0d],Age:{age}}}"
            ))
            .unwrap();
            let m = crate::entity_nbt::mob_from_nbt(&tag, &types, 2, None).unwrap();
            assert_eq!(m.extra.age, age);
            assert_eq!(m.is_baby(), baby);
            assert_eq!(
                render::body(&m, &visuals),
                if baby {
                    (
                        "rabbit_baby#main".into(),
                        "entity/rabbit/rabbit_brown_baby".into(),
                    )
                } else {
                    ("rabbit#main".into(), "entity/rabbit/rabbit_brown".into())
                }
            );
        }
    }
}
