//! Resolve shader packs' `layer.*` rules to the terrain mesher's per-state render layers.
use crate::game::ClientGame;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use sunrise_items::items::BlockTagFlags;
use sunrise_shaders::{BlockLayers, RenderLayer};
use sunrise_world::registry::{BlockRegistry, BlockStateId};

/// Number of opaque-cube states to identify before summarizing the rest.
const NOTED_STATES: usize = 4;

/// A pack's `layer.*` rules and the table they were last resolved to.
#[derive(Default)]
pub(super) struct PackLayers {
    /// The pack's name, for its notes.
    pack: String,
    rules: BlockLayers,
    resolved: Option<Resolved>,
}

/// A table of layers by state id and what it was resolved against.
struct Resolved {
    registry: &'static BlockRegistry,
    /// The world's block tag table when a rule uses a tag.
    tags: Option<Arc<BlockTagFlags>>,
    states: Arc<[Option<RenderLayer>]>,
}

impl Resolved {
    /// The cached table holds while its registry and any tag table have the same identity.
    fn resolves(&self, registry: &BlockRegistry, tags: Option<&Arc<BlockTagFlags>>) -> bool {
        std::ptr::eq(self.registry, registry)
            && match (&self.tags, tags) {
                (Some(held), Some(now)) => Arc::ptr_eq(held, now),
                (None, None) => true,
                _ => false,
            }
    }
}

impl PackLayers {
    pub(super) fn new(pack: &str, rules: BlockLayers) -> Self {
        Self {
            pack: pack.to_owned(),
            rules,
            resolved: None,
        }
    }
}

/// Resolve rules by state ID and report rules that attempted to move full opaque cubes.
fn layer_table(
    registry: &BlockRegistry,
    rules: &BlockLayers,
    tags: &HashMap<String, HashSet<String>>,
    solid_render: impl Fn(u32) -> bool,
) -> (Arc<[Option<RenderLayer>]>, Vec<String>) {
    let mut by_block: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, subject, tag) in rules.subjects() {
        if !tag {
            by_block.entry(subject).or_default().push(i);
        } else if let Some(members) = tags.get(subject) {
            for member in members {
                by_block.entry(member.as_str()).or_default().push(i);
            }
        }
    }
    // Each refusing rule, by line and word, with the first states it named (in id order) and
    // how many it named.
    let mut kept: Vec<((u32, &str), Vec<String>, usize)> = Vec::new();
    let mut table = Vec::with_capacity(registry.len());
    for i in 0..registry.len() as u32 {
        let layer = registry.state(BlockStateId(i)).and_then(|state| {
            let candidates = by_block.get(state.name.as_str())?;
            let rule = rules.pick(candidates.iter().copied(), |k| {
                state.properties.get(k).map(String::as_str)
            })?;
            if !solid_render(i) {
                return Some(rule.layer);
            }
            let at = match kept.iter().position(|(r, ..)| *r == (rule.line, rule.word)) {
                Some(at) => at,
                None => {
                    kept.push(((rule.line, rule.word), Vec::new(), 0));
                    kept.len() - 1
                }
            };
            let (_, named, count) = &mut kept[at];
            *count += 1;
            if named.len() == NOTED_STATES {
                return None;
            }
            let properties: Vec<_> = state
                .properties
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            named.push(format!("{}[{}]", state.name, properties.join(",")));
            None
        });
        table.push(layer);
    }
    let notes = kept
        .into_iter()
        .map(|((line, word), named, count)| {
            let more = match count - named.len() {
                0 => String::new(),
                more => format!(" and {more} more"),
            };
            format!(
                "line {line}: {word}: full opaque cubes keep their layer: {}{more}",
                named.join(" ")
            )
        })
        .collect();
    if table.iter().all(Option::is_none) {
        return (Arc::from([]), notes);
    }
    (table.into(), notes)
}

