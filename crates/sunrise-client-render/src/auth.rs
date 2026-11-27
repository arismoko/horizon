//! The Microsoft account the launcher signed in, handed over in a session
//! file (docs/microsoft-account.md), and the Minecraft services call
//! online-mode servers need: authlib's MinecraftServicesSessionService.joinServer
//! through the services discovery document. The access token stays in this
//! process's memory: it is never logged, printed or written anywhere.
use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, SystemTime},
};
use sunrise_java_properties::now_millis;

/// MinecraftServicesEnvironment.PROD.
pub const DEFAULT_DISCOVERY: &str = "https://discovery.minecraftservices.com/minecraft/client";
/// A session file older than this is refused: it can't be replayed later.
const MAX_SESSION_FILE_AGE: Duration = Duration::from_secs(120);

/// A signed-in account (vanilla's User with type msa).
pub struct AuthSession {
    pub name: String,
    pub uuid: u128,
    pub xuid: Option<String>,
    access_token: String,
}
impl std::fmt::Debug for AuthSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthSession")
            .field("name", &self.name)
            .field("uuid", &sunrise_protocol::crypto::uuid_string(self.uuid))
            .field("access_token", &"<hidden>")
            .finish()
    }
}

pub use sunrise_level::profile_lookup::parse_uuid;

impl AuthSession {
    /// For tests and harnesses.
    pub fn new(name: &str, uuid: u128, access_token: &str) -> Self {
        Self {
            name: name.to_owned(),
            uuid,
            xuid: None,
            access_token: access_token.to_owned(),
        }
    }

    pub fn access_token(&self) -> &str {
        &self.access_token
    }
    /// The session file's JSON: {"name", "uuid", "access_token", "user_type": "msa", "xuid"}.
    pub fn parse(text: &str) -> Result<Self, String> {
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|_| "the session is not valid JSON")?;
        let field = |k: &str| v.get(k).and_then(|s| s.as_str());
        let name = field("name").ok_or("the session has no name")?;
        if name.is_empty() || name.chars().count() > 16 {
            return Err("the session's name is not a valid player name".into());
        }
        let uuid = field("uuid")
            .and_then(parse_uuid)
            .ok_or("the session has no valid uuid")?;
        let access_token = field("access_token")
            .filter(|t| !t.is_empty())
            .ok_or("the session has no access token")?;
        if field("user_type").is_some_and(|t| t != "msa") {
            return Err("the session is not a Microsoft account session".into());
        }
        Ok(Self {
            name: name.to_owned(),
            uuid,
            xuid: field("xuid").map(str::to_owned),
            access_token: access_token.to_owned(),
        })
    }
    /// Read the session file the launcher wrote and delete it at once. It
    /// must be fresh and, on Unix, readable by its owner only.
    pub fn load(path: &Path) -> Result<Self, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let refuse = |why: &str| {
            let _ = std::fs::remove_file(path);
            Err(format!("{}: {why}", path.display()))
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o077 != 0 {
                return refuse("other users can read it");
            }
        }
        let age = meta
            .modified()
            .ok()
            .and_then(|m| SystemTime::now().duration_since(m).ok())
            .unwrap_or_default();
        if age > MAX_SESSION_FILE_AGE {
            return refuse("it is too old (start the game from the launcher)");
        }
        let text = std::fs::read_to_string(path);
        let _ = std::fs::remove_file(path);
        Self::parse(&text.map_err(|e| format!("{}: {e}", path.display()))?)
    }
    /// The access token's expiry (its JWT `exp`) has passed, or is a minute away: the session
    /// service would refuse it. A token that doesn't read as a JWT counts as good: the server
    /// says.
    pub fn expired(&self) -> bool {
        let exp = self
            .access_token
            .split('.')
            .nth(1)
            .and_then(|p| crate::skins::base64_decode(&p.replace('-', "+").replace('_', "/")))
            .and_then(|json| serde_json::from_slice::<serde_json::Value>(&json).ok())
            .and_then(|v| v.get("exp")?.as_u64());
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        exp.is_some_and(|exp| exp <= now + 60)
    }
}

