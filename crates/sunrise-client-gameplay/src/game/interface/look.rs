//! The engine's only reader of the platform's look (docs/layers.md, "Look").
use crate::game::ClientGame;
use sunrise_hooks::look::{self, Look};

/// What `Game::look` last read: the generation it saw and the look then.
#[derive(Default)]
pub(crate) struct LookCache {
    generation: u64,
    look: Look,
}

impl ClientGame<'_> {
    /// This player's look, re-read only when some crate changed one: one atomic load otherwise.
    #[allow(
        dead_code,
        reason = "the first knob readers come with the features that move out"
    )]
    pub(in crate::game::client_side) fn look(&mut self) -> &Look {
        let generation = look::engine::generation();
        if self.client.look.generation != generation {
            self.client.look.look = look::engine::get(sunrise_hooks::seat());
            self.client.look.generation = generation;
        }
        &self.client.look.look
    }
}

#[cfg(test)]
mod tests {
    use sunrise_hooks::look::Look;

    #[test]
    fn the_default_look_is_what_the_extras_default_to() {
        let extras = sunrise_client_render::extras::Extras::default();
        let look = Look::default();
        assert_eq!(look.camera_name_tag, extras.own_name_tag);
        assert_eq!(look.chat.slide_in, extras.chat_animation);
        assert_eq!(look.chat.hide_indicator, !extras.chat_indicator);
    }
}
