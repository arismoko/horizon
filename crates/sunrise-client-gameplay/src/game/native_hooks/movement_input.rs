use mod_api::{MovementInput, MovementKeys};
use sunrise_sim::sprint::Keys;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Local player input, after sampling and before auto-jump and sprint decisions.
    fn native_movement_input_update(&self, call: mod_api::__private::MovementInputUpdateCall<'_>) {
        self.fire_movement_input_update(call);
    }

    /// Lend both parts of the sampled input together, without deriving either from edits to the other.
    pub fn update_movement_input(&self, keys: Keys) -> Keys {
        if !mod_api::__private::enabled_listened(mod_api::HookId::MovementInputUpdate) {
            return keys;
        }
        let mut input = MovementInput {
            move_vector: keys.direction(),
            key_presses: MovementKeys {
                forward: keys.forward,
                backward: keys.backward,
                left: keys.left,
                right: keys.right,
                jump: keys.jump,
                sneak: keys.shift,
                sprint: keys.sprint,
            },
        };
        self.native_movement_input_update(mod_api::__private::MovementInputUpdateCall {
            input: &mut input,
        });
        Keys {
            forward: input.key_presses.forward,
            backward: input.key_presses.backward,
            left: input.key_presses.left,
            right: input.key_presses.right,
            jump: input.key_presses.jump,
            shift: input.key_presses.sneak,
            sprint: input.key_presses.sprint,
            analog: Some(input.move_vector),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn movement_dispatch_fixture(&self, fire: bool) -> f32 {
        let mut input = MovementInput {
            move_vector: [0.0, 1.0],
            ..MovementInput::default()
        };
        let call = mod_api::__private::MovementInputUpdateCall { input: &mut input };
        if fire {
            self.native_movement_input_update(call);
        } else {
            mod_api::dispatch_movement_input_update(self, call);
        }
        input.move_vector[1]
    }
}
