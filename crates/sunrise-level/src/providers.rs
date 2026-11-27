//! Vanilla's `context_int_provider` and `context_float_provider` values (furnace
//! fuel times, brewing times), evaluated from the data pack's JSON.
mod evaluation;

pub use evaluation::{ConditionContext, Evaluation};
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

pub const NUMBER_PROVIDER_KINDS: [&str; 2] = ["context_int_provider", "context_float_provider"];

/// `ContextIntProviderTypes.bootstrap`.
const INT_PROVIDER_TYPES: &[&str] = &[
    "abs",
    "avg",
    "binomial",
    "conditional",
    "constant",
    "sub",
    "environment_attribute",
    "from_float",
    "max",
    "min",
    "floor_mod",
    "floor_div",
    "mod",
    "div",
    "negate",
    "number_dispatcher",
    "pow",
    "mul",
    "score",
    "storage",
    "add",
    "uniform",
    "weighted_list",
];
/// `ContextFloatProviderTypes.bootstrap`.
const FLOAT_PROVIDER_TYPES: &[&str] = &[
    "abs",
    "avg",
    "ceil",
    "conditional",
    "constant",
    "cos",
    "sub",
    "enchantment_level",
    "environment_attribute",
    "floor",
    "from_int",
    "length",
    "max",
    "min",
    "mod",
    "negate",
    "number_dispatcher",
    "pow",
    "mul",
    "div",
    "round",
    "sin",
    "sqrt",
    "storage",
    "add",
    "truncate",
    "uniform",
    "weighted_list",
];

/// `ContextIntProviders.DIRECT_CODEC` / `ContextFloatProviders.DIRECT_CODEC`, the
/// registry file codecs: an inline number, or an object of a registered provider
/// type. A reference by id exists only in the holder `CODEC`.
fn validate(value: &Value, types: &[&str]) -> Result<(), String> {
    match value {
        Value::Number(_) => Ok(()),
        Value::Object(fields) => {
            let kind = fields.get("type").and_then(Value::as_str).unwrap_or("");
            let name = kind.strip_prefix("minecraft:").unwrap_or(kind);
            if types.contains(&name) {
                Ok(())
            } else {
                Err(format!("unknown provider type {kind:?}"))
            }
        }
        _ => Err("expected a number or a typed object".into()),
    }
}

/// Evaluator for `context_int_provider` / `context_float_provider` values.
#[derive(Clone)]
pub struct Providers(Arc<Definitions>);

#[derive(Default)]
struct Definitions {
    ints: BTreeMap<String, Value>,
    floats: BTreeMap<String, Value>,
}
fn load_dir(dir: &Path, namespace: &str, out: &mut BTreeMap<String, Value>) {
    let Ok(folder) = sunrise_world::json_folder::read(dir) else {
        return;
    };
    for file in &folder.files {
        if sunrise_world::json_folder::valid_resource_path(namespace, &file.name)
            && let Ok(bytes) = file.bytes()
            && let Ok(value) = serde_json::from_slice(bytes)
        {
            out.insert(format!("{namespace}:{}", file.name), value);
        }
    }
}
impl Definitions {
    fn load(root: &Path) -> Self {
        let mut ints = BTreeMap::new();
        let mut floats = BTreeMap::new();
        let mut namespaces: std::collections::BTreeSet<String> =
            std::iter::once("minecraft".to_owned())
                .chain(sunrise_items::content_namespaces::namespaces(root))
                .collect();
        namespaces.extend(
            fs::read_dir(root.join("data"))
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned()),
        );
        for namespace in namespaces {
            let data = root.join("data").join(&namespace);
            load_dir(&data.join("context_int_provider"), &namespace, &mut ints);
            load_dir(
                &data.join("context_float_provider"),
                &namespace,
                &mut floats,
            );
        }
        Self { ints, floats }
    }
}

