#[cfg(any(test, feature = "test-support"))]
pub mod join_order_tests {

    use crate::game::{
        boats_common::Boat,
        entity::{EntityHandle, EntityLists, JoinOrderError},
        fireworks::Rocket,
    };
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::{
        entities::ArmorStand, join_seq::JoinList, mobs::Mob, thrown_trident::ThrownTrident,
    };

    #[derive(Default)]
    pub struct Lists {
        pub mobs: JoinList<Mob>,
        pub stands: JoinList<ArmorStand>,
        pub tridents: JoinList<ThrownTrident>,
        pub rockets: JoinList<Rocket>,
        pub boats: JoinList<Boat, false>,
    }

    impl Lists {
        pub fn view(&self) -> EntityLists<'_> {
            EntityLists {
                mobs: &self.mobs,
                dropped: &[],
                boats: &self.boats,
                carts: &[],
                armor_stands: &self.stands,
                tnts: &[],
                falling: &[],
                orbs: &[],
                projectiles: &[],
                rockets: &self.rockets,
                tridents: &self.tridents,
                general: &[],
            }
        }
        pub fn walk(&self) -> Result<Vec<EntityHandle>, JoinOrderError> {
            let mut seen = Vec::new();
            self.view().walk_in_join_order(|h, _| seen.push(h))?;
            Ok(seen)
        }
    }
    /// Joins different kinds of entities interleaved, the way a level gets them.
    pub fn mixed() -> (Lists, Vec<EntityHandle>) {
        let mut l = Lists::default();
        l.trident(1);
        l.rocket(2);
        l.stand(3);
        l.boat(4);
        l.trident(5);
        l.stand(6);
        l.rocket(7);
        let joined = vec![
            EntityHandle::Trident(0),
            EntityHandle::Rocket(0),
            EntityHandle::ArmorStand(0),
            EntityHandle::Boat(0),
            EntityHandle::Trident(1),
            EntityHandle::ArmorStand(1),
            EntityHandle::Rocket(1),
        ];
        (l, joined)
    }
    impl Lists {
        pub fn stand(&mut self, id: u32) {
            self.stands.push(ArmorStand::new(id, at(), 0.0));
        }
        pub fn trident(&mut self, id: u32) {
            self.tridents
                .push(ThrownTrident::remote(id, at(), 0.0, 0.0));
        }
        pub fn rocket(&mut self, id: u32) {
            self.rockets.push(Rocket::remote(id, at(), Vec3::ZERO));
        }
        pub fn boat(&mut self, id: u32) {
            self.boats.push(Boat::new(id, "oak".into(), at(), 0.0));
        }
    }

    pub fn at() -> Vec3 {
        Vec3::new(0.5, 100.0, 0.5)
    }
}
