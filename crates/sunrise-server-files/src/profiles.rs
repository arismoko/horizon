//! Account profile validation, discovery URL, and Minecraft Services JSON/key codecs.
//! Online HTTPS lookups live in `sunrise-server-profiles-http`.
use crate::{
    json::{self, Json},
    user_lists::NameAndId,
};
use sunrise_protocol::rsa::RsaPublicKey;

/// Mojang's services discovery document.
pub const DEFAULT_DISCOVERY: &str = "https://discovery.minecraftservices.com/minecraft/client";

/// The discovery document's URL: `SUNRISE_API_DISCOVERY` overrides it like
/// authlib's `-Dminecraft.api.discovery.host`.
pub fn discovery_url() -> String {
    std::env::var("SUNRISE_API_DISCOVERY")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| DEFAULT_DISCOVERY.to_owned())
}

/// StringUtil.isValidPlayerName: at most 16 UTF-16 units, none a control
/// character, a space or above ASCII.
pub fn valid_player_name(name: &str) -> bool {
    name.encode_utf16().count() <= 16 && name.encode_utf16().all(|c| c > 32 && c < 127)
}

/// Authlib's `ServicesKeySet`, with the keys kept separate by purpose.
#[derive(Debug, Default)]
pub struct ServicesKeySet {
    /// `ServicesKeyType.PROFILE_KEY`: signs players' chat certificates.
    pub player_certificate_keys: Vec<RsaPublicKey>,
    /// `ServicesKeyType.PROFILE_PROPERTY`: signs profile properties such as textures.
    pub profile_property_keys: Vec<RsaPublicKey>,
}

/// The key set answer: `{"profilePropertyKeys": [...], "playerCertificateKeys":
/// [...]}`, each `{"publicKey": base64 X.509}`. One bad key makes the whole
/// answer malformed (parseList throws).
pub fn profile_key_keys(answer: &str) -> Result<ServicesKeySet, &'static str> {
    use base64::Engine;
    const MALFORMED: &str = "Received malformed Minecraft Services public key data";
    let doc = json::parse(answer, true).map_err(|_| MALFORMED)?;
    let list = |name: &str| -> Result<Vec<RsaPublicKey>, &'static str> {
        let Some(entries) = doc.get(name) else {
            return Ok(Vec::new());
        };
        let Json::Array(entries) = entries else {
            return Err(MALFORMED);
        };
        entries
            .iter()
            .map(|e| {
                let text = e
                    .get("publicKey")
                    .and_then(Json::as_string)
                    .ok_or(MALFORMED)?;
                let der = base64::engine::general_purpose::STANDARD
                    .decode(text)
                    .map_err(|_| MALFORMED)?;
                RsaPublicKey::from_der(&der).map_err(|_| MALFORMED)
            })
            .collect()
    };
    Ok(ServicesKeySet {
        profile_property_keys: list("profilePropertyKeys")?,
        player_certificate_keys: list("playerCertificateKeys")?,
    })
}

/// The answer, authlib's NameAndId: `{"id", "name"}`, the id undashed.
pub fn found(answer: &str, name: &str) -> Option<NameAndId> {
    let profile = json::parse(answer, true).ok()?;
    let found = profile.get("name")?.as_string()?;
    let id = profile.get("id")?.as_string()?.replace('-', "");
    (found.eq_ignore_ascii_case(name) && id.len() == 32).then_some(())?;
    Some(NameAndId {
        id: u128::from_str_radix(&id, 16).ok()?,
        name: found,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_answers() {
        assert!(valid_player_name("Alex_123"));
        assert!(!valid_player_name("two words"));
        assert!(!valid_player_name("ThisNameIsTooLong"));
        assert!(!valid_player_name("caf\u{e9}"));
        let answer = r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch"}"#;
        assert_eq!(
            found(answer, "notch"),
            Some(NameAndId {
                id: 0x069a79f444e94726a5befca90e38aaf5,
                name: "Notch".into()
            })
        );
        assert_eq!(found(answer, "Steve"), None);
        assert_eq!(found("{}", "Notch"), None);
        assert_eq!(found("", "Notch"), None);
    }

    #[test]
    fn services_key_sets() {
        // The 1024-bit test key of sunrise-protocol's RSA tests, base64.
        let key = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQCrAmB0+SRQqPcm84lbsRF2yo8GXzEAzUFDusT302dO3P+VtBoFJdI2WlScEG7GaFiBs5IXdCD9Mf9cFE/xG6JDEedRzSxrXRgXtq9mhqQ8ldYSQ2EwBeaUW2CTzZUiPOxeiFaCbanS7hsyM2KsHOvqs2WcvEUxmptUDdqBakeY/wIDAQAB";
        let answer = format!(
            r#"{{"profilePropertyKeys":[{{"publicKey":"{key}"}}],"playerCertificateKeys":[{{"publicKey":"{key}"}},{{"publicKey":"{key}"}}]}}"#
        );
        let keys = profile_key_keys(&answer).unwrap();
        assert_eq!(keys.player_certificate_keys.len(), 2);
        assert_eq!(keys.profile_property_keys.len(), 1);
        assert_eq!(keys.profile_property_keys[0].size(), 128);
        let empty = profile_key_keys("{}").unwrap();
        assert!(empty.player_certificate_keys.is_empty());
        assert!(empty.profile_property_keys.is_empty());
        let bad = r#"{"profilePropertyKeys":[{"publicKey":"AAAA"}],"playerCertificateKeys":[]}"#;
        assert!(profile_key_keys(bad).is_err());
        assert!(profile_key_keys("not json").is_err());
    }
}
