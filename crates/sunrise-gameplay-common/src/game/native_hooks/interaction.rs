#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InteractionDecision {
    pub canceled: bool,
    pub swing: bool,
}

impl InteractionDecision {
    pub fn canceled_swing(self) -> Option<bool> {
        self.canceled.then_some(self.swing)
    }
}
