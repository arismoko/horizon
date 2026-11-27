//! The `gui_layer_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before a vanilla or mod HUD layer (`RenderGuiLayerEvent.Pre`). Draw below the
    /// layer through `event.draw`, or return `Flow::Cancel` to skip its entire body,
    /// including side effects like the air-bubble sound, and `gui_layer_post`.
    /// Fires even when the layer's vanilla drawing is hidden.
    /// A wrapped or replaced layer fires this once, with the slot's original ID.
    /// To change a layer's body rather than veto the whole slot, wrap or replace it
    /// at startup in `register_gui_layers`.
    ///
    /// The vanilla anchors, in HUD order, are `minecraft:crosshair`, `hotbar`,
    /// `effects`, `boss_overlay`, `player_health` (hearts only), `armor_level`,
    /// `food_level`, `vehicle_health`, `air_level`,
    /// `contextual_info_bar` (jump, locator or experience bar, with the level number),
    /// `selected_item_name`, `sleep_overlay`, `overlay_message` (action bar), `title`,
    /// `scoreboard_sidebar`, `subtitle_overlay`, `chat` and `tab_list`. Conditions like
    /// F1 or not riding a mount only skip the original vanilla drawing, not these hooks.
    /// The HUD chat slot is visited once, even while the chat screen is open. Focused
    /// chat draws separately in that screen, without another layer event. Deferred subtitle
    /// paint also runs without another event; canceling its original slot prevents scheduling it.
    /// Camera overlays and the debug screen are not HUD layers. The selected item name and action bar keep vanilla's
    /// fixed positions; adding bars doesn't move them. Cancel and redraw the
    /// corresponding layer if it needs to move.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_gui_layer_pre",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 3,
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderGuiLayerEvent.Pre",
        cancellable = true
    )]
    fn gui_layer_pre(layer: &str, draw: &mut dyn DrawContext, canceled: &mut bool);
}
