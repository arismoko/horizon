//! A blockstate-driven inspection gallery, separate from gameplay simulation.
pub mod catalog;
pub(crate) mod cube;
use crate::{Result, sprites::Sprites};
pub use catalog::StateModels;
use catalog::descriptor;
pub use cube::CubeStyle;
use mod_api::{
    __private::game::{CallbackOwner, StandaloneModelBaker, StandaloneModelRegistration},
    Answered,
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};
use sunrise_core::position::BlockPos;
use sunrise_world::registry::{BlockRegistry, BlockStateId};

/// The standalone models native mods asked for (`native_standalone_models`).
static STANDALONE: OnceLock<Arc<StandaloneModels>> = OnceLock::new();
/// Test fixture shared with the resource-bake worker.
static STANDALONE_FIXTURE: Mutex<Option<Arc<StandaloneModels>>> = Mutex::new(None);

#[derive(Default)]
struct StandaloneModels {
    names: Vec<String>,
    bakers: Vec<StandaloneBaker>,
}

struct StandaloneBaker {
    registration: StandaloneModelBaker,
    mod_id: &'static str,
    owner: CallbackOwner,
}

#[cfg(feature = "test-support")]
static FIXTURE_CHANGED: std::sync::Condvar = std::sync::Condvar::new();

fn namespaced(name: &str) -> bool {
    name.split_once(':')
        .is_some_and(|(ns, path)| !ns.is_empty() && !path.is_empty())
}

fn standalone_names(mut names: Vec<String>) -> Vec<String> {
    names.retain(|n| namespaced(n));
    names.sort();
    names.dedup();
    names
}

fn freeze_standalone(
    registrations: Vec<Answered<StandaloneModelRegistration>>,
) -> StandaloneModels {
    let mut frozen = StandaloneModels::default();
    for Answered { answer, mod_id, .. } in registrations {
        match answer {
            StandaloneModelRegistration::File(id) => frozen.names.push(id),
            StandaloneModelRegistration::Baker(registration) => {
                let id = &registration.id;
                if !namespaced(id) {
                    log::warn!(
                        "Sunrise: mod {mod_id} standalone baker ID {id:?} needs a namespace and path"
                    );
                    continue;
                }
                let Some(owner) = CallbackOwner::new(mod_id) else {
                    log::warn!(
                        "Sunrise: mod {mod_id} standalone baker {id} has no installed owner"
                    );
                    continue;
                };
                frozen.bakers.push(StandaloneBaker {
                    registration,
                    mod_id,
                    owner,
                });
            }
        }
    }
    frozen.names = standalone_names(frozen.names);
    frozen
}

/// Sets the models no blockstate names that every resource load bakes
/// beside the block models ([`StateModels::standalone`]), once at startup.
/// File IDs are namespaced, sorted and deduplicated; bakers keep registration order.
/// Returns false when already set.
pub fn register_standalone(registrations: Vec<Answered<StandaloneModelRegistration>>) -> bool {
    STANDALONE
        .set(Arc::new(freeze_standalone(registrations)))
        .is_ok()
}

/// Declarations baked beside block models; a test fixture overrides process registration.
fn standalone() -> Option<Arc<StandaloneModels>> {
    STANDALONE_FIXTURE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .or_else(|| STANDALONE.get().cloned())
}

#[cfg(any(test, feature = "test-support"))]
pub fn has_standalone_models() -> bool {
    standalone().is_some()
}

/// Installs standalone declarations until dropped, including on the bake worker, without sealing
/// process registration. Other fixture holders wait until the active fixture finishes.
#[cfg(feature = "test-support")]
pub struct StandaloneFixture;

#[cfg(feature = "test-support")]
pub fn standalone_fixture(names: Vec<String>) -> StandaloneFixture {
    StandaloneFixture::install(StandaloneModels {
        names: standalone_names(names),
        bakers: Vec::new(),
    })
}

#[cfg(feature = "test-support")]
impl StandaloneFixture {
    /// Install a dispatch-attributed batch while its mod set is installed.
    pub fn from_registrations(registrations: Vec<Answered<StandaloneModelRegistration>>) -> Self {
        Self::install(freeze_standalone(registrations))
    }

