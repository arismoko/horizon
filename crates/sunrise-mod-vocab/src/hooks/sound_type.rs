//! The `sound_type` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A block state's contextual sound type (`IBlockExtension.getSoundType`) is being read. `pos`
    /// and `state` describe the block; `entity` is the id and type of the entity stepping, landing,
    /// placing or digging, or `None` for a break or a dispenser placement. `cause` names the
    /// action, and `sound` starts with the state's base sound type. Edit its event keys or base
    /// volume and pitch without affecting the sound type's identity. The placing item
    /// may supply its own placement event, and a horse's hoof sound uses the returned
    /// type's identity. The game applies the action's normal factors and
    /// lets the sound engine resolve it. An event absent from the sound resources plays nothing.
    /// Supply keys parsed at load time, not raw names parsed again on every call.
    ///
    /// The read-only level host (`LevelSide`) reads the side that asked, at the shared selection
    /// sites and in client prediction alike: a part can pick a sound from a neighbouring block
    /// (`IBlockExtension.getSoundType` receives a `LevelReader`). It reads blocks, states, light,
    /// fluids, weather, enabled features, tags and resident block entities without loading
    /// chunks, and has no container, shape or signal view.
    ///
    /// Runs on the side that plays the sound. Keep answers a pure function of the inputs: the
    /// placer's client and the server must agree. Mining hits and sculk growth do not ask this
    /// hook. A mod that panics in this callback is reported and skipped by this hook for the
    /// rest of the session; its unfinished edit is not used.
    #[notes = r#"## Block sound type

- **`BlockPart::sound_type(context, event)`** edits the contextual `BlockSoundType` for the attached block, as in NeoForge's `IBlockExtension.getSoundType`. Use `SoundEventKey::parse` at load time and retain the keys on the part. A bare name or `:name` uses `minecraft`, and a syntactically valid event need not already be loaded. A mod's event can be provided by `assets/<namespace>/sounds.json`; an absent event plays nothing rather than reverting to vanilla. Playback reads the selected slot: step, fall, place, break, or the hit sound for a sniffer digging. A solid bucket chooses its own placement event but uses the returned sound type's levels. A horse chooses its hoof sound from the returned type's identity, not its event keys. There is no block-specific volume or pitch admission check: the usual action factors and the sound engine still apply. The hook runs on the ordinary and combination step, horse step (asked at the position below even when a snow layer answers), mob fall, block item placement, client break and sniffer dig paths; it is not the mining hit or sculk sound hook."#]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        contract = "listen",
        cancellable = false,
        scope = "block",
        key = "state",
        host = "level",
        version = 4,
        target = "sunrise_gameplay_common::game::native_hooks::sound_type::FrictionView<'_>::native_sound_type",
        neoforge = "IBlockExtension.getSoundType"
    )]
    fn sound_type(
        pos: BlockPos,
        state: BlockState,
        entity: Option<(u32, &str)>,
        cause: BlockSoundCause,
        sound: &mut BlockSoundType,
    );
}
