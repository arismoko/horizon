//! A moving collider as the game keeps it: the boxes and pose a mod gave an entity
//! (`set_collider`), stored as the clients receive them, with the grid the collision queries use.
//!
//! The server keeps one per collider entity and a client keeps the copy its tracking server
//! sent. Both hold the boxes as `f32`, rounded when the mod sets them, so the two sides
//! collide against the same numbers.
use mod_api::{Collider, ColliderBox, ColliderError, ColliderPose};
use std::sync::Arc;
use sunrise_core::{
    geometry::{
        Aabb, Vec3,
        oriented::{self, ColliderBoxes, Frame, OrientedCollider},
    },
    nbt::{Compound, Tag},
};
use sunrise_protocol::collider_sync::{self, WireBox};

/// How long ago the collider carrying the local player last changed, as the player's own client
/// moves it with the copy it holds. The server is told it before each position report
/// (`sunrise:carry_age`) as where to start its search of the collider's recent transforms.
#[derive(Debug, Default)]
pub struct CarryAge {
    held: Option<(u32, Frame, u8)>,
}

impl CarryAge {
    /// Notes the collider carrying the player this tick and its frame as the client holds it
    /// (`None`: nothing carries the player): each tick without a change makes the copy a tick
    /// older.
    pub fn observe(&mut self, carrier: Option<(u32, Frame)>) {
        self.held = match (carrier, self.held) {
            (None, _) => None,
            (Some((id, frame)), Some((held, was, ticks))) if held == id && was == frame => {
                Some((id, frame, ticks.saturating_add(1)))
            }
            (Some((id, frame)), _) => Some((id, frame, 0)),
        };
    }

    /// What to tell the server, while a collider carries the player.
    pub fn hint(&self) -> Option<sunrise_protocol::carry_age::Hint> {
        self.held
            .map(|(collider, _, ticks)| sunrise_protocol::carry_age::Hint { collider, ticks })
    }
}

#[derive(Clone, Debug)]
pub struct ColliderState {
    wire: Arc<Vec<WireBox>>,
    rotation: [f32; 4],
    pivot: [f64; 3],
    geometry: Arc<ColliderBoxes>,
    /// Changes whenever the boxes or the pose do, so a placement made from this state can
    /// be kept while the number is the same.
    serial: u64,
}

fn next_serial() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn aabb(b: &WireBox) -> Aabb {
    Aabb::new(
        Vec3::new(
            f64::from(b.min[0]),
            f64::from(b.min[1]),
            f64::from(b.min[2]),
        ),
        Vec3::new(
            f64::from(b.max[0]),
            f64::from(b.max[1]),
            f64::from(b.max[2]),
        ),
    )
}

/// Validates and normalizes a [`ColliderPose`].
/// Refuses a non-finite pivot or a zero, NaN or infinite quaternion with
/// `ColliderError::NonFinite { index: None }`.
pub fn normalize_pose(pose: ColliderPose) -> Result<ColliderPose, ColliderError> {
    if !oriented::check_pivot(pose.pivot) {
        return Err(ColliderError::NonFinite { index: None });
    }
    let rotation = oriented::normalize_rotation(pose.rotation)
        .ok_or(ColliderError::NonFinite { index: None })?;
    Ok(ColliderPose::new(rotation, pose.pivot))
}

impl ColliderState {
    /// A collider from boxes and a pose already checked (the wire's or a save's).
    pub fn new(wire: Vec<WireBox>, rotation: [f32; 4], pivot: [f64; 3]) -> Self {
        let geometry = Arc::new(ColliderBoxes::new(wire.iter().map(aabb).collect()));
        Self {
            wire: Arc::new(wire),
            rotation,
            pivot,
            geometry,
            serial: next_serial(),
        }
    }

    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// Checks a mod's collider and keeps it as `f32`: refused by name, never clamped or truncated.
    pub fn from_api(collider: &Collider) -> Result<Self, ColliderError> {
        let pose = normalize_pose(collider.pose)?;
        let bytes = collider_sync::encoded_bytes(collider.boxes.len());
        if bytes > collider_sync::MAX_PAYLOAD_BYTES {
            return Err(ColliderError::TooLarge {
                bytes: bytes as u64,
                limit: collider_sync::MAX_PAYLOAD_BYTES as u64,
            });
        }
        let mut wire = Vec::with_capacity(collider.boxes.len());
        for (index, b) in collider.boxes.iter().enumerate() {
            if !oriented::check_box(b.min, b.max) {
                return Err(ColliderError::NonFinite {
                    index: Some(index as u64),
                });
            }
            let low = |i: usize| b.min[i].min(b.max[i]) as f32;
            let high = |i: usize| b.min[i].max(b.max[i]) as f32;
            wire.push(WireBox {
                min: [low(0), low(1), low(2)],
                max: [high(0), high(1), high(2)],
            });
        }
        Ok(Self::new(wire, pose.rotation, pose.pivot))
    }

