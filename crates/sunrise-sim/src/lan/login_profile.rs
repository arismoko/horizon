//! `player_login_profile`: a login's properties, offered to the server thread once the login is
//! admitted and before `LoginFinished` sends them, so a mod can edit them.
use std::sync::mpsc::{Receiver, Sender, SyncSender, sync_channel};
use sunrise_core::profile::AccountProperty;

/// One admitted login's properties, waiting for the server thread's edit.
pub struct LoginProfileRequest {
    pub uuid: u128,
    pub name: String,
    pub properties: Vec<AccountProperty>,
    /// The properties to send; dropped unanswered, the login keeps its own.
    pub reply: SyncSender<Vec<AccountProperty>>,
}

/// `ByteBufCodecs.GAME_PROFILE_PROPERTIES`: at most 16 properties, a name of at most 64 UTF-16
/// units, a value of at most 32767 and a present signature of at most 1024 (`Utf8String`).
const MAX_PROPERTIES: usize = 16;
const MAX_NAME_UNITS: usize = 64;
const MAX_VALUE_UNITS: usize = 32767;
const MAX_SIGNATURE_UNITS: usize = 1024;

/// Whether the login packet can carry `properties`.
pub fn properties_fit(properties: &[AccountProperty]) -> bool {
    let units = |text: &str| text.encode_utf16().count();
    properties.len() <= MAX_PROPERTIES
        && properties.iter().all(|property| {
            units(&property.name) <= MAX_NAME_UNITS
                && units(&property.value) <= MAX_VALUE_UNITS
                && property
                    .signature
                    .as_deref()
                    .is_none_or(|signature| units(signature) <= MAX_SIGNATURE_UNITS)
        })
}

/// The properties a login sends: the server thread's edit when a mod listens, else its own.
/// Waits for one server turn; a server that stops first leaves the login its own properties.
pub(super) fn edited(
    requests: Option<&Sender<LoginProfileRequest>>,
    uuid: u128,
    name: &str,
    properties: Vec<AccountProperty>,
) -> Vec<AccountProperty> {
    let Some(requests) = requests else {
        return properties;
    };
    if !mod_api::__private::listened(mod_api::HookId::PlayerLoginProfile) {
        return properties;
    }
    let (reply, answer): (_, Receiver<_>) = sync_channel(1);
    let request = LoginProfileRequest {
        uuid,
        name: name.to_owned(),
        properties: properties.clone(),
        reply,
    };
    if requests.send(request).is_err() {
        return properties;
    }
    answer.recv().unwrap_or(properties)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(name: &str, value: &str, signature: Option<&str>) -> AccountProperty {
        AccountProperty {
            name: name.into(),
            value: value.into(),
            signature: signature.map(Into::into),
        }
    }

    /// The codec's bounds, counted in UTF-16 units: each at its limit fits, one more doesn't;
    /// an empty name and an empty present signature fit.
    #[test]
    fn properties_fit_the_login_codec_bounds() {
        let at = |c: &str, n: usize| c.repeat(n);
        assert!(properties_fit(&[property("", "", Some(""))]));
        assert!(properties_fit(&[property(
            &at("é", 64),
            &at("é", 32767),
            Some(&at("a", 1024))
        )]));
        assert!(!properties_fit(&[property(&at("é", 65), "", None)]));
        assert!(!properties_fit(&[property("n", &at("é", 32768), None)]));
        assert!(!properties_fit(&[property("n", "", Some(&at("a", 1025)))]));
        // A character outside the basic plane is two units.
        assert!(!properties_fit(&[property(&at("😀", 33), "", None)]));
        let sixteen = vec![property("textures", "v", None); 16];
        assert!(properties_fit(&sixteen));
        assert!(!properties_fit(
            &[sixteen, vec![property("x", "", None)]].concat()
        ));
    }
}
