//! Client tutorial progression (`net.minecraft.client.tutorial`).

use super::toast_manager::Toast;
use sunrise_client::hud::{HudAtlas, HudBatch};
use sunrise_client::toasts::ToastClock;
use sunrise_client_gameplay::tutorial::social_hint_progress;
pub(in crate::game) use sunrise_client_gameplay::tutorial::{Hint, Previous, Step, Tutorial};

impl crate::game::ClientGame<'_> {
    /// `Tutorial.setStep` persists immediately so it survives leaving a world.
    pub(in crate::game::client_side) fn persist_tutorial_step(&mut self) {
        self.client.settings.file.set(
            "tutorialStep",
            sunrise_client::options_file::V::Int(self.client.tutorial.step as i32),
        );
        self.client
            .settings
            .save(&sunrise_gameplay_common::game::options_path());
    }
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn reset_tutorial(&mut self) {
        self.client.tutorial = Tutorial::new(Step::from_option(
            self.client.settings.file.int("tutorialStep"),
        ));
        self.client
            .toast_manager
            .queued
            .retain(|entry| !matches!(entry, Toast::Tutorial(_)));
        self.client
            .toast_manager
            .visible
            .retain(|entry| !matches!(entry.toast, Toast::Tutorial(_)));
    }

    fn tutorial_tagged(&self, item: bool, tag: &str, id: &str) -> bool {
        let items = self.common.res.items();
        let tags = if item {
            &items.item_tags
        } else {
            &items.block_tags
        };
        tags.get(tag).is_some_and(|members| members.contains(id))
    }

    pub(in crate::game::client_side) fn tutorial_get_item(
        &mut self,
        stack: Option<&sunrise_items::items::ItemStack>,
    ) {
        // The server thread's settings are a stale copy of the window's: a
        // guest's tutorial is their own client's to record.
        if self.common.options.dedicated.is_some()
            || self.client.title.0.is_some()
            || self.common.player.is_none()
        {
            return;
        }
        let Some(stack) = stack else { return };
        let id = &*stack.item;
        let tree = self.tutorial_tagged(true, "minecraft:completes_find_tree_tutorial", id);
        let log = self.tutorial_tagged(true, "minecraft:logs", id);
        let plank = self.tutorial_tagged(true, "minecraft:planks", id);
        if self.client.tutorial.get_item(tree, log, plank) {
            self.persist_tutorial_step();
        }
    }

    pub(in crate::game) fn tutorial_destroy(
        &mut self,
        state: sunrise_world::registry::BlockStateId,
        progress: f32,
    ) {
        // The server thread's settings are a stale copy of the window's: a
        // guest's tutorial is their own client's to record.
        if self.common.options.dedicated.is_some()
            || self.client.title.0.is_some()
            || self.common.player.is_none()
        {
            return;
        }
        let name = &self.common.res.facts().get(state).name;
        let is_log = self.tutorial_tagged(false, "minecraft:logs", name);
        if self.client.tutorial.destroy(is_log, progress) {
            self.persist_tutorial_step();
        }
    }

    /// Tick the current step and queue `Gui.addSocialInteractionsToast` on the first server.
    pub(in crate::game::client_side) fn tick_tutorial(&mut self) {
        use sunrise_client::actions::Action;
        if self.client.title.0.is_some()
            || self.client.remote.is_none()
            || self.common.player.is_none()
            || self.common.options.dedicated.is_some()
        {
            return;
        }
        // Minecraft.tick: also show this in a single-player world once it is
        // published to LAN, but not merely because its client uses memory networking.
        if !self.client.settings.file.bool("joinedFirstServer")
            && (!self.has_singleplayer_server() || self.is_published())
        {
            if let Some(toast) = self.tutorial_toast(Hint::SocialInteractions, Some(0.0)) {
                self.client.toast_manager.post(
                    ToastClock::of(self.common.clock.still()),
                    Toast::Tutorial(toast),
                );
            }
            self.client.settings.file.set(
                "joinedFirstServer",
                sunrise_client::options_file::V::Bool(true),
            );
            self.client
                .settings
                .save(&sunrise_gameplay_common::game::options_path());
        }
        if self.client.screen == super::Screen::Playing
            && ([
                Action::Forward,
                Action::Left,
                Action::Back,
                Action::Right,
                Action::Jump,
            ]
            .into_iter()
            .any(|action| self.held_mapping(action))
                || self
                    .input_move()
                    .is_some_and(|[left, forward]| left.abs() > 0.3 || forward.abs() > 0.3))
        {
            self.client.tutorial.input();
        }
        let items = self.common.res.items();
        let in_tag =
            |tag: &str, id: &str| items.item_tags.get(tag).is_some_and(|set| set.contains(id));
        let mut previous = Previous::default();
        if self.client.tutorial.waiting == 0
            && !matches!(self.client.tutorial.step, Step::Movement | Step::None)
        {
            for stack in self
                .common
                .inventory
                .iter()
                .chain(self.common.armor.iter())
                .flatten()
                .chain(self.common.offhand.iter())
            {
                previous.tree_item |= in_tag("minecraft:completes_find_tree_tutorial", &stack.item);
                previous.log |= in_tag("minecraft:logs", &stack.item);
                previous.plank |= in_tag("minecraft:planks", &stack.item);
            }
            if let Some(mined) = self.common.stats.values.get("minecraft:mined")
                && let Some(tree) = items
                    .block_tags
                    .get("minecraft:completes_find_tree_tutorial")
            {
                previous.mined_tree = mined
                    .iter()
                    .any(|(id, count)| *count > 0 && tree.contains(id));
            }
            if let Some(crafted) = self.common.stats.values.get("minecraft:crafted")
                && let Some(planks) = items.item_tags.get("minecraft:planks")
            {
                previous.crafted_plank = crafted
                    .iter()
                    .any(|(id, count)| *count > 0 && planks.contains(id));
            }
        }
        let survival = self.common.mode == sunrise_command::args::GameMode::Survival;
        if self.client.tutorial.tick(survival, &previous) {
            self.persist_tutorial_step();
        }
        if self.client.tutorial.step == Step::FindTree
            && self.pick_entity_hit().is_none()
            && let Some(hit) = self.pick()
        {
            let name = &self
                .common
                .res
                .facts()
                .get(self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
                .name;
            if self.tutorial_tagged(false, "minecraft:completes_find_tree_tutorial", name)
                && self.client.tutorial.look_at_tree()
            {
                self.persist_tutorial_step();
            }
        }
    }
}