impl Providers {
    pub fn load(root: &Path) -> Self {
        Self(Arc::new(Definitions::load(root)))
    }
    /// The world's registry definitions: the enabled pack winner replaces
    /// each resource-root entry before the world publishes its tables.
    pub fn load_with(
        root: &Path,
        overlay: &sunrise_world::loot::PackOverlay<'_>,
    ) -> Result<Self, String> {
        let mut providers = Definitions::load(root);
        for (kind, table) in NUMBER_PROVIDER_KINDS
            .into_iter()
            .zip([&mut providers.ints, &mut providers.floats])
        {
            for (id, files) in overlay.kind(kind) {
                let value = serde_json::from_slice(sunrise_world::loot::PackOverlay::top(files))
                    .map_err(|error| format!("{kind} {id}: {error}"))?;
                if overlay.conditions.allows_id(&value, kind, id) {
                    table.insert(id.to_owned(), value);
                } else {
                    table.remove(id);
                }
            }
        }
        let mut errors: Vec<String> = [
            ("context_int_provider", INT_PROVIDER_TYPES, &providers.ints),
            (
                "context_float_provider",
                FLOAT_PROVIDER_TYPES,
                &providers.floats,
            ),
        ]
        .into_iter()
        .flat_map(|(kind, types, table)| {
            table.iter().filter_map(move |(id, value)| {
                validate(value, types)
                    .err()
                    .map(|error| format!("{kind} {id}: {error}"))
            })
        })
        .collect();
        if !errors.is_empty() {
            errors.sort();
            return Err(errors.join("\n"));
        }
        Ok(Self(Arc::new(providers)))
    }
    /// The accepted definitions used by provider evaluation, never a resource-file inventory.
    pub fn registry(&self, registry: &str) -> Option<&BTreeMap<String, Value>> {
        match registry {
            "minecraft:context_int_provider" => Some(&self.0.ints),
            "minecraft:context_float_provider" => Some(&self.0.floats),
            _ => None,
        }
    }
    fn eval(v: &Value, fast: bool, table: &BTreeMap<String, Value>, depth: u32) -> f64 {
        if depth > 16 {
            return 0.0;
        }
        match v {
            Value::Number(n) => n.as_f64().unwrap_or(0.0),
            Value::String(name) => table
                .get(name)
                .map_or(0.0, |v| Self::eval(v, fast, table, depth + 1)),
            Value::Object(o) => {
                let f =
                    |k: &str| Self::eval(o.get(k).unwrap_or(&Value::Null), fast, table, depth + 1);
                match o.get("type").and_then(Value::as_str).unwrap_or("") {
                    "minecraft:div" => {
                        let r = f("right");
                        if r == 0.0 {
                            0.0
                        } else {
                            (f("left") / r).trunc()
                        }
                    }
                    "minecraft:mul" => f("left") * f("right"),
                    "minecraft:add" | "minecraft:sum" => f("left") + f("right"),
                    "minecraft:sub" => f("left") - f("right"),
                    "minecraft:constant" => f("value"),
                    "minecraft:conditional" => {
                        // The only condition in the pinned data: block/fast_cooking.
                        let condition = o.get("condition").and_then(Value::as_str)
                            == Some("minecraft:block/fast_cooking");
                        if condition && fast {
                            f("on_true")
                        } else {
                            f("on_false")
                        }
                    }
                    _ => 0.0,
                }
            }
            _ => 0.0,
        }
    }
    /// The chance that a `compostable` layers provider adds a layer: the weight of its non-zero
    /// layers in the default distribution (the composter's empty-level case is left out), or its
    /// plain number.
    pub fn layer_chance(&self, v: &Value) -> f32 {
        let def = v.as_str().and_then(|n| self.0.ints.get(n)).unwrap_or(v);
        let base = def.get("default").unwrap_or(def);
        if let Some(n) = base.as_f64() {
            return n as f32;
        }
        let weights: Vec<(f64, f64)> = base["distribution"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| {
                (
                    e["data"].as_f64().unwrap_or(0.0),
                    e["weight"].as_f64().unwrap_or(0.0),
                )
            })
            .collect();
        let total: f64 = weights.iter().map(|(_, w)| w).sum();
        if total <= 0.0 {
            return 0.0;
        }
        (weights
            .iter()
            .filter(|(d, _)| *d > 0.0)
            .map(|(_, w)| w)
            .sum::<f64>()
            / total) as f32
    }
    /// A plain `context_int_provider` value or reference (no context conditions).
    pub fn int_value(&self, v: &Value) -> i32 {
        Self::eval(v, false, &self.0.ints, 0) as i32
    }
    /// A plain `context_float_provider` value or reference; `default` when absent.
    pub fn float_value(&self, v: &Value, default: f32) -> f32 {
        if v.is_null() {
            default
        } else {
            Self::eval(v, false, &self.0.floats, 0) as f32
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn empty() -> Self {
        Self(Arc::new(Definitions::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_compost_chance_is_the_weight_of_the_non_zero_layers() {
        let providers = Providers::empty();
        assert_eq!(providers.layer_chance(&json!(0.3)), 0.3);
        let layers = json!({"default": {"distribution": [
            {"data": 0, "weight": 1}, {"data": 1, "weight": 3}]}});
        assert_eq!(providers.layer_chance(&layers), 0.75);
        assert_eq!(providers.layer_chance(&json!("minecraft:missing")), 0.0);
    }

    #[test]
    fn a_burn_time_is_the_provider_resolved() {
        let providers = Providers::empty();
        assert_eq!(providers.int_value(&json!(200)), 200);
        let sum = json!({"type": "minecraft:add", "left": 100, "right": 50});
        assert_eq!(providers.int_value(&sum), 150);
    }

    #[test]
    fn an_unregistered_provider_type_refuses_the_load() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-provider-types-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let ints = root.join("data/example/context_int_provider");
        let floats = root.join("data/example/context_float_provider");
        std::fs::create_dir_all(&ints).unwrap();
        std::fs::create_dir_all(&floats).unwrap();
        std::fs::write(
            ints.join("ok.json"),
            r#"{"type":"minecraft:add","left":1,"right":2}"#,
        )
        .unwrap();
        std::fs::write(ints.join("bad.json"), r#"{"type":"example:nope"}"#).unwrap();
        // `from_float` is an int provider type only.
        std::fs::write(
            floats.join("bad.json"),
            r#"{"type":"minecraft:from_float"}"#,
        )
        .unwrap();
        // A reference by id is only in the holder codec, not the file codec.
        std::fs::write(ints.join("reference.json"), r#""example:ok""#).unwrap();
        std::fs::write(floats.join("reference.json"), r#""example:ok""#).unwrap();
        let Err(error) = Providers::load_with(&root, &sunrise_world::loot::PackOverlay::NONE)
        else {
            panic!("unregistered provider types loaded")
        };
        assert_eq!(error.lines().count(), 4, "{error}");
        assert!(
            error.contains("context_int_provider example:reference"),
            "{error}"
        );
        assert!(
            error.contains("context_float_provider example:reference"),
            "{error}"
        );
        assert!(
            error.contains("context_int_provider example:bad"),
            "{error}"
        );
        assert!(
            error.contains("context_float_provider example:bad"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_reads_share_accepted_names_with_provider_evaluation() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-provider-entries-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let folder = root.join("data/example/context_int_provider/sub");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("value.json"), "17").unwrap();
        std::fs::write(folder.join("invalid.json"), "{").unwrap();
        std::fs::write(folder.join("Invalid_Name.json"), "19").unwrap();
        let providers = Providers::load(&root);
        let table = providers
            .registry("minecraft:context_int_provider")
            .unwrap();
        assert_eq!(table.len(), 1);
        assert_eq!(table.get("example:sub/value"), Some(&json!(17)));
        assert!(!table.contains_key("example:sub/invalid"));
        assert_eq!(providers.int_value(&json!("example:sub/Invalid_Name")), 0);
        assert_eq!(providers.int_value(&json!("example:sub/value")), 17);
        assert!(providers.registry("example:unknown").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
