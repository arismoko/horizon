//! Resolve mod panels beside the window and open book using the vanilla recipe-book centering rule.
use super::*;

/// A mod's answer as room: not finite or below 0 counts as 0.
fn room(answer: f32) -> f32 {
    if answer.is_finite() {
        answer.max(0.0)
    } else {
        0.0
    }
}

/// Return the signed offset only if the window, book, room and toggle all fit.
pub(in crate::game::client_side) fn room_offset(
    width: f32,
    x: f32,
    image: f32,
    book: Option<f32>,
    toggle: Option<[f32; 2]>,
    [left, right]: [f32; 2],
) -> f32 {
    if left == 0.0 && right == 0.0 {
        return 0.0;
    }
    let offset = (left / 2.0).floor() - (right / 2.0).floor();
    let first = book.map_or(x, |book| book.min(x));
    let reachable = toggle.is_none_or(|[l, r]| l + offset >= 0.0 && r + offset <= width);
    if first + offset - left >= 0.0 && x + offset + image + right <= width && reachable {
        offset
    } else {
        0.0
    }
}

impl crate::game::ClientGame<'_> {
    /// Ask once per frame for left and right room beside an eligible inventory screen.
    pub(in crate::game::client_side) fn mod_room(&self, batch: &HudBatch) -> [f32; 2] {
        let kind = self.common.station.as_ref().map(|s| s.kind);
        if self.client.screen != Screen::Inventory
            || matches!(
                kind,
                Some(stations::Kind::Merchant | stations::Kind::Beacon)
            )
            || self.book_covers_menu(batch.width)
            || !mod_api::__private::listened(mod_api::HookId::ScreenRoom)
        {
            return [0.0; 2];
        }
        if let Some(rooms) = self
            .client
            .screen_state
            .0
            .build
            .room_answer(self.client.frames, self.client.screen)
        {
            return rooms;
        }
        let image = self.native_screen_width().unwrap_or(176.0);
        let height = self.screen_height();
        let (x, y) = batch.centered_origin(image, height);
        let rect = [x + self.recipe_book_shift(batch.width), y, image, height];
        // Asked while the screen is laid out: nothing is drawn beside it yet.
        let view = super::screen_backgrounds::without_regions(
            &self.screen_view_at(self.client.screen, rect),
        );
        // Memoize zero during dispatch to stop recursive layout queries from firing the hook again.
        let build = &self.client.screen_state.0.build;
        build.keep_room_answer(self.client.frames, self.client.screen, [0.0; 2]);
        let (mut left, mut right) = (0.0, 0.0);
        self.native_screen_room(&view, &mut left, &mut right);
        let rooms = [room(left), room(right)];
        build.keep_room_answer(self.client.frames, self.client.screen, rooms);
        rooms
    }

    /// The signed offset of the container and open book after the mods reserve room.
    pub(in crate::game::client_side) fn screen_room_offset(&self, batch: &HudBatch) -> f32 {
        let rooms = self.mod_room(batch);
        if rooms == [0.0; 2] {
            return 0.0;
        }
        let image = self.native_screen_width().unwrap_or(176.0);
        let (x, _) = batch.centered_origin(image, self.screen_height());
        let x = x + self.recipe_book_shift(batch.width);
        let (book, toggle) = (self.open_book_left(batch), self.mod_book_toggle_span(batch));
        room_offset(batch.width, x, image, book, toggle, rooms)
    }
}

#[cfg(test)]
mod tests {
    use super::{room, room_offset};

