use serde_json::Value;
use std::{collections::BTreeSet, sync::Arc};
use sunrise_game_data::text::Text;
use sunrise_sim::advancements::{self as adv, EntityView, Progress, Tree};

/// Where the player's fall, lava ride and Nether trip began, the blast that
/// last hit the player, and the effects the last tick saw (`ServerPlayer`'s
/// fields of the same names). A guest keeps their own, as `levitation` does.
#[derive(Clone, Debug, Default)]
pub struct TravelMarks {
    /// `startingToFallPosition`.
    pub fall: Option<[f64; 3]>,
    /// `currentExplosionImpactPos` and `currentExplosionCause` (the entity
    /// that made the blast, if any).
    pub blast: Option<([f64; 3], Option<EntityView>)>,
    /// `enteredLavaOnVehiclePosition`.
    pub lava_ride: Option<[f64; 3]>,
    /// `enteredNetherPosition`.
    pub nether: Option<[f64; 3]>,
    /// The (effect, amplifier) pairs `EffectsChangedTrigger` last saw.
    pub effects: Vec<(&'static str, i32)>,
    /// `PlayerEffects::changes` as the last tick saw it.
    pub effect_changes: u32,
}
#[derive(Default)]
pub struct AdvState {
    pub tree: Arc<Tree>,
    pub remote_tree: Option<RemoteTree>,
    /// `ClientAdvancements.selectedTab`: the connection remembers the root
    /// independently of the open screen's tabs and the current tree.
    pub selected_tab: Option<String>,
    pub progress: Progress,
    pub loaded: bool,
    /// Awards skip the toast (`/advancement grant ... everything`).
    pub quiet: bool,
    /// The inventory last seen by `inventory_changed`.
    pub last_inventory: u64,
    /// `ServerPlayer.levitationStartPos` and `levitationStartTime`.
    pub levitation: Option<([f64; 3], u64)>,
    /// The fall, lava ride, Nether trip, blast and effects `tick_marks` follows.
    pub marks: TravelMarks,
    /// While the game acts for a network player (`as_guest`): the
    /// advancements whose progress changed (`progressChanged`), the recipes
    /// unlocked and the announcements, for their client and everyone's.
    pub changed: BTreeSet<usize>,
    pub unlocked: Vec<String>,
    pub announced: Vec<Text>,
    /// `inventory_changed`'s criteria by item, built for `tree`.
    pub inventory_index: Option<(Arc<Tree>, Arc<adv::InventoryIndex>)>,
}
impl AdvState {
    /// Drop the index built for the previous tree after a new one is installed.
    pub fn reset_tree(&mut self) {
        self.inventory_index = None;
    }
}
/// The packet tree's insertion-time links. The current ID table in `Tree`
/// can replace a node without replacing the node its parent or children know.
#[derive(Default)]
pub struct RemoteTree {
    pub positions: Vec<(f32, f32)>,
    /// Node identity for each ID in `Tree::list` order.
    pub current: Vec<usize>,
    pub nodes: Vec<RemoteNode>,
}

pub struct RemoteNode {
    pub id: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub displayed: bool,
}

impl RemoteTree {
    /// Root and nearest displayed parent of each current node, resolved by
    /// ID only after walking the actual insertion-time ancestors.
    pub fn widget_links<'a>(
        &'a self,
        tree: &'a Tree,
    ) -> impl Iterator<Item = (Option<usize>, Option<usize>)> + 'a {
        // Every parent precedes its child in the arena, including after compaction.
        let mut ancestry: Vec<(usize, Option<usize>)> = Vec::with_capacity(self.nodes.len());
        for (i, node) in self.nodes.iter().enumerate() {
            let links = node.parent.map_or((i, None), |parent| {
                let (root, ancestor) = ancestry[parent];
                (
                    root,
                    if self.nodes[parent].displayed {
                        Some(parent)
                    } else {
                        ancestor
                    },
                )
            });
            ancestry.push(links);
        }
        let current_index = move |i: usize| tree.by_id.get(&self.nodes[i].id).copied();
        self.current.iter().map(move |&node| {
            let (root, displayed_parent) = ancestry[node];
            (
                current_index(root),
                displayed_parent.and_then(current_index),
            )
        })
    }

    /// Release identities no current node can reach in either direction.
    /// A removed or overwritten node remains as long as its links still matter.
    pub fn prune_unreachable(&mut self) {
        if self.nodes.len() == self.current.len() {
            return;
        }
        let mut live = vec![false; self.nodes.len()];
        let mut stack = self.current.clone();
        while let Some(i) = stack.pop() {
            if !std::mem::replace(&mut live[i], true) {
                stack.extend(self.nodes[i].parent);
                stack.extend(self.nodes[i].children.iter().copied());
            }
        }
        if live.iter().all(|&yes| yes) {
            return;
        }
        let mut new_indices = vec![0; live.len()];
        let (mut i, mut next) = (0, 0);
        self.nodes.retain(|_| {
            let keep = live[i];
            if keep {
                new_indices[i] = next;
                next += 1;
            }
            i += 1;
            keep
        });
        for node in &mut self.nodes {
            node.parent = node.parent.map(|i| new_indices[i]);
            for child in &mut node.children {
                *child = new_indices[*child];
            }
        }
        for node in &mut self.current {
            *node = new_indices[*node];
        }
    }
}

