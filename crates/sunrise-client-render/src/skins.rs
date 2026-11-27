//! Player skins from profile `textures` properties: authlib's
//! `MinecraftServicesSessionService.unpackTextures`, vanilla's
//! `SkinManager` lookup (which texture a player shows, and when the default
//! one) and `SkinTextureDownloader.processLegacySkin`.

pub use sunrise_level::profile_lookup::{
    NameOrId, ProfileInfo, ProfileResolver, Resolve, ResolvedProfile, Signature, TEXTURE_URIS,
    Textures, base64_decode, cape_location, discovery_texture_uris, dynamic_nbt, location,
    nbt_text, parse_profile, profile_texture_nbt, profile_uuid_nbt, property_signature, sha1,
    skin_location, texture_hash, unpack, valid_player_name,
};
#[cfg(test)]
use sunrise_server_profiles_http::ProfileAnswer;
pub use sunrise_server_profiles_http::fetch_own_properties;
use sunrise_server_profiles_http::http_get;

/// `MinecraftServicesDiscoveryService`'s valid texture URIs: the discovery
/// document's `profiles` / `getTexture` `validUris`, which
/// `MinecraftServicesSessionService.isAllowedTextureDomain` checks texture URLs
/// against. Production's document lists [`TEXTURE_URIS`], so they are used
/// without asking it; another environment (`SUNRISE_API_DISCOVERY`) is asked
/// once per game, and keeps [`TEXTURE_URIS`] when its document lists none or
/// can't be fetched.
pub fn texture_uris() -> &'static [String] {
    static URIS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    URIS.get_or_init(|| {
        crate::auth::discovery_overridden()
            .then(|| crate::auth::discovery().ok())
            .flatten()
            .and_then(|doc| discovery_texture_uris(&doc))
            .unwrap_or_else(|| TEXTURE_URIS.map(str::to_owned).to_vec())
    })
}

/// A profile's `textures` property as its player's skin is asked for it: the key of
/// `SkinManager`'s skin cache (its `CacheKey`: the profile and the property), so one answer
/// serves every list entry and entity that carries the property.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackedTextures {
    pub profile: u128,
    pub value: String,
    pub signature: Option<String>,
}

impl PackedTextures {
    /// `MinecraftServicesSessionService.unpackTextures`: the property's textures, with the
    /// signature state the `profile_property_keys` give them.
    fn unpack(&self, profile_property_keys: &[sunrise_protocol::rsa::RsaPublicKey]) -> Textures {
        unpack(
            &self.value,
            property_signature(
                &self.value,
                self.signature.as_deref(),
                profile_property_keys,
            ),
            texture_uris(),
        )
    }
}

/// Unpack a profile for an insecure-only consumer (heads, mannequins and
/// previews). The caller is not making a trust decision about the property.
pub fn unpack_insecure(value: &str) -> Textures {
    unpack(value, Signature::Unsigned, texture_uris())
}

/// The skin URL of a `minecraft:profile` component's property list.
pub fn profile_skin_url(profile: &serde_json::Value) -> Option<String> {
    unpack_insecure(&parse_profile(profile).textures?).skin
}

/// The textures of a Minecraft services `/minecraft/profile` answer: the
/// `ACTIVE` skin and cape (`MinecraftProfileResponse`). Unsigned, like the
/// local player's own textures, and only URLs under the allowed domains count.
pub fn services_profile_textures(profile: &serde_json::Value) -> Textures {
    let active = |key: &str| {
        profile
            .get(key)?
            .as_array()?
            .iter()
            .find(|t| t.get("state").and_then(|s| s.as_str()) == Some("ACTIVE"))
    };
    let skin_entry = active("skins");
    let skin = skin_entry
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .filter(|url| {
            texture_uris()
                .iter()
                .any(|u| url.starts_with(&u.replace("{textureId}", "")))
        })
        .map(str::to_owned);
    let slim = skin_entry
        .and_then(|t| t.get("variant"))
        .and_then(|v| v.as_str())
        .is_some_and(|v| v.eq_ignore_ascii_case("slim"));
    let cape = active("capes")
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .filter(|url| {
            texture_uris()
                .iter()
                .any(|u| url.starts_with(&u.replace("{textureId}", "")))
        })
        .map(str::to_owned);
    Textures {
        skin,
        slim,
        cape,
        elytra: None,
        signature: Signature::Unsigned,
    }
}

/// `profile_skin_url` on a block entity's `profile` NBT (`SkullBlockEntity`), checked against the
/// live `texture_uris()` list.
pub fn profile_skin_url_nbt(profile: &sunrise_core::nbt::Tag) -> Option<String> {
    sunrise_level::profile_lookup::profile_skin_url_nbt(profile, texture_uris())
}

/// `ProfileResolver.Cached` with client head-profile caches (`HeadProfiles`).
pub struct Resolver {
    cache: sunrise_server_profiles_http::ProfileCache,
    heads: HeadProfiles,
}
impl Default for Resolver {
    fn default() -> Self {
        Self::new()
    }
}
impl Resolver {
    pub fn new() -> Self {
        Self {
            cache: sunrise_server_profiles_http::ProfileCache::new(),
            heads: HeadProfiles::default(),
        }
    }
    /// A resolver whose lookups `lookup` starts instead of the services.
    #[cfg(test)]
    pub(crate) fn with_lookup(
        lookup: impl FnMut(NameOrId, ProfileAnswer) + Send + 'static,
    ) -> Self {
        Self {
            cache: sunrise_server_profiles_http::ProfileCache::with_lookup(lookup),
            heads: HeadProfiles::default(),
        }
    }
    /// How the resolution of `key` stands, starting it the first time.
    pub fn state(&mut self, key: &NameOrId) -> Resolve {
        self.cache.state(key)
    }
    pub fn peek(&mut self, key: &NameOrId) -> Resolve {
        self.cache.peek(key)
    }
}

const MAX_HEAD_PROFILES: usize = 4096;

#[derive(Debug, PartialEq)]
struct HeadProfile {
    info: ProfileInfo,
    skin: Option<(String, String)>,
}

fn prepare_head(profile: &serde_json::Value) -> HeadProfile {
    let info = parse_profile(profile);
    let skin = info.textures.as_deref().and_then(|value| {
        let url = unpack_insecure(value).skin?;
        let location = skin_location(&texture_hash(&url)?);
        Some((url, location))
    });
    HeadProfile { info, skin }
}

#[derive(Default)]
struct HeadProfiles {
    entries: std::collections::HashMap<serde_json::Value, std::sync::Arc<HeadProfile>>,
    nbt: std::collections::HashMap<sunrise_core::nbt::Tag, std::sync::Arc<serde_json::Value>>,
    resolved: std::collections::HashMap<String, std::sync::Arc<HeadProfile>>,
}

impl HeadProfiles {
    fn get(&mut self, profile: &serde_json::Value) -> std::sync::Arc<HeadProfile> {
        if let Some(parsed) = self.entries.get(profile) {
            return parsed.clone();
        }
        if self.entries.len() >= MAX_HEAD_PROFILES {
            self.entries.clear();
        }
        let parsed = std::sync::Arc::new(prepare_head(profile));
        self.entries.insert(profile.clone(), parsed.clone());
        parsed
    }
}

impl Resolver {
    /// Cache by NBT value rather than address: equipment can be replaced or
    /// mutated in place, and neither must leave a stale profile behind.
    pub fn head_profile_nbt(
        &mut self,
        profile: &sunrise_core::nbt::Tag,
    ) -> std::sync::Arc<serde_json::Value> {
        if let Some(json) = self.heads.nbt.get(profile) {
            return json.clone();
        }
        if self.heads.nbt.len() >= MAX_HEAD_PROFILES {
            self.heads.nbt.clear();
        }
        let json = std::sync::Arc::new(crate::scoreboard::tag_to_json(profile));
        self.heads.nbt.insert(profile.clone(), json.clone());
        json
    }
}

enum SkinEffect {
    Resolve(NameOrId),
    Download(String),
    Resource {
        location: String,
        namespace: String,
        path: String,
    },
    Again(String),
    Drawn(String),
}

/// Pending head-skin effects of a GUI draw group. Extraction uses the current skin or fallback;
/// new requests and atlas uses are replayed in order only if the outer group commits.
#[derive(Default)]
pub struct HeadSkinJournal(std::cell::RefCell<Vec<SkinEffect>>);

impl HeadSkinJournal {
    /// On refusal or unwind, discard just the effects added by this scope.
    pub fn scope<T, E>(&self, draw: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        struct Rollback<'a> {
            journal: &'a HeadSkinJournal,
            mark: usize,
            committed: bool,
        }
        impl Drop for Rollback<'_> {
            fn drop(&mut self) {
                if !self.committed {
                    self.journal.0.borrow_mut().truncate(self.mark);
                }
            }
        }
        let mut pending = Rollback {
            journal: self,
            mark: self.0.borrow().len(),
            committed: false,
        };
        let value = draw()?;
        pending.committed = true;
        Ok(value)
    }

    /// Whether any extraction requested a change to the skin caches.
    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    /// Record that `location` was drawn from the atlas.
    pub fn record_drawn(&self, location: &str) {
        self.0
            .borrow_mut()
            .push(SkinEffect::Drawn(location.to_owned()));
    }

    /// Record that `location` was missing from the atlas and should be queued again.
    pub fn record_again(&self, location: &str) {
        self.0
            .borrow_mut()
            .push(SkinEffect::Again(location.to_owned()));
    }

    /// Apply accepted requests and atlas uses without extracting the items again.
    pub fn commit(self, resolver: &mut Resolver, downloads: &mut Downloader) {
        for effect in self.0.into_inner() {
            match effect {
                SkinEffect::Resolve(key) => {
                    resolver.state(&key);
                }
                SkinEffect::Download(url) => {
                    downloads.state(Kind::Skin, &url);
                }
                SkinEffect::Resource {
                    location,
                    namespace,
                    path,
                } => downloads.resource(&location, &namespace, &path),
                SkinEffect::Again(location) => {
                    downloads.again(&location);
                }
                SkinEffect::Drawn(location) => downloads.drawn(&location),
            }
        }
    }
}

/// `PlayerSkinRenderCache.getOrDefault` for a head: the resolved skin once
/// its texture is in the atlas (`atlas`), else `partialProfile`'s default.
pub fn head_skin(
    profile: &serde_json::Value,
    resolver: &mut Resolver,
    downloads: &mut Downloader,
    atlas: impl Fn(&str) -> bool,
) -> String {
    head_skin_with_journal(profile, resolver, downloads, atlas, None)
}

/// Select the same skin as [`head_skin`], optionally postponing this draw's effects.
pub fn head_skin_with_journal(
    profile: &serde_json::Value,
    resolver: &mut Resolver,
    downloads: &mut Downloader,
    atlas: impl Fn(&str) -> bool,
    journal: Option<&HeadSkinJournal>,
) -> String {
    if let Some(texture) = profile.get("texture").and_then(|t| t.as_str())
        && let Some(sprite) = patch_skin_with_journal(texture, downloads, &atlas, journal)
    {
        return sprite;
    }
    let p = resolver.heads.get(profile);
    let resolved = p
        .info
        .dynamic
        .as_ref()
        .filter(|_| p.info.textures.is_none())
        .and_then(|key| resolved_head_with_journal(key, resolver, journal));
    match downloaded_skin_with_journal(
        resolved.as_deref().unwrap_or(&p),
        downloads,
        &atlas,
        journal,
    ) {
        Some(location) => location.to_owned(),
        None => crate::tab_list::default_skin(p.info.id).to_owned(),
    }
}

