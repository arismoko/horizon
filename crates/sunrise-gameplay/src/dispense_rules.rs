//! Dispenser equipment and direction rules: slots, who may wear what, and the shared answer types.
use std::sync::Arc;
use sunrise_core::geometry::{Aabb, Direction, Vec3};
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::{Mob, Species};

pub type Pos = (i32, i32, i32);

/// The sound event of a plain click, a failed dispense and a launched projectile.
pub const CLICK: i32 = 1000;
pub const FAIL: i32 = 1001;
pub const LAUNCH: i32 = 1002;

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
/// `Direction.get3DDataValue`.
pub fn data_value(d: Direction) -> i32 {
    match d {
        Direction::Down => 0,
        Direction::Up => 1,
        Direction::North => 2,
        Direction::South => 3,
        Direction::West => 4,
        Direction::East => 5,
    }
}
/// `Direction.toYRot` (vertical directions give 270 through `data2d = -1`).
pub fn y_rot(d: Direction) -> f32 {
    match d {
        Direction::South => 0.0,
        Direction::West => 90.0,
        Direction::North => 180.0,
        Direction::East => 270.0,
        _ => 270.0,
    }
}
pub fn block_box(p: Pos) -> Aabb {
    Aabb::new(
        Vec3::new(f64::from(p.0), f64::from(p.1), f64::from(p.2)),
        Vec3::new(f64::from(p.0 + 1), f64::from(p.1 + 1), f64::from(p.2 + 1)),
    )
}
/// `forEachAccessibleNonEmptySection` order: x, then the packed z/y key.
pub fn section_order(at: Vec3) -> (i32, u32, u32) {
    let s = |v: f64| (v.floor() as i32) >> 4;
    let (x, y, z) = (s(at.x), s(at.y), s(at.z));
    (x, z as u32 & 0x3F_FFFF, y as u32 & 0xF_FFFF)
}
pub fn overlaps(a: Aabb, b: Aabb) -> bool {
    let (a0, a1, b0, b1) = (
        a.min_position(),
        a.max_position(),
        b.min_position(),
        b.max_position(),
    );
    a0.x < b1.x && b0.x < a1.x && a0.y < b1.y && b0.y < a1.y && a0.z < b1.z && b0.z < a1.z
}
/// `ItemStack.shrink(1)`; the slot is empty at zero.
pub fn shrink(mut stack: ItemStack) -> Option<ItemStack> {
    stack.count = stack.count.saturating_sub(1);
    (stack.count > 0).then_some(stack)
}
/// A dispenser's answer: the slot's new content and the level event played.
pub type Dispensed = (Option<ItemStack>, i32);

/// `EquipmentSlot`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    MainHand,
    OffHand,
    Feet,
    Legs,
    Chest,
    Head,
    Body,
    Saddle,
}
impl Slot {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "mainhand" => Self::MainHand,
            "offhand" => Self::OffHand,
            "feet" => Self::Feet,
            "legs" => Self::Legs,
            "chest" => Self::Chest,
            "head" => Self::Head,
            "body" => Self::Body,
            "saddle" => Self::Saddle,
            _ => return None,
        })
    }
    /// The player's armour array index (head, chest, legs, feet).
    pub fn armor_index(self) -> Option<usize> {
        match self {
            Self::Head => Some(0),
            Self::Chest => Some(1),
            Self::Legs => Some(2),
            Self::Feet => Some(3),
            _ => None,
        }
    }
    /// `ArmorStand` equipment index (mainhand, offhand, feet, legs, chest, head).
    pub fn stand_index(self) -> Option<usize> {
        match self {
            Self::MainHand => Some(0),
            Self::OffHand => Some(1),
            Self::Feet => Some(2),
            Self::Legs => Some(3),
            Self::Chest => Some(4),
            Self::Head => Some(5),
            _ => None,
        }
    }
}