static CURRENT: OnceLock<Arc<AuthSession>> = OnceLock::new();

/// The account this game was started with (set once at start).
pub fn set_current(session: AuthSession) {
    let _ = CURRENT.set(Arc::new(session));
}
pub fn current() -> Option<Arc<AuthSession>> {
    CURRENT.get().cloned()
}

pub use sunrise_server_profiles_http::{
    JoinError, JoinedProfile, discovery_overridden, endpoint, has_joined, parse_joined,
};
pub(crate) use sunrise_server_profiles_http::{agent, discovery};

/// The services discovery document URL: `SUNRISE_API_DISCOVERY` overrides
/// it like authlib's `-Dminecraft.api.discovery.host`.
pub fn discovery_url() -> String {
    std::env::var("SUNRISE_API_DISCOVERY")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| DEFAULT_DISCOVERY.to_owned())
}

/// `Minecraft`'s constructor futures for the title screen: `UserApiService.fetchProperties`
/// (the `player` service's `getAttributes`, with the bearer token) and
/// `SessionService.fetchProfile(uuid, true)` (`getProfileById`). A failed
/// attributes call falls back to the offline properties, a failed profile call
/// to no actions, as vanilla does. Blocks: call it off the main thread.
pub fn fetch_account_status(session: &AuthSession) -> crate::ban_notice::AccountStatus {
    use crate::ban_notice::{AccountStatus, parse_profile_actions, parse_properties};
    let Ok(doc) = discovery() else {
        return AccountStatus::default();
    };
    let properties = endpoint(&doc, "player", "getAttributes")
        .and_then(|url| {
            agent()
                .get(&url)
                .set("Authorization", &format!("Bearer {}", session.access_token))
                .call()
                .ok()?
                .into_string()
                .ok()
        })
        .and_then(|body| parse_properties(&body))
        .unwrap_or_default();
    let actions = endpoint(&doc, "session", "getProfileById")
        .and_then(|url| {
            let url = url.replace("{profileId}", &format!("{:032x}", session.uuid));
            agent()
                .get(&format!("{url}?unsigned=true"))
                .call()
                .ok()?
                .into_string()
                .ok()
        })
        .map(|body| parse_profile_actions(&body))
        .unwrap_or_default();
    AccountStatus {
        properties,
        actions,
    }
}

/// The JoinMinecraftServerRequest body; `selectedProfile` is the undashed UUID.
pub fn join_request(session: &AuthSession, server_hash: &str) -> String {
    serde_json::json!({
        "accessToken": session.access_token,
        "selectedProfile": format!("{:032x}", session.uuid),
        "serverId": server_hash,
    })
    .to_string()
}

/// A join that got no answer is worth one more try, as leaving and joining
/// again would be: `join` is safe to repeat, it only records the server's
/// hash. Any other answer is the service's own and stands.
fn with_one_retry(
    mut join: impl FnMut() -> Result<(), JoinError>,
    pause: Duration,
) -> Result<(), JoinError> {
    match join() {
        Err(JoinError::Unavailable) => {
            std::thread::sleep(pause);
            join()
        }
        result => result,
    }
}

/// MinecraftServicesSessionService.joinServer: tell the session service this
/// account is joining the server whose hash is `server_hash`. Sunrise tries
/// twice when the service doesn't answer; vanilla asks the player to.
pub fn join_server(session: &AuthSession, server_hash: &str) -> Result<(), JoinError> {
    with_one_retry(
        || join_server_once(session, server_hash),
        Duration::from_millis(500),
    )
}

