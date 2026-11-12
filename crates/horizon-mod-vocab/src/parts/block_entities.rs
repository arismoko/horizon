use super::*;
/// What answers a capability for a block entity of one type: given the block entity and the
/// side asked about, its handler, or none to leave it to the next provider. A closure
/// `|host, at| Some(Box::new(Tank(at)))` is one.
pub trait BlockCapabilityProvider<T: ?Sized>: Send + Sync {
    /// The handler for the block entity at `at`, or none.
    fn provide(&self, host: &mut dyn ServerHost, at: BlockSide) -> Option<Box<T>>;
}
impl<T: ?Sized, F> BlockCapabilityProvider<T> for F
where
    F: Fn(&mut dyn ServerHost, BlockSide) -> Option<Box<T>> + Send + Sync,
{
    fn provide(&self, host: &mut dyn ServerHost, at: BlockSide) -> Option<Box<T>> {
        self(host, at)
    }
}
// no wire: trait-object alias inherits its provider trait policy; callbacks are not serialized.
/// A block capability provider, as the registrar keeps it.
pub type BlockProvider<T> = dyn BlockCapabilityProvider<T>;

pub use super::capabilities::providers_of::{BlockProviderOf, EntityProviderOf, ItemProviderOf};

/// Snapshot the callbacks once at registry preparation for worldgen workers.
pub fn block_entity_transforms()
-> std::collections::HashMap<String, Arc<dyn crate::BlockEntityTransform>> {
    installed().map_or_else(Default::default, |table| {
        table.block_entity_transforms.clone()
    })
}

/// The registered saved-data transform, looked up only during structure placement.
pub fn block_entity_transform(id: &str) -> Option<&'static dyn crate::BlockEntityTransform> {
    installed()?
        .block_entity_transforms
        .get(id)
        .map(AsRef::as_ref)
}

/// Owned declarations of the block entity types mods registered.
/// The game converts these snapshots to its content model when registration freezes.
pub fn block_entity_types() -> &'static [BlockEntityTypeDecl] {
    installed().map_or(&[], |installed| &installed.block_entity_types)
}

pub(crate) fn resolved_block_entity_types()
-> &'static OnceLock<horizon_mod_runtime::registry_keys::Names> {
    static EMPTY: OnceLock<horizon_mod_runtime::registry_keys::Names> = OnceLock::new();
    installed().map_or(&EMPTY, |installed| &installed.resolved_block_entity_types)
}

impl HookParts {
    pub(crate) fn block_entity<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn BlockEntityPart>] {
        attached(&self.block_entities, id)
    }
    pub(crate) fn block_entity_kind<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn BlockEntityPart>] {
        attached(&self.block_entity_kinds, id)
    }
}

/// The providers mods registered for capability `key` and the type of the block entity at
/// `at`, for the game's own lookups (a hopper's): none where no mod registered one, so a
/// vanilla container's lookup costs one table check and allocates nothing, and the game needn't
/// work out its own answer. [`BlockEntityProviders::ask`] then asks them.
pub fn block_entity_providers<T: ?Sized + 'static>(
    key: &CapabilityKey<T>,
    host: &dyn Host,
    at: BlockSide,
) -> Option<BlockEntityProviders<T>> {
    let parts = installed()?.keys.capabilities.get(key.name)?.get(
        crate::content::registry_keys::block_entity_type_name(at.kind(host)?)?,
    )?;
    Some(BlockEntityProviders {
        parts,
        at,
        handler: PhantomData,
    })
}

// no wire: provider lookup borrows static capability dispatch tables and process-local type identity.
/// A block entity's providers of one capability ([`block_entity_providers`]).
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BlockEntityProviders<T: ?Sized + 'static> {
    pub(crate) parts: &'static [CapabilityPart<BlockProviderOf>],
    pub(crate) at: BlockSide,
    pub(crate) handler: PhantomData<fn() -> Box<T>>,
}

impl<T: ?Sized + 'static> BlockEntityProviders<T> {
    /// A mod's handler where the game would answer with its own unless a mod comes first: one
    /// registered `before_game`, else, where the game has no answer (`game`), the first mod's.
    pub fn ask(self, host: &mut dyn ServerHost, game: bool) -> Option<Capability<T>> {
        let at = self.at;
        let answer = ask_providers(
            host,
            |index| next_provider(self.parts, index),
            BlockProviderOf::get::<T>,
            |provider, host| provider.provide(host, at),
            |_| game,
        );
        match answer {
            Answer::Mod(found) => Some(found),
            Answer::Game | Answer::Nobody => None,
        }
    }
}

