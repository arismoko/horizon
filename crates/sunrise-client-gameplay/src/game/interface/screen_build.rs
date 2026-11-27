//! A screen's built state (`Screen.init`): made when the screen opens, remade
//! when the window or the GUI scale resizes it, dropped when it closes.
//!
//! Step 1 of #903 put the seam in: every screen built `BuildLayout::Empty`,
//! with the window's rectangle and nothing a screen drew or hit-tested.
//! Step 2 moves the container screens onto it (`BuildLayout::Container`): the
//! panel, the book and effect panels beside it, the creative tab rows and
//! every slot's hover box are made once here, and the drawing, the hit tests
//! and `screen_exclusion` read them instead of rebuilding them per call.
//! Step 3 adds the pause menu. Step 4 takes chat's and the command block
//! screen's resize out of their draws: their text boxes are re-made at the
//! build's size here (`ChatScreen.resize`,
//! `AbstractCommandBlockEditScreen.resize`), and their buttons are laid out
//! once per build.

use mod_api::ScreenView;
use sunrise_client::hud::HudBatch;
use sunrise_client::inventory_effects::{EffectsLayout, EffectsSpace};
use sunrise_client::window::GuiSize;

use super::Screen;

/// What the native hooks see of the screen showing (`observe_screen`), kept
/// beside its build so one `ScreenState` holds both.
#[derive(Clone)]
pub struct ObservedScreen {
    pub id: &'static str,
    /// Not a `Screen` value: the title's own screen or an overlay drawn over
    /// gameplay (sign editor, book, dialog, prompt). `set_screen` never sees
    /// it open or close.
    pub overlay: bool,
    pub mod_screen: Option<String>,
    pub menu: Option<String>,
    pub menu_generation: Option<u64>,
    /// Dialog, warning and waiting screens share one public id, not one receiver.
    pub dialog_key: Option<(bool, bool, Option<u64>)>,
    pub title: Option<std::sync::Arc<mod_api::Text>>,
    pub rect: [f32; 4],
    /// Actual GUI-scaled screen dimensions (`width`, `height`), used to detect
    /// viewport/GUI-scale resizes independently of a panel's `rect`.
    pub screen_size: [f32; 2],
    pub slots: usize,
    pub mouse: [f32; 2],
    pub partial_tick: f32,
    pub regions: Vec<mod_api::ScreenRegion>,
}

impl ObservedScreen {
    pub fn view(&self) -> ScreenView<'_> {
        ScreenView::from(mod_api::__private::built::ScreenView {
            id: self.id,
            mod_screen: self.mod_screen.as_deref(),
            menu: self.menu.as_deref(),
            title: self.title.as_deref(),
            rect: self.rect,
            slots: self.slots,
            mouse: self.mouse,
            partial_tick: self.partial_tick,
            regions: &self.regions,
        })
    }
}

/// What a screen's drawing and its input read: its GUI-scaled size, the panel
/// it draws in, and its own widgets as rectangles.
///
/// Built once per open and per resize, never per frame. A screen bumps its own
/// revision when one of its layout inputs moves (Options' page or scroll, a
/// book opening), and the build is remade then.
pub struct ScreenBuild {
    /// Which screen this was built for.
    pub(in crate::game) screen: Screen,
    identity: Option<&'static str>,
    /// The GUI-scaled size (`getGuiScaledWidth`/`getGuiScaledHeight`), in GUI pixels.
    pub(in crate::game) size: (f32, f32),
    /// The GUI scale (`narration`'s framebuffer space).
    pub(in crate::game) scale: f32,
    /// The panel the screen draws (`NativeScreenView.rect`), else the window.
    pub(in crate::game) rect: [f32; 4],
    /// The panel's `leftPos`/`topPos`.
    pub(in crate::game) origin: (f32, f32),
    /// The screen's own widgets, as rectangles.
    pub layout: BuildLayout,
    /// Panels drawn outside `rect`.
    pub(in crate::game) exclusion: Vec<[f32; 4]>,
}

/// A container screen's layout: the panel it draws in, the panels beside it,
/// and every slot's hover box. Read by its drawing, `slot_at`,
/// `clicked_outside` and `screen_exclusion`; never rebuilt per call.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerLayout {
    pub(in crate::game) slot_inputs: super::container_layout::SlotInputs,
    /// `leftPos`, `topPos`, `imageWidth`, `imageHeight` in GUI pixels.
    pub(in crate::game) image: [f32; 4],
    /// `getXOrigin`/`getYOrigin`: the player inventory's top-left corner.
    pub origin: (f32, f32),
    /// The open recipe book's panel.
    pub book: Option<[f32; 4]>,
    /// The effect panels' box beside the window.
    pub effects: Option<[f32; 4]>,
    pub(in crate::game) shows_effects: bool,
    pub(in crate::game) effects_revision: u64,
    /// The `RenderInventoryMobEffects` answer the panel box was laid out with.
    pub(in crate::game) effects_layout: EffectsLayout,
    /// The creative tab rows above and below the window.
    pub tabs: Option<[[f32; 4]; 2]>,
    /// Every slot's `isHovering` box (the 16x16 slot grown by one pixel on
    /// every side) and its slot id, in `slots()` order.
    pub slots: Vec<(usize, [f32; 4])>,
}

impl ContainerLayout {
    /// The areas the screen draws outside `image`, in the order the screen
    /// draws them: the creative tab rows then the effect panels, or the
    /// recipe book then the effect panels.
    pub fn exclusion(&self) -> Vec<[f32; 4]> {
        let mut areas = Vec::new();
        match self.tabs {
            Some([above, below]) => {
                areas.push(above);
                areas.push(below);
            }
            None => areas.extend(self.book),
        }
        areas.extend(self.effects);
        areas
    }
}