fn equine(s: Species) -> bool {
    matches!(
        s,
        Species::Horse
            | Species::Donkey
            | Species::Mule
            | Species::SkeletonHorse
            | Species::ZombieHorse
            | Species::Camel
            | Species::CamelHusk
            | Species::Llama
            | Species::TraderLlama
    )
}
/// Mobs with species-specific body equipment.
pub fn body_equipment_species(s: Species) -> bool {
    equine(s)
        || matches!(s, Species::Wolf | Species::HappyGhast | Species::SulfurCube)
        || sunrise_sim::mobs::nautilus::is_nautilus(s)
}
/// What a mob holds in a slot.
pub fn mob_slot_item(m: &Mob, slot: Slot) -> Option<Arc<str>> {
    let x = &m.extra;
    match slot {
        Slot::MainHand | Slot::OffHand | Slot::Feet | Slot::Legs | Slot::Chest | Slot::Head => {
            sunrise_sim::mobs::finalize::item(m, slot as usize)
        }
        Slot::Body => x.body_stack().map(|s| s.item.clone()),
        Slot::Saddle => m.saddle_item().map(|s| s.item),
    }
}
/// `LivingEntity.canUseSlot` and its overrides.
pub fn mob_can_use_slot(m: &Mob, slot: Slot) -> bool {
    let s = m.species();
    let grown = m.alive() && !m.is_baby();
    match s {
        Species::Pig | Species::Strider if slot == Slot::Saddle => grown,
        Species::Horse
        | Species::SkeletonHorse
        | Species::ZombieHorse
        | Species::Llama
        | Species::TraderLlama => true,
        _ if equine(s) && slot == Slot::Saddle => grown && m.extra.tame,
        _ if sunrise_sim::mobs::nautilus::is_nautilus(s)
            && matches!(slot, Slot::Saddle | Slot::Body) =>
        {
            grown && m.extra.tame
        }
        Species::SulfurCube | Species::HappyGhast if slot == Slot::Body => grown,
        _ => true,
    }
}
/// `Mob.canDispenserEquipIntoSlot` and its overrides.
pub fn mob_can_dispenser_equip(m: &Mob, slot: Slot) -> bool {
    let s = m.species();
    let loot = m.extra.pick_up_loot;
    match s {
        Species::Pig | Species::Strider => slot == Slot::Saddle || loot,
        _ if equine(s) => matches!(slot, Slot::Body | Slot::Saddle) && m.extra.tame || loot,
        _ if sunrise_sim::mobs::nautilus::is_nautilus(s) => {
            matches!(slot, Slot::Body | Slot::Saddle) || loot
        }
        Species::SulfurCube | Species::HappyGhast => slot == Slot::Body,
        Species::Dolphin | Species::Panda | Species::Fox => slot == Slot::MainHand && loot,
        Species::Allay => false,
        _ => loot,
    }
}
/// `getEquipSound`: the saddle overrides of pigs, striders, horses, camels and nautili.
pub fn equip_sound(m: &Mob, slot: Slot, default: Option<String>) -> Option<String> {
    if slot != Slot::Saddle {
        return default;
    }
    let s = m.species();
    Some(match s {
        Species::Pig => "minecraft:entity.pig.saddle".into(),
        Species::Strider => "minecraft:entity.strider.saddle".into(),
        Species::Camel => "minecraft:entity.camel.saddle".into(),
        Species::CamelHusk => "minecraft:entity.camel_husk.saddle".into(),
        _ if equine(s) => "minecraft:entity.horse.saddle".into(),
        _ if sunrise_sim::mobs::nautilus::is_nautilus(s) => {
            if m.fluids.eye_in_water {
                "minecraft:item.nautilus_saddle_underwater_equip".into()
            } else {
                "minecraft:item.nautilus_saddle_equip".into()
            }
        }
        _ => return default,
    })
}
pub fn mob_set_slot(m: &mut Mob, slot: Slot, stack: ItemStack) {
    m.set_equipment_stack(slot as usize, Some(stack));
}
/// A tipped arrow's lasting effect at an eighth of the potion's duration.
pub fn tipped_effect(stack: &ItemStack) -> Option<(&'static str, i32)> {
    stack
        .potion()
        .and_then(sunrise_level::brewing::potion)
        .and_then(|def| {
            def.effects
                .iter()
                .find(|(id, _, _)| !sunrise_level::brewing::effect(id).is_some_and(|e| e.instant))
                .map(|(id, duration, _)| {
                    (
                        sunrise_level::brewing::namespaced(id),
                        (duration / 8).max(1),
                    )
                })
        })
}