/// The skin of the profile `key` resolves to, by its `textures` property (each value parsed
/// once), starting or journaling the lookup the first time. Dynamic resolution is deliberately
/// not cached: pending answers and the resolver's expiring service results must still advance
/// on every draw.
fn resolved_head_with_journal(
    key: &NameOrId,
    resolver: &mut Resolver,
    journal: Option<&HeadSkinJournal>,
) -> Option<std::sync::Arc<HeadProfile>> {
    let resolution = if let Some(journal) = journal {
        journal
            .0
            .borrow_mut()
            .push(SkinEffect::Resolve(key.clone()));
        resolver.peek(key)
    } else {
        resolver.state(key)
    };
    let Resolve::Done(Some(profile)) = resolution else {
        return None;
    };
    let value = profile.textures()?;
    let cache = &mut resolver.heads.resolved;
    if let Some(parsed) = cache.get(value) {
        return Some(parsed.clone());
    }
    if cache.len() >= MAX_HEAD_PROFILES {
        cache.clear();
    }
    let property = serde_json::json!({"name": "textures", "value": value});
    let parsed = std::sync::Arc::new(prepare_head(&serde_json::json!({"properties": [property]})));
    cache.insert(value.to_owned(), parsed.clone());
    Some(parsed)
}

/// `head`'s skin once downloaded and in the atlas (`atlas`), marked or journaled as drawn; an
/// evicted one is offered or journaled for upload again.
fn downloaded_skin_with_journal<'h>(
    head: &'h HeadProfile,
    downloads: &mut Downloader,
    atlas: &impl Fn(&str) -> bool,
    journal: Option<&HeadSkinJournal>,
) -> Option<&'h str> {
    let (url, location) = head.skin.as_ref()?;
    let state = if let Some(journal) = journal {
        downloads.receive_completed();
        journal
            .0
            .borrow_mut()
            .push(SkinEffect::Download(url.clone()));
        texture_hash(url)
            .and_then(|hash| downloads.states.get(&(Kind::Skin, hash)).copied())
            .unwrap_or(Download::Pending)
    } else {
        downloads.state(Kind::Skin, url)
    };
    if state != Download::Ready {
        return None;
    }
    let present = atlas(location);
    if let Some(journal) = journal {
        let effect = if present {
            SkinEffect::Drawn(location.clone())
        } else {
            SkinEffect::Again(location.clone())
        };
        journal.0.borrow_mut().push(effect);
    } else if present {
        downloads.drawn(location);
    } else {
        // Its slot was evicted: offer the image again.
        downloads.again(location);
    }
    present.then_some(location.as_str())
}

/// The skin a GUI face of the player `uuid` draws (`GuiCanvas::player_face`): `listed`, the drawn
/// skin of the connection's `PlayerInfo` for `uuid` when there is one; else the profile
/// `NameOrId::Id(uuid)` resolves to, as a head's `Dynamic` profile resolves ([`head_skin`]),
/// started once and never waited on, with the UUID's default skin until its texture is in the
/// atlas (`atlas`). A resolved skin's name is copied into `name`, a buffer reused from face to
/// face, so a face allocates nothing once it has room.
pub fn face_skin<'a>(
    uuid: u128,
    listed: Option<&'a str>,
    resolver: &mut Resolver,
    downloads: &mut Downloader,
    atlas: impl Fn(&str) -> bool,
    name: &'a mut String,
) -> &'a str {
    face_skin_with_journal(uuid, listed, resolver, downloads, atlas, None, name)
}

/// Select the same skin as [`face_skin`], optionally postponing this draw's effects.
pub fn face_skin_with_journal<'a>(
    uuid: u128,
    listed: Option<&'a str>,
    resolver: &mut Resolver,
    downloads: &mut Downloader,
    atlas: impl Fn(&str) -> bool,
    journal: Option<&HeadSkinJournal>,
    name: &'a mut String,
) -> &'a str {
    if let Some(skin) = listed {
        return skin;
    }
    let Some(head) = resolved_head_with_journal(&NameOrId::Id(uuid), resolver, journal) else {
        return crate::tab_list::default_skin(uuid);
    };
    match downloaded_skin_with_journal(&head, downloads, &atlas, journal) {
        Some(location) => {
            name.clear();
            name.push_str(location);
            name
        }
        None => crate::tab_list::default_skin(uuid),
    }
}

/// `PlayerSkin.Patch.apply` for a head: the sprite of a profile's `texture`
/// override (`ClientAsset.ResourceTexture`, a resource pack's
/// `<namespace>:textures/<path>.png`), which replaces the downloaded or
/// default skin. Vanilla textures are in the atlas already; another
/// namespace's loads into a skin slot, and `None` answers until it is there
/// (or when the packs lack it).
pub fn patch_skin(
    texture: &str,
    downloads: &mut Downloader,
    atlas: impl Fn(&str) -> bool,
) -> Option<String> {
    patch_skin_with_journal(texture, downloads, atlas, None)
}

fn patch_skin_with_journal(
    texture: &str,
    downloads: &mut Downloader,
    atlas: impl Fn(&str) -> bool,
    journal: Option<&HeadSkinJournal>,
) -> Option<String> {
    let (namespace, path) = texture.split_once(':').unwrap_or(("minecraft", texture));
    if !valid_texture_id(namespace, path) {
        return None;
    }
    if namespace == "minecraft" && atlas(path) {
        return Some(path.to_owned());
    }
    let location = format!("patch/{namespace}/{path}");
    if atlas(&location) {
        if let Some(journal) = journal {
            journal
                .0
                .borrow_mut()
                .push(SkinEffect::Drawn(location.clone()));
        } else {
            downloads.drawn(&location);
        }
        return Some(location);
    }
    if let Some(journal) = journal {
        journal.0.borrow_mut().push(SkinEffect::Resource {
            location,
            namespace: namespace.to_owned(),
            path: path.to_owned(),
        });
    } else {
        downloads.resource(&location, namespace, path);
    }
    None
}

/// `Identifier`'s characters (`isValidNamespace`, `isValidPath`), and no
/// `.`, `..` or empty segment (`FileUtil.decomposePath`, as the packs read
/// it): a server's `texture` never names a file outside the packs.
fn valid_texture_id(namespace: &str, path: &str) -> bool {
    let segment = |s: &str| !s.is_empty() && s != "." && s != "..";
    !namespace.is_empty()
        && namespace
            .chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.'))
        && path
            .chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.' | '/'))
        && segment(namespace)
        && path.split('/').all(segment)
}

/// A `width`x`height` RGBA image as the 64x64 skin it maps onto: each texel
/// the nearest of the image's at the same normalised UV.
fn resample_64(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let mut out = vec![0u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let (sx, sy) = (x * w / 64, y * h / 64);
            let s = (sy * w + sx) * 4;
            let d = (y * 64 + x) * 4;
            out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    Some(out)
}

/// A cape in a 64x64 skin slot: the 64x32 picture (or the older 22x17,
/// which has the same layout) in its top half. Vanilla draws a cape of any
/// size; a larger one is scaled down to 64x32 here.
pub fn process_cape(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let mut img = vec![0u8; 64 * 64 * 4];
    let (fit_w, fit_h) = if w <= 64 && h <= 32 { (w, h) } else { (64, 32) };
    for y in 0..fit_h {
        for x in 0..fit_w {
            let (sx, sy) = (x * w / fit_w, y * h / fit_h);
            let (s, d) = ((sy * w + sx) * 4, (y * 64 + x) * 4);
            img[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    Some(img)
}

/// The cape the launcher gives an offline name (`SUNRISE_OFFLINE_CAPE`, a
/// PNG's path), read once as `process_cape` lays it out.
pub fn offline_cape() -> Option<&'static [u8]> {
    static CAPE: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();
    CAPE.get_or_init(|| {
        let path = std::env::var_os("SUNRISE_OFFLINE_CAPE").filter(|p| !p.is_empty())?;
        let image = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| crate::hud::decode_bytes(&bytes).map_err(|e| e.to_string()))
            .and_then(|(w, h, rgba)| process_cape(w, h, &rgba).ok_or("not an image".into()));
        image
            .inspect_err(|e| eprintln!("Sunrise: no offline cape ({}): {e}", path.display()))
            .ok()
    })
    .as_deref()
}

/// The skin the launcher gives an offline name (`SUNRISE_OFFLINE_SKIN`, a
/// PNG's path, and `SUNRISE_OFFLINE_SKIN_MODEL`, `classic` or `slim`;
/// the launcher sets both): its 64x64 RGBA as `process_legacy_skin` lays it
/// out, and whether it is slim. Read once.
pub fn offline_skin() -> Option<&'static (Vec<u8>, bool)> {
    static SKIN: std::sync::OnceLock<Option<(Vec<u8>, bool)>> = std::sync::OnceLock::new();
    SKIN.get_or_init(|| {
        let path = std::env::var_os("SUNRISE_OFFLINE_SKIN").filter(|p| !p.is_empty())?;
        let model = std::env::var("SUNRISE_OFFLINE_SKIN_MODEL").unwrap_or_default();
        read_offline_skin(std::path::Path::new(&path), &model)
            .inspect_err(|e| eprintln!("Sunrise: no offline skin ({}): {e}", path.display()))
            .ok()
    })
    .as_ref()
}
fn read_offline_skin(path: &std::path::Path, model: &str) -> Result<(Vec<u8>, bool), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let (w, h, rgba) = crate::hud::decode_bytes(&bytes).map_err(|e| e.to_string())?;
    let skin = process_legacy_skin(w, h, &rgba).ok_or("not a 64x64 or 64x32 skin")?;
    Ok((skin, model == "slim"))
}

/// Where a texture download stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Download {
    Ready,
    Pending,
    Failed,
}

/// `MinecraftProfileTexture.Type`: which texture cache a download is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Skin,
    Cape,
    Elytra,
}

/// A resolved `PlayerSkin`. No custom body means the UUID's default body and
/// model, not that the cape and elytra failed to resolve.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ResolvedSkin {
    /// Custom body texture and whether its model is slim.
    pub custom_body: Option<(String, bool)>,
    pub cape: Option<String>,
    pub elytra: Option<String>,
}

impl ResolvedSkin {
    pub fn body(&self, uuid: u128) -> (&str, bool) {
        match &self.custom_body {
            Some((location, slim)) => (location.as_str(), *slim),
            None => default_body(uuid),
        }
    }
}

/// `DefaultPlayerSkin.get(UUID)`'s body texture and whether its model is slim.
fn default_body(uuid: u128) -> (&'static str, bool) {
    let body = crate::tab_list::default_skin(uuid);
    (body, body.contains("/slim/"))
}

/// `PlayerInfo.getSkin` as drawn from the atlas: the complete accepted
/// profile, or the UUID default without accessories while an upload is missing.
pub fn drawable_skin(
    uuid: u128,
    skin: Option<&ResolvedSkin>,
    show_cape: bool,
    mut in_atlas: impl FnMut(&str) -> bool,
) -> (&str, bool, Option<&str>) {
    if let Some(skin) = skin {
        let (body, slim) = skin.body(uuid);
        let body_ready = skin.custom_body.is_none() || in_atlas(body);
        let cape = skin.cape.as_deref().filter(|_| show_cape);
        // Ask for both: the atlas can queue a missing upload on either call.
        let cape_ready = cape.is_none_or(&mut in_atlas);
        if body_ready && cape_ready {
            return (body, slim, cape);
        }
    }
    let (body, slim) = default_body(uuid);
    (body, slim, None)
}

