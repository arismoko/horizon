//! Ordered raw/ordinary receiving turns and physical input ownership.
use super::*;

pub(super) struct PacketDrain {
    pub(super) conns: Vec<u32>,
    pub(super) next: usize,
    pub(super) started: bool,
    // Ordinary action deferral survives raw yields until this guest turn ends.
    pub(super) movement_held: bool,
    // While a spectator's turn travels, remember where to finish the other
    // connections in this pass.
    spectator_source: Option<String>,
}

impl Hub {
    fn drains_in(&self, g: &Guest) -> bool {
        self.watches(g) || (g.respawning && g.dimension == self.config.dimension)
    }

    /// Handle the players in this level's packets (`ServerGamePacketListenerImpl`
    /// for each), before the level's scheduled block ticks so a place landed in
    /// the same tick as a falling block's schedule still sees the support
    /// (#4510). `tick_level` then syncs what those packets (and the block ticks)
    /// changed.
    pub fn drain_packets(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        let mut events = Vec::new();
        let mut pass = self.packet_drain.take().unwrap_or_else(|| PacketDrain {
            conns: self
                .guests
                .iter()
                .filter(|g| self.drains_in(g))
                .map(|g| g.conn)
                .collect(),
            next: 0,
            started: false,
            movement_held: false,
            spectator_source: None,
        });
        if !pass.started {
            // A travelling spectator's turn ended and the original level is back.
            pass.spectator_source = None;
        }
        while let Some(&conn) = pass.conns.get(pass.next) {
            // A game turn may close the physical endpoint before its Left event
            // reaches the next frozen poll. Do not apply input in that incarnation.
            if let Some(i) = self.guests.connection_index(conn)
                && !self.discard_closed_input(i)
                && self.drains_in(&self.guests[i])
            {
                let g = &self.guests[i];
                let frozen = g.handover.is_some() || g.held || g.respawning;
                if frozen {
                    self.keep_acknowledgements(i, &mut events);
                }
                if self.process_inbox(
                    i,
                    ctx,
                    &mut events,
                    !pass.started,
                    &mut pass.movement_held,
                    &mut pass.spectator_source,
                ) {
                    pass.started = true;
                    self.packet_drain = Some(pass);
                    return events;
                }
                if !frozen && self.here(&self.guests[i]) {
                    self.tick_floating(i, &mut events);
                }
            }
            pass.next += 1;
            pass.started = false;
            pass.movement_held = false;
            if pass
                .spectator_source
                .as_deref()
                .is_some_and(|source| source != self.config.dimension.as_str())
            {
                self.packet_drain = Some(pass);
                return events;
            }
            pass.spectator_source = None;
        }
        events
    }

    /// A UUID teleport resumes the same connection in its new level, then restores
    /// the source level before the pass advances to the next connection.
    pub fn packet_resume_dimension(&self) -> Option<&str> {
        let pass = self.packet_drain.as_ref()?;
        let source = pass.spectator_source.as_deref()?;
        if pass.started
            && let Some(guest) = self.guest(pass.conns[pass.next])
        {
            return Some(&guest.dimension);
        }
        Some(source)
    }

    /// A game turn must finish before continuing the packet pass.
    /// Resuming drains only the frozen inbox turn: no poll, maintenance or world tick.
    pub fn packets_paused(&self) -> bool {
        self.packet_drain.is_some()
    }

    pub(super) fn bind_raw_player(&self, guest: &Guest, entity_id: i32) {
        if let Some(epoch) = self.payload_player_epoch {
            guest
                .sender
                .payload_transport()
                .bind_player(mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(epoch, entity_id as u32),
                    guest.uuid,
                ));
        }
    }

    pub(super) fn poll_raw_payload(
        &mut self,
        delivery: Box<crate::raw_payloads::Delivery>,
        credit: crate::raw_payloads::ingress::ReceiveLease<NetEvent>,
        events: &mut Vec<HostEvent>,
        credits: &mut Vec<crate::raw_payloads::ingress::ReceiveLease<NetEvent>>,
    ) {
        if let Some(conn) = delivery.conn {
            self.update_guest(conn, |g| {
                if g.left_reason.is_empty() {
                    g.inbox.push_back(InboxItem::Raw(delivery, credit));
                }
            });
        } else {
            events.push(HostEvent::RawPayload(delivery));
            credits.push(credit);
        }
    }
}

impl Hub {
    pub(super) fn discard_closed_input(&mut self, i: usize) -> bool {
        // The I/O owner can also close concurrently between packets.
        if !self.guests[i].left_reason.is_empty()
            || self.guests[i].sender.payload_transport().closed()
        {
            let discarded = self.guests.update(i, |g| std::mem::take(&mut g.inbox));
            drop(discarded);
            true
        } else {
            false
        }
    }
}

pub(super) fn yield_raw(
    delivery: Box<crate::raw_payloads::Delivery>,
    credit: crate::raw_payloads::ingress::ReceiveLease<NetEvent>,
    events: &mut Vec<HostEvent>,
    credits: &mut Vec<crate::raw_payloads::ingress::ReceiveLease<NetEvent>>,
) -> bool {
    credits.push(credit);
    if !delivery.incoming.epoch.active() {
        return false;
    }
    events.push(HostEvent::RawPayload(delivery));
    events.push(HostEvent::InputCredits(std::mem::take(credits)));
    true
}
