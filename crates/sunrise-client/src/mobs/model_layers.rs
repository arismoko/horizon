//! Engine registration and vanilla resource conversion for model layers.
use super::Part;
use crate::mobs_render::EntityModelValidationError;
use mod_api::{
    __private::game::LayerDefinitionRegistration, Answered, LayerDefinition, ModelPart,
    ModelPolygon, ModelVertex, PartPose,
};
use std::{collections::HashMap, path::Path, sync::OnceLock};

/// A vanilla layer's part tree from `entity-models.json`.
pub fn from_part(part: &Part) -> ModelPart {
    let p = part.pose;
    let mut pose = PartPose::offset_and_rotation(p[0], p[1], p[2], p[3], p[4], p[5]);
    pose.x_scale = p[6];
    pose.y_scale = p[7];
    pose.z_scale = p[8];
    let polygons = part
        .polygons
        .iter()
        .map(|poly| {
            ModelPolygon::new(
                poly.vertices
                    .map(|v| ModelVertex::new([v[0], v[1], v[2]], v[3], v[4])),
                poly.normal,
            )
        })
        .collect();
    let children = part
        .children
        .iter()
        .map(|c| (c.name.clone(), from_part(c)))
        .collect();
    ModelPart::new(polygons, children, pose)
}

static REGISTERED: OnceLock<HashMap<String, LayerDefinition>> = OnceLock::new();

#[derive(Debug)]
pub enum RegistrationError {
    Load(Box<dyn std::error::Error>),
    NotNonemptyObject,
    InvalidVanillaLayer(String),
    InvalidModLayer { owner: &'static str, id: String },
    VanillaCollision { owner: &'static str, id: String },
}

impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("register_layer_definitions: ")?;
        match self {
            Self::Load(error) => write!(f, "failed to load vanilla entity-models.json: {error}"),
            Self::NotNonemptyObject => {
                f.write_str("vanilla entity-models.json must be a nonempty object")
            }
            Self::InvalidVanillaLayer(id) => write!(
                f,
                "invalid vanilla model layer entry {id:?} in entity-models.json"
            ),
            Self::InvalidModLayer { owner, id } => write!(
                f,
                "mod {owner}: model layer {id:?} must be <namespace>:<path>#<layer> with a valid namespaced model identifier"
            ),
            Self::VanillaCollision { owner, id } => write!(
                f,
                "mod {owner}: model layer {id} collides with an existing vanilla model layer"
            ),
        }
    }
}

impl std::error::Error for RegistrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Load(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

/// Validate and stage accepted mod suppliers (last-writer-wins per ID), failing closed on a
/// missing/invalid vanilla `entity-models.json`, any malformed ID, or any collision with an
/// existing vanilla layer before invoking any supplier or publishing. Then invoke each winning
/// supplier once at client startup and publish the realized layers.
pub fn register(
    layers: Vec<Answered<LayerDefinitionRegistration>>,
    vanilla_root: &Path,
) -> Result<(), RegistrationError> {
    register_into(layers, vanilla_root, &REGISTERED)
}

/// Validate, realize, and publish model layer definitions into a caller-supplied slot (used by
/// `register` for the process table and by tests with a scoped table).
pub fn register_into(
    layers: Vec<Answered<LayerDefinitionRegistration>>,
    vanilla_root: &Path,
    slot: &OnceLock<HashMap<String, LayerDefinition>>,
) -> Result<(), RegistrationError> {
    let doc = super::EntityModelDocument::load(vanilla_root).map_err(|error| {
        match error.downcast::<EntityModelValidationError>() {
            Ok(error) => match *error {
                EntityModelValidationError::NotNonemptyObject(_) => {
                    RegistrationError::NotNonemptyObject
                }
                EntityModelValidationError::InvalidLayer { id, .. } => {
                    RegistrationError::InvalidVanillaLayer(id)
                }
            },
            Err(error) => RegistrationError::Load(error),
        }
    })?;
    let mut order: Vec<&str> = Vec::new();
    let mut staged: HashMap<&str, &Answered<LayerDefinitionRegistration>> = HashMap::new();
    for entry in &layers {
        let id = entry.answer.id.as_str();
        if !is_layer_id(id) {
            return Err(RegistrationError::InvalidModLayer {
                owner: entry.mod_id,
                id: id.to_owned(),
            });
        }
        if doc.model(id).is_some() {
            return Err(RegistrationError::VanillaCollision {
                owner: entry.mod_id,
                id: id.to_owned(),
            });
        }
        if staged.insert(id, entry).is_none() {
            order.push(id);
        }
    }
    if slot.get().is_some() {
        log::warn!("model layers were registered already; these are left out");
        return Ok(());
    }
    let mut definitions = HashMap::new();
    for id in order {
        let entry = staged[id];
        if let Some(definition) = entry.answer.build(entry.mod_id) {
            definitions.insert(id.to_owned(), definition);
        }
    }
    if slot.set(definitions).is_err() {
        log::warn!("model layers were registered already; these are left out");
    }
    Ok(())
}

/// The realized definition of the layer a native mod registered as `id`.
pub fn registered(id: &str) -> Option<&'static LayerDefinition> {
    REGISTERED.get()?.get(id)
}

