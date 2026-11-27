//! Connection/menu-instance scoped custom recipe placement, distinct from vanilla IDs.
#[path = "placement_catalogue.rs"]
mod catalogue;
#[cfg(test)]
#[path = "placement_sync_tests.rs"]
mod tests;

/// The Identifier stream codec uses STRING_UTF8 (32,767 UTF-16 code units).
/// Valid namespaced identifiers are ASCII, so this is their byte limit on the wire only.
pub(super) const RESOURCE_ID_BYTES: usize = 32767;
/// Budget for a Part in one play packet, including packet ID (a one-byte VarInt),
/// channel length (a one-byte VarInt), channel, settings tag, Wire variant tag,
/// epoch, request number, offset and byte-vector length.
pub const CHUNK_BYTES: usize = sunrise_protocol::frame::MAX_GUARANTEED_PACKET_BODY
    - (1 + 1 + super::settings_sync::CHANNEL.len() + 1 + 4 + 8 + 8 + 4 + 4);

/// A validated fully qualified identifier shared by book settings and placement.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, mod_api::Wire)]
#[wire(via = String)]
pub struct ResourceId(String);
impl ResourceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(super) fn fits_wire(&self) -> bool {
        self.0.len() <= RESOURCE_ID_BYTES
    }

    /// Takes a key obtained from or already checked against the frozen book registry.
    pub fn from_registered_book(book: &mod_api::RecipeBookTypeKey) -> Self {
        Self(book.as_str().to_owned())
    }
}
impl std::borrow::Borrow<str> for ResourceId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}
impl TryFrom<String> for ResourceId {
    type Error = ();
    fn try_from(value: String) -> Result<Self, ()> {
        if mod_api::Identifier::parse_namespaced(&value).is_some() {
            Ok(Self(value))
        } else {
            Err(())
        }
    }
}
impl From<&ResourceId> for String {
    fn from(value: &ResourceId) -> Self {
        value.0.clone()
    }
}

/// Original source index inside a server-authored recipe entry, never a vanilla ordinal.
#[derive(Clone, Debug, PartialEq, Eq, mod_api::Wire)]
pub struct DisplayId {
    pub recipe: ResourceId,
    pub source: u32,
}

/// The server grants this identity only for a live, negotiated menu instance.
#[derive(Clone, Debug, PartialEq, Eq, mod_api::Wire)]
pub struct CatalogIdentity {
    pub epoch: u64,
    pub number: u64,
    pub window: i32,
    pub nonce: u64,
    pub instance: u64,
    pub generation: u64,
    pub menu: ResourceId,
    pub book: ResourceId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, mod_api::Wire)]
pub enum Refusal {
    Unsupported,
    Stale,
    Unavailable,
    UnknownDisplay,
    NotPlaceable,
    Denied,
    NoRoom,
    Changed,
    TooLarge,
    Invalid,
}

#[derive(Clone, Debug, PartialEq, mod_api::Wire)]
pub enum Frame {
    Hello {
        epoch: u64,
        schema: u64,
    },
    Ready {
        epoch: u64,
        schema: u64,
    },
    Catalog {
        epoch: u64,
        number: u64,
        window: i32,
        nonce: u64,
        menu: ResourceId,
        book: ResourceId,
    },
    Start {
        identity: CatalogIdentity,
        layout: mod_api::RecipeBookLayout,
        storage: super::menu_layout::MenuLayoutMetadata,
        total: u32,
    },
    Part {
        epoch: u64,
        number: u64,
        offset: u32,
        bytes: Vec<u8>,
    },
    Place {
        catalog: CatalogIdentity,
        display: DisplayId,
        use_max: bool,
    },
    Ghost {
        catalog: CatalogIdentity,
        recipe: ResourceId,
        display: mod_api::RecipeBookDisplay,
    },
    Refused {
        epoch: u64,
        number: u64,
        window: i32,
        nonce: u64,
        reason: Refusal,
    },
    Invalidate {
        epoch: u64,
    },
}

/// Bump when the placement frame or catalogue protocol changes. Part size is
/// also included independently in `schema()`.
const PROTOCOL: u64 = 4;

/// Borrowed category view with concretized displays, avoiding deep copies of
/// `PlacementInfo` trees before wire depth and size limits are enforced.
pub struct BorrowedCategory<'a> {
    pub category: &'a mod_api::RecipeBookCategory,
    pub displays: Vec<BorrowedDisplayEntry<'a>>,
}

/// One borrowed entry inside a [`BorrowedCategory`].
pub struct BorrowedDisplayEntry<'a> {
    pub recipe: &'a str,
    pub displays: Vec<mod_api::RecipeBookDisplay>,
    pub placement: Option<&'a mod_api::PlacementInfo>,
}