/// The pause menu's widgets: its buttons and its icon row, as `PauseScreen.init`
/// lays them out. Read by the drawing, `pause_click` and `pause_key`; never
/// rebuilt per call.
#[derive(Clone, Debug, PartialEq)]
pub struct PauseLayout {
    /// The `PauseScreen.init` inputs this layout was made from: a server's
    /// links or a custom-additions dialog appearing, or a page registering a
    /// row, is a new layout even at the same size.
    pub(in crate::game) inputs: PauseInputs,
    /// `(x, y, width, label key, action id, active)`, in `PauseScreen`'s order.
    pub buttons: Vec<(f32, f32, f32, &'static str, u8, bool)>,
    /// `(x, y, sprite)`: the report bugs, feedback, friends and player reporting
    /// icons, in that order.
    pub(in crate::game) icons: [(f32, f32, &'static str); 4],
}

/// What `PauseScreen.init` reads and the paused screen can see change while it
/// is open: the connection (the disconnect label), a server's links, the
/// custom-additions dialog and the extra page rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PauseInputs {
    pub(in crate::game) remote: bool,
    pub(in crate::game) links: bool,
    pub(in crate::game) additions: bool,
    pub(in crate::game) rows: Vec<(&'static str, u8)>,
}

/// `InBedChatScreen`'s widget beside the chat box: the Leave Bed button
/// (`width / 2 - 100`, `height - 40`, 200x20). Laid out for every chat build
/// and drawn and hit-tested only while the player sleeps, so a sleeper's
/// chat opening on a plain chat build needs no rebuild.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChatLayout {
    pub leave_bed: (i32, i32, i32, i32),
}

/// `AbstractCommandBlockEditScreen.init`'s buttons: the output toggle, the
/// command block's three mode switches (none for a minecart) and Done/Cancel.
/// Read by the drawing and `command_block_click`.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandBlockLayout {
    /// A minecart's screen (`getPreviousY` 150, no mode switches): an input,
    /// so another kind of edit screen opening at the same size rebuilds.
    pub(in crate::game) cart: bool,
    /// `(button, x, y, width)`, all 20 high, in `init`'s order.
    pub(in crate::game) buttons: Vec<(super::command_blocks::Button, f32, f32, f32)>,
}

/// A screen's own widgets, as rectangles.
///
/// Step 2 fills `Container` for the inventory, the creative inventory and
/// every container or station window, step 3 fills `Pause` and step 4 `Chat`
/// and `CommandBlock`. (Options keeps its own built layout, keyed by its
/// revision, in `OptionsUi`.)
#[derive(Clone, Debug, Default, PartialEq)]
pub enum BuildLayout {
    /// Not migrated yet: the build has the window's rectangle only.
    #[default]
    Empty,
    /// A container screen: its panel, the panels beside it, its slot boxes.
    Container(ContainerLayout),
    /// The pause menu: its buttons and icon row.
    Pause(PauseLayout),
    /// The chat screen: the in-bed Leave Bed button.
    Chat(ChatLayout),
    /// The command block edit screen: its buttons.
    CommandBlock(CommandBlockLayout),
}

/// A layout a screen just made for its build. One arm per migrated screen;
/// `sync` replaces the built one when it differs.
pub enum ScreenLayout {
    Container(ContainerLayout),
    Pause(PauseLayout),
    Chat(ChatLayout),
    CommandBlock(CommandBlockLayout),
}

impl ScreenLayout {
    /// A layout that is only widgets: the window is its rectangle and it
    /// draws nothing outside it.
    fn widgets(self) -> BuildLayout {
        match self {
            ScreenLayout::Container(layout) => BuildLayout::Container(layout),
            ScreenLayout::Pause(layout) => BuildLayout::Pause(layout),
            ScreenLayout::Chat(layout) => BuildLayout::Chat(layout),
            ScreenLayout::CommandBlock(layout) => BuildLayout::CommandBlock(layout),
        }
    }
}

impl ScreenBuild {
    /// The build for `screen` at `gui`. A screen without a layout knows no
    /// panel and no widgets, so its rectangle is the window and its origin is
    /// zero.
    fn new(
        screen: Screen,
        identity: Option<&'static str>,
        gui: GuiSize,
        layout: Option<ScreenLayout>,
    ) -> Self {
        let size = (gui.size[0] as f32, gui.size[1] as f32);
        let (rect, origin, layout, exclusion) = match layout {
            Some(ScreenLayout::Container(layout)) => {
                let exclusion = layout.exclusion();
                (
                    layout.image,
                    layout.origin,
                    BuildLayout::Container(layout),
                    exclusion,
                )
            }
            Some(layout) => (
                [0.0, 0.0, size.0, size.1],
                (0.0, 0.0),
                layout.widgets(),
                Vec::new(),
            ),
            None => (
                [0.0, 0.0, size.0, size.1],
                (0.0, 0.0),
                BuildLayout::Empty,
                Vec::new(),
            ),
        };
        Self {
            screen,
            identity,
            size,
            scale: gui.scale as f32,
            rect,
            origin,
            layout,
            exclusion,
        }
    }

    /// The build's container layout, when it has one at this size.
    pub(in crate::game::client_side) fn container(
        &self,
        width: f32,
        height: f32,
    ) -> Option<&ContainerLayout> {
        if self.size != (width, height) {
            return None;
        }
        match &self.layout {
            BuildLayout::Container(layout) => Some(layout),
            _ => None,
        }
    }

