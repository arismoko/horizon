use super::*;

impl ItemEntity {
    pub fn environment_tick(
        &mut self,
        mut world: &World,
        facts: &BlockFacts,
        registry: &horizon_world::registry::BlockRegistry,
        can_be_hurt_by: &dyn Fn(&ItemStack, &str) -> bool,
    ) -> ItemEnvironmentTick {
        self.environment_tick_inner(&mut world, facts, registry, can_be_hurt_by, |_, _| false)
    }
    /// As `environment_tick`, allowing the caller to decide each actual damage attempt.
    pub fn environment_tick_with_hook(
        &mut self,
        mut world: &mut World,
        facts: &BlockFacts,
        registry: &horizon_world::registry::BlockRegistry,
        can_be_hurt_by: &dyn Fn(&ItemStack, &str) -> bool,
        mut invulnerable: impl FnMut(&mut World, &str) -> bool,
    ) -> ItemEnvironmentTick {
        self.environment_tick_inner(
            &mut world,
            facts,
            registry,
            can_be_hurt_by,
            |world, damage| invulnerable(world, damage),
        )
    }
    fn environment_tick_inner<W: std::ops::Deref<Target = World>>(
        &mut self,
        world: &mut W,
        facts: &BlockFacts,
        registry: &horizon_world::registry::BlockRegistry,
        can_be_hurt_by: &dyn Fn(&ItemStack, &str) -> bool,
        mut invulnerable: impl FnMut(&mut W, &str) -> bool,
    ) -> ItemEnvironmentTick {
        let at = world.block(
            self.position.x.floor() as i32,
            (self.position.y + 0.1).floor() as i32,
            self.position.z.floor() as i32,
        );
        let fluid = facts.get(at).fluid;
        let fire_immune = !can_be_hurt_by(&self.stack, "minecraft:in_fire");
        let mut spilled = Vec::new();
        if fluid == Fluid::Water || fire_immune {
            self.fire_ticks = 0;
        }
        if self.fire_ticks > 0 {
            if self.fire_ticks % 20 == 0
                && fluid != Fluid::Lava
                && can_be_hurt_by(&self.stack, "minecraft:on_fire")
                && !invulnerable(world, "minecraft:on_fire")
            {
                spilled.extend(self.hurt(1).unwrap_or_default());
            }
            self.fire_ticks -= 1;
        }
        let mut lava_hurt = false;
        // LavaCauldronBlock.entityInside applies LAVA_IGNITE and then lavaHurt.
        // A cauldron is not a lava fluid, and removed items get no block effects.
        if self.health > 0 && !fire_immune {
            let contacts = usize::from(fluid == Fluid::Lava)
                + lava_cauldron_contacts(world, registry, self.bounds());
            if contacts > 0 {
                self.fire_ticks = self.fire_ticks.max(300);
            }
            // StepBasedCollector coalesces ignition, but keeps each block's
            // runAfter(lavaHurt) callback. Stop once an item is destroyed.
            for _ in 0..contacts {
                if self.health <= 0 {
                    break;
                }
                if can_be_hurt_by(&self.stack, "minecraft:lava")
                    && !invulnerable(world, "minecraft:lava")
                {
                    spilled.extend(self.hurt(4).unwrap_or_default());
                    lava_hurt = true;
                }
            }
        }
        let fire_damage = crate::motion::environment::fire_contact(world, registry, self.bounds());
        if fire_damage > 0.0 && !fire_immune {
            self.fire_ticks = self.fire_ticks.max(160);
            if can_be_hurt_by(&self.stack, "minecraft:in_fire")
                && !invulnerable(world, "minecraft:in_fire")
            {
                spilled.extend(self.hurt(fire_damage as i32).unwrap_or_default());
            }
        }
        // CactusBlock.entityInside: `entity.hurt(cactus, 1.0)`. An item has no
        // hurt cooldown, so it loses 1 health every tick it touches a cactus
        // (5 health: gone in 5 ticks). Fire resistance does not help;
        // `ItemStack.canBeHurtBy` only skips it for a damage_resistant stack.
        if crate::motion::environment::cactus_contact(world, registry, self.bounds())
            && can_be_hurt_by(&self.stack, "minecraft:cactus")
            && !invulnerable(world, "minecraft:cactus")
        {
            spilled.extend(self.hurt(1).unwrap_or_default());
        }
        ItemEnvironmentTick {
            alive: self.health > 0,
            lava_hurt,
            spilled,
        }
    }
}
