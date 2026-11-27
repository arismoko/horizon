//! The `hurt_enemy` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player's hit landed on the mob `entity` of type `kind`
    /// (`minecraft:zombie`), holding `stack`: where `Item.hurtEnemy` is called,
    /// before the post-attack enchantments and the weapon's wear. It only
    /// hears. Another player as the target is not told. Server-side, for
    /// every player of the world (the helpers read the acting one).
    /// Read the borrowed stack with `get`; call `project` only to keep an owned copy.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_hurt_enemy",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        version = 2,
        neoforge = "none (vanilla Item.hurtEnemy; NeoForge adds no event or extension for it); LivingDamageEvent.Post comes later"
    )]
    fn hurt_enemy(stack: mod_api::ItemStackView<'_>, entity: u32, kind: &str);
}
