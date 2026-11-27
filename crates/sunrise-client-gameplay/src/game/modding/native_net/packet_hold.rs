//! Mod-owned barriers over one ordered queue for the current physical play incarnation.
use crate::game::ClientGame;
use crate::game::client_side::remote_client::{PendingCauldron, Remote};
use mod_api::{HoldError, HoldId, SendPacketError, content::crash::current_native_mod};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_sim::raw_payloads::queued_writer::PreparedPacket;

static NEXT: AtomicU64 = AtomicU64::new(1);

pub struct HeldPlay {
    packet: PreparedPacket,
    cauldron: Option<(i64, PendingCauldron)>,
}

impl HeldPlay {
    fn publish(self, remote: &Remote) -> Result<(), SendPacketError> {
        self.packet.try_publish()?;
        if let Some((receipt, click)) = self.cauldron {
            remote.pending_cauldrons.borrow_mut().insert(receipt, click);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaySend {
    Admitted,
    Cancelled,
    Unavailable,
}

struct Hold {
    id: HoldId,
    owner: &'static str,
    start: u64,
}

pub struct Holds<P = HeldPlay> {
    incarnation: Option<(u64, u64)>,
    active: Vec<Hold>,
    // Each packet is tagged with the most recently issued hold, even if that
    // hold has since ended. The tags never go backwards within the queue.
    latest: u64,
    packets: VecDeque<(u64, P)>,
}

impl<P> Default for Holds<P> {
    fn default() -> Self {
        Self {
            incarnation: None,
            active: Vec::new(),
            latest: 0,
            packets: VecDeque::new(),
        }
    }
}

impl Holds<HeldPlay> {
    pub(in crate::game::client_side) fn clear_cauldron_continuations(&mut self) {
        for (_, packet) in &mut self.packets {
            packet.cauldron = None;
        }
    }
}

impl<P> Holds<P> {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn sync(&mut self, incarnation: Option<(u64, u64)>) {
        if self.incarnation != incarnation {
            self.clear();
            self.incarnation = incarnation;
        }
    }

    pub fn start(&mut self, owner: &'static str) -> Result<HoldId, HoldError> {
        if self.incarnation.is_none() {
            return Err(HoldError::NoWorld);
        }
        let start = NEXT.fetch_add(1, Ordering::Relaxed);
        let id = HoldId::new(start);
        self.active.push(Hold { id, owner, start });
        self.latest = start;
        Ok(id)
    }

    fn end(&mut self, owner: &'static str, id: HoldId) -> Result<Hold, HoldError> {
        let index = self
            .active
            .iter()
            .position(|hold| hold.id == id)
            .ok_or(HoldError::Unknown)?;
        if self.active[index].owner != owner {
            return Err(HoldError::WrongOwner);
        }
        Ok(self.active.remove(index))
    }

    pub fn queue(&mut self, packet: P) {
        self.packets.push_back((self.latest, packet));
    }

    pub fn release(
        &mut self,
        owner: &'static str,
        id: HoldId,
        mut publish: impl FnMut(P) -> Result<(), SendPacketError>,
    ) -> Result<usize, HoldError> {
        self.end(owner, id)?;
        let ready = self.active.first().map_or(self.packets.len(), |hold| {
            self.packets.partition_point(|(tag, _)| *tag < hold.start)
        });
        let result = self
            .packets
            .drain(..ready)
            .enumerate()
            .try_for_each(|(sent, (_, packet))| {
                publish(packet).map_err(|error| HoldError::Send { sent, error })
            });
        if matches!(
            result,
            Err(HoldError::Send {
                error: SendPacketError::NotInPlay | SendPacketError::NotConnected,
                ..
            })
        ) {
            self.clear();
        }
        result.map(|()| ready)
    }

    pub fn discard(&mut self, owner: &'static str, id: HoldId) -> Result<(), HoldError> {
        let start = self.end(owner, id)?.start;
        // This hold was active for every packet admitted from `start` onward,
        // including overlapping packets. Older packets and other leases stay put.
        let keep = self.packets.partition_point(|(tag, _)| *tag < start);
        self.packets.truncate(keep);
        Ok(())
    }
}

#[cfg(feature = "client")]
impl ClientGame<'_> {
    fn hold_incarnation(&self) -> Option<(u64, u64)> {
        self.client
            .remote
            .as_ref()
            .filter(|r| r.joined && r.reconfiguring.is_none())?
            .session
            .hold_incarnation()
    }
    pub fn hold_packets(&mut self) -> Result<HoldId, HoldError> {
        let owner = current_native_mod().ok_or(HoldError::NoOwner)?;
        let mut holds = self.client.packet_holds.borrow_mut();
        holds.sync(self.hold_incarnation());
        holds.start(owner)
    }
    pub fn release_held(&mut self, hold: HoldId) -> Result<usize, HoldError> {
        let owner = current_native_mod().ok_or(HoldError::NoOwner)?;
        let mut holds = self.client.packet_holds.borrow_mut();
        holds.sync(self.hold_incarnation());
        let remote = self.client.remote.as_ref().ok_or(HoldError::Unknown)?;
        holds.release(owner, hold, |packet| packet.publish(remote))
    }
    pub fn drop_held(&mut self, hold: HoldId) -> Result<(), HoldError> {
        let owner = current_native_mod().ok_or(HoldError::NoOwner)?;
        let mut holds = self.client.packet_holds.borrow_mut();
        holds.sync(self.hold_incarnation());
        holds.discard(owner, hold)
    }

    pub(in crate::game) fn publish_mod_packet(
        &self,
        packet: PreparedPacket,
        owner: Option<&'static str>,
    ) {
        let mut holds = self.client.packet_holds.borrow_mut();
        if !holds.active.is_empty() {
            holds.sync(self.hold_incarnation());
            if !holds.active.is_empty() && holds.incarnation == Some(packet.incarnation()) {
                holds.queue(HeldPlay {
                    packet,
                    cauldron: None,
                });
                return;
            }
        }
        let result = if self
            .client
            .remote
            .as_ref()
            .is_some_and(|remote| remote.session.serial() == packet.incarnation().0)
        {
            packet.try_publish()
        } else {
            Err(SendPacketError::NotConnected)
        };
        if let Err(error) = result {
            let owner = owner.unwrap_or("game");
            log::warn!("{owner}: send_packet publication refused: {error:?}");
        }
    }
    /// The logical play packet already passed the client send hook. A cauldron
    /// continuation travels with its envelope; only a published packet expects a reply.
    pub(in crate::game::client_side) fn send_or_hold_play(
        &self,
        packet: S,
        cauldron: Option<(i64, PendingCauldron)>,
    ) -> PlaySend {
        let Some(remote) = &self.client.remote else {
            return PlaySend::Unavailable;
        };
        let mut holds = self.client.packet_holds.borrow_mut();
        if !holds.active.is_empty() || cauldron.is_some() {
            holds.sync(self.hold_incarnation());
        }
        if holds.active.is_empty() && cauldron.is_none() {
            return if remote.session.send(packet) {
                PlaySend::Admitted
            } else {
                PlaySend::Unavailable
            };
        }
        let Some((_, generation)) = holds.incarnation else {
            return PlaySend::Unavailable;
        };
        let result = remote
            .session
            .prepare_held_play(packet, generation)
            .and_then(|packet| {
                let packet = HeldPlay { packet, cauldron };
                if holds.active.is_empty() {
                    packet.publish(remote)
                } else {
                    holds.queue(packet);
                    Ok(())
                }
            });
        match result {
            Ok(()) => PlaySend::Admitted,
            Err(error) => {
                log::warn!("held play packet refused: {error:?}");
                holds.sync(self.hold_incarnation());
                PlaySend::Unavailable
            }
        }
    }
}
#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// `Connection.send` for the game's play packets: a mod may cancel one
    /// (`client_packet_send`), and the server never hears of it.
    pub fn send_play(&self, packet: S) -> PlaySend {
        if self.client.remote.is_none() {
            return PlaySend::Unavailable;
        }
        if self.client_packet_send_canceled(&packet) {
            return PlaySend::Cancelled;
        }
        self.send_or_hold_play(packet, None)
    }
}
