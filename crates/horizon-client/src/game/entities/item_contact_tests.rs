use super::*;
use horizon_client::{chunk::Chunk, join_seq::Joined as _};
use mod_api::{
    self, BlockPart, Flow, Hooks, HostSide, ItemAfterVerticalCollisionEvent, Registrar,
    RegistrationContext,
};
use std::sync::Mutex;

type Contact = (bool, u32, [i32; 3], [f64; 3], u32);

#[derive(Clone, Default)]
struct Probe {
    contacts: Arc<Mutex<Vec<Contact>>>,
}

#[mod_api::horizon_part]
impl BlockPart for Probe {
    fn item_after_vertical_collision(
        &self,
        _: HostSide,
        event: &mut ItemAfterVerticalCollisionEvent<'_>,
    ) -> Flow {
        let stack = event.stack.as_ref().expect("active item stack");
        self.contacts.lock().unwrap().push((
            event.client,
            event.entity,
            event.block_pos,
            event.item_pos,
            stack.count(),
        ));
        if stack.count() == 1 {
            return Flow::Cancel;
        }
        *event.stack = event.stack.take().unwrap().shrink(2);
        Flow::Continue
    }
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.block_key("minecraft:stone");
        r.block_part(key, self.clone());
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated mod registry"]
fn the_active_stack_survives_a_partial_contact_and_consumption_removes_the_entity() {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.options.dedicated = Some(Vec::new());
    game.native_lifecycle
        .begin_open(mod_api::NativeAuthority::IntegratedServer);
    game.world = World::new(game.res.facts().air, 1);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(4, 63, 4, stone).unwrap();

    // The centre of the item is outside the block, while its box overlaps it.
    let landing = [3.99, 64.0, 4.5];
    let mut input = ItemStack::new("minecraft:iron_ingot", 13);
    input
        .data_mut()
        .set_other("fixture:opaque", Some(horizon_core::nbt::Tag::Long(99)));
    let spawn = |game: &mut Game<'_>, count| {
        game.dropped.clear();
        let mut stack = input.clone();
        stack.count = count;
        game.spawn_item(stack, [landing[0], 64.1, landing[2]], [0.0, -0.3, 0.0], 20);
        game.ticked_entities_through = game.next_entity;
        game.dropped[0].clone()
    };
    let before = spawn(&mut game, 13);
    game.tick_items();
    let vanilla = game.dropped[0].clone();
    assert_eq!(vanilla.id, before.id);
    assert_eq!(
        vanilla.position,
        Vec3::new(landing[0], landing[1], landing[2])
    );
    assert_eq!(vanilla.velocity, Vec3::ZERO);
    assert_eq!(vanilla.stack, input);
    assert_eq!(vanilla.age, 1);

    let probe = Probe::default();
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("contact", Box::new(probe.clone()))].into();
    mod_api::testing::with_mods(mods.clone(), || {
        mod_api::__private::install_parts(&mods).unwrap();
        let registry = game.pack.assets.registry;
        mod_api::__private::part_tests::register_game_keys(
            (0..registry.len())
                .map(|i| {
                    registry
                        .state(mod_api::BlockStateId(i as u32))
                        .unwrap()
                        .name
                        .clone()
                })
                .collect::<std::collections::HashSet<_>>(),
        );
        mod_api::__private::mark_game_thread();
        game.resolve_parts().unwrap();
        let old = spawn(&mut game, 13);
        game.tick_items();
        let item = &game.dropped[0];
        assert_eq!(item.id, old.id);
        assert_eq!(item.join_seq(), old.join_seq());
        assert_eq!(item.position, vanilla.position);
        assert_eq!(item.velocity, vanilla.velocity);
        let mut expected = input.clone();
        expected.count = 11;
        assert_eq!(item.stack, expected);
        assert_eq!(
            probe.contacts.lock().unwrap().as_slice(),
            &[(false, old.id, [4, 63, 4], landing, 13)]
        );

        for count in [1, 2] {
            spawn(&mut game, count);
            game.tick_items();
            assert!(game.dropped.is_empty(), "consumed stack of {count}");
        }
    });
}
