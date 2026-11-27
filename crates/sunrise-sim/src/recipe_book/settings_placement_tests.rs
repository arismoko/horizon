use super::super::super::{
    placement_sync::{Frame, ResourceId, schema},
    settings_sync::Message,
};
use super::*;
#[test]
fn placement_requires_the_existing_negotiation_without_a_rate_budget() {
    let mut connection = Connection::default();
    let hello = settings_codec::encode(&Message::Placement(Frame::Hello {
        epoch: 7,
        schema: schema(),
    }))
    .unwrap();
    assert!(Connection::default().receive_placement(&hello).is_none());
    let mut books = CustomBooks::default();
    let book = ResourceId::try_from("demo:book".to_owned()).unwrap();
    // Hello alone without Offer does not admit placement.
    assert!(
        connection
            .receive(
                &settings_codec::encode(&Message::Hello).unwrap(),
                &mut books,
                || [book.clone()].into(),
                || 7,
            )
            .is_some()
    );
    assert!(connection.receive_placement(&hello).is_none());
    assert!(
        connection
            .receive(
                &settings_codec::encode(&Message::Offer(vec![book.clone()])).unwrap(),
                &mut books,
                || [book.clone()].into(),
                || 7,
            )
            .is_some()
    );
    // Refused/wrong-direction or removed retry tags do not latch later valid placement out.
    let wrong_direction = settings_codec::encode(&Message::Placement(Frame::Ready {
        epoch: 7,
        schema: schema(),
    }))
    .unwrap();
    assert!(connection.receive_placement(&wrong_direction).is_none());
    let wrong_ghost = settings_codec::encode(&Message::Placement(Frame::Ghost {
        catalog: super::super::super::placement_sync::CatalogIdentity {
            epoch: 7,
            number: 1,
            window: 1,
            nonce: 1,
            instance: 1,
            generation: 1,
            menu: ResourceId::try_from("demo:menu".to_owned()).unwrap(),
            book: book.clone(),
        },
        recipe: ResourceId::try_from("demo:recipe".to_owned()).unwrap(),
        display: mod_api::RecipeBookDisplay::Furnace {
            ingredient: mod_api::RecipeSlotDisplay::Item("minecraft:iron_ore".into()),
            fuel: mod_api::RecipeSlotDisplay::AnyFuel,
            result: mod_api::RecipeSlotDisplay::Item("minecraft:iron_ingot".into()),
            station: mod_api::RecipeSlotDisplay::Item("minecraft:furnace".into()),
            duration: 200,
            experience: 0.7,
        },
    }))
    .unwrap();
    assert!(connection.receive_placement(&wrong_ghost).is_none());
    assert!(connection.receive_placement(&[4]).is_none());
    assert!(connection.receive_placement(&[7]).is_none());
    for _ in 0..25 {
        assert!(matches!(
            connection.receive_placement(&hello),
            Some(Frame::Hello { epoch: 7, .. })
        ));
    }
    assert_eq!(connection.authority.as_ref().unwrap().epoch, 7);
}
