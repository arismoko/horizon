//! The `anvil_craft_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player took an anvil's result at `pos` (`AnvilCraftEvent.Pre`, matching NeoForge `AnvilMenu.onTake`), before the levels are charged, the inputs used up and the anvil worn. `inputs` borrows the input stacks and result from before the take. Set `cancel` to return early before charging levels, consuming inputs, damaging the anvil or firing `anvil_craft_post`, without undoing the click's item move (the listener handles any post-processing itself).
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_anvil_craft_pre",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "AnvilCraftEvent.Pre",
        cancellable = true,
        version = 3
    )]
    fn anvil_craft_pre(
        player: u128,
        name: &str,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::AnvilCraftInputs<'_>,
        cancel: &mut bool,
    );
}
