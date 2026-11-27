#[cfg(feature = "client")]
use mod_api::MustSurviveEvent;
use mod_api::{
    BlockPart, BlockState, Hooks, ItemPart, LevelSide, PlacementContextEvent, PlacementSource,
    Registrar, StateForPlacementEvent,
};
#[cfg(feature = "client")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(feature = "client")]
use sunrise_core::geometry::Vec3;
#[cfg(feature = "client")]
use sunrise_core::nbt::Tag;
use sunrise_core::{geometry::Direction, position::BlockPos};
#[cfg(feature = "client")]
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::GameGameExt1;
#[cfg(feature = "client")]
use sunrise_items::items::ItemStack;
#[cfg(feature = "client")]
use sunrise_sim::motion::PlayerMotion;
#[cfg(feature = "client")]
use sunrise_world::level::{PlaceContext, PlacementSource as EnginePlacementSource};

#[cfg(feature = "client")]
pub(crate) struct Move;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl ItemPart for Move {
    fn placement_context(
        &self,
        _: LevelSide<'_>,
        event: &mut PlacementContextEvent<'_>,
    ) -> Option<Option<mod_api::PlacementContext>> {
        let mut context = event.context.clone();
        match event.stack.count() {
            2 => return None,
            3 => {
                let placer = event.placer.expect("the local predicting player");
                assert_eq!(placer.player().entity().id(), 7);
                assert_eq!(placer.player().uuid(), 42);
                assert_eq!(
                    placer.eye_position(),
                    [12.5, 64.0 + f64::from(1.62_f32), 12.5]
                );
                assert_eq!(
                    event
                        .stack
                        .components()
                        .unwrap()
                        .as_nbt()
                        .get(&"minecraft:custom_name".into()),
                    Some(&Tag::String("Marker".into()))
                );
            }
            _ => {}
        }
        context.pos.y -= i32::try_from(event.stack.count()).unwrap() + 1;
        context.face = Direction::Down;
        Some(Some(context))
    }
}
#[cfg(feature = "client")]
pub(crate) struct Refuse;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl ItemPart for Refuse {
    fn placement_context(
        &self,
        _: LevelSide<'_>,
        _: &mut PlacementContextEvent<'_>,
    ) -> Option<Option<mod_api::PlacementContext>> {
        Some(None)
    }
}
#[cfg(feature = "client")]
pub(crate) enum Invalid {
    Nonfinite,
    Item,
    Bounds,
}
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl ItemPart for Invalid {
    fn placement_context(
        &self,
        _: LevelSide<'_>,
        event: &mut PlacementContextEvent<'_>,
    ) -> Option<Option<mod_api::PlacementContext>> {
        let mut context = event.context.clone();
        match self {
            Self::Nonfinite => context.hit[0] = f64::NAN,
            Self::Item => {
                context.item = Some(mod_api::__private::item_key("minecraft:glass").unwrap())
            }
            Self::Bounds => context.pos.y = 10_000,
        }
        Some(Some(context))
    }
}
/// Rewrites the placement into a dispenser-like `DirectionalPlaceContext` at the same cell,
/// along and clicking East, and claims the clicked block is not replaced.
#[cfg(feature = "client")]
pub(crate) struct Directional;
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl ItemPart for Directional {
    fn placement_context(
        &self,
        _: LevelSide<'_>,
        event: &mut PlacementContextEvent<'_>,
    ) -> Option<Option<mod_api::PlacementContext>> {
        let mut context = mod_api::PlacementContext::directional(
            event.context.pos,
            Direction::East,
            Direction::East,
            event.context.item,
        );
        context.replace_clicked = false;
        Some(Some(context))
    }
}
#[cfg(feature = "client")]
pub(crate) enum Survival {
    Skip,
    RequireWhen(Arc<AtomicBool>),
}
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl ItemPart for Survival {
    fn must_survive(&self, _: LevelSide<'_>, _: &mut MustSurviveEvent<'_>) -> Option<bool> {
        match self {
            Self::Skip => Some(false),
            Self::RequireWhen(required) => required.load(Ordering::Relaxed).then_some(true),
        }
    }
}
#[cfg(feature = "client")]
pub(crate) struct Parts(pub(crate) Arc<AtomicBool>);
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for Parts {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let stone = r.item_key("minecraft:stone");
        r.item_part(stone, Move);
        let dandelion = r.item_key("minecraft:dandelion");
        r.item_part(dandelion, Move);
        let dirt = r.item_key("minecraft:dirt");
        r.item_part(dirt, Refuse);
        let sand = r.item_key("minecraft:sand");
        r.item_part(sand, Invalid::Nonfinite);
        let red_sand = r.item_key("minecraft:red_sand");
        r.item_part(red_sand, Invalid::Item);
        let gravel = r.item_key("minecraft:gravel");
        r.item_part(gravel, Invalid::Bounds);
        let poppy = r.item_key("minecraft:poppy");
        r.item_part(poppy, Survival::Skip);
        let torch = r.item_key("minecraft:torch");
        r.item_part(torch, Survival::Skip);
        let scaffolding = r.item_key("minecraft:scaffolding");
        r.item_part(scaffolding, Survival::RequireWhen(self.0.clone()));
        for item in [
            "minecraft:oak_trapdoor",
            "minecraft:pointed_dripstone",
            "minecraft:oak_slab",
        ] {
            let key = r.item_key(item);
            r.item_part(key, Directional);
        }
    }
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn item_parts_rewrite_or_refuse_context_and_control_survival_on_both_sides() {
    let require_scaffolding_survival = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "placement",
        Box::new(Parts(require_scaffolding_survival.clone())),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let registry = game.pack.assets.registry;
            let air = registry.default_state("minecraft:air").unwrap();
            let scaffolding = registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:scaffolding".into(),
                    properties: [
                        ("bottom", "true"),
                        ("distance", "6"),
                        ("waterlogged", "false"),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
                })
                .unwrap();
            for authoritative in [true, false] {
                game.with_level(|b, level, ticks| {
                    level.native_authoritative = authoritative;
                    level.client_side = !authoritative;
                    let context = |item: &str| PlaceContext {
                        clicked_pos: BlockPos::new(4, 64, 4),
                        face: Direction::Up,
                        hit: Vec3::new(4.5, 64.0, 4.5),
                        inside: false,
                        source: EnginePlacementSource::Player {
                            yaw: 0.0,
                            pitch: 0.0,
                            sneaking: false,
                        },
                        item: item.into(),
                        can_use_game_master_blocks: false,
                    };
                    let preview = |level: &mut sunrise_gameplay_common::game::GameLevel<'_>,
                                   ticks: &mut _,
                                   item: &str| {
                        let stack = ItemStack::new(item, 1);
                        b.decide_placement(
                            level,
                            ticks,
                            &context(item),
                            |s| (s, s),
                            |level, context| level.rewrite_placement_context(&stack, context, None),
                        )
                        .map(|(_, preview)| preview)
                    };
                    let placed = preview(level, ticks, "minecraft:stone").unwrap();
                    assert_eq!(placed.pos, BlockPos::new(4, 62, 4));
                    let fallback = preview(level, ticks, "minecraft:glass").unwrap();
                    assert_eq!(fallback.pos, BlockPos::new(4, 64, 4));
                    assert!(preview(level, ticks, "minecraft:dirt").is_none());
                    for item in ["minecraft:sand", "minecraft:red_sand", "minecraft:gravel"] {
                        assert!(preview(level, ticks, item).is_none(), "{item}");
                    }
                    assert!(
                        b.placement_preview(level, ticks, &context("minecraft:poppy"))
                            .is_some(),
                        "survival can be skipped, even with no soil"
                    );
                    assert!(
                        b.placement_preview(level, ticks, &context("minecraft:torch"))
                            .is_none(),
                        "standing/wall candidates must survive even when final survival is skipped"
                    );
                    // Unlike the vanilla rewrite, a part cannot target a nonreplaceable cell.
                    b.set_block(level, ticks, BlockPos::new(4, 62, 4), fallback.state, 3);
                    assert!(preview(level, ticks, "minecraft:stone").is_none());
                    b.set_block(level, ticks, BlockPos::new(4, 62, 4), air, 3);
                });
            }

            // A part's directional rewrite is a `DirectionalPlaceContext`: its
            // `replacingClickedOnBlock()` comes from the clicked block, not the `false` the part
            // wrote, and a question vanilla throws on refuses the placement.
            let stone = registry.default_state("minecraft:stone").unwrap();
            let slab = registry.default_state("minecraft:oak_slab").unwrap();
            game.with_level(|b, level, ticks| {
                level.native_authoritative = true;
                level.client_side = false;
                let at = BlockPos::new(4, 64, 4);
                let context = |item: &str| PlaceContext {
                    clicked_pos: at,
                    face: Direction::Up,
                    hit: Vec3::new(4.5, 64.0, 4.5),
                    inside: false,
                    source: EnginePlacementSource::Player {
                        yaw: 0.0,
                        pitch: 0.0,
                        sneaking: false,
                    },
                    item: item.into(),
                    can_use_game_master_blocks: false,
                };
                let rewritten = |level: &mut sunrise_gameplay_common::game::GameLevel<'_>,
                                 ticks: &mut _,
                                 item: &str| {
                    let stack = ItemStack::new(item, 1);
                    b.decide_placement(
                        level,
                        ticks,
                        &context(item),
                        |s| (s, s),
                        |level, context| level.rewrite_placement_context(&stack, context, None),
                    )
                    .map(|(_, preview)| preview)
                };
                let trapdoor = rewritten(level, ticks, "minecraft:oak_trapdoor")
                    .expect("the clicked air is replaced");
                assert_eq!(trapdoor.pos, at);
                let trapdoor = registry.state(trapdoor.state).unwrap();
                assert_eq!(trapdoor.properties["facing"], "west");
                assert_eq!(trapdoor.properties["half"], "top");

                b.set_block(level, ticks, BlockPos::new(4, 65, 4), stone, 3);
                b.set_block(level, ticks, BlockPos::new(4, 63, 4), stone, 3);
                assert!(
                    rewritten(level, ticks, "minecraft:pointed_dripstone").is_none(),
                    "no vertical look"
                );
                let tip = b
                    .placement_preview(level, ticks, &context("minecraft:pointed_dripstone"))
                    .expect("a player's placement is unchanged");
                assert_eq!(
                    registry.state(tip.state).unwrap().properties["vertical_direction"],
                    "up"
                );

                b.set_block(level, ticks, at, slab, 3);
                assert!(
                    rewritten(level, ticks, "minecraft:oak_slab").is_none(),
                    "the slab's canBeReplaced asks the question it answers"
                );
                assert!(
                    b.placement_preview(level, ticks, &context("minecraft:oak_slab"))
                        .is_some(),
                    "a player doubles the slab"
                );
                let air = registry.default_state("minecraft:air").unwrap();
                for y in 63..=65 {
                    b.set_block(level, ticks, BlockPos::new(4, y, 4), air, 3);
                }
            });

            // Click the end of a scaffold extending six blocks from support.
            // Its horizontal walk reaches distance seven, where only vanilla's
            // skipped survival check lets the item place.
            game.world.set(4, 64, 4, scaffolding).unwrap();
            game.with_level(|b, level, ticks| {
                let context = PlaceContext {
                    clicked_pos: BlockPos::new(4, 64, 4),
                    face: Direction::Up,
                    hit: Vec3::new(4.5, 65.0, 4.5),
                    inside: false,
                    source: EnginePlacementSource::Player {
                        yaw: 0.0,
                        pitch: 0.0,
                        sneaking: false,
                    },
                    item: "minecraft:scaffolding".into(),
                    can_use_game_master_blocks: false,
                };
                assert_eq!(
                    b.placement_preview(level, ticks, &context).unwrap().pos,
                    BlockPos::new(4, 64, 5),
                );
                require_scaffolding_survival.store(true, Ordering::Relaxed);
                assert!(b.placement_preview(level, ticks, &context).is_none());
            });
        });

        // The connected client actually predicts through DryLevel.
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "PlacementPrediction").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player =
                Some(PlayerMotion::new(game.motion, Vec3::new(12.5, 64.0, 12.5)).unwrap());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            game.set_local_mode(sunrise_command::args::GameMode::Survival);
            let registry = game.pack.assets.registry;
            let air = registry.default_state("minecraft:air").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(4, 64, 4, stone).unwrap();
            game.world.set(4, 65, 4, stone).unwrap();
            let hit = sunrise_gameplay_common::game::Hit {
                pos: [4, 64, 4],
                face: [0, 1, 0],
                point: [4.5, 65.0, 4.5],
                inside: false,
            };
            let stack = |item| ItemStack::new(item, 1);
            assert!(
                !sunrise_client_gameplay::game::ClientGame::can_hint_block_placement(&stack(
                    "minecraft:stone"
                ))
            );
            assert!(
                !sunrise_client_gameplay::game::ClientGame::can_hint_block_placement(&stack(
                    "minecraft:poppy"
                ))
            );
            assert!(
                sunrise_client_gameplay::game::ClientGame::can_hint_block_placement(&stack(
                    "minecraft:glass"
                ))
            );
            assert!(
                game.remote_can_place(&hit, &stack("minecraft:glass"))
                    .is_none()
            );
            assert!(!game.remote_would_place(&hit, &stack("minecraft:stone")));
            let decline = ItemStack::new("minecraft:stone", 2);
            assert!(game.remote_can_place(&hit, &decline).is_none());
            let mut redirect = ItemStack::new("minecraft:stone", 3);
            redirect.data_mut().custom_name = Some(Tag::String("Marker".into()));
            let placed = game
                .remote_can_place(&hit, &redirect)
                .expect("redirect from the occupied original destination");
            let pos = placed.pos;
            assert_eq!(pos, BlockPos::new(4, 61, 4));
            assert!(placed.changes.contains(&(pos, stone)));
            assert_eq!(game.world.block(pos.x, pos.y, pos.z), air);

            game.player.as_mut().unwrap().position = Vec3::new(4.5, 63.0, 4.5);
            assert!(
                game.remote_can_place(&hit, &stack("minecraft:stone"))
                    .is_none(),
                "the player obstructs the final destination"
            );
            game.player.as_mut().unwrap().position = Vec3::new(12.5, 64.0, 12.5);
            game.world.set(4, 65, 4, air).unwrap();
            assert_eq!(
                game.remote_can_place(&hit, &decline).unwrap().pos,
                BlockPos::new(4, 65, 4)
            );
            let soil = registry.default_state("minecraft:dirt").unwrap();
            game.world.set(4, 64, 4, soil).unwrap();
            assert!(game.remote_would_place(&hit, &stack("minecraft:dandelion")));
            assert!(
                game.remote_can_place(&hit, &stack("minecraft:dandelion"))
                    .is_none(),
                "the final destination lacks support"
            );
            game.world.set(4, 64, 4, air).unwrap();
            assert!(
                game.remote_can_place(&hit, &stack("minecraft:dirt"))
                    .is_none()
            );
            assert!(
                game.remote_can_place(&hit, &stack("minecraft:poppy"))
                    .is_some()
            );
        })
    });
}

