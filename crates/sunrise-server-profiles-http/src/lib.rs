//! Online Minecraft Services account lookups and public-key fetches over HTTPS:
//! `MinecraftServicesProfileRepository.findProfileByName`,
//! `YggdrasilMinecraftSessionService.fetchProfile`, and `MinecraftServicesKeyInfo.get`.
#![forbid(unsafe_code)]

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use sunrise_level::profile_lookup::{
    NameOrId, ProfileResolver, Resolve, ResolvedProfile, parse_uuid,
};
use sunrise_server_files::{
    json::{self, Json},
    profiles::{
        DEFAULT_DISCOVERY, ServicesKeySet, discovery_url, found, profile_key_keys,
        valid_player_name,
    },
    user_lists::NameAndId,
};

/// How a server resolves a name it was given (CachedUserNameToIdResolver.get,
/// without the usercache.json cache): an offline server makes the offline
/// profile; an online one asks the services (for the name in lower case) and
/// knows nobody they don't.
pub fn resolve(name: &str, online: bool) -> Option<NameAndId> {
    if online && valid_player_name(name) {
        find_by_name(&discovery_url(), &name.to_ascii_lowercase())
    } else {
        (!online).then(|| NameAndId::offline(name))
    }
}

/// MinecraftServicesProfileRepository.findProfileByName: the account named
/// `name` (any case), or None when there is none or the services don't
/// answer (vanilla logs both as "Couldn't find profile with name").
pub fn find_by_name(discovery: &str, name: &str) -> Option<NameAndId> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("Sunrise/{}", sunrise_core::release::VERSION))
        .build();
    let doc = json::parse(&agent.get(discovery).call().ok()?.into_string().ok()?, true).ok()?;
    let url = json_endpoint(&doc, "profiles", "getByName")?.replace("{name}", &name.to_lowercase());
    found(&agent.get(&url).call().ok()?.into_string().ok()?, name)
}

/// YggdrasilMinecraftSessionService.fetchProfile(id, true), as the management protocol finds a player given by
/// id that the user cache doesn't know: the `session` / `getProfileById` endpoint, or None.
pub fn find_by_id(discovery: &str, id: u128) -> Option<NameAndId> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("Sunrise/{}", sunrise_core::release::VERSION))
        .build();
    let doc = json::parse(&agent.get(discovery).call().ok()?.into_string().ok()?, true).ok()?;
    let url = json_endpoint(&doc, "session", "getProfileById")?
        .replace("{profileId}", &format!("{id:032x}"));
    let separator = if url.contains('?') { '&' } else { '?' };
    let answer = agent
        .get(&format!("{url}{separator}unsigned=false"))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let name = json::parse(&answer, true).ok()?.get("name")?.as_string()?;
    found(&answer, &name).filter(|user| user.id == id)
}

/// MinecraftServicesKeyInfo.get: the services' public keys from the
/// discovery document's `authentication` / `getPublicKeys` endpoint.
/// The error is the line vanilla logs, after which it has no keys.
pub fn fetch_profile_key_keys(discovery: &str) -> Result<ServicesKeySet, &'static str> {
    const FAILED: &str = "Failed to request Minecraft Services public key";
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("Sunrise/{}", sunrise_core::release::VERSION))
        .build();
    let get = |url: &str| agent.get(url).call().ok()?.into_string().ok();
    let doc = get(discovery)
        .and_then(|d| json::parse(&d, true).ok())
        .ok_or(FAILED)?;
    let url = json_endpoint(&doc, "authentication", "getPublicKeys").ok_or(FAILED)?;
    profile_key_keys(&get(&url).ok_or(FAILED)?)
}

/// MinecraftServicesDiscoveryService.getUrl(service, endpoint).
fn json_endpoint(doc: &Json, service: &str, name: &str) -> Option<String> {
    doc.get("discovery")?
        .get(service)?
        .get("endpoints")?
        .get(name)?
        .get("uri")?
        .as_string()
}

