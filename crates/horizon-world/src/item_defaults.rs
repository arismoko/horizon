//! A shared snapshot of the item defaults loaded after registries freeze.
//! Native changes are applied by the owning load, never by metadata readers.

use crate::native_load_context::LoadContext;
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
};
mod component_text;
pub use component_text::{strip_transient_component_map_tag, strip_transient_component_tag};
pub use component_text::{
    validate_component_text, validate_component_text_tag, validate_dialog, validate_dialog_tag,
    validate_style, validate_style_tag, validate_text, validate_text_tag,
};

/// An immutable default-component snapshot. Kept only while a resource load
/// holds it: a later load reads the files and applies native hooks afresh.
pub type Defaults = BTreeMap<String, Value>;
static SHARED: Mutex<Vec<(PathBuf, Weak<ItemDefaults>)>> = Mutex::new(Vec::new());

/// A resource load already reading and applying hooks to `items.json` for a
/// root. Several startup tables ask for defaults in parallel; without this
/// gate they can all miss `SHARED` and rerun the native hook for every item.
struct InFlight {
    result: Mutex<Option<Result<Arc<ItemDefaults>, (io::ErrorKind, String)>>>,
    ready: Condvar,
}
static IN_FLIGHT: OnceLock<Mutex<std::collections::HashMap<PathBuf, Arc<InFlight>>>> =
    OnceLock::new();

fn in_flight() -> &'static Mutex<std::collections::HashMap<PathBuf, Arc<InFlight>>> {
    IN_FLIGHT.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// `DataComponentType.codec`: these vanilla types have no persistent data form.
/// Value/removal readers reject them; type-only lookups and network codecs do not.
pub fn is_transient_component(id: &str) -> bool {
    matches!(
        id,
        "minecraft:creative_slot_lock"
            | "minecraft:additional_trade_cost"
            | "minecraft:map_post_processing"
    )
}

/// `DataComponentPatch.PatchKey.CODEC`: resolve a persistent SET or REMOVE name.
/// Registration and payload validation remain the owning codec's responsibility.
pub fn validate_component_name(name: &str) -> Result<(), String> {
    let name = name.strip_prefix('!').unwrap_or(name);
    let mut scratch = [0u8; 128];
    let id = if name.contains(':') {
        name
    } else {
        // Stock transient names fit this buffer; longer unknown names are
        // left to the owning registry codec without allocating a temporary ID.
        let prefix = b"minecraft:";
        let length = prefix.len() + name.len();
        if length > scratch.len() {
            return Ok(());
        }
        scratch[..prefix.len()].copy_from_slice(prefix);
        scratch[prefix.len()..length].copy_from_slice(name.as_bytes());
        std::str::from_utf8(&scratch[..length]).expect("UTF-8 namespace and component")
    };
    if is_transient_component(id) {
        return Err(format!("'{id}' is not a persistent component"));
    }
    Ok(())
}

/// A known `DataComponentPatch` or exact-value map, not arbitrary JSON.
/// Only component codecs containing stacks or exact predicates recurse.
pub fn validate_component_map(value: &Value) -> Result<(), String> {
    let Some(map) = value.as_object() else {
        return Ok(());
    };
    for (name, payload) in map {
        validate_component_name(name)?;
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        match name {
            "container" => {
                for slot in payload.as_array().into_iter().flatten() {
                    validate_stack(&slot["item"])?;
                }
            }
            "bundle_contents" | "charged_projectiles" => {
                for stack in payload.as_array().into_iter().flatten() {
                    validate_stack(stack)?;
                }
            }
            "use_remainder" | "sulfur_cube_content" => validate_stack(payload)?,
            "lock" => crate::loot::validate_item_predicate(payload)?,
            "can_break" | "can_place_on" => {
                if let Some(predicates) = payload.as_array() {
                    for predicate in predicates {
                        crate::loot::validate_item_predicate(predicate)?;
                    }
                } else {
                    crate::loot::validate_item_predicate(payload)?;
                }
            }
            _ => {}
        }
        component_text::validate_component_text(name, payload)?;
    }
    Ok(())
}

/// `ItemStack.CODEC` / `ItemStackTemplate.CODEC`'s persistent component patch.
pub fn validate_stack(value: &Value) -> Result<(), String> {
    validate_component_map(&value["components"])
}

