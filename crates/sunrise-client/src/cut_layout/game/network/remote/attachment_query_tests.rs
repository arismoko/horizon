#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{AttachmentCodec, AttachmentDescriptor, AttachmentKey, Host};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::remote::{Holder, Tag};
use sunrise_mod_runtime::attachments::AttachmentSync;
use sunrise_protocol::attachment_sync::Message;
use {
    sunrise_client_gameplay::game::client_side::remote_client::session_state::Remote,
    sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry,
};

const VALUE_CODEC: AttachmentCodec<i32> = AttachmentCodec {
    encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
    decode: |c| match c.get(&"n".into()) {
        Some(Tag::Int(n)) => Ok(*n),
        _ => Err(mod_api::AttachmentError::Decode),
    },
};

struct MirrorMod(Arc<Mutex<Option<AttachmentKey<i32>>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for MirrorMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let key = r
            .register_attachment_type(
                AttachmentDescriptor::new("mirror:value", || 73i32)
                    .serialize(VALUE_CODEC)
                    .sync(VALUE_CODEC),
            )
            .unwrap();
        *self.0.lock().unwrap() = Some(key);
    }
}

fn values(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    key: &AttachmentKey<i32>,
) -> Vec<i32> {
    let host: &dyn Host = game;
    let batch = mod_api::AttachmentQueries::capture(host, key).unwrap();
    let query = batch.query(key).unwrap();
    let scope = host.attachment_read_scope().unwrap();
    let mut rows = query.iter();
    std::iter::from_fn(|| rows.try_next().unwrap())
        .map(|row| row.with_value(scope, |value| *value).unwrap())
        .collect()
}
fn client<'a>(game: &'a mut sunrise_client_gameplay::game::ClientGame<'_>) -> &'a mut Remote {
    game.client.remote.as_mut().unwrap()
}
fn sync(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, holder: Holder, value: i32) {
    game.remote_attachment_sync(Message {
        holder,
        replace: true,
        changes: vec![AttachmentSync {
            namespace: "mirror".into(),
            name: "value".into(),
            value: Some([("n".into(), Tag::Int(value))].into()),
        }],
    });
}
#[test]
#[ignore = "daily"]
fn synced_client_queries_follow_replacement_retirement_and_level_changes() {
    let key_slot = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "mirror",
        Box::new(MirrorMod(key_slot.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let value_key = key_slot
                .lock()
                .unwrap()
                .clone()
                .expect("MirrorMod registered");
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "QueryMirror").unwrap();
            let mut spawn = sunrise_protocol::packets::play::SpawnInfo {
                dimension_type: 0,
                dimension: "test:pocket".into(),
                seed: 0,
                game_type: 1,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            };
            game.enter_remote_dimension(
                &spawn,
                RemoteDimensionEntry::Login,
                sunrise_client_render::level_loading::Reason::Other,
            );
            client(game).logged_in = true;
            assert!(values(game, &value_key).is_empty());
            let host: &mut dyn Host = game;
            value_key.set(host, Holder::Level, 73).unwrap();
            assert_eq!(values(game, &value_key), [73]);
            client(game).attachments.clear();
            let host: &mut dyn Host = game;
            assert_eq!(value_key.get(host, Holder::Level, Clone::clone), Ok(73));
            assert_eq!(values(game, &value_key), [73]);
            client(game).attachments.clear();
            sync(game, Holder::Level, 41);
            assert_eq!(values(game, &value_key), [41]);
            let captured = mod_api::AttachmentQueries::capture(game, &value_key).unwrap();
            let query = captured.query(&value_key).unwrap();
            let row = query.iter().try_next().unwrap().unwrap();
            assert_eq!(
                row.holder,
                mod_api::AttachmentHolder::Level(game.level("test:pocket").unwrap())
            );
            let token = row.write_token();
            sync(game, Holder::Level, 42);
            assert_eq!(values(game, &value_key), [42]);
            assert_eq!(
                row.with_value(game.attachment_frame_borrow(), |value| *value),
                Ok(41)
            );
            client(game).attachments.remove(&Holder::Level);
            assert!(values(game, &value_key).is_empty());
            assert_eq!(
                game.attachment_validate_write(token),
                Err(mod_api::AttachmentRowError::StaleHolder)
            );
            sync(game, Holder::Chunk([1, 2]), 51);
            assert_eq!(values(game, &value_key), [51]);
            client(game).attachments.retain(|_, _| false);
            assert!(values(game, &value_key).is_empty());
            sync(game, Holder::Level, 61);
            client(game).attachments.clear();
            assert!(values(game, &value_key).is_empty());
            sync(game, Holder::Level, 71);
            spawn.dimension = "minecraft:the_nether".into();
            game.enter_remote_dimension(
                &spawn,
                RemoteDimensionEntry::Login,
                sunrise_client_render::level_loading::Reason::Other,
            );
            assert!(values(game, &value_key).is_empty());
            assert!(game.client.remote.as_ref().unwrap().attachments.is_empty());
            sync(game, Holder::Level, 81);
            assert_eq!(values(game, &value_key), [81]);
        });
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