/// Why joining a server through the session service failed, like the
/// AuthenticationException subclasses authlib throws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinError {
    /// No answer, or a 5xx (AuthenticationUnavailableException).
    Unavailable,
    /// 401 or ForbiddenOperationException (InvalidCredentialsException).
    InvalidSession,
    /// InsufficientPrivilegesException: multiplayer is off for the account.
    InsufficientPrivileges,
    /// multiplayer.access.banned or FORCED_USERNAME_CHANGE.
    Banned,
    /// Anything else, with the service's message.
    Other(String),
}
impl JoinError {
    /// MinecraftClientHttpException.toAuthenticationException.
    pub fn from_response(status: u16, body: &str) -> Self {
        let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let error = v.get("error").and_then(|e| e.as_str()).unwrap_or("");
        match error {
            "ForbiddenOperationException" => return Self::InvalidSession,
            "multiplayer.access.banned" | "FORCED_USERNAME_CHANGE" => return Self::Banned,
            "InsufficientPrivilegesException" => return Self::InsufficientPrivileges,
            _ => {}
        }
        match status {
            401 => Self::InvalidSession,
            500.. => Self::Unavailable,
            _ => Self::Other(
                v.get("errorMessage")
                    .and_then(|m| m.as_str())
                    .map_or_else(|| format!("Status: {status}"), str::to_owned),
            ),
        }
    }
    /// ClientHandshakePacketListenerImpl.authenticateServer's message: the
    /// key of the reason inside "disconnect.loginFailedInfo", or its text.
    pub fn reason(&self) -> Result<&'static str, &str> {
        match self {
            Self::Unavailable => Ok("disconnect.loginFailedInfo.serversUnavailable"),
            Self::InvalidSession => Ok("disconnect.loginFailedInfo.invalidSession"),
            Self::InsufficientPrivileges => Ok("disconnect.loginFailedInfo.insufficientPrivileges"),
            Self::Banned => Ok("disconnect.loginFailedInfo.userBanned"),
            Self::Other(message) => Err(message),
        }
    }
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("Sunrise/{}", sunrise_core::release::VERSION))
        .build()
}

/// Whether `SUNRISE_API_DISCOVERY` names another services environment.
pub fn discovery_overridden() -> bool {
    discovery_url() != DEFAULT_DISCOVERY
}