#[derive(Debug, PartialEq, Eq)]
pub enum SkinLookup {
    Ready(ResolvedSkin),
    Pending,
    Failed,
    Disallowed,
}

/// `SkinManager.registerTextures` joins all named textures before
/// `createLookup` may show them. Until the lookup is accepted, the player
/// shows its default body and model without profile accessories.
pub fn lookup(
    textures: &Textures,
    require_secure: bool,
    mut download: impl FnMut(Kind, &str) -> Download,
) -> SkinLookup {
    let mut resolved = ResolvedSkin::default();
    let mut pending = false;
    let mut failed = false;
    for (kind, url) in [
        (Kind::Skin, &textures.skin),
        (Kind::Cape, &textures.cape),
        (Kind::Elytra, &textures.elytra),
    ] {
        let Some(url) = url else { continue };
        let Some(hash) = texture_hash(url) else {
            failed = true;
            continue;
        };
        match download(kind, url) {
            Download::Failed => failed = true,
            Download::Pending => pending = true,
            Download::Ready => match kind {
                Kind::Skin => resolved.custom_body = Some((skin_location(&hash), textures.slim)),
                Kind::Cape => resolved.cape = Some(cape_location(&hash)),
                Kind::Elytra => resolved.elytra = Some(location("elytra/", &hash)),
            },
        }
    }
    if pending {
        SkinLookup::Pending
    } else if failed {
        SkinLookup::Failed
    } else if require_secure && textures.signature != Signature::Signed {
        SkinLookup::Disallowed
    } else {
        SkinLookup::Ready(resolved)
    }
}

/// `SkinTextureDownloader.processLegacySkin` on RGBA pixels: 64x32 skins
/// get the 64x64 layout, and the opaque parts lose their alpha. `None` for
/// any other size (the download fails).
pub fn process_legacy_skin(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    if width != 64 || height != 32 && height != 64 {
        return None;
    }
    let legacy = height == 32;
    let mut img = vec![0u8; 64 * 64 * 4];
    let n = rgba.len().min(img.len());
    img[..n].copy_from_slice(&rgba[..n]);
    let at = |x: i32, y: i32| ((y * 64 + x) * 4) as usize;
    if legacy {
        let rects = [
            (4, 16, 16, 32, 4, 4),
            (8, 16, 16, 32, 4, 4),
            (0, 20, 24, 32, 4, 12),
            (4, 20, 16, 32, 4, 12),
            (8, 20, 8, 32, 4, 12),
            (12, 20, 16, 32, 4, 12),
            (44, 16, -8, 32, 4, 4),
            (48, 16, -8, 32, 4, 4),
            (40, 20, 0, 32, 4, 12),
            (44, 20, -8, 32, 4, 12),
            (48, 20, -16, 32, 4, 12),
            (52, 20, -8, 32, 4, 12),
        ];
        for (sx, sy, ox, oy, w, h) in rects {
            for y in 0..h {
                for x in 0..w {
                    let (s, d) = (at(sx + x, sy + y), at(sx + ox + w - 1 - x, sy + oy + y));
                    let p: [u8; 4] = img[s..s + 4].try_into().unwrap();
                    img[d..d + 4].copy_from_slice(&p);
                }
            }
        }
    }
    let no_alpha = |img: &mut Vec<u8>, x0: i32, y0: i32, x1: i32, y1: i32| {
        for x in x0..x1 {
            for y in y0..y1 {
                img[at(x, y) + 3] = 255;
            }
        }
    };
    no_alpha(&mut img, 0, 0, 32, 16);
    if legacy {
        // doNotchTransparencyHack: a fully opaque hat layer is cleared.
        let opaque = (32..64).all(|x| (0..32).all(|y| img[at(x, y) + 3] >= 128));
        if opaque {
            for x in 32..64 {
                for y in 0..32 {
                    img[at(x, y) + 3] = 0;
                }
            }
        }
    }
    no_alpha(&mut img, 0, 16, 64, 32);
    no_alpha(&mut img, 16, 48, 48, 64);
    Some(img)
}

/// `SkinTextureDownloader`: fetches textures on background threads, from
/// the local cache (`<root>/<2 hex>/<sha1>`) or over HTTP, keeping each one
/// by kind and hash as vanilla's `TextureCache`s do. A texture that can't
/// be fetched fails, and the player keeps their default skin.
pub struct Downloader {
    root: std::path::PathBuf,
    states: std::collections::HashMap<(Kind, String), Download>,
    done: std::sync::mpsc::Receiver<((Kind, String), Option<Vec<u8>>)>,
    send: std::sync::mpsc::Sender<((Kind, String), Option<Vec<u8>>)>,
    ready: Vec<(String, Vec<u8>)>,
    /// Locations already in `ready`, so `again` does not double-queue.
    queued: std::collections::HashSet<String>,
    /// Decoded skin images by location, kept so a skin evicted from the
    /// atlas uploads again (`again`); the disk cache backs anything dropped.
    images: std::collections::HashMap<String, Vec<u8>>,
    image_order: std::collections::VecDeque<String>,
    /// Skins and capes drawn from the atlas since `take_drawn`.
    drawn: std::collections::HashSet<String>,
    /// The resource packs' root, where a profile's `texture` patch is read.
    resource_root: sunrise_assets::resource_source::ResourceSource,
    /// How each `patch/` texture of the resource packs loaded, by location.
    resources: std::collections::HashMap<String, Download>,
    /// When each failed texture download was last tried, so it can be retried:
    /// a transient network error must not pin a player to the default skin for
    /// the rest of the session.
    failed: std::collections::HashMap<(Kind, String), std::time::Instant>,
    /// How long a failed texture download waits before it is tried again.
    retry_after: std::time::Duration,
    /// The `states` keys, oldest first, so the oldest settled one can go ([`Bounds::states`]).
    state_order: std::collections::VecDeque<(Kind, String)>,
    /// The `resources` keys, oldest first ([`Bounds::resources`]).
    resource_order: std::collections::VecDeque<String>,
    /// Downloads started whose answer `state` has not taken yet: one thread each.
    in_flight: usize,
    /// Fetches a texture's body; the HTTP GET, but for a test.
    fetch: fn(&str) -> Option<Vec<u8>>,
    bounds: Bounds,
    skin_cache: SkinCache,
}
/// A worker thread's answer to the game thread. If the thread panics before giving it (a body
/// that breaks the decoder, say), dropping this sends the failure it was made with instead: a
/// download's place in `in_flight` is given back and the texture is retried after the wait, where
/// an answer that never came kept the key `Pending` for good and, with the bound, the place taken
/// for the session; a profile keeps the default skin.
struct Answer<T> {
    send: std::sync::mpsc::Sender<T>,
    /// The failure the game thread is owed until `give` sends the answer.
    owed: Option<T>,
}
impl<T> Answer<T> {
    fn give(&mut self, answer: T) {
        if self.owed.take().is_some() {
            let _ = self.send.send(answer);
        }
    }
}
impl<T> Drop for Answer<T> {
    fn drop(&mut self) {
        if let Some(failed) = self.owed.take() {
            let _ = self.send.send(failed);
        }
    }
}

/// What the downloader keeps and runs at most. Every texture URL a server (or a tab list of
/// hundreds of players) names used to add a `states` entry and a thread of its own, and every
/// `texture` id a `resources` entry, none of them ever dropped.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    /// Entries of `states`; the oldest settled (ready or failed) one is dropped past this. A
    /// dropped texture is loaded again from the disk cache when it is asked for.
    states: usize,
    /// Entries of `resources`; the oldest is dropped past this, and read from the packs again.
    resources: usize,
    /// Downloads running at once. Past this `state` says `Pending` and starts nothing, so the
    /// caller, which asks again every frame, starts it when one has finished.
    downloads: usize,
}
const BOUNDS: Bounds = Bounds {
    states: 4096,
    resources: 4096,
    downloads: 16,
};
/// `images` is bounded only so a long session on a busy server does not grow
/// without end; the disk cache holds everything it drops.
const MAX_SKIN_IMAGES: usize = 1024;

/// A skin cache entry's textures: `None` until the thread that unpacks the property answers.
struct SkinEntry {
    touched: std::time::Instant,
    textures: Option<std::sync::Arc<Textures>>,
}

/// `SkinManager.skinCache`: every profile property the game thread asked for, each unpacked and
/// its signature verified on a thread of its own, as vanilla unpacks on its background executor,
/// so the game thread only reads. An entry goes 15 s after its last ask (`expireAfterAccess`).
struct SkinCache {
    entries: std::collections::HashMap<PackedTextures, SkinEntry>,
    done: std::sync::mpsc::Receiver<(PackedTextures, Textures)>,
    send: std::sync::mpsc::Sender<(PackedTextures, Textures)>,
    /// Unpack threads started whose answer is not taken yet: at most `threads` run at once.
    running: usize,
    /// `Util.maxAllowedExecutorThreads`: the workers vanilla's background executor has, so the
    /// threads that unpack at once.
    threads: usize,
    /// Unpacks and verifies a property: [`PackedTextures::unpack`], or a test's.
    pack: fn(&PackedTextures, &[sunrise_protocol::rsa::RsaPublicKey]) -> Textures,
    swept: std::time::Instant,
}
/// `SkinManager`'s `expireAfterAccess`.
const SKIN_TTL: std::time::Duration = std::time::Duration::from_secs(15);

impl SkinCache {
    fn new() -> Self {
        let (send, done) = std::sync::mpsc::channel();
        Self {
            entries: Default::default(),
            done,
            send,
            running: 0,
            threads: sunrise_threads::worker_count()
                .saturating_sub(1)
                .clamp(1, 255),
            pack: PackedTextures::unpack,
            swept: std::time::Instant::now(),
        }
    }

