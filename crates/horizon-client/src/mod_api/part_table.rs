//! Parts (arismoko/horizon#41): code a mod attaches to one block, block entity, item or mod
//! menu type at startup. The scoped hooks ask the parts attached to the thing they are about,
//! after every mod's `Hooks` method. Parts attached by tag are resolved by each game against its
//! own tags, at world open and on `/reload`, into a table kept by that game's thread; that table
//! also indexes the block parts by state, so the state-keyed hooks look theirs up without the
//! block's name.
use super::ticket_controller::{self, ControllerInfo};
use super::{
    BlockEntityPart, BlockPart, BlockSide, BlockStateId, Capability, CapabilityKey,
    EntityCapabilityKey, EntityRef, Host, ItemCapabilityKey, ItemPart, ItemPlace, MenuPart,
    ModEntry, PlaceProvider, ServerHost, TicketController,
};
use std::{
    any::{Any, TypeId, type_name},
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    marker::PhantomData,
    num::NonZeroU32,
    rc::Rc,
    sync::{
        Arc, LazyLock, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

/// A part, with the mod that attached it.
pub struct Attached<P: ?Sized> {
    /// The mod's bundle id, retained for diagnostics after registration.
    pub(super) mod_id: &'static str,
    /// The mod's place in dispatch order.
    pub(super) mod_index: usize,
    /// Its place in its mod's registrations: a mod's parts on one thing keep this order, key or
    /// tag (#41 §3).
    order: usize,
    /// The part's kind: one per thing.
    kind: TypeId,
    pub(super) type_name: &'static str,
    pub(super) part: Arc<P>,
}

impl<P: ?Sized> Clone for Attached<P> {
    fn clone(&self) -> Self {
        Self {
            mod_id: self.mod_id,
            mod_index: self.mod_index,
            order: self.order,
            kind: self.kind,
            type_name: self.type_name,
            part: self.part.clone(),
        }
    }
}

/// Every mod's parts, by the id of the thing they are attached to, in dispatch order.
#[derive(Clone, Default)]
pub struct Parts {
    /// Block parts, and block entity parts as the block parts they also are.
    blocks: HashMap<Box<str>, Vec<Attached<dyn BlockPart>>>,
    /// The same block parts by state for this game's registry: an index into
    /// `block_slices` (one-based, so `None` names no parts), four bytes a
    /// state.
    block_states: Vec<Option<NonZeroU32>>,
    /// The parts `block_states` indexes, one entry per block that has any.
    block_slices: Vec<Arc<[Attached<dyn BlockPart>]>>,
    /// Block entity parts by block, for the hooks asked by block (`block_entity`, `screen`).
    block_entities: HashMap<Box<str>, Vec<Attached<dyn BlockEntityPart>>>,
    /// Block entity parts by block entity type, for the hooks asked by the entity
    /// (`block_entity_tick`, `menu_button`): its type id need not be its block's id.
    block_entity_kinds: HashMap<Box<str>, Vec<Attached<dyn BlockEntityPart>>>,
    items: HashMap<Box<str>, Vec<Attached<dyn ItemPart>>>,
    menus: HashMap<Box<str>, Vec<Attached<dyn MenuPart>>>,
    /// Capability providers by capability name, then by block entity type, in registration
    /// order.
    capabilities: Capabilities,
    /// Item capability providers by capability name, then by item, in registration order.
    item_capabilities: Capabilities,
    /// Entity capability providers by capability name, then by entity type, in registration
    /// order.
    entity_capabilities: Capabilities,
    /// The mods' own item places (`ItemPlace::Mod`), by provider name.
    place_providers: HashMap<&'static str, (&'static str, Arc<dyn PlaceProvider>)>,
}

type Capabilities = HashMap<&'static str, HashMap<Box<str>, Vec<CapabilityPart>>>;

/// A capability provider a mod registered: an `Arc` of [`BlockProvider`], [`ItemProvider`] or
/// [`EntityProvider`] of the capability's handler trait.
#[derive(Clone)]
pub(crate) struct CapabilityPart {
    mod_id: &'static str,
    /// The handler type it gives, for refusing a capability name answered with two.
    type_name: &'static str,
    /// Asked ahead of the game's own provider.
    before_game: bool,
    provider: Arc<dyn Any + Send + Sync>,
}

/// What answers a capability for a block entity of one type: given the block entity and the
/// side asked about, its handler, or none to leave it to the next provider.
pub type BlockProvider<T> = dyn Fn(&mut dyn ServerHost, BlockSide) -> Option<Box<T>> + Send + Sync;
/// What answers an item capability for one item: given where the stack lives, its handler, or
/// none to leave it to the next provider.
pub type ItemProvider<T> = dyn Fn(&mut dyn ServerHost, &ItemPlace) -> Option<Box<T>> + Send + Sync;
/// What answers an entity capability for one entity type: given the entity, its handler, or none
/// to leave it to the next provider.
pub type EntityProvider<T> = dyn Fn(&mut dyn ServerHost, EntityRef) -> Option<Box<T>> + Send + Sync;

/// Which things a capability's providers are registered by.
#[derive(Clone, Copy)]
pub(crate) enum ProvidedBy {
    BlockEntity,
    Item,
    Entity,
}

/// The parts attached by tag, by tag name, in registration order.
#[derive(Default)]
struct TagParts {
    blocks: Vec<(Box<str>, Attached<dyn BlockPart>)>,
    items: Vec<(Box<str>, Attached<dyn ItemPart>)>,
}

/// What every mod's [`Hooks::register`](super::Hooks::register) attached.
#[derive(Default)]
struct Installed {
    keys: Parts,
    tags: TagParts,
    /// The ticket controllers the mods registered; a [`TicketController`]'s slot indexes it.
    controllers: Vec<ControllerInfo>,
}

/// The ticket controllers registered at startup.
pub(super) fn installed_controllers() -> &'static [ControllerInfo] {
    INSTALLED
        .get()
        .map_or(&[], |installed| installed.controllers.as_slice())
}

static INSTALLED: OnceLock<Installed> = OnceLock::new();
#[cfg(test)]
thread_local! {
    /// Tests install their own parts per thread, as they do their mods (`TEST_MODS`).
    static TEST_PARTS: Cell<Option<&'static Installed>> = const { Cell::new(None) };
}
static NO_PARTS: LazyLock<Parts> = LazyLock::new(Parts::default);
static OFF_THREAD_WARNED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// This thread's game's parts, its tags resolved; none before its world opens.
    static GAME_PARTS: RefCell<Option<Rc<Parts>>> = const { RefCell::new(None) };
    /// Whether a `Game` runs on this thread.
    static GAME_THREAD: Cell<bool> = const { Cell::new(false) };
}

fn installed() -> Option<&'static Installed> {
    #[cfg(test)]
    if let Some(installed) = TEST_PARTS.get() {
        return Some(installed);
    }
    INSTALLED.get()
}

/// The parts table a dispatch reads: the process-wide one, or this game thread's.
pub(crate) enum PartsRef {
    Shared(&'static Parts),
    Game(Rc<Parts>),
}

impl std::ops::Deref for PartsRef {
    type Target = Parts;
    fn deref(&self) -> &Parts {
        match self {
            Self::Shared(parts) => parts,
            Self::Game(parts) => parts,
        }
    }
}

/// The parts hook `hook` asks: this game thread's resolved table, then the shared key parts
/// when there are no tag parts, then the off-thread path.
pub(crate) fn parts(hook: &'static str) -> PartsRef {
    let Some(installed) = installed() else {
        return PartsRef::Shared(&NO_PARTS);
    };
    if let Some(parts) = GAME_PARTS.with_borrow(Option::clone) {
        return PartsRef::Game(parts);
    }
    if installed.tags.blocks.is_empty() && installed.tags.items.is_empty() {
        return PartsRef::Shared(&installed.keys);
    }
    if !GAME_THREAD.get() {
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("unnamed");
        debug_assert!(false, "hook {hook} dispatched off a game thread ({thread})");
        if !OFF_THREAD_WARNED.swap(true, Ordering::Relaxed) {
            log::warn!(
                "Native hook {hook} ran off a game thread ({thread}): parts attached by tag are not asked"
            );
        }
    }
    PartsRef::Shared(&installed.keys)
}

/// Mark this thread as running a `Game`, whose dispatches read its own resolved parts.
pub(crate) fn mark_game_thread() {
    GAME_THREAD.set(true);
}

/// The ids a game knows, which a part's key must name.
pub(crate) struct KnownIds<'a> {
    pub block: &'a dyn Fn(&str) -> bool,
    pub item: &'a dyn Fn(&str) -> bool,
    pub state_count: usize,
    pub state_block: &'a dyn Fn(usize) -> &'a str,
}

/// Resolve this thread's game's parts against its tags (`block_tags`, `item_tags`: flattened,
/// tag name without `#` to members). Each run starts again from the key parts, so `/reload` drops
/// what left a tag. A key naming a block or item the game doesn't know is an error, naming the
/// mod; a missing tag or a part a tag gives twice is a warning, and the earlier mod's part stays.
pub(crate) fn resolve_parts(
    block_tags: &HashMap<String, HashSet<String>>,
    item_tags: &HashMap<String, HashSet<String>>,
    known: KnownIds<'_>,
) -> Result<(), Vec<String>> {
    let Some(installed) = installed() else {
        return Ok(());
    };
    let mut unknown: Vec<String> = unknown_keys(&installed.keys.blocks, known.block, "block")
        .chain(unknown_keys(&installed.keys.items, known.item, "item"))
        .collect();
    if !unknown.is_empty() {
        unknown.sort();
        return Err(unknown);
    }
    let mut parts = installed.keys.clone();
    add_tagged(
        &mut parts.blocks,
        &installed.tags.blocks,
        block_tags,
        "block",
    );
    add_tagged(&mut parts.items, &installed.tags.items, item_tags, "item");
    let mut names: HashMap<&str, NonZeroU32> = HashMap::with_capacity(parts.blocks.len());
    let mut block_slices: Vec<Arc<[Attached<dyn BlockPart>]>> =
        Vec::with_capacity(parts.blocks.len());
    for (name, attached) in &parts.blocks {
        names.insert(
            name,
            NonZeroU32::new(block_slices.len() as u32 + 1).unwrap(),
        );
        block_slices.push(Arc::from(attached.clone()));
    }
    let mut block_states = Vec::with_capacity(known.state_count);
    for state in 0..known.state_count {
        block_states.push(names.get((known.state_block)(state)).copied());
    }
    parts.block_slices = block_slices;
    parts.block_states = block_states;
    GAME_PARTS.set(Some(Rc::new(parts)));
    Ok(())
}

/// A line for each part attached to an id `known` refuses.
fn unknown_keys<'a, P: ?Sized>(
    table: &'a HashMap<Box<str>, Vec<Attached<P>>>,
    known: &'a dyn Fn(&str) -> bool,
    scope: &'a str,
) -> impl Iterator<Item = String> + 'a {
    table
        .iter()
        .filter(|(id, _)| !known(id))
        .flat_map(move |(id, list)| {
            list.iter().map(move |part| {
                format!(
                    "{} attached {} to {scope} {id}, which this game does not have",
                    part.mod_id, part.type_name
                )
            })
        })
}

/// Attach each tag part to its tag's members, in dispatch order: by mod, then by registration
/// within a mod.
fn add_tagged<P: ?Sized>(
    table: &mut HashMap<Box<str>, Vec<Attached<P>>>,
    tagged: &[(Box<str>, Attached<P>)],
    tags: &HashMap<String, HashSet<String>>,
    scope: &str,
) {
    for (tag, part) in tagged {
        let Some(members) = tags.get(&**tag) else {
            log::warn!(
                "{} attached {} to {scope} tag #{tag}, which does not exist",
                part.mod_id,
                part.type_name
            );
            continue;
        };
        for member in members {
            let list = table.entry(member.as_str().into()).or_default();
            match list.iter_mut().find(|a| a.kind == part.kind) {
                Some(other) => {
                    log::warn!(
                        "{} is attached to {member} twice, by {} and {} (#{tag}); {}'s is kept",
                        part.type_name,
                        other.mod_id,
                        part.mod_id,
                        if other.mod_index < part.mod_index {
                            other.mod_id
                        } else {
                            part.mod_id
                        },
                    );
                    if part.mod_index < other.mod_index {
                        *other = part.clone();
                    }
                }
                None => list.push(part.clone()),
            }
            list.sort_by_key(|a| (a.mod_index, a.order));
        }
    }
}

impl Parts {
    pub(crate) fn block_state(&self, state: BlockStateId) -> &[Attached<dyn BlockPart>] {
        self.block_states
            .get(state.0 as usize)
            .and_then(|index| *index)
            .map_or(
                &[],
                |index| &self.block_slices[index.get() as usize - 1][..],
            )
    }

    // Each takes the thing's id as a closure: with nothing attached in that scope the id is never
    // worked out, so a game with no parts pays no registry read on the hot hooks.
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
    pub(crate) fn item<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn ItemPart>] {
        attached(&self.items, id)
    }
    pub(crate) fn menu<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn MenuPart>] {
        attached(&self.menus, id)
    }
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

/// The providers mods registered for capability `name`, of the things `by` names, for the id
/// `id` works out, in registration order. With none registered for `name`, `id` isn't asked.
pub(crate) fn providers<'h, S: AsRef<str>>(
    by: ProvidedBy,
    name: &str,
    host: &'h dyn Host,
    id: impl FnOnce(&'h dyn Host) -> Option<S>,
) -> &'static [CapabilityPart] {
    let table = installed().map(|installed| match by {
        ProvidedBy::BlockEntity => &installed.keys.capabilities,
        ProvidedBy::Item => &installed.keys.item_capabilities,
        ProvidedBy::Entity => &installed.keys.entity_capabilities,
    });
    let by_id = table
        .and_then(|table| table.get(name))
        .filter(|by_id| !by_id.is_empty());
    by_id
        .and_then(|by_id| by_id.get(id(host)?.as_ref()))
        .map_or(&[][..], Vec::as_slice)
}

/// Asks `parts`, a capability's providers, in the fork's chosen order (NeoForge's javadoc
/// doesn't state one between mods' providers and the game's): those registered `before_game`
/// in registration order, then the game's own (`game`: whether it answers), then the others in
/// registration order; the first to answer wins.
pub(crate) fn ask_providers<P: ?Sized + 'static, T: ?Sized>(
    parts: &[CapabilityPart],
    host: &mut dyn ServerHost,
    call: impl Fn(&P, &mut dyn ServerHost) -> Option<Box<T>>,
    game: impl FnOnce(&mut dyn ServerHost) -> bool,
) -> Answer<T> {
    let ask = |before_game: bool, host: &mut dyn ServerHost| {
        parts
            .iter()
            .filter(|part| part.before_game == before_game)
            .find_map(|part| {
                let provider = part.provider.downcast_ref::<Arc<P>>()?;
                Some(Capability {
                    handler: call(provider, host)?,
                    owner: Some(part.mod_id),
                })
            })
    };
    if let Some(found) = ask(true, host) {
        return Answer::Mod(found);
    }
    if game(host) {
        return Answer::Game;
    }
    ask(false, host).map_or(Answer::Nobody, Answer::Mod)
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
    let parts = providers(ProvidedBy::BlockEntity, key.name, host, |host| {
        at.kind(host)
    });
    (!parts.is_empty()).then_some(BlockEntityProviders {
        parts,
        at,
        handler: PhantomData,
    })
}

/// A block entity's providers of one capability ([`block_entity_providers`]).
pub struct BlockEntityProviders<T: ?Sized + 'static> {
    parts: &'static [CapabilityPart],
    at: BlockSide,
    handler: PhantomData<fn() -> Box<T>>,
}

impl<T: ?Sized + 'static> BlockEntityProviders<T> {
    /// A mod's handler where the game would answer with its own unless a mod comes first: one
    /// registered `before_game`, else, where the game has no answer (`game`), the first mod's.
    pub fn ask(self, host: &mut dyn ServerHost, game: bool) -> Option<Capability<T>> {
        let at = self.at;
        let answer = ask_providers::<BlockProvider<T>, T>(
            self.parts,
            host,
            |provider, host| provider(host, at),
            |_| game,
        );
        match answer {
            Answer::Mod(found) => Some(found),
            Answer::Game | Answer::Nobody => None,
        }
    }
}

/// The mod's provider of the item places named `provider`, with the mod's index.
pub(crate) fn place_provider(provider: &str) -> Option<(&'static str, Arc<dyn PlaceProvider>)> {
    let (owner, provider) = installed()?.keys.place_providers.get(provider)?;
    Some((*owner, provider.clone()))
}

fn attached<'a, 'k, P: ?Sized>(
    table: &'a HashMap<Box<str>, Vec<Attached<P>>>,
    id: impl FnOnce() -> Option<&'k str>,
) -> &'a [Attached<P>] {
    if table.is_empty() {
        return &[];
    }
    id().and_then(|id| table.get(id)).map_or(&[], Vec::as_slice)
}

/// What a mod's [`Hooks::register`](super::Hooks::register) attaches its parts with. A thing takes one part of each
/// kind: the same kind twice on one thing refuses the mods at startup, naming both.
pub struct Registrar {
    mod_id: &'static str,
    mod_index: usize,
    /// Parts registered so far, by every mod: the next part's `order`.
    registered: usize,
    parts: Parts,
    tags: TagParts,
    controllers: Vec<ControllerInfo>,
    refused: Vec<String>,
}

impl Registrar {
    fn new(mod_id: &'static str) -> Self {
        Self {
            mod_id,
            mod_index: 0,
            registered: 0,
            parts: Parts::default(),
            tags: TagParts::default(),
            controllers: Vec::new(),
            refused: Vec::new(),
        }
    }