/// NBT counterpart of the persistent map boundary. It borrows payloads rather
/// than converting them to JSON; runtime/network component decoding stays separate.
pub fn validate_component_tag(id: &str, payload: &horizon_core::nbt::Tag) -> Result<(), String> {
    use horizon_core::nbt::Tag;
    validate_component_name(id)?;
    match id.strip_prefix("minecraft:").unwrap_or(id) {
        "container" => {
            if let Tag::List(slots) = payload {
                for slot in slots {
                    if let Some(item) = slot.get("item") {
                        validate_stack_tag(item)?;
                    }
                }
            }
        }
        "bundle_contents" | "charged_projectiles" => {
            if let Tag::List(stacks) = payload {
                for stack in stacks {
                    validate_stack_tag(stack)?;
                }
            }
        }
        "use_remainder" | "sulfur_cube_content" => validate_stack_tag(payload)?,
        "lock" => validate_item_predicate_tag(payload)?,
        "can_break" | "can_place_on" => {
            if let Tag::List(predicates) = payload {
                for predicate in predicates {
                    validate_item_predicate_tag(predicate)?;
                }
            } else {
                validate_item_predicate_tag(payload)?;
            }
        }
        _ => {}
    }
    component_text::validate_component_text_tag(id, payload)?;
    Ok(())
}
/// Borrowed persistent component-map boundary. Unknown/native payloads stay opaque.
pub fn validate_component_map_tag(value: &horizon_core::nbt::Tag) -> Result<(), String> {
    if let horizon_core::nbt::Tag::Compound(map) = value {
        let mut scratch = [0u8; 128];
        for (id, payload) in map {
            if let Some(id) = ascii_name(id, &mut scratch) {
                validate_component_tag(id, payload)?;
            }
        }
    }
    Ok(())
}
/// `ItemStackTemplate.CODEC` / `ItemStack.CODEC`, not their network codecs.
pub fn validate_stack_tag(value: &horizon_core::nbt::Tag) -> Result<(), String> {
    if let Some(components) = value.get("components") {
        validate_component_map_tag(components)?;
    }
    Ok(())
}
/// Known vanilla codec names are ASCII; long/native or non-ASCII names are opaque.
pub(crate) fn ascii_name<'a>(
    name: &horizon_core::nbt::JavaString,
    scratch: &'a mut [u8; 128],
) -> Option<&'a str> {
    if name.0.len() > scratch.len() {
        return None;
    }
    for (out, unit) in scratch.iter_mut().zip(&name.0) {
        if *unit > 127 {
            return None;
        }
        *out = *unit as u8;
    }
    std::str::from_utf8(&scratch[..name.0.len()]).ok()
}
/// Persistent exact item/block predicate names; any-value type predicates stay valid.
pub fn validate_item_predicate_tag(value: &horizon_core::nbt::Tag) -> Result<(), String> {
    use horizon_core::nbt::Tag;
    validate_stack_tag(value)?;
    if let Some(Tag::Compound(predicates)) = value.get("predicates") {
        for (name, value) in predicates {
            let mut scratch = [0u8; 128];
            let Some(name) = ascii_name(name, &mut scratch) else {
                continue;
            };
            validate_partial_predicate_tag(name, value)?;
        }
    }
    Ok(())
}

/// Only the stock collection sub-predicates contain nested item predicates.
pub fn validate_partial_predicate_tag(
    id: &str,
    value: &horizon_core::nbt::Tag,
) -> Result<(), String> {
    use horizon_core::nbt::Tag;
    if matches!(
        id.strip_prefix("minecraft:").unwrap_or(id),
        "container" | "bundle_contents" | "charged_projectiles"
    ) && let Some(items) = value.get("items")
    {
        if let Some(Tag::List(contains)) = items.get("contains") {
            for item in contains {
                validate_item_predicate_tag(item)?;
            }
        }
        if let Some(Tag::List(counts)) = items.get("count") {
            for count in counts {
                if let Some(test) = count.get("test") {
                    validate_item_predicate_tag(test)?;
                }
            }
        }
    }
    Ok(())
}

/// Share parsed defaults with the other tables built during this load. Callers
/// that need a fresh read regardless of other loads can still use [`read`].
pub fn shared(root: &Path) -> io::Result<Arc<ItemDefaults>> {
    let held = || {
        SHARED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(path, _)| path == root)
            .and_then(|(_, value)| value.upgrade())
    };
    if let Some(value) = held() {
        return Ok(value);
    }

    let (flight, leader) = {
        let mut flights = in_flight().lock().unwrap_or_else(|e| e.into_inner());
        match flights.entry(root.to_owned()) {
            std::collections::hash_map::Entry::Occupied(entry) => (entry.get().clone(), false),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let flight = Arc::new(InFlight {
                    result: Mutex::new(None),
                    ready: Condvar::new(),
                });
                entry.insert(flight.clone());
                (flight, true)
            }
        }
    };
    if !leader {
        let mut result = flight.result.lock().unwrap_or_else(|e| e.into_inner());
        while result.is_none() {
            result = flight.ready.wait(result).unwrap_or_else(|e| e.into_inner());
        }
        return result
            .clone()
            .expect("item defaults flight result")
            .map_err(|(kind, message)| io::Error::new(kind, message));
    }

    // Native hooks run without either cache lock, so they can read other data.
    let result = read(root)
        .map(Arc::new)
        .map_err(|e| (e.kind(), e.to_string()));
    if let Ok(value) = &result {
        let mut cache = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(other) = cache
            .iter()
            .find(|(path, _)| path == root)
            .and_then(|(_, v)| v.upgrade())
        {
            drop(cache);
            let result = Ok(other);
            let mut published = flight.result.lock().unwrap_or_else(|e| e.into_inner());
            *published = Some(result.clone());
            flight.ready.notify_all();
            in_flight()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(root);
            return result.map_err(|(kind, message)| io::Error::new(kind, message));
        }
        cache.retain(|(_, value)| value.strong_count() > 0);
        cache.push((root.to_owned(), Arc::downgrade(value)));
    }
    {
        let mut published = flight.result.lock().unwrap_or_else(|e| e.into_inner());
        *published = Some(result.clone());
        flight.ready.notify_all();
    }
    in_flight()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(root);
    result.map_err(|(kind, message)| io::Error::new(kind, message))
}

