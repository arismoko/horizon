use super::*;
pub(crate) type Capabilities<P> = HashMap<&'static str, HashMap<Box<str>, Vec<CapabilityPart<P>>>>;

/// A capability provider a mod registered: a [`BlockProviderOf`], [`ItemProviderOf`] or
/// [`EntityProviderOf`].
#[derive(Clone)]
pub(crate) struct CapabilityPart<P> {
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    /// The handler type it gives, for refusing a capability name answered with two.
    pub(crate) type_name: Arc<str>,
    /// Asked ahead of the game's own provider.
    pub(crate) before_game: bool,
    pub(crate) provider: P,
}

/// A capability provider whose handler type is a mod's own (a key a library's `[api]` crate
/// publishes), erased: only mods linked with that crate know its handler type.
// hidden: the registrar's erased provider, not part of the mod surface
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: transfers the handler name and a retained provider resource id, reporting unsupported handlers rather than serializing the native Any object.
#[doc(hidden)]
#[derive(Clone)]
pub struct OwnProvider {
    /// The `Arc` of the provider trait object, which a lookup of that handler type downcasts:
    /// a wasm mod's, in the game or another group, is the game's proxy, which no lookup takes.
    pub(crate) provider: Arc<dyn Any + Send + Sync>,
    pub(crate) handler: Arc<str>,
}

/// The registrar's table, as the game reads it.
pub(crate) struct InstalledProviders;

pub(crate) fn next<P: Clone>(
    table: &Capabilities<P>,
    name: &str,
    id: &str,
    index: &mut u32,
) -> Option<(P, &'static str, bool)> {
    next_provider(table.get(name)?.get(id)?, index)
}

/// A provider by its handler type: one case for each handler type `mod_api`'s keys name
/// (generated from them), and [`OwnProvider`] for a mod's own.
#[allow(missing_docs)]
pub(crate) mod providers_of {
    use super::*;
    // The keys' handler types and contexts, as the generated cases name them.
    #[allow(unused_imports)]
    use crate::*;
    include!(concat!(
        env!("OUT_DIR"),
        "/capability_providers_generated.rs"
    ));
}

/// Who answers a capability: a mod's provider, the game's own, or nobody.
pub(crate) enum Answer<T: ?Sized> {
    Mod(Capability<T>),
    Game,
    Nobody,
}

/// The handler `answer` names: a mod's, or `game`, the game's own when it answered.
pub(crate) fn answered<T: ?Sized>(
    answer: Answer<T>,
    game: Option<Box<T>>,
) -> Option<Capability<T>> {
    match answer {
        Answer::Mod(found) => Some(found),
        Answer::Game => game.map(|handler| Capability {
            handler,
            owner: None,
        }),
        Answer::Nobody => None,
    }
}

/// Asks the providers `next` gives (advancing a registration cursor in dispatch order: those
/// registered `before_game`, then the others), and the game's own (`game`: whether it answers) between the
/// two, in the fork's chosen order (NeoForge's javadoc doesn't state one between mods'
/// providers and the game's); the first to answer wins. `get` takes a provider of this
/// capability's handler type out of its case: a provider of a mod's own type the asker isn't
/// linked with is passed over.
pub(crate) fn ask_providers<O, P: ?Sized + 'static, T: ?Sized>(
    host: &mut dyn ServerHost,
    next: impl Fn(&mut u32) -> Option<(O, &'static str, bool)>,
    get: impl Fn(&O) -> Option<&Arc<P>>,
    call: impl Fn(&P, &mut dyn ServerHost) -> Option<Box<T>>,
    game: impl FnOnce(&mut dyn ServerHost) -> bool,
) -> Answer<T> {
    let mut game = Some(game);
    let mut index = 0;
    while let Some((provider, owner, before_game)) = next(&mut index) {
        if !before_game
            && let Some(game) = game.take()
            && game(host)
        {
            return Answer::Game;
        }
        if let Some(handler) = get(&provider).and_then(|provider| call(provider, host)) {
            return Answer::Mod(Capability {
                handler,
                owner: Some(owner),
            });
        }
    }
    match game.is_some_and(|game| game(host)) {
        true => Answer::Game,
        false => Answer::Nobody,
    }
}

/// CapabilityProviders operations for capabilities.
// hidden: per-kind lookup implementation, exposed for generated provider bindings
#[doc(hidden)]
pub trait CapabilitiesCapabilityProviders {
    /// Whether any mod registered capability `name` for the things `by` names. This presence
    /// check includes disabled mods; enumeration skips their providers.
    fn provided(&self, by: ProvidedBy, name: &str) -> bool;
}

impl CapabilitiesCapabilityProviders for InstalledProviders {
    fn provided(&self, by: ProvidedBy, name: &str) -> bool {
        installed().is_some_and(|installed| match by {
            ProvidedBy::BlockEntity => installed.keys.capabilities.contains_key(name),
            ProvidedBy::Item => installed.keys.item_capabilities.contains_key(name),
            ProvidedBy::Entity => installed.keys.entity_capabilities.contains_key(name),
        })
    }
}