pub struct HintToast {
    hint: Hint,
    pub lines: Vec<(sunrise_game_data::text::Text, bool)>,
    pub title_lines: usize,
    progress: Option<f32>,
    smooth: f32,
    last_smooth: i64,
    pub wanted: bool,
    slots: usize,
    slide: sunrise_client::toasts::Slide,
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn hide_social_tutorial(&mut self) {
        if let Some(toast) = self.client.toast_manager.find_mut(|entry| match entry {
            Toast::Tutorial(t) if t.hint == Hint::SocialInteractions => Some(t),
            _ => None,
        }) {
            toast.wanted = false;
        }
    }

    /// `Tutorial.key`: bold belongs to the bound key, not the translation using it.
    pub fn tutorial_toast(&self, hint: Hint, progress: Option<f32>) -> Option<HintToast> {
        use sunrise_client::keybinds;
        use sunrise_game_data::text::Text;
        let key =
            |mapping: &str| keybinds::display_name(self.client.settings.keys.get(mapping)).bold();
        let (title_key, fallback, description_key, description, title_args) = match hint {
            Hint::Move => (
                "tutorial.move.title",
                "Move with %1$s, %2$s, %3$s, %4$s",
                "tutorial.move.description",
                "Jump with %1$s",
                vec![
                    key("key.forward"),
                    key("key.left"),
                    key("key.back"),
                    key("key.right"),
                ],
            ),
            Hint::Look => (
                "tutorial.look.title",
                "Look around",
                "tutorial.look.description",
                "Use your mouse to turn",
                vec![],
            ),
            Hint::FindTree => (
                "tutorial.find_tree.title",
                "Find a tree",
                "tutorial.find_tree.description",
                "Punch it to collect wood",
                vec![],
            ),
            Hint::PunchTree => (
                "tutorial.punch_tree.title",
                "Punch the tree",
                "tutorial.punch_tree.description",
                "Hold %1$s to break",
                vec![],
            ),
            Hint::OpenInventory => (
                "tutorial.open_inventory.title",
                "Open inventory",
                "tutorial.open_inventory.description",
                "Press %1$s",
                vec![],
            ),
            Hint::SocialInteractions => (
                "tutorial.socialInteractions.title",
                "Social Interactions",
                "tutorial.socialInteractions.description",
                "Press %1$s to open social interactions",
                vec![],
            ),
            Hint::CraftPlanks => (
                "tutorial.craft_planks.title",
                "Craft wooden planks",
                "tutorial.craft_planks.description",
                "The recipe book can help",
                vec![],
            ),
        };
        let mut title = Text::translate_with_fallback(title_key, fallback, &title_args);
        let desc_args = match hint {
            Hint::Move => vec![key("key.jump")],
            Hint::PunchTree => vec![key("key.attack")],
            Hint::OpenInventory => vec![key("key.inventory")],
            Hint::SocialInteractions => vec![key("key.socialInteractions")],
            _ => vec![],
        };
        let mut description = Some(Text::translate_with_fallback(
            description_key,
            description,
            &desc_args,
        ));
        if !self.admit_toast(mod_api::ToastKind::Tutorial, &mut title, &mut description) {
            return None;
        }
        let font = &self.client.gui.atlas.font;
        let mut lines = font.split_lines(&title, 126.0);
        let title_lines = lines.len();
        if let Some(description) = description {
            lines.extend(font.split_lines(&description, 126.0));
        }
        let height = 10 + 11 * lines.len().max(2);
        let slots = height.div_ceil(32);
        Some(HintToast {
            hint,
            lines,
            title_lines,
            progress,
            smooth: 0.0,
            last_smooth: 0,
            wanted: true,
            slots,
            slide: Default::default(),
        })
    }

