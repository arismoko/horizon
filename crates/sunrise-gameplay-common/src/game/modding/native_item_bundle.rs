//! Bundle-backed ITEM_ITEMS and value-level bundle operations; the place handler owns
//! transactional writes.
use mod_api::{BundleConfig, BundleMetrics, BundleReadError};
use sunrise_items::bundle::{BundleSpec, CustomBundleContents, CustomBundleError};
use sunrise_items::items::ItemStack;
use sunrise_sim::menu::container_items::{can_be_in_bundle, fits_inside_container_items};

fn bundle_spec(config: &BundleConfig) -> BundleSpec<'_> {
    BundleSpec::custom(&config.component, config.capacity_multiplier)
}

fn map_bundle_error(err: CustomBundleError) -> BundleReadError {
    match err {
        CustomBundleError::UnsupportedFormat => BundleReadError::UnsupportedFormat,
        CustomBundleError::Weight(_) => BundleReadError::ExcessiveWeight,
    }
}

pub trait NativeItemBundleGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    fn bundle_accepts(&self, config: &BundleConfig, item: &ItemStack) -> bool {
        if !can_be_in_bundle(&self.loop_context, item) {
            return false;
        }
        let Some(tag) = config.filter_tag.as_deref() else {
            return true;
        };
        let tag = tag.strip_prefix('#').unwrap_or(tag);
        self.items()
            .item_tags
            .get(tag)
            .is_some_and(|members| members.contains(item.item.as_ref()))
    }

    fn bundle_slots(
        &self,
        stack: &ItemStack,
        config: &BundleConfig,
    ) -> Result<Option<Vec<Option<ItemStack>>>, BundleReadError> {
        if stack.count == 0 {
            return Ok(None);
        }
        let Some(contents) = CustomBundleContents::try_of(self.items(), stack, bundle_spec(config))
            .map_err(map_bundle_error)?
        else {
            return Ok(None);
        };
        Ok(Some(contents.items().iter().cloned().map(Some).collect()))
    }

    fn bundle_capacity(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
        config: &BundleConfig,
    ) -> Result<u32, BundleReadError> {
        if stack.count == 0 {
            return Ok(0);
        }
        let room =
            CustomBundleContents::try_room_in_stack(self.items(), stack, bundle_spec(config), item)
                .map_err(map_bundle_error)?;
        if slot != 0 || !self.bundle_accepts(config, item) {
            return Ok(0);
        }
        Ok(room.unwrap_or(0))
    }

    fn bundle_insert_slot(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
        config: &BundleConfig,
    ) -> Result<Option<(u32, ItemStack)>, BundleReadError> {
        if stack.count == 0 {
            return Ok(None);
        }
        let items = self.items();
        let Some(mut bundle) = CustomBundleContents::try_of(items, stack, bundle_spec(config))
            .map_err(map_bundle_error)?
        else {
            return Ok(None);
        };
        if slot != 0 {
            return Ok(Some((0, stack.clone())));
        }
        let fits = fits_inside_container_items(&self.loop_context, item)
            && config.filter_tag.as_deref().is_none_or(|tag| {
                let tag = tag.strip_prefix('#').unwrap_or(tag);
                items
                    .item_tags
                    .get(tag)
                    .is_some_and(|members| members.contains(item.item.as_ref()))
            });
        let moved = bundle
            .try_insert_checked(items, &mut item.clone(), fits)
            .map_err(map_bundle_error)?;
        if moved == 0 {
            return Ok(Some((0, stack.clone())));
        }
        let mut changed = stack.clone();
        changed.count = 1;
        bundle.store(items, &mut changed);
        changed.data_mut().wire = Default::default();
        changed.tidy();
        Ok(Some((moved, changed)))
    }

    fn bundle_extract(
        &self,
        stack: &ItemStack,
        slot: usize,
        max: u32,
        config: &BundleConfig,
    ) -> Result<Option<(Option<ItemStack>, ItemStack)>, BundleReadError> {
        if stack.count == 0 {
            return Ok(None);
        }
        let items = self.items();
        let spec = bundle_spec(config);
        let Some(mut bundle) =
            CustomBundleContents::try_of(items, stack, spec).map_err(map_bundle_error)?
        else {
            return Ok(None);
        };
        if slot != 0 || max == 0 {
            return Ok(Some((None, stack.clone())));
        }
        // Selection belongs to the UI. Automation always takes from the front.
        let Some(out) = bundle
            .try_extract_front(items, max)
            .map_err(map_bundle_error)?
        else {
            return Ok(Some((None, stack.clone())));
        };
        let mut changed = stack.clone();
        changed.count = 1;
        bundle.store(items, &mut changed);
        changed.data_mut().wire = Default::default();
        changed.tidy();
        Ok(Some((Some(out), changed)))
    }

    fn bundle_metrics(
        &self,
        stack: &ItemStack,
        config: &BundleConfig,
    ) -> Result<Option<BundleMetrics>, BundleReadError> {
        if stack.count == 0 {
            return Ok(None);
        }
        // The weight is computed once as the contents are read, as vanilla memoizes it on the
        // `BundleContents` value; the fullness, flag and bar all read that one weight.
        let Some(contents) = CustomBundleContents::try_of(self.items(), stack, bundle_spec(config))
            .map_err(map_bundle_error)?
        else {
            return Ok(None);
        };
        Ok(Some(BundleMetrics::new(
            contents.weight_fraction(),
            contents.is_full(),
            contents.bar(),
            contents.items().len() as u32,
        )))
    }

    #[cfg(any(test, feature = "test-support"))]
    fn item_bundle_slots(&self, stack: &ItemStack) -> Option<Vec<Option<ItemStack>>> {
        self.bundle_slots(stack, &BundleConfig::vanilla()).unwrap()
    }

    #[cfg(any(test, feature = "test-support"))]
    fn item_bundle_capacity(&self, stack: &ItemStack, slot: usize, item: &ItemStack) -> u32 {
        self.bundle_capacity(stack, slot, item, &BundleConfig::vanilla())
            .unwrap()
    }

    #[cfg(any(test, feature = "test-support"))]
    fn item_bundle_insert_slot(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
    ) -> Option<(u32, ItemStack)> {
        self.bundle_insert_slot(stack, slot, item, &BundleConfig::vanilla())
            .unwrap()
    }

    #[cfg(any(test, feature = "test-support"))]
    fn item_bundle_extract(
        &self,
        stack: &ItemStack,
        slot: usize,
        max: u32,
    ) -> Option<(Option<ItemStack>, ItemStack)> {
        self.bundle_extract(stack, slot, max, &BundleConfig::vanilla())
            .unwrap()
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeItemBundleGameExt1<'__sg> for G {}

use crate::game::tables::GameItems as _;
