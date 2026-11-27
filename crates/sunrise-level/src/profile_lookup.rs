//! Portable player profile codecs, NBT helpers, default skins, and `ProfileResolver` seam.
use crate::scoreboard::Comp;

/// `Either<String, UUID>`: the key a `Dynamic` `ResolvableProfile` resolves
/// by (`ResolvableProfile.nameOrId`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NameOrId {
    Name(String),
    Id(u128),
}

/// `GameType.SPECTATOR` id.
pub const SPECTATOR: i32 = 3;

/// One `PlayerInfo` the list can show.
#[derive(Clone, Debug, PartialEq)]
pub struct TabPlayer {
    pub uuid: u128,
    /// `GameProfile.name`: the score holder and team member name.
    pub name: String,
    /// `GameType` id (0 survival, 1 creative, 2 adventure, 3 spectator).
    pub game_mode: i32,
    /// Milliseconds; negative when unknown.
    pub latency: i32,
    /// `tabListDisplayName` as a component (its root style matters: a
    /// spectator's name is made italic at the root).
    pub display_name: Option<Comp>,
    pub list_order: i32,
    pub show_hat: bool,
    /// Skin texture path, e.g. `entity/player/wide/steve`.
    pub skin: String,
}
impl TabPlayer {
    /// A player with the default skin for their UUID.
    pub fn new(uuid: u128, name: impl Into<String>) -> Self {
        Self {
            uuid,
            name: name.into(),
            game_mode: 0,
            latency: 0,
            display_name: None,
            list_order: 0,
            show_hat: true,
            skin: default_skin(uuid).to_owned(),
        }
    }
}

/// `DefaultPlayerSkin.get(UUID)`: the default skin by `UUID.hashCode`.
pub fn default_skin(uuid: u128) -> &'static str {
    const SKINS: [&str; 18] = [
        "entity/player/slim/alex",
        "entity/player/slim/ari",
        "entity/player/slim/efe",
        "entity/player/slim/kai",
        "entity/player/slim/makena",
        "entity/player/slim/noor",
        "entity/player/slim/steve",
        "entity/player/slim/sunny",
        "entity/player/slim/zuri",
        "entity/player/wide/alex",
        "entity/player/wide/ari",
        "entity/player/wide/efe",
        "entity/player/wide/kai",
        "entity/player/wide/makena",
        "entity/player/wide/noor",
        "entity/player/wide/steve",
        "entity/player/wide/sunny",
        "entity/player/wide/zuri",
    ];
    let hilo = ((uuid >> 64) as u64) ^ (uuid as u64);
    let hash = ((hilo >> 32) as i32) ^ (hilo as i32);
    SKINS[hash.rem_euclid(SKINS.len() as i32) as usize]
}

/// `Property.hasSignature` and the services key check, as authlib's
/// `SignatureState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Signature {
    #[default]
    Unsigned,
    Signed,
    Invalid,
}

/// `MinecraftServicesSessionService.getPropertySignatureState`: verify the raw
/// property value, not its decoded JSON, using only `PROFILE_PROPERTY` keys.
pub fn property_signature(
    value: &str,
    signature: Option<&str>,
    profile_property_keys: &[sunrise_protocol::rsa::RsaPublicKey],
) -> Signature {
    let Some(signature) = signature else {
        return Signature::Unsigned;
    };
    let Some(signature) = base64_decode(signature) else {
        return Signature::Invalid;
    };
    if profile_property_keys.iter().any(|key| {
        key.verify(
            sunrise_protocol::rsa::SignatureHash::Sha1,
            value.as_bytes(),
            &signature,
        )
    }) {
        Signature::Signed
    } else {
        Signature::Invalid
    }
}

/// `MinecraftProfileTextures`: the texture URLs a profile names. Empty when
/// the payload doesn't decode or names a URL outside the allowed domains.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Textures {
    pub skin: Option<String>,
    /// The skin's `metadata.model` is `slim` (`PlayerModelType.SLIM`).
    pub slim: bool,
    pub cape: Option<String>,
    pub elytra: Option<String>,
    pub signature: Signature,
}