    pub(in crate::game::client_side) fn update_tutorial_toasts(&mut self) {
        if self.common.clock.still() {
            return;
        }
        let wanted = self.client.tutorial.hint();
        if wanted.map(|(hint, _)| hint) != self.client.tutorial.cancelled_hint() {
            self.client.tutorial.set_cancelled_hint(None);
        }
        let mut has_current_hint = false;
        for entry in self.client.toast_manager.all_mut() {
            let Toast::Tutorial(t) = entry else { continue };
            if t.hint == Hint::SocialInteractions {
                continue;
            }
            t.wanted = wanted.is_some_and(|(hint, _)| hint == t.hint);
            if t.wanted {
                t.progress = wanted.and_then(|(_, progress)| progress);
                has_current_hint = true;
            }
        }
        if let Some((hint, progress)) = wanted
            && self.client.tutorial.cancelled_hint() != Some(hint)
            && !has_current_hint
        {
            if let Some(toast) = self.tutorial_toast(hint, progress) {
                self.client
                    .toast_manager
                    .post(ToastClock::Real, Toast::Tutorial(toast));
            } else {
                self.client.tutorial.set_cancelled_hint(Some(hint));
            }
        }
        self.client
            .toast_manager
            .queued
            .retain(|entry| !matches!(entry, Toast::Tutorial(t) if !t.wanted));
    }
}

impl HintToast {
    pub(in crate::game::client_side) fn slots(&self) -> usize {
        self.slots
    }

    pub(in crate::game::client_side) fn update(
        &mut self,
        clock: ToastClock,
    ) -> (Option<bool>, bool) {
        let (sound, finished) = self.slide.update_on(clock, |fully| {
            self.wanted && (self.hint != Hint::SocialInteractions || social_hint_progress(fully).0)
        });
        if self.hint == Hint::SocialInteractions {
            let (_, progress) = social_hint_progress(self.slide.fully_for);
            self.progress = Some(progress);
            self.smooth = progress;
        } else if let Some(progress) = self.progress {
            let delta = ((self.slide.fully_for - self.last_smooth) as f32 / 100.0).clamp(0.0, 1.0);
            self.smooth += (progress - self.smooth) * delta;
            self.last_smooth = self.slide.fully_for;
        }
        (sound, finished)
    }

    pub(in crate::game::client_side) fn draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        y: f32,
    ) {
        let height = (10 + 11 * self.lines.len().max(2)) as f32;
        let x = batch.width - 160.0 * self.slide.portion;
        sunrise_client::ui::nine_slice(
            batch,
            atlas,
            "toast/tutorial",
            x,
            y,
            160.0,
            height,
            u32::MAX,
        );
        let icon = match self.hint {
            Hint::Move => "movement_keys",
            Hint::Look => "mouse",
            Hint::FindTree | Hint::PunchTree => "tree",
            Hint::OpenInventory => "recipe_book",
            Hint::CraftPlanks => "wooden_planks",
            Hint::SocialInteractions => "social_interactions",
        };
        batch.sprite_scaled(
            atlas,
            &format!("toast/{icon}"),
            x + 6.0,
            y + 6.0,
            20.0,
            20.0,
            u32::MAX,
        );
        let text_y = y + 7.0 + ((self.lines.len().max(2) - self.lines.len()) * 11) as f32 / 2.0;
        for (n, (line, _)) in self.lines.iter().enumerate() {
            batch.text_styled(
                atlas,
                line,
                x + 30.0,
                text_y + 11.0 * n as f32,
                if n < self.title_lines {
                    0xFF50_0050
                } else {
                    0xFF00_0000
                },
                false,
            );
        }
        if let Some(progress) = self.progress {
            batch.fill(atlas, x + 3.0, y + height - 4.0, 154.0, 1.0, u32::MAX);
            let color = if progress >= self.smooth {
                0xFF00_5500
            } else {
                0xFF55_0000
            };
            batch.fill(
                atlas,
                x + 3.0,
                y + height - 4.0,
                154.0 * self.smooth.clamp(0.0, 1.0),
                1.0,
                color,
            );
        }
    }
}

use sunrise_gameplay_common::game::integrated_common::IntegratedCommonGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
