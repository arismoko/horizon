//! Fork-shaped conversion of the upstream declaration.

pub trait Hooks {
    /// `item_interact_mob(entity, kind, stack, swing)` runs after ordinary entity interactions pass, for an alive native Mob selected by the exact packet ID or unobstructed crosshair. First `Some(true)` owns and swings once; `Some(false)` owns without a swing; `None` passes. The nonempty stack belongs to the acting hand. Armor stands, other real players and dragon-part tables are not covered. Bundles need "hooks": @API:item_interact_mob@.
    #[hook(
        side = "server",
        contract = "replace",
        out = "swing",
        scope = "item",
        key = "stack",
        neoforge = "none (Item.interactLivingEntity)"
    )]
    fn item_interact_mob();
}