pub(crate) fn is_layer_id(id: &str) -> bool {
    // ModelLayerLocation.toString uses the fully qualified Identifier (even when either
    // half is empty). Its layer String may contain any characters, including further '#'.
    id.split_once('#')
        .and_then(|(location, _)| location.split_once(':'))
        .is_some_and(|(namespace, path)| mod_api::Identifier::from_parts(namespace, path).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{CubeDeformation, CubeListBuilder, MeshDefinition};

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sunrise-model-layers-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn freezes_winning_layer_suppliers_and_reports_the_failed_supplier() {
        use mod_api::__private::{HookPanicReport, game::RegistrationContextNew, take_hook_panics};
        use mod_api::{Hooks, ModEntry, RegisterLayerDefinitionsEvent, RegistrationContext};
        use std::sync::{Arc, Mutex};

        struct Layers(&'static [(&'static str, i32)], Arc<Mutex<Vec<i32>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Layers {
            fn register_layer_definitions(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterLayerDefinitionsEvent<'_>,
            ) {
                for &(id, offset) in self.0 {
                    let calls = self.1.clone();
                    event.register(id, move || {
                        calls.lock().unwrap().push(offset);
                        assert!(offset >= 0, "panicking model layer supplier");
                        let mut mesh = MeshDefinition::new();
                        mesh.add_or_replace_child(
                            mesh.root(),
                            "head",
                            CubeListBuilder::create(),
                            PartPose::offset(0.0, offset as f32, 0.0),
                        );
                        LayerDefinition::create(mesh, 64, 32)
                    });
                }
            }
        }

        let dir = scratch_dir("freeze");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let slot: OnceLock<HashMap<String, LayerDefinition>> = OnceLock::new();
        let registered = |id: &str| slot.get().and_then(|map| map.get(id));

        // Missing, invalid, non-object, and empty entity-models.json fail closed before publishing.
        assert!(register_into(Vec::new(), &dir, &slot).is_err());
        std::fs::write(dir.join("entity-models.json"), "not json").unwrap();
        assert!(register_into(Vec::new(), &dir, &slot).is_err());
        std::fs::write(dir.join("entity-models.json"), "[]").unwrap();
        assert!(register_into(Vec::new(), &dir, &slot).is_err());
        std::fs::write(dir.join("entity-models.json"), "{}").unwrap();
        assert!(register_into(Vec::new(), &dir, &slot).is_err());

        std::fs::write(
            dir.join("entity-models.json"),
            r#"{"minecraft:zombie#main": {"pose": [0,0,0,0,0,0,1,1,1], "children": {}}}"#,
        )
        .unwrap();

        // Vanilla collision fails closed before calling any supplier or publishing.
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "colliding",
                Box::new(Layers(
                    &[("minecraft:zombie#main", 1), ("colliding:ok#main", 2)],
                    calls.clone(),
                )),
            )]
            .into(),
            || {
                let mut suppliers = Vec::new();
                mod_api::dispatch_register_layer_definitions(
                    &RegistrationContext::new(),
                    &mut suppliers,
                );
                assert!(matches!(register_into(suppliers, &dir, &slot),
                    Err(RegistrationError::VanillaCollision { owner: "colliding", id }) if id == "minecraft:zombie#main"));
                assert!(calls.lock().unwrap().is_empty());
                assert!(registered("colliding:ok#main").is_none());
            },
        );

        // Malformed model identifier fails closed before calling any supplier or publishing.
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "badid",
                Box::new(Layers(&[("BadMod:golem#main", 1)], calls.clone())),
            )]
            .into(),
            || {
                let mut suppliers = Vec::new();
                mod_api::dispatch_register_layer_definitions(
                    &RegistrationContext::new(),
                    &mut suppliers,
                );
                assert!(matches!(register_into(suppliers, &dir, &slot),
                    Err(RegistrationError::InvalidModLayer { owner: "badid", id }) if id == "BadMod:golem#main"));
                assert!(calls.lock().unwrap().is_empty());
            },
        );

        // Valid registrations with unrestricted suffixes and mod-to-mod last-writer-wins publish.
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new(
                    "first",
                    Box::new(Layers(
                        &[
                            ("first:broken#main", -1),
                            ("minecraft:custom_golem#main", 3),
                            ("first:golem#outer:armor", 6),
                            ("first:golem#a#b", 7),
                        ],
                        calls.clone(),
                    )),
                ),
                ModEntry::new(
                    "second",
                    Box::new(Layers(
                        &[("first:golem#outer:armor", 9), ("second:golem#main", 12)],
                        calls.clone(),
                    )),
                ),
            ]
            .into(),
            || {
                let mut suppliers = Vec::new();
                mod_api::dispatch_register_layer_definitions(
                    &RegistrationContext::new(),
                    &mut suppliers,
                );
                assert!(calls.lock().unwrap().is_empty());
                register_into(suppliers, &dir, &slot).unwrap();
                assert_eq!(*calls.lock().unwrap(), [-1, 3, 9, 7, 12]);
                assert_eq!(
                    take_hook_panics(),
                    [HookPanicReport {
                        mod_id: "first",
                        hook: "build_layer_definition",
                    }],
                );

                assert!(registered("first:broken#main").is_none());
                assert!(registered("minecraft:zombie#main").is_none());
                assert_eq!(
                    registered("minecraft:custom_golem#main")
                        .unwrap()
                        .bake_root()
                        .get_child("head")
                        .unwrap()
                        .y,
                    3.0
                );
                for _ in 0..2 {
                    let model = registered("first:golem#outer:armor").unwrap().bake_root();
                    assert_eq!(model.get_child("head").unwrap().y, 9.0);
                }
                assert_eq!(
                    registered("first:golem#a#b")
                        .unwrap()
                        .bake_root()
                        .get_child("head")
                        .unwrap()
                        .y,
                    7.0
                );
                assert_eq!(
                    registered("second:golem#main")
                        .unwrap()
                        .bake_root()
                        .get_child("head")
                        .unwrap()
                        .y,
                    12.0
                );

                let mut late = Vec::new();
                mod_api::dispatch_register_layer_definitions(
                    &RegistrationContext::new(),
                    &mut late,
                );
                register_into(late, &dir, &slot).unwrap();
                assert_eq!(*calls.lock().unwrap(), [-1, 3, 9, 7, 12]);
                assert!(take_hook_panics().is_empty());
            },
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn layer_ids_require_namespaced_model_and_preserve_unrestricted_layer_suffix() {
        for good in [
            "minecraft:zombie#main",
            "mymod:golem#main",
            "my_mod:mobs/golem.v2#outer_armor",
            "mymod:golem#outer:armor",
            "mymod:golem#a#b",
            "mymod:golem#Outer Layer",
            "mymod:golem#",
            "mymod:#main",
            ":golem#main",
        ] {
            assert!(is_layer_id(good), "{good}");
        }
        for bad in [
            "mymod:golem",
            "golem#main",
            "MyMod:golem#main",
            "#main",
            "mymod:bad space#main",
        ] {
            assert!(!is_layer_id(bad), "{bad}");
        }
    }
    /// `HumanoidModel.createMesh(CubeDeformation.NONE, 0)` built with these
    /// builders bakes what vanilla baked for the zombie
    /// (`minecraft:zombie#main` in the prepared `entity-models.json`):
    /// `cargo test -p sunrise-client-lib --lib model_layers -- --ignored`
    /// with SUNRISE_TEST_RESOURCES set to the prepared resources.
    #[test]
    #[ignore = "daily"]
    fn the_humanoid_mesh_bakes_as_vanillas_zombie() {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("SUNRISE_TEST_RESOURCES"),
        );
        let vanilla =
            super::super::load_model(&root, "minecraft:zombie#main").expect("zombie#main");
        let vanilla = from_part(&vanilla);

        // `HumanoidModel.createMesh`: (part, parent, texOffs, mirror, origin, size, grow, pose).
        let half = CubeDeformation::new(0.5);
        let none = CubeDeformation::NONE;
        let (z4, limb) = ([-4.0, -8.0, -4.0], [4.0, 12.0, 4.0]);
        #[rustfmt::skip]
        let parts = [
            ("head", "", (0, 0), false, z4, [8.0; 3], none, [0.0; 3]),
            ("hat", "head", (32, 0), false, z4, [8.0; 3], half, [0.0; 3]),
            ("body", "", (16, 16), false, [-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], none, [0.0; 3]),
            ("right_arm", "", (40, 16), false, [-3.0, -2.0, -2.0], limb, none, [-5.0, 2.0, 0.0]),
            ("left_arm", "", (40, 16), true, [-1.0, -2.0, -2.0], limb, none, [5.0, 2.0, 0.0]),
            ("right_leg", "", (0, 16), false, [-2.0, 0.0, -2.0], limb, none, [-1.9, 12.0, 0.0]),
            ("left_leg", "", (0, 16), true, [-2.0, 0.0, -2.0], limb, none, [1.9, 12.0, 0.0]),
        ];
        let mut mesh = MeshDefinition::new();
        for (name, parent, (u, v), mirror, origin, size, grow, [x, y, z]) in parts {
            let parent = mesh.child(mesh.root(), parent).unwrap_or(mesh.root());
            let cubes = CubeListBuilder::create()
                .tex_offs(u, v)
                .set_mirror(mirror)
                .add_box_with(origin, size, grow);
            mesh.add_or_replace_child(parent, name, cubes, PartPose::offset(x, y, z));
        }
        let ours = LayerDefinition::create(mesh, 64, 64).bake_root();

        fn same(path: &str, ours: &ModelPart, vanilla: &ModelPart) {
            assert_eq!(ours.store_pose(), vanilla.store_pose(), "{path} pose");
            assert_eq!(ours.polygons, vanilla.polygons, "{path} faces");
            let mut names: Vec<_> = ours.children().map(|(n, _)| n).collect();
            let mut theirs: Vec<_> = vanilla.children().map(|(n, _)| n).collect();
            names.sort_unstable();
            theirs.sort_unstable();
            assert_eq!(names, theirs, "{path} children");
            for (name, child) in ours.children() {
                same(
                    &format!("{path}/{name}"),
                    child,
                    vanilla.get_child(name).unwrap(),
                );
            }
        }
        same("root", &ours, &vanilla);
    }
}
