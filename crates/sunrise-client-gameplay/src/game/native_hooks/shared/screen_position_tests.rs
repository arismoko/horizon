use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::native_hooks::mod_api;

#[derive(Debug, PartialEq)]
pub struct Frame {
    pub pointer: [i32; 2],
    pub size: [i32; 2],
    pub origin: [f32; 2],
}

#[derive(Default)]
pub struct ScreenEvents {
    /// The block and position captured by the drawn screen's factory.
    pub draw: Vec<(Option<String>, Option<[i32; 3]>)>,
    pub frames: Vec<Frame>,
    pub tooltip_at: Vec<[f32; 2]>,
    pub data: Vec<Vec<i32>>,
    pub(in crate::game) data_slots: Vec<usize>,
    pub(in crate::game) extra: Vec<(i32, Vec<u8>)>,
    pub cancel_opening: bool,
}
pub type Seen = Arc<Mutex<ScreenEvents>>;
/// A mod menu type no block opens here.
pub const PACK: &str = "test:pack";

pub(in crate::game) struct Position(pub(in crate::game) Seen);
impl mod_api::MenuScreenFactory for Position {
    fn create(
        &self,
        _: &dyn mod_api::ClientHost,
        open: mod_api::MenuScreenOpen<'_>,
    ) -> Option<mod_api::MenuScreen> {
        // A block's screen leaves slot 4 out; without a block it shows none.
        {
            let mut seen = self.0.lock().unwrap();
            seen.extra.push((open.window, open.extra.to_vec()));
            seen.data_slots.push(open.data_slots);
        }
        let slots = match open.block {
            Some(_) => (0..open.backing_slots)
                .map(|i| (i != 4).then(mod_api::ScreenSlot::default))
                .collect(),
            None => Vec::new(),
        };
        let (block, pos) = (open.block.map(str::to_owned), open.pos);
        let draw_seen = self.0.clone();
        let tooltip_seen = self.0.clone();
        Some(
            mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                width: 176.0,
                height: if block.is_some() { 166.0 } else { 133.0 },
                inventory_x: 8.0,
                inventory_y: 84.0,
                slots,
                own_labels: true,
                own_background: true,
                ..Default::default()
            })
            .with_draw(move |_, frame, _| {
                let mut seen = draw_seen.lock().unwrap();
                seen.draw.push((block.clone(), pos));
                seen.data.push(frame.data.to_vec());
                seen.frames.push(Frame {
                    pointer: frame.pointer,
                    size: frame.gui_size,
                    origin: frame.origin,
                });
            })
            .with_tooltip(move |_, frame, _, _| {
                let mut seen = tooltip_seen.lock().unwrap();
                let draw = seen.frames.last().unwrap();
                assert_eq!(frame.pointer, draw.pointer);
                assert_eq!(frame.gui_size, draw.size);
                assert_eq!(frame.origin, draw.origin);
                seen.tooltip_at.push(frame.at);
            }),
        )
    }
}
pub(in crate::game) struct Listener(pub(in crate::game) Seen);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Listener {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_menu_screen(PACK, Arc::new(Position(self.0.clone())))
            .unwrap();
    }

    fn screen_opening(
        &self,
        _: &dyn mod_api::ClientHost,
        _: &mut mod_api::ScreenOpeningEvent<'_>,
    ) -> Option<mod_api::ScreenOpening> {
        self.0
            .lock()
            .unwrap()
            .cancel_opening
            .then_some(mod_api::ScreenOpening::Cancel)
    }
}
pub fn with_listener(test: impl FnOnce(&mut crate::game::ClientGame<'_>, &Seen)) {
    let seen = Seen::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(Listener(seen.clone())),
    )]
    .into();
    mod_api::testing::engine_platform::with_mods(mods.clone(), || {
        mod_api::testing::engine_platform::install_parts_fixture(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_client_game(|game| test(game, &seen));
    });
}

#[test]
#[ignore = "daily"]
fn screen_extra_data_borrows_only_the_open_windows_bytes() {
    use sunrise_protocol::{
        connection::Pipe,
        menu_extra::{self, CHANNEL, LIVE_DATA_REVISION, Message},
        packets::{
            configuration::CustomPayload,
            play::{Clientbound as C, ContainerClose, OpenScreen},
        },
        registry::Ids,
    };

    sunrise_level::containers::with_mod_menu_types(
        [(
            PACK.into(),
            "test".into(),
            0,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            None,
        )],
        || {
            with_listener(|game, seen| {
                let (pipe, _server) = Pipe::pair();
                game.connect_local(pipe, "Extra bytes").unwrap();
                let remote = game.client.remote.as_mut().unwrap();
                Arc::make_mut(&mut remote.registries).menu_extra_revision = LIVE_DATA_REVISION;
                Arc::make_mut(&mut remote.builtin)
                    .registries
                    .insert("minecraft:menu".into(), Ids::from_names(vec![PACK.into()]));

                let receive = |game: &mut crate::game::ClientGame<'_>, window, bytes: &[u8]| {
                    game.handle_packet(C::CustomPayload(CustomPayload {
                        channel: CHANNEL.into(),
                        data: menu_extra::encode(&Message {
                            window,
                            pos: None,
                            data: bytes.to_vec(),
                        })
                        .unwrap(),
                    }))
                    .unwrap();
                };
                let open = |game: &mut crate::game::ClientGame<'_>, window| {
                    game.handle_packet(C::OpenScreen(OpenScreen {
                        container: window,
                        menu: 0,
                        title: sunrise_core::nbt::Tag::String("Test".into()),
                    }))
                    .unwrap();
                };
                let expected = [0x12, 0x34, 0x56];
                receive(game, 1, &expected);
                assert!(seen.lock().unwrap().extra.is_empty());
                open(game, 1);
                assert_eq!(seen.lock().unwrap().extra, [(1, expected.to_vec())]);

                open(game, 2); // No payload at all.
                receive(game, 4, &[0x78]);
                assert_eq!(seen.lock().unwrap().extra.len(), 2);
                open(game, 3); // Discards the payload for window 4.
                open(game, 4); // The mismatched payload was not saved for later.
                let calls = [
                    (1, expected.to_vec()),
                    (2, vec![]),
                    (3, vec![]),
                    (4, vec![]),
                ];
                assert_eq!(seen.lock().unwrap().extra, calls);
                game.handle_packet(C::ContainerClose(ContainerClose { container: 4 }))
                    .unwrap();
                assert!(game.shown_native_screen().is_none());
                assert_eq!(seen.lock().unwrap().extra, calls);
            })
        },
    );
}

