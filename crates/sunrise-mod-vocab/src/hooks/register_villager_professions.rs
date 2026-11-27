//! Load-phase declarative profession registration.
pub trait Hooks {
    /// Declare villager professions once per mod load, before world admission.
    /// The engine resolves every reference atomically with the owning resource tables.
    /// Vanilla replacement and overlapping custom acquisition POIs are refused.
    /// NeoForge adds a profession through `RegisterEvent` on `Registries.VILLAGER_PROFESSION`;
    /// this hook declares it as data, so only the registration moment corresponds.
    /// If this listener panics or a phase call is refused, none of its registrations take effect.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_villager_professions::native_register_villager_professions",
        receiver = "associated",
        dispatch = "flat",
        side = "server",
        version = 2,
        contract = "listen",
        staged_surface = "professions",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn register_villager_professions(professions: &mut dyn VillagerProfessions);
}