    /// Register ticket controller `name`, the way NeoForge's `RegisterTicketControllersEvent`
    /// does once at startup, and keep the handle for `chunk.set_mod_forced`. `name` is
    /// `quarry` or `mymod:quarry`; the controller's id is in the calling mod's namespace
    /// (`mymod:quarry`), and another mod's namespace refuses the mods at startup, as does an id
    /// registered twice. `validates` is whether it validates its saved tickets: the
    /// `validate_tickets` hook then runs for it when a level loads them (NeoForge's
    /// `LoadingValidationCallback`). A world's tickets of a controller no loaded mod registers are
    /// dropped when it loads.
    pub fn register_ticket_controller(&mut self, name: &str, validates: bool) -> TicketController {
        ticket_controller::register(
            &mut self.controllers,
            &mut self.refused,
            self.mod_id,
            name,
            validates,
        )
    }

    /// Attach `part` to block `block` (`mymod:shaft`, `minecraft:oak_log`).
    pub fn block_part<P: BlockPart>(&mut self, block: &str, part: P) {
        self.attach::<P, dyn BlockPart>(|p| &mut p.blocks, block, Arc::new(part));
    }
    /// Attach `part` to the mod block entity type `kind` (the id a `block_entity` hook gives the
    /// entity, `mymod:press`) and to `blocks`, the blocks that hold one. It answers the hooks
    /// asked by the entity (`block_entity_tick`, `menu_button`), those asked by block
    /// (`block_entity`, `screen`) and every block hook of those blocks.
    pub fn block_entity_part<P: BlockEntityPart>(&mut self, kind: &str, blocks: &[&str], part: P) {
        let part = Arc::new(part);
        if !self.attach::<P, dyn BlockEntityPart>(|p| &mut p.block_entity_kinds, kind, part.clone())
        {
            return;
        }
        for block in blocks {
            // Every block entity part is a block part too, so a second one is found there.
            if self.attach::<P, dyn BlockPart>(|p| &mut p.blocks, block, part.clone()) {
                self.attach::<P, dyn BlockEntityPart>(
                    |p| &mut p.block_entities,
                    block,
                    part.clone(),
                );
            }
        }
    }
    /// Attach `part` to every block in tag `tag` (`minecraft:logs`, without `#`), as each game's
    /// tags have it when its world opens and after `/reload`.
    pub fn block_part_tag<P: BlockPart>(&mut self, tag: &str, part: P) {
        let part = self.tagged::<P, dyn BlockPart>(tag, Arc::new(part));
        self.tags.blocks.extend(part);
    }
    /// Attach `part` to every item in tag `tag` (`minecraft:axes`, without `#`).
    pub fn item_part_tag<P: ItemPart>(&mut self, tag: &str, part: P) {
        let part = self.tagged::<P, dyn ItemPart>(tag, Arc::new(part));
        self.tags.items.extend(part);
    }
    /// Attach `part` to item `item` (`mymod:wrench`).
    pub fn item_part<P: ItemPart>(&mut self, item: &str, part: P) {
        self.attach::<P, dyn ItemPart>(|p| &mut p.items, item, Arc::new(part));
    }
    /// Answer capability `key` for the mod block entity type `kind` (`mymod:press`) with
    /// `provider`, which gives a handler for one block entity and side, or none:
    /// `r.block_entity_capability(&ITEMS, "mymod:press", |_, _| Some(Box::new(PressItems)))`.
    /// Like NeoForge's `RegisterCapabilitiesEvent`, providers add up: in the fork's chosen order
    /// (NeoForge's javadoc states none), the game's own is asked first, then the mods' in
    /// registration order, and the first to answer wins.
    pub fn block_entity_capability<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        kind: &str,
        provider: impl Fn(&mut dyn ServerHost, BlockSide) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        let provider: Arc<BlockProvider<T>> = Arc::new(provider);
        self.capability::<T, _>(|p| &mut p.capabilities, key.name, kind, false, provider);
    }
    /// [`block_entity_capability`](Self::block_entity_capability), asked ahead of the game's own
    /// provider (NeoForge's priority), as a mod that replaces how a block entity's slots are
    /// reached.
    pub fn block_entity_capability_before_game<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        kind: &str,
        provider: impl Fn(&mut dyn ServerHost, BlockSide) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        let provider: Arc<BlockProvider<T>> = Arc::new(provider);
        self.capability::<T, _>(|p| &mut p.capabilities, key.name, kind, true, provider);
    }
    /// Answer item capability `key` for item `item` (`mymod:backpack`) with `provider`, which
    /// gives a handler for the stack at one place, or none: `r.item_capability(&ITEM_ITEMS,
    /// "mymod:backpack", |_, _| Some(Box::new(Backpack)))`. Asked as
    /// [`block_entity_capability`](Self::block_entity_capability)'s are.
    pub fn item_capability<T: ?Sized + 'static>(
        &mut self,
        key: &ItemCapabilityKey<T>,
        item: &str,
        provider: impl Fn(&mut dyn ServerHost, &ItemPlace) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        let provider: Arc<ItemProvider<T>> = Arc::new(provider);
        self.capability::<T, _>(
            |p| &mut p.item_capabilities,
            key.name,
            item,
            false,
            provider,
        );
    }
    /// [`item_capability`](Self::item_capability), asked ahead of the game's own provider.
    pub fn item_capability_before_game<T: ?Sized + 'static>(
        &mut self,
        key: &ItemCapabilityKey<T>,
        item: &str,
        provider: impl Fn(&mut dyn ServerHost, &ItemPlace) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        let provider: Arc<ItemProvider<T>> = Arc::new(provider);
        self.capability::<T, _>(|p| &mut p.item_capabilities, key.name, item, true, provider);
    }
    /// Answer entity capability `key` for entity type `kind` (`mymod:robot`,
    /// `minecraft:player`) with `provider`, which gives a handler for one entity, or none:
    /// `r.entity_capability(&ENTITY_ITEMS, "mymod:robot", |_, robot| Some(Box::new(Cargo(robot))))`.
    /// Asked as [`block_entity_capability`](Self::block_entity_capability)'s are.
    pub fn entity_capability<T: ?Sized + 'static>(
        &mut self,
        key: &EntityCapabilityKey<T>,
        kind: &str,
        provider: impl Fn(&mut dyn ServerHost, EntityRef) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        let provider: Arc<EntityProvider<T>> = Arc::new(provider);
        self.capability::<T, _>(
            |p| &mut p.entity_capabilities,
            key.name,
            kind,
            false,
            provider,
        );
    }
    /// [`entity_capability`](Self::entity_capability), asked ahead of the game's own provider,
    /// as a mod that replaces how a player's inventory is reached.
    pub fn entity_capability_before_game<T: ?Sized + 'static>(
        &mut self,
        key: &EntityCapabilityKey<T>,
        kind: &str,
        provider: impl Fn(&mut dyn ServerHost, EntityRef) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        let provider: Arc<EntityProvider<T>> = Arc::new(provider);
        self.capability::<T, _>(
            |p| &mut p.entity_capabilities,
            key.name,
            kind,
            true,
            provider,
        );
    }
    /// Read and write the item places `ItemPlace::Mod { provider: name, .. }` with `provider`,
    /// as an accessory mod exposes its slots. One provider a name.
    pub fn place_provider(&mut self, name: &'static str, provider: Arc<dyn PlaceProvider>) {
        if !namespaced(name) {
            self.refused.push(format!(
                "{} provides item places {name:?}, which is not a namespaced name",
                self.mod_id
            ));
            return;
        }
        if let Some((other, _)) = self.parts.place_providers.get(name) {
            self.refused.push(format!(
                "item places {name} are provided twice, by {} and {}",
                *other, self.mod_id
            ));
            return;
        }
        self.parts
            .place_providers
            .insert(name, (self.mod_id, provider));
    }

    /// Adds `provider` for capability `name` and `id` to `table`. Refused for a capability name
    /// or an id with no namespace, and for a capability name another provider answers with
    /// another handler type (two mods each declaring their own key under one name, rather than
    /// sharing the one the defining mod's `[api]` crate publishes): asked by one type, the other's
    /// providers would never answer.
    fn capability<T: ?Sized + 'static, P: ?Sized + Send + Sync + 'static>(
        &mut self,
        table: fn(&mut Parts) -> &mut Capabilities,
        name: &'static str,
        id: &str,
        before_game: bool,
        provider: Arc<P>,
    ) {
        let me = self.mod_id;
        if !namespaced(name) {
            self.refused.push(format!(
                "{me} answered capability {name:?}, which is not a namespaced name"
            ));
            return;
        }
        if !namespaced(id) {
            self.refused.push(format!(
                "{me} answered {name} for {id:?}, which is not a namespaced id"
            ));
            return;
        }
        let providers = table(&mut self.parts).entry(name).or_default();
        if let Some(other) = providers.values().flatten().next()
            && !other.provider.is::<Arc<P>>()
        {
            self.refused.push(format!(
                "capability {name} is answered with two handler types, {} by {} and {} by {me}",
                other.type_name,
                other.mod_id,
                type_name::<T>(),
            ));
            return;
        }
        providers
            .entry(id.into())
            .or_default()
            .push(CapabilityPart {
                mod_id: self.mod_id,
                type_name: type_name::<T>(),
                before_game,
                provider: Arc::new(provider),
            });
    }
    /// Attach `part` to the mod menu type `menu`.
    pub fn menu_part<P: MenuPart>(&mut self, menu: &str, part: P) {
        self.attach::<P, dyn MenuPart>(|p| &mut p.menus, menu, Arc::new(part));
    }

    /// `part`, of kind `K`, for tag `tag`; none if it is refused.
    fn tagged<K: 'static, P: ?Sized>(
        &mut self,
        tag: &str,
        part: Arc<P>,
    ) -> Option<(Box<str>, Attached<P>)> {
        let name = type_name::<K>();
        if tag.starts_with('#') || !namespaced(tag) {
            self.refused.push(format!(
                "{} attached {name} to tag {tag:?}, which is not a namespaced tag name (write it without #)",
                self.mod_id
            ));
            return None;
        }
        self.registered += 1;
        Some((
            tag.into(),
            Attached {
                mod_id: self.mod_id,
                mod_index: self.mod_index,
                order: self.registered,
                kind: TypeId::of::<K>(),
                type_name: name,
                part,
            },
        ))
    }

    /// Attach `part`, of kind `K`, to `id` in `table`; false if it is refused.
    fn attach<K: 'static, P: ?Sized>(
        &mut self,
        table: fn(&mut Parts) -> &mut HashMap<Box<str>, Vec<Attached<P>>>,
        id: &str,
        part: Arc<P>,
    ) -> bool {
        let name = type_name::<K>();
        if !namespaced(id) {
            self.refused.push(format!(
                "{} attached {name} to {id:?}, which is not a namespaced id",
                self.mod_id
            ));
            return false;
        }
        let list = table(&mut self.parts).entry(id.into()).or_default();
        if let Some(other) = list.iter().find(|a| a.kind == TypeId::of::<K>()) {
            self.refused.push(format!(
                "{name} is attached to {id} twice, by {} and {}",
                other.mod_id, self.mod_id
            ));
            return false;
        }
        self.registered += 1;
        list.push(Attached {
            mod_id: self.mod_id,
            mod_index: self.mod_index,
            order: self.registered,
            kind: TypeId::of::<K>(),
            type_name: name,
            part,
        });
        true
    }
}

fn namespaced(id: &str) -> bool {
    id.split_once(':')
        .is_some_and(|(namespace, path)| !namespace.is_empty() && !path.is_empty())
}

/// Every mod's [`Hooks::register`](super::Hooks::register) run against one registrar, in dispatch order.
fn register_all(mods: &[ModEntry]) -> Registrar {
    let mut registrar = Registrar::new(mods[0].id);
    for (index, hooks) in super::entered_registration(mods, "register").enumerate() {
        registrar.mod_id = mods[index].id;
        registrar.mod_index = index;
        hooks.register(&mut registrar);
    }
    registrar
}

/// The ticket controllers `mods` register, for a test's mods.
#[cfg(any(test, feature = "testing"))]
pub(super) fn registered_controllers(mods: &[ModEntry]) -> Vec<ControllerInfo> {
    if mods.is_empty() {
        return Vec::new();
    }
    register_all(mods).controllers
}

