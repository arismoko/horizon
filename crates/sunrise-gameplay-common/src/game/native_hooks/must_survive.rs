use super::*;

impl GameLevel<'_> {
    /// `BlockItem.mustSurvive`, keyed by the placing item.
    pub(in crate::game) fn native_must_survive(&self, item: &str, answer: &mut Option<bool>) {
        self.fire_must_survive(item, answer);
    }
}