    /// The build's pause layout, when it has one at this size.
    pub(in crate::game::client_side) fn pause(
        &self,
        width: f32,
        height: f32,
    ) -> Option<&PauseLayout> {
        if self.size != (width, height) {
            return None;
        }
        match &self.layout {
            BuildLayout::Pause(layout) => Some(layout),
            _ => None,
        }
    }

    /// The build's chat layout, when it has one at this size.
    pub(in crate::game::client_side) fn chat(
        &self,
        width: f32,
        height: f32,
    ) -> Option<&ChatLayout> {
        match &self.layout {
            BuildLayout::Chat(layout) if self.size == (width, height) => Some(layout),
            _ => None,
        }
    }

    /// The build's command block layout, when it has one at this size.
    pub(in crate::game::client_side) fn command_block(
        &self,
        width: f32,
        height: f32,
    ) -> Option<&CommandBlockLayout> {
        match &self.layout {
            BuildLayout::CommandBlock(layout) if self.size == (width, height) => Some(layout),
            _ => None,
        }
    }
}

/// The last handled mouse press on a screen, for vanilla's 250ms double-click
/// detection (`MouseHandler.LastClick`, `MouseHandler.java:101-106`).
#[derive(Clone, Copy, Debug)]
pub struct LastScreenClick {
    time: u64,
    title_mode: Option<u64>,
    pub(in crate::game) screen_instance: u64,
    pub(in crate::game) button: i32,
}

/// What the game knows about the screen showing: the snapshot the native hooks
/// see (`ObservedScreen`) and the build the screen's drawing will read.
#[derive(Default)]
pub struct ScreenState {
    pub(in crate::game) observed: Option<ObservedScreen>,
    pub build: ScreenBuildState,
    /// The last draw's regions and the screen they are of (`ScreenView::regions`).
    regions: (Option<Screen>, Vec<mod_api::ScreenRegion>),
    /// The box the recipe book's tab (and category arrow) sprites made as this
    /// frame painted them (`recipe_book_screen::sprite_bounds`); None when the
    /// book drew none.
    pub(in crate::game) drawn_book_tabs: std::cell::Cell<Option<[f32; 4]>>,
    pub widgets: super::screen_widgets::MountedScreenWidgets,
    /// A mod's context menu over this screen (`ClientHost::open_context_menu`).
    #[cfg(feature = "client")]
    pub(in crate::game) context_menu: Option<super::context_menu::ContextMenu>,
    /// Press-to-release ownership for mouse, keyboard and controller input consumed by a context menu.
    #[cfg(feature = "client")]
    pub(super) context_menu_input: super::context_menu::ContextMenuInput,
    /// Monotonic identity of the currently open screen instance. Bumped on
    /// every screen open, close, or replacement, and preserved across
    /// same-instance resize/rebuild (`Screen.resize`/`rebuildWidgets`).
    pub screen_instance: u64,
    /// Identity of the text field that last owned IME composition, so switching
    /// directly between two fields clears the old field's preedit and area.
    pub(in crate::game) ime_focus: Option<u64>,
    /// Last handled screen mouse press for 250ms double-click detection.
    pub last_click: Option<LastScreenClick>,
    /// A mod's `ScreenOpening::Vanilla` answer is opening its screen: that
    /// change fires no screen hook again.
    pub(in crate::game) replacing: bool,
    /// The last change the screen hooks decided went another way than asked
    /// (a mod's `Cancel`, `ModScreen` or `Vanilla`): a caller's grab after it
    /// leaves the screen that shows its pointer. Never set with no listener.
    pub(in crate::game) answered: std::cell::Cell<bool>,
}

impl ScreenState {
    /// Advance the receiving screen's identity. Rebuilding children is not a replacement.
    pub fn bump_instance(&mut self) -> u64 {
        self.screen_instance = self.screen_instance.wrapping_add(1);
        self.last_click = None;
        #[cfg(feature = "client")]
        if self.context_menu.is_some() {
            self.context_menu = None;
            self.context_menu_input.clear_unowned_keys_if_idle();
        }
        self.screen_instance
    }

    /// Whether a press at `now` on `screen_instance` with `button` is a vanilla
    /// double-click (within 250ms on the same screen instance and button,
    /// `MouseHandler.java:101-106`).
    pub fn is_double_click(
        &self,
        now: u64,
        screen_instance: u64,
        title_mode: Option<u64>,
        button: i32,
    ) -> bool {
        self.last_click.is_some_and(|last| {
            now.checked_sub(last.time)
                .is_some_and(|elapsed| elapsed < 250)
                && last.title_mode == title_mode
                && last.screen_instance == screen_instance
                && last.button == button
        })
    }

    /// Record a handled mouse press for subsequent double-click detection.
    pub fn record_click(
        &mut self,
        now: u64,
        screen_instance: u64,
        title_mode: Option<u64>,
        button: i32,
    ) {
        self.last_click = Some(LastScreenClick {
            time: now,
            title_mode,
            screen_instance,
            button,
        });
    }
}

/// One frame's `RenderInventoryMobEffects` answer and what it was taken for.
///
/// The display's change builds twice in one frame (`apply_ui_settings` then
/// `observe_screen`); the hook's contract is one dispatch a frame, so the
/// second build reuses this answer instead of asking again.
#[derive(Clone, Copy, PartialEq)]
struct EffectsMemo {
    frame: u64,
    screen: Screen,
    space: EffectsSpace,
    answer: EffectsLayout,
}

/// Memoize the left and right room so multiple layout queries fire the hook once per frame.
#[derive(Clone, Copy, PartialEq)]
struct RoomMemo {
    frame: u64,
    screen: Screen,
    room: [f32; 2],
}

/// The open screen's build and what it was made from.
#[derive(Default)]
pub struct ScreenBuildState {
    pub current: Option<ScreenBuild>,
    /// The frame's effects answer, while this frame asked for one.
    effects: std::cell::Cell<Option<EffectsMemo>>,
    /// The frame's `screen_room` answer, while this frame asked for one.
    room: std::cell::Cell<Option<RoomMemo>>,
    #[cfg(any(test, feature = "test-support"))]
    pub builds: u32,
}

impl ScreenBuildState {
    /// A fresh overlay instance must build even when its kind is unchanged.
    pub fn reopened(&mut self) {
        self.current = None;
    }

