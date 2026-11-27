//! The `sound_events` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `starting` is true for starts and loop replays, false for a live channel refresh.
    /// A ticking sound's live refresh applies volume, pitch and `effects` transiently; a refresh
    /// for a category volume change applies volume and pitch. `None` stops the channel.
    /// Base volume and pitch edits, including zero, negative and infinite values, go through
    /// `SoundEngine.calculateVolume` and `calculatePitch`, as they do in `play_local_sound`,
    /// at a start and at a live refresh alike. Invalid `effects` (`SoundEffects::valid`) refuse playback.
    /// A sound is about to play (`PlaySoundEvent`, vanilla's `SoundEngine.play`): `original` is the sound as asked for, `sound` starts as a copy of it. Edit `sound` (its volume, pitch, source, position, identifier, looping, delay, attenuation, relativeness or `effects`) to change what plays, or set it to `None` to play nothing. `effects` (`SoundEffects`) is the mixer's DSP for this one sound: its gain, a low-pass and a high-pass on the direct sound and sends to the shared `room`, `hall` and `cavern` reverbs, each in a fixed range (`SoundEffects::valid`: a value out of range, NaN or an infinity refuses the play, never clamped). A sound that doesn't tick keeps the effects it started with. It is asked before the engine's own checks (output open, sound resolved, volume, channel pool), for every sound the client plays through the sound engine, so answer fast; later mods see earlier mods' edits in `sound` and the sound as asked for in `original`. A new `identifier` is resolved again, with its own weighted pick. The listener gets the `ClientHost`. A start fires inside the play call itself and plays at once with the answer, as vanilla's `SoundEngine.play` does; a live refresh fires once per audio tick and applies at that tick. Client-side only; a server has no sound.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "PlaySoundEvent",
        cancellable = false,
        target = "sunrise_client_gameplay::game::native_hooks::play_sound::ClientGame<'_>::native_play_sound",
        version = 3
    )]
    /// `instance` is an engine-local sound instance id, unchanged across its live updates.
    fn play_sound(
        instance: u64,
        starting: bool,
        original: &SoundPlay,
        sound: &mut Option<SoundPlay>,
    );
    /// A sound that is not streamed has started on `channel` (`PlaySoundSourceEvent`), after the engine set its pitch, volume, attenuation, looping and position: `sound` is what is playing, after `play_sound`'s edits. A notice: the channel is an id only and gives no control. Streamed sounds (music, records) fire `play_streaming_source` instead. Client-side only.
    #[hook(
        side = "client",
        contract = "listen",
        context = "loop",
        neoforge = "PlaySoundSourceEvent",
        cancellable = false,
        target = "sunrise_client_render::sound_engine::hooks::native_play_sound_source"
    )]
    fn play_sound_source(sound: &SoundPlay, channel: &ChannelRef);
    /// A streamed sound (music, a record) has started on `channel` (`PlayStreamingSourceEvent`): `sound` is what is playing, after `play_sound`'s edits. A notice: the channel is an id only and gives no control. Other sounds fire `play_sound_source` instead. Client-side only.
    #[hook(
        side = "client",
        contract = "listen",
        context = "loop",
        neoforge = "PlayStreamingSourceEvent",
        cancellable = false,
        target = "sunrise_client_render::sound_engine::hooks::native_play_streaming_source"
    )]
    fn play_streaming_source(sound: &SoundPlay, channel: &ChannelRef);
    /// The sound engine opened its output (`SoundEngineLoadEvent`): once at window start, after each resource reload (with the `context` of the new resource generation) and when the output device changes. Nothing fires with no sound output (no device, or audio turned off). Client-side only.
    #[hook(
        side = "client",
        contract = "listen",
        context = "loop",
        neoforge = "SoundEngineLoadEvent",
        cancellable = false,
        target = "sunrise_client_render::sound_engine::hooks::native_sound_engine_load"
    )]
    fn sound_engine_load();
}