pub(crate) struct Substitute(pub(crate) &'static str);
#[mod_api::sunrise_part]
impl BlockPart for Substitute {
    fn state_for_placement(
        &self,
        level: LevelSide<'_>,
        _: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockState>> {
        let (LevelSide::Server(level) | LevelSide::Client(level)) = level;
        Some(level.block_state_id(self.0))
    }
}
pub(crate) struct PlacementSubstitution;
#[mod_api::__subscriptions]
impl Hooks for PlacementSubstitution {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let door = r.block_key("minecraft:oak_door");
        r.block_part(door, Substitute("minecraft:stone"));
        let furnace = r.block_key("minecraft:furnace");
        r.block_part(furnace, Substitute("minecraft:oak_door"));
    }

    fn entity_place(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::EntityPlaceEvent,
    ) -> mod_api::Flow {
        if event.pos.z != 9 {
            return mod_api::Flow::Continue;
        }
        let mod_api::Placement::Multi(writes) = &event.placement else {
            panic!("the held door pre-clears above the chosen stone");
        };
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].pos, event.pos);
        assert_eq!(
            Some(writes[0].state),
            host.block_state_id("minecraft:stone")
        );
        assert_eq!(writes[1].pos, event.pos.offset(0, 1, 0));
        assert_eq!(Some(writes[1].state), host.block_state_id("minecraft:air"));
        mod_api::Flow::Cancel
    }
}

