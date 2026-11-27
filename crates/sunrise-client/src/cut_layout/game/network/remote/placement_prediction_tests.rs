#[cfg(feature = "client")]
use mod_api::{
    BlockPart, BlockState, CanSurviveEvent, CanSustainPlantEvent, FenceGateInWallEvent, Hooks,
    LevelSide, MayPlaceOnEvent, Registrar, StateForPlacementEvent,
};
#[cfg(feature = "client")]
use std::sync::Arc;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::GameGameExt1;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_gameplay_common::game::remote::BlockStateId;
use sunrise_gameplay_common::game::remote::dry_level::DryLevel;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::remote::dry_level::DryLevelGameExt2 as _;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::remote::{BlockEntity, C, Chunk, Holder, ItemStack, Tag, play};
#[cfg(feature = "client")]
use {
    crate::game::server_side::native_entity_refs::tests::with_game_role_client,
    sunrise_client_gameplay::game::native_connection_groups::ReceivedGroup,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
};

/// Every answer below is a client's: the hooks are asked by its placement prediction.
#[cfg(feature = "client")]
pub(crate) fn client(level: LevelSide<'_>) {
    assert!(matches!(level, LevelSide::Client(_)));
}

#[cfg(feature = "client")]
pub(crate) struct Refuse;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for Refuse {
    fn state_for_placement(
        &self,
        level: LevelSide<'_>,
        _: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockState>> {
        client(level);
        Some(None)
    }
}

#[cfg(feature = "client")]
pub(crate) struct Fall;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for Fall {
    fn can_survive(&self, level: LevelSide<'_>, _: &mut CanSurviveEvent) -> Option<bool> {
        client(level);
        Some(false)
    }
}

#[cfg(feature = "client")]
pub(crate) struct Soil;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for Soil {
    fn can_sustain_plant(
        &self,
        level: LevelSide<'_>,
        _: &mut CanSustainPlantEvent,
    ) -> Option<bool> {
        client(level);
        Some(true)
    }
}

#[cfg(feature = "client")]
pub(crate) struct Hardy;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for Hardy {
    fn may_place_on(&self, level: LevelSide<'_>, _: &mut MayPlaceOnEvent) -> Option<bool> {
        client(level);
        Some(true)
    }
}

#[cfg(feature = "client")]
pub(crate) struct PlaceMod;
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for PlaceMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let dirt = r.block_key("minecraft:dirt");
        r.block_part(dirt, Refuse);
        let cobblestone = r.block_key("minecraft:cobblestone");
        r.block_part(cobblestone, Fall);
        let glass = r.block_key("minecraft:glass");
        r.block_part(glass, Soil);
        let birch = r.block_key("minecraft:birch_sapling");
        r.block_part(birch, Hardy);
    }
}

/// `test` on a survival-mode client game with `mods`' parts, connected to a server and with
/// chunk (0, 0) loaded and empty.
#[cfg(feature = "client")]
pub(crate) fn on_a_client(
    mods: mod_api::Mods,
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game_role_client(
            root,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            |game| {
                let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "PlacePrediction").unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                game.set_local_mode(sunrise_command::args::GameMode::Survival);
                test(game);
            },
        );
    });
}

/// The click at the top of `[x, 64, z]`.
#[cfg(feature = "client")]
pub(crate) fn top_of(x: i32, z: i32) -> sunrise_gameplay_common::game::Hit {
    sunrise_gameplay_common::game::Hit {
        pos: [x, 64, z],
        face: [0, 1, 0],
        point: [f64::from(x) + 0.5, 65.0, f64::from(z) + 0.5],
        inside: false,
    }
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn client_placement_prediction_asks_placement_and_survival_parts() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("place_test", Box::new(PlaceMod))].into();
    on_a_client(mods, |game| {
        // (ground, held block, whether the client predicts the placement)
        for (i, (ground, held, placed)) in [
            ("minecraft:stone", "minecraft:dirt", false),
            ("minecraft:stone", "minecraft:cobblestone", false),
            ("minecraft:stone", "minecraft:oak_sapling", false),
            ("minecraft:glass", "minecraft:oak_sapling", true),
            ("minecraft:stone", "minecraft:birch_sapling", true),
        ]
        .into_iter()
        .enumerate()
        {
            let x = i as i32 * 2;
            let below = game.pack.assets.registry.default_state(ground).unwrap();
            game.world.set(x, 64, 0, below).unwrap();
            let preview = game.remote_can_place(&top_of(x, 0), &ItemStack::new(held, 1));
            assert_eq!(
                preview.map(|placed| placed.pos),
                placed.then(|| sunrise_core::position::BlockPos::new(x, 65, 0)),
                "{held} on {ground}"
            );
        }
    });
}

