//! A custom menu has no template-owned spare space. Use external gutters unless
//! its declaration explicitly reserves an internal toggle position.
#[derive(Debug, PartialEq)]
pub(super) struct BookLayout {
    pub shift: f32,
    pub overlay: bool,
    pub toggle: (f32, f32),
}
impl BookLayout {
    /// Keep the validated toggle anchored to the window when room moves it.
    pub(super) fn shifted_toggle(&self, room_offset: f32) -> (f32, f32) {
        (self.toggle.0 + room_offset, self.toggle.1)
    }
    /// The toggle's left and right edges before room moves it: 20 wide, as `layout` fits it.
    pub(super) fn toggle_span(&self) -> [f32; 2] {
        [self.toggle.0, self.toggle.0 + 20.0]
    }
}
pub(super) fn layout(
    view: [f32; 2],
    menu: [f32; 2],
    anchor: Option<mod_api::RecipeBookToggle>,
    open: bool,
) -> Option<BookLayout> {
    // Vanilla's fixed 77-pixel translation only separates its 176-wide menu.
    let overlay = menu[0] > 176.0 || view[0] < 379.0 || view[0] < menu[0] + 202.0;
    let shift = if open && !overlay { 77.0 } else { 0.0 };
    let x = ((view[0] - menu[0]) / 2.0).trunc() + shift;
    let y = ((view[1] - menu[1]) / 2.0).trunc();
    let fits =
        |bx: f32, by: f32| bx >= 0.0 && by >= 0.0 && bx + 20.0 <= view[0] && by + 18.0 <= view[1];
    let (px, py) = (
        ((view[0] - 147.0) / 2.0).floor(),
        ((view[1] - 166.0) / 2.0).floor(),
    );
    let clear = |bx: f32, by: f32| {
        !overlay || bx + 20.0 <= px || bx >= px + 147.0 || by + 18.0 <= py || by >= py + 166.0
    };
    let toggle = if let Some(anchor) = anchor {
        let top = match anchor.from {
            mod_api::ToggleOrigin::Menu => y,
            // Vanilla's `height / 2`: the window's height is whole GUI pixels.
            mod_api::ToggleOrigin::ScreenCentre => (view[1].floor() / 2.0).floor(),
        };
        let pos = (x + anchor.x, top + anchor.y);
        fits(pos.0, pos.1).then_some(pos)?
    } else {
        [
            (x + menu[0] + 4.0, y + 5.0),
            (x - 24.0, y + 5.0),
            (x + menu[0] - 20.0, y - 22.0),
        ]
        .into_iter()
        .find(|&(bx, by)| fits(bx, by) && clear(bx, by))?
    };
    Some(BookLayout {
        shift,
        overlay,
        toggle,
    })
}
pub(super) fn refusal(block: &str, menu: &str, anchored: bool) -> String {
    let reason = if anchored {
        "anchored toggle outside the viewport"
    } else {
        "no usable toggle gutter; declare Screen.recipe_book_toggle"
    };
    format!("recipe book refused for mod block {block} menu {menu}: {reason}")
}
#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::*;
    fn menu_anchor(x: f32, y: f32) -> mod_api::RecipeBookToggle {
        mod_api::RecipeBookToggle {
            x,
            y,
            from: mod_api::ToggleOrigin::Menu,
        }
    }
    #[test]
    fn a_screen_centre_anchor_follows_vanillas_integer_half_of_the_window_height() {
        let centred = |view_height: f32, menu_height: f32| {
            let anchor = mod_api::RecipeBookToggle {
                x: 5.0,
                y: -49.0,
                from: mod_api::ToggleOrigin::ScreenCentre,
            };
            layout(
                [427.0, view_height],
                [176.0, menu_height],
                Some(anchor),
                false,
            )
            .unwrap()
            .toggle
        };
        // `height / 2 - 49` with height 241 is 120 - 49, whatever the menu's own height.
        assert_eq!(centred(241.0, 166.0), (130.0, 71.0));
        assert_eq!(centred(241.0, 165.0), (130.0, 71.0));
        // The menu-top form of the same spot moves with the menu's rounded top edge.
        let from_menu = layout(
            [427.0, 241.0],
            [176.0, 165.0],
            Some(menu_anchor(5.0, 34.0)),
            false,
        )
        .unwrap()
        .toggle;
        assert_eq!(from_menu, (130.0, 72.0));
    }
    #[test]
    fn closed_gutter_and_declared_anchor_follow_the_room_offset() {
        let gutter = layout([427.0, 240.0], [176.0, 166.0], None, false).unwrap();
        assert_eq!(gutter.shifted_toggle(-74.0), (231.0, 42.0));
        let anchor = layout(
            [427.0, 240.0],
            [176.0, 166.0],
            Some(menu_anchor(8.0, 6.0)),
            false,
        )
        .unwrap();
        assert_eq!(anchor.shifted_toggle(-74.0), (59.0, 43.0));
    }
    #[test]
    fn narrow_overlay_rejects_covered_gutters() {
        // Panel spans x=76..223, y=37..203. Both side gutters overlap it;
        // the above-menu toggle at (180, 15) is clear.
        assert_eq!(
            layout([300.0, 240.0], [100.0, 166.0], None, true)
                .unwrap()
                .toggle,
            (180.0, 15.0)
        );
        assert!(layout([300.0, 240.0], [100.0, 100.0], None, true).is_none());
    }
    #[test]
    fn anchored_refusal_names_the_invalid_anchor() {
        assert!(
            layout(
                [240.0, 240.0],
                [240.0, 240.0],
                Some(menu_anchor(230.0, 6.0)),
                false
            )
            .is_none()
        );
        assert_eq!(
            refusal("demo:machine", "demo:menu", true),
            "recipe book refused for mod block demo:machine menu demo:menu: anchored toggle outside the viewport"
        );
    }
    #[test]
    fn independent_gutter_overlay_and_anchor_positions() {
        assert_eq!(
            layout([427.0, 240.0], [176.0, 166.0], None, true),
            Some(BookLayout {
                shift: 77.0,
                overlay: false,
                toggle: (382.0, 42.0)
            })
        );
        assert_eq!(
            layout([427.0, 240.0], [300.0, 166.0], None, true),
            Some(BookLayout {
                shift: 0.0,
                overlay: true,
                toggle: (367.0, 42.0)
            })
        );
        assert_eq!(
            layout([240.0, 240.0], [220.0, 166.0], None, false),
            Some(BookLayout {
                shift: 0.0,
                overlay: true,
                toggle: (210.0, 15.0)
            })
        );
        assert_eq!(
            layout(
                [240.0, 240.0],
                [240.0, 240.0],
                Some(menu_anchor(8.0, 6.0)),
                true
            ),
            Some(BookLayout {
                shift: 0.0,
                overlay: true,
                toggle: (8.0, 6.0)
            })
        );
        assert_eq!(
            layout([600.0, 240.0], [300.0, 166.0], None, true),
            Some(BookLayout {
                shift: 0.0,
                overlay: true,
                toggle: (454.0, 42.0)
            })
        );
        assert!(layout([240.0, 240.0], [240.0, 240.0], None, false).is_none());
        assert_eq!(
            refusal("demo:machine", "demo:menu", false),
            "recipe book refused for mod block demo:machine menu demo:menu: no usable toggle gutter; declare Screen.recipe_book_toggle"
        );
        assert!(
            layout(
                [240.0, 240.0],
                [240.0, 240.0],
                Some(menu_anchor(f32::NAN, 2.0)),
                false
            )
            .is_none()
        );
    }
}