fn join_server_once(session: &AuthSession, server_hash: &str) -> Result<(), JoinError> {
    let doc = discovery()?;
    let url = endpoint(&doc, "session", "join").ok_or(JoinError::Unavailable)?;
    match agent()
        .post(&url)
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&join_request(session, server_hash))
    {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(status, response)) => {
            if status >= 500 {
                log::warn!("The session service's join answered {status}");
            }
            Err(JoinError::from_response(
                status,
                &response.into_string().unwrap_or_default(),
            ))
        }
        Err(ureq::Error::Transport(e)) => {
            log::warn!("The session service's join: {e}");
            Err(JoinError::Unavailable)
        }
    }
}

/// The account's chat key pair (ProfileKeyPair): the private key that signs
/// its chat, the public key with the services' signature over it, and when
/// to fetch a new one (refreshedAfter, epoch milliseconds). Like the access
/// token it stays in memory: vanilla's profilekeys file is never written.
pub struct ChatKeyPair {
    pub private: sunrise_protocol::rsa::RsaPrivateKey,
    pub public: sunrise_protocol::secure_chat::ProfileKey,
    pub refreshed_after: i64,
}
impl std::fmt::Debug for ChatKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatKeyPair")
            .field("expires_at", &self.public.expires_at)
            .field("refreshed_after", &self.refreshed_after)
            .finish_non_exhaustive()
    }
}
impl ChatKeyPair {
    /// ProfileKeyPair.dueRefresh.
    pub fn due_refresh(&self, now: i64) -> bool {
        self.refreshed_after < now
    }
}

