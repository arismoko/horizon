use super::*;

#[test]
fn players_receive_another_players_initial_main_hand_and_changes() {
    let (mut hub, viewer) = hub_with_a_guest();
    let (mut second, _sender_rx) = hub_with_a_guest();
    let mut sender = second.guests.remove(0);
    sender.conn = 2;
    sender.uuid = 2;
    sender.entity_id = 2;
    sender.info.main_hand = 0; // Initial ClientInformation from configuration.
    hub.guests.push(sender);
    hub.guests.update_all(|g| {
        g.sent_chunks.insert((0, 0));
    });
    hub.entity_types.insert("minecraft:player".into(), 0);
    let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
    let mut next_entity = 3;
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &crate::worldborder::WorldBorder::default(),
        streaming: None,
    };
    let received = || {
        queued_play_packets(&viewer)
            .filter_map(|packet| match packet {
                C::SetEntityData(data) if data.id == 2 => Some(data.values),
                _ => None,
            })
            .flatten()
            .filter(|(index, _, _)| *index == 15)
            .collect::<Vec<_>>()
    };

    hub.track_entities(&mut ctx, None);
    assert_eq!(received(), [(15, serializer::HUMANOID_ARM, Value::Id(0))]);

    // Update through the play-phase ClientInformation handler. Right is the
    // default and is still sent to clients that saw the initial left arm.
    for expected in [vec![(15, serializer::HUMANOID_ARM, Value::Id(1))], vec![]] {
        hub.guests.update(1, |g| {
            let info = ClientInformation {
                main_hand: 1,
                ..g.info.clone()
            };
            g.inbox
                .push_back(InboxItem::Packet(S::ClientInformation(info), None));
        });
        hub.process_inbox(1, &mut ctx, &mut Vec::new(), false, &mut false, &mut None);
        hub.track_entities(&mut ctx, None);
        assert_eq!(received(), expected);
    }
}
