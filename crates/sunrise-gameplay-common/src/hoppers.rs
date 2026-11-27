//! Hopper state and the item-column caches, with the block queries they share.
use sunrise_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
};
use sunrise_items::items::ItemStack;
use sunrise_level::{containers::BlockEntity, world::World};
use sunrise_sim::mobs::Mob;
use sunrise_world::registry::BlockStateId;

pub type Pos = (i32, i32, i32);

/// `HopperBlockEntity` transient state (`TransferCooldown`, `tickedGameTime`).
#[derive(Clone, Copy, Debug)]
pub struct HopperState {
    pub cooldown: i32,
    pub ticked: i64,
}
impl Default for HopperState {
    fn default() -> Self {
        Self {
            cooldown: -1,
            ticked: 0,
        }
    }
}

impl HopperState {
    /// A transferred holder keeps its cooldown and starts with a fresh ticker timestamp.
    pub fn with_cooldown(cooldown: i32) -> Self {
        Self {
            cooldown,
            ticked: 0,
        }
    }

    /// `HopperBlockEntity.loadAdditional`: only the cooldown is saved; the
    /// last tick time starts at zero for the new block entity.
    pub fn load(tag: &sunrise_core::nbt::Tag) -> Self {
        let canonical = sunrise_game_data::block_entity_nbt::canonical(tag);
        let cooldown = match canonical.as_ref().and_then(|t| t.get("TransferCooldown")) {
            Some(sunrise_core::nbt::Tag::Int(value)) => *value,
            _ => -1,
        };
        Self::with_cooldown(cooldown)
    }
}

/// The cooldown of the hopper at `pos`, from its state or its saved block entity, over the hoppers' state and the world.
pub fn hopper_cooldown_in(
    hoppers: &std::collections::HashMap<Pos, HopperState>,
    world: &sunrise_level::world::World,
    pos: Pos,
) -> i32 {
    hoppers.get(&pos).map_or_else(
        || {
            world
                .chunks
                .get(&(pos.0 >> 4, pos.2 >> 4))
                .and_then(|c| c.saved.as_deref())
                .and_then(|s| s.block_entity_sources.iter().find(|(p, _)| *p == pos))
                .map_or(-1, |(_, tag)| HopperState::load(tag).cooldown)
        },
        |s| s.cooldown,
    )
}

pub fn dir_from_name(name: &str) -> Option<Direction> {
    Some(match name {
        "down" => Direction::Down,
        "up" => Direction::Up,
        "north" => Direction::North,
        "south" => Direction::South,
        "west" => Direction::West,
        "east" => Direction::East,
        _ => return None,
    })
}
pub fn step(d: Direction) -> [i32; 3] {
    match d {
        Direction::Down => [0, -1, 0],
        Direction::Up => [0, 1, 0],
        Direction::North => [0, 0, -1],
        Direction::South => [0, 0, 1],
        Direction::West => [-1, 0, 0],
        Direction::East => [1, 0, 0],
    }
}
pub fn offset(p: Pos, d: Direction) -> Pos {
    let s = step(d);
    (p.0 + s[0], p.1 + s[1], p.2 + s[2])
}

/// Physical room for this item and components, in an empty or matching stack
/// (`HopperBlockEntity.canMergeItems`).
pub fn room(limit: u32, current: Option<&ItemStack>, stack: &ItemStack) -> u32 {
    match current {
        None => limit,
        Some(current) if current.same_item_same_components(stack) => {
            limit.saturating_sub(current.count)
        }
        Some(_) => 0,
    }
}