    /// The answer this frame already took for `screen` in `space`, so a
    /// second build of the same frame asks the mods nothing.
    pub(in crate::game::client_side) fn effects_answer(
        &self,
        frame: u64,
        screen: Screen,
        space: EffectsSpace,
    ) -> Option<EffectsLayout> {
        self.effects
            .get()
            .filter(|memo| memo.frame == frame && memo.screen == screen && memo.space == space)
            .map(|memo| memo.answer)
    }

    /// Keeps this frame's answer for a second build in the same frame.
    pub(in crate::game::client_side) fn keep_effects_answer(
        &self,
        frame: u64,
        screen: Screen,
        space: EffectsSpace,
        answer: EffectsLayout,
    ) {
        self.effects.set(Some(EffectsMemo {
            frame,
            screen,
            space,
            answer,
        }));
    }

    /// The room the mods asked this frame for `screen`, if they were asked.
    pub(in crate::game::client_side) fn room_answer(
        &self,
        frame: u64,
        screen: Screen,
    ) -> Option<[f32; 2]> {
        self.room
            .get()
            .filter(|memo| memo.frame == frame && memo.screen == screen)
            .map(|memo| memo.room)
    }

    /// Keeps this frame's room for the layout's other inputs in the same frame.
    pub(in crate::game::client_side) fn keep_room_answer(
        &self,
        frame: u64,
        screen: Screen,
        room: [f32; 2],
    ) {
        self.room.set(Some(RoomMemo {
            frame,
            screen,
            room,
        }));
    }

    /// Whether `sync` would make a new build for this screen, identity and
    /// size: the caller makes the fresh layout only then.
    pub(in crate::game::client_side) fn needs_build(
        &self,
        screen: Screen,
        shown: bool,
        identity: Option<&'static str>,
        gui: GuiSize,
    ) -> bool {
        shown
            && self.current.as_ref().is_none_or(|build| {
                build.screen != screen
                    || build.identity != identity
                    || build.size != (gui.size[0] as f32, gui.size[1] as f32)
                    || build.scale != gui.scale as f32
            })
    }

