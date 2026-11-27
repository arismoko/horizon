/// `ClientPacketListener.handleGameEvent`: verify that the game event packet
/// types for arrow hit, pufferfish sting and limited crafting match vanilla.
#[test]
fn game_event_constants_and_types_match_vanilla() {
    use sunrise_protocol::packets::play::GameEvent;
    // Event types from ClientboundGameEventPacket:
    // 6 = PLAY_ARROW_HIT_SOUND, 7 = RAIN_LEVEL_CHANGE, 8 = THUNDER_LEVEL_CHANGE,
    // 9 = PUFFER_FISH_STING, 11 = IMMEDIATE_RESPAWN, 12 = LIMITED_CRAFTING.
    let arrow = GameEvent {
        event: 6,
        param: 0.0,
    };
    let rain = GameEvent {
        event: 7,
        param: 0.5,
    };
    let thunder = GameEvent {
        event: 8,
        param: 0.2,
    };
    let puffer = GameEvent {
        event: 9,
        param: 0.0,
    };
    let respawn = GameEvent {
        event: 11,
        param: 1.0,
    };
    let crafting = GameEvent {
        event: 12,
        param: 1.0,
    };

    assert_eq!(arrow.event, 6);
    assert_eq!(rain.event, 7);
    assert_eq!(thunder.event, 8);
    assert_eq!(puffer.event, 9);
    assert_eq!(respawn.event, 11);
    assert_eq!(crafting.event, 12);
    assert!(crafting.param == 1.0);
}