#[cfg(feature = "client")]
pub(crate) struct StorageFacing;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for StorageFacing {
    fn state_for_placement(
        &self,
        level: LevelSide<'_>,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockState>> {
        let level = match level {
            LevelSide::Client(level) | LevelSide::Server(level) => level,
        };
        let pos = event.context.pos;
        let present = level.has_item_capability([pos.x + 1, pos.y, pos.z], None)?;
        level
            .state_with_property(event.state, "axis", if present { "x" } else { "z" })
            .map(Some)
    }
}

#[cfg(feature = "client")]
pub(crate) struct StoragePort;
#[cfg(feature = "client")]
impl mod_api::ResourceHandler<mod_api::ItemResource> for StoragePort {
    fn size(&self, _: &mut dyn mod_api::ServerHost) -> usize {
        0
    }
    fn resource(&self, _: &mut dyn mod_api::ServerHost, _: usize) -> Option<mod_api::ItemResource> {
        None
    }
    fn amount(&self, _: &mut dyn mod_api::ServerHost, _: usize) -> u32 {
        0
    }
    fn capacity(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: usize,
        _: Option<&mod_api::ItemResource>,
    ) -> u32 {
        0
    }
    fn insert_at(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mod_api::TransactionContext,
        _: usize,
        _: &mod_api::ItemResource,
        _: u32,
    ) -> std::result::Result<u32, mod_api::TransactionError<mod_api::Overanswered>> {
        Ok(0)
    }
    fn extract_at(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mod_api::TransactionContext,
        _: usize,
        _: &mod_api::ItemResource,
        _: u32,
    ) -> std::result::Result<u32, mod_api::TransactionError<mod_api::Overanswered>> {
        Ok(0)
    }
}

#[cfg(feature = "client")]
pub(crate) struct PlacementStorageMod;
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for PlacementStorageMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let log = r.block_key("minecraft:oak_log");
        r.block_part(log, StorageFacing);
        let fire = r.block_entity_type_key("minecraft:campfire");
        r.block_entity_capability(&mod_api::ITEMS, &fire, |level, at, state, _, offer| {
            if at.side.is_none() && level.state_with_property(state, "lit", "true") == Some(state) {
                offer.offer(|| Box::new(StoragePort));
            }
        });
    }
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn placement_and_prediction_choose_from_their_local_item_providers() {
    use mod_api::{Direction, LevelHost};
    use sunrise_core::position::BlockPos;
    use sunrise_world::level::{PlaceContext, PlaceOutcome, PlacementSource};
    use {
        sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
    };

    if run_fresh_cases(
        "cut_layout::game::remote::placement_prediction_tests::placement_and_prediction_choose_from_their_local_item_providers",
        "SUNRISE_PLACEMENT_STORAGE_CHILD",
        &["run"],
    ) {
        return;
    }
    const TARGET: [i32; 3] = [4, 65, 4];
    const PORT: [i32; 3] = [5, 65, 4];
    let mods = || {
        vec![mod_api::ModEntry::new(
            "placement_storage",
            Box::new(PlacementStorageMod),
        )]
        .into()
    };
    let root = fixture_root();
    let builtin = sunrise_protocol::registry::Builtin::load(
        &sunrise_protocol::registry::Builtin::find(&root).unwrap(),
    )
    .unwrap();
    let kinds = &builtin.get("minecraft:block_entity_type").unwrap().names;
    let resolve_kinds = || {
        // This fresh fixture bypasses content loading, which normally resolves
        // provider keys against the engine's block-entity registry.
        mod_api::__private::resolve_block_entity_types(kinds.iter().map(String::as_str), |id| {
            kinds.iter().any(|kind| kind == id).then_some(())
        })
        .unwrap();
    };
    let server_state = std::cell::Cell::new(None);
    fn setup(
        game: &mut sunrise_gameplay_common::game::CommonGame<'_>,
        lit: bool,
    ) -> sunrise_world::registry::BlockStateId {
        let registry = game.pack.assets.registry;
        let state = registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:campfire".into(),
                properties: [
                    ("facing", "north"),
                    ("lit", if lit { "true" } else { "false" }),
                    ("signal_fire", "false"),
                    ("waterlogged", "false"),
                ]
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
            })
            .unwrap();
        let ground = registry.default_state("minecraft:stone").unwrap();
        game.world.set(4, 64, 4, ground).unwrap();
        game.world.set(5, 65, 4, state).unwrap();
        game.world.block_entities.insert(
            (5, 65, 4),
            sunrise_level::containers::BlockEntity::Campfire(Box::default()),
        );
        state
    }
    mod_platform::testing::with_mods(mods(), || {
        with_game_role(root, ExecutionRole::IntegratedServer, |game| {
            resolve_kinds();
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            game.sync_tick_chunks();
            setup(game, true);
            let context = PlaceContext {
                clicked_pos: BlockPos::new(4, 64, 4),
                face: Direction::Up,
                hit: sunrise_core::geometry::Vec3::new(4.5, 65.0, 4.5),
                inside: false,
                source: PlacementSource::Player {
                    yaw: 0.0,
                    pitch: 0.0,
                    sneaking: false,
                },
                item: "minecraft:oak_log".to_owned(),
                can_use_game_master_blocks: false,
            };
            let placed =
                game.with_level(|rules, level, ticks| rules.place_block(level, ticks, &context));
            let PlaceOutcome::Placed { pos, state } = placed else {
                panic!("server refused placement")
            };
            assert_eq!(pos, BlockPos::new(4, 65, 4));
            assert_eq!(
                game.pack.assets.registry.state(state).unwrap().properties["axis"],
                "x"
            );
            server_state.set(Some(state));
        });
    });
    on_a_client(mods(), |game| {
        resolve_kinds();
        game.client.remote.as_mut().unwrap().logged_in = true;
        setup(game, false);
        let guess = game
            .remote_can_place(&top_of(4, 4), &ItemStack::new("minecraft:oak_log", 1))
            .unwrap();
        assert_eq!(guess.pos, BlockPos::new(4, 65, 4));
        assert_eq!(guess.changes.len(), 1);
        let guessed = guess.changes[0].1;
        assert_eq!(
            game.pack.assets.registry.state(guessed).unwrap().properties["axis"],
            "z"
        );
        game.predict_blocks(&guess.changes, 7);
        assert_eq!(game.world.block(4, 65, 4), guessed);
        let server = server_state.get().unwrap();
        game.handle_packet(C::BlockUpdate(play::BlockUpdate {
            pos: TARGET,
            state: server.0 as i32,
        }))
        .unwrap();
        game.handle_packet(C::BlockChangedAck(play::BlockChangedAck { sequence: 7 }))
            .unwrap();
        assert_eq!(game.world.block(4, 65, 4), server);

        // Keep the same campfire state but remove its block entity in the disposable level.
        let campfire = setup(game, true);
        game.with_level(|_, level, ticks| {
            assert_eq!(
                level.view(ticks).has_item_capability(PORT, None),
                Some(true)
            );
        });
        game.check_dry_fire_odds_view(
            |_, dry, _| {
                dry.remove_block_entity(BlockPos::new(5, 65, 4), campfire, false);
            },
            |view| assert_eq!(view.has_item_capability(PORT, None), Some(false)),
        );
        // A staged replacement also removes the previous provider through the placement view.
        let air = game.world.air;
        game.with_level(|_, level, ticks| {
            let changes =
                sunrise_world::level::PreviewBlocks::from([(BlockPos::new(5, 65, 4), air)]);
            assert_eq!(
                level
                    .view(ticks)
                    .previewing(Some(&changes))
                    .has_item_capability(PORT, None),
                Some(false)
            );
            assert_eq!(
                level.view(ticks).has_item_capability(PORT, None),
                Some(true)
            );
        });
    });
}

