use super::*;

/// Equipment accepted by `LivingEntity.canGlideUsing`, including both hands.
/// Armor indices precede main hand (4) and off hand (5).
/// An iterator, so the per-tick glide checks allocate nothing.
pub(super) fn gliding_slots<'a>(
    items: &'a Items,
    armor: &'a [Option<ItemStack>; 4],
    mainhand: Option<&'a ItemStack>,
    offhand: Option<&'a ItemStack>,
) -> impl Iterator<Item = usize> + 'a {
    armor
        .iter()
        .map(Option::as_ref)
        .chain([mainhand, offhand])
        .zip(["head", "chest", "legs", "feet", "mainhand", "offhand"])
        .enumerate()
        .filter_map(move |(i, (stack, slot))| {
            stack
                .filter(|s| horizon_client::item_sync::can_glide_using(s, slot, items))
                .map(|_| i)
        })
}

impl Game<'_> {
    pub(super) fn glider_worn(&self) -> bool {
        gliding_slots(
            self.res.items(),
            &self.armor,
            self.inventory[self.selected].as_ref(),
            self.offhand.as_ref(),
        )
        .next()
        .is_some()
    }

    pub(super) fn wear_glider(&mut self, points: u32) {
        // Every 20th glide tick only, so collecting the candidates here is fine.
        let pieces: Vec<usize> = gliding_slots(
            self.res.items(),
            &self.armor,
            self.inventory[self.selected].as_ref(),
            self.offhand.as_ref(),
        )
        .collect();
        if pieces.is_empty() {
            return;
        }
        let pick = random_source::entity().next_int_bound(pieces.len() as i32);
        let slot = pieces[pick.unwrap_or(0) as usize];
        let stack = match slot {
            4 => self.inventory[self.selected].clone(),
            5 => self.offhand.clone(),
            i => self.armor[i].clone(),
        };
        // `ItemStack.hurtAndBreak` skips a stack that isn't `isDamageableItem` (no `damage`
        // component, say), though it may still glide.
        if let Some(stack) = stack
            && horizon_client::stations::is_damageable(self.res.items(), &stack)
        {
            let loss = self.wear(&stack, points);
            let target = match slot {
                4 => &mut self.inventory[self.selected],
                5 => &mut self.offhand,
                i => &mut self.armor[i],
            };
            if let Some(piece) = target.as_mut() {
                piece.damage += loss;
            }
        }
    }
}