/// The discovery document, fetched once per game (a failure is retried next time).
pub fn discovery() -> Result<Arc<serde_json::Value>, JoinError> {
    static CACHE: Mutex<Option<Arc<serde_json::Value>>> = Mutex::new(None);
    if let Some(d) = CACHE.lock().unwrap().clone() {
        return Ok(d);
    }
    let doc: serde_json::Value = agent()
        .get(&discovery_url())
        .call()
        .inspect_err(|e| log::warn!("The services discovery document: {e}"))
        .ok()
        .and_then(|r| r.into_string().ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or(JoinError::Unavailable)?;
    let doc = Arc::new(doc);
    *CACHE.lock().unwrap() = Some(doc.clone());
    Ok(doc)
}

/// MinecraftServicesDiscoveryService.getUrl(service, endpoint).
pub fn endpoint(doc: &serde_json::Value, service: &str, name: &str) -> Option<String> {
    doc.pointer(&format!("/discovery/{service}/endpoints/{name}/uri"))
        .and_then(|u| u.as_str())
        .map(str::to_owned)
}

/// What the session service says about a player joining this server
/// (MinecraftServicesSessionService.hasJoinedServer).
#[derive(Clone, Debug, PartialEq)]
pub struct JoinedProfile {
    pub uuid: u128,
    pub properties: Vec<sunrise_core::profile::AccountProperty>,
}

/// HttpDiscoveryService.buildQuery: `URLEncoder.encode(value, UTF-8)`.
fn url_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'*' | b'_' => {
                out.push(char::from(b));
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The session service's verify endpoint, asked by an online-mode server
/// whether `name` joined with `server_hash` (and from `ip`, when the server
/// prevents proxy connections). `Ok(None)`: it didn't, or the service said
/// no; `Err(Unavailable)`: no answer.
pub fn has_joined(
    name: &str,
    server_hash: &str,
    ip: Option<std::net::IpAddr>,
) -> Result<Option<JoinedProfile>, JoinError> {
    let doc = discovery()?;
    let url = endpoint(&doc, "session", "verify").ok_or(JoinError::Unavailable)?;
    let mut query = format!(
        "username={}&serverId={}",
        url_encode(name),
        url_encode(server_hash)
    );
    if let Some(ip) = ip {
        query.push_str(&format!("&ip={}", url_encode(&ip.to_string())));
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    let body = match agent().get(&format!("{url}{separator}{query}")).call() {
        Ok(response) => response.into_string().unwrap_or_default(),
        Err(ureq::Error::Status(status, response)) => {
            let error =
                JoinError::from_response(status, &response.into_string().unwrap_or_default());
            return match error {
                JoinError::Unavailable => Err(error),
                _ => Ok(None),
            };
        }
        Err(ureq::Error::Transport(_)) => return Err(JoinError::Unavailable),
    };
    Ok(parse_joined(&body))
}

/// HasJoinedMinecraftServerResponse: `id` and `properties`; no id is no profile.
pub fn parse_joined(body: &str) -> Option<JoinedProfile> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let uuid = parse_uuid(v.get("id")?.as_str()?)?;
    let properties = v
        .get("properties")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| {
            Some(sunrise_core::profile::AccountProperty {
                name: p.get("name")?.as_str()?.to_owned(),
                value: p.get("value")?.as_str()?.to_owned(),
                signature: p
                    .get("signature")
                    .and_then(|s| s.as_str())
                    .map(str::to_owned),
            })
        })
        .collect();
    Some(JoinedProfile { uuid, properties })
}

/// A GET over HTTP or HTTPS (textures.minecraft.net serves both; vanilla
/// uses https): the body of a 2xx answer, 16 MiB at most, after redirects.
pub fn http_get(url: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let response = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("Java")
        .build()
        .get(url)
        .call()
        .ok()?;
    let mut body = Vec::new();
    response
        .into_reader()
        .take(16 << 20)
        .read_to_end(&mut body)
        .ok()?;
    Some(body)
}

/// `ProfileResolver.Cached`: resolves a `Dynamic` profile through the
/// services — `profileCacheByName` asks `profiles`/`getByName` for the id,
/// `profileCacheById` asks `session`/`getProfileById` for the profile —
/// at most 256 entries kept 10 minutes after their last use, fetched on
/// background threads.
pub struct ProfileCache {
    entries: std::collections::HashMap<NameOrId, Entry>,
    done: std::sync::mpsc::Receiver<(NameOrId, Option<ResolvedProfile>)>,
    send: std::sync::mpsc::Sender<(NameOrId, Option<ResolvedProfile>)>,
    last_sweep: std::time::Instant,
    /// Starts the lookup of a key, answering on the sender when done: [`fetch_in_background`],
    /// or a test's fake backend.
    lookup: Lookup,
}
pub type ProfileAnswer = std::sync::mpsc::Sender<(NameOrId, Option<ResolvedProfile>)>;
type Lookup = Box<dyn FnMut(NameOrId, ProfileAnswer) + Send>;
struct Entry {
    touched: std::time::Instant,
    /// `None` while the fetch is in flight.
    resolved: Option<Option<ResolvedProfile>>,
}
impl Entry {
    fn live_at(&self, now: std::time::Instant) -> bool {
        now - self.touched < PROFILE_TTL
    }
    fn resolution(&self) -> Resolve {
        self.resolved
            .clone()
            .map_or(Resolve::Pending, Resolve::Done)
    }
}
/// `ProfileResolver.Cached`'s cache bounds.
const MAX_PROFILES: usize = 256;
const PROFILE_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// `fetchByNameOrId`: `getByName` answers `{id, name}`; `getProfileById`
/// (`fetchProfile(id, requireSecure)`) answers `{id, name, properties}` —
/// asked `unsigned=false` so the `textures` property is signed.
fn resolve_profile(key: &NameOrId, doc: &serde_json::Value) -> Option<ResolvedProfile> {
    let id = match key {
        NameOrId::Id(id) => *id,
        NameOrId::Name(name) => {
            let url =
                endpoint(doc, "profiles", "getByName")?.replace("{name}", &name.to_lowercase());
            let body = http_get(&url)?;
            let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
            parse_uuid(v.get("id")?.as_str()?)?
        }
    };
    let url =
        endpoint(doc, "session", "getProfileById")?.replace("{profileId}", &format!("{id:032x}"));
    let separator = if url.contains('?') { '&' } else { '?' };
    let body = http_get(&format!("{url}{separator}unsigned=false"))?;
    let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
    Some(ResolvedProfile {
        id: parse_uuid(v.get("id")?.as_str()?)?,
        name: v.get("name")?.as_str()?.to_owned(),
        properties: v
            .get("properties")?
            .as_array()?
            .iter()
            .map(|p| {
                Some((
                    p.get("name")?.as_str()?.to_owned(),
                    p.get("value")?.as_str()?.to_owned(),
                    p.get("signature")
                        .and_then(|s| s.as_str())
                        .map(str::to_owned),
                ))
            })
            .collect::<Option<Vec<_>>>()?,
    })
}

/// The services' lookup of `key` on a background thread, never waited on.
fn fetch_in_background(key: NameOrId, send: ProfileAnswer) {
    std::thread::spawn(move || {
        let resolved = discovery().ok().and_then(|doc| resolve_profile(&key, &doc));
        let _ = send.send((key, resolved));
    });
}

impl Default for ProfileCache {
    fn default() -> Self {
        Self::new()
    }
}
impl ProfileCache {
    pub fn new() -> Self {
        Self::with_lookup(fetch_in_background)
    }
    /// A resolver whose lookups `lookup` starts instead of the services.
    pub fn with_lookup(lookup: impl FnMut(NameOrId, ProfileAnswer) + Send + 'static) -> Self {
        let (send, done) = std::sync::mpsc::channel();
        Self {
            entries: Default::default(),
            done,
            send,
            last_sweep: std::time::Instant::now(),
            lookup: Box::new(lookup),
        }
    }
    fn sweep_time(&self) -> Option<std::time::Instant> {
        let now = std::time::Instant::now();
        (now - self.last_sweep >= std::time::Duration::from_secs(30)).then_some(now)
    }
    /// `CacheBuilder.expireAfterAccess` with `maximumSize`. Compute the same
    /// removals for a prospective lookup and for a committed sweep.
    fn over_capacity(&self, now: std::time::Instant) -> Vec<NameOrId> {
        if self.entries.len() <= MAX_PROFILES {
            return Vec::new();
        }
        let mut live = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.live_at(now))
            .collect::<Vec<_>>();
        let excess = live.len().saturating_sub(MAX_PROFILES);
        if excess == 0 {
            return Vec::new();
        }
        live.sort_by_key(|(_, entry)| entry.touched);
        live.into_iter()
            .take(excess)
            .map(|(key, _)| key.clone())
            .collect()
    }
    fn sweep(&mut self) {
        if let Some(now) = self.sweep_time() {
            self.last_sweep = now;
            for key in self.over_capacity(now) {
                self.entries.remove(&key);
            }
            self.entries.retain(|_, entry| entry.live_at(now));
        }
    }
    fn receive_completed(&mut self) {
        while let Ok((key, resolved)) = self.done.try_recv() {
            self.entries.insert(
                key,
                Entry {
                    touched: std::time::Instant::now(),
                    resolved: Some(resolved),
                },
            );
        }
    }
    /// Inspect the cached resolution without starting a lookup or mutating access times.
    pub fn peek(&mut self, key: &NameOrId) -> Resolve {
        self.receive_completed();
        let Some(entry) = self.entries.get(key) else {
            return Resolve::Pending;
        };
        if let Some(now) = self.sweep_time()
            && (!entry.live_at(now) || self.over_capacity(now).contains(key))
        {
            return Resolve::Pending;
        }
        entry.resolution()
    }
    /// How the resolution of `key` stands, starting it the first time.
    pub fn state(&mut self, key: &NameOrId) -> Resolve {
        self.receive_completed();
        self.sweep();
        if let Some(entry) = self.entries.get_mut(key) {
            entry.touched = std::time::Instant::now();
            return entry.resolution();
        }
        // `fetchByName` refuses an invalid name without a fetch; like every
        // other empty answer that is the cached result.
        let resolved = matches!(key, NameOrId::Name(name) if !valid_player_name(name));
        self.entries.insert(
            key.clone(),
            Entry {
                touched: std::time::Instant::now(),
                resolved: resolved.then_some(None),
            },
        );
        if resolved {
            return Resolve::Done(None);
        }
        (self.lookup)(key.clone(), self.send.clone());
        Resolve::Pending
    }
}