    /// Reconciles the build with what shows now: a different screen or a
    /// different GUI size makes it, the same one keeps it, and nothing shown
    /// drops it. A call that changes nothing allocates nothing, so the frame
    /// path checks it every frame, and a menu swap on the same screen keeps it.
    ///
    /// `layout` is supplied only when the frame's allocation-free input
    /// comparison found a change. The same screen can keep its panel while its layout
    /// moves under it (a book opens, the effects answer changes, a creative
    /// tab swaps the slots), so a different layout remakes the build in place.
    pub(in crate::game::client_side) fn sync(
        &mut self,
        screen: Screen,
        shown: bool,
        identity: Option<&'static str>,
        gui: GuiSize,
        layout: Option<ScreenLayout>,
    ) {
        if !shown {
            self.current = None;
            return;
        }
        if self.needs_build(screen, shown, identity, gui) {
            self.current = Some(ScreenBuild::new(screen, identity, gui, layout));
            #[cfg(any(test, feature = "test-support"))]
            {
                self.builds += 1;
            }
            return;
        }
        let Some(layout) = layout else {
            return;
        };
        let build = self.current.as_mut().expect("shown");
        let (rect, origin, built, exclusion) = match layout {
            ScreenLayout::Container(layout) => {
                if matches!(&build.layout, BuildLayout::Container(built) if *built == layout) {
                    return;
                }
                let exclusion = layout.exclusion();
                (
                    layout.image,
                    layout.origin,
                    BuildLayout::Container(layout),
                    exclusion,
                )
            }
            layout => {
                let layout = layout.widgets();
                if build.layout == layout {
                    return;
                }
                (build.rect, build.origin, layout, Vec::new())
            }
        };
        build.rect = rect;
        build.origin = origin;
        build.layout = built;
        build.exclusion = exclusion;
        #[cfg(any(test, feature = "test-support"))]
        {
            self.builds += 1;
        }
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `Gui.setScreen`: `KeyMapping.releaseAll` on a successful screen or overlay
    /// opening/replacement transition (never on per-frame observation or resize).
    pub(in crate::game) fn release_mappings_on_screen_open(&mut self) {
        self.client.mod_keys.release_all();
        self.release_screen_mappings();
    }

    /// Framebuffer size and GUI projection, including before the renderer is initialized.
    pub fn gui_frame_now(&self) -> ([u32; 2], GuiSize) {
        let framebuffer = self.client.renderer.0.as_ref().map_or_else(
            || {
                self.client.window.0.as_ref().map_or([1280, 720], |window| {
                    let size = window.inner_size();
                    [size.width, size.height]
                })
            },
            |renderer| [renderer.config.width, renderer.config.height],
        );
        let inputs = self.client.settings.gui_scale_inputs();
        (
            framebuffer,
            GuiSize::new(framebuffer, inputs.requested, inputs.unicode),
        )
    }

    /// `Screen.init`/`Screen.resize` builds the open screen, and drops it when it closes.
    pub fn build_screen(&mut self) {
        self.build_screen_at(self.gui_frame_now().0);
    }

    pub fn build_screen_at(&mut self, framebuffer: [u32; 2]) {
        let shown = self.client.screen != Screen::Playing
            || self.client.title.0.is_some()
            || self.screen_over_playing();
        let inputs = self.client.settings.gui_scale_inputs();
        let gui = GuiSize::new(framebuffer, inputs.requested, inputs.unicode);
        let identity = self
            .client
            .title
            .0
            .as_ref()
            .map(|title| title.mode.id())
            .or_else(|| {
                self.screen_over_playing()
                    .then(|| self.observed_screen_view().map(|view| view.id))
                    .flatten()
            });
        // Validate the assigned mod book before its flags can affect any
        // cached origin, slot rectangles or exclusion areas.
        if self.native_screen_book().is_some() {
            self.ensure_mod_book_displays();
        }
        // The container screens' layout is a build input: the frame's own
        // state, and the mods' effects answer, compared here so the panels and
        // the exclusion never disagree by a frame.
        let container = (shown
            && matches!(self.client.screen, Screen::Inventory | Screen::Creative))
        .then(|| {
            let batch = sunrise_client::hud::HudBatch::new(
                framebuffer,
                self.client.settings.gui_scale_inputs(),
            );
            self.container_layout_inputs(&batch)
        });
        let container = container.and_then(|inputs| {
            let unchanged = self
                .client
                .screen_state
                .0
                .build
                .current
                .as_ref()
                .is_some_and(|build| {
                    build.screen == self.client.screen
                        && build.identity == identity
                        && build.scale == gui.scale as f32
                        && build
                            .container(gui.size[0] as f32, gui.size[1] as f32)
                            .is_some_and(|old| {
                                old.image == inputs.image
                                    && old.origin == inputs.origin
                                    && old.book == inputs.book
                                    && old.shows_effects == inputs.shows_effects
                                    && old.effects_revision == inputs.effects_revision
                                    && old.effects_layout == inputs.effects_layout
                                    && old.tabs == inputs.tabs
                                    && old.slot_inputs == inputs.slot_inputs
                            })
                });
            (!unchanged).then(|| {
                self.build_container_layout(
                    &sunrise_client::hud::HudBatch::new(
                        framebuffer,
                        self.client.settings.gui_scale_inputs(),
                    ),
                    inputs,
                )
            })
        });
        // The pause menu's inputs can change while it is open: a server's
        // links or a custom-additions dialog arriving. Compare them here and
        // make the layout only when they moved.
        let pause = (shown && self.client.screen == Screen::Paused).then(|| {
            let unchanged = self
                .client
                .screen_state
                .0
                .build
                .current
                .as_ref()
                .is_some_and(|build| {
                    build.screen == self.client.screen
                        && build.identity == identity
                        && build.scale == gui.scale as f32
                        && build
                            .pause(gui.size[0] as f32, gui.size[1] as f32)
                            .is_some_and(|old| self.pause_inputs_match(&old.inputs))
                });
            (!unchanged).then(|| self.pause_layout(gui.size[0] as f32, gui.size[1] as f32))
        });
        let pause = pause.flatten();
        let text = if shown {
            self.text_screen_layout(identity, gui)
        } else {
            None
        };
        let layout = container
            .map(ScreenLayout::Container)
            .or_else(|| pause.map(ScreenLayout::Pause))
            .or(text);
        self.client
            .screen_state
            .0
            .build
            .sync(self.client.screen, shown, identity, gui, layout);
    }

    /// What the open container screen draws outside its panel, from the
    /// `layout` that draws it in a window `width` wide: the recipe book's
    /// panel beside the window (not one a narrow window draws over it) and
    /// each creative tab of the page shown, then the effects beside the
    /// window. The book's tabs join them as painted (`publish_screen_regions`).
    fn collect_screen_regions(
        &self,
        screen: Screen,
        layout: &ContainerLayout,
        width: f32,
        out: &mut Vec<mod_api::ScreenRegion>,
    ) {
        use mod_api::{ScreenRegion, ScreenRegionKind};
        if let Some(book) = layout.book
            && !self.book_covers_menu(width)
        {
            out.push(ScreenRegion::new(ScreenRegionKind::RecipeBook, book));
        }
        if screen == Screen::Creative {
            // `CreativeModeInventoryScreen.extractTabButton`: 26 x 32 each.
            let (ox, oy) = (layout.image[0], layout.image[1]);
            let creative = &self.client.creative_screen.0;
            for tab in creative.tabs.iter().filter(|t| creative.on_page(t)) {
                let (x, y) = sunrise_client::creative::Creative::tab_position(tab);
                out.push(ScreenRegion::new(
                    ScreenRegionKind::CreativeTab,
                    [ox + x, oy + y, 26.0, 32.0],
                ));
            }
        }
        if let Some(effects) = layout.effects {
            out.push(ScreenRegion::new(ScreenRegionKind::Effects, effects));
        }
    }

    /// What this frame drew outside the panel becomes what the hooks read
    /// (`ScreenView::regions`): the layout's regions and the book's tabs as
    /// painted, or nothing when `screen_render_pre` cancelled the drawing.
    /// Called once the screen has drawn, before `screen_render_post`; the
    /// input hooks then read it until the next draw.
    pub(in crate::game::client_side) fn publish_screen_regions(
        &mut self,
        batch: &HudBatch,
        region_screen: Option<Screen>,
    ) {
        use mod_api::{ScreenRegion, ScreenRegionKind};
        let (_, mut regions) = std::mem::take(&mut self.client.screen_state.0.regions);
        regions.clear();
        let tabs = self.client.screen_state.0.drawn_book_tabs.take();
        let shown = region_screen.filter(|_| self.screen_view_listened());
        if let Some(screen) = shown
            && let Some(layout) = self.built_container(batch)
        {
            self.collect_screen_regions(screen, layout, batch.width, &mut regions);
            if screen == Screen::Inventory
                && let Some(tabs) = tabs
            {
                let at = regions
                    .iter()
                    .position(|r| r.kind == ScreenRegionKind::RecipeBook)
                    .map_or(0, |n| n + 1);
                regions.insert(
                    at,
                    ScreenRegion::new(ScreenRegionKind::RecipeBookTabs, tabs),
                );
            }
        }
        self.client.screen_state.0.regions = (shown, regions);
    }

    /// The last published draw's regions of `screen` (`ScreenView::regions`).
    /// Empty for another screen and when no screen hook listens.
    pub fn screen_regions(&self, screen: Screen) -> &[mod_api::ScreenRegion] {
        match &self.client.screen_state.0.regions {
            (Some(of), regions) if *of == screen => regions,
            _ => &[],
        }
    }

    /// The build's container layout, when it holds `batch`'s size.
    pub fn built_container(
        &self,
        batch: &sunrise_client::hud::HudBatch,
    ) -> Option<&ContainerLayout> {
        self.client
            .screen_state
            .0
            .build
            .current
            .as_ref()?
            .container(batch.width, batch.height)
    }

    /// The build's pause layout, when it holds `(width, height)`.
    pub(in crate::game::client_side) fn built_pause(
        &self,
        width: f32,
        height: f32,
    ) -> Option<&PauseLayout> {
        self.client
            .screen_state
            .0
            .build
            .current
            .as_ref()?
            .pause(width, height)
    }

    /// The build's chat layout, when it holds `(width, height)`.
    pub fn built_chat(&self, width: f32, height: f32) -> Option<&ChatLayout> {
        self.client
            .screen_state
            .0
            .build
            .current
            .as_ref()?
            .chat(width, height)
    }

    /// The build's command block layout, when it holds `(width, height)`.
    pub(in crate::game::client_side) fn built_command_block(
        &self,
        width: f32,
        height: f32,
    ) -> Option<&CommandBlockLayout> {
        self.client
            .screen_state
            .0
            .build
            .current
            .as_ref()?
            .command_block(width, height)
    }

    /// Re-makes the shown chat or command box at `(w, h)` when it was made at
    /// another size; at its own size it compares and changes nothing.
    fn resize_text_screens(&mut self, w: f32, h: f32) {
        let m = sunrise_client::screen_draw::Measure(&self.client.gui.atlas.font);
        let c = sunrise_client::chat::Commands {
            dispatcher: super::chat::chat_tree(&self.client.remote),
            registries: &self.common.ui.registries,
        };
        match self.client.screen {
            Screen::Chat => {
                if let Some(screen) = self.client.chat_ui.chat_screen.as_mut() {
                    screen.resize(&m, &c, w as i32, h as i32);
                }
            }
            Screen::CommandBlock => {
                if let Some(screen) = self.client.command_block_screen.0.as_mut() {
                    screen.resize(&m, &c, w, h);
                }
            }
            _ => {}
        }
    }

    /// `ChatScreen.resize` and `AbstractCommandBlockEditScreen.resize` at the
    /// build's size: the open text box is re-made here, keeping its text, and
    /// never in the draw. Then the screen's layout, when the build lacks it at
    /// this size (or, for the edit screen, for this kind of block). An
    /// unchanged frame only compares: the boxes keep their size and nothing
    /// is laid out.
    pub(in crate::game::client_side) fn text_screen_layout(
        &mut self,
        identity: Option<&'static str>,
        gui: GuiSize,
    ) -> Option<ScreenLayout> {
        let (w, h) = (gui.size[0] as f32, gui.size[1] as f32);
        let cart = match self.client.screen {
            Screen::Chat => None,
            Screen::CommandBlock => Some(self.client.command_block_screen.0.as_ref()?.cart()),
            _ => return None,
        };
        self.resize_text_screens(w, h);
        let built = self
            .client
            .screen_state
            .0
            .build
            .current
            .as_ref()
            .filter(|build| {
                build.screen == self.client.screen
                    && build.identity == identity
                    && build.scale == gui.scale as f32
            });
        match cart {
            None => built.and_then(|build| build.chat(w, h)).is_none().then(|| {
                ScreenLayout::Chat(ChatLayout {
                    leave_bed: Self::leave_bed_button(w, h),
                })
            }),
            Some(cart) => built
                .and_then(|build| build.command_block(w, h))
                .is_none_or(|old| old.cart != cart)
                .then(|| {
                    ScreenLayout::CommandBlock(CommandBlockLayout {
                        cart,
                        buttons: super::command_blocks::CommandBlockScreen::buttons(w, h, cart),
                    })
                }),
        }
    }
}

use sunrise_gameplay_common::game::native_book_displays::NativeBookDisplaysGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    pub fn gui(framebuffer: [u32; 2], scale: u32) -> GuiSize {
        GuiSize::new(framebuffer, scale, false)
    }