/// The `getTexture` endpoint's valid URIs in production's services discovery
/// document (`MinecraftServicesEnvironment.PROD`).
pub const TEXTURE_URIS: [&str; 2] = [
    "https://textures.minecraft.net/texture/{textureId}",
    "http://textures.minecraft.net/texture/{textureId}",
];

/// The `profiles` / `getTexture` endpoint's `validUris` in a discovery
/// document, when it lists any.
pub fn discovery_texture_uris(doc: &serde_json::Value) -> Option<Vec<String>> {
    let uris: Vec<String> = doc
        .pointer("/discovery/profiles/endpoints/getTexture/validUris")?
        .as_array()?
        .iter()
        .filter_map(|u| u.as_str().map(str::to_owned))
        .collect();
    (!uris.is_empty()).then_some(uris)
}

/// `java.util.Base64.getDecoder().decode`.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    use base64::{
        Engine, alphabet,
        engine::{
            DecodePaddingMode,
            general_purpose::{GeneralPurpose, GeneralPurposeConfig},
        },
    };

    // Java's basic decoder allows nonzero unused bits and missing padding,
    // but if an '=' is present the padding quantum must be complete.
    const CONFIG: GeneralPurposeConfig =
        GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true);
    const PADDED: GeneralPurpose = GeneralPurpose::new(&alphabet::STANDARD, CONFIG);
    const UNPADDED: GeneralPurpose = GeneralPurpose::new(
        &alphabet::STANDARD,
        CONFIG.with_decode_padding_mode(DecodePaddingMode::RequireNone),
    );
    let engine = if text.ends_with('=') {
        &PADDED
    } else {
        &UNPADDED
    };
    engine.decode(text).ok()
}

/// `unpackTextures(property)`: the textures of a `textures` property's
/// base64 JSON payload.
pub fn unpack(value: &str, signature: Signature, valid_uris: &[impl AsRef<str>]) -> Textures {
    let empty = Textures::default();
    let Some(json) = base64_decode(value)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    else {
        return empty;
    };
    let Some(textures) = json.get("textures").and_then(|t| t.as_object()) else {
        return empty;
    };
    if textures.is_empty() {
        return empty;
    }
    let mut out = Textures {
        signature,
        ..empty.clone()
    };
    // Authlib's MinecraftServicesSessionService validates every URL in
    // unpackTextures before returning any textures. A bad cape rejects the skin too.
    for (kind, texture) in textures {
        let Some(url) = texture.get("url").and_then(|u| u.as_str()) else {
            return empty;
        };
        let allowed = valid_uris
            .iter()
            .any(|u| url.starts_with(&u.as_ref().replace("{textureId}", "")));
        if !allowed {
            return empty;
        }
        if kind == "SKIN" {
            out.slim = texture
                .get("metadata")
                .and_then(|m| m.get("model"))
                .and_then(|m| m.as_str())
                == Some("slim");
        }
        let slot = match kind.as_str() {
            "SKIN" => &mut out.skin,
            "CAPE" => &mut out.cape,
            "ELYTRA" => &mut out.elytra,
            _ => continue,
        };
        *slot = Some(url.to_owned());
    }
    out
}

/// `profile_skin_url` on a block entity's `profile` NBT (`SkullBlockEntity`):
/// the same `properties` list, held as a compound. A name-only profile (a
/// string tag) has no properties and no skin URL.
pub fn profile_skin_url_nbt(
    profile: &sunrise_core::nbt::Tag,
    valid_uris: &[impl AsRef<str>],
) -> Option<String> {
    let properties = match profile.get("properties")? {
        sunrise_core::nbt::Tag::List(list) => list,
        _ => return None,
    };
    let value = properties.iter().find_map(|p| {
        let name = nbt_text(p.get("name"))?;
        (name == "textures")
            .then(|| nbt_text(p.get("value")))
            .flatten()
    })?;
    unpack(&value, Signature::Unsigned, valid_uris).skin
}

/// `ResolvableProfile.skinPatch().body()` of a skull's `profile` NBT: the
/// `texture` id that replaces its skin.
pub fn profile_texture_nbt(profile: &sunrise_core::nbt::Tag) -> Option<String> {
    nbt_text(profile.get("texture"))
}

