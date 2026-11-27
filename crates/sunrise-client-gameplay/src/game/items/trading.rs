#[cfg(test)]
pub(crate) mod tests {
    use crate::game::client_side::trading_client::{
        SHIFT, drag_scroll, on_scroller, scroller_cursor, scroller_rect,
    };

    use sunrise_client::widgets::Cursor;

    #[test]
    fn scroller_takes_presses_only_with_more_than_seven_offers() {
        let x = SHIFT + 97.0;
        assert!(on_scroller((x, 20.0), 8));
        assert!(!on_scroller((x, 20.0), 7));
        // The track's edges: x is exclusive both ways, y includes 158.
        assert!(!on_scroller((SHIFT + 94.0, 20.0), 8));
        assert!(!on_scroller((SHIFT + 100.0, 20.0), 8));
        assert!(!on_scroller((x, 18.0), 8));
        assert!(on_scroller((x, 158.0), 8));
        assert!(!on_scroller((x, 158.5), 8));
    }
    #[test]
    fn dragged_scroller_picks_the_nearest_offer() {
        // 10 offers: the list scrolls 3 steps over the track's 112 px.
        assert_eq!(drag_scroll(0.0, 10), 0);
        assert_eq!(drag_scroll(31.5, 10), 0);
        assert_eq!(drag_scroll(31.5 + 112.0 / 2.0, 10), 2);
        assert_eq!(drag_scroll(31.5 + 112.0, 10), 3);
        assert_eq!(drag_scroll(500.0, 10), 3);
    }
    /// `MerchantScreen.extractScroller`: the thumb, not the whole track, takes
    /// the cursor; the vertical resize while it is dragged.
    #[test]
    fn the_scroller_thumb_asks_for_its_cursor() {
        // 8 offers: steps 2, the thumb at the track's top, 139 px tall.
        let rect = scroller_rect(8, 0).unwrap();
        assert_eq!(rect, [44.0, 18.0, 6.0, 27.0]);
        let at = |x: f32, y: f32, dragging: bool| scroller_cursor((x, y), rect, dragging);
        assert_eq!(at(45.0, 19.0, false), Some(Cursor::PointingHand));
        assert_eq!(at(45.0, 19.0, true), Some(Cursor::ResizeNs));
        assert_eq!(at(43.0, 19.0, false), None);
        assert_eq!(at(45.0, 46.0, false), None);
        assert_eq!(at(45.0, 45.0, false), Some(Cursor::PointingHand));
        // 7 offers: the disabled scroller takes no cursor.
        assert!(scroller_rect(7, 0).is_none());
    }
}
