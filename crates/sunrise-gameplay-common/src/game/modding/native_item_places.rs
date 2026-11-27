#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use mod_api::ServerHost;

    use mod_api::{ItemResource, TransactionError};

    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    use sunrise_core::geometry::Direction;

    use sunrise_items::items::ItemStack;

    use crate::game::api_values::TryIntoApi;

    use sunrise_protocol::packets::configuration::ClientInformation;

    use sunrise_sim::lan::{Joined, PlayerSender, guest::Guest};

    /// A placed guest on connection `conn`, with UUID and entity id `conn + 122` and
    /// `conn + 19`.
    pub fn guest(conn: u32) -> Guest {
        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn) + 122,
                name: format!("guest{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            conn as i32 + 19,
            [0.0; 3],
            8,
        );
        guest.placed = true;
        guest
    }
    /// One player's offer in a trade: inventory index `.0` must still hold exactly `.1`.
    pub(in crate::game) type Offer = (usize, ItemStack);
    pub fn offers() -> ([Offer; 1], [Offer; 1]) {
        (
            [(0, ItemStack::new("minecraft:diamond", 3))],
            [(0, ItemStack::new("minecraft:emerald", 5))],
        )
    }
    #[cfg(any(test, feature = "test-support"))]
    /// A mod's `ENTITY_AUTOMATION` handler: one slot of up to 64 items, with the sides it was
    /// asked for.
    #[derive(Clone, Default)]
    pub struct Bin {
        pub held: Arc<std::sync::Mutex<Option<(String, u32)>>>,
        pub sides: Arc<std::sync::Mutex<Vec<Option<Direction>>>>,
    }
    #[cfg(any(test, feature = "test-support"))]
    impl mod_api::ResourceHandler<ItemResource> for Bin {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            let held = self.held.lock().unwrap();
            held.as_ref()
                .map(|(item, _)| ItemResource::of(&ItemStack::new(item, 1).try_into_api().unwrap()))
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            self.held.lock().unwrap().as_ref().map_or(0, |(_, n)| *n)
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&ItemResource>) -> u32 {
            64
        }
        fn insert_at(
            &self,
            _: &mut dyn ServerHost,
            _: &mod_api::TransactionContext,
            _: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> Result<u32, TransactionError<mod_api::Overanswered>> {
            let item = mod_api::__private::item_key_id(resource.as_stack().item())
                .unwrap()
                .to_string();
            let mut held = self.held.lock().unwrap();
            Ok(match held.as_mut() {
                Some((have, n)) if *have == item => {
                    let went = amount.min(64 - *n);
                    *n += went;
                    went
                }
                Some(_) => 0,
                None => {
                    let went = amount.min(64);
                    *held = Some((item, went));
                    went
                }
            })
        }
        fn extract_at(
            &self,
            _: &mut dyn ServerHost,
            _: &mod_api::TransactionContext,
            _: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> Result<u32, TransactionError<mod_api::Overanswered>> {
            let item = mod_api::__private::item_key_id(resource.as_stack().item())
                .unwrap()
                .to_string();
            let mut held = self.held.lock().unwrap();
            let Some((_, n)) = held.as_mut().filter(|(have, _)| *have == item) else {
                return Ok(0);
            };
            let came = amount.min(*n);
            *n -= came;
            if *n == 0 {
                *held = None;
            }
            Ok(came)
        }
    }
}
