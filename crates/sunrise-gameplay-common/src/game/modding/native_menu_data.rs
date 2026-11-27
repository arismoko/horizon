pub type ContainerPos = (i32, i32, i32);

/// A menu the player opened, including the block entity instances it opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeardMenu {
    pub window: i32,
    pub generation: u64,
    /// The menu type's name; a mount's inventory has none in vanilla.
    pub menu: String,
    /// The block the menu is of.
    pub pos: Option<[i32; 3]>,
    /// Each position and its generation when the menu opened; combined menus hold both halves.
    pub openers: [Option<(ContainerPos, u64)>; 2],
}
