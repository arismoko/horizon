//! Handles to supported menus: client reads and clicks.
use crate::game::ClientGame;
use mod_api::__private::game::*;
use mod_api::{DragKind, LevelReadError, MenuClick, MenuRef, MouseButton, PlayerRef, SwapWith};
use sunrise_items::items::ItemStack;
use sunrise_sim::menu::Input;

impl ClientGame<'_> {
    /// Submit clicks on the local player's live menu through normal prediction
    /// and `ContainerClick`. Slots use the same menu order as `slot`; `None`
    /// is outside (-999). All clicks are validated before any prediction or send.
    /// A drag expands to one complete quick-craft gesture.
    /// Refuses stale menus, invalid slots/hotbar indices, spectators, non-client hosts,
    /// the creative inventory screen and empty batches without changing items.
    /// True means submitted, not accepted by the server: its corrections win.
    pub fn menu_click(&mut self, menu: MenuRef, clicks: &[MenuClick]) -> bool {
        if self.common.is_logical_server()
            || self.client.screen == super::Screen::Creative
            || self.common.spectator()
        {
            return false;
        }
        let Some(name) = self.native_client_menu_name(menu) else {
            return false;
        };
        let Some(ops) = client_click_ops(name, clicks, self.instabuild()) else {
            return false;
        };
        self.remote_menu_ops(&ops)
    }

    /// Clear the creative screen's local cursor without sending a packet.
    pub(in crate::game) fn menu_destroy_carried(
        &mut self,
        menu: MenuRef,
    ) -> Result<bool, mod_api::CarriedDestroyError> {
        use mod_api::CarriedDestroyError;
        if self.common.is_logical_server()
            || self.common.player.is_none()
            || self.common.res.rest.get().is_none()
            || !self.client.remote.as_ref().is_some_and(|r| r.logged_in)
        {
            return Err(CarriedDestroyError::Unavailable);
        }
        if self.native_client_menu_name(menu).is_none() {
            return Err(CarriedDestroyError::Stale);
        }
        if !self.common.creative {
            return Err(CarriedDestroyError::NotCreative);
        }
        // The visible screen can outlive a change to the active menu.
        if self.client.screen != super::Screen::Creative || self.remote_menu_type().is_some() {
            return Err(CarriedDestroyError::ServerOwnedMenu);
        }
        Ok(self.common.carried.take().is_some())
    }

    /// Close the local player's open container screen as Escape does
    /// (`AbstractContainerScreen.onClose`, `LocalPlayer.closeContainer`): the
    /// server is told (`ContainerClose`) and the screen goes. Refuses stale
    /// menus, non-client hosts and a menu whose screen is not open, such as
    /// the inventory menu while playing.
    pub(in crate::game) fn menu_close(&mut self, menu: MenuRef) -> bool {
        if self.common.is_logical_server()
            || self.client.screen != super::Screen::Inventory
            || self.native_client_menu_name(menu).is_none()
        {
            return false;
        }
        self.close_container_client();
        self.set_screen(super::Screen::Playing);
        self.capture(true);
        true
    }

    /// Press button `button` of the local player's open menu as a click on
    /// one of its buttons does (`MultiPlayerGameMode.handleInventoryButtonClick`,
    /// `ContainerButtonClick`): the server's menu decides what it does
    /// (`clickMenuButton`, and `menu_button` for a mod's menu). For a client
    /// mod's own input on a menu's screen other than a press, such as a key
    /// or the wheel; no sound, that is the screen's. Refuses
    /// stale menus, non-client hosts and the inventory menu, which no server
    /// opened. True means sent, not that the menu took it.
    pub(in crate::game) fn menu_click_button(&mut self, menu: MenuRef, button: i32) -> bool {
        !self.common.is_logical_server()
            && self.native_client_menu_name(menu).is_some()
            && self.remote_button_click(button)
    }

    pub(in crate::game) fn menu_slot_boxes(&self, menu: MenuRef) -> Option<Vec<Option<[f32; 4]>>> {
        if !matches!(
            self.client.screen,
            super::Screen::Inventory | super::Screen::Creative
        ) || !self.screen_hooks_open()
        {
            return None;
        }
        let name = self.native_client_menu_name(menu)?;
        // The framebuffer `Renderer::render` leaves behind while it has the renderer out, so a
        // frame callback reads the boxes too.
        let framebuffer = self.renderer_framebuffer()?;
        let batch = super::HudBatch::new(framebuffer, self.client.settings.gui_scale_inputs());
        self.inventory_menu_slot_boxes(menu.fields().window, name, &batch)
    }

    pub(in crate::game) fn menu_creative_catalog(
        &self,
        menu: MenuRef,
    ) -> Option<Vec<([f32; 4], Option<ItemStack>)>> {
        use sunrise_client::creative::Creative;
        if self.client.screen != super::Screen::Creative || !self.screen_hooks_open() {
            return None;
        }
        self.native_client_menu_name(menu)?;
        let framebuffer = self.renderer_framebuffer()?;
        if !self.creative_tab_open() {
            return Some(Vec::new());
        }
        let batch = super::HudBatch::new(framebuffer, self.client.settings.gui_scale_inputs());
        let (ox, oy) = Self::creative_origin(&batch);
        let shown = self
            .client
            .creative_screen
            .0
            .display_window(&self.common.pack.names);
        Some(
            Creative::display_slots()
                .map(|(slot, (x, y))| ([ox + x, oy + y, 16.0, 16.0], shown.stack(slot).cloned()))
                .collect(),
        )
    }

    pub(in crate::game::client_side) fn inventory_menu_slot_boxes(
        &self,
        window: i32,
        name: &str,
        batch: &super::HudBatch,
    ) -> Option<Vec<Option<[f32; 4]>>> {
        if self.book_covers_menu(batch.width) {
            return None;
        }
        let kind = self.native_screen_menu_type(window, name);
        let slot_id = |slot| {
            sunrise_gameplay_common::game::inventory_screen_slot_with_type(name, slot, || kind)
        };
        match kind {
            None => {
                slot_id(0)?;
            }
            Some(kind) if kind.slot_count() == 0 => return Some(Vec::new()),
            Some(_) => {}
        }
        let visible = self.slots();
        let (ox, oy) = self.inventory_origin(batch);
        Some(
            (0..)
                .map_while(slot_id)
                .map(|slot| {
                    visible.contains(&slot).then(|| {
                        let (x, y) = self.slot_position(slot);
                        [ox + x, oy + y, 16.0, 16.0]
                    })
                })
                .collect(),
        )
    }

    pub(in crate::game) fn menu_merchant_offers(
        &self,
        menu: MenuRef,
    ) -> Result<Option<&[sunrise_sim::merchant::MerchantOffer]>, LevelReadError> {
        if self.native_client_menu_name(menu) != Some("minecraft:merchant") {
            return Ok(None);
        }
        let merchant = self
            .common
            .station
            .as_ref()
            .and_then(|station| station.merchant.as_ref())
            .ok_or(LevelReadError::Unavailable)?;
        Ok(Some(&merchant.session.offers))
    }

    pub(in crate::game) fn native_client_menu_name(&self, menu: MenuRef) -> Option<&str> {
        let remote = self
            .client
            .remote
            .as_ref()
            .filter(|remote| remote.logged_in)?;
        let epoch = self.common.native_lifecycle.epoch()?;
        (menu.fields().player.fields().entity.epoch() == epoch
            && menu.fields().player.fields().entity.id()
                == u32::try_from(remote.entity_id).ok()?
            && menu.fields().player.fields().uuid == remote.profile
            && menu.fields().generation == remote.menu.generation
            && menu.fields().window == remote.menu.as_ref().map_or(0, |entry| entry.0))
        .then_some(())?;
        Some(
            remote
                .menu
                .as_ref()
                .map_or("inventory", |entry| entry.1.as_str()),
        )
    }

    pub(in crate::game) fn client_player_container_menu(
        &self,
        player: PlayerRef,
    ) -> Option<MenuRef> {
        self.player_read_both(player)?;
        let remote = self.client.remote.as_ref()?;
        let (window, name) = remote
            .menu
            .as_ref()
            .map_or((0, "inventory"), |entry| (entry.0, entry.1.as_str()));
        if self.native_screen_menu_type(window, name).is_none() {
            sunrise_gameplay_common::game::inventory_screen_slot_with_type(name, 0, || None)?;
        }
        Some(mod_api::__private::game::menu_ref(
            player,
            window,
            remote.menu.generation,
        ))
    }

    pub(in crate::game) fn client_menu_kind(&self, menu: MenuRef) -> Option<Option<String>> {
        let name = self.native_client_menu_name(menu)?;
        Some(
            (name != "inventory"
                && !name.starts_with(sunrise_gameplay_common::game::remote::MOUNT_MENU))
            .then(|| name.to_owned()),
        )
    }

    pub(in crate::game) fn client_menu_slot_count(&self, menu: MenuRef) -> Option<usize> {
        let name = self.native_client_menu_name(menu)?;
        if let Some(kind) = self.native_screen_menu_type(menu.fields().window, name) {
            return Some(kind.slot_count());
        }
        let count = (0..)
            .map_while(|slot| {
                sunrise_gameplay_common::game::inventory_screen_slot_with_type(name, slot, || None)
            })
            .count();
        (count != 0).then_some(count)
    }

    pub(in crate::game) fn client_menu_state_id(&self, menu: MenuRef) -> Option<i32> {
        let remote = self.client.remote.as_ref()?;
        self.native_client_menu_name(menu)?;
        Some(remote.state_id)
    }

    pub(in crate::game) fn client_menu_carried(&self, menu: MenuRef) -> Option<Option<ItemStack>> {
        self.native_client_menu_name(menu)?;
        Some(self.common.carried.clone())
    }

    pub(in crate::game) fn client_menu_slot(
        &self,
        menu: MenuRef,
        slot: usize,
    ) -> Option<Option<ItemStack>> {
        let name = self.native_client_menu_name(menu)?;
        let index =
            sunrise_gameplay_common::game::inventory_screen_slot(name, i32::try_from(slot).ok()?)?;
        Some(self.slot_view(index))
    }
}
pub(in crate::game) fn client_click_ops(
    menu: &str,
    clicks: &[MenuClick],
    creative: bool,
) -> Option<Vec<(Option<usize>, i32, Input)>> {
    if clicks.is_empty() {
        return None;
    }
    let mapped_slot = |slot| {
        sunrise_gameplay_common::game::inventory_screen_slot(
            menu,
            i32::from(i16::try_from(slot).ok()?),
        )
    };
    let mut ops = Vec::new();
    for click in clicks {
        let op = match click {
            MenuClick::Pickup { slot, button } => (
                match slot {
                    Some(slot) => Some(mapped_slot(*slot)?),
                    None => None,
                },
                match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                },
                Input::Pickup,
            ),
            MenuClick::QuickMove { slot } => (Some(mapped_slot(*slot)?), 0, Input::QuickMove),
            MenuClick::Swap { slot, with } => (
                Some(mapped_slot(*slot)?),
                match with {
                    SwapWith::Hotbar(index) if *index <= 8 => i32::from(*index),
                    SwapWith::Offhand => 40,
                    _ => return None,
                },
                Input::Swap,
            ),
            MenuClick::Clone { slot } if creative => (Some(mapped_slot(*slot)?), 2, Input::Clone),
            MenuClick::Throw { slot, whole_stack } => (
                Some(mapped_slot(*slot)?),
                i32::from(*whole_stack),
                Input::Throw,
            ),
            MenuClick::Drag { kind, slots } => {
                let mode = match kind {
                    DragKind::Split => 0,
                    DragKind::One => 4,
                    DragKind::Clone if creative => 8,
                    _ => return None,
                };
                ops.push((None, mode, Input::QuickCraft));
                for slot in slots {
                    ops.push((Some(mapped_slot(*slot)?), mode + 1, Input::QuickCraft));
                }
                ops.push((None, mode + 2, Input::QuickCraft));
                continue;
            }
            MenuClick::PickupAll { slot } => (Some(mapped_slot(*slot)?), 0, Input::PickupAll),
            _ => return None,
        };
        ops.push(op);
    }
    Some(ops)
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    #[cfg(any(test, feature = "test-support"))]
    use super::*;
    #[cfg(any(test, feature = "test-support"))]
    use std::sync::Mutex;
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
    };
    use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
    use sunrise_protocol::packets::configuration::ClientInformation;
    use sunrise_sim::lan::{Joined, PlayerSender, guest::Guest};

    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::game) const PANEL: &str = "test:panel";

    #[cfg(any(test, feature = "test-support"))]
    pub struct Panel {
        pub(in crate::game) opening: Arc<Mutex<Option<(MenuRef, usize)>>>,
    }
    #[cfg(any(test, feature = "test-support"))]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Panel {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_menu_screen(
                PANEL,
                Arc::new(
                    |_: &dyn mod_api::ClientHost, open: mod_api::MenuScreenOpen<'_>| {
                        Some(mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                            width: 176.0,
                            height: 166.0,
                            slots: (0..open.backing_slots)
                                .map(|index| {
                                    Some(mod_api::ScreenSlot {
                                        position: [
                                            8.0 + (index % 9) as f32 * 18.0,
                                            18.0 + (index / 9) as f32 * 18.0,
                                        ],
                                        no_item_icon: None,
                                    })
                                })
                                .collect(),
                            ..Default::default()
                        }))
                    },
                ),
            )
            .unwrap();
        }

        fn screen_opening(
            &self,
            host: &dyn mod_api::ClientHost,
            event: &mut mod_api::ScreenOpeningEvent<'_>,
        ) -> Option<mod_api::ScreenOpening> {
            if event.current.id == "playing" && event.new.menu == Some(PANEL) {
                let menu = host.local_player()?.container_menu(host)?;
                let count = menu.slot_count(host)?;
                *self.opening.lock().unwrap() = Some((menu, count));
            }
            None
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_panel(
        slots: usize,
        test: impl FnOnce(&mut crate::game::ClientGame<'_>, &Mutex<Option<(MenuRef, usize)>>),
    ) {
        with_panel_fixture(slots, |opening| {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let _server =
                    crate::game::client_side::native_client_session::tests::logged_client(game);
                install_panel_registry(game);
                test(game, opening);
            });
        });
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_panel_fixture(slots: usize, test: impl FnOnce(&Mutex<Option<(MenuRef, usize)>>)) {
        let opening = Arc::new(Mutex::new(None));
        let panel = Panel {
            opening: opening.clone(),
        };
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("test", Box::new(panel))].into();
        mod_api::testing::engine_platform::with_mods(mods.clone(), || {
            mod_api::testing::engine_platform::install_parts_fixture(&mods).unwrap();
            sunrise_level::containers::with_mod_menu_types(
                [(
                    PANEL.into(),
                    "test".into(),
                    slots,
                    0,
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                    None,
                )],
                || test(&opening),
            );
        });
    }

    #[cfg(any(test, feature = "test-support"))]
    fn install_panel_registry(game: &mut crate::game::ClientGame<'_>) {
        Arc::make_mut(&mut game.client.remote.as_mut().unwrap().builtin)
            .registries
            .insert(
                "minecraft:menu".into(),
                sunrise_protocol::registry::Ids::from_names(vec![PANEL.into()]),
            );
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::game::client_side) fn open_panel(game: &mut crate::game::ClientGame<'_>) {
        use sunrise_protocol::packets::play::{Clientbound, OpenScreen};
        game.handle_packet(Clientbound::OpenScreen(OpenScreen {
            container: 1,
            menu: 0,
            title: sunrise_core::nbt::Tag::String("Panel".into()),
        }))
        .unwrap();
        assert!(game.native_screen_shown());
    }

    /// A frame callback reads the open inventory's slot boxes while `Renderer::render` has the
    /// renderer out, from the framebuffer it left behind (#5864).
    #[test]
    #[ignore = "daily"]
    fn slot_boxes_are_read_during_a_frame() {
        with_panel(0, |game, _| {
            // In the world, past the level load: the inventory is a screen hooks see.
            game.client.remote.as_mut().unwrap().level_load.finish();
            game.client.title.0 = None;
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
                )
                .unwrap(),
            );
            game.client.screen = super::super::Screen::Inventory;
            let menu = game
                .player_container_menu(game.local_player().unwrap())
                .unwrap();
            assert!(game.client.renderer.0.is_none());
            assert_eq!(game.menu_slot_boxes(menu), None, "no window, no boxes");
            game.client
                .render_snapshot
                .set(Some(super::super::RenderSnapshot {
                    framebuffer: [800, 600],
                    prepared_generation: None,
                    staged_prepared_draws: 0,
                }));
            let batch =
                super::super::HudBatch::new([800, 600], game.client.settings.gui_scale_inputs());
            assert!(
                game.screen_hooks_open(),
                "the open screen counts during a frame"
            );
            assert_eq!(game.native_client_menu_name(menu), Some("inventory"));
            assert!(
                game.inventory_menu_slot_boxes(0, "inventory", &batch)
                    .is_some()
            );
            let boxes = game.menu_slot_boxes(menu).unwrap();
            assert_eq!(boxes.len(), 46);
            assert_eq!(
                Some(boxes),
                game.inventory_menu_slot_boxes(0, "inventory", &batch)
            );
        });
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn check_inventory_boxes_follow_menu_indices_and_the_input_layout(
        game: &mut crate::game::ClientGame<'_>,
        opening: &Mutex<Option<(MenuRef, usize)>>,
    ) {
        use sunrise_gameplay_common::game::Side as _;

        game.client.screen = super::super::Screen::Inventory;
        let batch =
            super::super::HudBatch::new([800, 600], game.client.settings.gui_scale_inputs());
        let boxes = game
            .inventory_menu_slot_boxes(0, "inventory", &batch)
            .unwrap();
        assert_eq!(boxes.len(), 46);
        for (index, expected_slot) in [(0, 200), (9, 9), (36, 0), (45, 504)] {
            let [x, y, w, h] = boxes[index].unwrap();
            assert_eq!(
                game.slot_at(&batch, x + w / 2.0, y + h / 2.0),
                Some(expected_slot)
            );
        }
        assert!(game.inventory_menu_slot_boxes(1, PANEL, &batch).is_none());
        game.client.screen = super::super::Screen::Playing;
        open_panel(game);
        let menu = game
            .player_container_menu(game.local_player().unwrap())
            .unwrap();
        assert_eq!(
            *opening.lock().unwrap(),
            Some((menu, 0)),
            "the same live handle and count are available during screen_opening"
        );
        assert!(
            game.inventory_menu_slot_boxes(1, "unsupported", &batch)
                .is_none()
        );
        assert_eq!(
            game.inventory_menu_slot_boxes(1, PANEL, &batch),
            Some(vec![])
        );
        assert!(
            crate::game::ClientState::as_server_host_mut(game).is_none(),
            "the client cannot write the live menu's data slots"
        );
    }

    #[cfg(test)]
    pub(in crate::game::client_side) fn creative_tab(
        game: &mut crate::game::ClientGame<'_>,
        pick: impl Fn(&sunrise_client::creative::Tab) -> bool,
    ) {
        let tab = game
            .client
            .creative_screen
            .0
            .tabs
            .iter()
            .position(pick)
            .unwrap();
        game.client.creative_screen.0.select(tab);
    }

    #[cfg(test)]
    /// A hidden window's offscreen renderer over a joined level with the player's body in it,
    /// so the public screen reads have a window to lay the screen out in. The event loop
    /// returned holds the window.
    pub(in crate::game::client_side) fn offscreen_window(
        game: &mut crate::game::ClientGame<'_>,
    ) -> winit::event_loop::EventLoop<()> {
        let (event_loop, window) = crate::game::native_entity_refs::tests::test_window([320, 240]);
        game.options.offscreen = true;
        {
            let renderer = game.new_renderer(window).unwrap();
            game.set_renderer(renderer);
        }
        let remote = game.client.remote.as_mut().unwrap();
        remote.level_load.finish();
        let mut body = sunrise_sim::motion::PlayerMotion::new(
            game.common.motion,
            sunrise_core::geometry::Vec3::ZERO,
        )
        .unwrap();
        body.entity_id = remote.entity_id;
        game.player = Some(body);
        event_loop
    }

    #[cfg(test)]
    pub(in crate::game::client_side) fn window_batch(
        game: &crate::game::ClientGame<'_>,
    ) -> super::super::HudBatch {
        let config = &game.client.renderer.0.as_ref().unwrap().config;
        super::super::HudBatch::new(
            [config.width, config.height],
            game.client.settings.gui_scale_inputs(),
        )
    }

    #[cfg(test)]
    /// The creative screen's boxes, read through `MenuRef::slot_boxes`, are `InventoryMenu`'s
    /// 46 slots where `CreativeModeInventoryScreen.selectTab` puts them, and each box's centre
    /// hovers its slot.
    #[test]
    #[ignore = "daily"]
    fn creative_boxes_follow_the_selected_tab() {
        sunrise_gameplay_common::game::in_own_process(
            "game::client_side::native_menu_refs::tests::creative_boxes_follow_the_selected_tab",
            || {
                with_panel(0, |game, _| {
                    let _window = offscreen_window(game);
                    game.creative = true;
                    game.client.screen = super::super::Screen::Creative;
                    game.refresh_creative_tabs();
                    let batch = window_batch(game);
                    let (ox, oy) = crate::game::ClientGame::creative_origin(&batch);
                    // Menu index, game slot, position on the inventory tab, shown on item tabs.
                    let expected = |index: usize| -> Option<(usize, (f32, f32), bool)> {
                        let row = |i: usize| (9.0 + (i % 9) as f32 * 18.0, (i / 9) as f32 * 18.0);
                        match index {
                            5..=8 => {
                                let i = index - 5;
                                let at =
                                    (54.0 + (i / 2) as f32 * 54.0, 6.0 + (i % 2) as f32 * 27.0);
                                Some((500 + i, at, false))
                            }
                            9..=35 => {
                                let (x, y) = row(index - 9);
                                Some((index, (x, 54.0 + y), false))
                            }
                            36..=44 => Some((index - 36, (row(index - 36).0, 112.0), true)),
                            45 => Some((504, (35.0, 20.0), false)),
                            _ => None,
                        }
                    };
                    for tab in ["inventory", "items", "search"] {
                        creative_tab(game, |t| match tab {
                            "inventory" => t.inventory,
                            "items" => !t.inventory && !t.search && !t.items.is_empty(),
                            _ => t.search,
                        });
                        let host: &dyn mod_api::ClientHost = &*game;
                        let menu = host.local_player().unwrap().container_menu(host).unwrap();
                        let boxes = menu.slot_boxes(host).unwrap();
                        assert_eq!(boxes.len(), 46, "{tab}");
                        for (index, found) in boxes.into_iter().enumerate() {
                            let want = expected(index)
                                .filter(|&(_, _, item_tab)| tab == "inventory" || item_tab);
                            assert_eq!(
                                found,
                                want.map(|(_, (x, y), _)| [ox + x, oy + y, 16.0, 16.0]),
                                "{tab} slot {index}"
                            );
                            if let Some((slot, (x, y), _)) = want {
                                assert_eq!(
                                    game.slot_at(&batch, ox + x + 8.0, oy + y + 8.0),
                                    Some(slot),
                                    "{tab} slot {index}"
                                );
                            }
                        }
                    }
                })
            },
        );
    }

    #[cfg(test)]
    /// `screen_hovered_stack` gives a mod the hovered slot's whole stack, components included,
    /// on the inventory screen and on both kinds of creative tab, where a catalog slot shows
    /// the scrolled list; nothing over an empty slot, the trash slot or the world.
    #[test]
    #[ignore = "daily"]
    fn the_hovered_stack_keeps_its_components() {
        sunrise_gameplay_common::game::in_own_process(
            "game::client_side::native_menu_refs::tests::the_hovered_stack_keeps_its_components",
            || {
                use mod_api::{Tag, prelude::ScreensClientHost};
                use sunrise_client::creative::Creative;
                use sunrise_core::nbt::JavaString;
                use sunrise_gameplay_common::game::{
                    api_values::TryIntoEngine as _, tables::GameItems as _,
                };

                fn component<'s>(stack: &'s mod_api::ItemStack, id: &str) -> Option<&'s Tag> {
                    stack.components()?.as_nbt().get(&JavaString::from(id))
                }
                let key = |id| mod_api::__private::item_key(id).unwrap();
                let is_sword = |stack: Option<mod_api::ItemStack>| {
                    let stack = stack.expect("the sword");
                    assert_eq!(stack.item(), key("minecraft:diamond_sword"));
                    assert_eq!(stack.count(), 1);
                    assert_eq!(stack.damage_component(), Some(5));
                    assert_eq!(
                        component(&stack, "minecraft:enchantments"),
                        Some(&Tag::compound([("minecraft:sharpness", Tag::Int(3))]))
                    );
                    assert_eq!(
                        component(&stack, "minecraft:custom_name"),
                        Some(&Tag::String("Blade".into()))
                    );
                };
                let is_potion = |stack: Option<mod_api::ItemStack>| {
                    let stack = stack.expect("the potion");
                    assert_eq!(stack.item(), key("minecraft:potion"));
                    assert_eq!(stack.count(), 1);
                    assert_eq!(
                        component(&stack, "minecraft:potion_contents"),
                        Some(&Tag::compound([(
                            "potion",
                            Tag::String("minecraft:strong_healing".into())
                        )]))
                    );
                };
                let is_stone = |stack: Option<mod_api::ItemStack>| {
                    let stack = stack.expect("the stone");
                    assert_eq!(stack.item(), key("minecraft:stone"));
                    assert_eq!(stack.count(), 17);
                    assert!(stack.components().is_none());
                };
                with_panel(0, |game, _| {
                    let _window = offscreen_window(game);
                    let batch = window_batch(game);
                    let hover = |game: &mut crate::game::ClientGame<'_>, slot: usize| {
                        let (ox, oy) = game.inventory_origin(&batch);
                        let (x, y) = game.slot_position(slot);
                        game.client.cursor = (ox + x + 8.0, oy + y + 8.0);
                        ScreensClientHost::screen_hovered_stack(&*game)
                    };
                    let sword = game
                        .item_from_snbt(r#"{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5,"minecraft:enchantments":{"minecraft:sharpness":3},"minecraft:custom_name":"Blade"}}"#)
                        .unwrap()
                        .try_into_engine(game.items())
                        .unwrap();
                    let potion = game
                        .item_from_snbt(r#"{id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:strong_healing"}}}"#)
                        .unwrap()
                        .try_into_engine(game.items())
                        .unwrap();
                    game.inventory[0] = Some(sword.clone());
                    game.inventory[9] = Some(potion.clone());
                    game.inventory[10] = Some(ItemStack::new("minecraft:stone", 17));
                    game.inventory[11] = None;
                    game.armor[0] = Some(sword.clone());

                    game.client.screen = super::super::Screen::Inventory;
                    is_potion(hover(game, 9));
                    is_stone(hover(game, 10));
                    assert_eq!(hover(game, 11), None);
                    game.client.screen = super::super::Screen::Playing;
                    assert_eq!(hover(game, 9), None);

                    game.creative = true;
                    game.client.screen = super::super::Screen::Creative;
                    game.refresh_creative_tabs();
                    creative_tab(game, |t| t.inventory);
                    is_potion(hover(game, 9));
                    is_sword(hover(game, 500));
                    assert_eq!(hover(game, 11), None);
                    let (ox, oy) = game.inventory_origin(&batch);
                    let (tx, ty) = Creative::TRASH_POSITION;
                    game.client.cursor = (ox + tx + 8.0, oy + ty + 8.0);
                    assert_eq!(ScreensClientHost::screen_hovered_stack(&*game), None);

                    // Seven rows: scrolled to the bottom, the first shown row is the third.
                    creative_tab(game, |t| !t.inventory && !t.search && !t.items.is_empty());
                    let tab = game.client.creative_screen.0.selected_index();
                    let mut items = vec![ItemStack::new("minecraft:stone", 17); 63];
                    items[0] = sword.clone();
                    items[1].count = 0;
                    items[18] = potion;
                    game.client.creative_screen.0.tabs[tab].items = items;
                    game.client.creative_screen.0.forget_items();
                    is_sword(hover(game, Creative::DISPLAY));
                    assert_eq!(hover(game, Creative::DISPLAY + 1), None);
                    is_stone(hover(game, Creative::DISPLAY + 2));
                    is_sword(hover(game, 0));
                    game.client.creative_screen.0.scroll = 1.0;
                    is_potion(hover(game, Creative::DISPLAY));

                    game.client.creative_screen.0 = Creative::from_json(&serde_json::json!({
                        "schema": 4, "item_features": {}, "potion_features": {}, "tabs": [{
                            "id": "minecraft:search", "type": "SEARCH", "title": "Search Items",
                            "build": [], "search_metadata": [{
                                "stack": r#"{id:"minecraft:stone",count:17}"#,
                                "ungrouped": true
                            }]
                        }]
                    }))
                    .unwrap();
                    is_stone(hover(game, Creative::DISPLAY));
                    is_sword(hover(game, 0));
                });
            },
        );
    }

    #[cfg(test)]
    /// The creative catalog follows tab, scroll and cursor state through the client host.
    #[test]
    #[ignore = "daily"]
    fn creative_catalog_shows_the_scrolled_list() {
        sunrise_gameplay_common::game::in_own_process(
            "game::client_side::native_menu_refs::tests::creative_catalog_shows_the_scrolled_list",
            || {
                use mod_api::{Tag, prelude::ScreensClientHost};
                use sunrise_client::creative::Creative;
                use sunrise_core::nbt::JavaString;
                use sunrise_gameplay_common::game::{
                    api_values::TryIntoEngine as _, tables::GameItems as _,
                };

                let key = |id| mod_api::__private::item_key(id).unwrap();
                let named = |stack: &mod_api::ItemStack| {
                    stack
                        .components()
                        .and_then(|c| c.as_nbt().get(&JavaString::from("minecraft:custom_name")))
                        .cloned()
                };
                with_panel(0, |game, _| {
                    game.creative = true;
                    game.client.screen = super::super::Screen::Creative;
                    game.refresh_creative_tabs();
                    creative_tab(game, |t| !t.inventory && !t.search && !t.items.is_empty());
                    let menu = game
                        .player_container_menu(game.local_player().unwrap())
                        .unwrap();
                    let host: &dyn mod_api::ClientHost = &*game;
                    assert_eq!(menu.creative_catalog(host), None, "no window");

                    let _window = offscreen_window(game);
                    let batch = window_batch(game);
                    // A 320x240 window at GUI scale 1: leftPos (320 - 195) / 2, topPos (240 - 136) / 2.
                    assert_eq!((batch.width, batch.height), (320.0, 240.0));
                    let cell_box = |cell: usize| {
                        let (column, row) = ((cell % 9) as f32, (cell / 9) as f32);
                        [
                            62.0 + 9.0 + column * 18.0,
                            52.0 + 18.0 + row * 18.0,
                            16.0,
                            16.0,
                        ]
                    };
                    let sword = game
                        .item_from_snbt(r#"{id:"minecraft:diamond_sword",count:1,components:{"minecraft:custom_name":"Blade"}}"#)
                        .unwrap()
                        .try_into_engine(game.items())
                        .unwrap();
                    // The bottom scroll position starts on row three, with three empty cells at the end.
                    let tab = game.client.creative_screen.0.selected_index();
                    let mut items = vec![ItemStack::new("minecraft:stone", 17); 60];
                    items[0] = sword;
                    items[1].count = 0;
                    items[18] = ItemStack::new("minecraft:dirt", 3);
                    game.client.creative_screen.0.tabs[tab].items = items;
                    game.client.creative_screen.0.forget_items();

                    // (item, count, custom name) of each cell, from the fixture above.
                    type Want = Option<(&'static str, u32, Option<Tag>)>;
                    let check = |game: &mut ClientGame<'_>, want: &dyn Fn(usize) -> Want| {
                        let host: &dyn mod_api::ClientHost = &*game;
                        let menu = host.local_player().unwrap().container_menu(host).unwrap();
                        let cells = menu.creative_catalog(host).unwrap();
                        assert_eq!(cells.len(), 45);
                        for (cell, (rect, stack)) in cells.into_iter().enumerate() {
                            assert_eq!(rect, cell_box(cell), "cell {cell}");
                            let found = stack.as_ref().map(|s| (s.item(), s.count(), named(s)));
                            let want = want(cell).map(|(id, count, name)| (key(id), count, name));
                            assert_eq!(found, want, "cell {cell}");
                            game.client.cursor = (rect[0] + 8.0, rect[1] + 8.0);
                            assert_eq!(
                                ScreensClientHost::screen_hovered_stack(&*game),
                                stack,
                                "cell {cell}"
                            );
                        }
                    };
                    check(game, &|cell| match cell {
                        0 => Some((
                            "minecraft:diamond_sword",
                            1,
                            Some(Tag::String("Blade".into())),
                        )),
                        1 => None,
                        18 => Some(("minecraft:dirt", 3, None)),
                        _ => Some(("minecraft:stone", 17, None)),
                    });
                    game.client.creative_screen.0.scroll = 1.0;
                    check(game, &|cell| match cell {
                        0 => Some(("minecraft:dirt", 3, None)),
                        42.. => None,
                        _ => Some(("minecraft:stone", 17, None)),
                    });

                    let generation = game.client.remote.as_ref().unwrap().menu.generation;
                    let stale =
                        mod_api::__private::game::menu_ref(menu.fields().player, 0, generation + 1);
                    let host: &dyn mod_api::ClientHost = &*game;
                    assert_eq!(stale.creative_catalog(host), None, "stale handle");

                    creative_tab(game, |t| t.inventory);
                    let host: &dyn mod_api::ClientHost = &*game;
                    assert_eq!(
                        menu.creative_catalog(host),
                        Some(Vec::new()),
                        "inventory tab"
                    );

                    game.client.screen = super::super::Screen::Inventory;
                    let host: &dyn mod_api::ClientHost = &*game;
                    assert_eq!(menu.creative_catalog(host), None, "inventory screen");

                    game.client.screen = super::super::Screen::Creative;
                    game.client.creative_screen.0 = Creative::from_json(&serde_json::json!({
                        "schema": 4, "item_features": {}, "potion_features": {}, "tabs": [{
                            "id": "minecraft:search", "type": "SEARCH", "title": "Search Items",
                            "build": [], "search_metadata": [{
                                "stack": r#"{id:"minecraft:stone",count:17}"#,
                                "ungrouped": true
                            }]
                        }]
                    }))
                    .unwrap();
                    check(game, &|cell| {
                        (cell == 0).then_some(("minecraft:stone", 17, None))
                    });
                });
            },
        );
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn number_key_swaps_predict_with_the_undisplayed_hotbar() {
        use sunrise_gameplay_common::game::inventory_slots::MOD_MENU_SLOT_BASE;

        with_panel(1, |game, _| {
            open_panel(game);
            let slot = MOD_MENU_SLOT_BASE;
            assert_eq!(game.slots(), [slot]);
            let held = ItemStack::new("minecraft:stone", 5);
            let stored = ItemStack::new("minecraft:dirt", 2);
            game.inventory[2] = Some(held.clone());
            *game.slot_mut(slot) = Some(stored.clone());
            assert!(game.menu_ops(&[(Some(slot), 2, Input::Swap)]));
            assert_eq!(game.inventory[2], Some(stored));
            assert_eq!(*game.slot_mut(slot), Some(held));
            assert_eq!(game.slots(), [slot]);
        });
    }

    #[cfg(test)]
    #[test]
    fn client_click_actions_encode_vanilla_ops() {
        let actions = [
            MenuClick::Pickup {
                slot: Some(36),
                button: MouseButton::Left,
            },
            MenuClick::Pickup {
                slot: None,
                button: MouseButton::Right,
            },
            MenuClick::QuickMove { slot: 9 },
            MenuClick::Swap {
                slot: 9,
                with: SwapWith::Hotbar(8),
            },
            MenuClick::Swap {
                slot: 9,
                with: SwapWith::Offhand,
            },
            MenuClick::Clone { slot: 9 },
            MenuClick::Throw {
                slot: 9,
                whole_stack: false,
            },
            MenuClick::Throw {
                slot: 9,
                whole_stack: true,
            },
            MenuClick::PickupAll { slot: 9 },
        ];
        assert_eq!(
            client_click_ops("inventory", &actions, true),
            Some(vec![
                (Some(0), 0, Input::Pickup),
                (None, 1, Input::Pickup),
                (Some(9), 0, Input::QuickMove),
                (Some(9), 8, Input::Swap),
                (Some(9), 40, Input::Swap),
                (Some(9), 2, Input::Clone),
                (Some(9), 0, Input::Throw),
                (Some(9), 1, Input::Throw),
                (Some(9), 0, Input::PickupAll),
            ])
        );
        for (kind, mode, creative) in [
            (DragKind::Split, 0, false),
            (DragKind::One, 4, false),
            (DragKind::Clone, 8, true),
        ] {
            assert_eq!(
                client_click_ops(
                    "inventory",
                    &[
                        MenuClick::Drag {
                            kind,
                            slots: vec![36, 9]
                        },
                        MenuClick::QuickMove { slot: 9 },
                    ],
                    creative
                ),
                Some(vec![
                    (None, mode, Input::QuickCraft),
                    (Some(0), mode + 1, Input::QuickCraft),
                    (Some(9), mode + 1, Input::QuickCraft),
                    (None, mode + 2, Input::QuickCraft),
                    (Some(9), 0, Input::QuickMove),
                ])
            );
        }
    }

    #[cfg(test)]
    #[test]
    fn client_click_actions_refuse_invalid_batches_before_prediction() {
        assert!(client_click_ops("inventory", &[], false).is_none());
        for invalid in [
            MenuClick::QuickMove { slot: 46 },
            MenuClick::QuickMove { slot: usize::MAX },
            MenuClick::Swap {
                slot: 9,
                with: SwapWith::Hotbar(9),
            },
            MenuClick::Swap {
                slot: 9,
                with: SwapWith::Hotbar(u8::MAX),
            },
            MenuClick::Clone { slot: 9 },
            MenuClick::Drag {
                kind: DragKind::Clone,
                slots: vec![9],
            },
            MenuClick::Drag {
                kind: DragKind::Split,
                slots: vec![9, usize::MAX],
            },
        ] {
            assert!(
                client_click_ops(
                    "inventory",
                    &[MenuClick::QuickMove { slot: 9 }, invalid,],
                    false
                )
                .is_none()
            );
        }
        assert!(
            client_click_ops("unsupported", &[MenuClick::QuickMove { slot: 0 }], true).is_none()
        );
    }

    pub fn guest() -> Guest {
        let (tx, _) = std::sync::mpsc::channel();
        let mut g = Guest::new(
            Joined {
                conn: 1,
                uuid: 123,
                name: "guest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            1,
            [0.0; 3],
            10,
        );
        g.placed = true;
        g
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_give_stack::NativeGiveStackGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