/// The profile's uuid as u128: `id` as an int array (block entity NBT) or a
/// uuid string (older and component forms). Used for `DefaultPlayerSkin.get`
/// when a head's skin is not downloaded yet.
pub fn profile_uuid_nbt(profile: &sunrise_core::nbt::Tag) -> Option<u128> {
    match profile.get("id")? {
        sunrise_core::nbt::Tag::IntArray(v) if v.len() == 4 => Some(
            v.iter()
                .fold(0u128, |a, &w| (a << 32) | u128::from(w as u32)),
        ),
        sunrise_core::nbt::Tag::String(s) => {
            let digits: String = s.to_string_lossy().chars().filter(|c| *c != '-').collect();
            u128::from_str_radix(&digits, 16).ok()
        }
        _ => None,
    }
}

pub fn nbt_text(tag: Option<&sunrise_core::nbt::Tag>) -> Option<String> {
    match tag {
        Some(sunrise_core::nbt::Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// `GameProfile` as `fetchProfile` returns it: the account's id, name and
/// property map (its `textures` property is the skin).
#[derive(Clone, Debug)]
pub struct ResolvedProfile {
    pub id: u128,
    pub name: String,
    /// `(name, value, signature)` properties.
    pub properties: Vec<(String, String, Option<String>)>,
}
impl ResolvedProfile {
    /// The `textures` property's value, for consumers that don't require a signed profile.
    pub fn textures(&self) -> Option<&str> {
        self.properties
            .iter()
            .find(|(n, _, _)| n == "textures")
            .map(|(_, value, _)| value.as_str())
    }
}

/// Where a `Dynamic` profile's resolution stands (`resolveProfile`'s future).
#[derive(Clone, Debug)]
pub enum Resolve {
    Pending,
    /// The services' answer: `None` for no profile or no answer, which the
    /// cache holds like any result (`Optional.empty`).
    Done(Option<ResolvedProfile>),
}

/// Portable profile lookup seam (`ProfileResolver.Cached`) for server commands and modding.
pub trait ProfileResolver: Send + Sync {
    fn lookup(&self, query: &NameOrId) -> Resolve;
}

/// `StringUtil.isValidPlayerName` (the check `fetchByName` makes first).
pub fn valid_player_name(name: &str) -> bool {
    name.encode_utf16().count() <= 16 && name.encode_utf16().all(|c| c > 32 && c < 127)
}

/// A UUID with or without dashes.
pub fn parse_uuid(text: &str) -> Option<u128> {
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    (hex.len() == 32)
        .then(|| u128::from_str_radix(&hex, 16).ok())
        .flatten()
}

/// `ResolvableProfile` (an item's `minecraft:profile` component, a skull's
/// `profile` tag): the `textures` property it carries, the key a `Dynamic`
/// profile resolves by, and `partialProfile.id` — the fallback skin's key.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileInfo {
    /// `GameProfile.id` (`DefaultPlayerSkin.get`).
    pub id: u128,
    /// The `textures` property's value, for the insecure head lookup.
    pub textures: Option<String>,
    /// `Dynamic` profiles resolve by name or id.
    pub dynamic: Option<NameOrId>,
}

/// A profile's `id`: `UUIDUtil.CODEC` (an int array of four, or a UUID
/// string).
fn profile_id(v: &serde_json::Value) -> Option<u128> {
    match v {
        serde_json::Value::Array(a) => {
            let ints: Option<Vec<i64>> = a.iter().map(|x| x.as_i64()).collect();
            let i = ints?;
            (i.len() == 4).then(|| {
                ((i[0] as u32 as u128) << 96)
                    | ((i[1] as u32 as u128) << 64)
                    | ((i[2] as u32 as u128) << 32)
                    | i[3] as u32 as u128
            })
        }
        serde_json::Value::String(s) => parse_uuid(s),
        _ => None,
    }
}

/// `ResolvableProfile`'s read of a `profile` value: a bare name is
/// `createUnresolved` (the `PLAYER_NAME` codec's alternative), a compound
/// `name` + `id` + `properties` (`Partial.MAP_CODEC` or a `GameProfile`).
pub fn parse_profile(v: &serde_json::Value) -> ProfileInfo {
    if let serde_json::Value::String(name) = v {
        return ProfileInfo {
            id: sunrise_protocol::crypto::offline_uuid(name),
            textures: None,
            dynamic: valid_player_name(name).then(|| NameOrId::Name(name.clone())),
        };
    }
    let name = v.get("name").and_then(|n| n.as_str()).map(str::to_owned);
    let id = v.get("id").and_then(profile_id);
    let properties = v.get("properties").and_then(|p| p.as_array());
    let textures = properties
        .into_iter()
        .flatten()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("textures"))
        .and_then(|p| p.get("value")?.as_str().map(str::to_owned));
    // `ResolvableProfile.create`: a Partial is Dynamic with no properties
    // and exactly one of a name or an id.
    let dynamic =
        (properties.is_none_or(|p| p.is_empty()) && name.is_some() != id.is_some()).then(|| {
            name.as_deref().map_or_else(
                || NameOrId::Id(id.unwrap()),
                |n| NameOrId::Name(n.to_owned()),
            )
        });
    ProfileInfo {
        // `createPartialProfile`: the id, else the name's offline UUID, else nil.
        id: id
            .or_else(|| name.as_deref().map(sunrise_protocol::crypto::offline_uuid))
            .unwrap_or(0),
        textures,
        dynamic,
    }
}

/// `ResolvableProfile.create`'s `Dynamic` key from a skull's `profile` tag:
/// a bare name string is `createUnresolved`, and a compound without
/// properties and with exactly one of a name or an id is `Dynamic`.
pub fn dynamic_nbt(profile: &sunrise_core::nbt::Tag) -> Option<NameOrId> {
    if let sunrise_core::nbt::Tag::String(s) = profile {
        let name = s.to_string_lossy();
        return valid_player_name(&name).then_some(NameOrId::Name(name));
    }
    if matches!(profile.get("properties"), Some(sunrise_core::nbt::Tag::List(l)) if !l.is_empty()) {
        return None;
    }
    match (nbt_text(profile.get("name")), profile_uuid_nbt(profile)) {
        (Some(name), None) => Some(NameOrId::Name(name)),
        (None, Some(id)) => Some(NameOrId::Id(id)),
        _ => None,
    }
}

/// `MinecraftProfileTexture.getHash`: the base name of the URL's path.
pub fn texture_hash(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let path = &rest[rest.find('/').unwrap_or(rest.len())..];
    let path = &path[..path.find(['?', '#']).unwrap_or(path.len())];
    let name = &path[path.rfind(['/', '\\']).map_or(0, |i| i + 1)..];
    Some(name[..name.rfind('.').unwrap_or(name.len())].to_owned())
}

/// `SkinManager.TextureCache.registerTexture`: the texture id of a skin,
/// `skins/` and the SHA-1 of its hash's UTF-16 code units.
pub fn skin_location(hash: &str) -> String {
    location("skins/", hash)
}
/// `registerTexture` in the cape cache: `capes/` and the same SHA-1.
pub fn cape_location(hash: &str) -> String {
    location("capes/", hash)
}
pub fn location(prefix: &str, hash: &str) -> String {
    let units: Vec<u8> = hash.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let digest = sha1(&units);
    let mut out = String::from(prefix);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// SHA-1 (FIPS 180-4).
pub fn sha1(data: &[u8]) -> [u8; 20] {
    use sha1::{Digest, Sha1};
    Sha1::digest(data).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `MinecraftServicesDiscoveryService`: another environment's document
    /// names its own texture host; one that lists none names nothing.
    #[test]
    fn texture_uris_come_from_the_discovery_document() {
        use super::{Signature, discovery_texture_uris, unpack};
        let doc = serde_json::json!({"discovery": {"profiles": {"endpoints": {"getTexture": {
            "validUris": ["https://skins.example.org/t/{textureId}"]
        }}}}});
        let uris = discovery_texture_uris(&doc).unwrap();
        assert_eq!(uris, ["https://skins.example.org/t/{textureId}"]);
        // {"textures":{"SKIN":{"url":"https://skins.example.org/t/abc"}}}
        let ours =
            "eyJ0ZXh0dXJlcyI6eyJTS0lOIjp7InVybCI6Imh0dHBzOi8vc2tpbnMuZXhhbXBsZS5vcmcvdC9hYmMifX19";
        assert_eq!(
            unpack(ours, Signature::Signed, &uris).skin.as_deref(),
            Some("https://skins.example.org/t/abc")
        );
        assert_eq!(
            unpack(ours, Signature::Signed, &super::TEXTURE_URIS).skin,
            None
        );
        let none = serde_json::json!({"discovery": {"profiles": {"endpoints": {}}}});
        assert_eq!(discovery_texture_uris(&none), None);
        let empty = serde_json::json!({"discovery": {"profiles": {"endpoints": {"getTexture": {"validUris": []}}}}});
        assert_eq!(discovery_texture_uris(&empty), None);
    }

    /// `MinecraftProfileTexture.getMetadata("model")`: `slim` marks a slim
    /// skin; no metadata is the wide (classic) model.
    #[test]
    fn a_skins_model_comes_from_its_metadata() {
        use super::{Signature, TEXTURE_URIS, unpack};
        let slim = "eyJ0ZXh0dXJlcyI6eyJTS0lOIjp7InVybCI6Imh0dHBzOi8vdGV4dHVyZXMubWluZWNyYWZ0Lm5ldC90ZXh0dXJlL2FiYyIsIm1ldGFkYXRhIjp7Im1vZGVsIjoic2xpbSJ9fX19";
        let wide = "eyJ0ZXh0dXJlcyI6eyJTS0lOIjp7InVybCI6Imh0dHBzOi8vdGV4dHVyZXMubWluZWNyYWZ0Lm5ldC90ZXh0dXJlL2FiYyJ9fX0=";
        let t = unpack(slim, Signature::Signed, &TEXTURE_URIS);
        assert_eq!(
            t.skin.as_deref(),
            Some("https://textures.minecraft.net/texture/abc")
        );
        assert!(t.slim);
        assert!(!unpack(wide, Signature::Signed, &TEXTURE_URIS).slim);
    }
    /// Authlib's `MinecraftServicesSessionService` rejects the whole profile in
    /// `unpackTextures` when any texture URL is outside the allowed origins.
    #[test]
    fn unpack_rejects_the_skin_when_another_texture_is_rejected() {
        use super::{Signature, TEXTURE_URIS, unpack};
        let mixed = "eyJ0ZXh0dXJlcyI6IHsiU0tJTiI6IHsidXJsIjogImh0dHBzOi8vdGV4dHVyZXMubWluZWNyYWZ0Lm5ldC90ZXh0dXJlL3NraW4xIn0sICJDQVBFIjogeyJ1cmwiOiAiaHR0cHM6Ly9ldmlsLmV4YW1wbGUvdGV4dHVyZS9jYXBlMSJ9fX0=";
        let t = unpack(mixed, Signature::Unsigned, &TEXTURE_URIS);
        assert_eq!(t, super::Textures::default());
    }
    #[test]
    fn profiles_parse_like_resolvable_profiles() {
        use super::{NameOrId, parse_profile};
        // A bare name is `createUnresolved`: dynamic by name, and its partial
        // id is the name's offline UUID.
        let p = parse_profile(&serde_json::json!("Notch"));
        assert_eq!(p.dynamic, Some(NameOrId::Name("Notch".into())));
        assert_eq!(p.id, sunrise_protocol::crypto::offline_uuid("Notch"));
        assert!(p.textures.is_none());
        // A name-only compound resolves the same way.
        let p = parse_profile(&serde_json::json!({"name": "Notch"}));
        assert_eq!(p.dynamic, Some(NameOrId::Name("Notch".into())));
        // An id-only compound resolves by id (int array or UUID string).
        let id = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef_u128;
        let ints: Vec<i64> = [
            (id >> 96) as i32 as i64,
            (id >> 64) as i32 as i64,
            (id >> 32) as i32 as i64,
            id as i32 as i64,
        ]
        .into();
        let p = parse_profile(&serde_json::json!({"id": ints}));
        assert_eq!(p.dynamic, Some(NameOrId::Id(id)));
        let p = parse_profile(&serde_json::json!({"id": "01234567-89ab-cdef-0123-4567abcdef00"}));
        assert_eq!(
            p.dynamic,
            Some(NameOrId::Id(0x0123_4567_89ab_cdef_0123_4567_abcd_ef00))
        );
        // Name and id but no properties is Static: it never resolves, and its
        // default skin keys off the id.
        let p = parse_profile(&serde_json::json!({"name": "Notch", "id": ints}));
        assert!(p.dynamic.is_none());
        assert_eq!(p.id, id);
        // A `textures` property makes it Static (unsigned and signed).
        let p = parse_profile(&serde_json::json!({
            "properties": [{"name": "textures", "value": "v"}]
        }));
        assert!(p.dynamic.is_none());
        assert_eq!(p.textures.as_deref(), Some("v"));
        let p = parse_profile(&serde_json::json!({
            "properties": [{"name": "textures", "value": "v", "signature": "s"}]
        }));
        assert_eq!(p.textures.as_deref(), Some("v"));
        // Empty properties leave it dynamic.
        let p = parse_profile(&serde_json::json!({"name": "Notch", "properties": []}));
        assert_eq!(p.dynamic, Some(NameOrId::Name("Notch".into())));
    }

    #[test]
    fn skull_block_entity_profiles_supply_head_skins() {
        let value = "eyJ0ZXh0dXJlcyI6IHsiU0tJTiI6IHsidXJsIjogImh0dHBzOi8vdGV4dHVyZXMubWluZWNyYWZ0Lm5ldC90ZXh0dXJlL2FiYzEyMyJ9fX0=";
        let profile = sunrise_core::nbt::parse_snbt(&format!(
            r#"{{name:"T",id:[I;-158327916,1572358581,-2125825222,-487948715],properties:[{{name:"textures",value:"{value}"}}]}}"#
        ))
        .unwrap()
        .0;
        assert_eq!(
            super::profile_skin_url_nbt(&profile, &TEXTURE_URIS).as_deref(),
            Some("https://textures.minecraft.net/texture/abc123")
        );
        assert_eq!(
            super::profile_uuid_nbt(&profile),
            Some(
                [-158327916, 1572358581, -2125825222, -487948715]
                    .iter()
                    .fold(0u128, |a, &w: &i32| (a << 32) | u128::from(w as u32))
            )
        );
        // A name-only profile (a string tag) has no properties: no url, no id.
        let named = sunrise_core::nbt::Tag::String("MHF_Blaze".into());
        assert_eq!(super::profile_skin_url_nbt(&named, &TEXTURE_URIS), None);
        assert_eq!(super::profile_uuid_nbt(&named), None);
        // A uuid in string form decodes too.
        let text_id = sunrise_core::nbt::parse_snbt(
            r#"{name:"T",id:"e843a1f4-0000-0001-0000-000200000003"}"#,
        )
        .unwrap()
        .0;
        assert_eq!(
            super::profile_uuid_nbt(&text_id),
            Some(u128::from_str_radix("e843a1f4000000010000000200000003", 16).unwrap())
        );
    }

    #[test]
    fn sha1_and_base64() {
        let hex: String = sha1(b"abc").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(base64_decode("aGk=").unwrap(), b"hi");
        assert_eq!(base64_decode("aGk").unwrap(), b"hi");
        assert!(base64_decode("a").is_none());
        assert!(base64_decode("aGk=x").is_none());
        assert_eq!(
            texture_hash("http://textures.minecraft.net/texture/abc.png?x=1").unwrap(),
            "abc"
        );
    }

    #[test]
    fn base64_padding_and_unused_bits_match_the_jdk_basic_decoder() {
        // Observed with Java 25.0.5's Base64.getDecoder(). Unused low bits
        // in the final sextet don't contribute to the decoded output.
        for text in ["aGm=", "aGm"] {
            assert_eq!(base64_decode(text).as_deref(), Some(b"hi".as_slice()));
        }
        for text in ["Zh=", "Zh===", "Z h==", "Zg==\n"] {
            assert!(base64_decode(text).is_none(), "{text:?}");
        }
        assert_eq!(base64_decode("Zh==").as_deref(), Some(b"f".as_slice()));
        assert_eq!(base64_decode("Zh").as_deref(), Some(b"f".as_slice()));
    }
}