/// The block columns any item entity's box reaches, so that a hopper with
/// no item over it skips the scan of every item. Kept for one block-entity
/// pass, in which items do not move: removing one leaves a superset, still
/// right, and adding one (always pushed at the end, under a new id) or
/// removing the last rebuilds the columns.
#[derive(Default)]
pub struct ItemColumns {
    /// The item count and last item id when last asked, if built.
    seen: Option<(usize, Option<u32>)>,
    cells: sunrise_level::fast_hash::FxHashSet<(i32, i32)>,
}
impl ItemColumns {
    /// Whether an item's box may reach into column (x, z).
    pub fn may_reach(
        &mut self,
        items: &[sunrise_sim::entities::ItemEntity],
        x: i32,
        z: i32,
    ) -> bool {
        if items.is_empty() {
            return false;
        }
        let (len, last) = (items.len(), items.last().map(|i| i.id));
        let stale = self
            .seen
            .is_none_or(|(seen_len, seen_last)| len > seen_len || last != seen_last);
        self.seen = Some((len, last));
        if stale {
            self.cells.clear();
            for item in items {
                let b = item.bounds();
                let (lo, hi) = (b.min_position(), b.max_position());
                for cx in lo.x.floor() as i32..=hi.x.floor() as i32 {
                    for cz in lo.z.floor() as i32..=hi.z.floor() as i32 {
                        self.cells.insert((cx, cz));
                    }
                }
            }
        }
        self.cells.contains(&(x, z))
    }
    /// Whether an item's box may reach into any column of x0..=x1, z0..=z1.
    pub fn may_reach_area(
        &mut self,
        items: &[sunrise_sim::entities::ItemEntity],
        (x0, x1): (i32, i32),
        (z0, z1): (i32, i32),
    ) -> bool {
        (x0..=x1).any(|x| (z0..=z1).any(|z| self.may_reach(items, x, z)))
    }
}

/// [`ItemColumns`] for a pass in which items can move (a command, an
/// explosion's push), kept with the item list's
/// [`ListStamp`](sunrise_sim::join_seq::ListStamp): after any write the
/// first question scans as before (`true`), and the second with nothing
/// written in between builds the columns. A pass that writes the items
/// between questions costs no more than the scans did.
#[derive(Default)]
pub struct StampedItemColumns {
    stamp: Option<sunrise_sim::join_seq::ListStamp>,
    columns: ItemColumns,
}
impl StampedItemColumns {
    /// [`ItemColumns::may_reach_area`] for the items as they are now.
    pub fn may_reach_area(
        &mut self,
        items: &sunrise_sim::join_seq::JoinList<sunrise_sim::entities::ItemEntity>,
        xs: (i32, i32),
        zs: (i32, i32),
    ) -> bool {
        if self.stamp != Some(items.stamp()) {
            self.stamp = Some(items.stamp());
            self.columns = ItemColumns::default();
            return true;
        }
        self.columns.may_reach_area(items, xs, zs)
    }
}
/// `ChestBlock.isCatSittingOnChest`: a sitting cat whose box meets the block
/// above `key` (the box of the block above, not the chest's own).
pub fn cat_sitting_above(mobs: &[Mob], key: Pos) -> bool {
    let area = Aabb::new(
        Vec3::new(f64::from(key.0), f64::from(key.1 + 1), f64::from(key.2)),
        Vec3::new(
            f64::from(key.0 + 1),
            f64::from(key.1 + 2),
            f64::from(key.2 + 1),
        ),
    );
    mobs.iter().any(|m| {
        m.species() == sunrise_sim::mobs::Species::Cat
            && m.alive()
            && sunrise_sim::mobs::tame::cat_sitting(m)
            && m.bounds().intersects(area)
    })
}
/// `ChiseledBookShelfBlock.getAnalogOutputSignal` and
/// `ShelfBlock.getAnalogOutputSignal`, from the block entity Sunrise keeps as
/// it was loaded; `None` for any other block. `direction` points from the
/// block to the comparator.
pub fn shelf_signal(
    world: &World,
    registry: &'static sunrise_world::registry::BlockRegistry,
    state: BlockStateId,
    pos: BlockPos,
    direction: Direction,
) -> Option<i32> {
    let info = registry.state(state)?;
    let chiseled = info.name == "minecraft:chiseled_bookshelf";
    if !chiseled && !info.name.ends_with("_shelf") {
        return None;
    }
    if chiseled {
        // The last slot touched, plus one (`lastInteractedSlot` starts at -1).
        return Some(match world.block_entities.get(&(pos.x, pos.y, pos.z)) {
            Some(BlockEntity::ChiseledBookshelf(b)) => b.last_slot + 1,
            _ => 0,
        });
    }
    // Only read from the back: a bit for each of the three slots in use.
    let facing = info.properties.get("facing").and_then(|f| dir_from_name(f));
    if facing.map(Direction::opposite) != Some(direction) {
        return Some(0);
    }
    let Some(BlockEntity::Shelf(shelf)) = world.block_entities.get(&(pos.x, pos.y, pos.z)) else {
        return Some(0);
    };
    Some(
        shelf
            .items
            .iter()
            .enumerate()
            .map(|(slot, item)| i32::from(item.is_some()) << slot)
            .sum(),
    )
}
