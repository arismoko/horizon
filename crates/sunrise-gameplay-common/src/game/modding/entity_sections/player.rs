//! File and tell each player move before `Entity.move` resumes.

use super::{EntitySections, PlayerMoveState, Scope, SectionWriter, tell_live};
use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use crate::game::native_lifecycle_common::NativeLifecycle;
use mod_api::{EntityRef, EntityResidency, PlayerRef};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use sunrise_core::geometry::Vec3;
use sunrise_sim::motion::{
    PlayerMotion,
    section_writes::{PositionObserver, PositionObserverScope},
};

#[derive(Clone, Copy)]
pub struct PlayerSectionScope {
    scope: Scope,
    id: u32,
    body_id: i32,
    local_player: Option<PlayerRef>,
    residency: EntityResidency,
    server: bool,
}

#[derive(Default)]
pub(in crate::game) struct PlayerSectionObserver {
    active: Cell<Option<PlayerSectionScope>>,
    sections: RefCell<EntitySections>,
}

impl PositionObserver for PlayerSectionObserver {
    fn position_written(&self, player: &PlayerMotion, from: Vec3) {
        let Some(active) = self
            .active
            .get()
            .filter(|active| active.body_id == player.entity_id)
        else {
            return;
        };
        let to = player.position.components();
        self.sections
            .borrow_mut()
            .moved(active.scope, active.id, Some(from.components()), to);
        tell_live(
            &self.sections,
            SectionWriter {
                entity: EntityRef::new(active.scope.epoch, active.id),
                residency: active.residency,
                position: to,
                player: Some(PlayerMoveState {
                    local_player: active.local_player,
                    on_ground: player.ground.on_ground,
                    velocity: player.velocity.components(),
                }),
            },
            active.server,
        );
    }
}

/// Return the table even if the move fails after committing a position.
pub struct PlayerSectionPass<'a> {
    _motion: PositionObserverScope,
    observer: &'a PlayerSectionObserver,
    sections: &'a mut EntitySections,
}

impl Drop for PlayerSectionPass<'_> {
    fn drop(&mut self) {
        self.observer.active.take();
        std::mem::swap(self.sections, &mut self.observer.sections.borrow_mut());
    }
}

impl PlayerSectionScope {
    pub fn begin(self, lifecycle: &mut NativeLifecycle) -> PlayerSectionPass<'_> {
        let observer = lifecycle
            .player_section_observer
            .get_or_insert_with(Default::default);
        observer.active.set(Some(self));
        let sections = &mut lifecycle.entity_sections;
        std::mem::swap(sections, &mut observer.sections.borrow_mut());
        PlayerSectionPass {
            _motion: PositionObserverScope::install(Rc::clone(observer) as Rc<dyn PositionObserver>),
            observer,
            sections,
        }
    }
}

pub trait PlayerGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    fn player_section_scope(&self) -> Option<PlayerSectionScope> {
        let scope = self.entity_section_scope()?;
        let id = u32::try_from(self.own_entity_id())
            .ok()
            .filter(|id| *id != 0)?;
        let entity = EntityRef::new(scope.epoch, id);
        Some(PlayerSectionScope {
            scope,
            id,
            body_id: self.player.as_ref()?.entity_id,
            local_player: self.local_player(),
            residency: self.entity_residency(entity),
            server: self.is_logical_server(),
        })
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> PlayerGameExt1<'__sg> for G {}

use crate::game::entity_sections::EntitySectionsGameExt1 as _;

use crate::game::player_identity::PlayerIdentityGameExt1 as _;