#[cfg(feature = "client")]
pub(crate) struct BedInstead;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for BedInstead {
    fn state_for_placement(
        &self,
        level: LevelSide<'_>,
        _: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockState>> {
        let LevelSide::Client(level) = level else {
            panic!("asked on a server");
        };
        Some(level.block_state_id("minecraft:red_bed"))
    }
}

#[cfg(feature = "client")]
pub(crate) struct BedPlacementMod;
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for BedPlacementMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let furnace = r.block_key("minecraft:furnace");
        r.block_part(furnace, BedInstead);
    }
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn predicts_the_selected_beds_component_adjusted_head() {
    use sunrise_core::nbt::{Compound, Tag};
    let mods = vec![mod_api::ModEntry::new(
        "placement_bed",
        Box::new(BedPlacementMod),
    )]
    .into();
    on_a_client(mods, |game| {
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let air = registry.default_state("minecraft:air").unwrap();
        let mut stack = ItemStack::new("minecraft:furnace", 1);
        stack.data_mut().set_other(
            "minecraft:block_state",
            Some(Tag::Compound(Compound::from([
                ("facing".into(), Tag::String("east".into())),
                ("occupied".into(), Tag::String("not_a_boolean".into())),
            ]))),
        );
        game.world.set(5, 64, 5, stone).unwrap();
        game.world.set(6, 64, 5, stone).unwrap();
        game.world.set(5, 65, 4, stone).unwrap(); // would block the default north head
        let placed = game
            .remote_can_place(&top_of(5, 5), &stack)
            .expect("east-facing bed");
        game.predict_place(&placed, &stack, 9);
        let foot = registry.state(game.world.block(5, 65, 5)).unwrap();
        let head = registry.state(game.world.block(6, 65, 5)).unwrap();
        for part in [foot, head] {
            assert_eq!(part.name, "minecraft:red_bed");
            assert_eq!(part.properties["facing"], "east");
            assert_eq!(part.properties["occupied"], "false");
        }
        assert_eq!(foot.properties["part"], "foot");
        assert_eq!(head.properties["part"], "head");
        assert_eq!(game.world.block(5, 65, 4), stone);
        assert_eq!(game.world.block(5, 65, 6), air);
    });
}