/// Instant.parse for the services' UTC times
/// (`2022-07-30T23:05:24.548227Z`): epoch milliseconds.
pub fn parse_instant(text: &str) -> Option<i64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (date.next()??, date.next()??, date.next()??);
    let (hms, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut hms = hms.splitn(3, ':').map(|p| p.parse::<i64>().ok());
    let (h, min, s) = (hms.next()??, hms.next()??, hms.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || min > 59 || s > 60 {
        return None;
    }
    if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let millis = format!("{fraction:0<3}")[..3].parse::<i64>().ok()?;
    // Days from 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + min) * 60_000 + s * 1000 + millis)
}

/// Crypt.stringToPemRsaPrivateKey / stringToRsaPublicKey: the base64
/// between a PEM block's armour lines.
fn pem_der(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .flat_map(|l| l.chars().filter(|c| !c.is_whitespace()))
        .collect();
    crate::skins::base64_decode(&body)
}

/// KeyPairResponse from the certificates endpoint, as
/// AccountProfileKeyPairManager.fetchProfileKeyPair reads it: the PEM key
/// pair, publicKeySignatureV2 (the signature over the profile id too),
/// expiresAt and refreshedAfter. None for anything malformed, or a private
/// key that isn't the public key's other half.
pub fn parse_certificates(body: &str) -> Option<ChatKeyPair> {
    use sunrise_protocol::rsa::{RsaPrivateKey, RsaPublicKey};
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let field = |path: &str| v.pointer(path).and_then(|s| s.as_str());
    let private = RsaPrivateKey::from_der(&pem_der(field("/keyPair/privateKey")?)?).ok()?;
    let key = pem_der(field("/keyPair/publicKey")?)?;
    if RsaPublicKey::from_der(&key).ok()? != private.public {
        return None;
    }
    let signature = crate::skins::base64_decode(field("/publicKeySignatureV2")?)?;
    Some(ChatKeyPair {
        private,
        public: sunrise_protocol::secure_chat::ProfileKey {
            expires_at: parse_instant(field("/expiresAt")?)?,
            key,
            signature,
        },
        refreshed_after: parse_instant(field("/refreshedAfter")?)?,
    })
}

/// UserApiService.getKeyPair: the certificates endpoint, POSTed with the
/// access token.
pub fn fetch_chat_keys(session: &AuthSession) -> Result<ChatKeyPair, JoinError> {
    let doc = discovery()?;
    let url = endpoint(&doc, "player", "getCertificates").ok_or(JoinError::Unavailable)?;
    let body = match agent()
        .post(&url)
        .set("Authorization", &format!("Bearer {}", session.access_token))
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string("")
    {
        Ok(response) => response.into_string().unwrap_or_default(),
        Err(ureq::Error::Status(status, response)) => {
            return Err(JoinError::from_response(
                status,
                &response.into_string().unwrap_or_default(),
            ));
        }
        Err(ureq::Error::Transport(_)) => return Err(JoinError::Unavailable),
    };
    parse_certificates(&body)
        .ok_or_else(|| JoinError::Other("Malformed profile key pair response".to_owned()))
}

/// AccountProfileKeyPairManager.MINIMUM_PROFILE_KEY_REFRESH_INTERVAL.
const KEY_RETRY: Duration = Duration::from_secs(60 * 60);

/// The chat key pair as AccountProfileKeyPairManager keeps it: fetched on a
/// background thread, kept until it is due a refresh, and on a failure kept
/// as it was (the next try waits an hour, or the next join).
#[derive(Default)]
struct ChatKeys {
    current: Option<Arc<ChatKeyPair>>,
    fetching: bool,
    next_try: Option<std::time::Instant>,
}
static CHAT_KEYS: std::sync::LazyLock<Mutex<std::collections::HashMap<u128, ChatKeys>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

/// prepareKeyPair (`now`: on a join) or, from a tick, shouldRefreshKeyPair
/// then prepareKeyPair: start fetching when there is no key pair or it is
/// due a refresh. Nothing without a signed-in account.
pub fn prepare_chat_keys(now: bool) {
    let Some(session) = current() else {
        return;
    };
    prepare_chat_keys_for(session, now);
}

/// Prepare keys for this account, independently of other accounts.
pub fn prepare_chat_keys_for(session: Arc<AuthSession>, now: bool) {
    let uuid = session.uuid;
    let mut accounts = CHAT_KEYS.lock().unwrap();
    let keys = accounts.entry(uuid).or_default();
    let instant = std::time::Instant::now();
    if keys.fetching || !now && keys.next_try.is_some_and(|t| instant < t) {
        return;
    }
    keys.next_try = Some(instant + KEY_RETRY);
    if keys
        .current
        .as_ref()
        .is_some_and(|k| !k.due_refresh(now_millis()))
    {
        return;
    }
    keys.fetching = true;
    drop(accounts);
    let spawned = std::thread::Builder::new()
        .name("Profile key fetch".into())
        .spawn(move || {
            let result = fetch_chat_keys(&session);
            let mut accounts = CHAT_KEYS.lock().unwrap();
            let keys = accounts.entry(uuid).or_default();
            keys.fetching = false;
            match result {
                Ok(pair) => keys.current = Some(Arc::new(pair)),
                Err(e) => log::error!("Failed to retrieve profile key pair: {e:?}"),
            }
        });
    if spawned.is_err() {
        CHAT_KEYS.lock().unwrap().entry(uuid).or_default().fetching = false;
    }
}

/// This account's key pair; keys for other accounts are never returned.
pub fn chat_keys_for(uuid: u128) -> Option<Arc<ChatKeyPair>> {
    CHAT_KEYS
        .lock()
        .unwrap()
        .get(&uuid)?
        .current
        .clone()
        .filter(|k| !k.public.has_expired(now_millis()))
}

const MINECRAFT_PROFILE: &str = "https://api.minecraftservices.com/minecraft/profile";

/// The signed-in account's active skin and cape from `GET /minecraft/profile`.
/// Single-player and integrated servers often send no `textures` property, or
/// one without the equipped cape; the client asks the profile endpoint once.
pub fn own_textures(session: &AuthSession) -> Option<crate::skins::Textures> {
    let response = agent()
        .get(MINECRAFT_PROFILE)
        .set("Authorization", &format!("Bearer {}", session.access_token))
        .call()
        .ok()?;
    if response.status() != 200 {
        return None;
    }
    let body = response.into_string().ok()?;
    let profile: serde_json::Value = serde_json::from_str(&body).ok()?;
    Some(crate::skins::services_profile_textures(&profile))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: u128 = 0x069a79f444e94726a5befca90e38aaf5;

    /// Runs `with_one_retry` over these answers, in order: the result and how
    /// many it asked for.
    fn joined(answers: &[Result<(), JoinError>]) -> (Result<(), JoinError>, usize) {
        let mut asked = 0;
        let result = with_one_retry(
            || {
                asked += 1;
                answers[asked - 1].clone()
            },
            Duration::ZERO,
        );
        (result, asked)
    }

    #[test]
    fn a_join_with_no_answer_is_tried_once_more() {
        let none = Err(JoinError::Unavailable);
        assert_eq!(joined(&[none.clone(), Ok(())]), (Ok(()), 2));
        // Twice without an answer: the message the player knows.
        assert_eq!(joined(&[none.clone(), none.clone()]), (none, 2));
    }

    #[test]
    fn a_join_the_service_answered_is_not_tried_again() {
        assert_eq!(joined(&[Ok(())]), (Ok(()), 1));
        for refused in [
            JoinError::InvalidSession,
            JoinError::Banned,
            JoinError::InsufficientPrivileges,
            JoinError::Other("Status: 429".into()),
        ] {
            assert_eq!(joined(&[Err(refused.clone())]), (Err(refused), 1));
        }
    }

    /// base64url without padding, as a JWT's parts are.
    fn base64url(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n =
                chunk.iter().fold(0u32, |n, &b| n << 8 | u32::from(b)) << (8 * (3 - chunk.len()));
            for k in 0..=chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * k) & 63) as usize]));
            }
        }
        out
    }

    #[test]
    fn a_token_past_its_exp_is_expired() {
        // header.payload.signature, the payload base64url without padding.
        let token = |exp: u64| {
            let json = format!(r#"{{"exp":{exp}}}"#);
            let payload = base64url(json.as_bytes());
            AuthSession::new("Ann", UUID, &format!("e30.{payload}.sig"))
        };
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(token(now - 10).expired());
        assert!(
            token(now + 30).expired(),
            "a minute from its end: as good as gone"
        );
        assert!(!token(now + 3600).expired());
        assert!(!AuthSession::new("Ann", UUID, "not-a-jwt").expired());
    }

    fn write(dir: &Path, text: &str, mode: u32) -> std::path::PathBuf {
        let path = dir.join("session.json");
        std::fs::write(&path, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let _ = mode;
        path
    }

    #[test]
    fn session_files_are_read_once_and_must_be_private() {
        let dir = std::env::temp_dir().join(format!("sunrise-auth-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = r#"{"name":"Notch","uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","access_token":"mock-token","user_type":"msa"}"#;
        let path = write(&dir, text, 0o600);
        let s = AuthSession::load(&path).unwrap();
        assert_eq!((s.name.as_str(), s.uuid), ("Notch", UUID));
        assert!(!path.exists(), "deleted after reading");
        assert!(!format!("{s:?}").contains("mock-token"));
        #[cfg(unix)]
        {
            let path = write(&dir, text, 0o644);
            assert!(
                AuthSession::load(&path)
                    .unwrap_err()
                    .contains("other users")
            );
            assert!(!path.exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_file_fields() {
        assert!(
            AuthSession::parse(r#"{"name":"Notch","uuid":"069a79f444e94726a5befca90e38aaf5"}"#)
                .is_err()
        );
        assert!(
            AuthSession::parse(
                r#"{"name":"","uuid":"069a79f444e94726a5befca90e38aaf5","access_token":"t"}"#
            )
            .is_err()
        );
        assert!(AuthSession::parse(r#"{"name":"A","uuid":"nope","access_token":"t"}"#).is_err());
        assert!(AuthSession::parse(r#"{"name":"A","uuid":"069a79f444e94726a5befca90e38aaf5","access_token":"t","user_type":"legacy"}"#).is_err());
        let s = AuthSession::parse(r#"{"name":"A","uuid":"069a79f444e94726a5befca90e38aaf5","access_token":"t","xuid":"25"}"#).unwrap();
        assert_eq!(s.xuid.as_deref(), Some("25"));
    }

    #[test]
    fn join_request_like_authlib() {
        let s = AuthSession::new("Notch", UUID, "tok");
        let v: serde_json::Value = serde_json::from_str(&join_request(&s, "-7c9d")).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"accessToken": "tok", "selectedProfile": "069a79f444e94726a5befca90e38aaf5", "serverId": "-7c9d"})
        );
    }

    #[test]
    fn instants_like_java() {
        assert_eq!(parse_instant("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_instant("1970-01-01T00:00:01.5Z"), Some(1500));
        // Instant.parse("2022-07-30T23:05:24.548227Z").toEpochMilli().
        assert_eq!(
            parse_instant("2022-07-30T23:05:24.548227Z"),
            Some(1_659_222_324_548)
        );
        assert_eq!(
            parse_instant("2024-02-29T12:00:00Z"),
            Some(1_709_208_000_000)
        );
        assert_eq!(parse_instant("1969-12-31T23:59:59Z"), Some(-1000));
        for bad in [
            "2022-07-30T23:05:24",
            "2022-13-01T00:00:00Z",
            "2022-07-30 23:05:24Z",
            "2022-07-30T23:05:24.5x1Z",
            "",
        ] {
            assert_eq!(parse_instant(bad), None, "{bad}");
        }
    }

    /// A throwaway 1024-bit key (`openssl genrsa`, PKCS#8 as the services
    /// send it under "RSA PRIVATE KEY") and its public half.
    const PRIVATE_B64: &str = "MIICdQIBADANBgkqhkiG9w0BAQEFAASCAl8wggJbAgEAAoGBAKSDqaLOF2Ltt4tzWblkFeNYhdss67BhXTbOMUYjpcPdtf4dzNtgZsTf4WEluvc8pItGWChdpq3xzPC9GL4PZ+Oh1o6VR/wSm20/50QT7V+YcSP1VrVKjDpDSg/UoC1Fmdc/vb9AO8Ip7TxuRYd/uyog/ThQ8aGcWtvvtwPqx4XrAgMBAAECgYA+ZGy2gQDflRZSPeoeZHXkphMF/7BXUWW/vk1zs1tPwEsgV/70iEgRjHL2nA+fapFg/Mm8bWAkghrehmjW1BLq5w8sGdGJCHfE/kauu78jJK0/ME38e9bsSEvv5HMMxyx0mlV8DQu2pzCT71ceT8VD/1pewUbpeZypj5pxtsCIMQJBANXgca/mnAQXTuowI8FqMiald6OuAcAWbOaNrCB5HHWuxH7hFatSqGrgp4YLqHv7bzeodU1y/uxwc+/01khd0rMCQQDE6mPfIdZxHhWkMUAKhMEnrLanfCL0dfyAe+lBpqgPwO3UurCfCaFX5RdYB3fCyvgy5foHN1/I8/Eobp+enrvpAkBgkZnjeXWiZz/Lf/b1AtUoG/g8IHpL2BRNuVQpyWVvQX0ZaNoW5BiFQXCClsr5SBr8f4SPdsfhzBidFOiX/jz/AkBDuk6Bd3T3UR6ma54YQBLjX38vWf330QD4/CV9RCH30Sa8DT4kZT6OKah+yYrCf+cgMoNtqfkS9/xRKMNT3krJAkAaFrAT6HF5Y9b5OLASbbFe7MZ4zNCzuEl6plKNUi+JCHESaU/VCCuYOLoWLeSDFYkZUoSmqiRxH8fVTsBz393x";
    const PUBLIC_B64: &str = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQCkg6mizhdi7beLc1m5ZBXjWIXbLOuwYV02zjFGI6XD3bX+HczbYGbE3+FhJbr3PKSLRlgoXaat8czwvRi+D2fjodaOlUf8EpttP+dEE+1fmHEj9Va1Sow6Q0oP1KAtRZnXP72/QDvCKe08bkWHf7sqIP04UPGhnFrb77cD6seF6wIDAQAB";

    fn pem(label: &str, b64: &str) -> String {
        let lines: Vec<&str> = b64
            .as_bytes()
            .chunks(76)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect();
        format!(
            "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
            lines.join("\n")
        )
    }

    fn certificates(private: &str, public: &str) -> String {
        serde_json::json!({
            "keyPair": {
                "privateKey": pem("RSA PRIVATE KEY", private),
                "publicKey": pem("RSA PUBLIC KEY", public),
            },
            "publicKeySignature": "djE=",
            "publicKeySignatureV2": "c2ln",
            "expiresAt": "2022-07-30T23:05:24.548227Z",
            "refreshedAfter": "2022-07-30T15:05:24.548227Z",
        })
        .to_string()
    }

    #[test]
    fn local_accounts_use_their_own_chat_keys() {
        let pair = |signature: &[u8]| {
            let mut keys = parse_certificates(&certificates(PRIVATE_B64, PUBLIC_B64)).unwrap();
            keys.public.expires_at = i64::MAX;
            keys.refreshed_after = i64::MAX;
            keys.public.signature = signature.to_vec();
            Arc::new(keys)
        };
        let ann = pair(b"ann");
        let bea = pair(b"bea");
        {
            let mut accounts = CHAT_KEYS.lock().unwrap();
            for (uuid, keys) in [(101, ann.clone()), (102, bea.clone())] {
                accounts.insert(
                    uuid,
                    ChatKeys {
                        current: Some(keys),
                        ..Default::default()
                    },
                );
            }
        }
        prepare_chat_keys_for(Arc::new(AuthSession::new("Ann", 101, "unused")), true);
        assert!(Arc::ptr_eq(&chat_keys_for(101).unwrap(), &ann));
        assert!(Arc::ptr_eq(&chat_keys_for(102).unwrap(), &bea));
        assert!(chat_keys_for(103).is_none());
        CHAT_KEYS.lock().unwrap().remove(&101);
        CHAT_KEYS.lock().unwrap().remove(&102);
    }

    #[test]
    #[ignore = "daily"]
    fn certificates_response() {
        let pair = parse_certificates(&certificates(PRIVATE_B64, PUBLIC_B64)).unwrap();
        assert_eq!(
            pair.public.key,
            crate::skins::base64_decode(PUBLIC_B64).unwrap()
        );
        assert_eq!(pair.public.signature, b"sig", "publicKeySignatureV2");
        assert_eq!(pair.public.expires_at, 1_659_222_324_548);
        assert_eq!(pair.refreshed_after, 1_659_193_524_548);
        assert_eq!(pair.private.public_der(), pair.public.key);
        assert!(pair.due_refresh(1_659_193_524_549));
        assert!(!pair.due_refresh(1_659_193_524_548));
        assert!(!format!("{pair:?}").contains("MII"));
        // A public key that isn't the private key's, and broken fields.
        let other = sunrise_protocol::rsa::RsaPrivateKey::generate(1024).unwrap();
        let other = base64(&other.public_der());
        assert!(parse_certificates(&certificates(PRIVATE_B64, &other)).is_none());
        assert!(parse_certificates(&certificates("bm9wZQ==", PUBLIC_B64)).is_none());
        assert!(parse_certificates("{}").is_none());
        assert!(parse_certificates("").is_none());
    }

    fn base64(bytes: &[u8]) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in bytes.chunks(3) {
            let n = c
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
            for i in 0..4 {
                out.push(if i <= c.len() {
                    char::from(A[(n >> (18 - 6 * i) & 63) as usize])
                } else {
                    '='
                });
            }
        }
        out
    }
}
