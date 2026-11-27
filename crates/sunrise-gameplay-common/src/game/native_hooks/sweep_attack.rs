/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// NeoForge's `SweepAttackEvent`: a player's damaging melee hit on a
    /// `target` (`minecraft:zombie`, `minecraft:oak_boat`) is about to sweep or
    /// not, after `native_critical_hit`. `vanilla_sweep` is `Player.isSweepAttack`
    /// (a charged, grounded, not critical sword hit below the speed limit);
    /// `sweeping` starts as it and may force or stop the sweep; `canceled` skips
    /// the later mods and keeps `sweeping` as it is (a mod doing its own sweep
    /// cancels with `sweeping` false). A critical hit that sweeps has both
    /// sounds. Fires on the Game that runs the attack, and on the attacking
    /// client, which only asks.
    fn native_sweep_attack(
        &mut self,
        target: &str,
        vanilla_sweep: bool,
        sweeping: &mut bool,
        canceled: &mut bool,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_sweep_attack(
            self,
            target,
            vanilla_sweep,
            sweeping,
            canceled,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
