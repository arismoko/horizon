use sunrise_client::{
    hud::rgba,
    toasts::{Slide, ToastClock},
};
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;
use sunrise_sim::advancements::Frame;

pub struct Toast {
    pub icon: ItemStack,
    pub frame: Frame,
    pub title: Text,
    pub description: Option<Text>,
    pub slide: Slide,
    /// A `RecipeToast`: the crafting station and result of each display
    /// unlocked while it lasts (`recipeItems`); empty for an advancement.
    pub recipes: Vec<(String, ItemStack)>,
    /// `RecipeToast.changed` and `lastChanged`.
    pub changed: bool,
    pub last_changed: i64,
}
impl Toast {
    /// `RecipeToast.addOrUpdate` on a new toast.
    pub(in crate::game) fn recipe(items: Vec<(String, ItemStack)>) -> Self {
        Self {
            icon: ItemStack::new("minecraft:air", 0),
            frame: Frame::Task,
            title: Text::translate("recipe.toast.title", &[]),
            description: Some(Text::translate("recipe.toast.description", &[])),
            slide: Slide::default(),
            recipes: items,
            changed: true,
            last_changed: 0,
        }
    }
    pub(in crate::game) fn is_recipe(&self) -> bool {
        !self.recipes.is_empty()
    }
    /// `RecipeToast.addItem`: more displays on a toast still queued or shown.
    pub(in crate::game) fn add_recipes(&mut self, items: Vec<(String, ItemStack)>) {
        self.recipes.extend(items);
        self.changed = true;
    }
    pub(in crate::game) fn is_challenge(&self) -> bool {
        !self.is_recipe() && self.frame == Frame::Challenge
    }
    pub(in crate::game) fn update(
        &mut self,
        clock: ToastClock,
        multiplier: f64,
    ) -> (Option<bool>, bool) {
        let mut slide = std::mem::take(&mut self.slide);
        let result = slide.update_on(clock, |fully| self.wants_show(fully, multiplier));
        self.slide = slide;
        result
    }
    /// `Toast.update` with `fullyVisibleForMs`: whether it wants to be shown.
    pub(in crate::game) fn wants_show(&mut self, fully_for: i64, multiplier: f64) -> bool {
        let time = 5000.0 * multiplier;
        if self.recipes.is_empty() {
            // AdvancementToast.
            return (fully_for as f64) < time;
        }
        if self.changed {
            self.last_changed = fully_for;
            self.changed = false;
        }
        ((fully_for - self.last_changed) as f64) < time
    }
    /// `RecipeToast.displayedRecipeIndex`: the displays take turns over the
    /// toast's display time.
    pub(in crate::game) fn displayed_recipe(&self, fully_for: i64, multiplier: f64) -> usize {
        let n = self.recipes.len() as f64;
        (fully_for as f64 / (5000.0 * multiplier / n).max(1.0) % n) as usize
    }
}

/// The toast's frame label and its colour (`AdvancementToast`: -30465, pink,
/// for a challenge, else -256, yellow), packed as `hud::rgba` for the batch.
pub fn toast_label(frame: Frame) -> (&'static str, u32) {
    match frame {
        Frame::Task => ("advancements.toast.task", rgba(255, 255, 0, 255)),
        Frame::Goal => ("advancements.toast.goal", rgba(255, 255, 0, 255)),
        Frame::Challenge => ("advancements.toast.challenge", rgba(255, 136, 255, 255)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn recipes(n: usize) -> Toast {
        Toast::recipe(
            (0..n)
                .map(|i| {
                    let item = ItemStack::new(&format!("minecraft:item_{i}"), 1);
                    ("minecraft:crafting_table".to_owned(), item)
                })
                .collect(),
        )
    }
    /// Drives a toast frame by frame (every `step` ms) and returns the slide
    /// portions at the given times and when it finished.
    pub fn run(toast: &mut Toast, step: i64, until: i64, at: &[i64]) -> (Vec<f32>, Option<i64>) {
        let mut slide = Slide::default();
        let mut seen = Vec::new();
        for now in (0..=until).step_by(step as usize) {
            let (_, finished) = slide.update(now, |f| toast.wants_show(f, 1.0));
            if finished {
                return (seen, Some(now));
            }
            if at.contains(&now) {
                seen.push(slide.portion);
            }
        }
        (seen, None)
    }
    #[test]
    fn toasts_slide_in_squared_and_out_after_five_seconds() {
        let mut toast = recipes(1);
        let (portions, finished) = run(&mut toast, 10, 10_000, &[0, 300, 600, 3000]);
        // (300 / 600)² = 0.25: ToastInstance.calculateVisiblePortion.
        assert_eq!(portions, [0.0, 0.25, 1.0, 1.0]);
        // Fully in at 600, shown 5000 ms, out over 600 more.
        assert!(
            finished.is_some_and(|t| (6200..=6220).contains(&t)),
            "{finished:?}"
        );
    }

    #[test]
    fn the_first_update_plays_the_show_sound_and_the_hide_sound_follows() {
        let mut toast = recipes(1);
        let mut slide = Slide::default();
        assert_eq!(slide.update(0, |f| toast.wants_show(f, 1.0)).0, Some(true));
        assert_eq!(slide.update(100, |f| toast.wants_show(f, 1.0)).0, None);
        assert_eq!(
            slide.update(5600, |f| toast.wants_show(f, 1.0)).0,
            Some(false)
        );
    }

    #[test]
    fn recipe_toasts_cycle_through_their_displays_over_the_display_time() {
        let toast = recipes(4);
        let shown: Vec<usize> = [0, 1249, 1250, 2500, 3750, 4999, 5000]
            .iter()
            .map(|&t| toast.displayed_recipe(t, 1.0))
            .collect();
        assert_eq!(shown, [0, 0, 1, 2, 3, 3, 0]);
        // notificationDisplayTime doubles each turn.
        assert_eq!(toast.displayed_recipe(2500, 2.0), 1);
    }

    #[test]
    fn unlocking_more_recipes_keeps_the_toast_up_five_more_seconds() {
        let mut toast = recipes(1);
        let mut slide = Slide::default();
        let mut finished = None;
        for now in (0..=20_000).step_by(10) {
            if now == 4000 {
                toast.add_recipes(recipes(1).recipes);
            }
            if slide.update(now, |f| toast.wants_show(f, 1.0)).1 {
                finished = Some(now);
                break;
            }
        }
        assert_eq!(toast.recipes.len(), 2);
        // lastChanged moved to 3400 ms fully shown: hidden at 8400 + 600.
        assert!(
            finished.is_some_and(|t| (9600..=9620).contains(&t)),
            "{finished:?}"
        );
    }
}