    use super::{
        BuildLayout, ContainerLayout, EffectsLayout, PauseInputs, PauseLayout, ScreenBuildState,
        ScreenLayout,
    };
    use sunrise_client::hud::{GuiScale, HudBatch};
    use sunrise_client::window::GuiSize;
    use sunrise_gameplay_common::game::Screen;

    /// A container layout with only the fields a case cares about set.
    fn container(
        book: Option<[f32; 4]>,
        effects: Option<[f32; 4]>,
        effects_layout: EffectsLayout,
        tabs: Option<[[f32; 4]; 2]>,
    ) -> ContainerLayout {
        ContainerLayout {
            slot_inputs: Default::default(),
            image: [10.0, 20.0, 176.0, 166.0],
            origin: (10.0, 20.0),
            book,
            effects,
            shows_effects: effects.is_some(),
            effects_revision: 0,
            effects_layout,
            tabs,
            slots: vec![(0, [9.0, 141.0, 18.0, 18.0])],
        }
    }

    /// One build per open and per resize; none for a frame that changed
    /// nothing, and none for a menu swapped on the same screen.
    #[test]
    fn a_build_per_open_and_resize_only() {
        let mut state = ScreenBuildState::default();
        let small = gui([854, 480], 0);
        state.sync(Screen::Inventory, true, None, small, None);
        assert_eq!(state.builds, 1);
        // Frames pass with the same screen and size.
        for _ in 0..3 {
            state.sync(Screen::Inventory, true, None, small, None);
        }
        assert_eq!(state.builds, 1);
        // A menu swapped on the same screen keeps the build.
        state.sync(Screen::Inventory, true, None, small, None);
        assert_eq!(state.builds, 1);
        // A window resize rebuilds once.
        let large = gui([1920, 1080], 0);
        state.sync(Screen::Inventory, true, None, large, None);
        assert_eq!(state.builds, 2);
        state.sync(Screen::Inventory, true, None, large, None);
        assert_eq!(state.builds, 2);
        // A GUI scale change rebuilds once.
        let scaled = GuiSize::new([1920, 1080], 3, false);
        state.sync(Screen::Inventory, true, None, scaled, None);
        assert_eq!(state.builds, 3);
        // A different screen opens.
        state.sync(Screen::Paused, true, None, scaled, None);
        assert_eq!(state.builds, 4);
    }