/// Encode the remote display projection directly from borrowed declarations.
pub fn encode_borrowed_catalogue(
    declarations: &[BorrowedCategory<'_>],
) -> Result<Vec<u8>, Refusal> {
    let mut writer = mod_api::wire::Writer::new();
    // Start.total and Part.offset are u32; individual Parts fit the connection frame.
    writer.limit(u32::MAX as usize);
    catalogue::write_borrowed_categories(&mut writer, declarations);
    if let Some(error) = writer.error() {
        log::warn!("[recipe book] cannot encode placement catalogue: {error}");
        return Err(Refusal::TooLarge);
    }
    Ok(writer.into_bytes())
}

/// Encode the remote display projection of already validated native declarations.
pub fn encode_catalogue(declarations: &[mod_api::RecipeBookCategory]) -> Result<Vec<u8>, Refusal> {
    let mut writer = mod_api::wire::Writer::new();
    // Start.total and Part.offset are u32; individual Parts fit the connection frame.
    writer.limit(u32::MAX as usize);
    catalogue::write_categories(&mut writer, declarations);
    if let Some(error) = writer.error() {
        log::warn!("[recipe book] cannot encode placement catalogue: {error}");
        return Err(Refusal::TooLarge);
    }
    Ok(writer.into_bytes())
}

/// Decode display data. The caller validates it before publishing a book index.
pub fn decode_catalogue(bytes: &[u8]) -> Result<Vec<mod_api::RecipeBookCategory>, Refusal> {
    let categories: Vec<catalogue::Category> =
        mod_api::wire::Reader::decode(bytes).map_err(|_| Refusal::Invalid)?;
    Ok(categories.into_iter().map(Into::into).collect())
}

/// What the client and the server must agree on before either sends a frame.
pub fn schema() -> u64 {
    [PROTOCOL, CHUNK_BYTES as u64, RESOURCE_ID_BYTES as u64]
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, word| {
            (hash ^ word).wrapping_mul(0x0100_0000_01b3)
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub window: i32,
    pub nonce: u64,
    pub menu: ResourceId,
    pub book: ResourceId,
}
struct PendingCatalog {
    number: u64,
    window: Window,
    incoming: Option<Incoming>,
}
struct Incoming {
    identity: CatalogIdentity,
    layout: mod_api::RecipeBookLayout,
    total: usize,
    bytes: Vec<u8>,
}
pub struct CatalogBytes {
    pub identity: CatalogIdentity,
    pub layout: mod_api::RecipeBookLayout,
    pub bytes: Vec<u8>,
}
#[derive(Default)]
pub struct Client {
    epoch: Option<u64>,
    ready: bool,
    number: u64,
    window: Option<Window>,
    pending: Option<PendingCatalog>,
    catalog: Option<CatalogIdentity>,
    completed: Option<CatalogIdentity>,
}
impl Client {
    /// Reset connection authority before negotiating a different epoch.
    pub fn hello(&mut self, epoch: u64) -> Frame {
        if self.epoch != Some(epoch) {
            *self = Self {
                epoch: Some(epoch),
                ..Self::default()
            };
        }
        Frame::Hello {
            epoch,
            schema: schema(),
        }
    }
    pub fn ready(&mut self, epoch: u64, received: u64) -> bool {
        if self.epoch != Some(epoch) || received != schema() {
            return false;
        }
        self.ready = true;
        true
    }
    pub fn close(&mut self) {
        self.window = None;
        self.pending = None;
        self.catalog = None;
        self.completed = None;
    }
    pub fn invalidate(&mut self, epoch: u64) -> bool {
        if self.epoch != Some(epoch) {
            return false;
        }
        self.pending = None;
        self.catalog = None;
        self.completed = None;
        true
    }
    /// A catalogue refusal must name the outstanding request, not a superseded window.
    pub fn refused(&mut self, epoch: u64, number: u64, window: i32, nonce: u64) -> bool {
        if self.epoch != Some(epoch) {
            return false;
        }
        if number == 0 {
            if self.ready {
                return false;
            }
            self.close();
            return true;
        }
        if !self.pending.as_ref().is_some_and(|pending| {
            pending.number == number
                && pending.window.window == window
                && pending.window.nonce == nonce
        }) {
            return false;
        }
        self.invalidate(epoch)
    }
    pub fn open(&mut self, window: Window) -> Option<Frame> {
        self.close();
        if !self.ready {
            return None;
        }
        let epoch = self.epoch?;
        let number = self.number.checked_add(1)?;
        self.number = number;
        self.window = Some(window.clone());
        self.pending = Some(PendingCatalog {
            number,
            window: window.clone(),
            incoming: None,
        });
        Some(Frame::Catalog {
            epoch,
            number,
            window: window.window,
            nonce: window.nonce,
            menu: window.menu,
            book: window.book,
        })
    }
    pub fn start(
        &mut self,
        identity: CatalogIdentity,
        layout: mod_api::RecipeBookLayout,
        total: u32,
    ) -> Result<(), Refusal> {
        let pending = self.pending.as_mut().ok_or(Refusal::Stale)?;
        let total = usize::try_from(total).map_err(|_| Refusal::TooLarge)?;
        if self.epoch != Some(identity.epoch)
            || identity.number != pending.number
            || identity.window != pending.window.window
            || identity.nonce != pending.window.nonce
            || identity.menu != pending.window.menu
            || identity.book != pending.window.book
            || identity.instance == 0
            || identity.generation == 0
        {
            return Err(Refusal::Stale);
        }
        if total == 0 {
            return Err(Refusal::Invalid);
        }
        if pending.incoming.is_some() {
            return Err(Refusal::Invalid);
        }
        pending.incoming = Some(Incoming {
            identity,
            layout,
            total,
            bytes: Vec::new(),
        });
        Ok(())
    }
    pub fn part(
        &mut self,
        epoch: u64,
        number: u64,
        offset: u32,
        bytes: Vec<u8>,
    ) -> Result<Option<CatalogBytes>, Refusal> {
        let pending = self.pending.as_mut().ok_or(Refusal::Stale)?;
        if self.epoch != Some(epoch) || pending.number != number {
            return Err(Refusal::Stale);
        }
        let incoming = pending.incoming.as_mut().ok_or(Refusal::Invalid)?;
        let offset = usize::try_from(offset).map_err(|_| Refusal::Invalid)?;
        if offset != incoming.bytes.len()
            || bytes.is_empty()
            || bytes.len() > incoming.total - incoming.bytes.len()
        {
            return Err(Refusal::Invalid);
        }
        incoming.bytes.extend(bytes);
        if incoming.bytes.len() != incoming.total {
            return Ok(None);
        }
        let incoming = pending.incoming.take().ok_or(Refusal::Invalid)?;
        self.pending = None;
        self.completed = Some(incoming.identity.clone());
        // The caller validates the complete body before accept_catalog publishes authority.
        Ok(Some(CatalogBytes {
            identity: incoming.identity,
            layout: incoming.layout,
            bytes: incoming.bytes,
        }))
    }
    pub fn accept_catalog(&mut self, identity: CatalogIdentity) -> bool {
        let Some(window) = &self.window else {
            return false;
        };
        if self.epoch != Some(identity.epoch)
            || window.window != identity.window
            || window.nonce != identity.nonce
            || window.menu != identity.menu
            || window.book != identity.book
            || self.pending.is_some()
            || self.completed.as_ref() != Some(&identity)
        {
            return false;
        }
        self.completed = None;
        self.catalog = Some(identity);
        true
    }
    pub fn place(&self, display: DisplayId, use_max: bool) -> Option<Frame> {
        let catalog = self.catalog.clone()?;
        Some(Frame::Place {
            catalog,
            display,
            use_max,
        })
    }
    /// Only a server ghost for the currently accepted catalogue may populate this opening.
    pub fn accepts_ghost(&self, catalog: &CatalogIdentity) -> bool {
        self.catalog.as_ref() == Some(catalog)
    }
    pub fn epoch(&self) -> Option<u64> {
        self.epoch
    }
    pub fn window(&self) -> Option<&Window> {
        self.window.as_ref()
    }
    pub fn catalog(&self) -> Option<&CatalogIdentity> {
        self.catalog.as_ref()
    }
}

/// Catalogue-transfer sequencing is connection-scoped, not reset by a repeated Hello or menu close.
pub struct Authority {
    epoch: u64,
    ready: bool,
    last: u64,
    catalog: Option<CatalogIdentity>,
}
impl Authority {
    pub fn new(epoch: u64) -> Self {
        Self {
            epoch,
            ready: false,
            last: 0,
            catalog: None,
        }
    }
    pub fn hello(&mut self, epoch: u64, offered: u64) -> Result<Frame, Refusal> {
        if epoch != self.epoch {
            return Err(Refusal::Stale);
        }
        if offered != schema() {
            self.ready = false;
            self.catalog = None;
            return Err(Refusal::Unsupported);
        }
        self.ready = true;
        Ok(Frame::Ready {
            epoch,
            schema: schema(),
        })
    }
    pub fn publish(&mut self, identity: CatalogIdentity) -> Result<(), Refusal> {
        if !self.ready || identity.epoch != self.epoch || identity.number <= self.last {
            return Err(Refusal::Stale);
        }
        self.last = identity.number;
        self.catalog = Some(identity);
        Ok(())
    }
    pub fn owns(&self, catalog: &CatalogIdentity) -> bool {
        self.ready && self.catalog.as_ref() == Some(catalog)
    }
    pub fn place(&self, catalog: &CatalogIdentity) -> Result<(), Refusal> {
        if !self.owns(catalog) {
            return Err(Refusal::Stale);
        }
        Ok(())
    }
    pub fn clear(&mut self) {
        self.catalog = None;
    }
}
