//! Code-free projection for the recipe book's remote display and craftable filter.
use mod_api::item::crafting::{Ingredient, IngredientRule, Shape};
use mod_api::{
    PlacementInfo, RecipeBookCategory, RecipeBookDisplay, RecipeBookDisplayEntry, RecipeBookTypeKey,
};
use std::collections::HashSet;
use sunrise_core::nbt::Compound;

#[derive(mod_api::Wire)]
pub(super) struct Category {
    id: String,
    recipe_type: String,
    icon: String,
    secondary_icon: Option<String>,
    target_menu: String,
    book: RecipeBookTypeKey,
    search_group: String,
    displays: Vec<Entry>,
}

#[derive(mod_api::Wire)]
struct Entry {
    recipe: String,
    displays: Vec<RecipeBookDisplay>,
    placement: Option<Cells>,
}

#[derive(mod_api::Wire)]
struct Cells {
    cells: Vec<Option<StaticIngredient>>,
    shape: Option<[u16; 2]>,
}

#[derive(mod_api::Wire)]
struct StaticIngredient {
    items: HashSet<String>,
    order: Vec<String>,
    children: Option<(Shape, Vec<Self>)>,
    rule: Option<Box<IngredientRule>>,
    display: Option<Box<Compound>>,
}

fn has_custom(value: &Ingredient) -> bool {
    let mut pending = vec![value];
    while let Some(ingredient) = pending.pop() {
        if ingredient.custom.is_some() {
            return true;
        }
        if let Some((_, children)) = &ingredient.children {
            pending.extend(children);
        }
    }
    false
}

fn full_order(items: &HashSet<String>, existing: &[String]) -> Vec<String> {
    let mut seen = HashSet::with_capacity(items.len());
    let mut order = Vec::with_capacity(items.len());
    for item in existing {
        if let Some(known) = items.get(item)
            && seen.insert(known.as_str())
        {
            order.push(known.clone());
        }
    }
    if order.len() < items.len() {
        let mut missing: Vec<&str> = items
            .iter()
            .map(String::as_str)
            .filter(|item| !seen.contains(item))
            .collect();
        missing.sort_unstable();
        order.extend(missing.into_iter().map(str::to_owned));
    }
    order
}

fn write_projected_ingredient(out: &mut mod_api::wire::Writer, value: &Ingredient) {
    if out.error().is_some() {
        return;
    }
    if has_custom(value) {
        let order = full_order(&value.items, &value.order);
        out.record(|out| {
            out.put(&value.items);
            out.put(&order);
            out.put(&0u8);
            out.put(&0u8);
            out.put(&value.display);
        });
        return;
    }
    write_static_ingredient(out, value);
}

fn write_static_ingredient(out: &mut mod_api::wire::Writer, value: &Ingredient) {
    if out.error().is_some() {
        return;
    }
    out.record(|out| {
        out.put(&value.items);
        out.put(&value.order);
        match &value.children {
            None => out.put(&0u8),
            Some((shape, children)) => {
                out.put(&1u8);
                out.put(shape);
                out.put_len(children.len());
                for child in children {
                    if out.error().is_some() {
                        return;
                    }
                    write_static_ingredient(out, child);
                }
            }
        }
        out.put(&value.rule);
        out.put(&value.display);
    });
}

fn write_cells(out: &mut mod_api::wire::Writer, info: &PlacementInfo) {
    if out.error().is_some() {
        return;
    }
    // Match PlacementInfo.createFromOptionals / Ingredient.isEmpty: an empty
    // candidate set collapses placement to NOT_PLACEABLE while keeping displays.
    let collapsed = info
        .cells
        .iter()
        .flatten()
        .any(|ingredient| ingredient.items.is_empty());
    out.record(|out| {
        if collapsed {
            out.put_len(0);
            out.put(&0u8);
            return;
        }
        out.put_len(info.cells.len());
        for cell in &info.cells {
            if out.error().is_some() {
                return;
            }
            match cell {
                None => out.put(&0u8),
                Some(ingredient) => {
                    out.put(&1u8);
                    write_projected_ingredient(out, ingredient);
                }
            }
        }
        out.put(&info.shape);
    });
}