/// A dandelion that stands while a cobblestone is east of it, as its level shows it.
#[cfg(feature = "client")]
pub(crate) struct BesideCobblestone;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for BesideCobblestone {
    fn can_survive(&self, level: LevelSide<'_>, event: &mut CanSurviveEvent) -> Option<bool> {
        let LevelSide::Client(level) = level else {
            panic!("asked on a server");
        };
        let east = [event.pos.x + 1, event.pos.y, event.pos.z];
        Some(level.block_state(east) == level.block_state_id("minecraft:cobblestone"))
    }
}

#[cfg(feature = "client")]
pub(crate) struct NeighbourMod;
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for NeighbourMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let dandelion = r.block_key("minecraft:dandelion");
        r.block_part(dandelion, BesideCobblestone);
    }
}

#[cfg(feature = "client")]
pub(crate) type BreakRead = (
    sunrise_core::position::BlockPos,
    Option<BlockState>,
    Option<BlockState>,
);

#[cfg(feature = "client")]
#[derive(Clone, Default)]
pub(crate) struct BreakObserver(pub(in crate::game) Arc<std::sync::Mutex<Vec<BreakRead>>>);

#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for BreakObserver {
    fn on_block_state_change(
        &self,
        level: LevelSide<'_>,
        event: &mut mod_api::OnBlockStateChangeEvent,
    ) {
        let LevelSide::Client(level) = level else {
            panic!("prediction notice must be client-side");
        };
        if event.predicted {
            self.0.lock().unwrap().push((
                event.pos,
                level.block_state([0, 65, 0]),
                level.block_state([1, 65, 0]),
            ));
        }
    }
}

#[cfg(feature = "client")]
pub(crate) struct BreakMod(pub(crate) BreakObserver);
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for BreakMod {
    fn register(&self, r: &mut dyn Registrar, context: &mod_api::RegistrationContext) {
        NeighbourMod.register(r, context);
        for name in ["minecraft:dandelion", "minecraft:cobblestone"] {
            let key = r.block_key(name);
            r.block_part(key, self.0.clone());
        }
    }
}

