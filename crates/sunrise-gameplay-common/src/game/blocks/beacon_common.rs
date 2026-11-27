//! Common beacon constants, color/base queries, and beam scan shared by server and client.
use super::stations_common::Kind;
use sunrise_core::position::BlockPos;
use sunrise_level::containers::{Beacon, BlockEntity};
use sunrise_level::dyes::{DYE_DIFFUSE, DYE_NAMES};
use sunrise_world::registry::BlockStateId;

/// `BEACON_EFFECTS` in order, with the pyramid level each needs.
pub const EFFECTS: [(&str, i32); 6] = [
    ("speed", 1),
    ("haste", 1),
    ("resistance", 2),
    ("jump_boost", 2),
    ("strength", 3),
    ("regeneration", 4),
];

/// `ARGB.average`.
pub(in crate::game) fn average(a: u32, b: u32) -> u32 {
    let ch = |c: u32, s: u32| (c >> s) & 255;
    [16, 8, 0]
        .iter()
        .fold(0, |acc, &s| acc | ((ch(a, s) + ch(b, s)) / 2) << s)
}

/// `BeaconBlockEntity.applyEffects` box: `AABB(pos).inflate(range).expandTowards(0, height, 0)`,
/// tested with `AABB.intersects` (strict) against the player's bounding box.
pub fn effect_box_hits(p: [i32; 3], range: f64, height: f64, lo: [f64; 3], hi: [f64; 3]) -> bool {
    let [x, y, z] = p.map(f64::from);
    lo[0] < x + 1.0 + range
        && hi[0] > x - range
        && lo[1] < y + 1.0 + range + height
        && hi[1] > y - range
        && lo[2] < z + 1.0 + range
        && hi[2] > z - range
}

impl crate::game::CommonGame<'_> {
    /// The beam color of a `BeaconBeamBlock` (stained glass and panes, the beacon).
    pub(in crate::game) fn beam_color(&self, name: &str) -> Option<u32> {
        if name == "minecraft:beacon" {
            return Some(DYE_DIFFUSE[0]);
        }
        let n = name.strip_prefix("minecraft:")?;
        let dye = n
            .strip_suffix("_stained_glass")
            .or_else(|| n.strip_suffix("_stained_glass_pane"))?;
        DYE_NAMES
            .iter()
            .position(|d| *d == dye)
            .map(|i| DYE_DIFFUSE[i])
    }

    /// `getHeight(WORLD_SURFACE)`: above the highest non-air block.
    pub fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.world
            .world_surface(x, z, |id| self.res.facts().is_air(id))
    }

    /// `updateBase`: complete layers of `beacon_base_blocks` below.
    pub(in crate::game) fn beacon_base(&self, x: i32, y: i32, z: i32) -> i32 {
        let base = self
            .res
            .items()
            .block_tags
            .get("minecraft:beacon_base_blocks");
        let mut levels = 0;
        for step in 1..=4 {
            let ly = y - step;
            if ly < self.world.bottom_y() {
                break;
            }
            let ok = (x - step..=x + step).all(|lx| {
                (z - step..=z + step).all(|lz| {
                    let name = &self.res.facts().get(self.world.block(lx, ly, lz)).name;
                    base.is_some_and(|t| t.contains(&**name))
                })
            });
            if !ok {
                break;
            }
            levels = step;
        }
        levels
    }

    /// Opening the menu shows the beacon's current powers.
    pub fn beacon_opened(&mut self) {
        let Some(s) = self.station.as_ref().filter(|s| s.kind == Kind::Beacon) else {
            return;
        };
        let pos = s.pos;
        let (p, sec) = match self.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
            Some(BlockEntity::Beacon(b)) => (b.primary.clone(), b.secondary.clone()),
            _ => (None, None),
        };
        let index =
            |e: Option<String>| e.and_then(|e| EFFECTS.iter().position(|(n, _)| e.ends_with(n)));
        if let Some(s) = self.station.as_mut() {
            s.beacon = [index(p), index(sec)];
        }
    }
}

pub trait BeaconCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `IBlockStateExtension.getBeaconColorMultiplier` of `state` at `pos` for `beacon`.
    fn beacon_color(
        &self,
        state: BlockStateId,
        name: &str,
        pos: [i32; 3],
        beacon: [i32; 3],
    ) -> Option<u32> {
        let own = self.beam_color(name);
        if !super::api_values::block_state::block_listens(
            mod_api::HookId::BeaconColorMultiplier,
            state,
        ) {
            return own;
        }
        let mut color = own;
        let [x, y, z] = pos;
        let [bx, by, bz] = beacon;
        self.native_beacon_color_multiplier(
            state,
            BlockPos::new(x, y, z),
            BlockPos::new(bx, by, bz),
            &mut color,
        );
        color.map(|c| c & 0xFF_FFFF)
    }

    /// Shared 10-block beam & pyramid scan; returns `(previous_levels, ambient_ready, reached_surface)`.
    fn advance_beacon_beam(&self, (x, y, z): (i32, i32, i32), b: &mut Beacon) -> (i32, bool, bool) {
        let mut check = if b.last_check_y < y {
            b.checking.clear();
            b.last_check_y = y - 1;
            y
        } else {
            b.last_check_y + 1
        };
        let surface = self.world_surface(x, z);
        let mut i = 0;
        while i < 10 && check <= surface {
            let state = self.world.block(x, check, z);
            let f = self.res.facts().get(state);
            if let Some(color) = self.beacon_color(state, &f.name, [x, check, z], [x, y, z]) {
                if b.checking.len() <= 1 {
                    b.checking.push((color, 1));
                } else if let Some(last) = b.checking.last_mut() {
                    if color == last.0 {
                        last.1 += 1;
                    } else {
                        let mixed = average(last.0, color);
                        b.checking.push((mixed, 1));
                    }
                }
            } else {
                let Some(last) = b
                    .checking
                    .last_mut()
                    .filter(|_| f.dampening < 15 || &*f.name == "minecraft:bedrock")
                else {
                    b.checking.clear();
                    b.last_check_y = surface;
                    break;
                };
                last.1 += 1;
            }
            check += 1;
            b.last_check_y += 1;
            i += 1;
        }
        let previous = b.levels;
        let mut ambient_ready = false;
        if self.world.game_time % 80 == 0 {
            if !b.sections.is_empty() {
                b.levels = self.beacon_base(x, y, z);
            }
            ambient_ready = b.levels > 0 && !b.sections.is_empty();
        }
        let reached_surface = b.last_check_y >= surface;
        if reached_surface {
            b.last_check_y = self.world.bottom_y() - 1;
            b.sections = std::mem::take(&mut b.checking);
        }
        (previous, ambient_ready, reached_surface)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> BeaconCommonGameExt1<'__sg> for G {}

use crate::game::native_hooks::beacon_color_multiplier::GameHooks as _;