impl ClientGame<'_> {
    /// Apply the picked pack's per-state layers and remesh when the resolved table changes.
    pub(super) fn sync_pack_layers(&mut self) {
        let registry = self.common.pack.assets.registry;
        let facts = self.common.res.facts();
        let items = self
            .common
            .server_items
            .as_deref()
            .or_else(|| self.client.remote.as_ref()?.tags.as_ref())
            .unwrap_or_else(|| self.common.res.items());
        let wanted = match self.client.pack_ids.as_mut().map(|ids| &mut ids.layers) {
            Some(layers) if !layers.rules.is_empty() => {
                let tags = layers
                    .rules
                    .subjects()
                    .any(|(_, _, tag)| tag)
                    .then_some(&items.block_tag_flags);
                let current = layers
                    .resolved
                    .as_ref()
                    .is_some_and(|r| r.resolves(registry, tags));
                if !current {
                    let solid_render = |id: u32| {
                        facts
                            .states
                            .get(id as usize)
                            .is_some_and(|s| s.solid_render)
                    };
                    let (states, notes) =
                        layer_table(registry, &layers.rules, &items.block_tags, solid_render);
                    let pack = &layers.pack;
                    for note in notes {
                        log::warn!("shader pack {pack}: block.properties: {note}");
                    }
                    log::info!(
                        "shader pack {pack}: layer.* moves {} of {} block states",
                        states.iter().filter(|s| s.is_some()).count(),
                        registry.len()
                    );
                    layers.resolved = Some(Resolved {
                        registry,
                        tags: tags.cloned(),
                        states,
                    });
                }
                layers.resolved.as_ref().map(|r| r.states.clone())
            }
            _ => None,
        };
        let held = &self.client.streaming.layer_overrides;
        let changed = match &wanted {
            Some(states) => !Arc::ptr_eq(states, held) && **states != **held,
            None => !held.is_empty(),
        };
        if let Some(states) = wanted {
            self.client.streaming.layer_overrides = states;
        } else if changed {
            self.client.streaming.layer_overrides = Arc::from([]);
        }
        if changed {
            self.all_sections_changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_shaders::{Macros, Properties};

    fn rules(text: &str) -> BlockLayers {
        BlockLayers::parse(&Properties::parse(text, &Macros::standard()))
    }

    /// Slabs by type (not cubes), logs by axis and stone (full opaque cubes, ids 5 to 7).
    fn registry() -> BlockRegistry {
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:oak_slab":1,"minecraft:birch_slab":3,"minecraft:oak_log":6,"minecraft:stone":7},"states":[
            ["",true,false,"","minecraft:air"],
            ["",false,false,"",{"id":"minecraft:oak_slab","properties":{"type":"bottom"}}],
            ["",false,false,"",{"id":"minecraft:oak_slab","properties":{"type":"top"}}],
            ["",false,false,"",{"id":"minecraft:birch_slab","properties":{"type":"bottom"}}],
            ["",false,false,"",{"id":"minecraft:birch_slab","properties":{"type":"top"}}],
            ["",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"x"}}],
            ["",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"y"}}],
            ["",false,false,"","minecraft:stone"]]}"#;
        BlockRegistry::from_facts_json(facts.as_bytes()).unwrap()
    }

    fn cube(id: u32) -> bool {
        (5..=7).contains(&id)
    }

    fn tag(name: &str, members: &[&str]) -> HashMap<String, HashSet<String>> {
        HashMap::from([(
            name.to_owned(),
            members.iter().map(|m| (*m).to_owned()).collect(),
        )])
    }

    /// A rule can move a single slab type without moving the other.
    #[test]
    fn states_get_their_rules_layer_by_id() {
        let registry = registry();
        let text = "layer.translucent=oak_slab:type=bottom\nlayer.cutout=%slabs:type=top\n";
        let slabs = tag("minecraft:slabs", &["minecraft:birch_slab"]);
        let (table, notes) = layer_table(&registry, &rules(text), &slabs, cube);
        use RenderLayer::*;
        assert_eq!(
            *table,
            [
                None,
                Some(Translucent),
                None,
                None,
                Some(Cutout),
                None,
                None,
                None
            ]
        );
        assert!(notes.is_empty());
        // Without the tag's members, only the named state moves.
        let (table, _) = layer_table(&registry, &rules(text), &HashMap::new(), cube);
        assert_eq!(
            *table,
            [None, Some(Translucent), None, None, None, None, None, None]
        );
        // Rules that move no state of this registry leave the table empty: the no-pack path.
        let (table, notes) =
            layer_table(&registry, &rules("layer.solid=mod:thing\n"), &slabs, cube);
        assert!(table.is_empty() && notes.is_empty());
        let (table, _) = layer_table(&registry, &rules("block.5=stone\n"), &slabs, cube);
        assert!(table.is_empty());
    }

    /// Rules naming full opaque cubes by block or tag keep those cubes on their existing layers.
    #[test]
    fn full_opaque_cubes_keep_their_layer_with_a_note() {
        let registry = registry();
        let text = "layer.translucent=oak_log:axis=x oak_slab:type=top\nlayer.cutout=%logs stone\n";
        let logs = tag("minecraft:logs", &["minecraft:oak_log"]);
        let (table, notes) = layer_table(&registry, &rules(text), &logs, cube);
        assert_eq!(
            *table,
            [
                None,
                None,
                Some(RenderLayer::Translucent),
                None,
                None,
                None,
                None,
                None
            ]
        );
        assert_eq!(
            notes,
            [
                "line 1: oak_log:axis=x: full opaque cubes keep their layer: minecraft:oak_log[axis=x]",
                "line 2: %logs: full opaque cubes keep their layer: minecraft:oak_log[axis=y]",
                "line 2: stone: full opaque cubes keep their layer: minecraft:stone[]",
            ]
        );
        // A rule naming more states than a note lists names the first ones and counts the rest
        // (every state is a cube here: seven states through the tag).
        let every = tag(
            "minecraft:every",
            &[
                "minecraft:oak_slab",
                "minecraft:birch_slab",
                "minecraft:oak_log",
                "minecraft:stone",
            ],
        );
        let (table, notes) =
            layer_table(&registry, &rules("layer.solid=%every\n"), &every, |_| true);
        assert!(table.is_empty());
        assert_eq!(
            notes,
            ["line 1: %every: full opaque cubes keep their layer: \
              minecraft:oak_slab[type=bottom] minecraft:oak_slab[type=top] \
              minecraft:birch_slab[type=bottom] minecraft:birch_slab[type=top] and 3 more"]
        );
        // Rules naming only cubes move nothing: the table is the no-pack one.
        let (table, notes) = layer_table(&registry, &rules("layer.solid=stone\n"), &logs, cube);
        assert!(table.is_empty());
        assert_eq!(notes.len(), 1);
    }

    /// A new registry or referenced tag table invalidates the resolved layers.
    #[test]
    fn a_table_holds_for_its_registry_and_tags_only() {
        let (a, b): (&'static _, &'static _) = (
            Box::leak(Box::new(registry())),
            Box::leak(Box::new(registry())),
        );
        let tags = Arc::new(BlockTagFlags::default());
        let rebuilt = Arc::new(BlockTagFlags::default());
        let resolved = |tags: Option<Arc<BlockTagFlags>>| Resolved {
            registry: a,
            tags,
            states: Arc::from([]),
        };
        let untagged = resolved(None);
        assert!(untagged.resolves(a, None));
        assert!(!untagged.resolves(b, None));
        let tagged = resolved(Some(tags.clone()));
        assert!(tagged.resolves(a, Some(&tags)));
        assert!(!tagged.resolves(a, Some(&rebuilt)));
        assert!(!tagged.resolves(b, Some(&tags)));
        assert!(!tagged.resolves(a, None));
        assert!(!untagged.resolves(a, Some(&tags)));
    }
}