    #[test]
    fn distinct_title_and_overlay_identities_rebuild_at_the_same_size() {
        let mut state = ScreenBuildState::default();
        let size = gui([854, 480], 0);
        state.sync(Screen::Playing, true, Some("title"), size, None);
        assert_eq!(state.builds, 1);
        state.sync(Screen::Playing, true, Some("multiplayer"), size, None);
        assert_eq!(state.builds, 2);
        state.sync(Screen::Playing, true, Some("sign_editor"), size, None);
        assert_eq!(state.builds, 3);
        state.sync(Screen::Playing, true, Some("book_view"), size, None);
        assert_eq!(state.builds, 4);
        state.sync(Screen::Playing, true, Some("book_view"), size, None);
        assert_eq!(state.builds, 4);
        state.reopened();
        state.sync(Screen::Playing, true, Some("book_view"), size, None);
        assert_eq!(state.builds, 5);
    }

    #[test]
    fn closing_drops_the_build() {
        let mut state = ScreenBuildState::default();
        let size = gui([1280, 720], 0);
        state.sync(Screen::Paused, true, None, size, None);
        assert!(state.current.is_some());
        state.sync(Screen::Playing, false, None, size, None);
        assert!(state.current.is_none());
        assert_eq!(state.builds, 1);
        // Nothing open builds nothing.
        state.sync(Screen::Playing, false, None, size, None);
        assert_eq!(state.builds, 1);
    }

    /// A title or an overlay shows with `Screen::Playing` under it: it is still
    /// something to build for.
    #[test]
    fn a_title_or_overlay_over_playing_builds() {
        let mut state = ScreenBuildState::default();
        state.sync(Screen::Playing, true, None, gui([854, 480], 0), None);
        assert_eq!(state.builds, 1);
        assert_eq!(state.current.as_ref().unwrap().screen, Screen::Playing);
    }

    /// The build's size and scale are the ones the HUD draws at
    /// (`HudBatch::new`), at every window size and GUI scale.
    #[test]
    fn the_build_is_the_size_and_scale_the_hud_draws_at() {
        for (window, scale, unicode) in [
            ([320, 240], 0, false),
            ([854, 480], 1, false),
            ([1280, 720], 2, true),
            ([1920, 1080], 3, false),
            ([2560, 1440], 4, false),
            ([3840, 2160], 0, true),
            ([1279, 719], 2, false),
        ] {
            let gui = GuiSize::new(window, scale, unicode);
            let batch = HudBatch::new(
                window,
                GuiScale {
                    requested: scale,
                    unicode,
                },
            );
            let mut state = ScreenBuildState::default();
            state.sync(Screen::Inventory, true, None, gui, None);
            let build = state.current.as_ref().unwrap();
            assert_eq!(
                build.size,
                (batch.width, batch.height),
                "{window:?} at {scale}"
            );
            assert_eq!(build.scale, batch.scale, "{window:?} at {scale}");
            assert_eq!(build.rect, [0.0, 0.0, batch.width, batch.height]);
            assert!(matches!(build.layout, BuildLayout::Empty));
        }
    }

