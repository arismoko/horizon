use std::collections::HashMap;

/// One edge of the riding graph sent by `SetPassengers`.
pub struct PassengerSeat {
    pub vehicle: u32,
    pub index: usize,
}

pub fn passenger_chain(
    vehicles: &HashMap<u32, PassengerSeat>,
    id: u32,
) -> impl Iterator<Item = u32> + '_ {
    std::iter::successors(Some(id), |id| vehicles.get(id).map(|seat| seat.vehicle))
        .take(vehicles.len() + 2)
}

/// Walks to the root without allocating. In a cycle, returns the last reached
/// ID for the collision code's bounded stack walk; there is no actual root.
pub fn passenger_root(
    vehicles: &HashMap<u32, PassengerSeat>,
    id: u32,
) -> std::result::Result<u32, u32> {
    let root = passenger_chain(vehicles, id).last().unwrap_or(id);
    if vehicles.contains_key(&root) {
        Err(root)
    } else {
        Ok(root)
    }
}
