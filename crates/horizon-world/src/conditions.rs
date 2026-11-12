//! NeoForge's load conditions (`neoforge:conditions`): a data file that has
//! the key loads only when every condition in it holds, and is silently
//! absent otherwise (NeoForge's `ICondition`). Mods' packs lean on it
//! for cross-mod recipes — a recipe for another mod's material only makes
//! sense to load when that mod's data is in the root.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Enabled package IDs captured by the engine for one owning data load.
/// Readers share this immutable view; they never recover IDs from namespaces.
/// Resource selection happens before evaluating ordinary JSON files. Stacked
/// tags and data maps instead evaluate each layer before merge or replace.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LoadConditions {
    mods: Vec<String>,
    registered: Option<RegisteredLookup>,
    context: Context,
    diagnostics: Diagnostics,
}

impl LoadConditions {
    /// A vanilla data load has the built-in package only.
    pub const fn vanilla() -> Self {
        Self {
            mods: Vec::new(),
            registered: None,
            context: Context {
                features: None,
                registries: None,
                tags: None,
            },
            diagnostics: Diagnostics::new(),
        }
    }

    /// Capture IDs visible to the owning load's side. The engine supplies its
    /// enabled-mod inventory's server view for logical-server data.
    pub fn new<'a>(ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut mods: Vec<_> = ids
            .into_iter()
            .filter(|id| *id != "minecraft")
            .map(str::to_owned)
            .collect();
        mods.sort();
        mods.dedup();
        Self {
            mods,
            registered: None,
            context: Context::default(),
            diagnostics: Diagnostics::default(),
        }
    }

    /// Use the recipe source's inventory while retaining this candidate's
    /// registered handlers, evaluation cache and diagnostics.
    pub fn with_mod_inventory<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut view = self.clone();
        view.mods = Self::new(ids).mods;
        if view.mods != self.mods {
            view.diagnostics = match &self.registered {
                Some(registered) => registered
                    .0
                    .inventories
                    .lock()
                    .unwrap()
                    .entry(view.mods.clone())
                    .or_default()
                    .clone(),
                // Without handlers there are no cached answers or errors;
                // keep foreign diagnostic deduplication shared across the candidate.
                None => self.diagnostics.clone(),
            };
        }
        view
    }

    pub fn mod_loaded(&self, id: &str) -> bool {
        id == "minecraft"
            || self
                .mods
                .binary_search_by(|loaded| loaded.as_str().cmp(id))
                .is_ok()
    }

    /// `IContext.enabledFeatures`: the world's enabled feature flag ids, which
    /// `neoforge:feature_flags_enabled` reads. Empty is vanilla's default set.
    pub fn with_features<'a>(mut self, flags: impl IntoIterator<Item = &'a str>) -> Self {
        let mut flags: BTreeSet<String> = flags.into_iter().map(qualified).collect();
        if flags.is_empty() {
            flags.insert("minecraft:vanilla".to_owned());
        }
        self.context.features = Some(Arc::new(flags));
        self
    }

    /// `IContext.registries`: every registry's entry ids (built-in and the
    /// packs' datapack registries), which `neoforge:registered` reads.
    pub fn with_registries<'a, I>(
        mut self,
        registries: impl IntoIterator<Item = (&'a str, I)>,
    ) -> Self
    where
        I: IntoIterator<Item = &'a str>,
    {
        self.context.registries = Some(Arc::new(Names::new(registries)));
        self
    }

    /// `IContext.getTag`: the tags this load has bound, as each registry's
    /// tags holding at least one entry. Readers that run before the tags load
    /// never get this view, so `neoforge:tag_empty` there omits the file
    /// instead of answering from an older load's tags.
    pub fn with_tags<'a, I>(&self, non_empty: impl IntoIterator<Item = (&'a str, I)>) -> Self
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut view = self.clone();
        view.context.tags = Some(Arc::new(Names::new(non_empty)));
        view
    }

    /// `ConditionalOps.decodeListWithElementConditions`: in every list of an
    /// admitted file, drop an element whose own `neoforge:conditions` fail and
    /// strip the key from one whose conditions hold, before the file decodes.
    pub fn filter_elements(&self, file: &mut Value, path: &str) {
        match file {
            Value::Array(elements) => elements.retain_mut(|element| {
                if element.get("neoforge:conditions").is_some() {
                    if !self.allows_file(element, path) {
                        return false;
                    }
                    if let Some(fields) = element.as_object_mut() {
                        fields.remove("neoforge:conditions");
                    }
                }
                self.filter_elements(element, path);
                true
            }),
            Value::Object(fields) => {
                for (key, value) in fields.iter_mut() {
                    if key != "neoforge:conditions" {
                        self.filter_elements(value, path);
                    }
                }
            }
            _ => {}
        }
    }

    /// `allows_id`, then `filter_elements` on an admitted file.
    pub fn admit_id(&self, file: &mut Value, kind: &str, id: &str) -> bool {
        if !self.allows_id(file, kind, id) {
            return false;
        }
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        self.filter_elements(file, &format!("data/{namespace}/{kind}/{path}.json"));
        true
    }

    /// Attach the engine's immutable registration table to this candidate load.
    pub fn with_lookup(
        mut self,
        owned: BTreeMap<String, String>,
        lookup: impl Fn(&str, &Value) -> Option<Result<bool, String>> + Send + Sync + 'static,
    ) -> Self {
        if self.diagnostics.0.is_none() {
            self.diagnostics = Diagnostics::default();
        }
        self.registered = Some(RegisteredLookup(std::sync::Arc::new(LookupState {
            lookup: Box::new(lookup),
            owned,
            inventories: std::sync::Mutex::new(BTreeMap::from([(
                self.mods.clone(),
                self.diagnostics.clone(),
            )])),
        })));
        self
    }

    /// Whether this candidate has any registered condition handlers.
    pub fn has_registered(&self) -> bool {
        self.registered.is_some()
    }

    /// Unknown condition kinds retain the built-in fail-closed behavior.
    pub fn allows(&self, file: &Value) -> bool {
        self.allows_file(file, "data file")
    }

    /// Evaluate a resource, constructing its path only for registered handlers or diagnostics.
    pub fn allows_id(&self, file: &Value, kind: &str, id: &str) -> bool {
        self.allows_context(file, || {
            let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
            format!("data/{namespace}/{kind}/{path}.json")
        })
    }

    /// Evaluate one tag layer.
    pub fn allows_tag(&self, file: &Value, registry: &str, id: &str) -> bool {
        self.allows_context(file, || {
            let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
            format!("data/{namespace}/tags/{registry}/{path}.json")
        })
    }

    /// Evaluate a resource whose namespace and path are stored separately.
    pub fn allows_parts(&self, file: &Value, kind: &str, namespace: &str, path: &str) -> bool {
        self.allows_context(file, || format!("data/{namespace}/{kind}/{path}.json"))
    }

    /// Evaluate a folder resource without joining paths in the no-mod fast path.
    pub fn allows_path(&self, file: &Value, directory: &std::path::Path, name: &str) -> bool {
        self.allows_context(file, || {
            directory.join(format!("{name}.json")).display().to_string()
        })
    }

    /// Evaluate once per entry and payload, outside the diagnostics mutex.
    pub fn allows_file(&self, file: &Value, path: &str) -> bool {
        self.allows_context(file, || path.to_owned())
    }

    fn allows_context(&self, file: &Value, path: impl FnOnce() -> String) -> bool {
        let Some(payload) = file.get("neoforge:conditions") else {
            return true;
        };
        let loaded = |id: &str| self.mod_loaded(id);
        let context = &self.context;
        let Some(registered) = &self.registered else {
            // The ordinary path uses only borrowed nodes and a boolean. No path,
            // argument copy, name set, cache key or diagnostics lock is needed.
            let mut unknown = false;
            let answer = evaluate(
                file,
                &loaded,
                &|_, _| None,
                &BTreeMap::new(),
                context,
                &mut |_| unknown = true,
            )
            .expect("empty lookup cannot fail");
            if !unknown {
                return answer;
            }
            let mut names = BTreeSet::new();
            let _ = evaluate(
                file,
                &loaded,
                &|_, _| None,
                &BTreeMap::new(),
                context,
                &mut |name| {
                    names.insert(name.to_owned());
                },
            );
            self.report_unknown(&path(), names);
            return answer;
        };
        let path = path();
        let path = path
            .rfind("/data/")
            .map_or(path.as_str(), |index| &path[index + 1..]);
        let state = self
            .diagnostics
            .0
            .as_ref()
            .expect("owning candidate diagnostics");
        let cell = state
            .lock()
            .unwrap()
            .cache
            .entry((path.to_owned(), payload.to_string(), context.key()))
            .or_default()
            .clone();
        *cell.get_or_init(|| {
            let mut unknown = BTreeSet::new();
            let result = evaluate(
                file,
                &loaded,
                &*registered.0.lookup,
                &registered.0.owned,
                context,
                &mut |name| {
                    unknown.insert(name.to_owned());
                },
            );
            if !unknown.is_empty() {
                self.report_unknown(path, unknown);
            }
            match result {
                Ok(answer) => answer,
                Err(error) => {
                    state
                        .lock()
                        .unwrap()
                        .errors
                        .push(format!("{path}: {error}"));
                    false
                }
            }
        })
    }

    fn report_unknown(&self, path: &str, names: BTreeSet<String>) {
        let diagnostic = || {
            format!(
                "{path}: unknown load conditions [{}]; file left out",
                names.into_iter().collect::<Vec<_>>().join(", ")
            )
        };
        if let Some(state) = &self.diagnostics.0 {
            let mut state = state.lock().unwrap();
            if !state.foreign_errors.contains_key(path) {
                let diagnostic = diagnostic();
                log::error!("{diagnostic}");
                state.foreign_errors.insert(path.to_owned(), diagnostic);
            }
        } else {
            log::error!("{}", diagnostic());
        }
    }

    /// Non-fatal foreign-namespace errors emitted by this candidate, one per file.
    pub fn foreign_errors(&self) -> Vec<String> {
        self.diagnostics
            .0
            .as_ref()
            .map(|state| {
                state
                    .lock()
                    .unwrap()
                    .foreign_errors
                    .values()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Refuse a failed candidate before publishing any of its tables.
    pub fn check_errors(&self) -> Result<(), String> {
        if let Some(state) = self.diagnostics.0.as_ref() {
            let state = state.lock().unwrap();
            if !state.errors.is_empty() {
                return Err(state.errors.join("; "));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
struct CandidateDiagnostics {
    // Keyed by the context too: a retained candidate's answer under one load's
    // features, registries or tags must not serve the next load.
    cache: BTreeMap<(String, String, u64), std::sync::Arc<std::sync::OnceLock<bool>>>,
    foreign_errors: BTreeMap<String, String>,
    errors: Vec<String>,
}
// Diagnostics do not contribute to a resource generation's cache identity.
#[derive(Clone, Debug)]
struct Diagnostics(Option<std::sync::Arc<std::sync::Mutex<CandidateDiagnostics>>>);
impl Diagnostics {
    const fn new() -> Self {
        Self(None)
    }
}
impl Default for Diagnostics {
    fn default() -> Self {
        Self(Some(Default::default()))
    }
}
impl PartialEq for Diagnostics {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Diagnostics {}
impl std::hash::Hash for Diagnostics {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

/// `ICondition.IContext`: what `feature_flags_enabled`, `registered` and
/// `tag_empty` read. A part the owning load didn't provide leaves its
/// condition unsupported: the file is omitted with one error, never answered stale.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct Context {
    features: Option<Arc<BTreeSet<String>>>,
    registries: Option<Arc<Names>>,
    tags: Option<Arc<Names>>,
}
impl Context {
    /// The cache key of answers given under this context.
    fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

/// Registry id -> entry ids, compared and hashed by a fingerprint taken once,
/// so cache identity never rehashes every entry.
#[derive(Debug)]
struct Names {
    by_registry: BTreeMap<String, BTreeSet<String>>,
    fingerprint: u64,
}
impl Names {
    fn new<'a, I>(registries: impl IntoIterator<Item = (&'a str, I)>) -> Self
    where
        I: IntoIterator<Item = &'a str>,
    {
        use std::hash::{Hash, Hasher};
        let mut by_registry: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (registry, ids) in registries {
            by_registry
                .entry(qualified(registry))
                .or_default()
                .extend(ids.into_iter().map(qualified));
        }
        let mut hasher = std::hash::DefaultHasher::new();
        by_registry.hash(&mut hasher);
        Self {
            by_registry,
            fingerprint: hasher.finish(),
        }
    }
    fn contains(&self, registry: &str, id: &str) -> bool {
        self.by_registry
            .get(registry)
            .is_some_and(|ids| ids.contains(id))
    }
}
impl PartialEq for Names {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
}
impl Eq for Names {}
impl std::hash::Hash for Names {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.fingerprint.hash(state);
    }
}

/// An identifier with vanilla's default namespace (`Identifier.parse`).
fn qualified(id: &str) -> String {
    let id = id.strip_prefix('#').unwrap_or(id);
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// A lower-layer lookup over a raw condition node, including `type`.
/// `None` means the engine has no handler for this name; the engine removes
/// `type` only after finding a handler, avoiding argument copies without mods.
pub type ConditionLookup<'a> = dyn Fn(&str, &Value) -> Option<Result<bool, String>> + 'a;
type SharedLookup = dyn Fn(&str, &Value) -> Option<Result<bool, String>> + Send + Sync;
struct LookupState {
    inventories: std::sync::Mutex<BTreeMap<Vec<String>, Diagnostics>>,
    lookup: Box<SharedLookup>,
    owned: BTreeMap<String, String>,
}
#[derive(Clone)]
struct RegisteredLookup(std::sync::Arc<LookupState>);
impl std::fmt::Debug for RegisteredLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegisteredLookup")
    }
}
impl PartialEq for RegisteredLookup {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for RegisteredLookup {}
impl std::hash::Hash for RegisteredLookup {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&std::sync::Arc::as_ptr(&self.0), state);
    }
}

/// Whether a package is enabled for the owning data load.
pub type ModLoaded<'a> = dyn Fn(&str) -> bool + 'a;

/// Evaluate built-in conditions with an empty registration lookup.
pub fn conditions_hold(v: &Value, loaded: &ModLoaded) -> bool {
    conditions_hold_with_lookup(v, loaded, &|_, _| None, &BTreeMap::new())
        .expect("empty condition lookup cannot fail")
}

/// The shared evaluator, including engine-provided registered conditions.
/// Unknown nodes propagate through combinators without becoming a passing `not`.
pub fn conditions_hold_with_lookup(
    v: &Value,
    loaded: &ModLoaded,
    lookup: &ConditionLookup,
    owned: &BTreeMap<String, String>,
) -> Result<bool, String> {
    evaluate(v, loaded, lookup, owned, &Context::default(), &mut |_| {})
}

fn evaluate(
    v: &Value,
    loaded: &ModLoaded,
    lookup: &ConditionLookup,
    owned: &BTreeMap<String, String>,
    context: &Context,
    unknown: &mut dyn FnMut(&str),
) -> Result<bool, String> {
    fn hold(
        c: &Value,
        loaded: &ModLoaded,
        lookup: &ConditionLookup,
        owned: &BTreeMap<String, String>,
        context: &Context,
        unknown: &mut dyn FnMut(&str),
    ) -> Result<Option<bool>, String> {
        // Both NeoForge codecs default `registry` to `minecraft:item`.
        let registry = || qualified(c["registry"].as_str().unwrap_or("minecraft:item"));
        Ok(match c["type"].as_str() {
            Some("neoforge:mod_loaded") => Some(c["modid"].as_str().is_some_and(loaded)),
            Some("neoforge:true") => Some(true),
            Some("neoforge:false") => Some(false),
            Some("neoforge:feature_flags_enabled") => match &context.features {
                Some(enabled) => Some(c["flags"].as_array().into_iter().flatten().all(|flag| {
                    flag.as_str()
                        .is_some_and(|flag| enabled.contains(&qualified(flag)))
                })),
                None => {
                    unknown("neoforge:feature_flags_enabled (no feature flags in this load)");
                    None
                }
            },
            Some("neoforge:registered") => match &context.registries {
                Some(registries) => Some(
                    c["value"]
                        .as_str()
                        .is_some_and(|id| registries.contains(&registry(), &qualified(id))),
                ),
                None => {
                    unknown("neoforge:registered (no registries in this load)");
                    None
                }
            },
            Some("neoforge:tag_empty") => match &context.tags {
                Some(tags) => Some(
                    !c["tag"]
                        .as_str()
                        .is_some_and(|tag| tags.contains(&registry(), &qualified(tag))),
                ),
                None => {
                    unknown("neoforge:tag_empty (tags not loaded yet)");
                    None
                }
            },
            Some("neoforge:not") => {
                hold(&c["value"], loaded, lookup, owned, context, unknown)?.map(|v| !v)
            }
            Some(kind @ ("neoforge:and" | "neoforge:or")) => {
                // Visit every child: an unknown node invalidates the whole file,
                // including after a sibling already determines its boolean result.
                let mut answer = kind == "neoforge:and";
                let mut known = true;
                for child in c["values"].as_array().into_iter().flatten() {
                    match hold(child, loaded, lookup, owned, context, unknown)? {
                        Some(value) => {
                            if kind == "neoforge:and" {
                                answer &= value;
                            } else {
                                answer |= value;
                            }
                        }
                        None => known = false,
                    }
                }
                known.then_some(answer)
            }
            Some(name) => match lookup(name, c) {
                Some(answer) => Some(answer?),
                None => {
                    if let Some(owner) = name
                        .split_once(':')
                        .and_then(|(namespace, _)| owned.get(namespace))
                    {
                        return Err(format!(
                            "unknown condition {name} in namespace registered by mod {owner}"
                        ));
                    }
                    unknown(name);
                    None
                }
            },
            None => None,
        })
    }
    let mut allowed = true;
    for condition in v["neoforge:conditions"].as_array().into_iter().flatten() {
        allowed &= hold(condition, loaded, lookup, owned, context, unknown)?.unwrap_or(false);
    }
    Ok(allowed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn load_conditions_capture_package_ids_and_keep_their_generation() {
        let mut enabled = vec!["code_only".to_owned()];
        let old = LoadConditions::new(enabled.iter().map(String::as_str));
        enabled.clear();
        let next = LoadConditions::new(enabled.iter().map(String::as_str));
        let requires_code = json!({"neoforge:conditions":[{
            "type":"neoforge:mod_loaded", "modid":"code_only"
        }]});
        assert!(old.allows(&requires_code));
        assert!(!next.allows(&requires_code));
        assert!(old.mod_loaded("minecraft") && next.mod_loaded("minecraft"));
        assert!(!old.mod_loaded("resource_namespace"));
        assert!(old.allows(&json!({"values":[]})));
        assert!(!old.allows(&json!({"neoforge:conditions":[{"type":"future:condition"}]})));
    }

    #[test]
    fn load_condition_cache_identity_uses_membership_not_input_order() {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let key = |conditions: &LoadConditions| {
            let mut hasher = DefaultHasher::new();
            conditions.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(
            key(&LoadConditions::new(["b", "a", "a"])),
            key(&LoadConditions::new(["a", "b"]))
        );
        assert_ne!(
            key(&LoadConditions::new(["a"])),
            key(&LoadConditions::vanilla())
        );
        assert_eq!(
            LoadConditions::new(["minecraft"]),
            LoadConditions::vanilla()
        );
    }

    fn loaded(id: &str) -> bool {
        matches!(id, "minecraft" | "create")
    }

    #[test]
    fn a_file_without_conditions_loads() {
        assert!(conditions_hold(
            &json!({"type": "minecraft:crafting"}),
            &loaded
        ));
    }

    #[test]
    fn mod_loaded_follows_the_answer_and_minecraft_always_holds() {
        assert!(conditions_hold(
            &json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "create"}]}),
            &loaded
        ));
        assert!(!conditions_hold(
            &json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "ic2"}]}),
            &loaded
        ));
        assert!(conditions_hold(
            &json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "minecraft"}]}),
            &loaded
        ));
    }

    #[test]
    fn not_and_or_combine_and_all_must_hold() {
        let not = json!({"type": "neoforge:not", "value": {"type": "neoforge:mod_loaded", "modid": "ic2"}});
        let mut c = serde_json::Map::new();
        c.insert("neoforge:conditions".to_owned(), json!([not]));
        assert!(conditions_hold(&Value::Object(c.clone()), &loaded));
        let both = json!([
            {"type": "neoforge:mod_loaded", "modid": "create"},
            {"type": "neoforge:or", "values": [
                {"type": "neoforge:mod_loaded", "modid": "ic2"},
                {"type": "neoforge:true"}
            ]}
        ]);
        c.insert("neoforge:conditions".to_owned(), both);
        assert!(conditions_hold(&Value::Object(c), &loaded));
        // One failing condition in the list fails the file.
        let failing = json!([
            {"type": "neoforge:true"},
            {"type": "neoforge:mod_loaded", "modid": "ic2"}
        ]);
        let mut f = serde_json::Map::new();
        f.insert("neoforge:conditions".to_owned(), failing);
        assert!(!conditions_hold(&Value::Object(f), &loaded));
    }

    #[test]
    fn an_unknown_kind_nested_in_the_tree_leaves_the_file_out() {
        // `not` of `not` of an unknown: the unknown is two levels down.
        let nested_not = json!({
            "neoforge:conditions": [
                {"type": "neoforge:not", "value":
                    {"type": "neoforge:not", "value":
                        {"type": "example:unknown"}}}
            ]
        });
        assert!(!conditions_hold(&nested_not, &loaded));
        // `or` with a `true` beside a `not` of an unknown: one child is
        // known-true, but the other's subtree holds an unknown.
        let nested_or = json!({
            "neoforge:conditions": [
                {"type": "neoforge:or", "values": [
                    {"type": "neoforge:true"},
                    {"type": "neoforge:not", "value":
                        {"type": "example:unknown"}}
                ]}
            ]
        });
        assert!(!conditions_hold(&nested_or, &loaded));
    }

    fn file(condition: Value) -> Value {
        json!({"neoforge:conditions": [condition]})
    }

    #[test]
    fn false_omits_the_file_and_not_false_loads_it() {
        let conditions = LoadConditions::vanilla();
        assert!(!conditions.allows(&file(json!({"type": "neoforge:false"}))));
        assert!(conditions.allows(&file(
            json!({"type": "neoforge:not", "value": {"type": "neoforge:false"}})
        )));
        assert!(
            conditions.allows(&file(json!({"type": "neoforge:or", "values": [
                {"type": "neoforge:false"}, {"type": "neoforge:true"}
            ]})))
        );
    }

    #[test]
    fn registered_reads_the_loads_registries_and_defaults_to_items() {
        let stone = file(json!({"type": "neoforge:registered", "value": "minecraft:stone"}));
        let missing = file(
            json!({"type": "neoforge:registered", "registry": "minecraft:item", "value": "test:missing"}),
        );
        let biome = file(
            json!({"type": "neoforge:registered", "registry": "worldgen/biome", "value": "plains"}),
        );
        // No registries in this load: unsupported, so the file is left out.
        assert!(!LoadConditions::vanilla().allows(&stone));
        let conditions = LoadConditions::new([]).with_registries([
            ("minecraft:item", vec!["minecraft:stone"]),
            ("minecraft:worldgen/biome", vec!["minecraft:plains"]),
        ]);
        assert!(conditions.allows(&stone));
        assert!(!conditions.allows(&missing));
        assert!(conditions.allows(&biome));
    }

    #[test]
    fn feature_flags_enabled_needs_every_flag() {
        let both = file(json!({"type": "neoforge:feature_flags_enabled",
            "flags": ["minecraft:vanilla", "minecraft:trade_rebalance"]}));
        let vanilla = file(json!({"type": "neoforge:feature_flags_enabled", "flags": ["vanilla"]}));
        assert!(!LoadConditions::vanilla().allows(&vanilla));
        let default = LoadConditions::new([]).with_features([]);
        assert!(default.allows(&vanilla) && !default.allows(&both));
        let experiment = LoadConditions::new([])
            .with_features(["minecraft:vanilla", "minecraft:trade_rebalance"]);
        assert!(experiment.allows(&both));
    }

    #[test]
    fn tag_empty_reads_only_the_tags_this_load_bound() {
        let tin = file(json!({"type": "neoforge:tag_empty", "tag": "c:ingots/tin"}));
        let before_tags = LoadConditions::new([]);
        assert!(!before_tags.allows(&tin), "no tag view yet: left out");
        let empty = before_tags.with_tags([("minecraft:item", vec!["minecraft:planks"])]);
        assert!(empty.allows(&tin));
        let with_tin = before_tags.with_tags([("minecraft:item", vec!["c:ingots/tin"])]);
        assert!(!with_tin.allows(&tin));
        assert_ne!(empty, with_tin, "the tag view is part of cache identity");
    }

    #[test]
    fn a_retained_candidate_does_not_reuse_an_answer_across_tag_views() {
        let tin = file(json!({"type": "neoforge:tag_empty", "tag": "c:ingots/tin"}));
        let candidate = LoadConditions::new([]).with_lookup(BTreeMap::new(), |_, _| None);
        let empty = candidate.with_tags([("minecraft:item", Vec::<&str>::new())]);
        let with_tin = candidate.with_tags([("minecraft:item", vec!["c:ingots/tin"])]);
        assert!(empty.allows_file(&tin, "data/a/recipe/b.json"));
        assert!(!with_tin.allows_file(&tin, "data/a/recipe/b.json"));
    }

    #[test]
    fn element_conditions_drop_failing_elements_and_strip_passing_keys() {
        let mut recipe = json!({
            "type": "minecraft:crafting_shapeless",
            "ingredients": [
                "minecraft:stick",
                {"item": "create:zinc", "neoforge:conditions": [
                    {"type": "neoforge:mod_loaded", "modid": "create"}]},
                {"item": "ic2:tin", "neoforge:conditions": [
                    {"type": "neoforge:mod_loaded", "modid": "ic2"}]}
            ],
            "result": {"id": "minecraft:stone"}
        });
        LoadConditions::new(["create"]).filter_elements(&mut recipe, "data/a/recipe/b.json");
        assert_eq!(
            recipe["ingredients"],
            json!(["minecraft:stick", {"item": "create:zinc"}])
        );
    }

    #[test]
    fn an_unknown_condition_does_not_load_the_file() {
        let mut c = serde_json::Map::new();
        c.insert(
            "neoforge:conditions".to_owned(),
            json!([{"type": "neoforge:future_thing"}]),
        );
        assert!(!conditions_hold(&Value::Object(c), &loaded));
    }
}

#[cfg(test)]
#[path = "condition_registry_tests.rs"]
mod registry_tests;

#[cfg(test)]
#[path = "condition_log_tests.rs"]
mod log_tests;
