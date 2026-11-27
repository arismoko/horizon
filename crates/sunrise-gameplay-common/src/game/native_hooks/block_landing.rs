use super::*;

impl HitLevel<'_> {
    /// `Block.fallOn`: entity `entity` of type `kind` landed on `state` at `pos` after falling
    /// `distance` blocks. Writable distance and multiplier feed `causeFallDamage` before
    /// sounds and hurt. A zero multiplier prevents both. A creative or
    /// fall-immune entity keeps its immunity. Players and mobs
    /// land here; dropped items also land with zero multiplier. Generated dispatch stages
    /// every requested API LandingWrite before committing the fall inputs or the engine write
    /// queue; an invalid state refuses the whole callback's output.
    pub fn native_fall_on(&mut self, call: super::fire::FallOnCall<'_>) {
        self.fire_fall_on(call);
    }
    /// `Block.bounceOn`: entity `entity` of type `kind` bounced off `state` at `pos` and fell
    /// `fall_distance` blocks. `movement` is the movement the bounce leaves it, before
    /// vanilla sets it: change it to change what the entity keeps. The entity types that simulate a
    /// bounce on the server use this path.
    pub(in crate::game) fn native_bounce_on(&mut self, call: super::fire::BounceOnCall<'_>) {
        self.fire_bounce_on(call);
    }
}
