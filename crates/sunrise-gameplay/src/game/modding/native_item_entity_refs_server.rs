//! Server-side dropped-item handle reads and writes for native mods.
use super::commands::Target;
use mod_api::{EntityRef, ItemEntityRef, ItemStackView};
use sunrise_gameplay_common::game::api_values::{project_stack, stack_read_metadata};
use sunrise_items::items::ItemStack;
use sunrise_sim::lan::playerdata::{ints_uuid, uuid_ints};

impl crate::game::ServerGame<'_> {
    /// Resolve the stable item identity, reusing the caller's known slot while it still matches.
    pub(in crate::game) fn native_item_entity_index(
        &self,
        item: ItemEntityRef,
        hint: Option<usize>,
    ) -> Option<usize> {
        let (Target::Item(_), Some(location)) =
            self.native_entity_target_with_location(item.entity())?
        else {
            return None;
        };
        hint.filter(|&i| {
            self.common
                .dropped
                .get(i)
                .is_some_and(|row| row.join == location.join)
        })
        .or_else(|| self.common.dropped.locate(location.join))
    }

    pub(in crate::game) fn server_item_entity(&self, entity: EntityRef) -> Option<ItemEntityRef> {
        let item = ItemEntityRef::new(entity);
        self.native_item_entity_index(item, None).map(|_| item)
    }

    /// Lend `ItemEntity.getItem`. Encode components only when the reader projects the view.
    pub(in crate::game) fn item_entity_stack(
        &self,
        item: ItemEntityRef,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()> {
        let stack = &self
            .common
            .dropped
            .get(self.native_item_entity_index(item, None)?)?
            .stack;
        let metadata = stack_read_metadata(stack).ok()?;
        let project = |metadata| project_stack(stack, metadata).ok();
        read(ItemStackView::resident(metadata, &project));
        Some(())
    }

    /// Writes a stack using `ItemEntity.setItem`; None clears it.
    pub(in crate::game) fn item_entity_set_stack(
        &mut self,
        item: ItemEntityRef,
        stack: Option<ItemStack>,
    ) -> bool {
        let Some(index) = self.native_item_entity_index(item, None) else {
            return false;
        };
        self.journal_set_item_entity_at(index, stack);
        true
    }

    /// The signed pickup delay; the infinite sentinel reads as vanilla's 32767.
    pub(in crate::game) fn item_entity_pickup_delay(&self, item: ItemEntityRef) -> Option<i32> {
        Some(self.common.dropped[self.native_item_entity_index(item, None)?].pickup_delay)
    }

    /// `ItemEntity.setPickUpDelay`.
    pub(in crate::game) fn item_entity_set_pickup_delay(
        &mut self,
        item: ItemEntityRef,
        ticks: i32,
    ) -> bool {
        let Some(index) = self.native_item_entity_index(item, None) else {
            return false;
        };
        self.journal_set_item_pickup_delay(index, ticks);
        true
    }

    /// `ItemEntity.setDefaultPickUpDelay`: ten ticks.
    pub(in crate::game) fn item_entity_set_default_pickup_delay(
        &mut self,
        item: ItemEntityRef,
    ) -> bool {
        self.item_entity_set_pickup_delay(item, 10)
    }

    /// `ItemEntity.setNoPickUpDelay`.
    pub(in crate::game) fn item_entity_set_no_pickup_delay(&mut self, item: ItemEntityRef) -> bool {
        self.item_entity_set_pickup_delay(item, 0)
    }

    /// `ItemEntity.setNeverPickUp`: the infinite-delay sentinel 32767.
    pub(in crate::game) fn item_entity_set_never_pickup(&mut self, item: ItemEntityRef) -> bool {
        self.item_entity_set_pickup_delay(
            item,
            sunrise_sim::entities::ItemEntity::INFINITE_PICKUP_DELAY,
        )
    }

    /// `ItemEntity.getAge`, including negative lifetime sentinels.
    pub(in crate::game) fn item_entity_age(&self, item: ItemEntityRef) -> Option<i32> {
        Some(
            self.common
                .dropped
                .get(self.native_item_entity_index(item, None)?)?
                .age,
        )
    }

    /// The UUID set by `ItemEntity.setTarget`, limiting pickup to that player.
    pub(in crate::game) fn item_entity_target(&self, item: ItemEntityRef) -> Option<u128> {
        self.common
            .dropped
            .get(self.native_item_entity_index(item, None)?)?
            .target
            .map(ints_uuid)
    }

    /// `ItemEntity.setTarget`; None allows any player to pick it up.
    pub(in crate::game) fn item_entity_set_target(
        &mut self,
        item: ItemEntityRef,
        target: Option<u128>,
    ) -> bool {
        let Some(index) = self.native_item_entity_index(item, None) else {
            return false;
        };
        self.journal_set_item_target(index, target.map(uuid_ints));
        true
    }

    /// `ItemEntity.getOwner`: the thrower as a resolved entity, not the pickup target.
    pub(in crate::game) fn item_entity_owner(&self, item: ItemEntityRef) -> Option<EntityRef> {
        let uuid = self
            .common
            .dropped
            .get(self.native_item_entity_index(item, None)?)?
            .thrower?;
        let target = self.target_by_uuid(ints_uuid(uuid))?;
        if self.target_level(target) != self.common.dim {
            return None;
        }
        let id = u32::try_from(self.network_entity_id(target)?).ok()?;
        self.entity(id)
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
