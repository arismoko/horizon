//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::testing::FakeJournal;

    #[cfg(test)]
    use super::*;
    use crate::parts::tests::*;
    #[cfg(test)]
    use crate::resource_util::move_resources;
    use crate::*;

    #[test]
    fn world_fluid_declarations_refuse_generated_names() {
        for (first, second) in [("oil", "flowing_oil"), ("flowing_oil", "oil")] {
            let mut registrar = super::PartRegistrar::new("maker");
            let options = {
                let mut value: crate::WorldFluid = Default::default();
                value.tick_delay = 20;
                value.drop_off = 2;
                value.slope_find_distance = 3;
                value.source_conversion = true;
                value.bucket = false;
                value
            };
            assert_ne!(
                registrar.declare_world_fluid(first, options.clone()),
                mod_vocab::__private::parts_installation::fluid_key(u32::MAX)
            );
            assert_eq!(
                registrar.declare_world_fluid(second, options),
                mod_vocab::__private::parts_installation::fluid_key(u32::MAX)
            );
            assert_eq!(registrar.world_fluids.len(), 1);
            let (_, (_, world)) = &registrar.world_fluids[0];
            assert_eq!(
                (
                    world.tick_delay,
                    world.drop_off,
                    world.slope_find_distance,
                    world.source_conversion,
                    world.bucket
                ),
                (20, 2, 3, true, false)
            );
        }
    }

    /// #6039: a world fluid's liquid block is a path in its namespace (or an id there), unset
    /// when the mod names none; another namespace or the flowing form's id is refused.
    #[test]
    fn world_fluid_declarations_name_their_liquid_block() {
        let named = |block: Option<&str>, flowing: Option<&str>| {
            let mut value: crate::WorldFluid = Default::default();
            value.block = block.map(str::to_owned);
            value.flowing = flowing.map(str::to_owned);
            value
        };
        let mut registrar = super::PartRegistrar::new("blood");
        registrar.declare_world_fluid(
            "life_essence_fluid",
            named(Some("life_essence_block"), None),
        );
        registrar.declare_world_fluid("plasma", named(Some("blood:plasma_block"), None));
        registrar.declare_world_fluid("oil", Default::default());
        let blocks: Vec<_> = registrar
            .world_fluids
            .iter()
            .map(|(_, (id, world))| (id.as_str(), world.block.as_deref()))
            .collect();
        assert_eq!(
            blocks,
            [
                ("blood:life_essence_fluid", Some("blood:life_essence_block")),
                ("blood:plasma", Some("blood:plasma_block")),
                ("blood:oil", None),
            ]
        );
        for (path, refused) in [
            ("other_ns", named(Some("other:block"), None)),
            (
                "as_flowing",
                named(Some("as_flowing_flow"), Some("as_flowing_flow")),
            ),
            // The default flowing id, though `flowing` is unset.
            ("dflt", named(Some("flowing_dflt"), None)),
        ] {
            assert_eq!(
                registrar.declare_world_fluid(path, refused),
                mod_vocab::__private::parts_installation::fluid_key(u32::MAX),
                "{path}"
            );
        }
        assert_eq!(registrar.world_fluids.len(), 3);
    }

    /// A port keeps its mod's ids: the flowing form and bucket are paths in the fluid's
    /// namespace, a bucket the mod declares itself any item id.
    #[test]
    fn world_fluid_declarations_name_their_flowing_form_and_bucket() {
        let named = |flowing: Option<&str>, bucket: bool, item: Option<&str>| {
            let mut value: crate::WorldFluid = Default::default();
            value.flowing = flowing.map(str::to_owned);
            value.bucket = bucket;
            value.bucket_item = item.map(str::to_owned);
            value
        };
        let mut registrar = super::PartRegistrar::new("maker");
        registrar.declare_world_fluid(
            "xp_still",
            named(Some("xp_flowing"), true, Some("xp_bucket")),
        );
        registrar.declare_world_fluid("tar", named(None, false, Some("other:tar_pot")));
        registrar.declare_world_fluid("oil", Default::default());
        let declared: Vec<_> = registrar
            .world_fluids
            .iter()
            .map(|(_, (id, world))| {
                (
                    id.as_str(),
                    world.flowing.as_deref(),
                    world.bucket_item.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            declared,
            [
                (
                    "maker:xp_still",
                    Some("maker:xp_flowing"),
                    Some("maker:xp_bucket")
                ),
                (
                    "maker:tar",
                    Some("maker:flowing_tar"),
                    Some("other:tar_pot")
                ),
                ("maker:oil", Some("maker:flowing_oil"), None),
            ]
        );
        assert!(
            registrar
                .registry_keys
                .borrow()
                .declared_fluids
                .contains_key("maker:xp_flowing")
        );
        for refused in [
            named(Some("other:flowing"), true, None),
            named(None, true, Some("other:bucket")),
            named(Some("Bad Name"), true, None),
            named(Some("refused"), true, None),
        ] {
            let before = registrar.refused.len();
            registrar.declare_world_fluid("refused", refused);
            assert_eq!(registrar.refused.len(), before + 1);
        }
        assert_eq!(registrar.world_fluids.len(), 3);
    }

    #[test]
    pub(crate) fn invalid_and_duplicate_fluid_declarations_are_refused() {
        let mut registrar = super::PartRegistrar::new("maker");
        let sentinel = mod_vocab::__private::parts_installation::fluid_key(u32::MAX);
        assert_ne!(registrar.declare_fluid("other:chocolate"), sentinel);
        assert_ne!(registrar.declare_fluid("minecraft:honey"), sentinel);
        assert_ne!(registrar.declare_fluid("chocolate"), sentinel);
        assert_ne!(
            registrar.declare_world_fluid("other:oil", crate::WorldFluid::default()),
            sentinel
        );
        assert_eq!(registrar.declare_fluid("Bad Name"), sentinel);
        assert_eq!(registrar.declare_fluid("minecraft:water"), sentinel);
        assert_eq!(registrar.declare_fluid("minecraft:flowing_lava"), sentinel);
        assert_eq!(
            registrar.declare_world_fluid("minecraft:water", crate::WorldFluid::default()),
            sentinel
        );
        registrar.declare_fluid("chocolate");
        assert_eq!(
            registrar.registry_keys.borrow().declared_fluids["other:chocolate"],
            "maker"
        );
        assert_eq!(
            registrar.registry_keys.borrow().declared_fluids["minecraft:honey"],
            "maker"
        );
        assert_eq!(registrar.world_fluids[0].0, "maker");
        assert_eq!(registrar.world_fluids[0].1.0, "other:oil");
        assert_eq!(registrar.refused.len(), 5, "{:?}", registrar.refused);
        assert!(registrar.refused[0].contains("maker"));
        assert!(registrar.refused[1].contains("minecraft:water"));
        assert!(registrar.refused[2].contains("minecraft:flowing_lava"));
        assert!(registrar.refused[3].contains("minecraft:water"));
        assert!(registrar.refused[4].contains("maker:chocolate"));
    }

    #[test]
    pub(crate) fn fluid_keys_resolve_declarations_after_requests_and_reject_unknown() {
        use mod_vocab::__private::{fluid_id, fluid_key, resolve_fluids};
        struct Request;
        #[mod_api::__subscriptions]
        impl Hooks for Request {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                assert_eq!(r.fluid_key("water"), r.fluid_key("minecraft:water"));
                r.fluid_key("maker:chocolate");
            }
        }
        struct Declare;
        #[mod_api::__subscriptions]
        impl Hooks for Declare {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_fluid("chocolate");
            }
        }
        let mods: Mods = vec![
            ModEntry::new("reader", Box::new(Request)),
            ModEntry::new("maker", Box::new(Declare)),
        ]
        .into();
        crate::__private::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        let error = resolve_fluids(&[], &[]).unwrap_err();
        assert_eq!(
            error,
            vec![mod_vocab::__private::parts_installation::unknown_fluid(
                "reader",
                "minecraft:water".to_owned()
            )]
        );
        resolve_fluids(&["minecraft:water", "minecraft:lava"], &[]).unwrap();
        let chocolate = fluid_key("maker:chocolate").unwrap();
        assert_eq!(fluid_id(chocolate), Some("maker:chocolate"));
        for name in ["minecraft:water", "minecraft:lava", "maker:chocolate"] {
            let key = fluid_key(name).unwrap();
            assert_eq!(
                mod_vocab::__private::fluid_key_utf16(&name.encode_utf16().collect::<Vec<_>>()),
                Some(key)
            );
            assert_eq!(fluid_id(key), Some(name));
        }
        for path in ["water", "lava"] {
            assert_eq!(
                mod_vocab::__private::fluid_key_utf16(&path.encode_utf16().collect::<Vec<_>>()),
                fluid_key(path)
            );
        }
        assert_eq!(
            mod_vocab::__private::parts_installation::validate_fluid_key(
                mod_vocab::__private::parts_installation::fluid_key_slot(chocolate)
            ),
            Ok(chocolate)
        );
        assert!(fluid_key("missing:fluid").is_none());
        assert_eq!(
            fluid_id(mod_vocab::__private::parts_installation::fluid_key(
                u32::MAX
            )),
            None
        );
    }
    /// A mod's tank: the game's tank at its position, reached from every side but its top, and
    /// misanswering as `lie` says.
    pub(crate) struct ModTank {
        pub(crate) at: BlockSide,
        pub(crate) lie: Option<Lie>,
    }
    impl ResourceHandler<FluidResource> for ModTank {
        fn size(&self, host: &mut dyn ServerHost) -> usize {
            if self.at.side == Some(Direction::Up) {
                return 0;
            }
            crate::backend_of(host)
                .and_then(|backend| backend.fluid_tank_count(self.at))
                .unwrap_or(0)
        }
        fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<FluidResource> {
            Some(FluidResource::of(
                &crate::backend_of(host).and_then(|backend| backend.fluid_tank(self.at, index))?,
            ))
        }
        fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
            crate::backend_of(host)
                .and_then(|backend| backend.fluid_tank(self.at, index))
                .map_or(0, |held| held.amount)
        }
        fn capacity(
            &self,
            host: &mut dyn ServerHost,
            index: usize,
            resource: Option<&FluidResource>,
        ) -> u32 {
            crate::backend_of(host).map_or(0, |backend| {
                backend.fluid_tank_capacity(
                    self.at,
                    index,
                    resource.map(|r| r.to_stack(1)).as_ref(),
                )
            })
        }
        fn insert_at(
            &self,
            host: &mut dyn ServerHost,
            tx: &TransactionContext,
            index: usize,
            resource: &FluidResource,
            amount: u32,
        ) -> Result<u32, TransactionError<Overanswered>> {
            let Some(backend) = crate::backend_mut_of(host) else {
                return Ok(0);
            };
            let rest = backend.fluid_tank_insert(self.at, index, resource.to_stack(amount), tx);
            let went = amount - rest.map_or(0, |rest| rest.amount);
            Ok(match self.lie {
                Some(Lie::TakesMore) => went + 100,
                _ => went,
            })
        }
        fn extract_at(
            &self,
            host: &mut dyn ServerHost,
            tx: &TransactionContext,
            index: usize,
            resource: &FluidResource,
            amount: u32,
        ) -> Result<u32, TransactionError<Overanswered>> {
            let held =
                crate::backend_of(host).and_then(|backend| backend.fluid_tank(self.at, index));
            if !held.is_some_and(|held| resource.matches(&held)) {
                return Ok(0);
            }
            let more = if self.lie == Some(Lie::GivesMore) {
                100
            } else {
                0
            };
            Ok(crate::backend_mut_of(host)
                .and_then(|backend| backend.fluid_tank_extract(self.at, index, amount + more, tx))
                .map_or(0, |out| out.amount))
        }
    }

    #[cfg(test)]
    /// One tank, as the game answers it: what it holds, its capacity, and whether it fills and
    /// drains. Truthfully: a `halving` tank takes half of each offer, a `half_room` one at most
    /// half its room, a `short` one gives one less than asked, and a `quarters` one gives and
    /// takes only whole quarter buckets, as a cauldron its thirds.
    #[derive(Clone)]
    pub(crate) struct GameTank {
        pub(crate) held: Option<FluidStack>,
        pub(crate) capacity: u32,
        pub(crate) fills: bool,
        pub(crate) drains: bool,
        pub(crate) halving: bool,
        pub(crate) half_room: bool,
        pub(crate) short: bool,
        pub(crate) quarters: bool,
    }
    #[cfg(test)]
    /// Game tanks by position, and the mod block entity types standing at some.
    #[derive(Clone, Default)]
    pub(crate) struct TankWorld {
        tanks: HashMap<[i32; 3], GameTank>,
        types: HashMap<[i32; 3], &'static str>,
        journal: FakeJournal<HashMap<[i32; 3], GameTank>>,
    }
    #[cfg(test)]
    pub(crate) const SOURCE: [i32; 3] = [0, 64, 0];
    #[cfg(test)]
    pub(crate) const TANK: [i32; 3] = [1, 64, 0];
    #[cfg(test)]
    pub(crate) const OTHER: [i32; 3] = [2, 64, 0];
    #[cfg(test)]
    impl TankWorld {
        #[cfg(test)]
        fn tank(&mut self, pos: [i32; 3], held: Option<FluidStack>, fills: bool) -> &mut Self {
            let tank = GameTank {
                held,
                capacity: 1000,
                fills,
                drains: true,
                halving: false,
                half_room: false,
                short: false,
                quarters: false,
            };
            self.tanks.insert(pos, tank);
            self
        }
        #[cfg(test)]
        fn held(&self, pos: [i32; 3]) -> Option<FluidStack> {
            self.tanks[&pos].held.clone()
        }
        #[cfg(test)]
        fn amount(&self, pos: [i32; 3]) -> u32 {
            self.tanks[&pos].held.as_ref().map_or(0, |held| held.amount)
        }
        #[cfg(test)]
        /// Moves up to `max` from `from`'s handler through `from_side` into `to`'s through
        /// `to_side`, in a transaction of its own.
        fn moved(
            &mut self,
            (from, from_side): ([i32; 3], Option<Direction>),
            (to, to_side): ([i32; 3], Option<Direction>),
            max: u32,
        ) -> Result<u32, crate::TransactionError<Overanswered>> {
            let host: &mut dyn ServerHost = self;
            let source = here().capability(host, &FLUIDS, from, from_side).unwrap();
            let target = here().capability(host, &FLUIDS, to, to_side).unwrap();
            match host.transaction(|host, tx| {
                move_resources(host, &source, &target, |_| true, max, Some(tx))
            }) {
                Ok(moved) => Ok(moved),
                Err(crate::TransactionError::Aborted(over)) => Err(over),
                Err(crate::TransactionError::InvalidContext) => {
                    Err(crate::TransactionError::InvalidContext)
                }
                Err(crate::TransactionError::Unavailable) => {
                    Err(crate::TransactionError::Unavailable)
                }
                Err(crate::TransactionError::CommitRefused) => {
                    Err(crate::TransactionError::CommitRefused)
                }
            }
        }
    }
    #[cfg(test)]
    impl mod_api::prelude::LevelBlocksBackend for TankWorld {
        fn level_with_capability_site(
            &self,
            at: BlockSide,
            visit: &mut dyn FnMut(&dyn crate::LevelHost, crate::BlockCapabilitySite),
        ) -> Result<(), crate::LevelReadError> {
            if at.level != here() {
                return Err(crate::LevelReadError::Unavailable);
            }
            let air = crate::__private::block_key("minecraft:air")
                .ok_or(crate::LevelReadError::Unavailable)?;
            let resident = crate::prelude::BlockEntitiesHost::block_entity(self, at.level, at.pos)
                .map(|entity| {
                    (
                        entity,
                        crate::prelude::BlockEntitiesBackend::block_entity_kind(self, entity),
                    )
                });
            visit(
                &crate::parts::tests::Level,
                crate::BlockCapabilitySite::new(
                    at,
                    crate::__private::block_state_from_ordinal(air, 0),
                    resident,
                    false,
                ),
            );
            Ok(())
        }
    }
    #[cfg(test)]
    impl mod_api::prelude::LevelTickingBackend for TankWorld {
        fn level_in_place(&self, level: LevelRef) -> bool {
            level == here()
        }
    }
    #[cfg(test)]
    impl mod_api::prelude::BlockEntitiesBackend for TankWorld {
        fn block_entity_kind(&self, entity: BlockEntityRef) -> Option<BlockEntityTypeKey> {
            self.types
                .get(&mod_vocab::__private::game::Fields::fields(&entity).position)
                .map(|id| type_key(id))
        }
    }
    #[cfg(test)]
    impl mod_api::prelude::FluidsEnergyBackend for TankWorld {
        fn fluid_tank_count(&self, BlockSide { pos, .. }: BlockSide) -> Option<usize> {
            self.tanks.contains_key(&pos).then_some(1)
        }
        fn fluid_tank(&self, BlockSide { pos, .. }: BlockSide, _: usize) -> Option<FluidStack> {
            self.tanks.get(&pos)?.held.clone()
        }
        fn fluid_tank_capacity(
            &self,
            BlockSide { pos, .. }: BlockSide,
            _: usize,
            _: Option<&FluidStack>,
        ) -> u32 {
            self.tanks
                .get(&pos)
                .filter(|tank| tank.fills)
                .map_or(0, |tank| tank.capacity)
        }
    }
    #[cfg(test)]
    impl Backend for TankWorld {}
    #[cfg(test)]
    mod_api::impl_backend_areas!(
        TankWorld,
        except(block_entities, fluids_energy, level_blocks, level_ticking)
    );
    #[cfg(test)]
    impl mod_api::prelude::FluidsEnergyBackendMut for TankWorld {
        fn fluid_tank_insert(
            &mut self,
            BlockSide { pos, .. }: BlockSide,
            _: usize,
            fluid: FluidStack,
            tx: &TransactionContext,
        ) -> Option<FluidStack> {
            self.journal.assert_current(tx);
            let Some(tank) = self.tanks.get_mut(&pos).filter(|tank| tank.fills) else {
                return Some(fluid);
            };
            let held = tank.held.as_ref().map_or(0, |held| held.amount);
            if tank
                .held
                .as_ref()
                .is_some_and(|held| !held.same_fluid_same_components(&fluid))
            {
                return Some(fluid);
            }
            let offered = if tank.halving {
                fluid.amount / 2
            } else {
                fluid.amount
            };
            let room = if tank.half_room {
                (tank.capacity - held) / 2
            } else {
                tank.capacity - held
            };
            let mut add = offered.min(room);
            if tank.quarters {
                add -= add % 250;
            }
            if add > 0 {
                tank.held = Some(fluid.with_amount(held + add));
            }
            (add < fluid.amount).then(|| fluid.with_amount(fluid.amount - add))
        }
        fn fluid_tank_extract(
            &mut self,
            BlockSide { pos, .. }: BlockSide,
            _: usize,
            max: u32,
            tx: &TransactionContext,
        ) -> Option<FluidStack> {
            self.journal.assert_current(tx);
            let tank = self.tanks.get_mut(&pos).filter(|tank| tank.drains)?;
            let held = tank.held.clone()?;
            let asked = if tank.short {
                max.saturating_sub(1)
            } else {
                max
            };
            let mut take = held.amount.min(asked);
            if tank.quarters {
                take -= take % 250;
            }
            if take == 0 {
                return None;
            }
            tank.held = (take < held.amount).then(|| held.with_amount(held.amount - take));
            Some(held.with_amount(take))
        }
    }
    #[cfg(test)]
    impl BackendMut for TankWorld {}
    #[cfg(test)]
    mod_api::impl_backend_mut_areas!(TankWorld, except(fluids_energy));
    #[cfg(test)]
    impl mod_api::prelude::BlockEntitiesHost for TankWorld {
        fn block_entity(&self, level: LevelRef, position: [i32; 3]) -> Option<BlockEntityRef> {
            Some(crate::__private::game::block_entity_ref(level, position, 1))
        }
    }
    #[cfg(test)]
    impl Host for TankWorld {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(mod_vocab::__private::parts_installation::backend_ref(self))
        }
    }
    #[cfg(test)]
    mod_api::impl_host_areas!(TankWorld, except(block_entities));
    #[cfg(test)]
    impl ServerHost for TankWorld {
        fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
            Some(mod_vocab::__private::parts_installation::backend_mut_ref(
                self,
            ))
        }
        fn with_current_transaction_scope(
            &mut self,
            callback: &mut dyn for<'loan> FnMut(
                &'loan mut dyn ServerHost,
                &'loan TransactionContext,
            ),
        ) -> Result<(), mod_vocab::TransactionUnavailable> {
            FakeJournal::with_current_scope(self, |host| &host.journal, callback)
        }

        fn with_transaction_scope(
            &mut self,
            callback: &mut dyn for<'loan> FnMut(
                &'loan mut dyn ServerHost,
                &'loan TransactionContext,
            ) -> bool,
        ) -> Result<bool, crate::TransactionScopeError> {
            FakeJournal::with_scope(
                self,
                |world| &mut world.journal,
                |world| world.tanks.clone(),
                |world, tanks| world.tanks = tanks,
                callback,
            )
        }
        fn transaction_matches(&self, tx: Option<&TransactionContext>) -> bool {
            self.journal.matches(tx)
        }
    }
    #[cfg(test)]
    mod_api::impl_server_host_areas!(TankWorld, except(mod_state));
    #[cfg(test)]
    impl crate::prelude::ModStateServerHost for TankWorld {
        fn transaction_lifecycle(&self) -> crate::TransactionLifecycle {
            if self.journal.depth() == 0 {
                crate::TransactionLifecycle::None
            } else {
                crate::TransactionLifecycle::Open
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn potion(amount: u32, potion: &str) -> FluidStack {
        let components = crate::Compound::from([(
            sunrise_core::nbt::JavaString::from("minecraft:potion_contents"),
            crate::Tag::compound([("potion", crate::Tag::String(potion.into()))]),
        )]);
        FluidStack {
            components: Some(Box::new(crate::Components::from(components))),
            ..FluidStack::new(
                mod_vocab::__private::fluid_key("test:potion").unwrap(),
                amount,
            )
        }
    }

    #[test]
    pub(crate) fn a_fluid_move_fills_only_with_the_same_fluid_and_components() {
        game_thread();
        let mut world = TankWorld::default();
        world.tank(SOURCE, Some(potion(1500, "minecraft:healing")), true);
        world.tank(TANK, None, true).types.insert(TANK, "test:tank");
        // The mod's tank takes nothing from above.
        let up = (TANK, Some(Direction::Up));
        let west = (TANK, Some(Direction::West));
        assert_eq!(world.moved((SOURCE, None), up, 4000), Ok(0));
        // What the tank has room for moves, and nothing more leaves the source.
        assert_eq!(world.moved((SOURCE, None), west, 4000), Ok(1000));
        assert_eq!(world.moved(west, (SOURCE, None), 400), Ok(400));
        assert_eq!(world.held(SOURCE), Some(potion(900, "minecraft:healing")));
        assert_eq!(world.held(TANK), Some(potion(600, "minecraft:healing")));
        // The same fluid with other components, or another fluid, doesn't go in.
        for other in [
            potion(500, "minecraft:poison"),
            FluidStack::new(mod_vocab::__private::fluid_key("test:potion").unwrap(), 500),
        ] {
            world.tank(SOURCE, Some(other.clone()), true);
            assert_eq!(world.moved((SOURCE, None), (TANK, None), 4000), Ok(0));
            assert_eq!(world.held(SOURCE), Some(other));
            assert_eq!(world.held(TANK), Some(potion(600, "minecraft:healing")));
        }
        // A drain-only tank has no capacity, and nothing moves.
        let water = FluidStack::new(
            mod_vocab::__private::fluid_key("minecraft:water").unwrap(),
            1000,
        );
        world.tank(SOURCE, Some(water.clone()), true);
        world.tank(TANK, None, false);
        let host: &mut dyn ServerHost = &mut world;
        let target = here().capability(host, &FLUIDS, TANK, None).unwrap();
        assert_eq!(
            target.capacity(host, 0, Some(&FluidResource::of(&water))),
            0
        );
        assert_eq!(world.moved((SOURCE, None), (TANK, None), 1000), Ok(0));
        assert_eq!(world.held(SOURCE), Some(water));
    }

    #[test]
    pub(crate) fn a_fluid_move_commits_only_the_destination_count_the_source_can_supply_exactly() {
        game_thread();
        let water = |amount| {
            FluidStack::new(
                mod_vocab::__private::fluid_key("minecraft:water").unwrap(),
                amount,
            )
        };
        type Setup = fn(&mut GameTank, &mut GameTank);
        let cases: [(&str, Setup, u32); 7] = [
            // NeoForge ResourceHandlerUtil.moveInternal visits each source index once.
            ("half of each offer", |_, t| t.halving = true, 500),
            (
                "half of each offer, from quarters",
                |s, t| (s.quarters, t.halving) = (true, true),
                500,
            ),
            (
                "half of each offer, from a drain-only tank",
                |s, t| (s.fills, t.halving) = (false, true),
                500,
            ),
            // Half its room: 500 fits a quarter-quantized source; 375 (half of 750) cannot be
            // extracted exactly, so the transfer rolls back to 0.
            (
                "half its room, from quarters",
                |s, t| (s.quarters, t.half_room) = (true, true),
                500,
            ),
            (
                "non-quarter half room, from quarters",
                |s, t| {
                    s.quarters = true;
                    t.half_room = true;
                    t.capacity = 750;
                },
                0,
            ),
            (
                "half its room, from a drain-only tank",
                |s, t| (s.fills, t.half_room) = (false, true),
                500,
            ),
            // It probes as available for 999 but cannot supply 999 after that much was inserted.
            ("a source giving one less", |s, _| s.short = true, 0),
        ];
        for (case, setup, moves) in cases {
            let mut world = TankWorld::default();
            world.tank(SOURCE, Some(water(1000)), true);
            world.tank(TANK, None, true);
            let [source, target] = world.tanks.get_disjoint_mut([&SOURCE, &TANK]);
            setup(source.unwrap(), target.unwrap());
            let moved = world.moved((SOURCE, None), (TANK, None), 1000);
            assert_eq!(moved, Ok(moves), "{case}");
            assert_eq!(
                (world.amount(SOURCE), world.amount(TANK)),
                (1000 - moves, moves),
                "{case}"
            );
        }
    }

    #[test]
    pub(crate) fn a_handler_answering_more_than_it_was_asked_aborts_the_whole_move_and_loses_no_fluid()
     {
        game_thread();
        let water = |amount| {
            Some(FluidStack::new(
                mod_vocab::__private::fluid_key("minecraft:water").unwrap(),
                amount,
            ))
        };
        for (liar, at) in [("test:gives_more", SOURCE), ("test:takes_more", TANK)] {
            let mut world = TankWorld::default();
            world.tank(SOURCE, water(1000), true);
            world.tank(OTHER, water(1000), true);
            world.tank(TANK, None, true).types.insert(at, liar);
            let host: &mut dyn ServerHost = &mut world;
            let ends = |host: &mut dyn ServerHost, pos| here().capability(host, &FLUIDS, pos, None);
            let (source, other, tank) = (
                ends(host, SOURCE).unwrap(),
                ends(host, OTHER).unwrap(),
                ends(host, TANK).unwrap(),
            );
            // Two moves into the tank in one transaction, the second from the source: the
            // first move through the liar fails, and its error aborts the transaction, so
            // every tank holds what it held.
            let moved = host.transaction(|host, tx| {
                let first = move_resources(host, &other, &tank, |_| true, 300, Some(tx))?;
                let second = move_resources(host, &source, &tank, |_| true, 300, Some(tx))?;
                Ok::<_, crate::TransactionError<Overanswered>>(first + second)
            });
            let crate::TransactionError::Aborted(crate::TransactionError::Aborted(error)) =
                moved.unwrap_err()
            else {
                panic!("expected aborted Overanswered");
            };
            assert_eq!(error.owner.as_deref(), Some("doubler"), "{liar}");
            assert!(crate::parts::warned_about("doubler"));
            let held = [SOURCE, OTHER, TANK].map(|pos| world.held(pos));
            assert_eq!(held, [water(1000), water(1000), None], "{liar}");
        }

        // The full probe is capped by what the source holds. Only the extraction
        // after the destination takes half can return more than was requested.
        let mut world = TankWorld::default();
        world.tank(SOURCE, water(1000), true);
        world.types.insert(SOURCE, "test:gives_more");
        world.tank(TANK, None, true);
        world.tanks.get_mut(&TANK).unwrap().halving = true;
        let crate::TransactionError::Aborted(error) =
            world.moved((SOURCE, None), (TANK, None), 1000).unwrap_err()
        else {
            panic!("expected aborted Overanswered");
        };
        assert_eq!(error.owner.as_deref(), Some("doubler"));
        assert_eq!([world.held(SOURCE), world.held(TANK)], [water(1000), None]);
    }
}

/// Built-in fluids (`BuiltInRegistries.FLUID`), which no mod can replace.
const VANILLA_FLUIDS: [&str; 5] = [
    "minecraft:empty",
    "minecraft:water",
    "minecraft:flowing_water",
    "minecraft:lava",
    "minecraft:flowing_lava",
];

impl PartRegistrar {
    fn is_builtin_fluid(&self, id: &str) -> bool {
        VANILLA_FLUIDS.contains(&id) || self.has_builtin_entry("minecraft:fluid", id)
    }

    // Only callers that have validated the key may insert a declaration.
    fn insert_declared_fluid(&mut self, id: String) -> crate::FluidKey {
        let previous = self
            .registry_keys
            .borrow_mut()
            .declared_fluids
            .insert(id.clone(), self.mod_id);
        if let Some(other) = previous {
            self.refused.push(format!(
                "fluid {id} declared by both {other} and {}",
                self.mod_id
            ));
        }
        self.fluid_key(&id)
    }
}

impl FluidsRegistrar for PartRegistrar {
    fn fluid_key(&mut self, id: &str) -> crate::FluidKey {
        match self
            .registry_keys
            .borrow_mut()
            .fluids
            .request(self.mod_id, id)
        {
            Ok(slot) => mod_vocab::__private::parts_installation::fluid_key(slot),
            Err(error) => {
                self.refused.push(error);
                mod_vocab::__private::parts_installation::fluid_key(u32::MAX)
            }
        }
    }

    fn declare_fluid(&mut self, path: &str) -> crate::FluidKey {
        let id = match self.declaration_id(path) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return mod_vocab::__private::parts_installation::fluid_key(u32::MAX);
            }
        };
        if self.is_builtin_fluid(&id) {
            self.refused
                .push(format!("mod {} declared built-in fluid {id}", self.mod_id));
            return mod_vocab::__private::parts_installation::fluid_key(u32::MAX);
        }
        self.insert_declared_fluid(id)
    }

    fn declare_world_fluid(&mut self, path: &str, mut fluid: crate::WorldFluid) -> crate::FluidKey {
        let id = match self.declaration_id(path) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return mod_vocab::__private::parts_installation::fluid_key(u32::MAX);
            }
        };
        let (namespace, path) = id.split_once(':').expect("validated declaration id");
        // A name the fluid gives is a path in its namespace, or an id there.
        let in_namespace = |name: &str| {
            let full = match name.split_once(':') {
                Some((own, _)) if own == namespace => name.to_owned(),
                Some(_) => return None,
                None => format!("{namespace}:{name}"),
            };
            crate::Identifier::parse_namespaced(&full).map(|_| full)
        };
        let flowing = match fluid.flowing.as_deref() {
            None => Some(format!("{namespace}:flowing_{path}")),
            Some(name) => in_namespace(name),
        };
        let bucket_item = match fluid.bucket_item.as_deref() {
            None => Some(None),
            // An item the mod declares itself may be in any namespace.
            Some(name) if !fluid.bucket && name.contains(':') => {
                crate::Identifier::parse_namespaced(name).map(|_| Some(name.to_owned()))
            }
            Some(name) => in_namespace(name).map(Some),
        };
        let block = match fluid.block.as_deref() {
            None => Some(None),
            Some(name) => in_namespace(name).map(Some),
        };
        let (Some(flowing), Some(bucket_item), Some(block)) = (flowing, bucket_item, block) else {
            self.refused.push(format!(
                "mod {}: world fluid {path:?}: flowing, block and a generated bucket_item are ids \
                 in namespace {namespace}, bucket_item an item id",
                self.mod_id
            ));
            return mod_vocab::__private::parts_installation::fluid_key(u32::MAX);
        };
        if block.as_ref() == Some(&flowing) {
            self.refused.push(format!(
                "mod {}: world fluid {path:?}: its block is not its flowing form's id",
                self.mod_id
            ));
            return mod_vocab::__private::parts_installation::fluid_key(u32::MAX);
        }
        fluid.flowing = Some(flowing.clone());
        fluid.bucket_item = bucket_item;
        fluid.block = block;
        if [id.as_str(), flowing.as_str()]
            .iter()
            .any(|candidate| self.is_builtin_fluid(candidate))
        {
            self.refused.push(format!(
                "mod {} declared built-in world fluid {id}",
                self.mod_id
            ));
            return mod_vocab::__private::parts_installation::fluid_key(u32::MAX);
        }
        if flowing == id
            || [id.as_str(), flowing.as_str()].iter().any(|id| {
                self.registry_keys
                    .borrow()
                    .declared_fluids
                    .contains_key(*id)
            })
        {
            self.refused
                .push(format!("world fluid {id}: name declared twice"));
            return mod_vocab::__private::parts_installation::fluid_key(u32::MAX);
        }
        if let Err(error) = fluid.check() {
            self.refused.push(format!(
                "mod {}: world fluid {path:?}: {error}",
                self.mod_id
            ));
            return mod_vocab::__private::parts_installation::fluid_key(u32::MAX);
        }
        let key = self.insert_declared_fluid(id.clone());
        if key != mod_vocab::__private::parts_installation::fluid_key(u32::MAX) {
            self.registry_keys
                .borrow_mut()
                .declared_fluids
                .insert(flowing.clone(), self.mod_id);
            self.fluid_key(&flowing);
            self.world_fluids.push((self.mod_id, (id, fluid)));
        }
        key
    }

    fn fluid_interaction(&mut self, interaction: crate::FluidInteraction) {
        self.fluid_interactions
            .push(mod_vocab::__private::RegisteredFluidInteraction::Declared(
                interaction,
            ));
    }

    fn fluid_interaction_handler(
        &mut self,
        source: crate::FluidKey,
        handler: Arc<dyn crate::FluidInteractionHandler>,
    ) {
        self.fluid_interactions
            .push(mod_vocab::__private::RegisteredFluidInteraction::Handler {
                owner: mod_vocab::__private::game::CallbackOwner::for_mod(self.mod_id),
                source,
                handler,
            });
    }

    fn attach_fluid_part(&mut self, fluid: &str, kind: &'static str, part: Arc<dyn FluidPart>) {
        self.attach(|p| &mut p.fluids, kind, fluid, part);
    }
}
