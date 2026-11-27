use super::*;
use mod_api::{ClientHost, Hooks, ResolvePlayerSkinEvent};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Choice {
    pub selection: Option<PlayerSkinOverride>,
    pub calls: usize,
}

pub struct SkinPicker(pub Arc<Mutex<Choice>>);

#[mod_api::__subscriptions]
impl Hooks for SkinPicker {
    fn resolve_player_skin(&self, _: &dyn ClientHost, event: &mut ResolvePlayerSkinEvent<'_>) {
        assert!(
            event.selection.is_none(),
            "each frame starts with no selection"
        );
        let mut choice = self.0.lock().unwrap();
        choice.calls += 1;
        event.selection.clone_from(&choice.selection);
    }
}
