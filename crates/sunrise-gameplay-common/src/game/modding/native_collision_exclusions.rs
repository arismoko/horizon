use mod_api::CollisionExclusionsError;
use sunrise_world::collision_exclusions::Refused;

pub fn refused(why: Refused) -> CollisionExclusionsError {
    match why {
        Refused::NotFinite => CollisionExclusionsError::NotFinite,
        Refused::Inverted => CollisionExclusionsError::Inverted,
    }
}