pub const CRATE_MENU: &str = "test:crate_menu";

/// A mod that adds `test:crate`, a block whose container opens `test:crate_menu`,
/// with the `Position` screen registered for that menu type.
pub struct CrateMod(pub Seen);
#[mod_api::__subscriptions]
impl mod_api::Hooks for CrateMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let block = r.block_key("test:crate");
        r.block_container(
            &block,
            mod_api::ModContainer {
                slots: 27,
                menu: Some(CRATE_MENU.into()),
                ..Default::default()
            },
        );
        r.register_menu_screen(CRATE_MENU, Arc::new(Position(self.0.clone())))
            .unwrap();
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new("test:crate", "minecraft:barrel").with_item(false))
            .unwrap();
    }
}

/// What the hopper screens heard, as (the screen's mod, the callback).
pub type Heard = Arc<Mutex<Vec<(&'static str, &'static str)>>>;
/// Mod `owner`'s screen for `menu`: it leaves slot `HIDDEN` inactive at open,
/// its button is `HIDDEN` and its tooltip names the owner.
#[derive(Clone)]
pub struct HopperScreen<const HIDDEN: usize> {
    pub owner: &'static str,
    pub menu: &'static str,
    pub heard: Heard,
}
impl<const HIDDEN: usize> mod_api::MenuScreenFactory for HopperScreen<HIDDEN> {
    fn create(
        &self,
        _: &dyn mod_api::ClientHost,
        open: mod_api::MenuScreenOpen<'_>,
    ) -> Option<mod_api::MenuScreen> {
        assert_eq!(open.backing_slots, 5);
        assert_eq!(open.player_slots, 36);
        assert_eq!(open.menu, self.menu);
        let owner = self.owner;
        let menu = self.menu;
        self.heard.lock().unwrap().push((owner, "create"));
        let draw = self.heard.clone();
        let click = self.heard.clone();
        let tooltip = self.heard.clone();
        Some(
            mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                width: 176.0,
                height: 133.0,
                inventory_x: 8.0,
                inventory_y: 51.0,
                slots: (0..5)
                    .map(|i| {
                        (i != HIDDEN).then_some(mod_api::ScreenSlot {
                            position: [44.0 + 18.0 * i as f32, 20.0],
                            no_item_icon: None,
                        })
                    })
                    .collect(),
                own_labels: true,
                own_background: true,
                ..Default::default()
            })
            .with_draw(move |host, _, _| {
                assert_eq!(mod_api::content::crash::current_native_mod(), Some(owner));
                let slot = usize::from(HIDDEN == 0);
                assert_eq!(
                    host.screen_slot(menu, slot),
                    Some([44.0 + 18.0 * slot as f32, 20.0])
                );
                draw.lock().unwrap().push((owner, "draw"));
            })
            .with_click(move |_, _, _| {
                click.lock().unwrap().push((owner, "click"));
                Some(HIDDEN as i32)
            })
            .with_tooltip(move |_, _, _, lines| {
                tooltip.lock().unwrap().push((owner, "tooltip"));
                lines.push((mod_api::Text::literal(owner), 0));
            }),
        )
    }
}
/// Mod `owner`: its block `<owner>:machine` opens `<owner>:hopper`, while it registers
/// its screen factory for `screen.menu` (testing cross-mod registration).
pub struct HopperMod<const HIDDEN: usize> {
    pub block_menu: &'static str,
    pub screen: HopperScreen<HIDDEN>,
}
#[mod_api::__subscriptions]
impl<const HIDDEN: usize> mod_api::Hooks for HopperMod<HIDDEN> {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let id = format!("{}:machine", self.screen.owner);
        let block = r.block_key(&id);
        r.block_container(
            &block,
            mod_api::ModContainer {
                slots: 5,
                menu: Some(self.block_menu.into()),
                ..Default::default()
            },
        );
        r.register_menu_screen(self.screen.menu, Arc::new(self.screen.clone()))
            .unwrap();
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        let id = format!("{}:machine", self.screen.owner);
        event
            .register(mod_api::Block::new(&id, "minecraft:barrel").with_item(false))
            .unwrap();
    }
}
