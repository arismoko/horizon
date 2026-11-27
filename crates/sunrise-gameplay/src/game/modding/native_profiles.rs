//! The profiles service: account profiles through the server's `ProfileResolver.Cached`, the
//! resolver `/fetchprofile` and dynamic player heads use.
use crate::game::ServerGame;
use mod_api::{AccountProfile, AccountProfileProperty, ProfileResolution};
use sunrise_level::profile_lookup::{NameOrId, Resolve};

impl ServerGame<'_> {
    /// Where the lookup of `key` stands, starting it the first time.
    pub(in crate::game) fn native_resolve_profile(&self, key: NameOrId) -> ProfileResolution {
        match self.server.profile_resolver.lookup(&key) {
            Resolve::Pending => ProfileResolution::Pending,
            Resolve::Done(profile) => ProfileResolution::Done(profile.map(|profile| {
                AccountProfile {
                    uuid: profile.id,
                    name: profile.name,
                    properties: profile
                        .properties
                        .into_iter()
                        .map(|(name, value, signature)| AccountProfileProperty {
                            name,
                            value,
                            signature,
                        })
                        .collect(),
                }
            })),
        }
    }
}