impl dyn Registrar + '_ {
    /// Declare a block entity type in a namespace owned by this mod's manifest.
    pub fn block_entity_type(&mut self, id: &str) -> crate::BlockEntityTypeBuilder<'_> {
        crate::BlockEntityTypeBuilder {
            id: id.to_owned(),
            registrar: self,
            transform: None,
            blocks: Vec::new(),
        }
    }
    /// Attach a Part to a native type, including one another mod declares later.
    /// Vanilla keys are usable for capabilities, but not for native payload Parts.
    pub fn block_entity_part<P: BlockEntityPart>(
        &mut self,
        kind: &crate::BlockEntityTypeKey,
        part: P,
    ) -> Result<(), crate::BlockEntityPartError> {
        self.attach_block_entity_part(kind, type_name::<P>(), Arc::new(part))
    }
    /// Answer capability `key` for block entity type `kind` with `provider`, which gives
    /// a handler for one block entity and side, or none:
    /// `r.block_entity_capability(&ITEMS, &press, |_, _| Some(Box::new(PressItems)))`.
    /// Like NeoForge's `RegisterCapabilitiesEvent`, providers add up: in the fork's chosen order
    /// (NeoForge's javadoc states none), the game's own is asked first, then the mods' in
    /// registration order, and the first to answer wins.
    pub fn block_entity_capability<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        kind: &crate::BlockEntityTypeKey,
        provider: impl Fn(&mut dyn ServerHost, BlockSide) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        self.provide_block_entity_capability(
            key.name,
            kind,
            false,
            BlockProviderOf::of(Arc::new(provider)),
        );
    }
    /// `block_entity_capability`, asked ahead of the game's own
    /// provider (NeoForge's priority), as a mod that replaces how a block entity's slots are
    /// reached.
    pub fn block_entity_capability_before_game<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        kind: &crate::BlockEntityTypeKey,
        provider: impl Fn(&mut dyn ServerHost, BlockSide) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        self.provide_block_entity_capability(
            key.name,
            kind,
            true,
            BlockProviderOf::of(Arc::new(provider)),
        );
    }
}

/// Registrar operations for block entities.
pub trait BlockEntitiesRegistrar {
    /// Set the transform of a type declared by this mod.
    fn set_block_entity_transform(
        &mut self,
        key: crate::BlockEntityTypeKey,
        step: Arc<dyn crate::BlockEntityTransform>,
    );
    /// Look up a vanilla or another mod's type. The final registry validates it at load.
    fn block_entity_type_key(&mut self, id: &str) -> crate::BlockEntityTypeKey;

    /// Declare block entity type `path` in a namespace owned by this mod's manifest, held by `blocks`; mods usually
    /// reach it through `block_entity_type`.
    fn declare_block_entity_type(
        &mut self,
        path: &str,
        blocks: Vec<String>,
    ) -> crate::BlockEntityTypeKey;

    /// Attach `part`, of kind `kind`, to the mod block entity type `key`.
    fn attach_block_entity_part(
        &mut self,
        key: &crate::BlockEntityTypeKey,
        kind: &'static str,
        part: Arc<dyn BlockEntityPart>,
    ) -> Result<(), crate::BlockEntityPartError>;

    /// Answer block entity capability `name` for type `kind` with `provider`, asked ahead of the
    /// game's own when `before_game`; mods reach it through `block_entity_capability`.
    fn provide_block_entity_capability(
        &mut self,
        name: &'static str,
        kind: &crate::BlockEntityTypeKey,
        before_game: bool,
        provider: BlockProviderOf,
    );
}

/// CapabilityProviders operations for block entities.
// hidden: per-kind lookup implementation, exposed for generated provider bindings
#[doc(hidden)]
pub trait BlockEntitiesCapabilityProviders {
    /// The next enabled provider of block entity capability `name` for block entity type `kind`,
    /// with its mod and whether it is asked ahead of the game. `index` starts at zero and advances
    /// past every visited registration, including disabled ones.
    fn block_entity_provider(
        &self,
        name: &str,
        kind: crate::BlockEntityTypeKey,
        index: &mut u32,
    ) -> Option<(BlockProviderOf, &'static str, bool)>;
}

impl BlockEntitiesCapabilityProviders for InstalledProviders {
    fn block_entity_provider(
        &self,
        name: &str,
        kind: crate::BlockEntityTypeKey,
        index: &mut u32,
    ) -> Option<(BlockProviderOf, &'static str, bool)> {
        let kind = crate::content::registry_keys::block_entity_type_name(kind)?;
        next(&installed()?.keys.capabilities, name, kind, index)
    }
}
