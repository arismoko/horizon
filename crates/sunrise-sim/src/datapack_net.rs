//! The registries of mods' datapack registries on the wire (`sunrise:datapack_registries`,
//! arismoko/horizon#811, stage 4): at the join, a Sunrise client offers the synchronized
//! registries it declared, with their schemas (`sunrise:datapack_offer`), and the server answers
//! with a snapshot of the ones both sides have, before any registry data. A required registry
//! the client did not offer ends the join with a reason; an optional one is left out and
//! nothing is sent in its place. The client checks the snapshot against its own declarations
//! before it keeps anything, and a snapshot that fails ends the join. The server publishes its set for joining clients; each client configuration owns
//! the set it received.
//!
//! A `/reload` that publishes a new set sends each joined Sunrise client a new snapshot
//! on the same channel, built from the offer owned by its live connection. The
//! client checks it as at the join and swaps its set in one step, or ends the connection.
use std::sync::{Arc, Mutex};
use sunrise_protocol::packets::custom_payload::SERVERBOUND_LIMIT;

/// The snapshot, server to client.
pub const CHANNEL: &str = "sunrise:datapack_registries";
/// The client's offer, client to server.
pub const OFFER_CHANNEL: &str = "sunrise:datapack_offer";

type Held = Mutex<Option<Arc<mod_api::DatapackRegistrySet>>>;
/// What this process's server last published.
static SERVED: Held = Mutex::new(None);

#[cfg(feature = "test-publication-scope")]
thread_local! {
    static SCOPED_SERVED: std::cell::RefCell<Option<Option<Arc<mod_api::DatapackRegistrySet>>>> = const { std::cell::RefCell::new(None) };
}

/// Isolates this thread's publication for resource-backed game fixtures.
#[cfg(feature = "test-publication-scope")]
pub fn isolate_server_publication() -> impl Drop {
    struct Scope(
        Option<Option<Arc<mod_api::DatapackRegistrySet>>>,
        std::marker::PhantomData<std::rc::Rc<()>>,
    );
    impl Drop for Scope {
        fn drop(&mut self) {
            SCOPED_SERVED.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    Scope(
        SCOPED_SERVED.with(|slot| slot.replace(Some(None))),
        std::marker::PhantomData,
    )
}
/// The snapshot of the set now served for `player`, from the offer they made at the join, when
/// there is one to send.
pub fn reload_payload(offer: Option<&[u8]>) -> Step {
    server_step(true, offer)
}

fn held(slot: &Held) -> Option<Arc<mod_api::DatapackRegistrySet>> {
    slot.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn put(slot: &Held, set: Option<Arc<mod_api::DatapackRegistrySet>>) {
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = set;
}

/// Keeps the set a server just published, for the clients that join it.
pub fn serve(set: Option<Arc<mod_api::DatapackRegistrySet>>) {
    #[cfg(feature = "test-publication-scope")]
    if SCOPED_SERVED.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(local) = slot.as_mut() {
            *local = set.clone();
            true
        } else {
            false
        }
    }) {
        return;
    }
    put(&SERVED, set);
}

/// The set this process's server last published.
pub fn served() -> Option<Arc<mod_api::DatapackRegistrySet>> {
    #[cfg(feature = "test-publication-scope")]
    if let Some(local) = SCOPED_SERVED.with(|slot| slot.borrow().clone()) {
        return local;
    }
    held(&SERVED)
}

/// What the client sends its server: its synchronized registries and their schemas, or `Ok(None)`
/// when it declared none. A failed encoding ends configuration rather than posing as no offer.
pub fn offer() -> Result<Option<Vec<u8>>, String> {
    let declarations = mod_api::__private::datapack_registry_declarations();
    if declarations
        .iter()
        .all(|declaration| declaration.sync == mod_api::DatapackRegistrySync::ServerOnly)
    {
        return Ok(None);
    }
    mod_api::DatapackSnapshot::encode_offer(declarations)
        .map(Some)
        .ok_or_else(|| {
            format!(
                "This client's datapack registry offer could not be encoded within the {SERVERBOUND_LIMIT}-byte serverbound custom payload limit."
            )
        })
}

/// What the server does for a joining client.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing: no mod declared a synchronized registry, or none applies to this client.
    Nothing,
    /// Send these bytes on [`CHANNEL`], before the registry data.
    Send(Vec<u8>),
    /// End the join with this reason, before any payload.
    Refuse(String),
}

/// The server's step for a client of the given brand, with the offer it sent if it sent one.
pub fn server_step(sunrise_client: bool, offer: Option<&[u8]>) -> Step {
    let declarations = mod_api::__private::datapack_registry_declarations();
    step_for(declarations, served().as_deref(), sunrise_client, offer)
}

