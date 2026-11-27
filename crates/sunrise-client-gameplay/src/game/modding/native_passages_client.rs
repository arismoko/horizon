use crate::game::ClientGame;
use mod_api::PassageId;
use sunrise_gameplay_common::game::native_passages::{HeldRefusal, MAX_PER_LEVEL, check, passage};
use sunrise_protocol::passage_sync::{Message, PassageForm};

impl ClientGame<'_> {
    /// The window forgot chunk `(x, z)`: the passages of that chunk go with it.
    pub(in crate::game::client_side) fn forget_chunk_passages(&mut self, x: i32, z: i32) {
        if !self.common.passages.held.is_empty() {
            self.common.passages.held.retain(|_, p| p.chunk() != [x, z]);
        }
    }

    /// A `sunrise:passage_sync` payload the server sent the window.
    pub fn remote_passage_sync(&mut self, data: &[u8]) -> Vec<(u64, HeldRefusal)> {
        let message = match sunrise_protocol::passage_sync::decode(data) {
            Ok(message) => message,
            Err(e) => {
                log::debug!("Sunrise: a bad passage sync: {e}");
                return Vec::new();
            }
        };
        let mut refused = Vec::new();
        match message {
            Message::Add(forms) => {
                for f in forms {
                    if let Err(why) = self.hold_passage(&f) {
                        log::debug!("Sunrise: passage {} not kept: {why:?}", f.id);
                        refused.push((f.id, why));
                    }
                }
            }
            Message::Remove(ids) => {
                for id in ids {
                    self.common.passages.held.remove(&PassageId::from_raw(id));
                }
            }
            Message::Refused { id, reason } => self.remote_passage_refused(id, reason),
        }
        refused
    }

    pub(in crate::game::client_side) fn hold_passage(
        &mut self,
        f: &PassageForm,
    ) -> Result<(), HeldRefusal> {
        let p = passage(f);
        check(&p).map_err(HeldRefusal::Bad)?;
        let [x, z] = p.chunk();
        if !self.common.world.chunks.contains_key(&(x, z)) {
            return Err(HeldRefusal::ChunkNotHeld);
        }
        let held = &mut self.common.passages.held;
        let id = PassageId::from_raw(f.id);
        if !held.contains_key(&id) && held.len() >= MAX_PER_LEVEL as usize {
            return Err(HeldRefusal::Full);
        }
        held.insert(id, p);
        Ok(())
    }
}