/// A placement's shape update asks a neighbour's survival part over the prediction's own
/// writes: the dandelion on stone, which its template pops off, stands because its part sees
/// the cobblestone just predicted beside it.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_neighbour_s_survival_part_reads_the_prediction_s_blocks() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "neighbour_test",
        Box::new(NeighbourMod),
    )]
    .into();
    on_a_client(mods, |game| {
        let state = |game: &sunrise_client_gameplay::game::ClientGame<'_>, name: &str| {
            game.pack.assets.registry.default_state(name).unwrap()
        };
        for x in [0, 1] {
            let stone = state(game, "minecraft:stone");
            game.world.set(x, 64, 0, stone).unwrap();
        }
        let dandelion = state(game, "minecraft:dandelion");
        game.world.set(0, 65, 0, dandelion).unwrap();
        let placed = game
            .remote_can_place(&top_of(1, 0), &ItemStack::new("minecraft:cobblestone", 1))
            .expect("the cobblestone is placed");
        let at = |x| {
            placed
                .changes
                .iter()
                .find(|(pos, _)| *pos == sunrise_core::position::BlockPos::new(x, 65, 0))
                .map(|&(_, state)| state)
        };
        assert_eq!(at(1), Some(state(game, "minecraft:cobblestone")));
        assert_eq!(at(0), None, "the dandelion stands");
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_predicted_break_reconciles_its_neighbour_s_shape_change() {
    let observer = BreakObserver::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "break_test",
        Box::new(BreakMod(observer.clone())),
    )]
    .into();
    on_a_client(mods, |game| {
        let registry = game.pack.assets.registry;
        let dirt = registry.default_state("minecraft:dirt").unwrap();
        let cobblestone = registry.default_state("minecraft:cobblestone").unwrap();
        let dandelion = registry.default_state("minecraft:dandelion").unwrap();
        let air = game.world.air;
        // The template survives on dirt. Only the Part needs the block east of it.
        game.world.set(0, 64, 0, dirt).unwrap();
        game.world.set(0, 65, 0, dandelion).unwrap();
        game.world.set(1, 65, 0, cobblestone).unwrap();

        game.predict_break([1, 65, 0], cobblestone, 7);
        let air_state = Some(air.try_into_api().unwrap());
        let at = sunrise_core::position::BlockPos::new;
        assert_eq!(
            *observer.0.lock().unwrap(),
            [
                (at(0, 65, 0), air_state, air_state),
                (at(1, 65, 0), air_state, air_state)
            ],
            "dependent notice precedes support notice; both read the completed shape writes"
        );
        game.server_block([1, 65, 0], cobblestone.0 as i32, None);
        game.end_predictions(6).unwrap();
        assert_eq!(game.world.block(1, 65, 0), air);
        assert_eq!(game.world.block(0, 65, 0), air, "the Part saw the break");

        game.end_predictions(7).unwrap();
        assert_eq!(game.world.block(1, 65, 0), cobblestone);
        assert_eq!(
            game.world.block(0, 65, 0),
            dandelion,
            "the dependent write rolled back too"
        );

        game.server_block([1, 65, 0], air.0 as i32, None);
        assert_eq!(game.world.block(1, 65, 0), air);
        assert_eq!(
            game.world.block(0, 65, 0),
            dandelion,
            "server updates skip local shape rules"
        );
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_denied_break_restores_wall_sign_text_and_synced_attachments() {
    use mod_api::{AttachmentCodec, AttachmentDescriptor, AttachmentKey, Host};
    use sunrise_mod_runtime::attachments::AttachmentSync;
    use sunrise_protocol::attachment_sync::Message;

    const VALUE_CODEC: AttachmentCodec<i32> = AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |c| match c.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(mod_api::AttachmentError::Decode),
        },
    };
    struct BreakMod(Arc<std::sync::Mutex<Option<AttachmentKey<i32>>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for BreakMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let key = r
                .register_attachment_type(
                    AttachmentDescriptor::new("break_test:wall_sign", || 0i32)
                        .serialize(VALUE_CODEC)
                        .sync(VALUE_CODEC),
                )
                .unwrap();
            *self.0.lock().unwrap() = Some(key);
        }
    }
    let key_slot = Arc::new(std::sync::Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "break_test",
        Box::new(BreakMod(key_slot.clone())),
    )]
    .into();
    on_a_client(mods, |game| {
        let value_key = key_slot
            .lock()
            .unwrap()
            .clone()
            .expect("BreakMod registered");
        let spawn = play::SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 0,
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
            sunrise_client::level_loading::Reason::Other,
        );
        game.client.remote.as_mut().unwrap().logged_in = true;
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));

        let registry = game.pack.assets.registry;
        let support = registry.default_state("minecraft:cobblestone").unwrap();
        let wall_sign = |name: &str| {
            registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: name.into(),
                    properties: [
                        ("facing".into(), "west".into()),
                        ("waterlogged".into(), "false".into()),
                    ]
                    .into_iter()
                    .collect(),
                })
                .unwrap()
        };
        let oak = wall_sign("minecraft:oak_wall_sign");
        let spruce = wall_sign("minecraft:spruce_wall_sign");
        let pos = (0, 65, 0);
        let holder = Holder::BlockEntity([0, 65, 0]);
        let attachment_values = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
            let host: &dyn Host = game;
            let query = mod_api::AttachmentQueries::query(host, &value_key).unwrap();
            let scope = host.attachment_read_scope().unwrap();
            let mut rows = query.iter();
            std::iter::from_fn(|| rows.try_next().unwrap())
                .map(|row| row.with_value(scope, |value| *value).unwrap())
                .collect::<Vec<_>>()
        };
        game.world.set(1, 65, 0, support).unwrap();
        game.server_block([0, 65, 0], oak.0 as i32, None);
        let Some(BlockEntity::Sign(sign)) = game.world.block_entities.get_mut(&pos) else {
            panic!("wall sign was not created");
        };
        sign.front.lines[0] = "North entrance".into();
        sign.back.lines[3] = "Keep this side clear".into();
        sign.waxed = true;
        let original = sign.clone();
        game.remote_attachment_sync(Message {
            holder,
            replace: true,
            changes: vec![AttachmentSync {
                namespace: value_key.namespace().into(),
                name: value_key.name().into(),
                value: Some([("n".into(), Tag::Int(41))].into()),
            }],
        });
        assert_eq!(attachment_values(game), [41]);

        game.predict_break([1, 65, 0], support, 7);
        assert_eq!(game.world.block(0, 65, 0), game.world.air);
        assert!(game.world.block_entities.get(&pos).is_none());
        assert!(attachment_values(game).is_empty());
        // A denied break need not come with block or block entity packets.
        game.end_predictions(7).unwrap();
        assert_eq!(game.world.block(0, 65, 0), oak);
        let Some(BlockEntity::Sign(restored)) = game.world.block_entities.get(&pos) else {
            panic!("wall sign was not restored");
        };
        assert_eq!(restored, &original);
        assert_eq!(attachment_values(game), [41]);

        // A different authoritative block must not inherit the old data, even if
        // its block entity has the same type.
        game.predict_break([1, 65, 0], support, 8);
        game.server_block([0, 65, 0], spruce.0 as i32, None);
        game.end_predictions(8).unwrap();
        assert_eq!(game.world.block(0, 65, 0), spruce);
        let Some(BlockEntity::Sign(replacement)) = game.world.block_entities.get(&pos) else {
            panic!("replacement wall sign was not created");
        };
        assert_eq!(replacement, &Default::default());
        assert!(attachment_values(game).is_empty());

        // A bare verification can supersede the block state, not the last explicit group.
        let cell = [0, 65, 0];
        game.server_block(cell, spruce.0 as i32, Some(ReceivedGroup(Some(23))));
        game.predict_break([1, 65, 0], support, 9);
        game.server_block(cell, spruce.0 as i32, Some(ReceivedGroup(None)));
        game.server_block(cell, spruce.0 as i32, None);
        assert_eq!(game.world.connection_group(0, 65, 0), Some(23));
        game.end_predictions(9).unwrap();
        assert_eq!(game.world.block(0, 65, 0), spruce);
        assert_eq!(game.world.connection_group(0, 65, 0), None);
    });
}

