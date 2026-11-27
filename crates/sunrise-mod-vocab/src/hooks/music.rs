//! The `music` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Choose this client's music, once a client tick: `music` and `volume`
    /// hold what vanilla chose (`Minecraft.getSituationalMusic` and
    /// `getMusicVolume`: the biome's track, the creative or underwater one, a
    /// boss's, the menu's; None for silence). Set `music` to a track of your
    /// own, a sound event of your content mod's `sounds.json` such as
    /// `Music::new("mymod:music.spring", 1200, 6000, false)` (the delays in
    /// ticks between songs, and whether it cuts off a different song playing),
    /// None to keep vanilla's quiet, or change `volume` (0 to 1) to fade it.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_music",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "SelectMusicEvent"
    )]
    fn music(music: &mut Option<Music>, volume: &mut f32);
}
