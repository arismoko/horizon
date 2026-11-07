//! Load-owned particle identities and the resolved provider table.
use horizon_mod_runtime::registry_keys::{ParticleKind, Particles};
use std::sync::OnceLock;

/// A particle requested at registration, resolved once when providers load.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the load-owned table creates keys
#[horizon_mod_api_macros::wasm(host_constructor = from_slot)]
pub struct ParticleKey(pub(crate) u32);

fn resolved() -> &'static OnceLock<Particles> {
    static EMPTY: OnceLock<Particles> = OnceLock::new();
    crate::parts::installed().map_or(&EMPTY, |parts| &parts.particles)
}
impl ParticleKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|registering| {
            let names = match registering {
                Some(requests) => &requests.particles,
                None => resolved()
                    .get()
                    .map_or(&crate::parts::requested_keys().particles, |p| &p.names),
            };
            names.id(slot).ok_or("unknown ParticleKey slot")?;
            Ok(Self(slot))
        })
    }
}

/// Resolve provider names once at load, before publishing either names or providers.
// hidden: only the game supplies the completed particle registry.
#[doc(hidden)]
pub fn resolve_particles<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<ParticleKind>,
) -> Result<(), Vec<String>> {
    let particles = horizon_mod_runtime::registry_keys::resolve_particles(
        &crate::parts::requested_keys().particles,
        ids,
        resolve,
    )?;
    resolved()
        .set(particles)
        .map_err(|_| vec!["particle registry already resolved".to_owned()])
}

/// Find a registered particle without allocating or parsing a name.
// hidden: generated metadata forwarders call this lookup at the owning load boundary.
#[doc(hidden)]
pub fn particle_key(id: &crate::Identifier) -> Option<ParticleKey> {
    Some(ParticleKey(*resolved().get()?.names.identifiers.get(id)?))
}

/// Read the already-resolved provider identity for a retained key.
// hidden: the game's particle conversion boundary consumes the provider index.
#[doc(hidden)]
pub fn particle_kind(key: ParticleKey) -> Option<ParticleKind> {
    resolved().get()?.kinds.get(key.0 as usize).copied()
}

/// Borrow a custom provider's name; no owned string is created.
// hidden: the game's particle conversion boundary sends custom provider names.
#[doc(hidden)]
pub fn particle_name(key: ParticleKey) -> Option<&'static str> {
    resolved().get()?.names.id(key.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn particle_load_refuses_atomically_and_publishes_provider_indices() {
        let registrar = crate::parts::PartRegistrar::new("test");
        registrar
            .registry_keys
            .borrow_mut()
            .particles
            .request("test", "test:spark")
            .unwrap();
        crate::parts::install_registrar(registrar, &crate::parts::tests::rule_schema()).unwrap();
        let id = crate::Identifier::parse_namespaced("test:spark").unwrap();
        assert!(
            resolve_particles(["minecraft:smoke"], |_| Some(ParticleKind::Vanilla(7))).is_err()
        );
        assert!(particle_key(&id).is_none());
        let calls = std::cell::Cell::new(0);
        resolve_particles(["minecraft:smoke", "test:spark"], |name| {
            calls.set(calls.get() + 1);
            Some(if name == "test:spark" {
                ParticleKind::Custom
            } else {
                ParticleKind::Vanilla(7)
            })
        })
        .unwrap();
        let key = particle_key(&id).unwrap();
        assert_eq!(particle_name(key), Some("test:spark"));
        assert_eq!(particle_kind(key), Some(ParticleKind::Custom));
        let smoke =
            particle_key(&crate::Identifier::parse_namespaced("minecraft:smoke").unwrap()).unwrap();
        assert_eq!(particle_kind(smoke), Some(ParticleKind::Vanilla(7)));
        assert_eq!(calls.get(), 2);
        assert_eq!(ParticleKey::from_slot(key.0), Ok(key));
        assert!(ParticleKey::from_slot(u32::MAX).is_err());
        assert!(resolve_particles(["test:spark"], |_| Some(ParticleKind::Custom)).is_err());
        assert_eq!(particle_kind(smoke), Some(ParticleKind::Vanilla(7)));
        assert_eq!(calls.get(), 2);
    }
}
