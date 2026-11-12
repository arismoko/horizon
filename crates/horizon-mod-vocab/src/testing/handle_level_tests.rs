use crate::testing::FakeHost;

#[test]
fn player_level_and_dimension_reads() {
    use crate::host::entities::EntitiesHost;
    use crate::host::players::PlayersHost;
    let mut host = FakeHost::new();
    let overworld = host.level("minecraft:overworld").unwrap();
    let nether = crate::LevelRef::new(overworld.epoch(), crate::DimensionKey::NETHER);
    host.join(1, 10);
    host.join(2, 20).level = nether;
    let p1 = host.player(1).unwrap();
    let p2 = host.player(2).unwrap();
    let unknown = crate::__private::game::player_ref(p1.entity(), 99);
    let stale = crate::__private::game::player_ref(
        crate::EntityRef::new(
            crate::WorldEpoch::new(p1.entity().epoch().get() + 1),
            p1.entity().id(),
        ),
        p1.uuid(),
    );
    let replaced = crate::__private::game::player_ref(p2.entity(), p1.uuid());
    for invalid in [unknown, stale, replaced] {
        assert_eq!(invalid.level(&host), None);
        assert_eq!(invalid.dimension(&host), None);
    }
    assert_eq!(p1.level(&host), Some(overworld));
    assert_eq!(
        p1.dimension(&host),
        Some(crate::Identifier::parse("minecraft:overworld").unwrap())
    );
    assert_eq!(p2.level(&host), Some(nether));
    assert_eq!(
        p2.dimension(&host),
        Some(crate::Identifier::parse("minecraft:the_nether").unwrap())
    );
    host.players = Default::default();
    for disconnected in [p1, p2] {
        assert_eq!(disconnected.level(&host), None);
        assert_eq!(disconnected.dimension(&host), None);
    }
}