#[cfg(feature = "client")]
pub(crate) static GUIDE_ASKS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(feature = "client")]
pub(crate) struct Counted;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for Counted {
    fn state_for_placement(
        &self,
        _: LevelSide<'_>,
        _: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockState>> {
        GUIDE_ASKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        None
    }
}

/// A dandelion's survival, asked from a neighbour's shape update.
#[cfg(feature = "client")]
pub(crate) struct CountedSurvival;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for CountedSurvival {
    fn can_survive(&self, _: LevelSide<'_>, _: &mut CanSurviveEvent) -> Option<bool> {
        GUIDE_ASKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        None
    }
}

#[cfg(feature = "client")]
pub(crate) struct CountedWall;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for CountedWall {
    fn fence_gate_in_wall(
        &self,
        level: LevelSide<'_>,
        _: &mut FenceGateInWallEvent,
    ) -> Option<bool> {
        client(level);
        GUIDE_ASKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Some(true)
    }
}

#[cfg(feature = "client")]
pub(crate) struct GuideMod;
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for GuideMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let dirt = r.block_key("minecraft:dirt");
        r.block_part(dirt, Counted);
        let dandelion = r.block_key("minecraft:dandelion");
        r.block_part(dandelion, CountedSurvival);
        let glass = r.block_key("minecraft:glass");
        r.block_part(glass, CountedWall);
    }
}

/// The controller guide's look at what a click would do asks no placement, survival or
/// fence-gate wall-height part; the click does.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn the_guide_s_look_asks_no_placement_or_survival_part_and_the_click_does() {
    use std::sync::atomic::Ordering;
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("guide_test", Box::new(GuideMod))].into();
    on_a_client(mods, |game| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 64, 0, stone).unwrap();
        let dirt = ItemStack::new("minecraft:dirt", 1);
        GUIDE_ASKS.store(0, Ordering::Relaxed);
        assert!(game.remote_would_place(&top_of(0, 0), &dirt));
        assert_eq!(GUIDE_ASKS.load(Ordering::Relaxed), 0);
        assert!(game.remote_can_place(&top_of(0, 0), &dirt).is_some());
        assert!(GUIDE_ASKS.load(Ordering::Relaxed) > 0);

        // A lever flipped beside a dandelion updates its shape, which asks its survival: the
        // guide's look at the lever doesn't, the click does.
        let registry = game.pack.assets.registry;
        let lever = registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:lever".into(),
                properties: [("face", "floor"), ("facing", "north"), ("powered", "false")]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
            })
            .unwrap();
        for x in [4, 5] {
            game.world.set(x, 64, 0, stone).unwrap();
        }
        game.world.set(4, 65, 0, lever).unwrap();
        game.world
            .set(
                5,
                65,
                0,
                registry.default_state("minecraft:dandelion").unwrap(),
            )
            .unwrap();
        let hit = sunrise_gameplay_common::game::Hit {
            pos: [4, 65, 0],
            face: [0, 1, 0],
            point: [4.5, 65.1, 0.5],
            inside: false,
        };
        GUIDE_ASKS.store(0, Ordering::Relaxed);
        assert!(game.block_takes_click(&hit, None));
        assert!(!game.block_swallows_click(&hit, None));
        assert_eq!(GUIDE_ASKS.load(Ordering::Relaxed), 0);
        let (result, _) = game.remote_block_reacts(&hit, None, true);
        assert!(result.consumes_action());
        assert!(GUIDE_ASKS.load(Ordering::Relaxed) > 0);

        // Glass isn't a wall, but its Part lowers the predicted gate next to it.
        game.yaw = 0.0; // Face south, so the gate checks its west and east neighbours.
        game.world.set(8, 64, 0, stone).unwrap();
        let glass = registry.default_state("minecraft:glass").unwrap();
        game.world.set(9, 65, 0, glass).unwrap();
        let gate = ItemStack::new("minecraft:oak_fence_gate", 1);
        let hit = top_of(8, 0);
        GUIDE_ASKS.store(0, Ordering::Relaxed);
        assert!(game.remote_would_place(&hit, &gate));
        assert_eq!(GUIDE_ASKS.load(Ordering::Relaxed), 0);
        let placed = game
            .remote_can_place(&hit, &gate)
            .expect("the gate is placed");
        let &(_, state) = placed
            .changes
            .iter()
            .find(|(pos, _)| *pos == placed.pos)
            .expect("the gate's predicted write");
        assert_eq!(registry.state(state).unwrap().properties["in_wall"], "true");
    });
}