#[derive(Debug, PartialEq)]
pub(in crate::game) struct Notice {
    pub pos: [i32; 3],
    pub(in crate::game) face: mod_api::BlockFace,
    pub(in crate::game) click: [f64; 3],
}

#[derive(Default)]
pub(in crate::game) struct VetoTrace {
    pub(in crate::game) occupy_target: bool,
    pub(in crate::game) decisions: Vec<BlockPos>,
    pub(in crate::game) vetoes: Vec<(BlockPos, mod_api::BlockState)>,
    pub(in crate::game) notices: Vec<Notice>,
}

#[derive(Clone)]
pub(in crate::game) struct VetoProbe(pub(in crate::game) Arc<Mutex<VetoTrace>>);

#[mod_api::sunrise_part]
impl ItemPart for VetoProbe {
    fn placement_context(
        &self,
        level: LevelSide<'_>,
        event: &mut PlacementContextEvent<'_>,
    ) -> Option<Option<mod_api::PlacementContext>> {
        let (LevelSide::Server(level) | LevelSide::Client(level)) = level;
        let mut context = event.context.clone();
        assert_eq!(event.stack.count(), 2);
        if context.pos.x == 4 {
            let placer = event.placer.expect("the acting server player");
            assert_eq!(placer.player().entity().id(), 20);
            assert_eq!(placer.player().uuid(), 123);
            assert_eq!(
                placer.eye_position(),
                [12.5, 64.0 + f64::from(1.62_f32), 12.5]
            );
            assert!(
                matches!(context.source, PlacementSource::Player { .. }),
                "player placement carries PlacementSource::Player"
            );
        } else {
            assert!(event.placer.is_none(), "a dispenser has no player");
            assert_eq!(
                context.source,
                PlacementSource::Directional {
                    direction: Direction::East,
                },
                "dispenser shulker box carries Directional {{ East }}"
            );
        }
        context.pos.z += 2;
        let BlockPos { x, y, z } = context.pos;
        if level.block_state([x, y, z]) != level.block_state_id("minecraft:air") {
            context.pos.z += 1;
        }
        context.face = Direction::Down;
        context.hit = [
            f64::from(x) + 0.125,
            f64::from(y) + 0.75,
            f64::from(context.pos.z) + 0.875,
        ];
        self.0.lock().unwrap().decisions.push(context.pos);
        Some(Some(context))
    }

