//! The `screen_room` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Room for a mod's own panels beside the open container screen's window, in GUI pixels:
    /// `left` and `right` start at 0; raise one to the width your panel takes on that side of the
    /// window, its gap included, and leave it be otherwise. A room on the left is measured from
    /// the recipe book open beside the window (its selected tab included), not from the window.
    /// Later mods see earlier mods' answers: keep the larger
    /// (`event.right = event.right.max(mine)`). When the left room, the recipe book, the window and
    /// the right room all fit across the screen, the window and the book move right by half the
    /// left room less half the right room (each rounded down), so the four stay centred as
    /// vanilla's recipe book moves the window right to make room for itself
    /// (`RecipeBookComponent.updateScreenPosition`); when they don't fit, or a mod menu's recipe
    /// book toggle would leave the screen, nothing moves. Version 2: a left room was added, so
    /// another mod's left answer can change or cancel the move a right-only answer gets.
    /// The query's `ScreenView::rect` is pre-reservation. Later draw/input views expose the
    /// shifted window; check the room you asked against the viewport before drawing in it.
    /// Keep this query pure and cheap. Its answer is memoized for the frame and screen; recursive
    /// queries during dispatch see zero. Asked once a
    /// frame while the player's inventory, a container or a mod's menu screen is open, not for the
    /// creative inventory, a merchant's or a beacon's screen, nor while the recipe book covers the
    /// window of a narrow screen; an answer that changes geometry builds the layout again in the
    /// same frame (slots, book and effect panels move with the window). Not finite or below 0
    /// counts as 0. Beyond NeoForge: a mod there reads the container screen's `leftPos` but has no
    /// event to move it, so it draws in the space that's left.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "right",
        neoforge = "none (a NeoForge mod reads AbstractContainerScreen.leftPos and cannot move it)",
        cancellable = false,
        version = 2
    )]
    fn screen_room(screen: &ScreenView<'_>, left: &mut f32, right: &mut f32);
}
