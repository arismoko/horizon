//! Post-placement notices through the actual player and dispenser block-item paths.
use mod_api::{
    BlockEntityPart, BlockPart, Hooks, HostBlockStates, ItemPart, OnBlockPlacedByEvent, ServerHost,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_items::items::ItemStack;

pub const CRATE: &str = "placed_probe:crate";
pub const SKULL: &str = "minecraft:wither_skeleton_skull";

#[derive(Debug, PartialEq)]
pub struct Notice {
    pub item: String,
    pub count: u32,
    pub placer: Option<u128>,
    pub pos: [i32; 3],
    pub face: mod_api::BlockFace,
    pub click: [f64; 3],
}

#[derive(Clone, Default)]
pub struct Probe {
    pub notices: Arc<Mutex<Vec<Notice>>>,
    pub cancel: Arc<AtomicBool>,
}

pub(in crate::game) struct Crate;
#[mod_api::sunrise_part]
impl BlockPart for Crate {}
#[mod_api::sunrise_part]
impl BlockEntityPart for Crate {
    fn block_entity(
        &self,
        _: mod_api::LevelSide<'_>,
        _: &mut mod_api::BlockEntityEvent<'_>,
    ) -> Option<Compound> {
        Some(Compound::from([("Marker".into(), Tag::Int(0))]))
    }
}

#[mod_api::sunrise_part]
impl ItemPart for Probe {
    fn on_block_placed_by(&self, host: &mut dyn ServerHost, event: &mut OnBlockPlacedByEvent<'_>) {
        let level = event.level;
        assert_eq!(level, host.level("minecraft:overworld").unwrap());
        assert_eq!(level.block_state(host, event.pos), Some(event.state));
        let item = event
            .stack
            .item()
            .id(host)
            .expect("placed item is registered")
            .to_owned();
        match item.as_str() {
            CRATE => {
                let entity = host
                    .block_entity(level, event.pos)
                    .expect("new block entity");
                assert_eq!(
                    entity.data(host).unwrap().get(&"Marker".into()),
                    Some(&Tag::Int(7)),
                    "stack block-entity data must already be loaded"
                );
                let name = event
                    .stack
                    .components()
                    .unwrap()
                    .as_nbt()
                    .get(&"minecraft:custom_name".into())
                    .unwrap()
                    .clone();
                assert_eq!(
                    entity.merge_data(
                        host,
                        Compound::from([
                            ("CustomName".into(), name),
                            ("PlacedByPart".into(), Tag::Byte(1)),
                        ])
                    ),
                    Ok(true)
                );
            }
            "minecraft:red_bed" => {
                assert_eq!(
                    event.state.property(&host.block_states(), "part").unwrap(),
                    Some("foot")
                );
                let [x, y, z] = event.pos;
                let head = level
                    .block_state(host, [x, y, z + 1])
                    .expect("bed head exists");
                assert_eq!(head.block(), event.state.block());
                assert_eq!(
                    head.property(&host.block_states(), "part").unwrap(),
                    Some("head")
                );
            }
            "minecraft:shulker_box" => {
                assert!(event.placer.is_none(), "dispensers have no placer");
                assert_eq!(
                    event
                        .state
                        .property(&host.block_states(), "facing")
                        .unwrap(),
                    Some("north")
                );
                let entity = host
                    .block_entity(level, event.pos)
                    .expect("shulker entity exists");
                assert_eq!(entity.name(host).as_deref(), Some("Shulker"));
            }
            SKULL => {
                assert_eq!(event.state.block().id(host), Some("minecraft:air"));
                crate::game::dispense_fixture::record(|| "item notice".into());
            }
            _ => panic!("unsubscribed item {item}"),
        }
        self.notices.lock().unwrap().push(Notice {
            item,
            count: event.stack.count(),
            placer: event.placer.map(|player| player.uuid()),
            pos: event.pos,
            face: event.face,
            click: event.click,
        });
    }
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("crate").blocks(&[CRATE]).build();
        r.block_entity_part(&kind, Crate).unwrap();
        for item in [CRATE, "minecraft:red_bed", "minecraft:shulker_box", SKULL] {
            let key = r.item_key(item);
            r.item_part(key, self.clone());
        }
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new(CRATE, "minecraft:stone"))
            .unwrap();
    }
    fn entity_place(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::EntityPlaceEvent,
    ) -> mod_api::Flow {
        if self.cancel.load(Ordering::SeqCst) {
            if event.placer.is_none() {
                assert!(matches!(event.placement, mod_api::Placement::Single));
            }
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}

pub fn crate_stack() -> ItemStack {
    let mut stack = ItemStack::new(CRATE, 3);
    stack.data_mut().custom_name = Some(Tag::String("Crate name".into()));
    stack.data_mut().set_other(
        "minecraft:block_entity_data",
        Some(Tag::Compound(Compound::from([
            ("id".into(), Tag::String(CRATE.into())),
            ("Marker".into(), Tag::Int(7)),
        ]))),
    );
    stack
}

pub fn shulker_stack() -> ItemStack {
    let mut stack = ItemStack::new("minecraft:shulker_box", 4);
    stack.data_mut().custom_name = Some(Tag::String("Shulker".into()));
    stack.data_mut().set_other(
        "minecraft:block_state",
        Some(Tag::Compound(Compound::from([(
            "facing".into(),
            Tag::String("north".into()),
        )]))),
    );
    stack
}