/// The largest stack size a native mod may set: `ItemStack.count` is a `u32`,
/// but the count is a VarInt on the wire and is saved as an `int`, so a
/// larger one would turn negative (and lose its item) on the way out.
pub const MAX_STACK_SIZE: i64 = i32::MAX as i64;

/// Vanilla's `ExtraCodecs.intRange(1, 99)` on a data pack's own value.
const VANILLA_MAX_STACK_SIZE: i64 = 99;
const STACK_SIZE: &str = "minecraft:max_stack_size";

/// `value` as a stack size in `1..=max`, or None when it is not a number.
fn stack_size(value: &Value, max: i64) -> Option<i64> {
    value.as_f64().map(|n| (n as i64).clamp(1, max))
}

// Root-keyed caches must also distinguish two snapshots of the same files.
fn next_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Hook point for NeoForge's `ModifyDefaultComponentsEvent`: asked once per
/// item after registries freeze. Metadata readers share the resulting snapshot;
/// a datapack reload does not replay this event.
#[inline(never)]
pub fn native_modify_default_components(
    context: &LoadContext,
    item: &str,
    components: &mut Map<String, Value>,
) {
    crate::native_load_hooks::modify_default_components(context, item, components);
}

type Prototypes = BTreeMap<String, BTreeMap<String, Value>>;

/// One owning load's published defaults and metadata, shared across loader workers.
pub struct ItemData {
    root: PathBuf,
    context: LoadContext,
    defaults: ItemDefaults,
    prototypes: Result<Arc<Prototypes>, Arc<serde_json::Error>>,
}

impl ItemData {
    pub fn load(root: &Path, context: LoadContext) -> io::Result<Self> {
        let mut defaults = read(root)?;
        defaults.apply_native(&context);
        let prototypes = defaults
            .entries()
            .iter()
            .map(|(item, components)| {
                serde_json::from_value(components.clone())
                    .map(|components| (item.clone(), components))
            })
            .collect::<serde_json::Result<Prototypes>>()
            .map(Arc::new)
            .map_err(Arc::new);
        Ok(Self {
            root: root.to_owned(),
            context,
            defaults,
            prototypes,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn defaults(&self) -> &ItemDefaults {
        &self.defaults
    }

    pub fn context(&self) -> &LoadContext {
        &self.context
    }

    /// Borrow the canonical table; conversion errors retain their renderer-load timing.
    pub fn prototypes(&self) -> io::Result<&Arc<Prototypes>> {
        self.prototypes
            .as_ref()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, Arc::clone(error)))
    }
}

/// One resource generation's default components. The owning loader applies
/// native changes before publishing this; consumers only borrow its entries.
pub struct ItemDefaults {
    entries: BTreeMap<String, Value>,
    generation: u64,
}

impl ItemDefaults {
    pub fn entries(&self) -> &BTreeMap<String, Value> {
        &self.entries
    }