/// Builds the parts from every mod's [`Hooks::register`](super::Hooks::register), in dispatch order. Once, at startup;
/// the error lists every part refused, naming its mods.
pub(crate) fn install_parts(mods: &[ModEntry]) -> Result<(), Vec<String>> {
    let installed = if mods.is_empty() {
        Installed::default()
    } else {
        let registrar = register_all(mods);
        if !registrar.refused.is_empty() {
            return Err(registrar.refused);
        }
        Installed {
            keys: registrar.parts,
            tags: registrar.tags,
            controllers: registrar.controllers,
        }
    };
    #[cfg(test)]
    TEST_PARTS.set(Some(Box::leak(Box::new(installed))));
    #[cfg(not(test))]
    if INSTALLED.set(installed).is_err() {
        log::warn!("Mod parts already installed; keeping the first table");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::*;
    use super::{Attached, KnownIds, mark_game_thread, resolve_parts};
    use crate::mod_api;
    use std::any::{TypeId, type_name};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    /// The level the test worlds are.
    fn here() -> LevelRef {
        LevelRef::new(NativeWorldEpoch::new(1), "minecraft:overworld")
    }

    /// The block at `pos` in [`here`], through `side`.
    fn on(pos: [i32; 3], side: Option<Direction>) -> BlockSide {
        BlockSide {
            level: here(),
            pos,
            side,
        }
    }

    /// A comparator reading of 5 more than it was.
    struct Gauge;
    impl BlockPart for Gauge {
        fn analog_output(&self, _: &dyn LevelHost, event: &mut AnalogOutputEvent) {
            event.output += 5;
        }
    }

    /// Connects redstone wire to its block from any side.
    struct Connector;
    impl BlockPart for Connector {
        fn should_redstone_wire_connect_to(
            &self,
            _: &dyn LevelHost,
            event: &mut ShouldRedstoneWireConnectToEvent,
        ) {
            event.connected = true;
        }
    }

    /// The block entity of type `test:press_entity`, held by block `test:press`: it ticks, and
    /// each tick adds a data slot.
    struct Press;
    impl BlockPart for Press {}
    impl BlockEntityPart for Press {
        fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
            event.ticks = true;
        }
        fn block_entity_tick(&self, _: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
            event.entity.data.push(1);
        }
    }
    /// The gold a `test:hoard` block entity holds, kept beside it.
    pub(crate) const HOARD: AttachmentType<u32> = AttachmentType {
        namespace: "test",
        name: "hoard",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    /// How many times a `test:hoard` was asked about its removal.
    pub(crate) static HOARD_ASKED: std::sync::atomic::AtomicU32 =
        std::sync::atomic::AtomicU32::new(0);
    /// The block entity of type `test:hoard`, held by block `test:press` as `test:press_entity`
    /// is: when it is removed it gives back the gold its `HOARD` counts.
    struct Hoard;
    impl BlockPart for Hoard {}
    impl BlockEntityPart for Hoard {
        /// With data `[1]`, it replaces its own block with granite.
        fn block_entity_tick(
            &self,
            host: &mut dyn ServerHost,
            event: &mut BlockEntityTickEvent<'_>,
        ) {
            if event.entity.data == [1] {
                let level = host.level("minecraft:overworld").unwrap();
                let state = host.block_state_default("minecraft:granite").unwrap();
                assert!(level.set_block(
                    host,
                    event.pos,
                    state,
                    NativeBlockUpdateFlags::KNOWN_SHAPE
                ));
            }
        }
        fn pre_remove_side_effects(
            &self,
            level: &mut dyn LevelRandomHost,
            event: &mut PreRemoveSideEffectsEvent<'_>,
        ) {
            HOARD_ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let BlockPos { x, y, z } = event.pos;
            if let Ok(Some(count @ 1..)) = HOARD.get_existing(level, Holder::BlockEntity([x, y, z]))
            {
                event
                    .drops
                    .push(ItemStack::new("minecraft:gold_ingot", count));
            }
        }
    }
    struct NoHost;
    impl Host for NoHost {}
    impl ServerHost for NoHost {}

    #[test]
    fn a_host_without_a_backend_refuses_handle_calls() {
        // NoHost implements nothing, so Host::backend keeps its None default:
        // every handle answers its typed refusal, never a no-op success.
        let level = LevelRef::new(NativeWorldEpoch::new(1), "minecraft:overworld");
        assert!(!level.in_place(&NoHost));
        assert_eq!(
            level.entities_in(&NoHost, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
            None
        );
    }

    /// A comparator reading of 100 more, for the blocks in tag `test:gauged`.
    struct TagGauge;
    impl BlockPart for TagGauge {
        fn analog_output(&self, _: &dyn LevelHost, event: &mut AnalogOutputEvent) {
            event.output += 100;
        }
    }

    /// Doubles every comparator reading, and attaches `Gauge` to `test:gauge`, `TagGauge` to
    /// tag `test:gauged` and `Press` to `test:press`.
    struct Doubler;
    #[mod_api::__subscriptions]
    impl Hooks for Doubler {
        fn register(&self, r: &mut Registrar) {
            r.block_part("test:gauge", Gauge);
            r.block_part("test:connector", Connector);
            r.block_part_tag("test:gauged", TagGauge);
            r.block_entity_part("test:press_entity", &["test:press"], Press);
            r.block_entity_part("test:hoard", &["test:press"], Hoard);
            // These handlers wrap the test hosts' own slots and tanks, as a mod replacing how a
            // block entity's are reached does: ahead of the game's.
            r.block_entity_capability_before_game(&ITEMS, "test:pipe", |_, at| {
                Some(Box::new(Pipe(at)))
            });
            r.block_entity_capability_before_game(&ITEMS, "test:greedy", |_, at| {
                Some(Box::new(Greedy(at)))
            });
            r.block_entity_capability_before_game(&ITEMS, "test:item_overstates", |_, _| {
                Some(Box::new(ItemOverstates))
            });
            r.entity_capability(&ENTITY_ITEMS, "test:robot", |_, _| Some(Box::new(Cargo)));
            r.item_capability_before_game(&ITEM_ITEMS, "test:pouch", |_, place| {
                Some(Box::new(Pouch(place.clone())))
            });
            r.item_capability(&ITEM_ENERGY, "test:cell", |_, place| {
                Some(Box::new(Cell(place.clone())))
            });
            r.item_capability_before_game(&ITEM_ITEMS, "test:bag", |_, place| {
                Some(Box::new(Bag(place.clone())))
            });
            r.item_capability_before_game(&ITEM_ITEMS, "test:case", |_, place| {
                Some(Box::new(Case(place.clone())))
            });
            r.place_provider("test:charms", Arc::new(Charms));
            r.place_provider("test:lying-place", Arc::new(deep_places::LyingPlace));
            r.item_capability_before_game(&ITEM_ITEMS, "test:outer", |_, place| {
                Some(Box::new(deep_places::Wrap(place.clone())))
            });
            // `test:ordered`: one provider that declines, two that answer, and one ahead of the
            // game that answers only at x = 9.
            r.block_entity_capability(&ITEMS, "test:ordered", |_, _| None);
            r.block_entity_capability(&ITEMS, "test:ordered", |_, _| Some(Box::new(Marked(1))));
            r.block_entity_capability(&ITEMS, "test:ordered", |_, _| Some(Box::new(Marked(2))));
            r.block_entity_capability_before_game(&ITEMS, "test:ordered", |_, at| {
                (at.pos[0] == 9).then(|| Box::new(Marked(3)) as _)
            });
            r.block_entity_capability_before_game(&ITEMS, "test:ordered", |_, at| {
                (at.pos[0] >= 9).then(|| Box::new(Marked(4)) as _)
            });
            // `test:ordered_item` the same, the one ahead of the game answering for two items.
            r.item_capability(&ITEM_ITEMS, "test:ordered_item", |_, _| None);
            r.item_capability(&ITEM_ITEMS, "test:ordered_item", |_, _| {
                Some(Box::new(Marked(1)))
            });
            r.item_capability(&ITEM_ITEMS, "test:ordered_item", |_, _| {
                Some(Box::new(Marked(2)))
            });
            r.item_capability_before_game(&ITEM_ITEMS, "test:ordered_item", |host, place| {
                let two = place.amount(host) == 2;
                two.then(|| Box::new(Marked(3)) as _)
            });
            for (id, lie) in [
                ("test:tank", None),
                ("test:gives_more", Some(Lie::GivesMore)),
                ("test:takes_more", Some(Lie::TakesMore)),
            ] {
                r.block_entity_capability_before_game(&FLUIDS, id, move |_, at| {
                    Some(Box::new(ModTank { at, lie }))
                });
            }
            for (id, lie) in [
                ("test:battery_gives_more", Lie::GivesMore),
                ("test:battery_takes_more", Lie::TakesMore),
            ] {
                r.block_entity_capability_before_game(&ENERGY, id, move |_, at| {
                    Some(Box::new(ModBattery { at, lie }))
                });
            }
            r.block_entity_capability(&CHEMICALS, "test:chem_tank", |_, at| {
                Some(Box::new(ChemTank(at)))
            });
        }
        fn analog_output(&self, _: &dyn LevelHost, event: &mut AnalogOutputEvent) {
            event.output *= 2;
        }
    }

    /// A test host's whole state, kept on each transaction it opens and put back on an abort, as
    /// the game's journal keeps its storages.
    trait Journal: Clone {
        fn kept(&mut self) -> &mut Vec<Self>;
    }
    fn open<J: Journal>(host: &mut J) {
        let mut now = host.clone();
        now.kept().clear();
        host.kept().push(now);
    }
    fn close<J: Journal>(host: &mut J, commit: bool) {
        let Some(was) = host.kept().pop() else {
            return;
        };
        if !commit {
            let kept = std::mem::take(host.kept());
            *host = was;
            *host.kept() = kept;
        }
    }

    /// A pipe's item capability, reached through `.0`: one slot from every side but its top,
    /// and what goes in is handed to the container east of it, through that container's west
    /// side.
    struct Pipe(BlockSide);
    impl ResourceHandler<ItemResource> for Pipe {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            usize::from(self.0.side != Some(Direction::Up))
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            None
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            0
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
            0
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            _: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            let [x, y, z] = self.0.pos;
            let east = [x + 1, y, z];
            let Some(next) = self
                .0
                .level
                .capability(host, &ITEMS, east, Some(Direction::West))
            else {
                return 0;
            };
            next.insert(host, tx, 0, resource, amount)
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    /// A pipe that hands what it's offered on as [`Pipe`] does, and answers one more than went.
    struct Greedy(BlockSide);
    impl ResourceHandler<ItemResource> for Greedy {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            None
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            0
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
            0
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            Pipe(self.0).insert(host, tx, index, resource, amount) + 1
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    /// A pipe at (0, 64, 0), and a one-slot vanilla chest east of it, answered by the game.
    #[derive(Clone, Default)]
    struct PipeWorld {
        chest: Option<ItemStack>,
        kept: Vec<Self>,
    }
    const PIPE: [i32; 3] = [0, 64, 0];
    const CHEST: [i32; 3] = [1, 64, 0];
    impl Journal for PipeWorld {
        fn kept(&mut self) -> &mut Vec<Self> {
            &mut self.kept
        }
    }
    impl Backend for PipeWorld {
        fn level_in_place(&self, level: LevelRef) -> bool {
            level == here()
        }
        fn block_entity_kind(
            &self,
            BlockEntityRef { position: pos, .. }: BlockEntityRef,
        ) -> Option<&str> {
            (pos == PIPE).then_some("test:pipe")
        }
        fn container_size(&self, BlockSide { pos, .. }: BlockSide) -> Option<usize> {
            (pos == CHEST).then_some(1)
        }
        fn container_stack(
            &self,
            BlockSide { pos, .. }: BlockSide,
            slot: usize,
        ) -> Option<ItemStack> {
            self.chest.clone().filter(|_| pos == CHEST && slot == 0)
        }
    }
    impl BackendMut for PipeWorld {
        fn container_insert(
            &mut self,
            BlockSide { pos, .. }: BlockSide,
            slot: usize,
            stack: ItemStack,
            _: &Transaction,
        ) -> Option<ItemStack> {
            if pos != CHEST || slot != 0 || self.chest.is_some() {
                return Some(stack);
            }
            self.chest = Some(stack);
            None
        }
    }
    impl Host for PipeWorld {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
        fn block_entity(&self, level: LevelRef, position: [i32; 3]) -> Option<BlockEntityRef> {
            Some(BlockEntityRef { level, position })
        }
    }
    impl ServerHost for PipeWorld {
        fn transaction_open(&mut self, _: TransactionSeal) {
            open(self);
        }
        fn transaction_close(&mut self, _: TransactionSeal, commit: bool) {
            close(self, commit);
        }
        fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
            Some(BackendMutRef::of(self))
        }
    }

    fn stick() -> ItemStack {
        stack("minecraft:stick", 1, 0)
    }

    fn stack(item: &str, count: u32, damage: u32) -> ItemStack {
        ItemStack {
            item: Arc::from(item),
            count,
            damage,
            extra: None,
        }
    }

    /// A pouch of sticks at `.0`: one slot, its stick count kept in the pouch's own stack (its
    /// damage, for the test), so the handler holds nothing.
    struct Pouch(ItemPlace);
    impl ResourceHandler<ItemResource> for Pouch {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
            (self.amount(host, index) > 0).then(|| ItemResource::of(&stick()))
        }
        fn amount(&self, host: &mut dyn ServerHost, _: usize) -> u32 {
            self.0.stack(host).map_or(0, |pouch| pouch.damage)
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, resource: &ItemResource) -> u32 {
            if resource.matches(&stick()) { 64 } else { 0 }
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            _: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            let Some(pouch) = self.0.stack(host).filter(|_| resource.matches(&stick())) else {
                return 0;
            };
            let filled = ItemResource::of(&stack("test:pouch", 1, pouch.damage + amount));
            match self.0.exchange(host, tx, &filled, 1) {
                1 => amount,
                _ => 0,
            }
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    /// A bag at `.0` holding one pouch: the pouch's sticks plus one kept in the bag's damage, 0
    /// when empty.
    struct Bag(ItemPlace);
    impl ResourceHandler<ItemResource> for Bag {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, host: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            let held = self.0.stack(host)?.damage;
            (held > 0).then(|| ItemResource::of(&stack("test:pouch", 1, held - 1)))
        }
        fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
            u32::from(self.resource(host, index).is_some())
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, resource: &ItemResource) -> u32 {
            u32::from(resource.item() == "test:pouch")
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            _: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            let pouch = resource.to_stack(1);
            match self.0.stack(host) {
                Some(bag) if bag.damage == 0 && amount > 0 && resource.item() == "test:pouch" => {
                    let full = ItemResource::of(&stack("test:bag", 1, pouch.damage + 1));
                    self.0.exchange(host, tx, &full, 1)
                }
                _ => 0,
            }
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            if amount == 0 || self.resource(host, index).as_ref() != Some(resource) {
                return 0;
            }
            let empty = ItemResource::of(&stack("test:bag", 1, 0));
            self.0.exchange(host, tx, &empty, 1)
        }
    }

    /// A case at `.0` with two slots kept in its damage: diamonds in the hundreds, sticks in
    /// the units. Before taking sticks it tries emptying its diamond slot in a transaction of
    /// its own and aborts that, as a handler checking what a write would do may.
    struct Case(ItemPlace);
    impl Case {
        fn held(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
            let damage = self.0.stack(host).map_or(0, |case| case.damage);
            [damage / 100, damage % 100][index]
        }
        fn rewrite(&self, host: &mut dyn ServerHost, tx: &Transaction, damage: u32) -> bool {
            let case = ItemResource::of(&stack("test:case", 1, damage));
            self.0.exchange(host, tx, &case, 1) == 1
        }
    }
    impl ResourceHandler<ItemResource> for Case {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            2
        }
        fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
            let item = ["minecraft:diamond", "minecraft:stick"][index];
            (self.held(host, index) > 0).then(|| ItemResource::of(&stack(item, 1, 0)))
        }
        fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
            self.held(host, index)
        }
        fn capacity(&self, _: &mut dyn ServerHost, index: usize, resource: &ItemResource) -> u32 {
            let item = ["minecraft:diamond", "minecraft:stick"][index];
            if resource.item() == item { 99 } else { 0 }
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            if index != 1 || resource.item() != "minecraft:stick" {
                return 0;
            }
            let damage = self.0.stack(host).map_or(0, |case| case.damage);
            match self.rewrite(host, tx, damage + amount) {
                true => amount,
                false => 0,
            }
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            if index != 1 || resource.item() != "minecraft:stick" {
                return 0;
            }
            let damage = self.0.stack(host).map_or(0, |case| case.damage);
            let tried: Result<(), ()> = host.transaction(|host, tx| {
                self.rewrite(host, tx, damage % 100);
                Err(())
            });
            assert!(tried.is_err());
            let damage = self.0.stack(host).unwrap().damage;
            assert_eq!(damage / 100, 1, "diamond restored after inner abort");
            let took = amount.min(damage % 100);
            match self.rewrite(host, tx, damage - took) {
                true => took,
                false => 0,
            }
        }
    }

    /// An accessory mod's charm slots: charm `key` is kept in player 1's inventory slot 20 +
    /// `key`. A charm slot has nowhere to put what doesn't stack with its charm, so it refuses
    /// that.
    struct Charms;
    impl PlaceProvider for Charms {
        fn stack(&self, host: &mut dyn ServerHost, key: u64) -> Option<ItemStack> {
            backend_mut_of(host).and_then(|backend| backend.game_stack(&charm_slot(key)))
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            key: u64,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            let place = charm_slot(key);
            match place.stack(host) {
                Some(held) if !resource.matches(&held) => 0,
                held => {
                    let room = 64 - held.map_or(0, |held| held.count);
                    place.insert(host, tx, resource, amount.min(room))
                }
            }
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            key: u64,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            charm_slot(key).extract(host, tx, resource, amount)
        }
    }
    fn charm_slot(key: u64) -> ItemPlace {
        ItemPlace::PlayerSlot {
            player: 1,
            slot: 20 + key as i32,
        }
    }

    /// Player 1's hands and slots, and what was given to them: an item split off their hand.
    #[derive(Clone, Default)]
    struct Player {
        main_hand: Option<ItemStack>,
        slots: HashMap<i32, ItemStack>,
        given: Vec<ItemStack>,
        kept: Vec<Self>,
    }
    const HAND: ItemPlace = ItemPlace::PlayerHand {
        player: 1,
        hand: Hand::Main,
    };
    impl Journal for Player {
        fn kept(&mut self) -> &mut Vec<Self> {
            &mut self.kept
        }
    }
    impl Backend for Player {
        fn level_in_place(&self, level: LevelRef) -> bool {
            level == here()
        }
        // Entity 1 is the player, 2 a pig, 3 a mod's robot, 4 a player in another level.
        fn entity_kind(&self, entity: EntityRef) -> Option<String> {
            let kind = [
                "minecraft:player",
                "minecraft:pig",
                "test:robot",
                "minecraft:player",
            ];
            kind.get(entity.id().checked_sub(1)? as usize)
                .map(|&kind| kind.into())
        }
        fn entity_level(&self, entity: EntityRef) -> Option<LevelRef> {
            Some(match entity.id() {
                4 => LevelRef::new(NativeWorldEpoch::new(1), "minecraft:the_nether"),
                _ => here(),
            })
        }
        fn entity_container_size(&self, _: u32) -> Option<usize> {
            None
        }
        // The player holds hotbar slot 5.
        fn player_selected_slot(&self, _: PlayerRef) -> Option<usize> {
            Some(5)
        }
        fn item_container_slots(&self, _: &ItemStack) -> Option<Vec<Option<ItemStack>>> {
            None
        }
    }
    impl BackendMut for Player {
        fn entity_uuid(&mut self, entity: EntityRef) -> Option<u128> {
            Some(u128::from(entity.id()))
        }
        fn game_stack(&mut self, place: &ItemPlace) -> Option<ItemStack> {
            match place {
                ItemPlace::PlayerHand { player: 1, .. } => self.main_hand.clone(),
                ItemPlace::PlayerSlot { player: 1, slot } => self.slots.get(slot).cloned(),
                _ => None,
            }
        }
        fn game_insert(&mut self, place: &ItemPlace, stack: ItemStack, _: &Transaction) -> u32 {
            if !self.is_place(place) {
                return 0;
            }
            let mut rest = stack.clone();
            match self.game_stack(place) {
                None => {
                    self.put(place, Some(stack.clone()));
                    rest.count = 0;
                }
                Some(mut held) if held.same_item_same_components(&rest) => {
                    let put = (64 - held.count).min(rest.count);
                    held.count += put;
                    rest.count -= put;
                    self.put(place, Some(held));
                }
                Some(_) => {}
            }
            self.given.extend((rest.count > 0).then_some(rest));
            stack.count
        }
        fn game_extract(&mut self, place: &ItemPlace, stack: ItemStack, _: &Transaction) -> u32 {
            let Some(mut held) = self.game_stack(place) else {
                return 0;
            };
            if !held.same_item_same_components(&stack) {
                return 0;
            }
            let took = held.count.min(stack.count);
            held.count -= took;
            self.put(place, (held.count > 0).then_some(held));
            took
        }
    }
    impl Host for Player {
        fn player(&self, uuid: u128) -> Option<PlayerRef> {
            let entity = EntityRef::new(NativeWorldEpoch::new(1), u32::try_from(uuid).ok()?);
            Some(PlayerRef { entity, uuid })
        }
        fn item_default_component(&self, item: &str, component: &str) -> Option<Tag> {
            let helmet = item == "minecraft:iron_helmet";
            match component {
                "minecraft:max_stack_size" => Some(Tag::Int(if helmet { 1 } else { 64 })),
                "minecraft:equippable" if helmet => Some(Tag::Compound(
                    [("slot".into(), Tag::String("head".into()))]
                        .into_iter()
                        .collect(),
                )),
                _ => None,
            }
        }
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
    }
    impl ServerHost for Player {
        fn transaction_open(&mut self, _: TransactionSeal) {
            open(self);
        }
        fn transaction_close(&mut self, _: TransactionSeal, commit: bool) {
            close(self, commit);
        }
        fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
            Some(BackendMutRef::of(self))
        }
    }
    impl Player {
        fn is_place(&self, place: &ItemPlace) -> bool {
            matches!(
                place,
                ItemPlace::PlayerHand { player: 1, .. } | ItemPlace::PlayerSlot { player: 1, .. }
            )
        }
        fn put(&mut self, place: &ItemPlace, stack: Option<ItemStack>) {
            match place {
                ItemPlace::PlayerHand { .. } => self.main_hand = stack,
                ItemPlace::PlayerSlot { slot, .. } => {
                    self.slots.remove(slot);
                    self.slots.extend(stack.map(|stack| (*slot, stack)));
                }
                _ => {}
            }
        }
    }

    /// A mod robot's three empty slots.
    struct Cargo;
    impl ResourceHandler<ItemResource> for Cargo {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            3
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            None
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            0
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
            0
        }
        fn insert(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    #[test]
    fn a_players_items_are_their_inventory_and_each_index_takes_only_what_fits_there() {
        game_thread();
        let entity = |id| EntityRef::new(NativeWorldEpoch::new(1), id);
        let helmet = || stack("minecraft:iron_helmet", 1, 0);
        let mut player = Player {
            slots: [
                (3, stack("minecraft:stick", 10, 0)),
                (5, stack("minecraft:stick", 60, 0)),
            ]
            .into_iter()
            .collect(),
            ..Player::default()
        };
        let host: &mut dyn ServerHost = &mut player;
        let items = entity(1).capability(host, &ENTITY_ITEMS).unwrap();
        assert_eq!(items.owner(), "the game");
        assert_eq!(items.size(host), 41);
        let (sticks, helmet_) = (ItemResource::of(&stick()), ItemResource::of(&helmet()));
        // The main inventory takes a stack of anything; an armour slot only what goes there,
        // a helmet on the head (index 39), not the feet (36).
        assert_eq!(items.capacity(host, 0, &sticks), 64);
        assert_eq!(items.capacity(host, 36, &sticks), 0);
        assert_eq!(items.capacity(host, 39, &helmet_), 1);
        assert_eq!(items.capacity(host, 36, &helmet_), 0);
        assert_eq!(items.capacity(host, 40, &helmet_), 1);
        assert_eq!(items.capacity(host, 39, &sticks), 0);
        assert_eq!(items.amount(host, 3), 10);
        // Into one index, no further: 54 of 60 fill slot 3, and nothing goes elsewhere.
        let insert = |host: &mut dyn ServerHost, index, resource: &ItemResource, amount| {
            let went = host.transaction(|host, tx| {
                Ok::<_, ()>(items.insert(host, tx, index, resource, amount))
            });
            went.unwrap()
        };
        assert_eq!(insert(host, 3, &sticks, 60), 54);
        assert_eq!(insert(host, 38, &sticks, 1), 0);
        assert_eq!(insert(host, 39, &helmet_, 1), 1);
        assert_eq!(player.slots[&3].count, 64);
        assert_eq!(player.slots[&103], helmet());
        assert!(player.given.is_empty());
        // Without an index, as `Inventory.add`: the selected slot's sticks first, then an
        // empty slot.
        let host: &mut dyn ServerHost = &mut player;
        let aborted = host.transaction(|host, tx| {
            assert_eq!(items.insert_any(host, tx, &sticks, 10), 10);
            Err::<(), ()>(())
        });
        assert!(aborted.is_err());
        assert_eq!(player.slots[&5], stack("minecraft:stick", 60, 0));
        let host: &mut dyn ServerHost = &mut player;
        let went =
            host.transaction(|host, tx| Ok::<_, ()>(items.insert_any(host, tx, &sticks, 10)));
        assert_eq!(went, Ok(10));
        assert_eq!(player.slots[&5], stack("minecraft:stick", 64, 0));
        assert_eq!(player.slots[&0], stack("minecraft:stick", 6, 0));
        assert!(player.given.is_empty());
    }

    #[test]
    fn an_entitys_items_come_from_its_types_provider_and_none_outside_the_level_in_place() {
        game_thread();
        let entity = |id| EntityRef::new(NativeWorldEpoch::new(1), id);
        let mut player = Player::default();
        let host: &mut dyn ServerHost = &mut player;
        // A pig has no items; a mod's robot has its provider's; a player in a parked level
        // is asked nothing.
        assert!(entity(2).capability(host, &ENTITY_ITEMS).is_none());
        let robot = entity(3).capability(host, &ENTITY_ITEMS).unwrap();
        assert_eq!((robot.owner(), robot.size(host)), ("doubler".to_owned(), 3));
        assert!(entity(4).capability(host, &ENTITY_ITEMS).is_none());
    }

    /// Items counted with what they hold: a pouch's sticks, a bag's pouch and its sticks.
    fn items(stacks: impl IntoIterator<Item = ItemStack>) -> u32 {
        stacks
            .into_iter()
            .map(|s| match &*s.item {
                "test:pouch" | "test:bag" => s.count * (1 + s.damage),
                _ => s.count,
            })
            .sum()
    }
    impl Player {
        fn items(&self) -> u32 {
            let held = self.main_hand.iter().chain(self.slots.values());
            items(held.chain(&self.given).cloned())
        }
    }

    /// Three shelf slots at (2, 64, 0), answered by the game's handler, else by
    /// `ItemOverstates`. A slot takes up to `limit` of one item, the `output` slot none, and
    /// gives at most one item per extract, unless `whole_extract`; with `one_per_insert`, it
    /// takes at most one per insert.
    #[derive(Clone)]
    struct Shelf {
        slots: [Option<ItemStack>; 3],
        limit: u32,
        output: Option<usize>,
        one_per_insert: bool,
        whole_extract: bool,
        overstates: bool,
        kept: Vec<Self>,
    }
    fn shelf(slots: [Option<ItemStack>; 3]) -> Shelf {
        Shelf {
            slots,
            limit: 64,
            output: None,
            one_per_insert: false,
            whole_extract: false,
            overstates: false,
            kept: Vec::new(),
        }
    }
    const SHELF: [i32; 3] = [2, 64, 0];
    fn shelf_slot(slot: usize) -> ItemPlace {
        ItemPlace::BlockEntity {
            level: here(),
            pos: SHELF,
            side: None,
            slot,
        }
    }
    impl Journal for Shelf {
        fn kept(&mut self) -> &mut Vec<Self> {
            &mut self.kept
        }
    }
    impl Backend for Shelf {
        fn level_in_place(&self, level: LevelRef) -> bool {
            level == here()
        }
        fn block_entity_kind(&self, _: BlockEntityRef) -> Option<&str> {
            self.overstates.then_some("test:item_overstates")
        }
        fn container_capacity(&self, _: BlockSide, slot: usize, _: &ItemStack) -> u32 {
            if self.output == Some(slot) {
                0
            } else {
                self.limit
            }
        }
        fn container_size(&self, BlockSide { pos, .. }: BlockSide) -> Option<usize> {
            (pos == SHELF).then_some(3)
        }
        fn container_stack(&self, _: BlockSide, slot: usize) -> Option<ItemStack> {
            self.slots.get(slot)?.clone()
        }
    }
    impl BackendMut for Shelf {
        fn container_insert(
            &mut self,
            _: BlockSide,
            slot: usize,
            stack: ItemStack,
            _: &Transaction,
        ) -> Option<ItemStack> {
            let held = self.slots.get(slot)?.clone();
            let room = match &held {
                _ if self.output == Some(slot) => 0,
                None => self.limit,
                Some(held) if held.same_item_same_components(&stack) => self.limit - held.count,
                Some(_) => 0,
            };
            let most = if self.one_per_insert { 1 } else { u32::MAX };
            let put = room.min(stack.count).min(most);
            if put > 0 {
                self.slots[slot] = Some(ItemStack {
                    count: held.map_or(0, |h| h.count) + put,
                    ..stack.clone()
                });
            }
            (put < stack.count).then(|| ItemStack {
                count: stack.count - put,
                ..stack
            })
        }
        fn container_extract(
            &mut self,
            _: BlockSide,
            slot: usize,
            max: u32,
            _: &Transaction,
        ) -> Option<ItemStack> {
            let most = if self.whole_extract { max } else { 1 };
            let held = self.slots.get_mut(slot)?.as_mut()?;
            let out = ItemStack {
                count: most.min(held.count),
                ..held.clone()
            };
            held.count -= out.count;
            if held.count == 0 {
                self.slots[slot] = None;
            }
            Some(out)
        }
    }
    impl Host for Shelf {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
        fn item_default_component(&self, item: &str, component: &str) -> Option<Tag> {
            // An iron sword's, as its item report gives them.
            let default = match component {
                "minecraft:max_damage" => Tag::Int(250),
                "minecraft:damage" => Tag::Int(0),
                "minecraft:enchantable" => {
                    Tag::Compound([("value".into(), Tag::Int(14))].into_iter().collect())
                }
                _ => return None,
            };
            (item == "minecraft:iron_sword").then_some(default)
        }
        fn block_entity(&self, level: LevelRef, position: [i32; 3]) -> Option<BlockEntityRef> {
            Some(BlockEntityRef { level, position })
        }
    }
    impl ServerHost for Shelf {
        fn transaction_open(&mut self, _: TransactionSeal) {
            open(self);
        }
        fn transaction_close(&mut self, _: TransactionSeal, commit: bool) {
            close(self, commit);
        }
        fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
            Some(BackendMutRef::of(self))
        }
    }
    impl Shelf {
        fn items(&self) -> u32 {
            items(self.slots.iter().flatten().cloned())
        }
    }

    /// A mod's handler for the shelf whose calls overstate: an extract takes and answers 5 more
    /// than asked, and an insert keeps nothing and answers 5 more than was offered.
    struct ItemOverstates;
    impl ResourceHandler<ItemResource> for ItemOverstates {
        fn size(&self, host: &mut dyn ServerHost) -> usize {
            backend_of(&*host)
                .and_then(|backend| backend.container_size(on(SHELF, None)))
                .unwrap_or(0)
        }
        fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
            Some(ItemResource::of(
                &backend_of(&*host)?.container_stack(on(SHELF, None), index)?,
            ))
        }
        fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
            backend_of(&*host)
                .and_then(|backend| backend.container_stack(on(SHELF, None), index))
                .map_or(0, |held| held.count)
        }
        fn capacity(
            &self,
            host: &mut dyn ServerHost,
            index: usize,
            resource: &ItemResource,
        ) -> u32 {
            backend_of(&*host).map_or(0, |backend| {
                backend.container_capacity(on(SHELF, None), index, &resource.to_stack(1))
            })
        }
        fn insert(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            amount: u32,
        ) -> u32 {
            amount + 5
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            _: &ItemResource,
            amount: u32,
        ) -> u32 {
            let Some(backend) = backend_mut_of(host) else {
                return 0;
            };
            backend
                .container_extract(on(SHELF, None), index, amount + 5, tx)
                .map_or(0, |out| out.count)
        }
    }

    /// State 1 is a `test:gauge`, state 2 a `test:other`.
    struct Level;
    impl LevelHost for Level {
        fn block_state(&self, _: [i32; 3]) -> BlockStateId {
            unimplemented!()
        }
        fn state_with_property(&self, _: BlockStateId, _: &str, _: &str) -> Option<BlockStateId> {
            None
        }
        fn state_block(&self, state: BlockStateId) -> Option<&str> {
            ["test:gauge", "test:other"]
                .get(state.0 as usize - 1)
                .copied()
        }
    }

    /// A chemical, as Mekanism's: the test mods' own resource, which the game knows nothing of.
    #[derive(Clone, Debug, PartialEq)]
    pub(crate) struct Chemical(pub(crate) String);

    /// The test mods' own capability, as Mekanism's `Capabilities.CHEMICAL`: the game has no
    /// answer for it, so only their providers do.
    pub(crate) const CHEMICALS: CapabilityKey<dyn ResourceHandler<Chemical>> =
        CapabilityKey::new("test:chemical");

    /// A mod block entity's one tank of 1000 of a chemical, kept in its saved data (`chemical`
    /// and `amount`), whose writes the game's journal keeps for a transaction.
    pub(crate) struct ChemTank(pub(crate) BlockSide);
    impl ChemTank {
        fn entity(&self, host: &dyn Host) -> Option<BlockEntityRef> {
            host.block_entity(self.0.level, self.0.pos)
        }
        fn held(&self, host: &dyn Host) -> Option<(Chemical, u32)> {
            let data = self.entity(host)?.data(host)?;
            let Some(Tag::String(chemical)) = data.get(&"chemical".into()) else {
                return None;
            };
            let Some(&Tag::Int(amount)) = data.get(&"amount".into()) else {
                return None;
            };
            let amount = u32::try_from(amount).ok().filter(|&a| a > 0)?;
            Some((Chemical(chemical.to_string_lossy()), amount))
        }
        fn set(&self, host: &mut dyn ServerHost, chemical: &Chemical, amount: u32) {
            let Some(entity) = self.entity(host) else {
                return;
            };
            let mut data = Compound::new();
            data.insert("chemical".into(), Tag::String(chemical.0.as_str().into()));
            data.insert("amount".into(), Tag::Int(amount as i32));
            entity.merge_data(host, data);
        }
    }
    impl ResourceHandler<Chemical> for ChemTank {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, host: &mut dyn ServerHost, _: usize) -> Option<Chemical> {
            self.held(host).map(|(chemical, _)| chemical)
        }
        fn amount(&self, host: &mut dyn ServerHost, _: usize) -> u32 {
            self.held(host).map_or(0, |(_, amount)| amount)
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &Chemical) -> u32 {
            1000
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            chemical: &Chemical,
            amount: u32,
        ) -> u32 {
            let held = match self.held(host) {
                Some((other, _)) if other != *chemical => return 0,
                Some((_, held)) => held,
                None => 0,
            };
            let went = amount.min(1000 - held);
            if went > 0 {
                self.set(host, chemical, held + went);
            }
            went
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            chemical: &Chemical,
            amount: u32,
        ) -> u32 {
            let Some((held_chemical, held)) = self.held(host).filter(|(c, _)| c == chemical) else {
                return 0;
            };
            let came = amount.min(held);
            self.set(host, &held_chemical, held - came);
            came
        }
    }

    /// A second test mod, sharing `CHEMICALS` and `Chemical` with the first, as a mod depending
    /// on another's `[api]` crate does: its vats answer the same capability.
    struct Chemist;
    #[mod_api::__subscriptions]
    impl Hooks for Chemist {
        fn register(&self, r: &mut Registrar) {
            r.block_entity_capability(&CHEMICALS, "test:chem_vat", |_, at| {
                Some(Box::new(ChemTank(at)))
            });
        }
    }

    /// The test mods, their parts installed, on a thread that runs a game.
    pub(crate) fn game_thread() -> super::super::Mods {
        let mods: super::super::Mods = vec![
            ModEntry::new("doubler", Box::new(Doubler)),
            ModEntry::new("chemist", Box::new(Chemist)),
        ]
        .into();
        super::super::install(mods.clone());
        install_parts(&mods).unwrap();
        mark_game_thread();
        mods
    }

    /// Install a weak-power part on glass for a real-game query.
    pub(crate) fn weak_power_game_thread() {
        struct WeakPower;
        impl BlockPart for WeakPower {
            fn should_check_weak_power(
                &self,
                _: &dyn LevelHost,
                event: &mut ShouldCheckWeakPowerEvent,
            ) {
                event.check = true;
            }
        }
        struct WeakPowerMod;
        #[mod_api::__subscriptions]
        impl Hooks for WeakPowerMod {
            fn register(&self, r: &mut Registrar) {
                r.block_part("minecraft:glass", WeakPower);
            }
        }
        let mods: super::super::Mods =
            vec![ModEntry::new("weak_power_test", Box::new(WeakPowerMod))].into();
        super::super::install(mods.clone());
        install_parts(&mods).unwrap();
        mark_game_thread();
    }

    /// Install a redstone connector on glass for real-game tests.
    pub(crate) fn redstone_game_thread() {
        struct RedstoneMod;
        #[mod_api::__subscriptions]
        impl Hooks for RedstoneMod {
            fn register(&self, r: &mut Registrar) {
                r.block_part("minecraft:glass", Connector);
            }
        }
        let mods: super::super::Mods =
            vec![ModEntry::new("redstone_test", Box::new(RedstoneMod))].into();
        super::super::install(mods.clone());
        install_parts(&mods).unwrap();
        mark_game_thread();
    }

    /// The comparator reading of state `state`, from 1.
    fn read(state: u32) -> i32 {
        let mut output = 1;
        dispatch_analog_output(
            &Level,
            LevelDimension::Overworld,
            BlockPos::new(0, 64, 0),
            BlockStateId(state),
            Direction::North,
            &mut output,
        );
        output
    }

    /// Resolve this thread's parts with tag `test:gauged` holding `members`.
    fn resolve(members: &[&str]) -> Result<(), Vec<String>> {
        let tags = HashMap::from([(
            "test:gauged".to_owned(),
            members.iter().map(|m| m.to_string()).collect(),
        )]);
        resolve_parts(
            &tags,
            &HashMap::new(),
            KnownIds {
                block: &|_| true,
                item: &|_| true,
                state_count: 4,
                state_block: &|state| match state {
                    1 => "test:gauge",
                    2 => "test:other",
                    3 => "test:connector",
                    _ => "minecraft:air",
                },
            },
        )
    }

    #[test]
    fn a_block_part_answers_for_its_block_only_after_the_mods() {
        game_thread();
        // No world is open, so no state has its parts yet: only the mod's own hook answers.
        assert_eq!(read(1), 2);
        resolve(&[]).unwrap();
        // Doubled by the mod, then 5 more from the part: (1 * 2) + 5.
        assert_eq!(read(1), 7);
        assert_eq!(read(2), 2);
    }

    #[test]
    fn redstone_connection_is_scoped_to_the_attached_block() {
        game_thread();
        resolve(&[]).unwrap();
        for (state, expected) in [(3, true), (2, false)] {
            let mut connected = false;
            dispatch_should_redstone_wire_connect_to(
                &Level,
                LevelDimension::Overworld,
                BlockPos::new(0, 64, 0),
                BlockStateId(state),
                Some(Direction::East),
                &mut connected,
            );
            assert_eq!(connected, expected);
        }
    }

    #[test]
    fn a_state_without_parts_answers_none_and_keeps_the_global_hook() {
        game_thread();
        resolve(&[]).unwrap();
        // Out of range, and in range but a block no part is attached to.
        assert!(
            super::parts("analog_output")
                .block_state(BlockStateId(99))
                .is_empty()
        );
        assert!(
            super::parts("analog_output")
                .block_state(BlockStateId(0))
                .is_empty()
        );
        // The mod's hook still hears both: no parts is not no subscribers.
        assert_eq!(read(0), 2);
        assert_eq!(read(99), 2);
    }

    #[test]
    fn each_game_answers_from_its_own_tags_and_reload_drops_what_left() {
        let mods = game_thread();
        // Both games resolve before either reads, so one shared table would fail one of them.
        let resolved = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let client = scope.spawn(|| {
                super::super::install(mods.clone());
                install_parts(&mods).unwrap();
                mark_game_thread();
                resolve(&["test:other"]).unwrap();
                resolved.wait();
                (read(1), read(2))
            });
            let server = scope.spawn(|| {
                super::super::install(mods.clone());
                install_parts(&mods).unwrap();
                mark_game_thread();
                resolve(&["test:gauge"]).unwrap();
                resolved.wait();
                let before = (read(1), read(2));
                // `/reload` with the tag emptied.
                resolve(&[]).unwrap();
                (before, read(1))
            });
            // The tag part comes after the key part of the same mod: (1 * 2) + 5 + 100.
            assert_eq!(client.join().unwrap(), (7, 102));
            assert_eq!(server.join().unwrap(), ((107, 2), 7));
        });
    }

    #[test]
    fn a_key_the_game_lacks_fails_naming_the_mod() {
        game_thread();
        let refused = resolve_parts(
            &HashMap::new(),
            &HashMap::new(),
            KnownIds {
                block: &|id| id != "test:gauge",
                item: &|_| true,
                state_count: 0,
                state_block: &|_| unreachable!(),
            },
        )
        .unwrap_err();
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert!(refused[0].starts_with("doubler attached "));
        assert!(refused[0].contains("Gauge to block test:gauge, which this game does not have"));
    }

    #[test]
    fn a_tag_giving_a_part_twice_keeps_the_earlier_mod() {
        let part = |mod_index| Attached::<dyn BlockPart> {
            mod_id: ["earlier", "later"][mod_index],
            mod_index,
            order: 0,
            kind: TypeId::of::<Gauge>(),
            type_name: "Gauge",
            part: Arc::new(Gauge),
        };
        let mut table = HashMap::from([("test:gauge".into(), vec![part(1)])]);
        let tags = HashMap::from([(
            "test:gauged".to_owned(),
            HashSet::from(["test:gauge".to_owned()]),
        )]);
        super::add_tagged(
            &mut table,
            &[("test:gauged".into(), part(0))],
            &tags,
            "block",
        );
        let kept: Vec<usize> = table["test:gauge"].iter().map(|a| a.mod_index).collect();
        assert_eq!(kept, [0]);
    }

    #[test]
    fn a_level_not_in_place_asks_no_provider() {
        game_thread();
        let mut world = PipeWorld::default();
        let host: &mut dyn ServerHost = &mut world;
        // The pipe's type has a provider, which answers in the level in place; parked while
        // the server ticks another, the same block entity's level gives none.
        let parked = LevelRef::new(NativeWorldEpoch::new(1), "minecraft:the_nether");
        assert!(here().capability(host, &ITEMS, PIPE, None).is_some());
        assert!(parked.capability(host, &ITEMS, PIPE, None).is_none());
    }

    #[test]
    fn a_mod_reads_a_vanilla_chests_items_from_the_game_with_no_provider_for_it() {
        game_thread();
        let mut world = PipeWorld {
            chest: Some(stick()),
            ..PipeWorld::default()
        };
        let host: &mut dyn ServerHost = &mut world;
        // No mod registered an items provider for the chest's type: the game's hoppers have no
        // mod's handler to look for there, yet a mod reading it gets the game's own slots.
        assert!(block_entity_providers(&ITEMS, host, on(CHEST, None)).is_none());
        let chest = here().capability(host, &ITEMS, CHEST, None).unwrap();
        assert_eq!(chest.owner(), "the game");
        assert_eq!(chest.size(host), 1);
        assert_eq!(chest.resource(host, 0), Some(ItemResource::of(&stick())));
    }

    #[test]
    fn a_capability_part_answers_by_side_and_forwards_to_its_neighbours_capability() {
        game_thread();
        let mut world = PipeWorld::default();
        let host: &mut dyn ServerHost = &mut world;
        let up = here()
            .capability(host, &ITEMS, PIPE, Some(Direction::Up))
            .unwrap();
        assert_eq!(up.size(host), 0);
        let pipe = here()
            .capability(host, &ITEMS, PIPE, Some(Direction::North))
            .expect("the pipe's part");
        assert_eq!(pipe.size(host), 1);
        let stick_ = ItemResource::of(&stick());
        // The pipe calls the chest's capability, the game's own, with the host it was given:
        // in an aborted transaction the stick goes in, and comes back out.
        let aborted: Result<(), u32> =
            host.transaction(|host, tx| Err(pipe.insert(host, tx, 0, &stick_, 1)));
        assert_eq!(aborted, Err(1));
        assert!(
            backend_of(&*host)
                .and_then(|backend| backend.container_stack(on(CHEST, None), 0))
                .is_none()
        );
        let went = host.transaction(|host, tx| Ok::<_, ()>(pipe.insert(host, tx, 0, &stick_, 1)));
        assert_eq!(went, Ok(1));
        assert_eq!(
            backend_of(&*host).and_then(|backend| backend.container_stack(on(CHEST, None), 0)),
            Some(stick())
        );
        let went = host.transaction(|host, tx| Ok::<_, ()>(pipe.insert(host, tx, 0, &stick_, 1)));
        assert_eq!(went, Ok(0));
        // No block entity with slots: no answer. A mod's own capability has no game answer.
        assert!(here().capability(host, &ITEMS, [5, 64, 0], None).is_none());
        const CHEMICALS: CapabilityKey<dyn ResourceHandler<ItemResource>> =
            CapabilityKey::new("test:chemicals");
        assert!(here().capability(host, &CHEMICALS, CHEST, None).is_none());
    }

    /// Puts `amount` of `item` into slot 0 of [`ITEM_ITEMS`]' handler for the item at `place`, in a
    /// transaction committed when all went in: how much did.
    fn fill(host: &mut dyn ServerHost, place: &ItemPlace, item: ItemStack, amount: u32) -> u32 {
        let handler = place
            .capability(host, &ITEM_ITEMS)
            .expect("the item's part");
        let resource = ItemResource::of(&item);
        let went =
            host.transaction(
                |host, tx| match handler.insert(host, tx, 0, &resource, amount) {
                    went if went == amount => Ok(went),
                    went => Err(went),
                },
            );
        went.unwrap_or_else(|went| went)
    }

    #[test]
    fn an_item_handler_writes_its_stack_back_where_it_lives_splitting_one_off() {
        game_thread();
        let mut player = Player {
            main_hand: Some(stack("test:pouch", 2, 0)),
            ..Player::default()
        };
        // Aborted, nothing changes.
        let host: &mut dyn ServerHost = &mut player;
        let pouch = HAND.capability(host, &ITEM_ITEMS).unwrap();
        let sticks = ItemResource::of(&stick());
        let aborted: Result<(), u32> =
            host.transaction(|host, tx| Err(pouch.insert(host, tx, 0, &sticks, 3)));
        assert_eq!(aborted, Err(3));
        assert_eq!(player.main_hand, Some(stack("test:pouch", 2, 0)));
        assert!(player.given.is_empty());
        // One of the two pouches is filled: it leaves the hand for the player, the other stays.
        let host: &mut dyn ServerHost = &mut player;
        assert_eq!(fill(host, &HAND, stick(), 3), 3);
        assert_eq!(player.main_hand, Some(stack("test:pouch", 1, 0)));
        assert_eq!(player.given, [stack("test:pouch", 1, 3)]);
        assert_eq!(player.items(), 2 + 3);
        // An item with no part and no game slots has no answer.
        player.main_hand = Some(stack("minecraft:stone", 1, 0));
        let host: &mut dyn ServerHost = &mut player;
        assert!(HAND.capability(host, &ITEM_ITEMS).is_none());
    }

    /// A mod's energy cell at `.0`: up to 1000, kept in the cell's own stack (its damage, for the
    /// test), so the handler holds nothing.
    struct Cell(ItemPlace);
    impl Cell {
        /// The cell here rewritten as holding `energy`: `moved` when that goes through, else 0.
        fn rewrite(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            energy: u32,
            moved: u32,
        ) -> u32 {
            let charged = ItemResource::of(&stack("test:cell", 1, energy));
            match self.0.exchange(host, tx, &charged, 1) {
                1 => moved,
                _ => 0,
            }
        }
    }
    impl EnergyHandler for Cell {
        fn amount(&self, host: &mut dyn ServerHost) -> u32 {
            self.0.stack(host).map_or(0, |cell| cell.damage)
        }
        fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
            1000
        }
        fn insert(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
            let held = self.amount(host);
            match amount.min(1000 - held) {
                0 => 0,
                went => self.rewrite(host, tx, held + went, went),
            }
        }
        fn extract(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
            let held = self.amount(host);
            match amount.min(held) {
                0 => 0,
                came => self.rewrite(host, tx, held - came, came),
            }
        }
    }

    #[test]
    fn an_items_energy_moves_through_its_place_and_a_charged_one_splits_off_its_stack() {
        game_thread();
        let slot = ItemPlace::PlayerSlot { player: 1, slot: 9 };
        let mut player = Player {
            main_hand: Some(stack("test:cell", 2, 600)),
            ..Player::default()
        };
        player.slots.insert(9, stack("test:cell", 1, 900));
        let host: &mut dyn ServerHost = &mut player;
        let hand = HAND.capability(host, &ITEM_ENERGY).unwrap();
        let other = slot.capability(host, &ITEM_ENERGY).unwrap();
        // The other cell has room for 100: one of the two in the hand gives that and splits off.
        let moved = host.transaction(|host, tx| move_energy(host, &hand, &other, 400, tx));
        assert_eq!(moved, Ok(100));
        assert_eq!(player.slots[&9], stack("test:cell", 1, 1000));
        assert_eq!(player.main_hand, Some(stack("test:cell", 1, 600)));
        assert_eq!(player.given, [stack("test:cell", 1, 500)]);
        // Aborted, a move leaves both as they were.
        let host: &mut dyn ServerHost = &mut player;
        let aborted = host.transaction(|host, tx| {
            let moved = move_energy(host, &other, &hand, 400, tx);
            Err::<(), _>(moved)
        });
        assert_eq!(aborted, Err(Ok(400)));
        assert_eq!(player.slots[&9], stack("test:cell", 1, 1000));
        assert_eq!(player.main_hand, Some(stack("test:cell", 1, 600)));
        // An item no mod answers for has no energy.
        player.main_hand = Some(stick());
        let host: &mut dyn ServerHost = &mut player;
        assert!(HAND.capability(host, &ITEM_ENERGY).is_none());
    }

    #[test]
    fn a_stack_inside_an_item_and_a_mods_place_are_reached_through_their_handlers() {
        game_thread();
        let mut player = Player {
            main_hand: Some(stack("test:bag", 1, 1)),
            ..Player::default()
        };
        player.slots.insert(20 + 4, stack("test:pouch", 1, 2));
        let host: &mut dyn ServerHost = &mut player;
        // The empty pouch in the bag in the hand takes 3 sticks: the pouch is rewritten in the
        // bag, and the bag in the hand.
        let in_bag = ItemPlace::Nested {
            parent: Box::new(HAND),
            slot: 0,
        };
        assert_eq!(fill(host, &in_bag, stick(), 3), 3);
        assert_eq!(in_bag.stack(host), Some(stack("test:pouch", 1, 3)));
        assert_eq!(HAND.stack(host), Some(stack("test:bag", 1, 4)));
        // A charm slot is a mod's place, read and written by its provider.
        let charm = ItemPlace::Mod {
            provider: "test:charms",
            key: 4,
        };
        assert_eq!(fill(host, &charm, stick(), 1), 1);
        assert_eq!(charm.stack(host), Some(stack("test:pouch", 1, 3)));
        let unknown = ItemPlace::Mod {
            provider: "test:none",
            key: 4,
        };
        assert!(unknown.stack(host).is_none());
        assert_eq!(insert(host, &unknown, stick()), 0);
    }

    /// `place.insert` of `stack` in a transaction of its own: how much went.
    fn insert(host: &mut dyn ServerHost, place: &ItemPlace, stack: ItemStack) -> u32 {
        let resource = ItemResource::of(&stack);
        let went = host
            .transaction(|host, tx| Ok::<_, ()>(place.insert(host, tx, &resource, stack.count)));
        went.unwrap_or(0)
    }

    /// `place.exchange` of `stack.count` of what is there for `stack`, in a transaction of its
    /// own: how many.
    fn exchange(host: &mut dyn ServerHost, place: &ItemPlace, stack: ItemStack) -> u32 {
        let resource = ItemResource::of(&stack);
        let swapped = host
            .transaction(|host, tx| Ok::<_, ()>(place.exchange(host, tx, &resource, stack.count)));
        swapped.unwrap_or(0)
    }

    /// Two three-slot game chests, `FROM` and `INTO`; `INTO`'s slots hold up to `limit` each.
    #[derive(Clone)]
    struct TwoChests {
        from: [Option<ItemStack>; 3],
        into: [Option<ItemStack>; 3],
        limit: u32,
        kept: Vec<Self>,
    }
    const FROM: [i32; 3] = [3, 64, 0];
    const INTO: [i32; 3] = [4, 64, 0];
    impl TwoChests {
        fn slots(&mut self, pos: [i32; 3]) -> Option<(&mut [Option<ItemStack>; 3], u32)> {
            match pos {
                FROM => Some((&mut self.from, 64)),
                INTO => Some((&mut self.into, self.limit)),
                _ => None,
            }
        }
        fn count(slots: &[Option<ItemStack>; 3], item: &str) -> u32 {
            slots
                .iter()
                .flatten()
                .filter(|s| &*s.item == item)
                .map(|s| s.count)
                .sum()
        }
    }
    impl Journal for TwoChests {
        fn kept(&mut self) -> &mut Vec<Self> {
            &mut self.kept
        }
    }
    impl Backend for TwoChests {
        fn level_in_place(&self, level: LevelRef) -> bool {
            level == here()
        }
        fn container_size(&self, BlockSide { pos, .. }: BlockSide) -> Option<usize> {
            [FROM, INTO].contains(&pos).then_some(3)
        }
        fn container_stack(
            &self,
            BlockSide { pos, .. }: BlockSide,
            slot: usize,
        ) -> Option<ItemStack> {
            match pos {
                FROM => self.from.get(slot)?.clone(),
                INTO => self.into.get(slot)?.clone(),
                _ => None,
            }
        }
        fn container_capacity(&self, _: BlockSide, _: usize, _: &ItemStack) -> u32 {
            self.limit
        }
    }
    impl BackendMut for TwoChests {
        fn container_insert(
            &mut self,
            BlockSide { pos, .. }: BlockSide,
            slot: usize,
            stack: ItemStack,
            _: &Transaction,
        ) -> Option<ItemStack> {
            let Some((slots, limit)) = self.slots(pos) else {
                return Some(stack);
            };
            let held = slots[slot].clone();
            let room = match &held {
                None => limit,
                Some(held) if held.same_item_same_components(&stack) => limit - held.count,
                Some(_) => 0,
            };
            let put = room.min(stack.count);
            if put > 0 {
                slots[slot] = Some(ItemStack {
                    count: held.map_or(0, |h| h.count) + put,
                    ..stack.clone()
                });
            }
            (put < stack.count).then(|| ItemStack {
                count: stack.count - put,
                ..stack
            })
        }
        fn container_extract(
            &mut self,
            BlockSide { pos, .. }: BlockSide,
            slot: usize,
            max: u32,
            _: &Transaction,
        ) -> Option<ItemStack> {
            let (slots, _) = self.slots(pos)?;
            let held = slots[slot].as_mut()?;
            let out = ItemStack {
                count: max.min(held.count),
                ..held.clone()
            };
            held.count -= out.count;
            if held.count == 0 {
                slots[slot] = None;
            }
            Some(out)
        }
    }
    impl Host for TwoChests {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
        fn block_entity(&self, _: LevelRef, _: [i32; 3]) -> Option<BlockEntityRef> {
            None
        }
    }
    impl ServerHost for TwoChests {
        fn transaction_open(&mut self, _: TransactionSeal) {
            open(self);
        }
        fn transaction_close(&mut self, _: TransactionSeal, commit: bool) {
            close(self, commit);
        }
        fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
            Some(BackendMutRef::of(self))
        }
    }

    /// A target that answers one more than it was given, and keeps nothing.
    struct TakesMore;
    impl ResourceHandler<ItemResource> for TakesMore {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            None
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            0
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
            64
        }
        fn insert(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            n: u32,
        ) -> u32 {
            n + 1
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    #[test]
    fn a_move_takes_what_the_filter_accepts_up_to_max_and_leaves_what_the_target_refuses() {
        game_thread();
        let sticks = |n| Some(stack("minecraft:stick", n, 0));
        let apples = Some(stack("minecraft:apple", 3, 0));
        let mut world = TwoChests {
            from: [sticks(5), apples, sticks(4)],
            into: [None, None, None],
            limit: 64,
            kept: Vec::new(),
        };
        let only_sticks = |r: &ItemResource| r.item() == "minecraft:stick";
        let moved = |world: &mut TwoChests, max| {
            let host: &mut dyn ServerHost = world;
            let from = here().capability(host, &ITEMS, FROM, None).unwrap();
            let to = here().capability(host, &ITEMS, INTO, None).unwrap();
            host.transaction(|host, tx| move_resources(host, &from, &to, only_sticks, max, tx))
                .unwrap()
        };
        // Up to 7 sticks: all of the first slot's, two of the third's; the apples stay.
        assert_eq!(moved(&mut world, 7), 7);
        assert_eq!(TwoChests::count(&world.from, "minecraft:stick"), 2);
        assert_eq!(TwoChests::count(&world.into, "minecraft:stick"), 7);
        assert_eq!(TwoChests::count(&world.from, "minecraft:apple"), 3);
        // A target with room for one more of the two (its other slots hold apples): that one
        // moves, the other stays put.
        world.limit = 8;
        world.into[1] = Some(stack("minecraft:apple", 1, 0));
        world.into[2] = Some(stack("minecraft:apple", 1, 0));
        assert_eq!(moved(&mut world, 64), 1);
        assert_eq!(TwoChests::count(&world.from, "minecraft:stick"), 1);
        assert_eq!(TwoChests::count(&world.into, "minecraft:stick"), 8);
        assert_eq!(moved(&mut world, 64), 0);
        assert_eq!(TwoChests::count(&world.from, "minecraft:stick"), 1);
    }

    #[test]
    fn a_move_into_a_handler_answering_more_than_it_was_given_is_undone() {
        game_thread();
        let mut world = TwoChests {
            from: [Some(stack("minecraft:stick", 5, 0)), None, None],
            into: [None, None, None],
            limit: 64,
            kept: Vec::new(),
        };
        let host: &mut dyn ServerHost = &mut world;
        let from = here().capability(host, &ITEMS, FROM, None).unwrap();
        let liar: Capability<dyn ResourceHandler<ItemResource>> = Capability {
            handler: Box::new(TakesMore),
            owner: Some("doubler"),
        };
        let moved =
            host.transaction(|host, tx| move_resources(host, &from, &liar, |_| true, 64, tx));
        let owner = "doubler".to_owned();
        assert_eq!(moved, Err(Overanswered { owner }));
        assert_eq!(world.from[0], Some(stack("minecraft:stick", 5, 0)));
        assert!(super::super::item_place::warned_about("doubler"));
    }

    #[test]
    fn a_changed_component_is_an_exchange_of_one_that_splits_it_off_and_an_abort_puts_it_back() {
        game_thread();
        let sword = |count, damage| stack("minecraft:iron_sword", count, damage);
        let mut shelf_ = shelf([Some(sword(2, 0)), None, None]);
        let host: &mut dyn ServerHost = &mut shelf_;
        let place = shelf_slot(0);
        let plain = place.resource(host).unwrap();
        let damage = ComponentId::new("minecraft:damage").unwrap();
        assert_eq!(plain.component(&damage), None);
        let worn = plain.with_component(host, &damage, Tag::Int(5));
        assert_eq!(worn.component(&damage), Some(Tag::Int(5)));
        // Aborted, nothing changed; committed, one sword of the two is worn and splits off.
        let aborted = host.transaction(|host, tx| {
            assert_eq!(place.exchange(host, tx, &worn, 1), 1);
            Err::<(), ()>(())
        });
        assert!(aborted.is_err());
        assert_eq!(shelf_.slots, [Some(sword(2, 0)), None, None]);
        let host: &mut dyn ServerHost = &mut shelf_;
        assert_eq!(exchange(host, &place, sword(1, 5)), 1);
        assert_eq!(shelf_.slots, [Some(sword(1, 0)), Some(sword(1, 5)), None]);
    }

    #[test]
    fn a_removed_default_component_is_recorded_in_the_patch_and_a_default_set_one_is_dropped() {
        game_thread();
        let shelf_ = shelf([None, None, None]);
        let host: &dyn Host = &shelf_;
        let sword = ItemResource::of(&stack("minecraft:iron_sword", 1, 0));
        let patch = |resource: &ItemResource| {
            let Some(Tag::Compound(patch)) =
                crate::item_data::encode_components(&resource.to_stack(1))
            else {
                return Vec::new();
            };
            patch
                .keys()
                .map(|key| key.to_string_lossy())
                .collect::<Vec<_>>()
        };
        // An iron sword has `max_damage` by default: removing it is a removal in its patch,
        // so the sword is no longer the plain one.
        let max_damage = ComponentId::new("minecraft:max_damage").unwrap();
        let unbreakable = sword.without_component(host, &max_damage);
        assert_eq!(patch(&unbreakable), ["!minecraft:max_damage"]);
        assert_ne!(unbreakable, sword);
        assert_eq!(unbreakable.component(&max_damage), None);
        // Set again, the removal goes and the value is the patch's own.
        let set = unbreakable.with_component(host, &max_damage, Tag::Int(10));
        assert_eq!(patch(&set), ["minecraft:max_damage"]);
        assert_eq!(set.component(&max_damage), Some(Tag::Int(10)));
        // A component the item hasn't by default only loses the stack's own value.
        let data = ComponentId::new("minecraft:custom_data").unwrap();
        let tagged = sword.with_component(host, &data, Tag::Compound(Compound::new()));
        assert_ne!(tagged, sword);
        assert_eq!(tagged.without_component(host, &data), sword);
        // Set to the item's default, a component keeps no value of the stack's own, as vanilla's
        // `set` drops it: the stack is the plain item, whatever NBT type the mod wrote.
        assert_eq!(set.with_component(host, &max_damage, Tag::Int(250)), sword);
        assert_eq!(
            unbreakable.with_component(host, &max_damage, Tag::Short(250)),
            sword
        );
        let enchantable = ComponentId::new("minecraft:enchantable").unwrap();
        let value = |v| Tag::Compound([("value".into(), v)].into_iter().collect());
        assert_eq!(
            sword.with_component(host, &enchantable, value(Tag::Byte(14))),
            sword
        );
        assert_ne!(
            sword.with_component(host, &enchantable, value(Tag::Int(15))),
            sword
        );
    }

    #[test]
    fn a_component_set_keeps_a_change_its_codec_reads_and_raw_nbt_keeps_its_types() {
        /// A stick's report: `custom_data` `{x: 1}`, a huge `max_damage` and a mod's `test:long`.
        struct Report;
        impl Host for Report {
            fn item_default_component(&self, _: &str, id: &str) -> Option<Tag> {
                let x = || Tag::Compound([("x".into(), Tag::Int(1))].into_iter().collect());
                match id {
                    "minecraft:custom_data" => Some(x()),
                    "minecraft:max_damage" | "test:long" => Some(Tag::Long(1 << 53)),
                    _ => None,
                }
            }
        }
        let stick = ItemResource::of(&stick());
        let set = |id, value: Tag| {
            let id = ComponentId::new(id).unwrap();
            stick.with_component(&Report, &id, value).component(&id)
        };
        // `custom_data` is kept as the NBT given (`CustomData.equals`): a byte where the report
        // has an int is a change, while the same int is the default.
        let x = |v| Tag::Compound([("x".into(), v)].into_iter().collect());
        assert_eq!(
            set("minecraft:custom_data", x(Tag::Byte(1))),
            Some(x(Tag::Byte(1)))
        );
        assert_eq!(set("minecraft:custom_data", x(Tag::Int(1))), None);
        // Integers compare exactly, past where an `f64` keeps them apart.
        let past = Tag::Long((1 << 53) + 1);
        assert_eq!(
            set("minecraft:max_damage", past.clone()),
            Some(past.clone())
        );
        assert_eq!(set("test:long", past.clone()), Some(past));
        assert_eq!(set("minecraft:max_damage", Tag::Int(0)), Some(Tag::Int(0)));
        // A vanilla component's codec reads any integer width: the default's value is it.
        assert_eq!(set("minecraft:max_damage", Tag::Long(1 << 53)), None);
        // A mod's component, whose codec the game doesn't know, is equal type for type.
        let double = Tag::Double(((1u64 << 53) as f64).to_bits());
        assert_eq!(set("test:long", double.clone()), Some(double));
    }

    #[test]
    fn a_component_id_is_a_namespaced_identifier() {
        for id in ["minecraft:damage", "mymod:charge/level"] {
            assert_eq!(
                ComponentId::new(id).map(|id| id.as_str().to_owned()),
                Some(id.into())
            );
        }
        for id in [
            "damage",
            "!minecraft:food",
            "not a namespaced id!",
            "Mymod:charge",
            ":path",
            "mymod:",
        ] {
            assert!(ComponentId::new(id).is_none(), "{id}");
        }
    }

    #[test]
    fn a_write_that_cant_go_through_changes_nothing_and_no_item_is_made_or_lost() {
        game_thread();
        let bread = |count| stack("minecraft:bread", count, 0);
        let stone = || Some(stack("minecraft:stone", 64, 0));
        // An output slot, as a furnace's, beside full ones: its iron comes out, but the bread
        // can go nowhere, so the iron goes back.
        let iron = || Some(stack("minecraft:iron_ingot", 1, 0));
        let mut shelf_ = shelf([iron(), stone(), stone()]);
        shelf_.output = Some(0);
        assert_eq!(exchange(&mut shelf_, &shelf_slot(0), bread(1)), 0);
        assert_eq!(shelf_.slots, [iron(), stone(), stone()]);
        // One of ten apples becomes the bread, which goes into the next slot.
        let mut shelf_ = shelf([Some(stack("minecraft:apple", 10, 0)), None, None]);
        assert_eq!(exchange(&mut shelf_, &shelf_slot(0), bread(1)), 1);
        assert_eq!(
            shelf_.slots,
            [Some(stack("minecraft:apple", 9, 0)), Some(bread(1)), None]
        );
        assert_eq!(shelf_.items(), 10);

        // Two empty pouches on a full shelf: a filled one has nowhere to go, so neither changes
        // and the sticks stay out.
        let mut shelf = shelf([Some(stack("test:pouch", 2, 0)), stone(), stone()]);
        let before = shelf.items();
        assert_eq!(fill(&mut shelf, &shelf_slot(0), stick(), 3), 0);
        assert_eq!(shelf.items(), before);
        assert_eq!(shelf.slots[0], Some(stack("test:pouch", 2, 0)));
        // With a slot free, one pouch is filled there.
        shelf.slots[2] = None;
        let before = shelf.items();
        assert_eq!(fill(&mut shelf, &shelf_slot(0), stick(), 3), 3);
        assert_eq!(
            shelf.slots,
            [
                Some(stack("test:pouch", 1, 0)),
                stone(),
                Some(stack("test:pouch", 1, 3))
            ]
        );
        assert_eq!(shelf.items(), before + 3);

        // Two pouches in a charm slot, which can't split: nothing changes. So with a bag in
        // that slot holding the pouch filled.
        let mut player = Player::default();
        player.slots.insert(20 + 1, stack("test:pouch", 2, 0));
        player.slots.insert(20 + 2, stack("test:bag", 2, 1));
        let before = player.items();
        for charm in [
            ItemPlace::Mod {
                provider: "test:charms",
                key: 1,
            },
            ItemPlace::Nested {
                parent: Box::new(ItemPlace::Mod {
                    provider: "test:charms",
                    key: 2,
                }),
                slot: 0,
            },
        ] {
            assert_eq!(fill(&mut player, &charm, stick(), 3), 0);
        }
        assert_eq!(player.items(), before);
        assert_eq!(player.slots[&21], stack("test:pouch", 2, 0));
        assert_eq!(player.slots[&22], stack("test:bag", 2, 1));
    }

    #[test]
    fn a_truthful_handler_moving_one_per_call_is_moved_in_passes_and_never_named() {
        game_thread();
        // A slot that gives one item per extract and takes one per insert: the 3 apples come
        // out, and the 3 bread go in, one pass each.
        let mut shelf_ = shelf([Some(stack("minecraft:apple", 3, 0)), None, None]);
        shelf_.one_per_insert = true;
        let bread = stack("minecraft:bread", 3, 0);
        assert_eq!(exchange(&mut shelf_, &shelf_slot(0), bread.clone()), 3);
        assert_eq!(shelf_.slots, [Some(bread), None, None]);
        assert!(!super::super::item_place::warned_about("the game"));
    }

    #[test]
    fn an_item_handler_answering_more_than_it_was_asked_or_offered_moves_nothing_and_is_named() {
        game_thread();
        let apples = |count| stack("minecraft:apple", count, 0);
        let bread = stack("minecraft:bread", 1, 0);
        let mut shelf_ = shelf([Some(apples(10)), None, None]);
        shelf_.whole_extract = true;
        shelf_.overstates = true;
        // Asked for one apple to swap for the bread, it takes and answers 6: the move is undone,
        // and the shelf keeps its 10.
        assert_eq!(exchange(&mut shelf_, &shelf_slot(0), bread), 0);
        assert_eq!(shelf_.slots, [Some(apples(10)), None, None]);
        // Offered 2 apples, it keeps none and answers 7 taken: undone, and the slot stays empty.
        assert_eq!(insert(&mut shelf_, &shelf_slot(1), apples(2)), 0);
        assert_eq!(shelf_.slots, [Some(apples(10)), None, None]);
        assert!(super::super::item_place::warned_about("doubler"));
    }

    #[test]
    fn a_change_inside_one_of_two_bags_splits_that_bag_off_whole() {
        game_thread();
        // Two bags in the hand, each holding an empty pouch.
        let mut player = Player {
            main_hand: Some(stack("test:bag", 2, 1)),
            ..Player::default()
        };
        let before = player.items();
        let in_bag = ItemPlace::Nested {
            parent: Box::new(HAND),
            slot: 0,
        };
        assert_eq!(fill(&mut player, &in_bag, stick(), 3), 3);
        // One bag leaves the hand holding the filled pouch; the other stays as it was, and no
        // pouch is loose.
        assert_eq!(player.main_hand, Some(stack("test:bag", 1, 1)));
        assert_eq!(player.given, [stack("test:bag", 1, 4)]);
        assert_eq!(player.items(), before + 3);
    }

    #[test]
    fn an_aborted_write_inside_a_nested_item_puts_every_level_back() {
        game_thread();
        // The pouch in the bag in the hand is filled in a transaction that then aborts: the
        // bag and its pouch are as they were, and nothing was given.
        let mut player = Player {
            main_hand: Some(stack("test:bag", 2, 1)),
            ..Player::default()
        };
        let in_bag = ItemPlace::Nested {
            parent: Box::new(HAND),
            slot: 0,
        };
        let host: &mut dyn ServerHost = &mut player;
        let pouch = in_bag.capability(host, &ITEM_ITEMS).unwrap();
        let sticks = ItemResource::of(&stick());
        let aborted: Result<(), u32> =
            host.transaction(|host, tx| Err(pouch.insert(host, tx, 0, &sticks, 3)));
        assert_eq!(aborted, Err(3));
        assert_eq!(player.main_hand, Some(stack("test:bag", 2, 1)));
        assert!(player.given.is_empty());
    }

    #[test]
    fn an_aborted_inner_write_of_a_handler_inside_an_item_changes_nothing() {
        game_thread();
        // A case of one diamond and one stick in the hand: its stick is taken and two put in;
        // on the way it tries emptying its diamond slot in a transaction it aborts. The
        // diamond stays, and no item is made or lost.
        let mut player = Player {
            main_hand: Some(stack("test:case", 1, 101)),
            ..Player::default()
        };
        let sticks = ItemPlace::Nested {
            parent: Box::new(HAND),
            slot: 1,
        };
        let host: &mut dyn ServerHost = &mut player;
        let resource = ItemResource::of(&stick());
        host.transaction(|host, tx| {
            assert_eq!(sticks.extract(host, tx, &resource, 1), 1);
            assert_eq!(sticks.insert(host, tx, &resource, 2), 2);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(player.main_hand, Some(stack("test:case", 1, 102)));
        assert!(player.given.is_empty());
    }

    mod deep_places {
        use super::*;

        /// An outer bag at `.0` holding one `test:bag`, kept in its damage as `Bag` keeps its
        /// pouch: 0 when empty.
        pub(super) struct Wrap(pub ItemPlace);
        impl ResourceHandler<ItemResource> for Wrap {
            fn size(&self, _: &mut dyn ServerHost) -> usize {
                1
            }
            fn resource(&self, host: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
                let n = self.0.stack(host)?.damage;
                (n > 0).then(|| ItemResource::of(&stack("test:bag", 1, n - 1)))
            }
            fn amount(&self, host: &mut dyn ServerHost, i: usize) -> u32 {
                u32::from(self.resource(host, i).is_some())
            }
            fn capacity(&self, _: &mut dyn ServerHost, _: usize, r: &ItemResource) -> u32 {
                u32::from(r.item() == "test:bag")
            }
            fn insert(
                &self,
                host: &mut dyn ServerHost,
                tx: &Transaction,
                _: usize,
                r: &ItemResource,
                n: u32,
            ) -> u32 {
                if n == 0 || r.item() != "test:bag" || self.0.stack(host).unwrap().damage != 0 {
                    return 0;
                }
                let new = ItemResource::of(&stack("test:outer", 1, r.to_stack(1).damage + 1));
                self.0.exchange(host, tx, &new, 1)
            }
            fn extract(
                &self,
                host: &mut dyn ServerHost,
                tx: &Transaction,
                i: usize,
                r: &ItemResource,
                n: u32,
            ) -> u32 {
                if n == 0 || self.resource(host, i).as_ref() != Some(r) {
                    return 0;
                }
                let new = ItemResource::of(&stack("test:outer", 1, 0));
                self.0.exchange(host, tx, &new, 1)
            }
        }

        /// A mod place that is the player's hand, and answers one more than it moved.
        pub(super) struct LyingPlace;
        impl PlaceProvider for LyingPlace {
            fn stack(&self, host: &mut dyn ServerHost, _: u64) -> Option<ItemStack> {
                HAND.stack(host)
            }
            fn insert(
                &self,
                host: &mut dyn ServerHost,
                tx: &Transaction,
                _: u64,
                r: &ItemResource,
                n: u32,
            ) -> u32 {
                HAND.insert(host, tx, r, n) + 1
            }
            fn extract(
                &self,
                host: &mut dyn ServerHost,
                tx: &Transaction,
                _: u64,
                r: &ItemResource,
                n: u32,
            ) -> u32 {
                HAND.extract(host, tx, r, n) + 1
            }
        }

        #[test]
        fn a_pouch_in_a_bag_in_a_bag_is_put_back_by_an_abort_at_any_depth() {
            game_thread();
            let in_outer = ItemPlace::Nested {
                parent: Box::new(HAND),
                slot: 0,
            };
            let in_bag = ItemPlace::Nested {
                parent: Box::new(in_outer.clone()),
                slot: 0,
            };
            for abort_depth in 0..3 {
                let initial = stack("test:outer", 1, 2);
                let mut player = Player {
                    main_hand: Some(initial.clone()),
                    ..Player::default()
                };
                let host: &mut dyn ServerHost = &mut player;
                let r = ItemResource::of(&stick());
                let result = host.transaction(|host, _| {
                    let middle = host.transaction(|host, _| {
                        let inner = host.transaction(|host, tx| {
                            let handler = in_bag.capability(host, &ITEM_ITEMS).unwrap();
                            assert_eq!(
                                handler.insert(host, tx, 0, &r, 3),
                                3,
                                "depth {abort_depth}"
                            );
                            if abort_depth == 2 { Err(()) } else { Ok(()) }
                        });
                        if abort_depth == 1 {
                            Err(())
                        } else {
                            inner.or(Ok(()))
                        }
                    });
                    if abort_depth == 0 {
                        Err(())
                    } else {
                        middle.or(Ok(()))
                    }
                });
                assert_eq!(result.is_err(), abort_depth == 0);
                assert_eq!(player.main_hand, Some(initial), "depth {abort_depth}");
                assert!(player.given.is_empty());
                // Committed, the outer bag is rewritten whole.
                let host: &mut dyn ServerHost = &mut player;
                assert_eq!(fill(host, &in_bag, stick(), 3), 3);
                assert_eq!(player.main_hand, Some(stack("test:outer", 1, 5)));
                assert!(player.given.is_empty());
            }
        }

        #[test]
        fn a_write_three_levels_down_in_one_of_two_stacked_bags_splits_that_one_off() {
            game_thread();
            let in_outer = ItemPlace::Nested {
                parent: Box::new(HAND),
                slot: 0,
            };
            let in_bag = ItemPlace::Nested {
                parent: Box::new(in_outer),
                slot: 0,
            };
            let mut player = Player {
                main_hand: Some(stack("test:outer", 2, 2)),
                ..Player::default()
            };
            assert_eq!(fill(&mut player, &in_bag, stick(), 3), 3);
            assert_eq!(player.main_hand, Some(stack("test:outer", 1, 2)));
            assert_eq!(player.given, [stack("test:outer", 1, 5)]);
        }

        #[test]
        fn an_index_less_move_goes_on_to_the_next_index() {
            game_thread();
            let mut s = shelf([
                Some(stack("minecraft:stick", 63, 0)),
                Some(stack("minecraft:stick", 63, 0)),
                None,
            ]);
            s.whole_extract = true;
            let host: &mut dyn ServerHost = &mut s;
            let h = here().capability(host, &ITEMS, SHELF, None).unwrap();
            let r = ItemResource::of(&stick());
            host.transaction(|host, tx| {
                assert_eq!(h.insert_any(host, tx, &r, 3), 3);
                assert_eq!(h.extract_any(host, tx, &r, 129), 129);
                Ok::<_, ()>(())
            })
            .unwrap();
            assert_eq!(s.slots, [None, None, None]);
        }

        #[test]
        fn a_mod_place_answering_more_than_it_was_asked_has_its_write_undone() {
            game_thread();
            let mut p = Player {
                main_hand: Some(stack("minecraft:stick", 2, 0)),
                ..Player::default()
            };
            let at = ItemPlace::Mod {
                provider: "test:lying-place",
                key: 0,
            };
            let host: &mut dyn ServerHost = &mut p;
            let r = ItemResource::of(&stick());
            host.transaction(|host, tx| {
                assert_eq!(at.insert(host, tx, &r, 1), 0);
                assert_eq!(at.extract(host, tx, &r, 1), 0);
                assert_eq!(HAND.stack(host), Some(stack("minecraft:stick", 2, 0)));
                Ok::<_, ()>(())
            })
            .unwrap();
            assert!(p.given.is_empty());
            assert!(super::super::super::item_place::warned_about("doubler"));
        }
    }

    #[test]
    fn ticket_controllers_get_the_callers_namespace_and_refuse_anothers() {
        let mut r = Registrar::new("first");
        r.register_ticket_controller("quarry", true);
        r.register_ticket_controller("first:drill", false);
        assert!(r.refused.is_empty(), "{:?}", r.refused);
        let ids: Vec<_> = r
            .controllers
            .iter()
            .map(|c| (&*c.id, c.validates))
            .collect();
        assert_eq!(ids, [("first:quarry", true), ("first:drill", false)]);
        // Another mod's namespace, and no name at all.
        r.register_ticket_controller("second:quarry", true);
        r.register_ticket_controller("first:", true);
        r.register_ticket_controller("", true);
        assert_eq!(r.refused.len(), 3, "{:?}", r.refused);
        assert!(r.refused[0].contains("first registered ticket controller \"second:quarry\""));
        assert!(r.refused[0].contains("not its own"));
        assert!(r.refused[1].contains("no name") && r.refused[2].contains("no name"));
        assert_eq!(r.controllers.len(), 2);
    }

    #[test]
    fn a_ticket_controller_registered_twice_refuses_the_mods_naming_mod_and_id() {
        let mut r = Registrar::new("first");
        r.register_ticket_controller("quarry", true);
        r.register_ticket_controller("first:quarry", false);
        r.mod_id = "second";
        // The same path under another mod is another id.
        r.register_ticket_controller("quarry", true);
        assert_eq!(r.refused.len(), 1, "{:?}", r.refused);
        assert_eq!(
            r.refused[0],
            "first registered ticket controller first:quarry twice"
        );
        assert_eq!(r.controllers.len(), 2);
        assert_eq!(&*r.controllers[1].id, "second:quarry");
    }

    #[test]
    fn a_place_provider_given_twice_is_refused() {
        let mut r = Registrar::new("first");
        r.place_provider("test:charms", Arc::new(Charms));
        r.mod_id = "second";
        r.mod_index = 1;
        r.place_provider("test:charms", Arc::new(Charms));
        r.place_provider("charms", Arc::new(Charms));
        r.block_entity_capability(&ITEMS, "pipe", |_, at| Some(Box::new(Pipe(at))));
        assert_eq!(r.refused.len(), 3, "{:?}", r.refused);
        assert!(
            r.refused[0]
                .contains("item places test:charms are provided twice, by first and second")
        );
        assert!(r.refused[1].contains("not a namespaced name"));
        assert!(r.refused[2].contains("not a namespaced id"));
    }

    /// A mod declaring its own key under a name another mod's providers answer with another
    /// handler type is refused at load, naming both mods and both types, as is a key whose name
    /// has no namespace; the same name with the same type, from another mod, is not.
    #[test]
    fn a_capability_name_answered_with_two_handler_types_is_refused() {
        const NOT_CHEMICALS: CapabilityKey<dyn ResourceHandler<FluidResource>> =
            CapabilityKey::new("test:chemical");
        const UNNAMESPACED: CapabilityKey<dyn ResourceHandler<Chemical>> =
            CapabilityKey::new("chemical");
        let mut r = Registrar::new("first");
        r.block_entity_capability(&CHEMICALS, "test:chem_tank", |_, at| {
            Some(Box::new(ChemTank(at)))
        });
        r.mod_id = "second";
        r.mod_index = 1;
        r.block_entity_capability(&NOT_CHEMICALS, "test:chem_vat", |_, at| {
            Some(Box::new(ModTank { at, lie: None }))
        });
        r.block_entity_capability(&CHEMICALS, "test:chem_vat", |_, at| {
            Some(Box::new(ChemTank(at)))
        });
        r.block_entity_capability(&UNNAMESPACED, "test:chem_vat", |_, at| {
            Some(Box::new(ChemTank(at)))
        });
        assert_eq!(r.refused.len(), 2, "{:?}", r.refused);
        let two_types = &r.refused[0];
        assert!(
            two_types.starts_with("capability test:chemical is answered with two handler types")
        );
        let chemicals = type_name::<dyn ResourceHandler<Chemical>>();
        let fluids = type_name::<dyn ResourceHandler<FluidResource>>();
        assert!(
            two_types.contains(&format!("{chemicals} by first")),
            "{two_types}"
        );
        assert!(
            two_types.contains(&format!("{fluids} by second")),
            "{two_types}"
        );
        assert!(r.refused[1].contains("capability \"chemical\", which is not a namespaced name"));
    }

    /// A handler that only tells which provider made it, by its slot count.
    struct Marked(usize);
    impl ResourceHandler<ItemResource> for Marked {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            self.0
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            None
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            0
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
            0
        }
        fn insert(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    /// `test:ordered` block entities, where the game has slots of its own from x = 8 on.
    struct Ordered;
    impl ServerHost for Ordered {}
    impl Backend for Ordered {
        fn level_in_place(&self, level: LevelRef) -> bool {
            level == here()
        }
        fn block_entity_kind(&self, _: BlockEntityRef) -> Option<&str> {
            Some("test:ordered")
        }
        fn container_size(&self, BlockSide { pos, .. }: BlockSide) -> Option<usize> {
            (pos[0] >= 8).then_some(27)
        }
    }
    impl Host for Ordered {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
        fn block_entity(&self, level: LevelRef, position: [i32; 3]) -> Option<BlockEntityRef> {
            Some(BlockEntityRef { level, position })
        }
    }

    #[test]
    fn providers_are_asked_ahead_of_the_game_then_the_game_then_the_mods_in_order() {
        game_thread();
        let host: &mut dyn ServerHost = &mut Ordered;
        let answer = |host: &mut dyn ServerHost, x| {
            let found = here().capability(host, &ITEMS, [x, 64, 0], None).unwrap();
            (found.owner.is_some(), found.size(host))
        };
        // No game slots: the first mod provider that answers; the one before it declined.
        assert_eq!(answer(host, 7), (true, 1));
        // The game's slots come before the mods' providers.
        assert_eq!(answer(host, 8), (false, 27));
        // Providers registered before the game come first where they answer, in registration
        // order: at x = 9 both answer, at x = 10 only the second.
        assert_eq!(answer(host, 9), (true, 3));
        assert_eq!(answer(host, 10), (true, 4));
        // The game's own lookup takes a mod's handler only where the game has none or a mod
        // comes first.
        let mod_only = |host: &mut dyn ServerHost, x| {
            let at = BlockSide {
                level: here(),
                pos: [x, 64, 0],
                side: None,
            };
            let game_slots = backend_of(&*host)
                .and_then(|backend| backend.container_size(at))
                .is_some();
            let providers = block_entity_providers(&ITEMS, host, at)?;
            providers
                .ask(host, game_slots)
                .map(|found| found.size(host))
        };
        assert_eq!(mod_only(host, 7), Some(1));
        assert_eq!(mod_only(host, 8), None);
        assert_eq!(mod_only(host, 9), Some(3));
        // With no provider for the block entity's type, or no type (a vanilla one), there are
        // no providers to ask, so the game needn't work out its own answer: a hopper's lookup
        // costs one table check.
        let at = BlockSide {
            level: here(),
            pos: [8, 64, 0],
            side: None,
        };
        assert!(block_entity_providers(&ITEMS, &TypeOf(None), at).is_none());
        assert!(block_entity_providers(&ITEMS, &TypeOf(Some("test:unregistered")), at).is_none());

        // Item providers in the same order, for the item in a player's hand: with no game slots
        // (damage 0), the first mod's that answers; with them (damage 1), the game's; for two
        // items, the one ahead of the game.
        let item = |damage, count| {
            let mut host = HeldItem(stack("test:ordered_item", count, damage));
            let host: &mut dyn ServerHost = &mut host;
            let found = HAND.capability(host, &ITEM_ITEMS).unwrap();
            (found.owner.is_some(), found.size(host))
        };
        assert_eq!(item(0, 1), (true, 1));
        assert_eq!(item(1, 1), (false, 27));
        assert_eq!(item(1, 2), (true, 3));
    }

    /// Block entities of one type, or vanilla ones (none).
    struct TypeOf(Option<&'static str>);
    impl Backend for TypeOf {
        fn block_entity_kind(&self, _: BlockEntityRef) -> Option<&str> {
            self.0
        }
    }
    impl Host for TypeOf {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
        fn block_entity(&self, level: LevelRef, position: [i32; 3]) -> Option<BlockEntityRef> {
            Some(BlockEntityRef { level, position })
        }
    }

    /// Player 1 holding one stack in their hand, which has game slots (`item_container`) when
    /// its damage is 1.
    struct HeldItem(ItemStack);
    impl Backend for HeldItem {
        fn item_container_slots(&self, stack: &ItemStack) -> Option<Vec<Option<ItemStack>>> {
            (stack.damage == 1).then(|| vec![None; 27])
        }
    }
    impl BackendMut for HeldItem {
        fn game_stack(&mut self, place: &ItemPlace) -> Option<ItemStack> {
            (*place == HAND).then(|| self.0.clone())
        }
    }
    impl Host for HeldItem {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
    }
    impl ServerHost for HeldItem {
        fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
            Some(BackendMutRef::of(self))
        }
    }

    /// How a mod tank misanswers: an extract takes and answers 100 mB more than asked
    /// (`GivesMore`), or an insert answers 100 mB more than went in (`TakesMore`).
    #[derive(Clone, Copy, PartialEq)]
    enum Lie {
        GivesMore,
        TakesMore,
    }
    /// A mod's tank: the game's tank at its position, reached from every side but its top, and
    /// misanswering as `lie` says.
    struct ModTank {
        at: BlockSide,
        lie: Option<Lie>,
    }
    impl ResourceHandler<FluidResource> for ModTank {
        fn size(&self, host: &mut dyn ServerHost) -> usize {
            if self.at.side == Some(Direction::Up) {
                return 0;
            }
            crate::mod_api::backend_of(host)
                .and_then(|backend| backend.fluid_tank_count(self.at))
                .unwrap_or(0)
        }
        fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<FluidResource> {
            Some(FluidResource::of(
                &crate::mod_api::backend_of(host)
                    .and_then(|backend| backend.fluid_tank(self.at, index))?,
            ))
        }
        fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
            crate::mod_api::backend_of(host)
                .and_then(|backend| backend.fluid_tank(self.at, index))
                .map_or(0, |held| held.amount)
        }
        fn capacity(
            &self,
            host: &mut dyn ServerHost,
            index: usize,
            resource: &FluidResource,
        ) -> u32 {
            crate::mod_api::backend_of(host).map_or(0, |backend| {
                backend.fluid_tank_capacity(self.at, index, &resource.to_stack(1))
            })
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            resource: &FluidResource,
            amount: u32,
        ) -> u32 {
            let Some(backend) = crate::mod_api::backend_mut_of(host) else {
                return 0;
            };
            let rest = backend.fluid_tank_insert(self.at, index, resource.to_stack(amount), tx);
            let went = amount - rest.map_or(0, |rest| rest.amount);
            match self.lie {
                Some(Lie::TakesMore) => went + 100,
                _ => went,
            }
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            resource: &FluidResource,
            amount: u32,
        ) -> u32 {
            let held = crate::mod_api::backend_of(host)
                .and_then(|backend| backend.fluid_tank(self.at, index));
            if !held.is_some_and(|held| resource.matches(&held)) {
                return 0;
            }
            let more = if self.lie == Some(Lie::GivesMore) {
                100
            } else {
                0
            };
            crate::mod_api::backend_mut_of(host)
                .and_then(|backend| backend.fluid_tank_extract(self.at, index, amount + more, tx))
                .map_or(0, |out| out.amount)
        }
    }

    /// One tank, as the game answers it: what it holds, its capacity, and whether it fills and
    /// drains. Truthfully: a `halving` tank takes half of each offer, a `half_room` one at most
    /// half its room, a `short` one gives one less than asked, and a `quarters` one gives and
    /// takes only whole quarter buckets, as a cauldron its thirds.
    #[derive(Clone)]
    struct GameTank {
        held: Option<FluidStack>,
        capacity: u32,
        fills: bool,
        drains: bool,
        halving: bool,
        half_room: bool,
        short: bool,
        quarters: bool,
    }
    /// Game tanks by position, and the mod block entity types standing at some.
    #[derive(Clone, Default)]
    struct TankWorld {
        tanks: HashMap<[i32; 3], GameTank>,
        types: HashMap<[i32; 3], &'static str>,
        kept: Vec<Self>,
    }
    const SOURCE: [i32; 3] = [0, 64, 0];
    const TANK: [i32; 3] = [1, 64, 0];
    const OTHER: [i32; 3] = [2, 64, 0];
    impl TankWorld {
        fn tank(&mut self, pos: [i32; 3], held: Option<FluidStack>, fills: bool) -> &mut Self {
            let tank = GameTank {
                held,
                capacity: 1000,
                fills,
                drains: true,
                halving: false,
                half_room: false,
                short: false,
                quarters: false,
            };
            self.tanks.insert(pos, tank);
            self
        }
        fn held(&self, pos: [i32; 3]) -> Option<FluidStack> {
            self.tanks[&pos].held.clone()
        }
        fn amount(&self, pos: [i32; 3]) -> u32 {
            self.tanks[&pos].held.as_ref().map_or(0, |held| held.amount)
        }
        /// Moves up to `max` from `from`'s handler through `from_side` into `to`'s through
        /// `to_side`, in a transaction of its own.
        fn moved(
            &mut self,
            (from, from_side): ([i32; 3], Option<Direction>),
            (to, to_side): ([i32; 3], Option<Direction>),
            max: u32,
        ) -> Result<u32, Overanswered> {
            let host: &mut dyn ServerHost = self;
            let source = here().capability(host, &FLUIDS, from, from_side).unwrap();
            let target = here().capability(host, &FLUIDS, to, to_side).unwrap();
            host.transaction(|host, tx| move_resources(host, &source, &target, |_| true, max, tx))
        }
    }
    impl Journal for TankWorld {
        fn kept(&mut self) -> &mut Vec<Self> {
            &mut self.kept
        }
    }
    impl Backend for TankWorld {
        fn level_in_place(&self, level: LevelRef) -> bool {
            level == here()
        }
        fn block_entity_kind(
            &self,
            BlockEntityRef { position: pos, .. }: BlockEntityRef,
        ) -> Option<&str> {
            self.types.get(&pos).copied()
        }
        fn fluid_tank_count(&self, BlockSide { pos, .. }: BlockSide) -> Option<usize> {
            self.tanks.contains_key(&pos).then_some(1)
        }
        fn fluid_tank(&self, BlockSide { pos, .. }: BlockSide, _: usize) -> Option<FluidStack> {
            self.tanks.get(&pos)?.held.clone()
        }
        fn fluid_tank_capacity(
            &self,
            BlockSide { pos, .. }: BlockSide,
            _: usize,
            _: &FluidStack,
        ) -> u32 {
            self.tanks
                .get(&pos)
                .filter(|tank| tank.fills)
                .map_or(0, |tank| tank.capacity)
        }
    }
    impl BackendMut for TankWorld {
        fn fluid_tank_insert(
            &mut self,
            BlockSide { pos, .. }: BlockSide,
            _: usize,
            fluid: FluidStack,
            _: &Transaction,
        ) -> Option<FluidStack> {
            let Some(tank) = self.tanks.get_mut(&pos).filter(|tank| tank.fills) else {
                return Some(fluid);
            };
            let held = tank.held.as_ref().map_or(0, |held| held.amount);
            if tank
                .held
                .as_ref()
                .is_some_and(|held| !held.same_fluid_same_components(&fluid))
            {
                return Some(fluid);
            }
            let offered = if tank.halving {
                fluid.amount / 2
            } else {
                fluid.amount
            };
            let room = if tank.half_room {
                (tank.capacity - held) / 2
            } else {
                tank.capacity - held
            };
            let mut add = offered.min(room);
            if tank.quarters {
                add -= add % 250;
            }
            if add > 0 {
                tank.held = Some(fluid.with_amount(held + add));
            }
            (add < fluid.amount).then(|| fluid.with_amount(fluid.amount - add))
        }
        fn fluid_tank_extract(
            &mut self,
            BlockSide { pos, .. }: BlockSide,
            _: usize,
            max: u32,
            _: &Transaction,
        ) -> Option<FluidStack> {
            let tank = self.tanks.get_mut(&pos).filter(|tank| tank.drains)?;
            let held = tank.held.clone()?;
            let asked = if tank.short {
                max.saturating_sub(1)
            } else {
                max
            };
            let mut take = held.amount.min(asked);
            if tank.quarters {
                take -= take % 250;
            }
            if take == 0 {
                return None;
            }
            tank.held = (take < held.amount).then(|| held.with_amount(held.amount - take));
            Some(held.with_amount(take))
        }
    }
    impl Host for TankWorld {
        fn block_entity(&self, level: LevelRef, position: [i32; 3]) -> Option<BlockEntityRef> {
            Some(BlockEntityRef { level, position })
        }
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
    }
    impl ServerHost for TankWorld {
        fn transaction_open(&mut self, _: TransactionSeal) {
            open(self);
        }
        fn transaction_close(&mut self, _: TransactionSeal, commit: bool) {
            close(self, commit);
        }
        fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
            Some(BackendMutRef::of(self))
        }
    }

    fn potion(amount: u32, potion: &str) -> FluidStack {
        let components = crate::item_data::StackData {
            potion: Some(Arc::from(potion)),
            ..Default::default()
        };
        FluidStack {
            components: Some(Box::new(components)),
            ..FluidStack::new("test:potion", amount)
        }
    }

    #[test]
    fn a_fluid_move_fills_only_with_the_same_fluid_and_components() {
        game_thread();
        let mut world = TankWorld::default();
        world.tank(SOURCE, Some(potion(1500, "minecraft:healing")), true);
        world.tank(TANK, None, true).types.insert(TANK, "test:tank");
        // The mod's tank takes nothing from above.
        let up = (TANK, Some(Direction::Up));
        let west = (TANK, Some(Direction::West));
        assert_eq!(world.moved((SOURCE, None), up, 4000), Ok(0));
        // What the tank has room for moves, and nothing more leaves the source.
        assert_eq!(world.moved((SOURCE, None), west, 4000), Ok(1000));
        assert_eq!(world.moved(west, (SOURCE, None), 400), Ok(400));
        assert_eq!(world.held(SOURCE), Some(potion(900, "minecraft:healing")));
        assert_eq!(world.held(TANK), Some(potion(600, "minecraft:healing")));
        // The same fluid with other components, or another fluid, doesn't go in.
        for other in [
            potion(500, "minecraft:poison"),
            FluidStack::new("test:potion", 500),
        ] {
            world.tank(SOURCE, Some(other.clone()), true);
            assert_eq!(world.moved((SOURCE, None), (TANK, None), 4000), Ok(0));
            assert_eq!(world.held(SOURCE), Some(other));
            assert_eq!(world.held(TANK), Some(potion(600, "minecraft:healing")));
        }
        // A drain-only tank has no capacity, and nothing moves.
        let water = FluidStack::new("minecraft:water", 1000);
        world.tank(SOURCE, Some(water.clone()), true);
        world.tank(TANK, None, false);
        let host: &mut dyn ServerHost = &mut world;
        let target = here().capability(host, &FLUIDS, TANK, None).unwrap();
        assert_eq!(target.capacity(host, 0, &FluidResource::of(&water)), 0);
        assert_eq!(world.moved((SOURCE, None), (TANK, None), 1000), Ok(0));
        assert_eq!(world.held(SOURCE), Some(water));
    }

    #[test]
    fn a_fluid_move_tries_again_with_what_the_target_took_and_loses_nothing() {
        game_thread();
        let water = |amount| FluidStack::new("minecraft:water", amount);
        type Setup = fn(&mut GameTank, &mut GameTank);
        let cases: [(&str, Setup, u32); 6] = [
            // Half of each offer: no request is ever taken whole, so nothing moves.
            ("half of each offer", |_, t| t.halving = true, 0),
            (
                "half of each offer, from quarters",
                |s, t| (s.quarters, t.halving) = (true, true),
                0,
            ),
            (
                "half of each offer, from a drain-only tank",
                |s, t| (s.fills, t.halving) = (false, true),
                0,
            ),
            // Half its room: moves of 500, 250, then 125, which quarters can't give.
            (
                "half its room, from quarters",
                |s, t| (s.quarters, t.half_room) = (true, true),
                750,
            ),
            (
                "half its room, from a drain-only tank",
                |s, t| (s.fills, t.half_room) = (false, true),
                999,
            ),
            // One less than asked: the 999 it gives all go in.
            ("a source giving one less", |s, _| s.short = true, 999),
        ];
        for (case, setup, moves) in cases {
            let mut world = TankWorld::default();
            world.tank(SOURCE, Some(water(1000)), true);
            world.tank(TANK, None, true);
            let [source, target] = world.tanks.get_disjoint_mut([&SOURCE, &TANK]);
            setup(source.unwrap(), target.unwrap());
            let moved = world.moved((SOURCE, None), (TANK, None), 1000);
            assert_eq!(moved, Ok(moves), "{case}");
            assert_eq!(
                (world.amount(SOURCE), world.amount(TANK)),
                (1000 - moves, moves),
                "{case}"
            );
        }
    }

    #[test]
    fn a_handler_answering_more_than_it_was_asked_aborts_the_whole_move_and_loses_no_fluid() {
        game_thread();
        let water = |amount| Some(FluidStack::new("minecraft:water", amount));
        for (liar, at) in [("test:gives_more", SOURCE), ("test:takes_more", TANK)] {
            let mut world = TankWorld::default();
            world.tank(SOURCE, water(1000), true);
            world.tank(OTHER, water(1000), true);
            world.tank(TANK, None, true).types.insert(at, liar);
            let host: &mut dyn ServerHost = &mut world;
            let ends = |host: &mut dyn ServerHost, pos| here().capability(host, &FLUIDS, pos, None);
            let (source, other, tank) = (
                ends(host, SOURCE).unwrap(),
                ends(host, OTHER).unwrap(),
                ends(host, TANK).unwrap(),
            );
            // Two moves into the tank in one transaction, the second from the source: the
            // first move through the liar fails, and its error aborts the transaction, so
            // every tank holds what it held.
            let moved = host.transaction(|host, tx| {
                let first = move_resources(host, &other, &tank, |_| true, 300, tx)?;
                let second = move_resources(host, &source, &tank, |_| true, 300, tx)?;
                Ok(first + second)
            });
            let owner = "doubler".to_owned();
            assert_eq!(moved, Err(Overanswered { owner }), "{liar}");
            let held = [SOURCE, OTHER, TANK].map(|pos| world.held(pos));
            assert_eq!(held, [water(1000), water(1000), None], "{liar}");
        }
    }

    /// A mod's battery: the game's at its position, misanswering by 100 as `lie` says.
    struct ModBattery {
        at: BlockSide,
        lie: Lie,
    }
    impl EnergyHandler for ModBattery {
        fn amount(&self, host: &mut dyn ServerHost) -> u32 {
            crate::mod_api::backend_of(host)
                .and_then(|backend| backend.energy_amount(self.at))
                .unwrap_or(0)
        }
        fn capacity(&self, host: &mut dyn ServerHost) -> u32 {
            crate::mod_api::backend_of(host).map_or(0, |backend| backend.energy_capacity(self.at))
        }
        fn insert(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
            let went = crate::mod_api::backend_mut_of(host)
                .map_or(0, |backend| backend.energy_insert(self.at, amount, tx));
            match self.lie {
                Lie::TakesMore => went + 100,
                Lie::GivesMore => went,
            }
        }
        fn extract(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
            let more = match self.lie {
                Lie::GivesMore => 100,
                Lie::TakesMore => 0,
            };
            crate::mod_api::backend_mut_of(host).map_or(0, |backend| {
                backend.energy_extract(self.at, amount + more, tx)
            })
        }
    }

    /// Batteries as the game answers them, by position: what each holds, and the most one
    /// insert moves, of a capacity of 1000; and the mod block entity types standing at some.
    #[derive(Clone, Default)]
    struct EnergyWorld {
        batteries: HashMap<[i32; 3], (u32, u32)>,
        types: HashMap<[i32; 3], &'static str>,
        kept: Vec<Self>,
    }
    impl EnergyWorld {
        fn amounts(&self, at: &[[i32; 3]]) -> Vec<u32> {
            at.iter().map(|pos| self.batteries[pos].0).collect()
        }
    }
    impl Journal for EnergyWorld {
        fn kept(&mut self) -> &mut Vec<Self> {
            &mut self.kept
        }
    }
    impl Host for EnergyWorld {
        fn block_entity(&self, level: LevelRef, position: [i32; 3]) -> Option<BlockEntityRef> {
            Some(BlockEntityRef { level, position })
        }
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
    }
    impl ServerHost for EnergyWorld {
        fn transaction_open(&mut self, _: TransactionSeal) {
            open(self);
        }
        fn transaction_close(&mut self, _: TransactionSeal, commit: bool) {
            close(self, commit);
        }
        fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
            Some(BackendMutRef::of(self))
        }
    }
    impl Backend for EnergyWorld {
        fn level_in_place(&self, level: LevelRef) -> bool {
            level == here()
        }
        fn block_entity_kind(
            &self,
            BlockEntityRef { position: pos, .. }: BlockEntityRef,
        ) -> Option<&str> {
            self.types.get(&pos).copied()
        }
        fn energy_amount(&self, BlockSide { pos, .. }: BlockSide) -> Option<u32> {
            Some(self.batteries.get(&pos)?.0)
        }
        fn energy_capacity(&self, _: BlockSide) -> u32 {
            1000
        }
    }
    impl BackendMut for EnergyWorld {
        fn energy_insert(&mut self, at: BlockSide, amount: u32, _: &Transaction) -> u32 {
            let Some((held, limit)) = self.batteries.get_mut(&at.pos) else {
                return 0;
            };
            let went = amount.min(*limit).min(1000 - *held);
            *held += went;
            went
        }
        fn energy_extract(&mut self, at: BlockSide, amount: u32, _: &Transaction) -> u32 {
            let Some((held, _)) = self.batteries.get_mut(&at.pos) else {
                return 0;
            };
            let came = amount.min(*held);
            *held -= came;
            came
        }
    }

    #[test]
    fn an_energy_move_takes_what_the_targets_room_and_limit_let_in_and_loses_none() {
        game_thread();
        let mut world = EnergyWorld::default();
        world.batteries.insert(SOURCE, (900, 1000));
        world.batteries.insert(TANK, (700, 1000));
        world.batteries.insert(OTHER, (0, 250));
        let mut moved = |from, to, max| {
            let host: &mut dyn ServerHost = &mut world;
            let source = here().capability(host, &ENERGY, from, None).unwrap();
            let target = here().capability(host, &ENERGY, to, None).unwrap();
            host.transaction(|host, tx| move_energy(host, &source, &target, max, tx))
        };
        // The target's room, then its limit per insert, each by a move tried again with what
        // it took; and no more than the source holds.
        assert_eq!(moved(SOURCE, TANK, 1000), Ok(300));
        assert_eq!(moved(SOURCE, OTHER, 1000), Ok(250));
        assert_eq!(moved(OTHER, SOURCE, 1000), Ok(250));
        assert_eq!(world.amounts(&[SOURCE, TANK, OTHER]), [600, 1000, 0]);
    }

    #[test]
    fn a_battery_answering_more_than_it_was_asked_aborts_the_whole_move_and_loses_no_energy() {
        game_thread();
        for (liar, at) in [
            ("test:battery_gives_more", SOURCE),
            ("test:battery_takes_more", TANK),
        ] {
            let mut world = EnergyWorld::default();
            world.batteries.insert(SOURCE, (500, 1000));
            world.batteries.insert(OTHER, (500, 1000));
            world.batteries.insert(TANK, (0, 1000));
            world.types.insert(at, liar);
            let host: &mut dyn ServerHost = &mut world;
            let ends = |host: &mut dyn ServerHost, pos| here().capability(host, &ENERGY, pos, None);
            let (source, other, tank) = (
                ends(host, SOURCE).unwrap(),
                ends(host, OTHER).unwrap(),
                ends(host, TANK).unwrap(),
            );
            // Two moves into the battery at `TANK` in one transaction, the second from the
            // source: a move through the liar fails, and its error aborts the transaction.
            let moved = host.transaction(|host, tx| {
                let first = move_energy(host, &other, &tank, 200, tx)?;
                let second = move_energy(host, &source, &tank, 200, tx)?;
                Ok(first + second)
            });
            let owner = "doubler".to_owned();
            assert_eq!(moved, Err(Overanswered { owner }), "{liar}");
            assert_eq!(
                world.amounts(&[SOURCE, OTHER, TANK]),
                [500, 500, 0],
                "{liar}"
            );
        }
    }

    #[test]
    fn a_mods_tag_part_registered_first_runs_before_its_key_part() {
        let mut r = Registrar::new("registrar-test");
        r.block_part_tag("test:gauged", TagGauge);
        r.block_part("test:gauge", Gauge);
        let tags = HashMap::from([(
            "test:gauged".to_owned(),
            HashSet::from(["test:gauge".to_owned()]),
        )]);
        super::add_tagged(&mut r.parts.blocks, &r.tags.blocks, &tags, "block");
        let order: Vec<&str> = r.parts.blocks["test:gauge"]
            .iter()
            .map(|a| a.type_name)
            .collect();
        assert_eq!(order, [type_name::<TagGauge>(), type_name::<Gauge>()]);
    }

    #[test]
    fn a_block_entity_part_answers_by_block_and_by_entity_type() {
        game_thread();
        let ticks = |block| {
            let mut ticks = false;
            dispatch_block_entity_ticks(&NoHost, block, &mut ticks);
            ticks
        };
        // The ticker is asked by block: the type id is not a block.
        assert!(ticks("test:press"));
        assert!(!ticks("test:press_entity"));
        let tick = |kind: &str| {
            let mut entity = ModBlockEntity::new(kind.into(), Default::default());
            dispatch_block_entity_tick(&mut NoHost, [0, 64, 0], &mut entity);
            entity.data.len()
        };
        // The tick is asked by the entity's type, which differs from its block's id.
        assert_eq!(tick("test:press_entity"), 1);
        assert_eq!(tick("test:press"), 0);
    }

    #[test]
    fn with_no_parts_the_block_entity_is_never_named() {
        let parts = Parts::default();
        let named = std::cell::Cell::new(false);
        let id = || {
            named.set(true);
            Some("test:press")
        };
        assert!(parts.block_entity(id).is_empty());
        assert!(!named.get(), "an empty table resolved the block's name");
    }

    #[test]
    fn the_same_part_twice_on_one_block_is_refused() {
        struct Other;
        impl BlockPart for Other {}
        let mut r = Registrar::new("first");
        r.block_part("test:gauge", Gauge);
        r.mod_id = "second";
        r.mod_index = 1;
        r.block_part("test:gauge", Other);
        r.block_part("test:gauge", Gauge);
        r.block_part("gauge", Other);
        r.block_part_tag("#test:gauged", Other);
        assert_eq!(r.parts.blocks["test:gauge"].len(), 2);
        assert_eq!(r.refused.len(), 3, "{:?}", r.refused);
        assert!(
            r.refused[0].contains("Gauge is attached to test:gauge twice, by first and second")
        );
        assert!(r.refused[1].contains("not a namespaced id"));
        assert!(r.refused[2].contains("write it without #"));
    }
}
