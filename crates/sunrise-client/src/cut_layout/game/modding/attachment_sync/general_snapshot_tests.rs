//! A general row's attachment snapshot (NeoForge `AttachmentSync`): no empty packet when
//! nothing is synced, and a `replace: true` snapshot with the current value when synced
//! attachments are present.
use super::transport_tests::VALUE_CODEC;
use mod_api::{AttachmentKey, GeneralEntityCategory, GeneralSavePolicy};
use std::sync::{Arc, Mutex};
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_mod_runtime::attachments::Holder;
use {
    sunrise_gameplay_common::game::general::GeneralEntityRecord,
    sunrise_gameplay_common::game::general::GeneralTypeEnvelope,
};

struct Declare(Arc<Mutex<Option<AttachmentKey<i32>>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Declare {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let value = r
            .register_attachment_type(
                mod_api::AttachmentDescriptor::new("sync_route:value", || 0i32)
                    .serialize(VALUE_CODEC)
                    .sync(VALUE_CODEC),
            )
            .unwrap();
        *self.0.lock().unwrap() = Some(value);
        r.declare_general_entity(
            "robot",
            GeneralEntityCategory::Misc,
            0.8,
            1.9,
            None,
            GeneralSavePolicy::Save,
            true,
            false,
            false,
            None,
            8,
            3,
            1,
            &[],
        )
        .unwrap();
    }
}

/// A server game with `sync_route:robot` declared and `sync_route:value` synced.
fn with_robot(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &AttachmentKey<i32>)) {
    mod_platform::__private::initialize().unwrap();
    let _phase = mod_platform::__private::RegistrationGuard::open();
    let value_slot = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "sync_route",
        Box::new(Declare(value_slot.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let value_key = value_slot
                .lock()
                .unwrap()
                .clone()
                .expect("Declare registered");
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(sunrise_gameplay_common::game::Dim::Overworld);
            game.build_registries();
            game.reset_entity_residency();
            test(game, &value_key)
        })
    });
}

/// One live `sync_route:robot` row; its id.
fn robot(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> u32 {
    let record = GeneralEntityRecord {
        position: [0.5, 64.0, 0.5],
        velocity: [0.0; 3],
        rotation: [0.0, 0.0],
        uuid: [1, 2, 3, 4],
        owner: None,
        custom_name: None,
        custom_name_visible: false,
        base: Default::default(),
        on_ground: false,
        fall_distance: 0.0,
        envelope: GeneralTypeEnvelope {
            type_key: "sync_route:robot".into(),
            payload_version: 1,
            payload_bytes: Vec::new(),
        },
        passengers: Vec::new(),
    };
    game.load_listed("sync_route:robot", None, None, record.to_tag())
        .unwrap();
    game.resident_entities.general.rows[0].id
}

fn value_of(changes: &[sunrise_mod_runtime::attachments::AttachmentSync]) -> Option<i32> {
    changes
        .iter()
        .find_map(|change| match change.value.as_ref()?.get(&"n".into()) {
            Some(Tag::Int(n)) => Some(*n),
            _ => None,
        })
}

#[test]
#[ignore = "daily"]
fn a_general_row_always_has_a_first_snapshot_holding_what_it_holds_now() {
    with_robot(|game, value_key| {
        let id = robot(game);
        // First pairing, nothing synced yet: no empty snapshot packet (NeoForge AttachmentSync).
        assert!(game.attachment_snapshot(Holder::Entity(id)).is_empty());
        // A client that pairs later is sent the value the row holds then.
        game.storage_scope(|game, _| {
            value_key.set(game, Holder::Entity(id), 7).unwrap();
            ((), true)
        });
        let late = game.attachment_snapshot(Holder::Entity(id));
        assert_eq!(late.len(), 1);
        assert!(late[0].replace);
        assert_eq!(value_of(&late[0].changes), Some(7));
        // Another entity with nothing synced still gets none.
        assert!(
            game.attachment_snapshot(Holder::Entity(id + 1000))
                .is_empty()
        );
    });
}

/// Matching NeoForge `ClientPayloadHandler.handleSyncAttachments`: an attachment sync for an
/// unknown entity logs a warning and does not mutate `remote.attachments`.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn remote_attachment_sync_ignores_unknown_entity() {
    use sunrise_mod_runtime::attachments::AttachmentSync;
    use sunrise_protocol::attachment_sync::Message;

    crate::game::server_side::item_entities::thrown_items_tests::with_remote_game(|game| {
        let holder = Holder::Entity(77);
        let message = || Message {
            holder,
            replace: true,
            changes: vec![AttachmentSync {
                namespace: "sync_route".into(),
                name: "value".into(),
                value: Some([("n".into(), Tag::Int(99))].into()),
            }],
        };
        game.remote_attachment_sync(message());
        assert!(
            !game
                .client
                .remote
                .as_ref()
                .unwrap()
                .attachments
                .contains_key(&holder),
            "unknown entity attachment sync must be ignored without mutating remote.attachments"
        );
        // Once the entity exists in the client level, attachment sync stores its values.
        game.spawn_entity(77, "minecraft:pig", [0.5, 64.0, 0.5], [0.0; 3]);
        game.remote_attachment_sync(message());
        assert!(
            game.client
                .remote
                .as_ref()
                .unwrap()
                .attachments
                .contains_key(&holder),
            "known entity attachment sync must populate remote.attachments"
        );
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn remote_player_attachments_only_apply_while_the_foreign_player_is_tracked() {
    use sunrise_mod_runtime::attachments::AttachmentSync;
    use sunrise_protocol::{
        attachment_sync::Message,
        packets::play::{AddEntity, Clientbound, PlayerInfo, PlayerInfoUpdate, RemoveEntities},
    };

    crate::game::server_side::item_entities::thrown_items_tests::with_remote_game(|game| {
        let (id, uuid) = (77, 0x1234);
        let player_kind = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:player")
            .unwrap();
        let holder = Holder::Player(uuid);
        let message = || Message {
            holder,
            replace: true,
            changes: vec![AttachmentSync {
                namespace: "sync_route".into(),
                name: "value".into(),
                value: Some([("n".into(), Tag::Int(99))].into()),
            }],
        };
        let mirrored = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
            game.client
                .remote
                .as_ref()
                .unwrap()
                .attachments
                .contains_key(&holder)
        };

        game.remote_attachment_sync(message());
        assert!(
            !mirrored(game),
            "sync before pairing must not create a mirror"
        );
        game.handle_packet(Clientbound::PlayerInfoUpdate(PlayerInfoUpdate {
            actions: 1,
            entries: vec![PlayerInfo {
                uuid,
                name: Some("TrackedPlayer".into()),
                ..Default::default()
            }],
        }))
        .unwrap();
        game.handle_packet(Clientbound::AddEntity(AddEntity {
            id,
            uuid,
            kind: player_kind,
            position: [0.5, 64.0, 0.5],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        }))
        .unwrap();
        game.remote_attachment_sync(message());
        assert!(mirrored(game), "the paired player receives attachment sync");
        game.handle_packet(Clientbound::RemoveEntities(RemoveEntities {
            ids: vec![id],
        }))
        .unwrap();
        assert!(!mirrored(game), "the departing player loses its mirror");
        game.remote_attachment_sync(message());
        assert!(
            !mirrored(game),
            "sync after removal must not recreate a mirror"
        );
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