    #[test]
    fn an_answer_that_is_not_finite_or_below_zero_is_no_room() {
        assert_eq!(room(149.0), 149.0);
        assert_eq!(room(0.0), 0.0);
        assert_eq!(room(-20.0), 0.0);
        assert_eq!(room(f32::NAN), 0.0);
        assert_eq!(room(f32::INFINITY), 0.0);
        assert_eq!(room(f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn room_on_the_right_moves_the_window_left_by_half_when_it_all_fits() {
        // 1440p at the automatic GUI scale: 427 wide, the window at 125. A panel of 147 and its gap of 2 fit right of it once the window moves 74 left.
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, None, [0.0, 149.0]),
            -74.0
        );
        // Just wide enough, and a pixel short.
        assert_eq!(
            room_offset(325.0, 74.0, 176.0, None, None, [0.0, 149.0]),
            -74.0
        );
        assert_eq!(
            room_offset(320.0, 72.0, 176.0, None, None, [0.0, 149.0]),
            0.0
        );
        // No room asked.
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, None, [0.0, 0.0]),
            0.0
        );
    }

    #[test]
    fn room_on_the_left_moves_the_window_right_by_half_when_it_all_fits() {
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, None, [149.0, 0.0]),
            74.0
        );
        // Just wide enough; a pixel short on the left, then on the right.
        assert_eq!(
            room_offset(325.0, 75.0, 176.0, None, None, [149.0, 0.0]),
            74.0
        );
        assert_eq!(
            room_offset(325.0, 74.0, 176.0, None, None, [149.0, 0.0]),
            0.0
        );
        assert_eq!(
            room_offset(324.0, 75.0, 176.0, None, None, [149.0, 0.0]),
            0.0
        );
    }

    #[test]
    fn room_on_both_sides_moves_the_window_by_the_difference_of_the_halves() {
        // 50 right less 74 left, with 1 pixel spare on the right and none on the left.
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, None, [100.0, 149.0]),
            -24.0
        );
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, None, [101.0, 149.0]),
            -24.0
        );
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, None, [102.0, 149.0]),
            -23.0
        );
        // A pixel too much on the left: nothing moves, the right room alone would.
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, None, [104.0, 149.0]),
            0.0
        );
        // Each half rounds down on its own.
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, None, [3.0, 5.0]),
            -1.0
        );
    }

    #[test]
    fn the_book_beside_the_window_moves_with_it_or_keeps_it_in_place() {
        // The book open beside the window (the window 77 right of centre, the book left of it): at 427 the three don't fit, so nothing moves; at 640 they do.
        assert_eq!(
            room_offset(427.0, 202.0, 176.0, Some(22.0), None, [0.0, 149.0]),
            0.0
        );
        assert_eq!(
            room_offset(640.0, 309.0, 176.0, Some(128.0), None, [0.0, 149.0]),
            -74.0
        );
        // Panel at 95 would fit after a shift of 90, but its selected tab at 63 would not.
        assert_eq!(
            room_offset(510.0, 244.0, 176.0, Some(63.0), None, [0.0, 180.0]),
            0.0
        );
        // The selected tab remains at 13 and the reservation ends at 470.
        assert_eq!(
            room_offset(510.0, 244.0, 176.0, Some(63.0), None, [0.0, 100.0]),
            -50.0
        );
        // The book's left edge is what runs out first on the left.
        assert_eq!(
            room_offset(640.0, 300.0, 176.0, Some(60.0), None, [0.0, 149.0]),
            0.0
        );
        // Room on the left is measured from the selected tab: 13 of it spare after moving 50 right, and refused when it would reach past the screen's edge.
        assert_eq!(
            room_offset(510.0, 244.0, 176.0, Some(63.0), None, [100.0, 0.0]),
            50.0
        );
        assert_eq!(
            room_offset(510.0, 244.0, 176.0, Some(63.0), None, [180.0, 0.0]),
            0.0
        );
        assert_eq!(
            room_offset(640.0, 309.0, 176.0, Some(128.0), None, [149.0, 0.0]),
            74.0
        );
        // Both sides around the book: 30 right less 74 left.
        assert_eq!(
            room_offset(640.0, 309.0, 176.0, Some(128.0), None, [60.0, 149.0]),
            -44.0
        );
        assert_eq!(
            room_offset(427.0, 202.0, 176.0, Some(22.0), None, [20.0, 149.0]),
            0.0
        );
    }

    #[test]
    fn a_mod_menus_book_toggle_stays_on_the_screen() {
        // A closed 176-wide mod menu at 125 on a 427-wide screen, its toggle in the right gutter at 305..325 (`layout`'s first gutter, 4 right of the window).
        let toggle = Some([305.0, 325.0]);
        // 248 on the left would fit the window at 249..425, but push the toggle to 429..449.
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, None, [248.0, 0.0]),
            124.0
        );
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, toggle, [248.0, 0.0]),
            0.0
        );
        // 100 moves the toggle to 355..375, still on the screen; the last that fits ends at 427.
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, toggle, [100.0, 0.0]),
            50.0
        );
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, toggle, [204.0, 0.0]),
            102.0
        );
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, toggle, [206.0, 0.0]),
            0.0
        );
        // Room on the right moves the toggle left with the window, as before.
        assert_eq!(
            room_offset(427.0, 125.0, 176.0, None, toggle, [0.0, 149.0]),
            -74.0
        );
    }
}
