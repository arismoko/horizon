use super::*;

impl Game<'_> {
    /// The grabbed mouse moved `delta` (x/y window pixels) since the last frame and is about to
    /// turn the player (`MouseHandler.handleAccumulatedMovement`, before `turnPlayer`): raw,
    /// before the sensitivity, the cinematic camera's smoothing and the inverted axes. Cancel
    /// drops this frame's turn. Client-side.
    pub(in crate::game) fn native_mouse_turn_pre(&mut self, delta: [f64; 2], canceled: &mut bool) {
        self.fire_mouse_turn_pre(delta, canceled);
    }
}
