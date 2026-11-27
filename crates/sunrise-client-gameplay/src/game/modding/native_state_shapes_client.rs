use crate::game::ClientGame;
use mod_api::{BlockShapeKind, LevelRef};
use std::ops::ControlFlow;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::native_state_shapes::Outline;
use sunrise_world::registry::BlockStateId;

impl ClientGame<'_> {
    pub(in crate::game) fn level_visit_block_shape(
        &self,
        level: LevelRef,
        state: BlockStateId,
        pos: [i32; 3],
        kind: BlockShapeKind,
        visit: &mut dyn FnMut([f64; 6]) -> ControlFlow<()>,
    ) -> Option<ControlFlow<()>> {
        (self.native_client_level() == Some(level)).then(|| {
            self.visit_state_shape(
                &self.common.world,
                &self.client.be_anims.shulkers,
                state,
                pos,
                kind,
                visit,
            )
        })
    }

    pub(in crate::game) fn outline_edges_iter(
        &self,
        state: BlockStateId,
        pos: [i32; 3],
    ) -> impl Iterator<Item = [f64; 6]> + '_ {
        let (outline, offset) = self.selected_outline(
            &self.common.world,
            state,
            &self.client.be_anims.shulkers,
            pos,
            true,
        );
        let (owned, cached) = match outline {
            Outline::Published(shape) => (shape.edges(), &[][..]),
            Outline::Declared(_, kind) => (
                Vec::new(),
                self.common
                    .collision
                    .cached_shapes()
                    .context_outline_edges(state, kind)
                    .unwrap_or_default(),
            ),
            Outline::Dynamic(_) => (sunrise_items::blocks::cube_edges(), &[][..]),
            Outline::Default(facts) => (Vec::new(), facts.outline_edges.as_ref()),
        };
        owned
            .into_iter()
            .chain(cached.iter().copied())
            .map(move |edge| {
                std::array::from_fn(|i| {
                    let axis = i % 3;
                    let local = match outline {
                        Outline::Dynamic(b) => b[axis] + edge[i] * (b[axis + 3] - b[axis]),
                        _ => edge[i],
                    };
                    f64::from(pos[axis]) + offset[axis] + local
                })
            })
    }
}

use sunrise_gameplay_common::game::native_state_shapes::NativeStateShapesGameExt1 as _;
