//! A click that uses the held item at nothing or at an entity (`Minecraft.startUseItem`,
//! `Player.interactOn`), the context a block's use sees, and the mod hooks that cancel a placement
//! or an emptied bucket.
/// The index in `list` of the entity an interact packet's `target` names,
/// by its server id (`Level.getEntity(id)`), or None for an id this level
/// does not hold: `handleInteract` finds nothing then either.
pub fn named_entity<T>(list: &[T], target: i32, id: impl Fn(&T) -> u32) -> Option<usize> {
    list.iter().position(|e| id(e) as i32 == target)
}