    /// Identity for caches of metadata derived from this snapshot.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Apply the registration-time event before the snapshot is shared.
    pub(crate) fn apply_native(&mut self, context: &LoadContext) {
        for (item, components) in &mut self.entries {
            let Value::Object(components) = components else {
                continue;
            };
            let pack = components.get(STACK_SIZE).cloned();
            native_modify_default_components(context, item, components);
            // Hooks bypass the codec. Preserve removal, bound numeric sizes
            // to the wire/save range, and keep the pack value for a non-number.
            match components.get(STACK_SIZE).cloned() {
                None => {}
                Some(size) => match stack_size(&size, MAX_STACK_SIZE) {
                    Some(size) => {
                        components.insert(STACK_SIZE.into(), size.into());
                    }
                    None => match pack {
                        Some(pack) => {
                            components.insert(STACK_SIZE.into(), pack);
                        }
                        None => {
                            components.remove(STACK_SIZE);
                        }
                    },
                },
            }
        }
        self.generation = next_generation();
    }
}

/// Decode a default-component report, keeping a data pack's stack sizes in
/// vanilla's `1..=99`. This is a codec, not a native firing boundary: the owning
/// loader calls `apply_native` once, then publishes the immutable snapshot.
pub fn read(root: &Path) -> io::Result<ItemDefaults> {
    let mut entries: BTreeMap<String, Value> =
        serde_json::from_slice(&fs::read(root.join("items.json"))?)?;
    for components in entries.values_mut() {
        if let Some(value) = components.get_mut(STACK_SIZE)
            && let Some(size) = stack_size(value, VANILLA_MAX_STACK_SIZE)
        {
            *value = size.into();
        }
    }
    Ok(ItemDefaults {
        entries,
        generation: next_generation(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[test]
    fn a_load_shares_one_snapshot_and_the_next_load_reads_changes() {
        let root =
            std::env::temp_dir().join(format!("horizon-default-snapshot-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let write = |size| {
            fs::write(
                root.join("items.json"),
                format!(r#"{{"test:item":{{"minecraft:max_stack_size":{size}}}}}"#),
            )
            .unwrap()
        };
        write(16);
        let first = super::shared(&root).unwrap();
        write(64);
        let second = super::shared(&root).unwrap();
        assert_eq!(
            second.entries()["test:item"]["minecraft:max_stack_size"],
            16
        );
        // Explicit fresh reads remain available even while a load owns its snapshot.
        assert_eq!(
            super::read(&root).unwrap().entries()["test:item"]["minecraft:max_stack_size"],
            64
        );
        drop((first, second));
        let next = super::shared(&root).unwrap();
        assert_eq!(next.entries()["test:item"]["minecraft:max_stack_size"], 64);
        drop(next);
        fs::write(root.join("items.json"), "broken").unwrap();
        assert!(super::shared(&root).is_err());
        write(2);
        assert_eq!(
            super::shared(&root).unwrap().entries()["test:item"]["minecraft:max_stack_size"],
            2
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// A data pack's stack size is kept to vanilla's 1..=99, so stack
    /// arithmetic never sees a 0, and a mod may raise it above 99 (up to the
    /// count the wire and the save carry).
    #[test]
    fn data_pack_sizes_stay_in_vanilla_range() {
        let root = std::env::temp_dir().join(format!("horizon-stack-sizes-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let items = r#"{"a:none": {"minecraft:max_stack_size": 0},
            "a:less": {"minecraft:max_stack_size": -5},
            "a:many": {"minecraft:max_stack_size": 500},
            "a:some": {"minecraft:max_stack_size": 16}, "a:unset": {}}"#;
        fs::write(root.join("items.json"), items).unwrap();
        let items = super::read(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();
        let size = |id: &str| items.entries()[id].get("minecraft:max_stack_size").cloned();
        assert_eq!(
            ["a:none", "a:less", "a:many", "a:some", "a:unset"].map(size),
            [
                Some(1.into()),
                Some(1.into()),
                Some(99.into()),
                Some(16.into()),
                None
            ]
        );
    }

    /// A mod's own size is kept to 1..=i32::MAX, a value that is not a number
    /// leaves the data pack's, and a removed component stays removed.
    #[test]
    fn mod_sizes_are_sanitised() {
        crate::native_load_hooks::testing::install();
        let root = std::env::temp_dir().join(format!("horizon-mod-sizes-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let items = r#"{"a:raised": {"minecraft:max_stack_size": 64},
            "a:zero": {"minecraft:max_stack_size": 16},
            "a:huge": {"minecraft:max_stack_size": 64},
            "a:string": {"minecraft:max_stack_size": 16},
            "a:removed": {"minecraft:max_stack_size": 64}}"#;
        fs::write(root.join("items.json"), items).unwrap();
        let inputs =
            crate::native_load_context::LoadInputs::new(Default::default(), root.join("settings"));
        let context = crate::native_load_context::LoadContext::new(
            crate::native_load_context::DataLoadOrigin::ServerDataLoad,
            std::sync::Arc::new(inputs),
            [],
        );
        let mut items = super::read(&root).unwrap();
        items.apply_native(&context);
        fs::remove_dir_all(&root).unwrap();
        let size = |id: &str| items.entries()[id].get("minecraft:max_stack_size").cloned();
        assert_eq!(
            ["a:raised", "a:zero", "a:huge", "a:string", "a:removed"].map(size),
            [
                Some(500.into()),
                Some(1.into()),
                Some(super::MAX_STACK_SIZE.into()),
                Some(16.into()),
                None
            ]
        );
    }
}
