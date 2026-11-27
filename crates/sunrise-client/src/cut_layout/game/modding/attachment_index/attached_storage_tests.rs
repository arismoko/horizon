//! Stock attached storage (`AttachedItems`, `AttachedEnergy`) through real holder writes and
//! the transaction journal.
use mod_api::{
    AttachedEnergy, AttachedItems, AttachmentDescriptor, AttachmentKey, EnergyHandler,
    HostTransaction, ITEM_STACKS, ItemResource, ResourceHandler, ServerHost, StackList,
};
use std::sync::{Arc, Mutex};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_mod_runtime::attachments::Holder;

const CHUNK: Holder = Holder::Chunk([2, 3]);

#[derive(Clone)]
struct StorageKeys {
    items: AttachmentKey<StackList<ItemResource>>,
    energy: AttachmentKey<u32>,
}

struct StorageMod {
    keys: Arc<Mutex<Option<StorageKeys>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for StorageMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let items = r
            .register_attachment_type(
                AttachmentDescriptor::new("storage_test:items", StackList::new)
                    .serialize(ITEM_STACKS),
            )
            .unwrap();
        let energy = r
            .register_attachment_type(AttachmentDescriptor::new("storage_test:energy", || 0u32))
            .unwrap();
        *self.keys.lock().unwrap() = Some(StorageKeys { items, energy });
    }
}

fn chunk_game(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.options.dedicated = Some(Vec::new());
    game.native_level_opened(Dim::Overworld);
    game.world
        .chunks
        .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
}

fn with_storage_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &StorageKeys)) {
    let keys = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "storage_test",
        Box::new(StorageMod { keys: keys.clone() }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let keys = keys.lock().unwrap().clone().expect("StorageMod registered");
            chunk_game(game);
            test(game, &keys);
        });
    });
}

#[test]
#[ignore = "daily"]
fn attached_items_are_undone_by_an_abort_and_saved_by_their_codec() {
    with_storage_game(|game, keys| {
        let dirt = ItemResource::of(&ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap());
        let slots = AttachedItems::new(&keys.items, CHUNK, 9, 64, |_| true);
        let host: &mut dyn ServerHost = game;
        let aborted = host
            .transaction(|host, tx| Err::<(), _>(slots.insert_at(host, tx, 4, &dirt, 10).unwrap()));
        assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(10)));
        assert_eq!(keys.items.has(host, CHUNK), Ok(false));
        let went = host.transaction(|host, tx| slots.insert_at(host, tx, 4, &dirt, 10));
        assert_eq!(went, Ok(10));
        assert_eq!(slots.amount(host, 4), 10);
        // What the save writes reads back to the same index.
        let held = keys
            .items
            .get_existing(host, CHUNK, |value| value.cloned())
            .unwrap()
            .unwrap();
        let reloaded = (ITEM_STACKS.decode)(&(ITEM_STACKS.encode)(&held).unwrap()).unwrap();
        assert_eq!(reloaded, StackList::from([(4, (dirt, 10))]));
    });
}

#[test]
#[ignore = "daily"]
fn attached_energy_refuses_past_its_capacity() {
    with_storage_game(|game, keys| {
        let battery = AttachedEnergy::new(&keys.energy, CHUNK, 100, 1000, 1000);
        let host: &mut dyn ServerHost = game;
        let insert = |host: &mut dyn ServerHost, amount| {
            host.transaction(|host, tx| battery.insert(host, tx, amount))
        };
        assert_eq!(insert(&mut *host, 150), Ok(100));
        assert_eq!(insert(&mut *host, 1), Ok(0));
        assert_eq!(battery.amount(host), 100);
    });
}