/// The comparator east of the clicked block's spot and what its part read of its signal.
#[cfg(feature = "client")]
pub(crate) const COMPARATOR: [i32; 3] = [2, 65, 0];
#[cfg(feature = "client")]
pub(crate) static COMPARATOR_READS: std::sync::Mutex<Vec<Option<i32>>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(feature = "client")]
pub(crate) struct ReadsComparator;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for ReadsComparator {
    fn can_survive(&self, level: LevelSide<'_>, _: &mut CanSurviveEvent) -> Option<bool> {
        let LevelSide::Client(level) = level else {
            panic!("asked on a server");
        };
        let read = level.signal(COMPARATOR, mod_api::Direction::East);
        COMPARATOR_READS.lock().unwrap().push(read);
        None
    }
}

#[cfg(feature = "client")]
pub(crate) struct ComparatorMod;
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for ComparatorMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let cobblestone = r.block_key("minecraft:cobblestone");
        r.block_part(cobblestone, ReadsComparator);
    }
}

/// A placement part reads a powered comparator's output from the client's own ticks, not from
/// the prediction's scratch ones: unknown while the client holds none, its value once it does.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_placement_part_reads_comparator_outputs_the_client_holds() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "comparator_test",
        Box::new(ComparatorMod),
    )]
    .into();
    on_a_client(mods, |game| {
        let registry = game.pack.assets.registry;
        let comparator = registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:comparator".into(),
                properties: [("facing", "east"), ("powered", "true")]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
            })
            .unwrap();
        let [x, y, z] = COMPARATOR;
        game.world.set(x, y, z, comparator).unwrap();
        game.world
            .set(0, 64, 0, registry.default_state("minecraft:stone").unwrap())
            .unwrap();
        let cobblestone = ItemStack::new("minecraft:cobblestone", 1);
        let read = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
            COMPARATOR_READS.lock().unwrap().clear();
            assert!(game.remote_can_place(&top_of(0, 0), &cobblestone).is_some());
            std::mem::take(&mut *COMPARATOR_READS.lock().unwrap())
        };
        let reads = read(game);
        assert!(
            !reads.is_empty() && reads.iter().all(Option::is_none),
            "{reads:?}"
        );
        for output in [0, 9] {
            game.block_ticks
                .comparator_outputs
                .insert(sunrise_core::position::BlockPos::new(x, y, z), output);
            let reads = read(game);
            assert!(
                !reads.is_empty() && reads.iter().all(|r| *r == Some(output)),
                "stored {output}, read {reads:?}"
            );
        }
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_placement_view_hides_block_entity_data_at_replaced_positions() {
    use mod_api::LevelHost;
    use sunrise_core::position::BlockPos;
    use sunrise_world::level::PreviewBlocks;

    on_a_client(Vec::new().into(), |game| {
        game.client.remote.as_mut().unwrap().logged_in = true;
        let pos = [1, 65, 1];
        let campfire = game
            .pack
            .assets
            .registry
            .default_state("minecraft:campfire")
            .unwrap();
        game.world.set(1, 65, 1, campfire).unwrap();
        game.world.block_entities.insert(
            (1, 65, 1),
            sunrise_level::containers::BlockEntity::Campfire(Box::default()),
        );
        let air = game.world.air;
        game.with_level(|_, live, ticks| {
            let view = live.view(ticks);
            let entity = view
                .block_entity(pos)
                .expect("campfire handle in the live view");
            let data = view.block_entity_data(entity).expect("live campfire data");

            let changes = PreviewBlocks::from([(BlockPos::new(1, 65, 1), air)]);
            let preview = live.view(ticks).previewing(Some(&changes));
            assert!(preview.block_entity_data(entity).is_none());
            assert_eq!(view.block_entity_data(entity), Some(data));
        });
    });
}

