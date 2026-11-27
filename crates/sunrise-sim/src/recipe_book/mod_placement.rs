//! Authoritative placement data crosses one checked load boundary, never a draw path.
#[cfg(test)]
#[path = "mod_placement_tests.rs"]
mod tests;
use super::{DisplayLoadError, DisplayResources, RecipeBookCategory};
use crate::{
    crafting::{Ingredient, Placement},
    menu::place::custom_plan::{MenuPlacement, PlacementRefusal},
};
use mod_api::{PlacementInfo, RecipeBookDisplayEntry, item::crafting::Shape};

fn invalid(reason: impl Into<String>) -> DisplayLoadError {
    DisplayLoadError::InvalidPlacement(reason.into())
}

pub(super) fn validate_batch(
    values: &[RecipeBookCategory],
    resources: &dyn DisplayResources,
) -> Result<(), DisplayLoadError> {
    for category in values {
        for entry in &category.displays {
            let Some(info) = &entry.placement else {
                continue;
            };
            validate(info, resources).map_err(|reason| {
                invalid(format!("recipe {} placement: {reason}", entry.recipe))
            })?;
        }
    }
    Ok(())
}
fn validate(info: &PlacementInfo, resources: &dyn DisplayResources) -> Result<(), String> {
    if let Some([w, h]) = info.shape
        && (w == 0
            || h == 0
            || usize::from(w).checked_mul(usize::from(h)) != Some(info.cells.len()))
    {
        return Err("shape dimensions disagree with cells".into());
    }
    for root in info.cells.iter().flatten() {
        validate_ingredient(root, resources)?;
    }
    Ok(())
}

fn validate_node_header<'a>(
    ingredient: &'a mod_api::Ingredient,
    resources: &dyn DisplayResources,
) -> Result<(std::slice::Iter<'a, mod_api::Ingredient>, bool), String> {
    if ingredient.items.iter().any(|id| !resources.item(id)) {
        return Err("unknown ingredient item".into());
    }
    if ingredient.custom.is_some() && (ingredient.children.is_some() || ingredient.rule.is_some()) {
        return Err("custom ingredient cannot also have a child shape or built-in rule".into());
    }
    let children = match &ingredient.children {
        Some((shape, children)) => {
            let valid = match shape {
                Shape::Compound | Shape::Intersection => !children.is_empty(),
                Shape::Difference => children.len() == 2,
                Shape::CustomDisplay => children.len() == 1,
            };
            if !valid {
                return Err("ingredient shape has invalid children".into());
            }
            children.iter()
        }
        None => [].iter(),
    };
    let simple = ingredient.rule.is_none() && ingredient.custom.is_none();
    Ok((children, simple))
}

fn validate_ingredient(
    root: &mod_api::Ingredient,
    resources: &dyn DisplayResources,
) -> Result<(), String> {
    let (children, simple) = validate_node_header(root, resources)?;
    let mut stack = vec![(root, children, simple)];
    loop {
        let next = stack.last_mut().expect("non-empty validate stack").1.next();
        if let Some(child) = next {
            let (children, simple) = validate_node_header(child, resources)?;
            stack.push((child, children, simple));
        } else {
            let (ingredient, _, simple) = stack.pop().expect("non-empty validate stack");
            let ordered: std::collections::HashSet<&String> = ingredient.order.iter().collect();
            if ordered.len() != ingredient.order.len()
                || !ordered.iter().all(|id| ingredient.items.contains(*id))
                || (simple && ordered.len() != ingredient.items.len())
            {
                return Err("ingredient holder order disagrees with its items".into());
            }
            if let Some(parent) = stack.last_mut() {
                parent.2 &= simple;
            } else {
                return Ok(());
            }
        }
    }
}

pub(super) fn compile(
    owner: &str,
    menu: &str,
    entry: &RecipeBookDisplayEntry,
    layout: &super::super::slot_layout::CheckedSlotLayout,
) -> Result<Option<MenuPlacement>, DisplayLoadError> {
    let info = entry.placement.as_ref().ok_or_else(|| {
        invalid(format!(
            "mod {owner} recipe {}: menu-bound recipe type must supply PlacementInfo",
            entry.recipe
        ))
    })?;
    let cells = info
        .cells
        .iter()
        .map(|cell| cell.as_ref().map(convert))
        .collect();
    let placement = Placement::of(
        cells,
        info.shape.map(|[w, h]| (usize::from(w), usize::from(h))),
    );
    let compiled = match layout.declaration() {
        mod_api::RecipeBookLayout::Mod => MenuPlacement::for_mod(menu.into(), placement),
        _ => MenuPlacement::new(menu.into(), placement, layout),
    };
    match compiled {
        Ok(plan) => Ok(Some(plan)),
        Err(PlacementRefusal::NotPlaceable) => Ok(None),
        Err(error) => Err(invalid(format!(
            "mod {owner} recipe {} placement: {error:?}",
            entry.recipe
        ))),
    }
}
// Only called after validate_batch accepted the native declaration.
fn convert(value: &mod_api::Ingredient) -> Ingredient {
    let convert_leaf = |node: &mod_api::Ingredient| Ingredient {
        items: node.items.clone(),
        order: node.order.clone(),
        tag: None,
        custom: node.custom.clone(),
        rule: None,
        compiled: None,
        display: node.display.as_deref().cloned(),
        component_defaults: Default::default(),
        shown: None,
        children: None,
    };
    let Some((root_shape, root_kids)) = &value.children else {
        return convert_leaf(value);
    };
    let mut stack: Vec<(
        &mod_api::Ingredient,
        Shape,
        std::slice::Iter<'_, mod_api::Ingredient>,
        Vec<Ingredient>,
    )> = vec![(
        value,
        root_shape.clone(),
        root_kids.iter(),
        Vec::with_capacity(root_kids.len()),
    )];
    loop {
        let next = stack.last_mut().expect("non-empty convert stack").2.next();
        if let Some(child) = next {
            if let Some((shape, kids)) = &child.children {
                stack.push((
                    child,
                    shape.clone(),
                    kids.iter(),
                    Vec::with_capacity(kids.len()),
                ));
            } else {
                stack
                    .last_mut()
                    .expect("non-empty convert stack")
                    .3
                    .push(convert_leaf(child));
            }
        } else {
            let (node, shape, _, children) = stack.pop().expect("non-empty convert stack");
            let mut converted = convert_leaf(node);
            converted.children = Some((shape, children));
            if let Some(parent) = stack.last_mut() {
                parent.3.push(converted);
            } else {
                return converted;
            }
        }
    }
}
