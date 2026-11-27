//! Worlds saved by Horizon, Sunrise's earlier name, read with their mods' attachments: a save
//! upgrade, so mods only ever see Sunrise's names. Horizon kept a holder's attachments under
//! `horizon:attachments` and a dimension's in `data/horizon/attachments.dat`; the next save writes
//! [`ATTACHMENTS_KEY`] and `data/sunrise/attachments.dat`.
use super::*;

/// The key Horizon saved a holder's attachments under.
const HORIZON_ATTACHMENTS_KEY: &str = "horizon:attachments";

/// A dimension's attachments file as Horizon named it, under the dimension folder `root`.
pub(super) fn horizon_attachments_path(root: &Path) -> PathBuf {
    root.join("data").join("horizon").join("attachments.dat")
}

/// A chunk (`region/`) or entity (`entities/`) record: the chunk and its block entities, or
/// every entity and its passengers.
pub(super) fn adopt_in_record(folder: Folder, record: &mut Tag) {
    match folder {
        Folder::Region => {
            adopt(record);
            if let Some(Tag::List(block_entities)) = record.get_mut("block_entities") {
                block_entities.iter_mut().for_each(adopt);
            }
        }
        Folder::Entities => {
            if let Some(Tag::List(entities)) = record.get_mut("Entities") {
                entities.iter_mut().for_each(adopt_in_entity);
            }
        }
        Folder::Poi => {}
    }
}

/// A player's saved data: the player and what they ride (`RootVehicle.Entity`).
pub(crate) fn adopt_in_player(player: &mut Tag) {
    adopt(player);
    if let Some(vehicle) = player
        .get_mut("RootVehicle")
        .and_then(|v| v.get_mut("Entity"))
    {
        adopt_in_entity(vehicle);
    }
}

/// An entity and its passengers, theirs too, without recursion.
fn adopt_in_entity(entity: &mut Tag) {
    let mut work = vec![entity];
    while let Some(entity) = work.pop() {
        adopt(entity);
        if let Tag::Compound(map) = entity
            && let Some(Tag::List(passengers)) = map.get_mut(&"Passengers".into())
        {
            work.extend(passengers.iter_mut());
        }
    }
}

/// Removes `horizon:attachments` from `holder`, keeping [`ATTACHMENTS_KEY`] if it is already present.
fn adopt(holder: &mut Tag) {
    if let Tag::Compound(map) = holder
        && let Some(saved) = map.remove(&HORIZON_ATTACHMENTS_KEY.into())
    {
        map.entry(ATTACHMENTS_KEY.into()).or_insert(saved);
    }
}
