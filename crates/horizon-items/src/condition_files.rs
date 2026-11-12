use super::*;
/// Select one JSON resource per ID before applying the owning load's conditions.
/// A skipped winning resource stays absent; it never uncovers a lower version.
pub(crate) fn data_files(root: &Path, kind: &str, overlay: &PackOverlay) -> HashMap<String, Value> {
    let mut files: HashMap<_, _> = horizon_world::damage_source::resource_files(root, kind)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, bytes)| {
            serde_json::from_slice::<Value>(&bytes)
                .ok()
                .map(|v| (id, v))
        })
        .collect();
    for (id, versions) in overlay.kind(kind) {
        if let Ok(value) = serde_json::from_slice::<Value>(PackOverlay::top(versions)) {
            files.insert(id.to_owned(), value);
        }
    }
    files.retain(|id, value| overlay.conditions.admit_id(value, kind, id));
    files
}

impl Items {
    /// Each tag registry's tags that hold an entry: the view of this load's
    /// bound tags that `neoforge:tag_empty` reads (`ICondition.IContext.getTag`).
    pub fn bound_tags(&self) -> impl Iterator<Item = (&str, impl Iterator<Item = &str>)> {
        [
            ("minecraft:block", &self.block_tags),
            ("minecraft:item", &self.item_tags),
            ("minecraft:fluid", &self.fluid_tags),
        ]
        .into_iter()
        .map(|(registry, tags)| {
            let bound = tags.iter().filter(|(_, members)| !members.is_empty());
            (registry, bound.map(|(tag, _)| tag.as_str()))
        })
    }
}
