//! The `standalone_models` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player finished editing a sign: `ServerGamePacketListenerImpl.updateSignText`
    /// has taken their update packet and every check has passed (they were
    /// editing that sign, within reach, the sign is not waxed),
    /// right before the lines go on the sign. `pos` in
    /// `dimension`, the `front` or back side, and the four `lines` as written,
    /// plain text with the formatting codes still in (vanilla strips them
    /// afterwards). Rewrite `lines` (made four lines again) and that is what
    /// the sign gets, or set
    /// `cancel` and the sign keeps its old text. With the player's state
    /// swapped in, so `PlayerRef::tell` and the helpers that act on the player
    /// reach them. Once for each update a player sends on a server. Not told:
    /// the host's own editing of a sign on a LAN or single-player world
    /// (the editor writes into their world directly), and the sign changes
    /// that are no edit of the text (dyeing, glow ink, waxing, `/data`,
    /// `/setblock`, or a mod using `BlockEntityRef::replace_data`).
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_sign_changed",
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no sign-editing event)",
        cancellable = true,
        version = 2
    )]
    fn sign_changed(
        player: u128,
        name: &str,
        dimension: &str,
        pos: [i32; 3],
        front: bool,
        lines: &mut Vec<String>,
        cancel: &mut bool,
    );
    /// Models no blockstate names, registered once on the client before resources load.
    /// Use [`StandaloneModelsEvent::register_model`] for plain files or
    /// [`StandaloneModelsEvent::register`] to bind a baker and its file dependencies.
    /// Every resource load rebakes both from the active pack files. Failed bakes publish
    /// no model and log the registering mod. Look up results through `RenderAssets::model`
    /// in or after `model_baking_completed`.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::native_standalone_models",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "registrations",
        answered_by = "mod",
        registration_append = "custom",
        version = 3,
        context = "registration",
        neoforge = "ModelEvent.RegisterStandalone"
    )]
    fn standalone_models(registrations: &mut Vec<Answered<StandaloneModelRegistration>>);
}
