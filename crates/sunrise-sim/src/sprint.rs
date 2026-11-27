//! LocalPlayer sprint decisions around KeyboardInput.tick. Conditions are the
//! current player queries; setting sprint dispatches to the live attribute state.

#[derive(Clone, Copy, Debug, Default)]
pub struct Keys {
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub shift: bool,
    pub sprint: bool,
    /// A sampled stick or listener-edited move vector (`[left, forward]`),
    /// used instead of deriving the direction from the keys.
    pub analog: Option<[f32; 2]>,
}
impl Keys {
    pub fn direction(self) -> [f32; 2] {
        match self.analog {
            Some(v) => v,
            None => sunrise_game_data::move_input::keyboard_direction(
                self.forward,
                self.backward,
                self.left,
                self.right,
            ),
        }
    }
    /// Add a stick's move vector: the direction flags code reading keys
    /// (sprint, vehicles, the server's input packet) see are set past a
    /// third of the travel.
    pub fn with_stick(mut self, stick: Option<[f32; 2]>) -> Self {
        if let Some([left, forward]) = stick {
            self.forward |= forward > 0.3;
            self.backward |= forward < -0.3;
            self.left |= left > 0.3;
            self.right |= left < -0.3;
            self.analog = Some([left, forward]);
        }
        self
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SprintState {
    pub trigger_time: i32,
    pub previous_forward: bool,
    pub previous_shift: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Conditions {
    pub sprinting: bool,
    pub mobility_restricted: bool,
    pub passenger: bool,
    pub vehicle_can_sprint: bool,
    pub vehicle_local: bool,
    pub enough_food: bool,
    pub may_fly: bool,
    pub flying: bool,
    pub shallow_water: bool,
    pub slow_item: bool,
    pub fall_flying: bool,
    pub under_water: bool,
    pub moving_slowly: bool,
    pub swimming: bool,
    pub in_water: bool,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub minor_horizontal_collision: bool,
}
impl Conditions {
    fn possible(self, allow_shallow_water: bool, shallow_water: impl FnOnce(bool) -> bool) -> bool {
        !self.mobility_restricted
            && if self.passenger {
                self.vehicle_can_sprint && self.vehicle_local
            } else {
                self.enough_food || self.may_fly
            }
            && (allow_shallow_water || !shallow_water(self.shallow_water))
    }
    fn can_start(self, forward: bool, shallow_water: impl FnOnce(bool) -> bool) -> bool {
        !self.sprinting
            && forward
            && self.possible(self.flying, shallow_water)
            && !self.slow_item
            && (!self.fall_flying || self.under_water)
            && (!self.moving_slowly || self.under_water)
    }
    fn should_stop(
        self,
        forward: bool,
        shift: bool,
        shallow_water: impl FnOnce(bool) -> bool,
    ) -> bool {
        if self.swimming {
            !self.possible(true, shallow_water)
                || !self.in_water
                || (!forward && !self.on_ground && !shift)
        } else {
            !self.possible(self.flying, shallow_water)
                || !forward
                || (self.horizontal_collision && !self.minor_horizontal_collision)
        }
    }
}
impl SprintState {
    /// `LocalPlayer.aiStep`'s sprint phase.
    /// The default vanilla window is seven ticks; options can select 0..10.
    /// The setter runs for every native request, including repeated true requests.
    pub fn tick<E>(
        &mut self,
        keys: Keys,
        conditions: Conditions,
        window: i32,
        mut set_sprinting: impl FnMut(bool) -> Result<(), E>,
    ) -> Result<(), E> {
        self.tick_with_context(
            keys,
            conditions,
            window,
            &mut (),
            |_, vanilla| vanilla,
            |_, value| set_sprinting(value),
        )
    }

    /// Run each shallow-water query at its own `LocalPlayer.isSprintingPossible` check.
    /// A setter takes effect before a later query in the same tick.
    pub fn tick_with_context<C, E>(
        &mut self,
        keys: Keys,
        conditions: Conditions,
        window: i32,
        context: &mut C,
        mut shallow_water: impl FnMut(&C, bool) -> bool,
        mut set_sprinting: impl FnMut(&mut C, bool) -> Result<(), E>,
    ) -> Result<(), E> {
        if self.trigger_time > 0 {
            self.trigger_time -= 1;
        }
        let old_forward = self.previous_forward;
        let old_shift = self.previous_shift;
        let forward = keys.direction()[1] > 1.0e-5;
        self.previous_forward = forward;
        self.previous_shift = keys.shift;
        if old_shift || (conditions.slow_item && !conditions.passenger) || keys.backward {
            self.trigger_time = 0;
        }
        let mut sprinting = conditions.sprinting;
        if conditions.can_start(forward, |vanilla| shallow_water(context, vanilla)) {
            if !old_forward {
                if self.trigger_time > 0 {
                    set_sprinting(context, true)?;
                    sprinting = true;
                } else {
                    // `Options.sprintWindow` uses `IntRange(0, 10)`.
                    self.trigger_time = window.clamp(0, 10);
                }
            }
            if keys.sprint {
                set_sprinting(context, true)?;
                sprinting = true;
            }
        }
        if sprinting
            && conditions.should_stop(forward, keys.shift, |vanilla| {
                shallow_water(context, vanilla)
            })
        {
            set_sprinting(context, false)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing() -> Conditions {
        Conditions {
            enough_food: true,
            on_ground: true,
            ..Conditions::default()
        }
    }

    fn step(state: &mut SprintState, keys: Keys, sprinting: &mut bool) {
        let current = *sprinting;
        let mut next = current;
        state
            .tick(
                keys,
                Conditions {
                    sprinting: current,
                    ..standing()
                },
                7,
                |value| {
                    next = value;
                    Ok::<(), ()>(())
                },
            )
            .unwrap();
        *sprinting = next;
    }

    /// Vanilla sprint is a state. Releasing the sprint key, including on the
    /// jump tick, keeps it while forward is held; releasing forward stops it.
    #[test]
    fn sprint_survives_releasing_the_key_through_a_jump() {
        let mut state = SprintState::default();
        let mut sprinting = false;
        step(
            &mut state,
            Keys {
                forward: true,
                sprint: true,
                ..Keys::default()
            },
            &mut sprinting,
        );
        assert!(sprinting);
        step(
            &mut state,
            Keys {
                forward: true,
                jump: true,
                ..Keys::default()
            },
            &mut sprinting,
        );
        assert!(
            sprinting,
            "a jump after the sprint key comes up must stay a sprint"
        );
        step(
            &mut state,
            Keys {
                jump: true,
                ..Keys::default()
            },
            &mut sprinting,
        );
        assert!(!sprinting);
    }
}