    /// `SkinManager.getOrLoad`: the textures of `packed`, its signature verified against
    /// `profile_property_keys`. The first ask starts the thread that unpacks them: `None` until
    /// it answers, so the player shows the default skin meanwhile; a property asked while every
    /// thread is busy is started by a later ask.
    fn get(
        &mut self,
        packed: &PackedTextures,
        profile_property_keys: &[sunrise_protocol::rsa::RsaPublicKey],
    ) -> Option<std::sync::Arc<Textures>> {
        while let Ok((key, textures)) = self.done.try_recv() {
            self.running = self.running.saturating_sub(1);
            // An answer whose entry is gone (swept, or forgotten with its keys) is nobody's.
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.textures = Some(std::sync::Arc::new(textures));
            }
        }
        let now = std::time::Instant::now();
        if now - self.swept >= SKIN_TTL {
            self.swept = now;
            self.entries
                .retain(|_, entry| now - entry.touched < SKIN_TTL);
        }
        if let Some(entry) = self.entries.get_mut(packed) {
            entry.touched = now;
            return entry.textures.clone();
        }
        if self.running >= self.threads {
            return None;
        }
        self.running += 1;
        self.entries.insert(
            packed.clone(),
            SkinEntry {
                touched: now,
                textures: None,
            },
        );
        let (send, pack, keys) = (self.send.clone(), self.pack, profile_property_keys.to_vec());
        let packed = packed.clone();
        std::thread::spawn(move || {
            let mut answer = Answer {
                send,
                owed: Some((packed.clone(), Textures::default())),
            };
            let textures = pack(&packed, &keys);
            answer.give((packed, textures));
        });
        None
    }
}
impl Downloader {
    /// The decoded 64×64 RGBA image an atlas location holds, when it is still in memory (Realms).
    pub fn image(&self, location: &str) -> Option<&[u8]> {
        self.images.get(location).map(Vec::as_slice)
    }
    /// Keeps the launcher's offline cape for `again`, as it skips `queue`.
    pub fn remember_image(&mut self, location: &str, rgba: &[u8]) {
        self.remember(location.to_owned(), rgba.to_vec());
    }
    pub fn new(root: std::path::PathBuf) -> Self {
        let (send, done) = std::sync::mpsc::channel();
        Self {
            root,
            states: Default::default(),
            done,
            send,
            ready: Vec::new(),
            queued: Default::default(),
            images: Default::default(),
            image_order: Default::default(),
            drawn: Default::default(),
            resource_root: sunrise_assets::resource_source::ResourceSource::indexed(
                Default::default(),
            ),
            resources: Default::default(),
            failed: Default::default(),
            retry_after: std::time::Duration::from_secs(60),
            state_order: Default::default(),
            resource_order: Default::default(),
            in_flight: 0,
            fetch: http_get,
            bounds: BOUNDS,
            skin_cache: SkinCache::new(),
        }
    }
    /// The resource packs changed (or were first loaded) at `root`: patch
    /// textures load again from there.
    pub fn set_resource_root(&mut self, root: sunrise_assets::resource_source::ResourceSource) {
        let previous = std::mem::take(&mut self.resources);
        self.images
            .retain(|location, _| !previous.contains_key(location));
        self.image_order
            .retain(|location| !previous.contains_key(location));
        self.queued
            .retain(|location| !previous.contains_key(location));
        self.ready
            .retain(|(location, _)| !previous.contains_key(location));
        self.resource_order.clear();
        self.resource_root = root;
    }
    /// `ClientAsset.ResourceTexture`: loads `<namespace>:textures/<path>.png`
    /// from the resource packs into a skin slot as `location`, once. A
    /// texture of another size is sampled as the model's normalised UVs would
    /// sample it (nearest texel) into the 64x64 slot.
    fn resource(&mut self, location: &str, namespace: &str, path: &str) {
        match self.resources.get(location) {
            Some(Download::Failed) => return,
            Some(_) => {
                self.again(location);
                return;
            }
            None => {}
        }
        let image = self
            .resource_root
            .read(&format!("assets/{namespace}/textures/{path}.png"))
            .ok()
            .and_then(|b| crate::hud::decode_bytes(&b).ok())
            .and_then(|(w, h, rgba)| resample_64(w, h, &rgba));
        let state = match image {
            Some(rgba) => {
                self.queue(location.to_owned(), rgba);
                Download::Ready
            }
            None => Download::Failed,
        };
        if self.resources.insert(location.to_owned(), state).is_none() {
            self.resource_order.push_back(location.to_owned());
            while self.resources.len() > self.bounds.resources
                && let Some(old) = self.resource_order.pop_front()
            {
                self.resources.remove(&old);
            }
        }
    }
    /// Forgets how `location` loaded, so `resource` reads it again.
    fn forget_resource(&mut self, location: &str) {
        if self.resources.remove(location).is_some() {
            self.resource_order.retain(|l| l != location);
        }
    }
    /// Records how `key` stands. A new key may push the oldest settled one out of `states`;
    /// downloads running are never dropped, so their answer has a key to land on.
    fn set_state(&mut self, key: (Kind, String), state: Download) {
        if self.states.insert(key.clone(), state).is_some() {
            return;
        }
        self.state_order.push_back(key);
        while self.states.len() > self.bounds.states {
            let Some(at) = self
                .state_order
                .iter()
                .position(|k| self.states.get(k) != Some(&Download::Pending))
            else {
                break;
            };
            if let Some(old) = self.state_order.remove(at) {
                self.states.remove(&old);
                self.failed.remove(&old);
            }
        }
    }
    fn receive_completed(&mut self) {
        while let Ok((key, image)) = self.done.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            let state = match image {
                Some(rgba) => {
                    match key.0 {
                        Kind::Skin => self.queue(skin_location(&key.1), rgba),
                        Kind::Cape => self.queue(cape_location(&key.1), rgba),
                        Kind::Elytra => {}
                    }
                    self.failed.remove(&key);
                    Download::Ready
                }
                None => {
                    self.failed.insert(key.clone(), std::time::Instant::now());
                    Download::Failed
                }
            };
            self.set_state(key, state);
        }
    }
    /// How the download of `url` stands, starting it the first time. A
    /// failure is permanent for `retry_after`, then tried again.
    pub fn state(&mut self, kind: Kind, url: &str) -> Download {
        self.receive_completed();
        let Some(hash) = texture_hash(url) else {
            return Download::Failed;
        };
        let key = (kind, hash);
        let full = self.in_flight >= self.bounds.downloads;
        if let Some(state) = self.states.get(&key).copied() {
            let due = self
                .failed
                .get(&key)
                .is_none_or(|at| at.elapsed() >= self.retry_after);
            // A failure that is due waits for a free download like a new texture does.
            if state != Download::Failed || !due || full {
                return state;
            }
            self.states.remove(&key);
            self.failed.remove(&key);
            self.state_order.retain(|k| *k != key);
        }
        if full {
            // Not recorded: the next call asks again, and starts it once one has finished.
            return Download::Pending;
        }
        self.in_flight += 1;
        self.set_state(key.clone(), Download::Pending);
        let digest: String = skin_location(&key.1)["skins/".len()..].to_owned();
        let (url, send, root) = (url.to_owned(), self.send.clone(), self.root.clone());
        let fetch = self.fetch;
        std::thread::spawn(move || {
            let mut answer = Answer {
                send,
                owed: Some((key.clone(), None)),
            };
            let image = Self::load_texture(&root, kind, &digest, &url, fetch);
            answer.give((key, image));
        });
        Download::Pending
    }

    /// `SkinManager.getOrLoad`: the textures of a profile's `textures` property, unpacked and its
    /// signature verified against `profile_property_keys` off the game thread. `None` until that
    /// answers: the player shows the default skin meanwhile, and on every ask until then.
    pub fn unpacked(
        &mut self,
        packed: &PackedTextures,
        profile_property_keys: &[sunrise_protocol::rsa::RsaPublicKey],
    ) -> Option<std::sync::Arc<Textures>> {
        self.skin_cache.get(packed, profile_property_keys)
    }

    /// Forgets every unpacked property for a new set of services keys. Minecraft makes its
    /// services once per run, and Sunrise fetches their keys for each connection: no verdict
    /// outlives the set it was made with, and a thread still unpacking for the old set answers
    /// a cache nobody reads.
    pub fn forget_unpacked(&mut self) {
        self.skin_cache = SkinCache::new();
    }
    /// One texture's pixels: the disk cache's body if it decodes, else a fetch
    /// (`SkinTextureDownloader.downloadSkin`). Only a body that decodes is
    /// written to the cache: an error page or a half image must not be what a
    /// later start reads instead of fetching again.
    fn load_texture(
        root: &std::path::Path,
        kind: Kind,
        digest: &str,
        url: &str,
        fetch: impl FnOnce(&str) -> Option<Vec<u8>>,
    ) -> Option<Vec<u8>> {
        let file = root
            .join(if digest.len() > 2 { &digest[..2] } else { "xx" })
            .join(digest);
        let decode = |bytes: &[u8]| -> Option<Vec<u8>> {
            let (w, h, rgba) = crate::hud::decode_bytes(bytes).ok()?;
            match kind {
                Kind::Skin => process_legacy_skin(w, h, &rgba),
                Kind::Cape => process_cape(w, h, &rgba),
                Kind::Elytra => Some(rgba),
            }
        };
        if let Some(image) = std::fs::read(&file).ok().and_then(|b| decode(&b)) {
            return Some(image);
        }
        let body = fetch(url)?;
        let image = decode(&body)?;
        let _ = std::fs::create_dir_all(file.parent()?);
        let _ = std::fs::write(&file, &body);
        Some(image)
    }
    /// Keeps a decoded skin for `again`, dropping the oldest past the bound.
    fn remember(&mut self, location: String, rgba: Vec<u8>) {
        if !self.images.contains_key(&location) {
            self.image_order.push_back(location.clone());
            while self.image_order.len() > MAX_SKIN_IMAGES
                && let Some(old) = self.image_order.pop_front()
            {
                self.images.remove(&old);
            }
        }
        self.images.insert(location, rgba);
    }
    /// An image off to the atlas: remembered for `again` and marked queued
    /// so nothing uploads the same location twice.
    fn queue(&mut self, location: String, rgba: Vec<u8>) {
        self.remember(location.clone(), rgba.clone());
        self.queued.insert(location.clone());
        self.ready.push((location, rgba));
    }
    /// A ready skin whose atlas slot was evicted (`show_skin` answered no):
    /// its image for upload again, from memory or the disk cache.
    pub fn again(&mut self, location: &str) -> bool {
        if self.queued.contains(location) {
            return true;
        }
        if location.starts_with("patch/") {
            let Some(image) = self.images.get(location).cloned() else {
                // Dropped from memory: read from the packs again.
                self.forget_resource(location);
                return false;
            };
            self.queue(location.to_owned(), image);
            return true;
        }
        let (digest, process): (_, fn(u32, u32, &[u8]) -> Option<Vec<u8>>) =
            if let Some(digest) = location.strip_prefix("skins/") {
                (digest, process_legacy_skin)
            } else if let Some(digest) = location.strip_prefix("capes/") {
                (digest, process_cape)
            } else {
                return false;
            };
        let image = self.images.get(location).cloned().or_else(|| {
            let file = self
                .root
                .join(if digest.len() > 2 { &digest[..2] } else { "xx" })
                .join(digest);
            let (w, h, rgba) = crate::hud::decode_bytes(&std::fs::read(&file).ok()?).ok()?;
            process(w, h, &rgba)
        });
        let Some(image) = image else {
            return false;
        };
        self.queue(location.to_owned(), image);
        true
    }
    /// Skins that finished since the last call: texture id and 64x64 RGBA.
    pub fn take_ready(&mut self) -> Vec<(String, Vec<u8>)> {
        for (location, _) in &self.ready {
            self.queued.remove(location);
        }
        std::mem::take(&mut self.ready)
    }
    /// A skin or cape in the atlas drawn this frame: `SkinSlots` keeps its slot.
    pub fn drawn(&mut self, location: &str) {
        if !self.drawn.contains(location) {
            self.drawn.insert(location.to_owned());
        }
    }
    /// The skins and capes drawn since the last call.
    pub fn take_drawn(&mut self) -> std::collections::HashSet<String> {
        std::mem::take(&mut self.drawn)
    }
}