fn step_for(
    declarations: &[mod_api::DatapackRegistryDeclaration],
    set: Option<&mod_api::DatapackRegistrySet>,
    sunrise_client: bool,
    offer: Option<&[u8]>,
) -> Step {
    use mod_api::DatapackRegistrySync;
    if declarations
        .iter()
        .all(|declaration| declaration.sync == DatapackRegistrySync::ServerOnly)
    {
        return Step::Nothing;
    }
    let Some(set) = set else {
        return Step::Nothing;
    };
    // A vanilla client, or a Sunrise one that sent no offer, has no registries to match.
    let offered = match (sunrise_client, offer) {
        (true, Some(bytes)) => match mod_api::DatapackSnapshot::decode_offer(bytes) {
            Ok(offered) => offered,
            Err(_) => return Step::Refuse("The client's mod registries are unreadable.".into()),
        },
        _ => Vec::new(),
    };
    match set.snapshot_for(declarations, &offered) {
        Err(mod_api::DatapackNegotiationError::RequiredRegistry { registry, mod_id }) => {
            Step::Refuse(format!(
                "Install the mod {mod_id} of the server: it needs the registry {registry}."
            ))
        }
        Err(_) => Step::Refuse("The client's mod registries are unreadable.".into()),
        Ok(snapshot) if snapshot.registries.is_empty() => Step::Nothing,
        Ok(snapshot) => match snapshot.encode() {
            Some(bytes) => Step::Send(bytes),
            None => Step::Refuse(format!(
                "The server's datapack registry snapshot for this client could not be encoded within the {}-byte transport limit.",
                mod_api::MAX_SNAPSHOT_BYTES
            )),
        },
    }
}