    fn install(models: StandaloneModels) -> Self {
        let mut fixture = STANDALONE_FIXTURE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while fixture.is_some() {
            fixture = FIXTURE_CHANGED
                .wait(fixture)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *fixture = Some(Arc::new(models));
        Self
    }
}

#[cfg(feature = "test-support")]
impl Drop for StandaloneFixture {
    fn drop(&mut self) {
        let mut fixture = STANDALONE_FIXTURE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *fixture = None;
        FIXTURE_CHANGED.notify_all();
    }
}

type Placement = (&'static str, &'static [(&'static str, &'static str)]);
const GALLERY: [Placement; 16] = [
    (
        "oak_stairs",
        &[
            ("facing", "north"),
            ("half", "top"),
            ("shape", "inner_left"),
        ],
    ),
    ("stone_slab", &[("type", "top")]),
    ("anvil", &[("facing", "east")]),
    ("torch", &[]),
    ("lantern", &[]),
    ("oak_fence", &[("north", "true"), ("east", "true")]),
    ("oak_fence_gate", &[("facing", "west"), ("open", "true")]),
    ("iron_bars", &[("north", "true"), ("south", "true")]),
    ("cobweb", &[]),
    ("grindstone", &[("face", "wall"), ("facing", "west")]),
    ("ladder", &[("facing", "west")]),
    ("amethyst_cluster", &[("facing", "east")]),
    ("stone", &[]),
    ("stone", &[]),
    ("stone", &[]),
    ("stone", &[]),
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ModelInstance {
    pub positions: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
    pub layer: u32,
    pub shade: f32,
}
#[derive(Default)]
pub struct PreparedModels {
    pub catalog: StateModels,
    pub placements: Vec<(BlockPos, BlockStateId)>,
    pub states: Vec<Value>,
}
pub fn prepare(
    root: &crate::resource_source::ResourceRoot,
    registry: &BlockRegistry,
    sprites: &mut Sprites,
) -> Result<PreparedModels> {
    prepare_with_models(
        root,
        registry,
        sprites,
        &crate::model_library::ParsedModels::default(),
    )
}
pub(crate) fn prepare_with_models(
    root: &crate::resource_source::ResourceRoot,
    registry: &BlockRegistry,
    sprites: &mut Sprites,
    models: &crate::model_library::ParsedModels,
) -> Result<PreparedModels> {
    let facts: std::collections::BTreeMap<String, Value> =
        sunrise_world::json_file::read(&root.prepared().join("model-states.json"))?;
    let blocks: Vec<&str> = facts
        .keys()
        .filter(|name| registry.default_state(name).is_some())
        .map(|name| name.strip_prefix("minecraft:").unwrap_or(name))
        .collect();
    let mut catalog = {
        let _step = crate::spans::startup::step("core.bake_states");
        StateModels::from_facts_with_models(
            root.source(),
            registry,
            &blocks,
            sprites,
            &facts,
            models,
        )?
    };
    {
        let _step = crate::spans::startup::step("core.bake_standalone");
        if let Some(standalone) = standalone() {
            catalog.bake_standalone(root.source(), sprites, models, &standalone.names)?;
            catalog.bake_standalone_bakers(root.source(), sprites, models, &standalone.bakers);
        }
    }
    let mut placements = Vec::new();
    let mut states = Vec::new();
    for (index, (name, properties)) in GALLERY.iter().enumerate() {
        let state = registry.resolve(&descriptor(name, properties))?;
        let actual = registry
            .state(state)
            .ok_or("missing gallery registry state")?;
        let position = BlockPos::new((index % 4) as i32 * 4 - 7, 4, (index / 4) as i32 * 4 + 14);
        // A state no definition binds (a pack's partial blockstate file
        // without vanilla's below it) is left out, as the mesher skips it,
        // instead of failing the whole pack.
        let selected = match catalog.collect(state, position) {
            Ok(selected) => selected,
            Err(e) => {
                log::warn!("Sunrise: gallery block {name} has no model: {e}");
                continue;
            }
        };
        states.push(json!({"block":actual.name,"properties":actual.properties,"state_id":state.0,
            "position":[position.x,position.y,position.z],
            "models":selected.iter().map(|(v,_)| json!({"model":v.model,"rotations":v.rotations,"uv_lock":v.uv_lock})).collect::<Vec<_>>()}));
        placements.push((position, state));
    }
    Ok(PreparedModels {
        catalog,
        placements,
        states,
    })
}
pub fn instance(face: &catalog::BakedFace, position: BlockPos) -> ModelInstance {
    let origin = [position.x as f32, position.y as f32, position.z as f32];
    ModelInstance {
        positions: face
            .quad
            .positions
            .map(|v| std::array::from_fn(|i| v[i] + origin[i])),
        uv: face.quad.uv,
        layer: face.layer,
        shade: cube::shade(face),
    }
}

pub fn load_visibility(
    root: &Path,
    registry: &BlockRegistry,
) -> Result<sunrise_world::block_render::FaceVisibility> {
    use std::collections::{BTreeMap, BTreeSet};
    use sunrise_world::{
        block_render::{FaceVisibility, RenderDefinition, SkipRendering},
        collision::CachedCollisionRegistry,
        registry::BlockTableFile,
        worldgen::tags::TagDefinitions,
    };
    let file = BlockTableFile::open(&root.join("occlusion.nbt"), registry)?;
    let occlusion = CachedCollisionRegistry::from_table(file.table("occlusion")?)?;
    let raw: BTreeMap<String, Value> =
        sunrise_world::json_file::read(&root.join("render-definitions.json"))?;
    let mut needs_tags = false;
    let definitions = raw
        .into_iter()
        .map(|(name, v)| -> std::result::Result<_, String> {
            let skip = match v["rule"].as_str().ok_or("missing render rule")? {
                "never" => SkipRendering::Never,
                "same_block" => SkipRendering::SameBlock,
                "bars" => SkipRendering::Bars,
                "leaves" => SkipRendering::Leaves,
                "liquid" => SkipRendering::Liquid,
                "mangrove_roots" => SkipRendering::MangroveRoots,
                _ => return Err("unrecognized render rule".into()),
            };
            let skip_rules = match v.get("skip_rendering") {
                None | Some(Value::Null) => None,
                Some(raw_rules) => {
                    let rules =
                        sunrise_mod_package::content::parse_face_rules(raw_rules, "skip_rendering")
                            .map_err(|e| format!("{name}: {e}"))?;
                    if rules
                        .iter()
                        .any(|r| matches!(r.neighbor, mod_api::FaceNeighbor::Tag { .. }))
                    {
                        needs_tags = true;
                    }
                    Some(rules)
                }
            };
            let hides_rules = match v.get("hides_neighbor_face") {
                None | Some(Value::Null) => Vec::new(),
                Some(raw_rules) => {
                    let rules = sunrise_mod_package::content::parse_face_rules(
                        raw_rules,
                        "hides_neighbor_face",
                    )
                    .map_err(|e| format!("{name}: {e}"))?;
                    if rules
                        .iter()
                        .any(|r| matches!(r.neighbor, mod_api::FaceNeighbor::Tag { .. }))
                    {
                        needs_tags = true;
                    }
                    rules
                }
            };
            let supports_external_face_hiding = match v.get("supports_external_face_hiding") {
                None | Some(Value::Null) => true,
                Some(flag) => flag.as_bool().ok_or_else(|| {
                    format!("{name}: supports_external_face_hiding: expected a boolean")
                })?,
            };
            Ok((
                name,
                RenderDefinition {
                    skip,
                    can_occlude: v["can_occlude"].as_bool().ok_or("missing occlusion flag")?,
                    bars_tag: v["bars_tag"].as_bool().ok_or("missing bars tag")?,
                    skip_rules,
                    hides_rules,
                    supports_external_face_hiding,
                },
            ))
        })
        .collect::<std::result::Result<BTreeMap<_, _>, String>>()?;
    if needs_tags {
        let tags_path = root.join("block-tags.json");
        let bundle: Value = sunrise_world::json_file::read(&tags_path)?;
        let mut tag_defs = TagDefinitions::default();
        for (tag_name, text) in bundle.as_object().ok_or("invalid block tag bundle")? {
            tag_defs.apply_json(
                tag_name,
                "vanilla",
                text.as_str().ok_or("invalid block tag text")?,
            )?;
        }
        let built = tag_defs.build(|block_name, _| {
            registry
                .default_state(block_name)
                .map(|_| block_name.to_owned())
        })?;
        let block_tags: BTreeMap<String, BTreeSet<String>> = built
            .tags
            .into_iter()
            .map(|(tag, members)| (tag, members.into_iter().collect()))
            .collect();
        Ok(FaceVisibility::new_with_tags(
            &occlusion,
            &definitions,
            &block_tags,
        )?)
    } else {
        Ok(FaceVisibility::new(&occlusion, &definitions)?)
    }
}

#[cfg(test)]
fn with_test_registrations<R>(
    register: impl Fn(&mut mod_api::StandaloneModelsEvent<'_>) + Send + Sync + 'static,
    run: impl FnOnce(Vec<Answered<StandaloneModelRegistration>>) -> R,
) -> R {
    use mod_api::{__private::game::RegistrationContextNew, Hooks, RegistrationContext};

    struct Register<F>(F);
    #[mod_api::__subscriptions]
    impl<F: Fn(&mut mod_api::StandaloneModelsEvent<'_>) + Send + Sync + 'static> Hooks for Register<F> {
        fn standalone_models(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::StandaloneModelsEvent<'_>,
        ) {
            (self.0)(event);
        }
    }

    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "registering_mod",
            Box::new(Register(register)),
        )]
        .into(),
        || {
            let mut staged = Vec::new();
            mod_api::dispatch_standalone_models(&RegistrationContext::new(), &mut staged);
            run(staged)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_names_are_namespaced_sorted_and_set_once() {
        assert!(standalone().is_none());
        with_test_registrations(
            |event| {
                for id in [
                    "lanef:block/b",
                    "nonamespace",
                    ":x",
                    "lanef:",
                    "lanef:block/a",
                    "lanef:block/b",
                ] {
                    event.register_model(id);
                }
                for id in [
                    "no_namespace",
                    ":empty_namespace",
                    "namespace:",
                    "other:valid",
                ] {
                    event.register(id, std::iter::empty::<&str>(), |_| Ok(()));
                }
            },
            |staged| {
                assert!(register_standalone(staged));
                assert!(!register_standalone(Vec::new()));
                let registered = standalone().unwrap();
                assert_eq!(registered.names, ["lanef:block/a", "lanef:block/b"]);
                let [baker] = registered.bakers.as_slice() else {
                    panic!("expected only the namespaced baker");
                };
                assert_eq!(baker.registration.id, "other:valid");
                assert_eq!(baker.mod_id, "registering_mod");
            },
        );
    }
}