/// Which downloaded skins and capes hold the atlas's skin slots
/// (`hud::SKIN_SLOTS`). Vanilla's `SkinManager.TextureCache.getOrLoad`
/// registers a texture per skin and keeps it. Here the slots are shared: a
/// new skin takes a free slot, else the least recently drawn skin's, but
/// never one drawn in the last frame. With every slot drawn, the new skin
/// waits (its player keeps the default skin) instead of pushing out one in
/// use, which then came back by pushing out another, so players vanished
/// for a frame and heads swapped textures as the slots went round.
pub struct SkinSlots {
    capacity: usize,
    /// Stored locations and their slot, the least recently drawn first.
    slots: Vec<(String, usize)>,
    /// Drawn in the last frame, or stored since: kept from eviction.
    kept: std::collections::HashSet<String>,
}
impl SkinSlots {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            slots: Vec::new(),
            kept: Default::default(),
        }
    }
    /// The atlas was replaced (a resource pack reload): every slot is blank,
    /// so no skin counts as stored and each uploads again.
    pub fn forget(&mut self) {
        self.slots.clear();
        self.kept.clear();
    }
    /// Whether `location` is in a slot.
    pub fn contains(&self, location: &str) -> bool {
        self.slots.iter().any(|(l, _)| l == location)
    }
    /// The slot `location` holds, if any.
    pub fn slot(&self, location: &str) -> Option<usize> {
        self.slots
            .iter()
            .find(|(l, _)| l == location)
            .map(|(_, slot)| *slot)
    }
    /// A new frame: the skins `drawn` in the last one
    /// (`Downloader::take_drawn`) become the most recently drawn, and keep
    /// their slots through this frame's `place`s.
    pub fn next_frame(&mut self, drawn: std::collections::HashSet<String>) {
        let (recent, old): (Vec<_>, Vec<_>) = std::mem::take(&mut self.slots)
            .into_iter()
            .partition(|(l, _)| drawn.contains(l));
        self.slots = old;
        self.slots.extend(recent);
        self.kept = drawn;
    }
    /// A slot for `location`: its own, a free one, else the least recently
    /// drawn one not kept, with the location it held. `None` while every
    /// slot is kept.
    pub fn place(&mut self, location: String) -> Option<(usize, Option<String>)> {
        if let Some((_, slot)) = self.slots.iter().find(|(l, _)| *l == location) {
            return Some((*slot, None));
        }
        let (slot, evicted) = if self.slots.len() < self.capacity {
            (self.slots.len(), None)
        } else {
            let i = self
                .slots
                .iter()
                .position(|(l, _)| !self.kept.contains(l))?;
            let (old, slot) = self.slots.remove(i);
            (slot, Some(old))
        };
        self.kept.insert(location.clone());
        self.slots.push((location, slot));
        Some((slot, evicted))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn head_profile_cache_matches_uncached_and_is_bounded() {
        use base64::Engine;
        let mut cache = super::HeadProfiles::default();
        for (url, slim) in [
            ("https://textures.minecraft.net/texture/abc", false),
            ("https://textures.minecraft.net/texture/abc", true),
            ("https://evil.example/texture/abc", false),
        ] {
            let payload = serde_json::json!({"textures": {
                "SKIN": {"url": url, "metadata": {"model": if slim { "slim" } else { "classic" }}},
                "CAPE": {"url": "http://textures.minecraft.net/texture/cape"}
            }});
            let value = base64::engine::general_purpose::STANDARD.encode(payload.to_string());
            for signed in [false, true] {
                let mut property = serde_json::json!({"name": "textures", "value": value});
                if signed {
                    property["signature"] = serde_json::Value::Null;
                }
                let profile = serde_json::json!({"properties": [property]});
                let parsed = cache.get(&profile);
                assert_eq!(*parsed, super::prepare_head(&profile));
                let textures = super::unpack(
                    &value,
                    if signed {
                        super::Signature::Signed
                    } else {
                        super::Signature::Unsigned
                    },
                    &super::TEXTURE_URIS,
                );
                assert_eq!(textures.slim, slim && !url.contains("evil"));
                // Authlib rejects the whole payload if any URL is invalid,
                // including the allowed cape when the skin is disallowed.
                assert_eq!(textures.cape.is_some(), !url.contains("evil"));
                assert_eq!(parsed.skin.is_some(), !url.contains("evil"));
                assert!(std::sync::Arc::ptr_eq(&parsed, &cache.get(&profile)));
            }
        }
        for profile in [
            serde_json::json!({"properties": [{"name": "textures", "value": "!bad"}]}),
            serde_json::json!({"name": "Notch"}),
            serde_json::json!({}),
        ] {
            let parsed = cache.get(&profile);
            assert_eq!(*parsed, super::prepare_head(&profile));
            assert!(parsed.skin.is_none());
            assert!(std::sync::Arc::ptr_eq(&parsed, &cache.get(&profile)));
        }
        cache.entries.clear();
        for i in 0..super::MAX_HEAD_PROFILES {
            cache.get(&serde_json::json!({"id": i}));
        }
        assert_eq!(cache.entries.len(), super::MAX_HEAD_PROFILES);
        cache.get(&serde_json::json!({}));
        assert_eq!(cache.entries.len(), 1);
    }

    #[test]
    fn head_profile_nbt_cache_tracks_value_changes() {
        let mut resolver = super::Resolver::new();
        let mut tag = sunrise_core::nbt::Tag::String("Notch".into());
        let first = resolver.head_profile_nbt(&tag);
        assert_eq!(*first, crate::scoreboard::tag_to_json(&tag));
        assert!(std::sync::Arc::ptr_eq(
            &first,
            &resolver.head_profile_nbt(&tag)
        ));
        tag = sunrise_core::nbt::Tag::String("Alex".into());
        assert_eq!(*resolver.head_profile_nbt(&tag), serde_json::json!("Alex"));
        for i in 0..super::MAX_HEAD_PROFILES {
            resolver.head_profile_nbt(&sunrise_core::nbt::Tag::Int(i as i32));
        }
        assert!(resolver.heads.nbt.len() < super::MAX_HEAD_PROFILES);
    }

    /// A profile's `texture` names a pack texture, never a file outside the
    /// packs: bad characters and `..` segments draw nothing.
    #[test]
    fn a_texture_patch_stays_inside_the_packs() {
        use super::valid_texture_id;
        assert!(valid_texture_id("ethereal", "entity/plush_blank"));
        assert!(valid_texture_id("minecraft", "entity/player/wide/steve"));
        assert!(!valid_texture_id("ethereal", "../../../secret"));
        assert!(!valid_texture_id("..", "entity/x"));
        assert!(!valid_texture_id("ethereal", "entity//x"));
        assert!(!valid_texture_id("Ethereal", "entity/x"));
        assert!(!valid_texture_id("ethereal", "entity\\x"));
        assert!(!valid_texture_id("", "entity/x"));
    }

    /// A pack reload swaps the atlas: its skin slots are blank, so a forgotten
    /// table holds nothing and hands out slots from the start again, even for
    /// skins drawn in the last frame.
    #[test]
    fn a_new_atlas_forgets_every_skin_slot() {
        use super::SkinSlots;
        let mut slots = SkinSlots::new(2);
        assert_eq!(slots.place("skins/a".into()), Some((0, None)));
        assert_eq!(slots.place("skins/b".into()), Some((1, None)));
        assert_eq!(slots.place("skins/c".into()), None);
        slots.forget();
        assert!(!slots.contains("skins/a"));
        assert!(!slots.contains("skins/b"));
        assert_eq!(slots.place("skins/c".into()), Some((0, None)));
        assert_eq!(slots.place("skins/a".into()), Some((1, None)));
    }

    /// The launcher's offline skin: a 64x64 or legacy 64x32 PNG laid out as
    /// a downloaded skin, slim only for the `slim` model; anything else is
    /// refused and the player keeps their default skin.
    #[test]
    fn offline_skins_read_like_downloaded_ones() {
        let png = |w: u32, h: u32| {
            let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
            let mut out = Vec::new();
            let mut encoder = png::Encoder::new(&mut out, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&rgba)
                .unwrap();
            (out, rgba)
        };
        let dir = std::env::temp_dir().join(format!("sunrise-offline-skin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("steve.png");
        for (h, model, slim) in [(64, "slim", true), (32, "classic", false), (64, "", false)] {
            let (bytes, rgba) = png(64, h);
            std::fs::write(&file, bytes).unwrap();
            let (skin, is_slim) = super::read_offline_skin(&file, model).unwrap();
            assert_eq!(skin, super::process_legacy_skin(64, h, &rgba).unwrap());
            assert_eq!(skin.len(), 64 * 64 * 4);
            assert_eq!(is_slim, slim);
        }
        std::fs::write(&file, png(32, 32).0).unwrap();
        assert!(super::read_offline_skin(&file, "slim").is_err());
        std::fs::write(&file, b"not a png").unwrap();
        assert!(super::read_offline_skin(&file, "slim").is_err());
        assert!(super::read_offline_skin(&dir.join("missing.png"), "").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn services_profile_answers_give_the_active_skin_and_cape() {
        let profile = serde_json::json!({
            "skins": [
                {"state": "INACTIVE", "url": "https://textures.minecraft.net/texture/old"},
                {"state": "ACTIVE", "url": "https://textures.minecraft.net/texture/skin1", "variant": "CLASSIC"}
            ],
            "capes": [{"state": "ACTIVE", "url": "http://textures.minecraft.net/texture/cape1"}]
        });
        let t = super::services_profile_textures(&profile);
        assert_eq!(
            t.skin.as_deref(),
            Some("https://textures.minecraft.net/texture/skin1")
        );
        assert_eq!(
            t.cape.as_deref(),
            Some("http://textures.minecraft.net/texture/cape1")
        );
        assert_eq!(t.signature, super::Signature::Unsigned);
        let elsewhere = serde_json::json!({"skins": [{"state": "ACTIVE", "url": "https://evil.example/texture/x"}]});
        assert_eq!(
            super::services_profile_textures(&elsewhere),
            super::Textures::default()
        );
        assert_eq!(
            super::services_profile_textures(&serde_json::json!({})),
            super::Textures::default()
        );
    }

    /// More skins drawn every frame than there are slots (#3829): a frame's
    /// uploads used to push out skins drawn the frame before, which then
    /// uploaded again and pushed out others, so the drawn skins went missing
    /// in turn. Those drawn keep their slots; the rest wait.
    #[test]
    fn skins_drawn_last_frame_keep_their_slots() {
        let mut d = super::Downloader::new(std::env::temp_dir().join("sunrise-test-skin-slots"));
        let skins: Vec<String> = (0..6).map(|i| format!("skins/{i}")).collect();
        for (i, s) in skins.iter().enumerate() {
            d.remember(s.clone(), vec![i as u8]);
        }
        let mut slots = super::SkinSlots::new(4);
        let mut last: Option<Vec<String>> = None;
        for frame in 0..8 {
            // draw_tab_list: the downloads that finished go to the atlas.
            slots.next_frame(d.take_drawn());
            let mut uploads = 0;
            for (location, _) in d.take_ready() {
                if slots.place(location).is_some() {
                    uploads += 1;
                }
            }
            // update_remote_skins: every skin is drawn, from its slot, or
            // asked for again while it has none.
            let mut shown = Vec::new();
            for s in &skins {
                if slots.contains(s) {
                    d.drawn(s);
                    shown.push(s.clone());
                } else {
                    d.again(s);
                }
            }
            if frame >= 2 {
                assert_eq!(uploads, 0, "frame {frame} uploaded again");
                assert_eq!(Some(&shown), last.as_ref(), "frame {frame} lost a skin");
            }
            // The first frame asks for the skins; from the next, four show.
            assert_eq!(shown.len(), if frame == 0 { 0 } else { 4 });
            last = Some(shown);
        }
        // A skin no longer drawn gives up its slot to a waiting one.
        let mut slots = super::SkinSlots::new(2);
        slots.next_frame(Default::default());
        assert_eq!(slots.place("skins/a".into()), Some((0, None)));
        assert_eq!(slots.place("skins/b".into()), Some((1, None)));
        assert_eq!(slots.place("skins/c".into()), None);
        slots.next_frame(["skins/b".to_owned()].into());
        assert_eq!(
            slots.place("skins/c".into()),
            Some((0, Some("skins/a".into())))
        );
        // Stored this frame, kept through it.
        assert_eq!(slots.place("skins/d".into()), None);
        assert!(slots.contains("skins/b") && slots.contains("skins/c"));
    }

    /// A skin evicted from the atlas re-uploads (`again`), once per ask.
    #[test]
    fn evicted_skins_upload_again() {
        let mut d = super::Downloader::new(std::env::temp_dir().join("sunrise-test-skins"));
        d.remember("skins/abc".into(), vec![1, 2, 3]);
        assert!(d.again("skins/abc"));
        assert!(!d.again("skins/missing"));
        // Once queued, more asks do not pile up uploads.
        assert!(d.again("skins/abc"));
        assert_eq!(
            d.take_ready(),
            vec![("skins/abc".to_owned(), vec![1, 2, 3])]
        );
        assert!(d.take_ready().is_empty());
        assert!(d.again("skins/abc"));
        assert_eq!(d.take_ready().len(), 1);
        // The first upload marks the location queued too: an `again` before
        // `take_ready` does not put the same skin in the atlas twice.
        d.queue("skins/def".into(), vec![4]);
        assert!(d.again("skins/def"));
        assert_eq!(d.take_ready(), vec![("skins/def".to_owned(), vec![4])]);
    }

    /// A cape fills the top half of a 64x64 slot; a larger one is scaled to 64x32.
    #[test]
    fn capes_fill_the_top_half_of_a_slot() {
        let at = |x: usize, y: usize| (y * 64 + x) * 4;
        let legacy = super::process_cape(22, 17, &[7; 22 * 17 * 4]).unwrap();
        assert_eq!(legacy.len(), 64 * 64 * 4);
        assert_eq!(legacy[at(21, 16)], 7);
        assert_eq!(legacy[at(22, 16)], 0);
        let mut hd = vec![0u8; 128 * 64 * 4];
        hd[(62 * 128 + 126) * 4] = 9;
        let hd = super::process_cape(128, 64, &hd).unwrap();
        assert_eq!(hd[at(63, 31)], 9);
        assert_eq!(hd[at(0, 32)], 0);
        assert!(super::process_cape(64, 32, &[0; 4]).is_none());
        let cape = super::cape_location("abc");
        assert_eq!(
            cape["capes/".len()..],
            super::skin_location("abc")["skins/".len()..]
        );
    }

    #[test]
    fn profile_components_supply_unsigned_head_skins() {
        let value = "eyJ0ZXh0dXJlcyI6IHsiU0tJTiI6IHsidXJsIjogImh0dHBzOi8vdGV4dHVyZXMubWluZWNyYWZ0Lm5ldC90ZXh0dXJlL2FiYzEyMyJ9fX0=";
        let profile = serde_json::json!({"properties": [{"name": "textures", "value": value}]});
        assert_eq!(
            super::profile_skin_url(&profile).as_deref(),
            Some("https://textures.minecraft.net/texture/abc123")
        );
        let other = serde_json::json!({"properties": [{"name": "other", "value": value}]});
        assert_eq!(super::profile_skin_url(&other), None);
        let bad = serde_json::json!({"properties": [{"name": "textures", "value": "not base64"}]});
        assert_eq!(super::profile_skin_url(&bad), None);
        assert_eq!(super::profile_skin_url(&serde_json::json!({})), None);
    }

    /// Textures come over plain HTTP too (a local server here; https is the
    /// same request through rustls), and only 2xx answers count.
    #[test]
    #[ignore = "daily"]
    fn http_get_reads_2xx_bodies() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                reader.read_line(&mut line).unwrap();
                while reader.read_line(&mut String::new()).unwrap() > 2 {}
                let reply: &[u8] = if line.contains("/texture/abc") {
                    b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nPNG!"
                } else {
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                };
                stream.write_all(reply).unwrap();
            }
        });
        assert_eq!(
            http_get(&format!("{base}/texture/abc")),
            Some(b"PNG!".to_vec())
        );
        assert_eq!(http_get(&format!("{base}/texture/missing")), None);
    }

    use super::*;

    /// `SkinManager.registerTextures` waits on the skin, cape and elytra
    /// together (`CompletableFuture.allOf`): a cape or elytra that cannot be
    /// fetched, or whose URL has no hash, fails the profile, and the player
    /// shows the default skin, as the skin's own failure does.
    #[test]
    fn a_failed_cape_or_elytra_shows_the_default_skin() {
        let textures = super::Textures {
            skin: Some("https://textures.minecraft.net/texture/skin1".into()),
            cape: Some("https://textures.minecraft.net/texture/cape1".into()),
            elytra: Some("https://textures.minecraft.net/texture/elytra1".into()),
            signature: super::Signature::Signed,
            slim: false,
        };
        let all_ready = super::lookup(&textures, true, |_, _| super::Download::Ready);
        let super::SkinLookup::Ready(resolved) = all_ready else {
            panic!("all textures are ready");
        };
        assert_eq!(
            resolved.custom_body,
            Some((super::skin_location("skin1"), false))
        );
        assert_eq!(resolved.cape, Some(super::cape_location("cape1")));
        for failing in [super::Kind::Skin, super::Kind::Cape, super::Kind::Elytra] {
            let skin = super::lookup(&textures, true, |kind, _| {
                if kind == failing {
                    super::Download::Failed
                } else {
                    super::Download::Ready
                }
            });
            assert_eq!(skin, super::SkinLookup::Failed, "{failing:?} failed");
        }
        let bad_cape_url = super::Textures {
            cape: Some("no-scheme".into()),
            ..textures.clone()
        };
        assert_eq!(
            super::lookup(&bad_cape_url, true, |_, _| super::Download::Ready),
            super::SkinLookup::Failed
        );
    }

    #[test]
    fn cape_only_profiles_draw_the_uuid_default_body_and_model() {
        use super::{Download, Kind, SkinLookup, Textures, drawable_skin, lookup};
        let mut textures = Textures {
            cape: Some("https://textures.minecraft.net/texture/cape1".into()),
            signature: super::Signature::Signed,
            ..Textures::default()
        };
        let SkinLookup::Ready(resolved) = lookup(&textures, true, |kind, _| {
            assert_eq!(kind, Kind::Cape);
            Download::Ready
        }) else {
            panic!("the cape-only profile must resolve without a custom body");
        };
        let cape = super::cape_location("cape1");
        for (id, body, slim) in [
            (0, "entity/player/slim/alex", true),
            (9, "entity/player/wide/alex", false),
        ] {
            assert_eq!(
                drawable_skin(id, Some(&resolved), true, |sprite| sprite == cape.as_str()),
                (body, slim, Some(cape.as_str()))
            );
            assert_eq!(
                drawable_skin(id, Some(&resolved), true, |_| false),
                (body, slim, None)
            );
        }
        assert_eq!(
            lookup(&textures, true, |_, _| Download::Failed),
            SkinLookup::Failed
        );
        assert_eq!(
            drawable_skin(0, None, true, |_| unreachable!()),
            ("entity/player/slim/alex", true, None)
        );

        textures.elytra = Some("https://textures.minecraft.net/texture/wings1".into());
        assert_eq!(
            lookup(&textures, true, |kind, _| match kind {
                Kind::Elytra => Download::Pending,
                _ => Download::Ready,
            }),
            SkinLookup::Pending
        );
        assert_eq!(
            lookup(&textures, true, |kind, _| match kind {
                Kind::Elytra => Download::Failed,
                _ => Download::Ready,
            }),
            SkinLookup::Failed
        );
        let SkinLookup::Ready(resolved) = lookup(&textures, true, |_, _| Download::Ready) else {
            panic!("the completed cape and elytra must resolve together");
        };
        assert_eq!(resolved.cape.as_deref(), Some(cape.as_str()));
        assert_eq!(resolved.elytra, Some(super::location("elytra/", "wings1")));

        textures.cape = None;
        let SkinLookup::Ready(wings_only) = lookup(&textures, true, |_, _| Download::Ready) else {
            panic!("elytra without a cape or custom body must still resolve");
        };
        assert_eq!(wings_only.body(9), ("entity/player/wide/alex", false));
        assert!(wings_only.cape.is_none());
        assert_eq!(wings_only.elytra, resolved.elytra);
    }

    #[test]
    fn an_unsigned_cape_is_allowed_locally_but_not_on_a_remote_player() {
        use super::{Download, Signature, SkinLookup, TEXTURE_URIS, lookup, unpack};
        let value = "eyJ0ZXh0dXJlcyI6eyJDQVBFIjp7InVybCI6Imh0dHBzOi8vdGV4dHVyZXMubWluZWNyYWZ0Lm5ldC90ZXh0dXJlL2FiY2QxMjM0In19fQ==";
        let unsigned = unpack(value, Signature::Unsigned, &TEXTURE_URIS);
        let ready = |_, _: &str| Download::Ready;
        let SkinLookup::Ready(local) = lookup(&unsigned, false, ready) else {
            panic!("the local player doesn't require a signature");
        };
        assert_eq!(local.cape, Some(super::cape_location("abcd1234")));
        assert_eq!(lookup(&unsigned, true, ready), SkinLookup::Disallowed);
    }

    /// A 1x1 PNG of `rgba`.
    fn png(rgba: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&rgba).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn resource_reload_replaces_pixels_and_drops_queued_old_generation_images() {
        use sunrise_assets::resource_source::{Resource, ResourceSource};
        let source = |color| {
            ResourceSource::indexed(std::collections::BTreeMap::from([(
                "assets/example/textures/player.png".into(),
                Resource::bytes(png(color)),
            )]))
        };
        let cache = std::env::temp_dir().join(format!(
            "sunrise-resource-skin-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut downloads = super::Downloader::new(cache.clone());
        downloads.set_resource_root(source([255, 0, 0, 255]));
        downloads.resource("patch/player", "example", "player");
        assert_eq!(
            &downloads.image("patch/player").unwrap()[..4],
            [255, 0, 0, 255]
        );
        assert_eq!(downloads.ready.len(), 1);
        downloads.set_resource_root(source([0, 0, 255, 255]));
        assert!(downloads.image("patch/player").is_none());
        assert!(downloads.ready.is_empty());
        assert!(!downloads.queued.contains("patch/player"));
        downloads.resource("patch/player", "example", "player");
        let ready = downloads.take_ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(&ready[0].1[..4], [0, 0, 255, 255]);
        assert!(
            !cache.exists(),
            "resource images never enter the URL download cache"
        );
    }

    /// A downloaded body reaches the disk cache only if it decodes; a broken
    /// cached file is fetched again and replaced. A failed fetch writes nothing.
    #[test]
    fn the_texture_cache_holds_only_what_decodes() {
        let root =
            std::env::temp_dir().join(format!("sunrise-test-skin-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let body = png([7, 8, 9, 255]);
        let file = root.join("ab").join("abcdef");
        let load = |fetched: Option<&[u8]>| {
            let fetched = fetched.map(<[u8]>::to_vec);
            super::Downloader::load_texture(
                &root,
                super::Kind::Elytra,
                "abcdef",
                "https://example/texture/abc.png",
                |_| fetched.clone(),
            )
        };
        assert_eq!(load(Some(&body)), Some(vec![7, 8, 9, 255]));
        assert_eq!(std::fs::read(&file).unwrap(), body);
        // From then on the cache answers; no fetch is needed.
        assert_eq!(load(None), Some(vec![7, 8, 9, 255]));
        // A broken file is fetched again and replaced, not kept.
        std::fs::write(&file, b"<html>not a png</html>").unwrap();
        assert_eq!(load(Some(&body)), Some(vec![7, 8, 9, 255]));
        assert_eq!(std::fs::read(&file).unwrap(), body);
        // A body that does not decode is not cached at all.
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(load(Some(b"<html>not a png</html>")), None);
        assert!(!file.exists());
    }

    fn wait_state(
        d: &mut super::Downloader,
        kind: super::Kind,
        url: &str,
        want: super::Download,
    ) -> super::Download {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let state = d.state(kind, url);
            if state == want || std::time::Instant::now() > deadline {
                return state;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// A texture download that failed is tried again after the wait, instead of
    /// pinning the player to the default skin for the rest of the session.
    #[test]
    #[ignore = "daily"]
    fn a_failed_download_is_retried_after_the_wait() {
        use std::io::{BufRead, BufReader, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let body = png([1, 2, 3, 4]);
        let served = std::sync::Arc::new(AtomicUsize::new(0));
        let count = served.clone();
        std::thread::spawn(move || {
            for (i, stream) in listener.incoming().take(2).enumerate() {
                let mut stream = stream.unwrap();
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                reader.read_line(&mut line).unwrap();
                while reader.read_line(&mut String::new()).unwrap() > 2 {}
                count.fetch_add(1, Ordering::SeqCst);
                if i == 0 {
                    stream
                        .write_all(
                            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .unwrap();
                } else {
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(head.as_bytes()).unwrap();
                    stream.write_all(&body).unwrap();
                }
            }
        });
        let root =
            std::env::temp_dir().join(format!("sunrise-test-skin-retry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut d = super::Downloader::new(root);
        let url = format!("{base}/texture/retry.png");
        assert_eq!(d.state(super::Kind::Elytra, &url), super::Download::Pending);
        assert_eq!(
            wait_state(&mut d, super::Kind::Elytra, &url, super::Download::Failed),
            super::Download::Failed
        );
        // Within the wait the failure stands: no request goes out.
        assert_eq!(d.state(super::Kind::Elytra, &url), super::Download::Failed);
        assert_eq!(served.load(Ordering::SeqCst), 1);
        // After it, the download is started again and takes the good answer.
        d.retry_after = std::time::Duration::ZERO;
        assert_eq!(d.state(super::Kind::Elytra, &url), super::Download::Pending);
        assert_eq!(
            wait_state(&mut d, super::Kind::Elytra, &url, super::Download::Ready),
            super::Download::Ready
        );
        assert_eq!(served.load(Ordering::SeqCst), 2);
    }
}

#[cfg(test)]
mod download_bound_tests {
    use super::{Bounds, Download, Downloader, Kind};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn downloader(name: &str, bounds: Bounds, fetch: fn(&str) -> Option<Vec<u8>>) -> Downloader {
        let root =
            std::env::temp_dir().join(format!("sunrise-test-skin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut d = Downloader::new(root);
        d.fetch = fetch;
        d.bounds = bounds;
        d
    }

    fn url(i: usize) -> String {
        format!("https://textures.example/texture/skin{i:05}")
    }

    /// Asks again until the texture's download has settled.
    fn wait(d: &mut Downloader, url: &str) -> Download {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let state = d.state(Kind::Skin, url);
            if state != Download::Pending || std::time::Instant::now() > deadline {
                return state;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    static RUNNING: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);
    /// How many gated fetches may finish: the test, not the clock, decides when one ends. One gate
    /// per test, since tests run side by side.
    struct Gate(std::sync::Mutex<usize>, std::sync::Condvar);
    impl Gate {
        const fn new() -> Self {
            Self(std::sync::Mutex::new(0), std::sync::Condvar::new())
        }
        fn permit(&self, count: usize) {
            *self.0.lock().unwrap() += count;
            self.1.notify_all();
        }
        fn pass(&self) {
            *self
                .1
                .wait_while(self.0.lock().unwrap(), |p| *p == 0)
                .unwrap() -= 1;
        }
    }
    static THREADS: Gate = Gate::new();
    static DUE: Gate = Gate::new();

    /// A fetch that ends only when the test grants it a permit, and counts how many run at once.
    fn gated_failure(_: &str) -> Option<Vec<u8>> {
        let now = RUNNING.fetch_add(1, Ordering::SeqCst) + 1;
        PEAK.fetch_max(now, Ordering::SeqCst);
        THREADS.pass();
        RUNNING.fetch_sub(1, Ordering::SeqCst);
        None
    }

    /// The same, not counted: tests run side by side, and the count is another test's.
    fn sleepy_failure(_: &str) -> Option<Vec<u8>> {
        std::thread::sleep(std::time::Duration::from_millis(20));
        None
    }

    /// A tab list of 60 players names 60 textures at once: no more than the bound run together,
    /// a waiting one starts only once a running one has finished, and every one ends. Every frame
    /// asks again. No fetch ends before the test lets it, so nothing here depends on timing.
    #[test]
    fn downloads_run_a_few_at_a_time_and_the_late_ones_start_later() {
        let bounds = Bounds {
            states: 4096,
            resources: 4096,
            downloads: 4,
        };
        let mut d = downloader("threads", bounds, gated_failure);
        let urls: Vec<String> = (0..60).map(url).collect();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        // Asked all at once: four started, and the others were not even recorded.
        let states: Vec<Download> = urls.iter().map(|u| d.state(Kind::Skin, u)).collect();
        assert!(states.iter().all(|s| *s == Download::Pending));
        assert_eq!(d.in_flight, 4);
        assert_eq!(d.states.len(), 4);
        // One finishes: its place is free, and nothing new started on its own.
        THREADS.permit(1);
        while d.in_flight == 4 {
            assert!(
                std::time::Instant::now() < deadline,
                "the permitted fetch never ended"
            );
            d.state(Kind::Skin, &urls[0]);
            std::thread::yield_now();
        }
        assert_eq!(d.in_flight, 3);
        assert_eq!(d.states.len(), 4);
        // The next texture asked takes that place, and only that one.
        let states: Vec<Download> = urls.iter().map(|u| d.state(Kind::Skin, u)).collect();
        assert_eq!(d.in_flight, 4);
        assert_eq!(d.states.len(), 5);
        assert_eq!(states.iter().filter(|s| **s == Download::Failed).count(), 1);
        // The rest may finish: each frame starts the waiting ones as places come free.
        THREADS.permit(usize::MAX / 2);
        loop {
            let states: Vec<Download> = urls.iter().map(|u| d.state(Kind::Skin, u)).collect();
            if states.iter().all(|s| *s == Download::Failed) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{} still pending",
                states.iter().filter(|s| **s == Download::Pending).count()
            );
            std::thread::yield_now();
        }
        assert!(
            PEAK.load(Ordering::SeqCst) <= 4,
            "{}",
            PEAK.load(Ordering::SeqCst)
        );
        assert_eq!(d.states.len(), 60);
        assert_eq!(d.in_flight, 0);
    }

    /// More textures than `states` holds: the map keeps to its bound, the oldest settled ones go
    /// (and their failure time with them), and one that went is asked again as a new texture.
    #[test]
    fn the_states_keep_to_their_bound_and_drop_the_oldest() {
        let bounds = Bounds {
            states: 8,
            resources: 4096,
            downloads: 16,
        };
        let mut d = downloader("states", bounds, |_| None);
        for i in 0..40 {
            let u = url(i);
            assert_eq!(wait(&mut d, &u), Download::Failed, "{i}");
            assert!(d.states.len() <= 8 && d.state_order.len() == d.states.len());
            assert!(d.failed.len() <= 8);
        }
        let kept: Vec<usize> = (0..40)
            .filter(|i| {
                d.states
                    .contains_key(&(Kind::Skin, super::texture_hash(&url(*i)).unwrap()))
            })
            .collect();
        assert_eq!(kept, (32..40).collect::<Vec<_>>());
        // The first one went: asked again, it starts again (and fails again).
        assert_eq!(d.state(Kind::Skin, &url(0)), Download::Pending);
        assert_eq!(d.in_flight, 1);
    }

    /// A fetch that panics, as a decoder can on a body a server chose.
    fn panicking(_: &str) -> Option<Vec<u8>> {
        panic!("a fetch that panics, on purpose");
    }

    /// A thread that panics before it answers still gives its place back: with two places and six
    /// textures whose fetch panics, every one settles as failed instead of the places staying
    /// taken for good.
    #[test]
    fn a_download_that_panics_gives_its_place_back() {
        let bounds = Bounds {
            states: 4096,
            resources: 4096,
            downloads: 2,
        };
        let mut d = downloader("panics", bounds, panicking);
        for i in 0..6 {
            assert_eq!(wait(&mut d, &url(i)), Download::Failed, "{i}");
        }
        assert_eq!(d.in_flight, 0);
        assert_eq!(d.states.len(), 6);
    }

    /// A fetch that ends only when the test lets it, so the test asks between its start and end.
    fn due_failure(_: &str) -> Option<Vec<u8>> {
        DUE.pass();
        None
    }

    /// A failure whose wait is over is retried like a new texture: not while every place is taken
    /// (it keeps answering `Failed`, and starts nothing), and as soon as one is free.
    #[test]
    fn a_failure_that_is_due_waits_for_a_free_place() {
        let bounds = Bounds {
            states: 4096,
            resources: 4096,
            downloads: 1,
        };
        let mut d = downloader("due", bounds, due_failure);
        let wait_time = d.retry_after;
        DUE.permit(1);
        assert_eq!(wait(&mut d, &url(0)), Download::Failed);
        // Another texture takes the only place; the first one's retry becomes due but has to wait.
        assert_eq!(d.state(Kind::Skin, &url(1)), Download::Pending);
        d.retry_after = std::time::Duration::ZERO;
        assert_eq!(d.state(Kind::Skin, &url(0)), Download::Failed);
        assert_eq!(d.in_flight, 1);
        d.retry_after = wait_time;
        DUE.permit(1);
        assert_eq!(wait(&mut d, &url(1)), Download::Failed);
        // The place is free: the retry starts.
        d.retry_after = std::time::Duration::ZERO;
        assert_eq!(d.state(Kind::Skin, &url(0)), Download::Pending);
        assert_eq!(d.in_flight, 1);
        DUE.permit(1);
    }

    /// A texture that is running is never the one dropped, however many settle behind it.
    #[test]
    fn a_running_download_keeps_its_place_in_the_states() {
        let bounds = Bounds {
            states: 2,
            resources: 4096,
            downloads: 16,
        };
        let mut d = downloader("running", bounds, sleepy_failure);
        assert_eq!(d.state(Kind::Skin, &url(0)), Download::Pending);
        for i in 1..6 {
            d.set_state((Kind::Cape, format!("settled{i}")), Download::Ready);
        }
        assert_eq!(
            d.states
                .get(&(Kind::Skin, super::texture_hash(&url(0)).unwrap())),
            Some(&Download::Pending)
        );
        assert!(d.states.len() <= 3);
        assert_eq!(wait(&mut d, &url(0)), Download::Failed);
    }

    /// `resources` keeps to its bound too: a server's `texture` ids, one for each unique one, no
    /// longer pile up, and one that was dropped is read from the packs again.
    #[test]
    fn the_resource_textures_keep_to_their_bound() {
        let bounds = Bounds {
            states: 4096,
            resources: 8,
            downloads: 16,
        };
        let mut d = downloader("resources", bounds, |_| None);
        for i in 0..40 {
            d.resource(&format!("patch/n/{i}"), "n", &format!("t{i}"));
            assert!(d.resources.len() <= 8 && d.resource_order.len() == d.resources.len());
        }
        let kept: Vec<usize> = (0..40)
            .filter(|i| d.resources.contains_key(&format!("patch/n/{i}")))
            .collect();
        assert_eq!(kept, (32..40).collect::<Vec<_>>());
        // Forgetting one by hand (as `again` does) leaves no stale key behind.
        d.forget_resource("patch/n/35");
        assert_eq!(d.resource_order.len(), d.resources.len());
        assert!(!d.resource_order.iter().any(|l| l == "patch/n/35"));
        // New packs start the count over.
        d.set_resource_root(sunrise_assets::resource_source::ResourceSource::directory(
            std::env::temp_dir(),
        ));
        assert!(d.resources.is_empty() && d.resource_order.is_empty());
    }
}

#[cfg(test)]
mod face_skin_tests {
    use super::{
        Download, Downloader, Kind, NameOrId, ProfileAnswer, ResolvedProfile, Resolver, face_skin,
        skin_location,
    };
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    /// `GuiCanvas::player_face`'s skin on a fake backend that never reaches the services: it
    /// records each lookup the resolver starts, and the test answers it. A listed player's skin
    /// wins and starts no lookup; an unlisted UUID starts one lookup however often it is drawn,
    /// and draws its default while the lookup and the download are pending and after the
    /// download until the upload, then its own skin. Every call answers at once.
    #[test]
    fn a_face_draws_the_listed_skin_else_the_default_until_its_one_lookup_is_uploaded() {
        let (listed, offline) = (
            0x0123_4567_89ab_cdef_0123_4567_89ab_cdef_u128,
            0x853c_80ef_3c37_49fd_aa49_938b_674a_dae6_u128,
        );
        let started: Arc<Mutex<Vec<(NameOrId, ProfileAnswer)>>> = Arc::default();
        let mut resolver = Resolver::with_lookup({
            let started = started.clone();
            move |key, answer| started.lock().unwrap().push((key, answer))
        });
        let mut downloads =
            Downloader::new(std::env::temp_dir().join("sunrise-test-face-skins-absent"));
        let mut uploaded = HashSet::new();
        let mut name = String::new();
        let skin = face_skin(
            listed,
            Some("skins/listed"),
            &mut resolver,
            &mut downloads,
            |l| uploaded.contains(l),
            &mut name,
        );
        assert_eq!(skin, "skins/listed");
        assert!(
            started.lock().unwrap().is_empty(),
            "a listed face looked up a profile"
        );

        let default = crate::tab_list::default_skin(offline);
        let face =
            |resolver: &mut Resolver, downloads: &mut Downloader, uploaded: &HashSet<String>| {
                let mut name = String::new();
                face_skin(
                    offline,
                    None,
                    resolver,
                    downloads,
                    |l| uploaded.contains(l),
                    &mut name,
                )
                .to_owned()
            };
        assert_eq!(face(&mut resolver, &mut downloads, &uploaded), default);
        assert_eq!(face(&mut resolver, &mut downloads, &uploaded), default);
        let (key, answer) = {
            let started = started.lock().unwrap();
            let keys: Vec<_> = started.iter().map(|(key, _)| key.clone()).collect();
            assert_eq!(keys, [NameOrId::Id(offline)], "one lookup, by id");
            started[0].clone()
        };

        // The services answer: a signed `textures` property naming texture `abc123`.
        let value = "eyJ0ZXh0dXJlcyI6IHsiU0tJTiI6IHsidXJsIjogImh0dHBzOi8vdGV4dHVyZXMubWluZWNyYWZ0Lm5ldC90ZXh0dXJlL2FiYzEyMyJ9fX0=";
        let profile = ResolvedProfile {
            id: offline,
            name: "Offline".into(),
            properties: vec![("textures".into(), value.into(), Some("sig".into()))],
        };
        answer.send((key, Some(profile))).unwrap();
        let download = (Kind::Skin, "abc123".to_owned());
        downloads.set_state(download.clone(), Download::Pending);
        assert_eq!(face(&mut resolver, &mut downloads, &uploaded), default);
        downloads.set_state(download, Download::Ready);
        assert_eq!(face(&mut resolver, &mut downloads, &uploaded), default);
        let location = skin_location("abc123");
        uploaded.insert(location.clone());
        assert_eq!(face(&mut resolver, &mut downloads, &uploaded), location);
        assert!(downloads.take_drawn().contains(&location));
        assert_eq!(
            started.lock().unwrap().len(),
            1,
            "a later face started a lookup"
        );
    }
}

#[cfg(test)]
mod skin_cache_tests {
    use super::{Downloader, PackedTextures, SKIN_TTL, Signature, Textures};
    use base64::Engine as _;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An unsigned property naming a skin.
    fn packed(profile: u128) -> PackedTextures {
        let payload = serde_json::json!({"textures": {"SKIN": {
            "url": format!("https://textures.minecraft.net/texture/{profile:x}"),
        }}});
        PackedTextures {
            profile,
            value: base64::engine::general_purpose::STANDARD.encode(payload.to_string()),
            signature: None,
        }
    }

    /// Asks until the thread that unpacks `packed` answers.
    fn settled(d: &mut Downloader, packed: &PackedTextures) -> std::sync::Arc<Textures> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(textures) = d.unpacked(packed, &[]) {
                return textures;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the property was never unpacked"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// The first ask starts a thread and reads nothing. The property is then answered at once,
    /// for every holder of it, until it has gone unasked for the TTL: dropped, it is unpacked
    /// again. Another profile's property is its own entry.
    #[test]
    fn a_property_is_unpacked_once_until_no_longer_asked_for() {
        let mut d = Downloader::new(std::env::temp_dir());
        let asked = packed(1);
        assert!(
            d.unpacked(&asked, &[]).is_none(),
            "the first ask only starts the thread"
        );
        assert_eq!(d.skin_cache.running, 1);
        let textures = settled(&mut d, &asked);
        assert_eq!(textures.signature, Signature::Unsigned);
        assert_eq!(
            textures.skin.as_deref(),
            Some("https://textures.minecraft.net/texture/1")
        );
        let again = d.unpacked(&asked, &[]).unwrap();
        assert!(std::sync::Arc::ptr_eq(&again, &textures));
        assert!(d.unpacked(&packed(2), &[]).is_none(), "another profile");
        // Idle past the TTL, on the sweep's own cadence.
        d.skin_cache.entries.get_mut(&asked).unwrap().touched -= SKIN_TTL;
        d.skin_cache.swept -= SKIN_TTL;
        assert!(d.unpacked(&asked, &[]).is_none());
        assert!(!std::sync::Arc::ptr_eq(&settled(&mut d, &asked), &textures));
    }

    /// `forget_unpacked` ends every verdict made with the keys it was made with: a property
    /// asked again is unpacked again, while a thread still running for the old keys has no
    /// reader left.
    #[test]
    fn forgotten_properties_are_unpacked_again() {
        let mut d = Downloader::new(std::env::temp_dir());
        let asked = packed(1);
        assert!(d.unpacked(&asked, &[]).is_none());
        let textures = settled(&mut d, &asked);
        assert!(d.unpacked(&packed(2), &[]).is_none());
        d.forget_unpacked();
        assert_eq!(
            d.skin_cache.running, 0,
            "the old thread is no longer counted"
        );
        assert!(d.unpacked(&asked, &[]).is_none());
        assert!(!std::sync::Arc::ptr_eq(&settled(&mut d, &asked), &textures));
    }

    /// How many held unpacks may end: the test, not the clock, decides when one does. One gate
    /// for these tests, since they run side by side.
    static PERMITS: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
    static RELEASED: std::sync::Condvar = std::sync::Condvar::new();
    static HELD: AtomicUsize = AtomicUsize::new(0);

    /// An unpack that ends only when the test permits it.
    fn held(packed: &PackedTextures, keys: &[sunrise_protocol::rsa::RsaPublicKey]) -> Textures {
        HELD.fetch_add(1, Ordering::SeqCst);
        let mut permits = PERMITS.lock().unwrap();
        while *permits == 0 {
            permits = RELEASED.wait(permits).unwrap();
        }
        *permits -= 1;
        drop(permits);
        HELD.fetch_sub(1, Ordering::SeqCst);
        packed.unpack(keys)
    }

    /// As many threads unpack as vanilla's background executor has workers. A tab list of five
    /// players is asked all at once, every frame: two unpack, the others start only once those
    /// have answered, and every one ends. No unpack ends before the test lets it, so nothing
    /// here depends on timing.
    #[test]
    fn unpacking_runs_a_few_at_a_time_and_the_late_ones_start_later() {
        let mut d = Downloader::new(std::env::temp_dir());
        d.skin_cache.threads = 2;
        d.skin_cache.pack = held;
        let asked: Vec<_> = (0..5u128).map(packed).collect();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut frames = 0;
        while !asked.iter().all(|p| d.unpacked(p, &[]).is_some()) {
            frames += 1;
            assert!(
                std::time::Instant::now() < deadline,
                "the five properties never unpacked"
            );
            assert!(d.skin_cache.running <= 2, "no more than two unpack at once");
            assert!(
                d.skin_cache.entries.len() <= 2 * frames,
                "a property asked while both threads are busy starts nothing"
            );
            // Every thread started is held at the gate (asking again takes the answers sent):
            // let those end, then ask again.
            while HELD.load(Ordering::SeqCst) != d.skin_cache.running {
                assert!(std::time::Instant::now() < deadline);
                d.unpacked(&asked[0], &[]);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            *PERMITS.lock().unwrap() += d.skin_cache.running;
            RELEASED.notify_all();
        }
        assert!(frames >= 3, "the last three waited for a free thread");
        assert_eq!(HELD.load(Ordering::SeqCst), 0);
    }

    fn panicking(_: &PackedTextures, _: &[sunrise_protocol::rsa::RsaPublicKey]) -> Textures {
        panic!("a property that breaks the unpacker")
    }

    /// A thread that panics before answering leaves the default skin and gives its place back,
    /// as the future's `handle` does.
    #[test]
    fn an_unpack_that_panics_leaves_the_default_skin() {
        let mut d = Downloader::new(std::env::temp_dir());
        d.skin_cache.threads = 1;
        d.skin_cache.pack = panicking;
        let asked = packed(1);
        assert!(d.unpacked(&asked, &[]).is_none());
        assert_eq!(*settled(&mut d, &asked), Textures::default());
        assert_eq!(d.skin_cache.running, 0, "its place is free again");
        d.skin_cache.pack = PackedTextures::unpack;
        assert!(d.unpacked(&packed(2), &[]).is_none());
        assert_eq!(d.skin_cache.running, 1);
        assert!(settled(&mut d, &packed(2)).skin.is_some());
    }
}
