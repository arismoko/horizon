//! Minecart handles for native server mods.
use super::{
    commands::Target,
    minecarts::{CartKind, Minecart, display_offset},
};
use mod_api::{EntityRef, MinecartKind, MinecartRef};
use sunrise_world::registry::BlockStateId;

impl crate::game::ServerGame<'_> {
    fn native_minecart_index(&self, cart: MinecartRef) -> Option<usize> {
        let Target::Cart(id) = self.native_entity_target(cart.entity())? else {
            return None;
        };
        self.common.carts.iter().position(|c| c.id == id)
    }

    fn native_minecart(&self, cart: MinecartRef) -> Option<&Minecart> {
        self.common.carts.get(self.native_minecart_index(cart)?)
    }

    /// Edits a cart, of one kind if `kind` is given. False for another kind
    /// or a handle that no longer resolves, including while held out of storage.
    fn native_minecart_edit(
        &mut self,
        cart: MinecartRef,
        kind: Option<CartKind>,
        edit: impl FnOnce(&mut Minecart),
    ) -> bool {
        let Some(index) = self.native_minecart_index(cart) else {
            return false;
        };
        if kind.is_some_and(|kind| self.common.carts[index].kind != kind) {
            return false;
        }
        self.common.carts.update(index, edit);
        true
    }

    /// Narrows an entity to a minecart of any kind, or None for another kind of
    /// entity, a stale handle, a client, or an entity held out of storage.
    pub fn minecart(&self, entity: EntityRef) -> Option<MinecartRef> {
        let cart = MinecartRef::new(entity);
        self.native_minecart_index(cart).map(|_| cart)
    }

    /// Which `AbstractMinecart` subclass the cart is.
    pub(in crate::game) fn minecart_kind(&self, cart: MinecartRef) -> Option<MinecartKind> {
        Some(match self.native_minecart(cart)?.kind {
            CartKind::Rideable => MinecartKind::Rideable,
            CartKind::Chest => MinecartKind::Chest,
            CartKind::Furnace => MinecartKind::Furnace,
            CartKind::Tnt => MinecartKind::Tnt,
            CartKind::Hopper => MinecartKind::Hopper,
            CartKind::CommandBlock => MinecartKind::CommandBlock,
            CartKind::Spawner => MinecartKind::Spawner,
        })
    }

    /// `AbstractMinecart.getMaxSpeed`, in blocks per tick: the old behaviour's
    /// 0.4 (0.2 in water), or with the `minecart_improvements` experiment the
    /// `max_minecart_speed` rule over 20 (halved in water). A furnace minecart
    /// takes half of that, three quarters in water (`MinecartFurnace`).
    pub(in crate::game) fn minecart_max_speed(&self, cart: MinecartRef) -> Option<f64> {
        Some(self.cart_max_speed(self.native_minecart(cart)?))
    }

    /// `AbstractMinecart.isOnRails`.
    pub(in crate::game) fn minecart_is_on_rails(&self, cart: MinecartRef) -> Option<bool> {
        Some(self.native_minecart(cart)?.on_rails)
    }

    /// `AbstractMinecart.isFlipped`.
    pub(in crate::game) fn minecart_flipped(&self, cart: MinecartRef) -> Option<bool> {
        Some(self.native_minecart(cart)?.flipped)
    }

    /// `AbstractMinecart.setFlipped`; saved as `FlippedRotation`, not synced.
    pub(in crate::game) fn minecart_set_flipped(
        &mut self,
        cart: MinecartRef,
        flipped: bool,
    ) -> bool {
        self.native_minecart_edit(cart, None, |c| c.flipped = flipped)
    }

    /// `VehicleEntity.getDamage`, the wobble damage (`DATA_ID_DAMAGE`).
    pub(in crate::game) fn minecart_damage(&self, cart: MinecartRef) -> Option<f32> {
        Some(self.native_minecart(cart)?.damage)
    }

    /// `VehicleEntity.setDamage`: synced to clients. The tick counts it down
    /// only while positive.
    pub(in crate::game) fn minecart_set_damage(&mut self, cart: MinecartRef, damage: f32) -> bool {
        self.native_minecart_edit(cart, None, |c| c.damage = damage)
    }

    /// `VehicleEntity.getHurtTime`, the wobble ticks (`DATA_ID_HURT`).
    pub(in crate::game) fn minecart_hurt_time(&self, cart: MinecartRef) -> Option<i32> {
        Some(self.native_minecart(cart)?.hurt_time)
    }

    /// `VehicleEntity.setHurtTime`: synced to clients.
    pub(in crate::game) fn minecart_set_hurt_time(
        &mut self,
        cart: MinecartRef,
        ticks: i32,
    ) -> bool {
        self.native_minecart_edit(cart, None, |c| c.hurt_time = ticks)
    }

    /// `AbstractMinecart.customDisplayBlockState`: the block the cart shows in
    /// place of its kind's default, None (inner) when it has none.
    pub(in crate::game) fn minecart_custom_display_block(
        &self,
        cart: MinecartRef,
    ) -> Option<Option<BlockStateId>> {
        let cart = self.native_minecart(cart)?;
        Some(
            cart.custom_display
                .as_ref()
                .and_then(|t| self.state_from_nbt(t)),
        )
    }

    /// Sets the block the cart shows, or with None its kind's default again:
    /// saved as `DisplayState` and synced to clients. False for a state the
    /// registry doesn't know, as the loader drops one that does not decode.
    pub(in crate::game) fn minecart_set_custom_display_block(
        &mut self,
        cart: MinecartRef,
        state: Option<BlockStateId>,
    ) -> bool {
        if state.is_some_and(|s| self.common.pack.assets.registry.state(s).is_none()) {
            return false;
        }
        let display = state.map(|s| self.common.state_to_nbt(s));
        self.native_minecart_edit(cart, None, |c| c.custom_display = display)
    }

    /// `AbstractMinecart.getDisplayOffset`, in sixteenths of a block: the set
    /// offset, else the kind's default.
    pub(in crate::game) fn minecart_display_offset(&self, cart: MinecartRef) -> Option<i32> {
        Some(display_offset(self.native_minecart(cart)?) as i32)
    }

    /// `AbstractMinecart.setDisplayOffset`, in sixteenths of a block: saved as
    /// `DisplayOffset` (only off the kind's default) and synced to clients.
    pub(in crate::game) fn minecart_set_display_offset(
        &mut self,
        cart: MinecartRef,
        offset: i32,
    ) -> bool {
        self.native_minecart_edit(cart, None, |c| c.custom_offset = Some(offset))
    }

    /// `MinecartFurnace`'s fuel ticks; None for another kind of cart.
    pub(in crate::game) fn minecart_fuel(&self, cart: MinecartRef) -> Option<i32> {
        let cart = self.native_minecart(cart)?;
        (cart.kind == CartKind::Furnace).then_some(cart.fuel)
    }

    /// Sets a furnace minecart's fuel ticks, any int as vanilla's field holds
    /// it (saved as a short, the low 16 bits). False for another kind of cart.
    /// The lit flag (`DATA_ID_FUEL`) follows with fuel above zero.
    pub(in crate::game) fn minecart_set_fuel(&mut self, cart: MinecartRef, fuel: i32) -> bool {
        self.native_minecart_edit(cart, Some(CartKind::Furnace), |c| c.fuel = fuel)
    }

    /// A furnace minecart's push (`MinecartFurnace.push`): the way its fuel drives it on rails,
    /// x and z; its y part stays between ticks and sets the push's length. None for another
    /// kind of cart.
    pub fn minecart_push(&self, cart: MinecartRef) -> Option<[f64; 3]> {
        let cart = self.native_minecart(cart)?;
        (cart.kind == CartKind::Furnace).then_some([cart.push.x, cart.push.y, cart.push.z])
    }

    /// Sets a furnace minecart's push, as a player's click with fuel sets it toward them
    /// (`MinecartFurnace.interact`): a cart a mod drives runs that way while it has fuel. False
    /// for another kind of cart or a push that isn't finite.
    pub fn minecart_set_push(&mut self, cart: MinecartRef, push: [f64; 3]) -> bool {
        if !push.iter().all(|v| v.is_finite()) {
            return false;
        }
        let [x, y, z] = push;
        self.native_minecart_edit(cart, Some(CartKind::Furnace), |c| {
            c.push = sunrise_core::geometry::Vec3::new(x, y, z)
        })
    }

    /// `MinecartTNT`'s fuse ticks, -1 when it is not primed; None for another
    /// kind of cart.
    pub(in crate::game) fn minecart_fuse(&self, cart: MinecartRef) -> Option<i32> {
        let cart = self.native_minecart(cart)?;
        (cart.kind == CartKind::Tnt).then_some(cart.fuse)
    }

    /// `MinecartTNT.primeFuse(null)`: an 80 tick fuse, the primed sound and
    /// entity event 70 for clients. False for another kind of cart, or with
    /// the `tnt_explodes` rule off.
    pub(in crate::game) fn minecart_prime_fuse(&mut self, cart: MinecartRef) -> bool {
        let Some(index) = self.native_minecart_index(cart) else {
            return false;
        };
        if self.common.carts[index].kind != CartKind::Tnt
            || !self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
        {
            return false;
        }
        let mut primed = self.common.carts[index].clone();
        self.prime_cart(&mut primed);
        self.native_minecart_edit(cart, Some(CartKind::Tnt), |c| c.fuse = primed.fuse)
    }

    /// `MinecartHopper.isEnabled`; None for another kind of cart.
    pub(in crate::game) fn minecart_hopper_enabled(&self, cart: MinecartRef) -> Option<bool> {
        let cart = self.native_minecart(cart)?;
        (cart.kind == CartKind::Hopper).then_some(cart.enabled)
    }

    /// `MinecartHopper.setEnabled`: saved as `Enabled`, not synced.
    pub(in crate::game) fn minecart_set_hopper_enabled(
        &mut self,
        cart: MinecartRef,
        enabled: bool,
    ) -> bool {
        self.native_minecart_edit(cart, Some(CartKind::Hopper), |c| c.enabled = enabled)
    }

    /// A command block minecart's command (`MinecartCommandBlock.getCommandBlock().getCommand()`).
    /// None for another kind of cart.
    pub(in crate::game) fn minecart_command(&self, cart: MinecartRef) -> Option<String> {
        let cart = self.native_minecart(cart)?;
        (cart.kind == CartKind::CommandBlock).then(|| cart.command.command.clone())
    }

    /// Sets a command block minecart's command (`BaseCommandBlock.setCommand`, which resets the
    /// success count), then `MinecartCommandBase.onUpdated`, as the edit screen's save does: the
    /// synced command and last output follow, so clients' editors show the new command. The
    /// output and its tracking are kept, and no chat note is sent. An activator rail then runs
    /// the new command. False for another kind of cart.
    pub(in crate::game) fn minecart_set_command(
        &mut self,
        cart: MinecartRef,
        command: &str,
    ) -> bool {
        let Some(id) = self
            .native_minecart(cart)
            .filter(|c| c.kind == CartKind::CommandBlock)
            .map(|c| c.id)
        else {
            return false;
        };
        self.native_minecart_edit(cart, None, |c| c.command.set_command(command));
        self.cart_command_updated(id);
        true
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
