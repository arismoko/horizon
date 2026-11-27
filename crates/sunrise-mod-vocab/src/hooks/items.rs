//! The `items` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Declare items in code once at startup on each side, before resources load.
    /// Pass each `mod_api::Item` to `event.register`. The declaration belongs to the
    /// submitting package — in its own namespaces or in a content package it serves
    /// (`served_by`) — and works like `data/<id>/sunrise/item/<path>.json` in a
    /// content mod. A block's item is declared with its block, not here.
    /// `crafting_remainder` may reference items across the host and its served
    /// packages; duplicates across layers and declarations into a `side = "server"`
    /// package's namespace are refused.
    /// `register` returns a typed `DeclarationError` for invalid IDs or fields.
    /// The game records the refusal and stops startup even if a returning listener ignores it.
    /// A panic discards this listener's declarations and diagnostics without rejecting startup.
    /// Bare item IDs use the submitting mod ID; valid explicit names retain their namespace.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::native_items",
        receiver = "associated",
        dispatch = "flat",
        side = "both",
        contract = "collect",
        out = "items",
        context = "registration",
        neoforge = "RegisterEvent",
        version = 5
    )]
    fn items(
        items: &mut dyn FnMut(
            mod_api::Answered<mod_api::Item>,
        ) -> Result<(), mod_api::DeclarationError>,
    );
}
