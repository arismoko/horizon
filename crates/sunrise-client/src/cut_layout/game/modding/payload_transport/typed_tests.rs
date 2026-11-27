#![cfg(any())]

//! Typed payloads (#813 step 4b-1) on the real routes: the integrated server over memory and a
//! published server over TCP. The values are decoded by the typed adapter and encoded by the
//! token sends; the expected bodies are written out by hand.
use mod_api::wire::{Reader, Wire, WireError, WireSchema, Writer};
#[cfg(test)]
use mod_api::{
    __private::{HookPanicReport, take_hook_panics},
    ModEntry,
};
use mod_api::{
    Hooks, IPayloadContext, PayloadAccepted, PayloadId, PayloadSendError, PlayerRef, Registrar,
    ToClient, ToServer, Wire,
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
use sunrise_protocol::packets::{configuration as cfg, play};

const NAME: &str =
    "cut_layout::game::payload_transport::typed_tests::typed_payloads_follow_the_raw_routes";
const UP: &str = "typed_probe:up";
const OTHER: &str = "typed_probe:other";
const DOWN: &str = "typed_probe:down";
const STAGED: f32 = 3.0;

static ENCODES: AtomicUsize = AtomicUsize::new(0);
static DECODES: AtomicUsize = AtomicUsize::new(0);

/// An i32 that counts its codec calls.
#[derive(Debug, PartialEq)]
struct Counted(i32);
impl Wire for Counted {
    const MIN: usize = <i32 as Wire>::MIN;
    fn put(&self, out: &mut Writer) {
        ENCODES.fetch_add(1, Ordering::SeqCst);
        out.put(&self.0);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        DECODES.fetch_add(1, Ordering::SeqCst);
        input.take().map(Self)
    }
    fn describe(schema: &mut WireSchema) -> u64 {
        i32::describe(schema)
    }
}

#[derive(Debug, PartialEq, Wire)]
struct Probe {
    counter: Counted,
    labels: Vec<String>,
    chain: Chain,
}
#[derive(Debug, PartialEq, Wire)]
struct Chain(Option<Box<Chain>>);

/// The same ids' value with another shape.
#[derive(Debug, PartialEq, Wire)]
struct Other(i32);

fn probe(counter: i32) -> Probe {
    Probe {
        counter: Counted(counter),
        labels: Vec::new(),
        chain: Chain(None),
    }
}

struct Tokens {
    up: ToServer<Probe>,
    other: ToServer<Probe>,
    down: ToClient<Probe>,
}
static TOKENS: Mutex<Option<Tokens>> = Mutex::new(None);
/// What the server handlers saw: channel, counter and the first peer's health then.
static SERVER: Mutex<Vec<(&str, i32, Option<f32>)>> = Mutex::new(Vec::new());
static CLIENT: Mutex<Vec<i32>> = Mutex::new(Vec::new());
/// UUID and entity ID independently observed by the single-player client.
static OWNER: Mutex<Option<(u128, u32)>> = Mutex::new(None);
/// Handler errors must reach the test before the payload wait times out.
static OWNER_RESULT: Mutex<Option<Result<(), String>>> = Mutex::new(None);
/// The first player to send 7, whose health the failing handler stages a write to.
static WITNESS: Mutex<Option<PlayerRef>> = Mutex::new(None);

fn down() -> ToClient<Probe> {
    TOKENS.lock().unwrap().as_ref().unwrap().down.clone()
}

fn witness_health(context: &mut dyn IPayloadContext) -> Option<f32> {
    let witness = (*WITNESS.lock().unwrap())?;
    let host = context.server_host().ok()?;
    host.living(witness.entity())?.health(host)
}

fn up(value: Probe, context: &mut dyn IPayloadContext) -> Result<(), String> {
    assert_eq!(std::thread::current().name(), Some("Server thread"));
    let counter = value.counter.0;
    if counter == 7 {
        let player = context.player().map_err(|e| format!("{e:?}"))?;
        WITNESS.lock().unwrap().get_or_insert(player);
    }
    let health = witness_health(context);
    SERVER.lock().unwrap().push((UP, counter, health));
    match counter {
        21 => {
            let result = (|| {
                let sender = context.player().map_err(|e| format!("{e:?}"))?;
                let host = context.server_host().map_err(|e| format!("{e:?}"))?;
                let players = host.players().server_players();
                if players != [sender] {
                    return Err(format!(
                        "expected the owner once, got {players:?}; sender {sender:?}"
                    ));
                }
                let owner = players[0];
                let expected = *OWNER.lock().unwrap();
                if Some((owner.uuid(), owner.entity().id())) != expected {
                    return Err(format!(
                        "listed owner {owner:?} differs from client {expected:?}"
                    ));
                }
                let name: &str = owner
                    .name(host)
                    .ok_or("connected owner's account name missing")?;
                let found = host.players().player_by_name(&name.to_uppercase());
                if found != Some(owner) {
                    return Err(format!(
                        "name lookup returned {found:?}, expected {owner:?}"
                    ));
                }
                down()
                    .send(host, owner, &probe(22))
                    .map(drop)
                    .map_err(|e| format!("send to listed owner: {e:?}"))
            })();
            *OWNER_RESULT.lock().unwrap() = Some(result.clone());
            result
        }
        7 => down()
            .reply(context, &probe(8))
            .map(drop)
            .map_err(|e| format!("reply: {e:?}")),
        13 => {
            let witness = WITNESS.lock().unwrap().expect("a witness sent 7 first");
            let host = context.server_host().map_err(|e| format!("{e:?}"))?;
            let living = host.living(witness.entity()).ok_or("no witness")?;
            if !living.set_health(host, STAGED) || living.health(host) != Some(STAGED) {
                return Err("could not stage witness health".into());
            }
            if down().reply(context, &probe(14)) != Ok(PayloadAccepted::Queued) {
                return Err("could not stage panic reply".into());
            }
            panic!("typed handler panics after a staged write and a staged reply");
        }
        _ => Ok(()),
    }
}

/// A healthy observer on its own callback: records the witness's health and echoes the counter.
fn other(value: Probe, context: &mut dyn IPayloadContext) -> Result<(), String> {
    let counter = value.counter.0;
    let health = witness_health(context);
    SERVER.lock().unwrap().push((OTHER, counter, health));
    down()
        .reply(context, &probe(counter))
        .map(drop)
        .map_err(|e| format!("reply: {e:?}"))
}

fn client(value: Probe, _: &mut dyn IPayloadContext) -> Result<(), String> {
    CLIENT.lock().unwrap().push(value.counter.0);
    Ok(())
}

struct Registration;
#[mod_api::__subscriptions]
impl Hooks for Registration {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let mut payloads = registrar.payload_handlers();
        let mut view = payloads.registrar("1");
        let tokens = Tokens {
            up: view.play_to_server_typed(PayloadId(UP.into()), up),
            other: view.play_to_server_typed(PayloadId(OTHER.into()), other),
            down: view.play_to_client_typed(PayloadId(DOWN.into()), client),
        };
        *TOKENS.lock().unwrap() = Some(tokens);
    }
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn typed_payloads_follow_the_raw_routes() {
    super::tests::require_resources();
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "SUNRISE_TYPED_TRANSPORT_CHILD",
        &["integrated", "singleplayer_list", "lan"],
    ) {
        return;
    }
    let case = std::env::var("SUNRISE_TYPED_TRANSPORT_CHILD").unwrap();
    let mods = vec![ModEntry::new("typed_probe", Box::new(Registration))].into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            game.client.settings.render_distance = 2;
            game.client.settings.simulation_distance = 2;
            let options = sunrise_sim::world_options::WorldOptions {
                world_type: sunrise_sim::save::WorldPreset::Flat,
                ..Default::default()
            };
            game.open_integrated("typed", Some("Typed"), Some(1), true, Some(&options))
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            while !game
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.logged_in && r.joined)
            {
                game.poll_remote().unwrap();
                assert!(Instant::now() < deadline, "integrated play not ready");
                std::thread::yield_now();
            }
            match case.as_str() {
                "integrated" => integrated(game),
                "singleplayer_list" => singleplayer_list(game),
                "lan" => lan(game),
                other => panic!("unknown case {other}"),
            }
            game.leave_server(None).unwrap();
        });
    });
}