    /// The pause menu's layout is made when it opens and when it resizes, not
    /// per frame: the build keeps it, and the readers get the same buttons and
    /// icons the frame drew.
    #[test]
    fn the_pause_layout_is_built_once_per_open_and_resize() {
        let mut state = ScreenBuildState::default();
        let small = gui([854, 480], 0);
        let layout = PauseLayout {
            inputs: PauseInputs::default(),
            buttons: vec![
                (113.0, 74.0, 204.0, "menu.returnToGame", 0, true),
                (113.0, 98.0, 98.0, "gui.advancements", 4, true),
            ],
            icons: [
                (173.0, 122.0, "pause_menu/bug"),
                (197.0, 122.0, "pause_menu/social_interactions"),
                (221.0, 122.0, "friends/friends"),
                (245.0, 122.0, "pause_menu/player_reporting"),
            ],
        };
        state.sync(
            Screen::Paused,
            true,
            None,
            small,
            Some(ScreenLayout::Pause(layout.clone())),
        );
        assert_eq!(state.builds, 1);
        // Frames pass no fresh layout: the build keeps the one it made.
        for _ in 0..3 {
            state.sync(Screen::Paused, true, None, small, None);
        }
        assert_eq!(state.builds, 1);
        let built = state.current.as_ref().unwrap();
        assert_eq!(
            built.pause(small.size[0] as f32, small.size[1] as f32),
            Some(&layout)
        );
        // Another size is another build, and reads the new one.
        let large = gui([1920, 1080], 0);
        let moved = PauseLayout {
            inputs: PauseInputs::default(),
            buttons: vec![(500.0, 200.0, 204.0, "menu.returnToGame", 0, true)],
            icons: layout.icons,
        };
        state.sync(
            Screen::Paused,
            true,
            None,
            large,
            Some(ScreenLayout::Pause(moved.clone())),
        );
        assert_eq!(state.builds, 2);
        assert_eq!(
            state
                .current
                .as_ref()
                .unwrap()
                .pause(large.size[0] as f32, large.size[1] as f32),
            Some(&moved)
        );
        assert_eq!(
            state
                .current
                .as_ref()
                .unwrap()
                .pause(small.size[0] as f32, small.size[1] as f32),
            None
        );
        // A server's links appearing while the menu is open is another
        // layout at the same size, even with the same buttons.
        let linked = PauseLayout {
            inputs: PauseInputs {
                links: true,
                ..Default::default()
            },
            buttons: moved.buttons.clone(),
            icons: moved.icons,
        };
        state.sync(
            Screen::Paused,
            true,
            None,
            large,
            Some(ScreenLayout::Pause(linked.clone())),
        );
        assert_eq!(state.builds, 3);
        assert_eq!(
            state
                .current
                .as_ref()
                .unwrap()
                .pause(large.size[0] as f32, large.size[1] as f32),
            Some(&linked)
        );
        // The same inputs keep it.
        state.sync(
            Screen::Paused,
            true,
            None,
            large,
            Some(ScreenLayout::Pause(linked)),
        );
        assert_eq!(state.builds, 3);
        // A different screen is its own build.
        state.sync(Screen::Options, true, None, large, None);
        assert_eq!(state.builds, 4);
        assert!(matches!(
            state.current.as_ref().unwrap().layout,
            BuildLayout::Empty
        ));
    }

    /// The build's exclusion is the panels that build draws, and it follows
    /// them in the same call: the effects appearing or the mod's answer moving
    /// them never leaves the exclusion a frame behind.
    #[test]
    fn the_exclusion_is_the_panels_the_build_draws() {
        let mut state = ScreenBuildState::default();
        let size = gui([854, 480], 0);
        let book = [100.0, 20.0, 147.0, 166.0];
        let mut layout = container(Some(book), None, EffectsLayout::vanilla(200), None);
        state.sync(
            Screen::Inventory,
            true,
            None,
            size,
            Some(ScreenLayout::Container(layout.clone())),
        );
        assert_eq!(state.builds, 1);
        assert_eq!(state.current.as_ref().unwrap().exclusion, vec![book]);
        // A potion takes effect: from this call on the book and the panel.
        let panel = [200.0, 20.0, 100.0, 32.0];
        layout.effects = Some(panel);
        state.sync(
            Screen::Inventory,
            true,
            None,
            size,
            Some(ScreenLayout::Container(layout.clone())),
        );
        assert_eq!(state.builds, 2);
        assert_eq!(state.current.as_ref().unwrap().exclusion, vec![book, panel]);
        // The mod answers compact and nudged: the exclusion follows at once.
        let moved = [205.0, 20.0, 32.0, 32.0];
        layout.effects = Some(moved);
        layout.effects_layout = EffectsLayout {
            compact: true,
            offset: 5,
            hidden: false,
        };
        state.sync(
            Screen::Inventory,
            true,
            None,
            size,
            Some(ScreenLayout::Container(layout.clone())),
        );
        assert_eq!(state.builds, 3);
        assert_eq!(state.current.as_ref().unwrap().exclusion, vec![book, moved]);
        // The panel the drawing reads is the one the exclusion named.
        assert_eq!(
            state.current.as_ref().unwrap().origin,
            (layout.origin.0, layout.origin.1)
        );
        assert_eq!(
            state.current.as_ref().unwrap().rect,
            [10.0, 20.0, 176.0, 166.0]
        );
        // An unchanged frame keeps the build.
        state.sync(
            Screen::Inventory,
            true,
            None,
            size,
            Some(ScreenLayout::Container(layout)),
        );
        assert_eq!(state.builds, 3);
    }

    /// The creative screen's exclusion is its two tab rows over and under the
    /// window, then the effects; the same layout keeps the same build.
    #[test]
    fn the_creative_tab_rows_are_excluded_above_and_below() {
        let mut state = ScreenBuildState::default();
        let size = gui([1280, 720], 0);
        let tabs = Some([[0.0, 0.0, 195.0, 28.0], [0.0, 136.0, 195.0, 28.0]]);
        let layout = container(None, None, EffectsLayout::vanilla(100), tabs);
        state.sync(
            Screen::Creative,
            true,
            None,
            size,
            Some(ScreenLayout::Container(layout.clone())),
        );
        assert_eq!(
            state.current.as_ref().unwrap().exclusion,
            vec![[0.0, 0.0, 195.0, 28.0], [0.0, 136.0, 195.0, 28.0]]
        );
        state.sync(
            Screen::Creative,
            true,
            None,
            size,
            Some(ScreenLayout::Container(layout)),
        );
        assert_eq!(state.builds, 1);
    }
}
