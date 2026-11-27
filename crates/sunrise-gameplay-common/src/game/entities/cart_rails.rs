//! The track under a minecart: `BaseRailBlock.isRail` and the shape a cart reads,
//! which the rail's parts may answer (`rail_direction`).
use super::minecarts::{Rail, Shape};
use mod_api::RailShape;
use sunrise_core::position::BlockPos;
use sunrise_world::{level::classes::Class, registry::BlockStateId};

pub trait CartRailsGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The candidate selected by `getCurrentBlockPosOrRailBelow` only needs the tag.
    fn tagged_rail(&self, state: BlockStateId) -> bool {
        let name = self.res.facts().get(state).name.as_ref();
        self.items()
            .block_tags
            .get("minecraft:rails")
            .is_some_and(|rails| rails.contains(name))
    }

    /// `BaseRailBlock.isRail`: the rail tag and the base rail block class.
    fn is_rail(&self, state: BlockStateId) -> bool {
        self.tagged_rail(state) && self.res.behaviours().data(state).is(Class::BaseRail)
    }

    /// The rail at (x, y, z) as `cart` (none when the rail is read for itself) sees it: a block
    /// in `#minecraft:rails` built on `BaseRailBlock` (`BaseRailBlock.isRail`), with the shape
    /// its parts answer (`IBaseRailBlockExtension.getRailDirection`).
    fn rail(&self, cart: Option<u32>, x: i32, y: i32, z: i32) -> Option<Rail> {
        let state = self.world.block(x, y, z);
        if !self.is_rail(state) {
            return None;
        }
        let name = self.res.facts().get(state).name.clone();
        let s = self.pack.assets.registry.state(state)?;
        let text = s.properties.get("shape").map_or("", String::as_str);
        let powered = s.properties.get("powered").is_some_and(|v| v == "true");
        let shape = self.cart_rail_direction(cart, state, BlockPos::new(x, y, z), text);
        Some(Rail {
            name,
            shape,
            powered,
        })
    }

    /// The shape named `text` of the rail `state` at `pos`, after the parts of the state.
    fn cart_rail_direction(
        &self,
        cart: Option<u32>,
        state: sunrise_world::registry::BlockStateId,
        pos: BlockPos,
        text: &str,
    ) -> Shape {
        let vanilla = Shape::parse(text);
        if !super::api_values::block_state::block_listens(mod_api::HookId::RailDirection, state) {
            return vanilla;
        }
        self.side_cart_rail_direction(cart, state, pos, text, vanilla)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> CartRailsGameExt1<'__sg> for G {}

pub fn dispatch_cart_rail_direction(
    side: mod_api::LevelSide<'_>,
    dim: sunrise_core::dimension::Dimension,
    state: mod_api::BlockState,
    pos: BlockPos,
    cart: Option<mod_api::EntityRef>,
    text: &str,
) -> Shape {
    let mut shape = RailShape::from_name(text).unwrap_or(RailShape::NorthSouth);
    mod_api::dispatch_rail_direction(side, dim, state, pos, cart, &mut shape);
    Shape::parse(shape.name())
}

use crate::game::side_ops::mobs_rest::MobsRestSideGameExt as _;

use crate::game::tables::GameItems as _;
