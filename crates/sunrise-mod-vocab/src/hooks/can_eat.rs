//! The `can_eat` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `Player.canEat`, from `Consumable.canConsume` when eating starts
    /// and block interaction eligibility for cake and candle cake.
    /// `player` is the eater: the acting server player (including a LAN guest)
    /// or the local player whose use the client predicts. Read their effects
    /// through `host.living(player.entity())`.
    /// For food items, `stack` is the food being consumed. For cake and candle
    /// cake, `stack` is `None` regardless of the held item. Vanilla computes
    /// eligibility with always-eat false for cake, or the stack's food component
    /// for food. `vanilla_can_eat` is that answer (creative, always-eat, or
    /// hungry). Answer `Some(false)` to refuse the food,
    /// `Some(true)` to let a full player eat; `can_eat` left `None` keeps
    /// `vanilla_can_eat`. Both sides (the client starts eating as the
    /// server does), so answer from the stack and the player alone.
    /// Not asked without a resident eater or an open lifecycle epoch; vanilla's
    /// answer stands during those world/login lifecycle gaps.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_can_eat",
        receiver = "read",
        dispatch = "flat",
        side = "both",
        version = 3,
        contract = "replace",
        out = "can_eat",
        host = "read",
        neoforge = "none (vanilla Player.canEat; NeoForge adds no event or extension for it)"
    )]
    fn can_eat(
        player: PlayerRef,
        stack: Option<&ItemStack>,
        vanilla_can_eat: bool,
        can_eat: &mut Option<bool>,
    );
}