/// Validates a received snapshot against this client's declarations and builds the replacement set.
fn receive(
    declarations: &[mod_api::DatapackRegistryDeclaration],
    bytes: &[u8],
) -> Result<mod_api::DatapackRegistrySet, String> {
    let refused = |error: mod_api::DatapackSnapshotError| {
        format!("The server's mod registries do not match this client's: {error:?}")
    };
    let snapshot = mod_api::DatapackSnapshot::decode(bytes).map_err(refused)?;
    let candidate = snapshot.candidate(declarations).map_err(refused)?;
    Ok(mod_api::DatapackRegistrySet::received(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{
        DatapackCandidate, DatapackField, DatapackFieldKind, DatapackRegistryDeclaration,
        DatapackRegistrySet, DatapackRegistrySync,
    };

    fn spell(sync: DatapackRegistrySync) -> DatapackRegistryDeclaration {
        DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:spell".into(),
            fields: vec![DatapackField::required(
                "power",
                DatapackFieldKind::Int { min: 0, max: 100 },
            )],
            sync,
        }
    }

    fn set(declaration: &DatapackRegistryDeclaration) -> DatapackRegistrySet {
        let files = vec![(
            "data/addon/example/spell/fire.json".to_owned(),
            vec![("pack".to_owned(), br#"{"power": 7}"#.to_vec())],
        )];
        DatapackRegistrySet::from_candidate(
            DatapackCandidate::build(
                std::slice::from_ref(declaration),
                &files,
                &mut mod_api::__private::NoVanillaValueDecoder,
            )
            .unwrap(),
        )
    }

    #[test]
    fn a_client_with_the_mod_receives_the_registry_and_reads_it() {
        let declaration = spell(DatapackRegistrySync::Required);
        let declarations = std::slice::from_ref(&declaration);
        let server = set(&declaration);
        let offer = mod_api::DatapackSnapshot::encode_offer(declarations).unwrap();
        let Step::Send(bytes) = step_for(declarations, Some(&server), true, Some(&offer)) else {
            panic!("a snapshot is sent");
        };
        let client = receive(declarations, &bytes).unwrap();
        assert_eq!(
            client
                .get("example:spell", "addon:fire")
                .unwrap()
                .get("power"),
            Some(&mod_api::DatapackValue::Int(7))
        );
        // Truncated bytes are refused and nothing is kept.
        assert!(receive(declarations, &bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn a_reload_snapshot_replaces_the_set() {
        let declaration = spell(DatapackRegistrySync::Required);
        let declarations = std::slice::from_ref(&declaration);
        let first = set(&declaration);
        let offer = mod_api::DatapackSnapshot::encode_offer(declarations).unwrap();
        let send = |server: &DatapackRegistrySet| {
            let Step::Send(bytes) = step_for(declarations, Some(server), true, Some(&offer)) else {
                panic!("a snapshot is sent");
            };
            bytes
        };
        let joined = receive(declarations, &send(&first)).unwrap();
        assert_eq!(
            joined
                .get("example:spell", "addon:fire")
                .unwrap()
                .get("power"),
            Some(&mod_api::DatapackValue::Int(7))
        );
        let reloaded = DatapackRegistrySet::from_candidate(
            DatapackCandidate::build(
                declarations,
                &[(
                    "data/addon/example/spell/fire.json".to_owned(),
                    vec![("pack".to_owned(), br#"{"power": 9}"#.to_vec())],
                )],
                &mut mod_api::__private::NoVanillaValueDecoder,
            )
            .unwrap(),
        );
        let bytes = send(&reloaded);
        let next = receive(declarations, &bytes).unwrap();
        assert_eq!(
            next.get("example:spell", "addon:fire")
                .unwrap()
                .get("power"),
            Some(&mod_api::DatapackValue::Int(9))
        );
        assert_eq!(
            joined
                .get("example:spell", "addon:fire")
                .unwrap()
                .get("power"),
            Some(&mod_api::DatapackValue::Int(7))
        );
    }

    #[test]
    fn a_reload_cannot_change_which_registries_a_client_is_sent() {
        let declaration = spell(DatapackRegistrySync::Required);
        let declarations = std::slice::from_ref(&declaration);
        let first = set(&declaration);
        let reloaded = DatapackRegistrySet::from_candidate(
            DatapackCandidate::build(
                declarations,
                &[],
                &mut mod_api::__private::NoVanillaValueDecoder,
            )
            .unwrap(),
        );
        let offer = mod_api::DatapackSnapshot::decode_offer(
            &mod_api::DatapackSnapshot::encode_offer(declarations).unwrap(),
        )
        .unwrap();
        let shape = |set: &DatapackRegistrySet| {
            let snapshot = set.snapshot_for(declarations, &offer).unwrap();
            snapshot
                .registries
                .iter()
                .map(|registry| registry.key.clone())
                .collect::<Vec<_>>()
        };
        // The entries go, the registry (and so the negotiation) stays.
        assert_eq!(shape(&first), shape(&reloaded));
        assert_eq!(shape(&reloaded), vec!["example:spell".to_owned()]);
    }

    #[test]
    fn a_required_registry_refuses_a_client_that_lacks_it() {
        let declaration = spell(DatapackRegistrySync::Required);
        let declarations = std::slice::from_ref(&declaration);
        let server = set(&declaration);
        for (sunrise, offer) in [(false, None), (true, None)] {
            let Step::Refuse(reason) = step_for(declarations, Some(&server), sunrise, offer) else {
                panic!("the join is refused");
            };
            assert!(reason.contains("example") && reason.contains("example:spell"));
        }
        // Bytes that are not an offer are refused too.
        assert!(matches!(
            step_for(declarations, Some(&server), true, Some(&[1, 2, 3])),
            Step::Refuse(_)
        ));
    }

    #[test]
    fn optional_and_server_only_registries_never_end_a_join() {
        let required = spell(DatapackRegistrySync::Required);
        let offer =
            mod_api::DatapackSnapshot::encode_offer(std::slice::from_ref(&required)).unwrap();
        let oversized = DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:oversized".into(),
            // This schema alone, before any entry bytes or Wire framing, fills the snapshot
            // budget. The peer offers only the small required registry.
            fields: vec![DatapackField::with_default(
                "body",
                DatapackFieldKind::Text {
                    max_len: mod_api::MAX_SNAPSHOT_BYTES as u32,
                },
                mod_api::DatapackValue::Text("x".repeat(mod_api::MAX_SNAPSHOT_BYTES)),
            )],
            sync: DatapackRegistrySync::Optional,
        };
        let mut declarations = [required, oversized];
        let server = DatapackRegistrySet::from_candidate(
            DatapackCandidate::build(
                &declarations,
                &[],
                &mut mod_api::__private::NoVanillaValueDecoder,
            )
            .unwrap(),
        );
        for sync in [
            DatapackRegistrySync::Optional,
            DatapackRegistrySync::ServerOnly,
        ] {
            declarations[1].sync = sync;
            let Step::Send(bytes) = step_for(&declarations, Some(&server), true, Some(&offer))
            else {
                panic!("only the negotiated registry should be encoded");
            };
            let received = mod_api::DatapackSnapshot::decode(&bytes).unwrap();
            assert_eq!(received.registries.len(), 1);
            assert_eq!(received.registries[0].key, "example:spell");
            assert_eq!(
                step_for(
                    &declarations[1..],
                    Some(&server),
                    sync == DatapackRegistrySync::ServerOnly,
                    None,
                ),
                Step::Nothing
            );
        }
        // No world published yet: nothing to send.
        assert_eq!(
            step_for(&declarations, None, true, Some(&offer)),
            Step::Nothing
        );
    }
}

#[path = "datapack_delivery.rs"]
mod delivery;
pub use delivery::reload_connection;

#[path = "datapack_transport.rs"]
mod transport;
pub use transport::{Receiver, chunks};