    /// The collider as a mod reads it back.
    pub fn to_api(&self) -> Collider {
        Collider::new(
            self.wire
                .iter()
                .map(|b| ColliderBox {
                    min: b.min.map(f64::from),
                    max: b.max.map(f64::from),
                })
                .collect(),
        )
        .with_pose(self.pose())
    }

    pub fn wire(&self) -> &[WireBox] {
        &self.wire
    }

    pub fn pose(&self) -> ColliderPose {
        ColliderPose::new(self.rotation, self.pivot)
    }

    pub fn rotation(&self) -> [f32; 4] {
        self.rotation
    }

    pub fn pivot(&self) -> [f64; 3] {
        self.pivot
    }

    pub fn set_pose(&mut self, pose: ColliderPose) {
        self.rotation = pose.rotation;
        self.pivot = pose.pivot;
        self.serial = next_serial();
    }

    pub fn geometry(&self) -> &ColliderBoxes {
        &self.geometry
    }

    /// The frame of the collider for an entity at `origin`.
    pub fn frame(&self, origin: Vec3) -> Frame {
        Frame::from_pose(
            origin,
            Vec3::new(self.pivot[0], self.pivot[1], self.pivot[2]),
            self.rotation.map(f64::from),
        )
    }

    /// The collider placed at `origin`, for the collision queries.
    pub fn oriented(&self, origin: Vec3) -> OrientedCollider {
        OrientedCollider::new(self.frame(origin), self.geometry.clone())
    }

    /// The world box holding the collider at `origin`, or None for no boxes.
    pub fn world_bounds(&self, origin: Vec3) -> Option<Aabb> {
        self.oriented(origin).world_bounds()
    }

    /// The messages that give a client the whole collider.
    pub fn messages(&self, entity: u32) -> Vec<collider_sync::Message> {
        collider_sync::whole(entity, self.rotation, self.pivot, &self.wire)
    }

    /// Applies a message about this collider's entity as a client does. A run that does not
    /// follow the ones before it, or a patch outside the boxes held, is dropped.
    pub fn apply(held: &mut Option<Self>, message: &collider_sync::Message) {
        use collider_sync::Message as M;
        match message {
            M::Boxes {
                rotation,
                pivot,
                total,
                first,
                boxes,
                ..
            } => {
                let mut wire = if *first == 0 {
                    Vec::new()
                } else {
                    match held.take() {
                        Some(state) if state.wire.len() == *first as usize => {
                            Arc::unwrap_or_clone(state.wire)
                        }
                        _ => return,
                    }
                };
                wire.extend_from_slice(boxes);
                if wire.len() > *total as usize || wire.len() > collider_sync::MAX_BOXES_IN_PAYLOAD
                {
                    return;
                }
                *held = Some(Self::new(wire, *rotation, *pivot));
            }
            M::Patch {
                truncate,
                replaced,
                appended,
                ..
            } => {
                let Some(state) = held.as_mut() else { return };
                let mut wire = Arc::unwrap_or_clone(std::mem::take(&mut state.wire));
                if let Some(n) = truncate {
                    wire.truncate(*n as usize);
                }
                for (i, b) in replaced {
                    if let Some(slot) = wire.get_mut(*i as usize) {
                        *slot = *b;
                    }
                }
                wire.extend_from_slice(appended);
                wire.truncate(collider_sync::MAX_BOXES_IN_PAYLOAD);
                *state = Self::new(wire, state.rotation, state.pivot);
            }
            M::Pose {
                rotation, pivot, ..
            } => {
                if let Some(state) = held.as_mut() {
                    state.set_pose(ColliderPose::new(*rotation, *pivot));
                }
            }
            M::Clear { .. } => *held = None,
        }
    }