/// Synchronized `ProfileResolver.Cached` for server commands and modding.
pub struct CachedProfileResolver {
    cache: Mutex<ProfileCache>,
}

impl Default for CachedProfileResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl CachedProfileResolver {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(ProfileCache::new()),
        }
    }

    /// A resolver whose lookups `lookup` starts instead of the services.
    pub fn with_lookup(lookup: impl FnMut(NameOrId, ProfileAnswer) + Send + 'static) -> Self {
        Self {
            cache: Mutex::new(ProfileCache::with_lookup(lookup)),
        }
    }

    /// How the resolution of `key` stands, starting it the first time.
    pub fn state(&mut self, key: &NameOrId) -> Resolve {
        self.cache
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .state(key)
    }
}

impl ProfileResolver for CachedProfileResolver {
    fn lookup(&self, query: &NameOrId) -> Resolve {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .state(query)
    }
}

/// The signed-in account's own profile properties (`Minecraft.profileFuture`:
/// `fetchProfile(id, true)`, so its `textures` property is signed), or empty
/// without an answer. Blocks on the network.
pub fn fetch_own_properties(id: u128) -> Vec<(String, String, Option<String>)> {
    discovery()
        .ok()
        .and_then(|doc| resolve_profile(&NameOrId::Id(id), &doc))
        .map(|p| p.properties)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: u128 = 0x069a79f444e94726a5befca90e38aaf5;

    #[test]
    fn resolve_offline_and_invalid_names() {
        assert_eq!(resolve("Alex", false), Some(NameAndId::offline("Alex")));
        assert_eq!(resolve("not valid", true), None);
    }

    #[test]
    fn verify_answers() {
        assert_eq!(url_encode("Tester_1"), "Tester_1");
        assert_eq!(url_encode("-5a f"), "-5a+f");
        assert_eq!(url_encode("::1"), "%3A%3A1");
        let body = r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch","properties":[{"name":"textures","value":"e30=","signature":"c2ln"}]}"#;
        let p = parse_joined(body).unwrap();
        assert_eq!(p.uuid, UUID);
        assert_eq!(p.properties.len(), 1);
        assert_eq!(p.properties[0].signature.as_deref(), Some("c2ln"));
        assert_eq!(parse_joined(""), None);
        assert_eq!(parse_joined(r#"{"name":"Notch"}"#), None);
    }

    #[test]
    fn errors_map_like_authlib() {
        let e = |status, body| JoinError::from_response(status, body);
        assert_eq!(
            e(
                403,
                r#"{"error":"ForbiddenOperationException","errorMessage":"Invalid token"}"#
            ),
            JoinError::InvalidSession
        );
        assert_eq!(e(401, ""), JoinError::InvalidSession);
        assert_eq!(
            e(403, r#"{"error":"InsufficientPrivilegesException"}"#),
            JoinError::InsufficientPrivileges
        );
        assert_eq!(
            e(403, r#"{"error":"multiplayer.access.banned"}"#),
            JoinError::Banned
        );
        assert_eq!(e(503, "down"), JoinError::Unavailable);
        assert_eq!(
            e(429, r#"{"errorMessage":"Too many"}"#),
            JoinError::Other("Too many".into())
        );
        assert_eq!(
            JoinError::InvalidSession.reason(),
            Ok("disconnect.loginFailedInfo.invalidSession")
        );
    }

    #[test]
    fn discovery_endpoints() {
        let doc = serde_json::json!({"discovery": {"session": {"endpoints": {"join": {"uri": "https://s/join"}}}}});
        assert_eq!(
            endpoint(&doc, "session", "join").as_deref(),
            Some("https://s/join")
        );
        assert_eq!(endpoint(&doc, "session", "verify"), None);
    }

    /// `getByName` and `getProfileById` answer from the services the
    /// discovery document names; a profile resolves through both.
    #[test]
    #[ignore = "daily"]
    fn profiles_resolve_through_the_services() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let reply = |status: &str, body: &str| {
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            for stream in listener.incoming().take(3) {
                let mut stream = stream.unwrap();
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                reader.read_line(&mut line).unwrap();
                while reader.read_line(&mut String::new()).unwrap() > 2 {}
                let body = if line.contains("/users/profiles/minecraft/") {
                    r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch"}"#
                } else if line.contains("/session/minecraft/profile/")
                    && line.contains("unsigned=false")
                {
                    r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch","properties":[{"name":"textures","value":"dmFsdWU=","signature":"c2ln"}]}"#
                } else {
                    "{}"
                };
                stream.write_all(reply("200 OK", body).as_bytes()).unwrap();
            }
        });
        let doc = serde_json::json!({"discovery": {
            "profiles": {"endpoints": {"getByName": {"uri": format!("{base}/users/profiles/minecraft/{{name}}")}}},
            "session": {"endpoints": {"getProfileById": {"uri": format!("{base}/session/minecraft/profile/{{profileId}}")}}},
        }});
        let resolved =
            super::resolve_profile(&super::NameOrId::Name("Notch".into()), &doc).unwrap();
        assert_eq!(resolved.id, 0x069a_79f4_44e9_4726_a5be_fca9_0e38_aaf5);
        assert_eq!(resolved.name, "Notch");
        assert_eq!(
            resolved.properties,
            vec![(
                "textures".to_owned(),
                "dmFsdWU=".to_owned(),
                Some("c2ln".to_owned())
            )]
        );
        let by_id = super::resolve_profile(&super::NameOrId::Id(resolved.id), &doc).unwrap();
        assert_eq!(by_id.name, "Notch");
        // A discovery without `profiles`/`getByName` resolves no name.
        let doc = serde_json::json!({"discovery": {
            "session": {"endpoints": {"getProfileById": {"uri": format!("{base}/session/minecraft/profile/{{profileId}}")}}},
        }});
        assert!(super::resolve_profile(&super::NameOrId::Name("Notch".into()), &doc).is_none());
    }
}
