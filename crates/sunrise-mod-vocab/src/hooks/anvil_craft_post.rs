//! The `anvil_craft_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player took an anvil's result at `pos` and the levels, inputs and anvil wear are done (`AnvilCraftEvent.Post`); a broken anvil is already air. `inputs` borrows the same pre-take input stacks and result as `anvil_craft_pre`. Not called when `anvil_craft_pre` cancelled.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_anvil_craft_post",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "AnvilCraftEvent.Post",
        version = 2
    )]
    fn anvil_craft_post(
        player: u128,
        name: &str,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::AnvilCraftInputs<'_>,
    );
}
