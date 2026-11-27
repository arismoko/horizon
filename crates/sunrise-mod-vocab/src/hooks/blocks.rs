//! The `blocks` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Declare blocks in code once at startup on each side, before resources load.
    /// Pass each `mod_api::Block` to `event.register`. The declaration belongs to the
    /// submitting package — in its own namespaces or in a content package it serves
    /// (`served_by`) — and works like `data/<id>/sunrise/block/<path>.json` in a
    /// content mod, including a block item unless disabled with `Block::with_item(false)`.
    /// Duplicate IDs across the host and served layers, and declarations into a
    /// `side = "server"` package's namespace, are refused.
    /// `register` returns a typed `DeclarationError` for invalid IDs or fields.
    /// The game records the refusal and stops startup even if a returning listener ignores it.
    /// A panic discards this listener's declarations and diagnostics without rejecting startup.
    #[notes = r#"## Villager farming

Farmers recognize mod blocks made from farmland and `CropBlock` templates, including declarations from
`blocks`. A crop block's generated item is its seed: add that item to
`minecraft:villager_plantable_seeds` to let farmers plant its default block state from their inventory.
Untagged items and tagged items without a block are skipped. Crop maturity follows the template's class:
beetroots mature at age 3, ordinary crops at 7; torchflower crops never reach their maximum age 2 while
still crops. Pitcher crops are not `CropBlock`s and are not harvested by this behavior. Farming still
requires a farmer at work and `mob_griefing` enabled."#]
    /// Bare block IDs use the submitting mod ID; valid explicit names retain their namespace.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::native_blocks",
        receiver = "associated",
        dispatch = "flat",
        side = "both",
        contract = "collect",
        out = "blocks",
        context = "registration",
        neoforge = "RegisterEvent",
        version = 5
    )]
    fn blocks(
        blocks: &mut dyn FnMut(
            mod_api::Answered<mod_api::Block>,
        ) -> Result<(), mod_api::DeclarationError>,
    );
}