    fn on_block_placed_by(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::OnBlockPlacedByEvent<'_>,
    ) {
        self.0.lock().unwrap().notices.push(Notice {
            pos: event.pos,
            face: event.face,
            click: event.click,
        });
    }
}

#[mod_api::__subscriptions]
impl Hooks for VetoProbe {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for item in ["minecraft:stone", "minecraft:shulker_box"] {
            let key = r.item_key(item);
            r.item_part(key, self.clone());
        }
    }
    fn entity_place(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::EntityPlaceEvent,
    ) -> mod_api::Flow {
        let occupy_target = {
            let mut trace = self.0.lock().unwrap();
            trace.vetoes.push((event.pos, event.placed_against));
            trace.occupy_target
        };
        if occupy_target {
            let level = host.level("minecraft:overworld").unwrap();
            let marker = host.block_state_id("minecraft:obsidian").unwrap();
            let BlockPos { x, y, z } = event.pos;
            assert_eq!(
                level.set_block(host, [x, y, z], marker, mod_api::BlockUpdateFlags::NONE),
                Ok(true)
            );
        }
        mod_api::Flow::Continue
    }
}

#[cfg(feature = "client")]
#[derive(Clone, Default)]
pub(in crate::game) struct SourceSpy(
    pub(in crate::game) Arc<Mutex<Vec<(bool, BlockPos, PlacementSource)>>>,
);

#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for SourceSpy {
    fn state_for_placement(
        &self,
        level: LevelSide<'_>,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockState>> {
        let on_server = matches!(level, LevelSide::Server(_));
        self.0
            .lock()
            .unwrap()
            .push((on_server, event.context.pos, event.context.source));
        None
    }
}

#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for SourceSpy {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for block in ["minecraft:stone", "minecraft:shulker_box"] {
            let key = r.block_key(block);
            r.block_part(key, self.clone());
        }
    }
}
