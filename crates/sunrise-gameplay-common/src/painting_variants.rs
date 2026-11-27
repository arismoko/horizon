//! Painting variants and the hanging-entity placement rules.
use std::sync::Arc;
use sunrise_items::items::ItemStack;
use sunrise_sim::entities::{ItemFrame, Painting};

pub use sunrise_sim::entities::PaintingVariant as Variant;
/// Variants by id plus the `#minecraft:placeable` list.
#[derive(Clone, Default)]
pub struct PaintingVariants {
    pub all: Vec<Variant>,
    pub placeable: Vec<Arc<str>>,
}
impl PaintingVariants {
    pub fn load(root: &std::path::Path) -> Self {
        let data = root.join("data/minecraft");
        let mut all = Vec::new();
        if let Ok(folder) = sunrise_world::json_folder::read(&data.join("painting_variant")) {
            for file in &folder.files {
                let name = file.name.as_str();
                let Ok(v) =
                    serde_json::from_slice::<serde_json::Value>(file.bytes().unwrap_or_default())
                else {
                    continue;
                };
                if let Ok(variant) = Variant::decode(&format!("minecraft:{name}"), &v) {
                    all.push(variant);
                }
            }
        }
        let placeable =
            sunrise_core::prepared_files::read(&data.join("tags/painting_variant/placeable.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| v["values"].as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|v| v.as_str().map(Arc::from))
                .collect();
        Self { all, placeable }
    }
    pub fn get(&self, id: &str) -> Option<&Variant> {
        self.all.iter().find(|v| &*v.id == id)
    }
    /// The variant a stack's `painting/variant` component names.
    pub fn of_stack(&self, stack: &ItemStack) -> Option<&Variant> {
        let id = sunrise_sim::save::text(stack.data()?.other("minecraft:painting/variant"))?;
        self.get(&sunrise_world::datafix::dynamic::namespaced(&id))
    }
    /// `VariantUtils.readVariant` then `Painting.defineSynchedData`'s
    /// `VariantUtils.getAny`: a saved id that does not resolve, or none at all,
    /// leaves the registry's first variant in place. `Identifier.CODEC` reads a
    /// bare id in the `minecraft` namespace.
    pub fn saved_or_default(&self, id: Option<&str>) -> Option<Variant> {
        id.map(sunrise_world::datafix::dynamic::namespaced)
            .and_then(|id| self.get(&id))
            .or_else(|| self.all.first())
            .cloned()
    }
}
fn overlaps(a: ([f64; 3], [f64; 3]), b: ([f64; 3], [f64; 3])) -> bool {
    (0..3).all(|i| a.0[i] < b.1[i] && b.0[i] < a.1[i])
}
/// `HangingEntity.canCoexist(true)` for an item frame.
pub fn frame_coexists(f: &ItemFrame, frames: &[ItemFrame], paintings: &[Painting]) -> bool {
    !frames
        .iter()
        .any(|o| o.id != f.id && o.facing == f.facing && overlaps(o.bounds(), f.bounds()))
        && !paintings
            .iter()
            .any(|o| o.step() == f.step() && overlaps(o.bounds(), f.bounds()))
}
/// `HangingEntity.canCoexist(false)` for a painting.
pub fn painting_coexists(p: &Painting, paintings: &[Painting], frames: &[ItemFrame]) -> bool {
    !paintings
        .iter()
        .any(|o| o.id != p.id && overlaps(o.bounds(), p.bounds()))
        && !frames
            .iter()
            .any(|o| o.step() == p.step() && overlaps(o.bounds(), p.bounds()))
}

/// `HangingEntityItem.mayPlace` for a painting: not a floor or ceiling.
/// Direction.get2DDataValue: south 0, west 1, north 2, east 3.
pub fn painting_facing(face: [i32; 3]) -> Option<u8> {
    if face[1] != 0 {
        return None;
    }
    Some(match face {
        [0, 0, 1] => 0,
        [-1, 0, 0] => 1,
        [0, 0, -1] => 2,
        _ => 3,
    })
}