/// Exercise enumeration and a handle-addressed send, not a reply to the sender.
fn singleplayer_list(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    assert_eq!(game.client.integrated.as_ref().unwrap().published(), None);
    let owner = game.local_player().expect("logged-in local player");
    *OWNER.lock().unwrap() = Some((owner.uuid(), owner.entity().id()));
    let token = TOKENS.lock().unwrap().as_ref().unwrap().up.clone();
    assert_eq!(token.send(game, &probe(21)), Ok(PayloadAccepted::Queued));
    let deadline = Instant::now() + Duration::from_secs(60);
    while CLIENT.lock().unwrap().is_empty() {
        game.poll_remote().unwrap();
        if let Some(Err(error)) = OWNER_RESULT.lock().unwrap().as_ref() {
            panic!("single-player owner check failed: {error}");
        }
        assert!(
            Instant::now() < deadline,
            "payload to listed single-player owner"
        );
        std::thread::yield_now();
    }
    assert_eq!(*CLIENT.lock().unwrap(), [22]);
    assert_eq!(game.client.integrated.as_ref().unwrap().published(), None);
}

fn integrated(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    // A declared typed payload costs nothing while no message is sent.
    let start = game.world.game_time;
    let deadline = Instant::now() + Duration::from_secs(60);
    while game.world.game_time < start + 100 {
        game.poll_remote().unwrap();
        assert!(Instant::now() < deadline, "100 server ticks");
        std::thread::yield_now();
    }
    assert_eq!(
        (
            ENCODES.load(Ordering::SeqCst),
            DECODES.load(Ordering::SeqCst)
        ),
        (0, 0)
    );
    let (up, other) = {
        let tokens = TOKENS.lock().unwrap();
        let tokens = tokens.as_ref().unwrap();
        (tokens.up.clone(), tokens.other.clone())
    };
    // Forged tokens: another slot, and the same claim read as another value type.
    let mut out = mod_api::wire::Writer::new();
    mod_api::wire::Wire::put(up.token(), &mut out);
    let bytes = out.into_bytes();
    let mut forged = mod_api::wire::Reader::decode::<mod_api::PayloadToken>(&bytes).unwrap();
    forged.slot = other.token().slot;
    let mut out = mod_api::wire::Writer::new();
    mod_api::wire::Wire::put(&forged, &mut out);
    let forged: ToServer<Probe> = mod_api::wire::Reader::decode(&out.into_bytes()).unwrap();
    let retyped: ToServer<Other> = mod_api::wire::Reader::decode(&bytes).unwrap();
    let host: &mut dyn mod_api::ClientHost = game;
    let id = PayloadId(UP.into());
    assert_eq!(
        forged.send(host, &probe(5)),
        Err(PayloadSendError::StaleToken(id.clone()))
    );
    assert_eq!(
        retyped.send(host, &Other(5)),
        Err(PayloadSendError::StaleToken(id))
    );
    assert_eq!(
        ENCODES.load(Ordering::SeqCst),
        0,
        "a stale token encodes nothing"
    );
    for (token, counter) in [(&up, 1), (&other, 2), (&up, 3), (&up, 7)] {
        assert_eq!(
            token.send(host, &probe(counter)),
            Ok(PayloadAccepted::Queued)
        );
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while CLIENT.lock().unwrap().len() < 2 {
        game.poll_remote().unwrap();
        assert!(Instant::now() < deadline, "typed reply");
        std::thread::yield_now();
    }
    let seen: Vec<_> = SERVER
        .lock()
        .unwrap()
        .iter()
        .map(|(id, counter, _)| (*id, *counter))
        .collect();
    assert_eq!(seen, [(UP, 1), (OTHER, 2), (UP, 3), (UP, 7)]);
    assert_eq!(*CLIENT.lock().unwrap(), [2, 8]);
}

#[cfg(any())]
fn lan(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    let port = game
        .client
        .integrated
        .as_ref()
        .unwrap()
        .call(|server| {
            server.publish(sunrise_gameplay::game::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            })
        })
        .expect("integrated server answered publication request")
        .unwrap();
    // i32 counter, empty list, then an absent optional child.
    let empty = |low_byte| [low_byte, 0, 0, 0, 0, 0, 0, 0, 0];
    let nested = |levels: usize| {
        let mut body = empty(7)[..8].to_vec();
        body.extend(std::iter::repeat_n(1, levels));
        body.push(0);
        body
    };
    // Each malformed body ends its own session by name, with no handler call and no reply.
    for (index, body) in [
        empty(7)[..5].to_vec(),          // truncated list length
        vec![7, 0, 0, 0, 1, 0x10, 0, 0], // 4097 labels
        nested(33),
        [empty(7).as_slice(), &[0xaa]].concat(),
    ]
    .into_iter()
    .enumerate()
    {
        let mut peer = super::sender_tests::connect_peer(port, &format!("Malformed{index}"));
        send(&mut peer, UP, body);
        let reason = await_disconnect(&mut peer, DOWN);
        assert!(reason.contains("typed_probe"), "{reason}");
    }
    // A clientbound-only channel sent by a client: a forged direction, no handler runs.
    let mut forged = super::sender_tests::connect_peer(port, "ForgedDirection");
    send(&mut forged, DOWN, empty(7).to_vec());
    await_disconnect(&mut forged, DOWN);
    assert!(SERVER.lock().unwrap().is_empty());
    assert!(CLIENT.lock().unwrap().is_empty());
    // An independent session is accepted: 7, then the order A=1, B=2, A=3, then 7 again,
    // whose reply orders them before any other connection's message.
    let mut witness = super::sender_tests::connect_peer(port, "TypedWitness");
    send(&mut witness, UP, empty(7).to_vec());
    assert_eq!(await_reply(&mut witness, DOWN), empty(8));
    let full = SERVER.lock().unwrap()[0].2.expect("the witness's health");
    send(&mut witness, UP, empty(1).to_vec());
    send(&mut witness, OTHER, empty(2).to_vec());
    send(&mut witness, UP, empty(3).to_vec());
    send(&mut witness, UP, empty(7).to_vec());
    assert_eq!(await_reply(&mut witness, DOWN), empty(2));
    assert_eq!(await_reply(&mut witness, DOWN), empty(8));
    // The next callback on this peer runs after the panic. Its reply must come first:
    // the staged reply is discarded without closing the connection.
    let mut failing = super::sender_tests::connect_peer(port, "TypedPanic");
    send(&mut failing, UP, empty(13).to_vec());
    send(&mut failing, OTHER, empty(4).to_vec());
    assert_eq!(await_reply(&mut failing, DOWN), empty(4));
    assert_eq!(
        SERVER.lock().unwrap().last().copied(),
        Some((OTHER, 4, Some(full))),
        "the failed handler's write was rolled back"
    );
    // A fresh peer still negotiates the channel, but skips the disabled callback.
    let mut later = super::sender_tests::connect_peer(port, "TypedLater");
    send(&mut later, UP, empty(7).to_vec());
    send(&mut later, OTHER, empty(5).to_vec());
    assert_eq!(await_reply(&mut later, DOWN), empty(5));
    let seen: Vec<_> = SERVER
        .lock()
        .unwrap()
        .iter()
        .map(|(id, counter, _)| (*id, *counter))
        .collect();
    assert_eq!(
        seen,
        [
            (UP, 7),
            (UP, 1),
            (OTHER, 2),
            (UP, 3),
            (UP, 7),
            (UP, 13),
            (OTHER, 4),
            (OTHER, 5),
        ]
    );
    assert_eq!(
        take_hook_panics(),
        [HookPanicReport {
            mod_id: "typed_probe",
            hook: "payload",
        }]
    );
}