    /// The collider as a save holds it: the rotation, the pivot, and the box corners as
    /// big-endian `f32`s.
    pub fn to_nbt(&self) -> Compound {
        let mut corners = Vec::with_capacity(self.wire.len() * 24);
        for b in self.wire.iter() {
            b.min
                .iter()
                .chain(&b.max)
                .for_each(|v| corners.extend_from_slice(&v.to_be_bytes()));
        }
        [
            (
                "rotation".into(),
                Tag::List(
                    self.rotation
                        .iter()
                        .map(|v| Tag::Float(v.to_bits()))
                        .collect(),
                ),
            ),
            (
                "pivot".into(),
                Tag::List(
                    self.pivot
                        .iter()
                        .map(|v| Tag::Double(v.to_bits()))
                        .collect(),
                ),
            ),
            ("corners".into(), Tag::ByteArray(corners)),
        ]
        .into_iter()
        .collect()
    }

    /// A saved collider, or None for one that is malformed or exceeds the payload limit
    /// (a save is not trusted).
    pub fn from_nbt(tag: &Tag) -> Option<Self> {
        let Some(Tag::List(rotation)) = tag.get("rotation") else {
            return None;
        };
        let rotation: Vec<f32> = rotation
            .iter()
            .filter_map(|t| match t {
                Tag::Float(bits) => Some(f32::from_bits(*bits)),
                _ => None,
            })
            .collect();
        let rotation: [f32; 4] = rotation.try_into().ok()?;
        let rotation = oriented::normalize_rotation(rotation)?;
        let pivot = match tag.get("pivot") {
            None => [0.0; 3],
            Some(Tag::List(list)) => {
                let values: Vec<f64> = list
                    .iter()
                    .filter_map(|t| match t {
                        Tag::Double(bits) => Some(f64::from_bits(*bits)),
                        _ => None,
                    })
                    .collect();
                let pivot: [f64; 3] = values.try_into().ok()?;
                if !oriented::check_pivot(pivot) {
                    return None;
                }
                pivot
            }
            Some(_) => return None,
        };
        let Some(Tag::ByteArray(corners)) = tag.get("corners") else {
            return None;
        };
        if corners.len() % 24 != 0
            || collider_sync::encoded_bytes(corners.len() / 24) > collider_sync::MAX_PAYLOAD_BYTES
        {
            return None;
        }
        let mut wire = Vec::with_capacity(corners.len() / 24);
        for at in corners.as_chunks::<24>().0 {
            let mut v = [0.0f32; 6];
            for (slot, bytes) in v.iter_mut().zip(at.as_chunks::<4>().0) {
                *slot = f32::from_be_bytes(*bytes);
            }
            let wide = v.map(f64::from);
            if !oriented::check_box([wide[0], wide[1], wide[2]], [wide[3], wide[4], wide[5]]) {
                return None;
            }
            wire.push(WireBox {
                min: [v[0], v[1], v[2]],
                max: [v[3], v[4], v[5]],
            });
        }
        Some(Self::new(wire, rotation, pivot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::geometry::Axis;

    #[test]
    fn the_carry_age_counts_ticks_without_a_change_and_starts_over_with_one() {
        let at = |x: f64| Frame::translation(Vec3::new(x, 0.0, 0.0));
        let mut age = CarryAge::default();
        assert_eq!(age.hint(), None);
        let ticks = |age: &CarryAge| age.hint().map(|h| (h.collider, h.ticks));
        age.observe(Some((7, at(0.0))));
        assert_eq!(ticks(&age), Some((7, 0)));
        age.observe(Some((7, at(0.0))));
        age.observe(Some((7, at(0.0))));
        assert_eq!(ticks(&age), Some((7, 2)));
        age.observe(Some((7, at(2.0))));
        assert_eq!(ticks(&age), Some((7, 0)));
        age.observe(Some((9, at(2.0))));
        assert_eq!(ticks(&age), Some((9, 0)));
        age.observe(None);
        assert_eq!(age.hint(), None);
    }

    fn collider(n: usize) -> Collider {
        Collider::new(
            (0..n)
                .map(|i| ColliderBox {
                    min: [(i % 400) as f64, (i / 400 % 100) as f64, 0.0],
                    max: [(i % 400) as f64 + 1.0, (i / 400 % 100) as f64 + 1.0, 1.0],
                })
                .collect(),
        )
    }

    #[test]
    fn a_collider_is_stored_and_read_back_as_f32() {
        let mut c = collider(2).with_pivot([1.5, -2.0, 3.0]);
        c.boxes[0].min = [0.1, 0.0, 0.0];
        let state = ColliderState::from_api(&c).unwrap();
        let back = state.to_api();
        assert_eq!(back.boxes[0].min[0], f64::from(0.1_f32));
        assert_eq!(back.boxes.len(), 2);
        assert_eq!(back.pose.pivot, [1.5, -2.0, 3.0]);
        // An inverted box is read with its corners swapped.
        let mut c = collider(1);
        c.boxes[0].min = [1.0, 1.0, 1.0];
        c.boxes[0].max = [0.0, 0.0, 0.0];
        let back = ColliderState::from_api(&c).unwrap().to_api();
        assert_eq!(back.boxes[0].min, [0.0; 3]);
        assert_eq!(back.boxes[0].max, [1.0; 3]);
    }

    /// Every refusal is by name, and none changes what was held.
    #[test]
    fn bad_colliders_are_refused_by_name() {
        let mut c = collider(3);
        c.boxes[2].max[1] = f64::NAN;
        assert_eq!(
            ColliderState::from_api(&c).unwrap_err(),
            ColliderError::NonFinite { index: Some(2) }
        );
        // Zero, NaN, and infinite quaternions are refused by name; non-unit finite quaternions
        // are normalized when set.
        for bad_q in [
            [0.0, 0.0, 0.0, 0.0],
            [f32::NAN, 0.0, 0.0, 1.0],
            [0.0, f32::INFINITY, 0.0, 1.0],
        ] {
            let c = collider(1).with_rotation(bad_q);
            assert_eq!(
                ColliderState::from_api(&c).unwrap_err(),
                ColliderError::NonFinite { index: None },
                "{bad_q:?}"
            );
        }
        let non_unit = collider(1).with_rotation([0.0, 0.0, 0.0, 2.0]);
        assert_eq!(
            ColliderState::from_api(&non_unit).unwrap().rotation(),
            [0.0, 0.0, 0.0, 1.0]
        );
        // Non-finite pivot is refused by name.
        let bad_pivot = collider(1).with_pivot([0.0, f64::NAN, 0.0]);
        assert_eq!(
            ColliderState::from_api(&bad_pivot).unwrap_err(),
            ColliderError::NonFinite { index: None }
        );
        // Far boxes are allowed (no artificial reach cap).
        let mut far = collider(1);
        far.boxes[0].max[0] = 2000.0;
        assert!(ColliderState::from_api(&far).is_ok());
        // At the payload limit is accepted; one box past it is refused with TooLarge.
        let at_cap = collider(collider_sync::MAX_BOXES_IN_PAYLOAD);
        assert!(ColliderState::from_api(&at_cap).is_ok());
        let mut over = at_cap;
        over.boxes.push(over.boxes[0]);
        assert_eq!(
            ColliderState::from_api(&over).unwrap_err(),
            ColliderError::TooLarge {
                bytes: collider_sync::encoded_bytes(over.boxes.len()) as u64,
                limit: collider_sync::MAX_PAYLOAD_BYTES as u64,
            }
        );
        // Corners that differ as f64 but are the same f32 make a box with no volume.
        let mut thin = collider(0);
        thin.boxes.push(ColliderBox {
            min: [1.0, 0.0, 0.0],
            max: [1.0 + 1.0e-9, 2.0, 2.0],
        });
        thin.boxes.push(ColliderBox {
            min: [0.0, 1.0, 0.0],
            max: [3.0, 1.0 + 1.0e-9, 3.0],
        });
        let state = ColliderState::from_api(&thin).unwrap();
        assert_eq!(state.wire()[0].min[0], state.wire()[0].max[0]);
        let placed = state.oriented(Vec3::ZERO);
        let mover = Aabb::new(Vec3::new(-0.6, 0.0, 0.5), Vec3::new(-0.2, 1.8, 1.0));
        assert_eq!(placed.collide(mover, Axis::X, 4.0), 4.0);
        let mut heights = Vec::new();
        placed.step_heights(mover, Vec3::new(1.0, 0.0, 0.0), 2.0, &mut heights);
        assert!(heights.is_empty(), "{heights:?}");
        // Save round-trip preserves pivot; malformed saves are dropped.
        let saved = ColliderState::from_api(&collider(2).with_pivot([1.0, 2.0, 3.0]))
            .unwrap()
            .to_nbt();
        let loaded = ColliderState::from_nbt(&Tag::Compound(saved.clone())).unwrap();
        assert_eq!(loaded.pivot(), [1.0, 2.0, 3.0]);
        let mut zero_q = saved.clone();
        zero_q.insert("rotation".into(), Tag::List(vec![Tag::Float(0); 4]));
        assert!(ColliderState::from_nbt(&Tag::Compound(zero_q)).is_none());
        let mut nan_pivot = saved;
        nan_pivot.insert(
            "pivot".into(),
            Tag::List(vec![
                Tag::Double(f64::NAN.to_bits()),
                Tag::Double(0),
                Tag::Double(0),
            ]),
        );
        assert!(ColliderState::from_nbt(&Tag::Compound(nan_pivot)).is_none());
    }

    /// What the server sends, applied as a client does, gives the server's collider.
    #[test]
    fn a_client_copy_follows_the_whole_and_the_changes() {
        let server = ColliderState::from_api(
            &collider(40_000_usize.min(collider_sync::MAX_BOXES_IN_PAYLOAD))
                .with_pivot([2.0, 0.5, -1.0]),
        )
        .unwrap();
        let mut held = None;
        for m in server.messages(7) {
            ColliderState::apply(&mut held, &m);
        }
        let held_state = held.clone().unwrap();
        assert_eq!(held_state.wire(), server.wire());
        assert_eq!(held_state.pivot(), server.pivot());
        // A change: one box replaced, one cut, a new pose.
        let mut next = server.to_api();
        next.boxes[5].max[1] = 3.0;
        next.boxes.pop();
        next = next.with_pose(ColliderPose::new(
            [
                0.0,
                std::f32::consts::FRAC_1_SQRT_2,
                0.0,
                std::f32::consts::FRAC_1_SQRT_2,
            ],
            [3.0, 1.0, 0.0],
        ));
        let next = ColliderState::from_api(&next).unwrap();
        for m in collider_sync::changes(7, server.wire(), next.wire()) {
            ColliderState::apply(&mut held, &m);
        }
        ColliderState::apply(
            &mut held,
            &collider_sync::Message::Pose {
                entity: 7,
                rotation: next.rotation(),
                pivot: next.pivot(),
            },
        );
        let held = held.unwrap();
        assert_eq!(held.wire(), next.wire());
        assert_eq!(held.pose(), next.pose());
        // A run out of order, and a patch with nothing held, change nothing.
        let mut none = None;
        ColliderState::apply(
            &mut none,
            &collider_sync::Message::Patch {
                entity: 7,
                truncate: None,
                replaced: vec![],
                appended: next.wire()[..1].to_vec(),
            },
        );
        assert!(none.is_none());
        let mut late = None;
        ColliderState::apply(
            &mut late,
            &collider_sync::Message::Boxes {
                entity: 7,
                rotation: [0.0, 0.0, 0.0, 1.0],
                pivot: [0.0; 3],
                total: 5,
                first: 2,
                boxes: next.wire()[..1].to_vec(),
            },
        );
        assert!(late.is_none());
        // A clear drops it.
        let mut some = Some(next);
        ColliderState::apply(&mut some, &collider_sync::Message::Clear { entity: 7 });
        assert!(some.is_none());
    }
}