/// The guess asks the live level's placement and survival questions over its own writes, with
/// the live ticks it reads; a look that asks no mods asks none.
#[test]
fn a_prediction_asks_the_live_level_over_its_writes() {
    use sunrise_core::{position::BlockPos, random::LegacyRandom};
    use sunrise_world::level::{LevelAccess, PreviewBlocks, Ticks};

    struct Live;
    impl sunrise_world::level::BlockEntityNeighbors for Live {}
    impl LevelAccess for Live {
        fn block_state(&self, _: BlockPos) -> BlockStateId {
            BlockStateId(0)
        }
        fn set_raw(&mut self, _: BlockPos, _: BlockStateId) -> Option<BlockStateId> {
            panic!("the guess must not write to the live level");
        }
        fn is_loaded(&self, _: BlockPos) -> bool {
            true
        }
        fn is_client_side(&self) -> bool {
            true
        }
        fn random(&mut self) -> &mut LegacyRandom {
            panic!("the guess must not draw from the live level");
        }
        fn game_time(&self) -> i64 {
            0
        }
        /// Survives over the block the preview set below it, from the live ticks.
        fn before_can_survive(
            &self,
            ticks: &Ticks,
            preview: Option<&PreviewBlocks>,
            pos: BlockPos,
            _: BlockStateId,
            survives: &mut Option<bool>,
        ) -> bool {
            assert_eq!(ticks.random_tick_value, 7, "asked with the live ticks");
            let below = BlockPos::new(pos.x, pos.y - 1, pos.z);
            *survives = Some(preview.is_some_and(|p| p.contains_key(&below)));
            false
        }
    }

    impl sunrise_gameplay_common::game::remote::dry_level::PredictionBase for Live {}

    let live = Live;
    let live_ticks = Ticks::new(7);
    let mut guess = DryLevel::new(&live);
    guess.reads = Some(&live_ticks);
    let pos = BlockPos::new(0, 1, 0);
    let ask = |guess: &DryLevel<'_>| {
        let mut survives = None;
        guess.before_can_survive(&Ticks::default(), None, pos, BlockStateId(1), &mut survives);
        survives
    };
    assert_eq!(ask(&guess), Some(false));
    guess.set_raw(BlockPos::new(0, 0, 0), BlockStateId(1));
    assert_eq!(ask(&guess), Some(true));
    guess.asks_mods = false;
    assert_eq!(ask(&guess), None);
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn blocks_answer_a_held_item_s_click_on_the_client_as_vanilla() {
    on_a_client(Vec::new().into(), |game| {
        let registry = game.pack.assets.registry;
        let state = |name: &str, properties: &[(&str, &str)]| {
            registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: name.into(),
                    properties: properties
                        .iter()
                        .map(|(key, value)| ((*key).into(), (*value).into()))
                        .collect(),
                })
                .unwrap()
        };
        let click = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, item: &str| {
            let held = ItemStack::new(item, 1);
            game.remote_block_reacts(&top_of(0, 0), Some(&held), true).0
        };
        // RedStoneOreBlock.useItemOn: SUCCESS, unless a block item would go on top (PASS).
        game.world
            .set(
                0,
                64,
                0,
                state("minecraft:redstone_ore", &[("lit", "false")]),
            )
            .unwrap();
        assert!(click(game, "minecraft:stick").should_swing());
        assert!(!click(game, "minecraft:cobblestone").consumes_action());
        // PumpkinBlock.useItemOn: shears are SUCCESS; anything else goes on.
        game.world
            .set(0, 64, 0, state("minecraft:pumpkin", &[]))
            .unwrap();
        assert!(click(game, "minecraft:shears").should_swing());
        assert!(!click(game, "minecraft:stick").consumes_action());
        // WeatheringCopperGolemStatueBlock.useItemOn: an axe on an unaffected statue is SUCCESS.
        game.world
            .set(
                0,
                64,
                0,
                state(
                    "minecraft:copper_golem_statue",
                    &[
                        ("copper_golem_pose", "standing"),
                        ("facing", "north"),
                        ("waterlogged", "false"),
                    ],
                ),
            )
            .unwrap();
        assert!(click(game, "minecraft:iron_axe").should_swing());
        // BeehiveBlock.useItemOn on a client level: a full hive takes a glass bottle; its
        // shears are the server's, so they go on.
        game.world
            .set(
                0,
                64,
                0,
                state(
                    "minecraft:beehive",
                    &[("facing", "north"), ("honey_level", "5")],
                ),
            )
            .unwrap();
        assert!(click(game, "minecraft:glass_bottle").consumes_action());
        assert!(!click(game, "minecraft:shears").consumes_action());
    });
}