/// Send `data` on `channel` from a TCP peer in play.
pub(super) fn send(
    peer: &mut sunrise_protocol::connection::Connection,
    channel: &str,
    data: Vec<u8>,
) {
    peer.send(
        &play::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: channel.into(),
            data,
        })
        .encode(),
    )
    .unwrap();
}

/// The next payload on `channel`, answering keep-alives on the way.
pub(super) fn await_reply(
    peer: &mut sunrise_protocol::connection::Connection,
    channel: &str,
) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(Instant::now() < deadline, "typed reply deadline");
        let (id, body) = peer.recv().unwrap();
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::CustomPayload(p) if p.channel == channel => return p.data,
            play::Clientbound::KeepAlive(p) => peer
                .send(&play::Serverbound::KeepAlive(p).encode())
                .unwrap(),
            play::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
}

/// The session's disconnect reason, or how its stream closed; a payload on `channel` before
/// it fails the test.
pub(super) fn await_disconnect(
    peer: &mut sunrise_protocol::connection::Connection,
    channel: &str,
) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(Instant::now() < deadline, "disconnect deadline");
        let (id, body) = match peer.recv() {
            Ok(packet) => packet,
            // A protocol refusal closes the stream without a reason packet.
            Err(error) => return format!("closed: {error}"),
        };
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::CustomPayload(p) if p.channel == channel => {
                panic!("a reply was sent before the refusal: {:?}", p.data)
            }
            play::Clientbound::KeepAlive(p) => peer
                .send(&play::Serverbound::KeepAlive(p).encode())
                .unwrap(),
            play::Clientbound::Disconnect(p) => {
                let lang = [("disconnect.genericReason".to_owned(), "%s".to_owned())].into();
                return sunrise_protocol::text::plain(&p.reason, &lang);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