fn write_category_header(out: &mut mod_api::wire::Writer, category: &RecipeBookCategory) {
    out.put(&category.id);
    out.put(&category.recipe_type);
    out.put(&category.icon);
    out.put(&category.secondary_icon);
    out.put(&category.target_menu);
    out.put(&category.book);
    out.put(&category.search_group);
}

fn write_entry(
    out: &mut mod_api::wire::Writer,
    recipe: &str,
    displays: &[RecipeBookDisplay],
    placement: Option<&PlacementInfo>,
) {
    if out.error().is_some() {
        return;
    }
    out.record(|out| {
        out.put(recipe);
        out.put(displays);
        match placement {
            None => out.put(&0u8),
            Some(info) => {
                out.put(&1u8);
                write_cells(out, info);
            }
        }
    });
}

pub(super) fn write_categories(
    out: &mut mod_api::wire::Writer,
    declarations: &[RecipeBookCategory],
) {
    out.put_len(declarations.len());
    for category in declarations {
        if out.error().is_some() {
            return;
        }
        out.record(|out| {
            write_category_header(out, category);
            out.put_len(category.displays.len());
            for entry in &category.displays {
                if out.error().is_some() {
                    return;
                }
                write_entry(
                    out,
                    &entry.recipe,
                    &entry.displays,
                    entry.placement.as_ref(),
                );
            }
        });
    }
}

pub(super) fn write_borrowed_categories(
    out: &mut mod_api::wire::Writer,
    declarations: &[super::BorrowedCategory<'_>],
) {
    out.put_len(declarations.len());
    for borrowed in declarations {
        if out.error().is_some() {
            return;
        }
        out.record(|out| {
            write_category_header(out, borrowed.category);
            out.put_len(borrowed.displays.len());
            for entry in &borrowed.displays {
                if out.error().is_some() {
                    return;
                }
                write_entry(out, entry.recipe, &entry.displays, entry.placement);
            }
        });
    }
}

impl From<StaticIngredient> for Ingredient {
    fn from(mut value: StaticIngredient) -> Self {
        let leaf = |node: StaticIngredient| Self {
            items: node.items,
            order: node.order,
            custom: None,
            children: None,
            rule: node.rule,
            display: node.display,
        };
        let Some((root_shape, root_kids)) = value.children.take() else {
            return leaf(value);
        };
        let mut stack: Vec<(
            StaticIngredient,
            Shape,
            std::vec::IntoIter<StaticIngredient>,
            Vec<Self>,
        )> = vec![(value, root_shape, root_kids.into_iter(), Vec::new())];
        loop {
            let next = stack.last_mut().expect("non-empty decode stack").2.next();
            if let Some(mut child) = next {
                if let Some((shape, kids)) = child.children.take() {
                    let cap = kids.len();
                    stack.push((child, shape, kids.into_iter(), Vec::with_capacity(cap)));
                } else {
                    stack
                        .last_mut()
                        .expect("non-empty decode stack")
                        .3
                        .push(leaf(child));
                }
            } else {
                let (node, shape, _, children) = stack.pop().expect("non-empty decode stack");
                let mut converted = leaf(node);
                converted.children = Some((shape, children));
                if let Some(parent) = stack.last_mut() {
                    parent.3.push(converted);
                } else {
                    return converted;
                }
            }
        }
    }
}

impl From<Category> for RecipeBookCategory {
    fn from(value: Category) -> Self {
        Self {
            id: value.id,
            recipe_type: value.recipe_type,
            icon: value.icon,
            secondary_icon: value.secondary_icon,
            target_menu: value.target_menu,
            book: value.book,
            search_group: value.search_group,
            displays: value
                .displays
                .into_iter()
                .map(|entry| RecipeBookDisplayEntry {
                    recipe: entry.recipe,
                    displays: entry.displays,
                    placement: entry.placement.map(|info| PlacementInfo {
                        cells: info
                            .cells
                            .into_iter()
                            .map(|cell| cell.map(Into::into))
                            .collect(),
                        shape: info.shape,
                    }),
                })
                .collect(),
        }
    }
}
