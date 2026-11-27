//! Frozen authoritative requirements mapped to checked menu-owned destinations.
use crate::{crafting::Placement, recipe_book::slot_layout::CheckedSlotLayout};
use mod_api::RecipeBookLayout as L;

#[derive(Clone, Debug)]
pub struct MenuPlacement {
    pub(super) menu: String,
    pub(super) placement: Placement,
    pub(super) inputs: Vec<usize>,
    pub(super) cells: Vec<(usize, usize)>,
    pub(super) clear: Vec<usize>,
    pub(super) virtual_result: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementRefusal {
    NotPlaceable,
    InvalidLayout,
    Unavailable,
    Denied,
}

impl MenuPlacement {
    /// Compile once per table load. Display candidates are never consulted.
    pub fn new(
        menu: String,
        placement: Placement,
        layout: &CheckedSlotLayout,
    ) -> Result<Self, PlacementRefusal> {
        if placement.impossible() || placement.ingredients.is_empty() {
            return Err(PlacementRefusal::NotPlaceable);
        }
        let (width, height, inputs, result, virtual_result) = match layout.declaration() {
            L::Crafting {
                width,
                height,
                slots,
                result,
            } => (
                usize::from(*width),
                usize::from(*height),
                slots.iter().copied().map(usize::from).collect::<Vec<_>>(),
                usize::from(*result),
                layout.computed_result().map(usize::from),
            ),
            L::Furnace { input, result, .. } | L::Stonecutter { input, result } => {
                (1, 1, vec![usize::from(*input)], usize::from(*result), None)
            }
            L::Mod => return Err(PlacementRefusal::InvalidLayout),
            L::Smithing {
                template,
                base,
                addition,
                result,
            } => (
                3,
                1,
                vec![
                    usize::from(*template),
                    usize::from(*base),
                    usize::from(*addition),
                ],
                usize::from(*result),
                None,
            ),
        };
        let (w, h) = placement.shaped.unwrap_or((width, height));
        if w == 0
            || h == 0
            || w > width
            || h > height
            || placement.slots.len() > inputs.len()
            || placement.ingredients.len() > inputs.len()
            || placement.shaped.is_some() && w.checked_mul(h) != Some(placement.slots.len())
            || !matches!(layout.declaration(), L::Crafting { .. })
                && (placement.shaped.is_some() || placement.slots.len() != inputs.len())
        {
            return Err(PlacementRefusal::InvalidLayout);
        }
        Self::check_slot_use(&placement)?;
        let cells = crate::recipe_book::place_cells(width, height, w, h, &placement.slots)
            .into_iter()
            .filter(|(ingredient, _)| *ingredient != -1)
            .map(|(ingredient, cell)| (ingredient as usize, inputs[cell]))
            .collect();
        let mut clear = inputs.clone();
        if virtual_result.is_none() && !matches!(layout.declaration(), L::Crafting { .. }) {
            clear.push(result);
        }
        Ok(Self {
            menu,
            placement,
            inputs,
            cells,
            clear,
            virtual_result,
        })
    }

    /// Every ingredient is used by exactly one slot of the placement.
    fn check_slot_use(placement: &Placement) -> Result<(), PlacementRefusal> {
        let mut seen = vec![false; placement.ingredients.len()];
        for index in &placement.slots {
            if *index == -1 {
                continue;
            }
            let index = usize::try_from(*index).map_err(|_| PlacementRefusal::InvalidLayout)?;
            let used = seen.get_mut(index).ok_or(PlacementRefusal::InvalidLayout)?;
            if *used {
                return Err(PlacementRefusal::InvalidLayout);
            }
            *used = true;
        }
        if seen.iter().any(|used| !*used) {
            return Err(PlacementRefusal::InvalidLayout);
        }
        Ok(())
    }

    /// The requirements of a recipe in a `Mod` layout's menu, with no grid yet: the mod's
    /// `recipe_book_place` part names the grid when it asks for the placement (`in_grid`).
    pub fn for_mod(menu: String, placement: Placement) -> Result<Self, PlacementRefusal> {
        if placement.impossible() || placement.ingredients.is_empty() {
            return Err(PlacementRefusal::NotPlaceable);
        }
        Self::check_slot_use(&placement)?;
        Ok(Self {
            menu,
            placement,
            inputs: Vec::new(),
            cells: Vec::new(),
            clear: Vec::new(),
            virtual_result: None,
        })
    }

    /// This recipe placed in the grid a mod names, as `ServerPlaceRecipe.placeRecipe` takes it:
    /// `grid` holds the menu slot of each cell, row by row in a `width` by `height` grid, and the
    /// slots cleared first are `clear` and the grid's own. A recipe never reaches a cell past the
    /// last slot given. The reason of a refusal is for the mod.
    pub fn in_grid(
        &self,
        width: u16,
        height: u16,
        grid: &[u16],
        clear: &[u16],
        menu_size: usize,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 || grid.is_empty() {
            return Err("the grid needs a size and at least one slot".into());
        }
        let (width, height) = (usize::from(width), usize::from(height));
        if let Some(slot) = grid
            .iter()
            .chain(clear)
            .find(|slot| usize::from(**slot) >= menu_size)
        {
            return Err(format!("slot {slot} is outside the menu"));
        }
        let mut inputs: Vec<usize> = Vec::with_capacity(grid.len());
        for &slot in grid {
            let slot = usize::from(slot);
            if inputs.contains(&slot) {
                return Err(format!("slot {slot} is in the grid twice"));
            }
            inputs.push(slot);
        }
        let mut cleared: Vec<usize> = Vec::with_capacity(clear.len() + inputs.len());
        for slot in clear
            .iter()
            .copied()
            .map(usize::from)
            .chain(inputs.iter().copied())
        {
            if !cleared.contains(&slot) {
                cleared.push(slot);
            }
        }
        let (w, h) = self.placement.shaped.unwrap_or((width, height));
        if w > width || h > height {
            return Err("the recipe is larger than the grid".into());
        }
        let mut cells = Vec::new();
        for (ingredient, cell) in
            crate::recipe_book::place_cells(width, height, w, h, &self.placement.slots)
        {
            if ingredient == -1 {
                continue;
            }
            let slot = *inputs
                .get(cell)
                .ok_or("the recipe needs a cell the grid has no slot for")?;
            cells.push((ingredient as usize, slot));
        }
        let mut reached = vec![false; self.placement.ingredients.len()];
        for (ingredient, _) in &cells {
            reached[*ingredient] = true;
        }
        if reached.contains(&false) {
            return Err("the grid has fewer cells than the recipe has ingredients".into());
        }
        Ok(Self {
            menu: self.menu.clone(),
            placement: self.placement.clone(),
            inputs,
            cells,
            clear: cleared,
            virtual_result: None,
        })
    }
}
