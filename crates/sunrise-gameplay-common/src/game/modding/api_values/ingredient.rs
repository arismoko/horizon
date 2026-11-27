//! Ingredient conversions retain the binding to the same defining mod and preserve slot displays.
use super::*;
use mod_api::item::crafting::Shape;

// Both conversions keep shape and child order without using the call stack.
fn convert_tree<T, U, E>(
    root: &T,
    children: impl Fn(&T) -> Option<&(Shape, Vec<T>)>,
    mut convert: impl FnMut(&T, Option<(Shape, Vec<U>)>) -> Result<U, E>,
) -> Result<U, E> {
    let Some((shape, kids)) = children(root) else {
        return convert(root, None);
    };
    let mut pending = vec![(root, shape, kids.iter(), Vec::with_capacity(kids.len()))];
    loop {
        let frame = pending
            .last_mut()
            .expect("unfinished ingredient conversion");
        if let Some(child) = frame.2.next() {
            if let Some((shape, kids)) = children(child) {
                pending.push((child, shape, kids.iter(), Vec::with_capacity(kids.len())));
            } else {
                frame.3.push(convert(child, None)?);
            }
        } else {
            let (node, shape, _, kids) = pending.pop().expect("completed ingredient conversion");
            let converted = convert(node, Some((shape.clone(), kids)))?;
            if let Some(parent) = pending.last_mut() {
                parent.3.push(converted);
            } else {
                return Ok(converted);
            }
        }
    }
}

impl TryIntoApi for sunrise_sim::crafting::Ingredient {
    type Api = mod_api::Ingredient;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        convert_tree(
            self,
            |node| node.children.as_ref(),
            |node, children| {
                Ok(mod_api::Ingredient {
                    items: node.items.clone(),
                    order: node.order.clone(),
                    custom: node.custom.clone(),
                    rule: node.rule.clone().map(Box::new),
                    display: node.display.clone().map(Box::new),
                    children,
                })
            },
        )
    }
}
impl TryIntoEngine for mod_api::Ingredient {
    type Engine = sunrise_sim::crafting::Ingredient;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        convert_tree(
            self,
            |node| node.children.as_ref(),
            |node, children| {
                let mut ingredient = sunrise_sim::crafting::Ingredient {
                    compiled: None,
                    component_defaults: items.component_defaults.clone(),
                    shown: node.display.as_ref().and_then(|display| {
                        sunrise_sim::crafting::Ingredient::resolve_display(display, items)
                    }),
                    items: node.items.clone(),
                    order: node.order.clone(),
                    tag: None,
                    custom: node.custom.clone(),
                    rule: node.rule.as_deref().cloned(),
                    display: node.display.as_deref().cloned(),
                    children,
                };
                ingredient
                    .prepare_components(items)
                    .ok_or(StackWriteError::UnknownItem)?;
                Ok(ingredient)
            },
        )
    }
}