/// The tag tables a predicate's `#tag` reads, as borrows of the game's fields.
#[derive(Clone, Copy)]
pub struct TagSources<'a> {
    /// The world's packs' tags over the game's own, by registry.
    pub pack_tags: Option<
        &'a std::collections::HashMap<
            &'static str,
            std::collections::HashMap<String, std::collections::HashSet<String>>,
        >,
    >,
    pub items: &'a sunrise_items::items::Items,
    pub mob_data: &'a sunrise_sim::mobs::MobData,
    pub damage: &'a sunrise_world::damage_source::Catalog,
    pub resources: &'a std::path::Path,
}
impl TagSources<'_> {
    pub fn contains(&self, registry: &str, tag: &str, id: &str) -> bool {
        // The world's packs' tags over the game's own, as the server's
        // registries bind them (`TagLoader.loadTagsForExistingRegistries`).
        if let Some(tags) = self.pack_tags.and_then(|t| t.get(registry)) {
            return tags.get(tag).is_some_and(|t| t.contains(id));
        }
        match registry {
            "item" => self.items.item_in_tag(tag, id),
            "block" => self.items.block_in_tag(tag, id),
            "damage_type" => self
                .damage
                .id(id)
                .is_some_and(|source| self.damage.in_tag(source, tag)),
            "entity_type" => self.items.entity_type_tags.contains(tag, id),
            "worldgen/biome" => self.mob_data.biome_has(tag, id),
            "worldgen/structure" => vanilla_structure_tags(self.resources)
                .get(tag)
                .is_some_and(|t| t.contains(id)),
            _ => false,
        }
    }
}
pub(in crate::game) type Tags =
    std::collections::HashMap<String, std::collections::HashSet<String>>;
/// The game's own `worldgen/structure` tags (a world without data packs has
/// no `WorldTables` to hold them), loaded once for the resource root first
/// asked for.
pub(in crate::game) fn vanilla_structure_tags(root: &std::path::Path) -> &'static Tags {
    static TAGS: std::sync::OnceLock<Tags> = std::sync::OnceLock::new();
    TAGS.get_or_init(|| {
        sunrise_items::items::load_tags(
            root,
            "worldgen/structure",
            &sunrise_world::loot::PackOverlay::NONE,
        )
        .unwrap_or_default()
    })
}
/// An advancement's title or description: a whole component (ComponentSerialization.CODEC),
/// with its siblings and styles, as a plugin's `{"text":"","extra":[...]}` needs.
pub fn text_of(v: &Value) -> Text {
    sunrise_game_data::scoreboard::comp_text(v)
}
