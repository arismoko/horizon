use super::*;
use mod_api::prelude::*;

/// A server whose upgrades mark what they were given: an entity gains `upgraded_from`, a block entity is refused.
struct World;
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World);
impl mod_api::ServerHost for World {}
mod_api::impl_server_host_areas!(World, except(world_edit));
impl WorldEditServerHost for World {
    fn upgrade_entity_nbt(
        &self,
        data_version: i32,
        mut nbt: mod_api::Compound,
    ) -> Result<mod_api::Compound, String> {
        nbt.insert("upgraded_from".into(), mod_api::Tag::Int(data_version));
        Ok(nbt)
    }
    fn upgrade_block_entity_nbt(
        &self,
        data_version: i32,
        _: mod_api::Compound,
    ) -> Result<mod_api::Compound, String> {
        Err(format!("data version {data_version} is negative"))
    }
}

#[test]
fn datafix_upgrades_round_trip_the_data_and_the_refusal() {
    // The platform phase this scope sets is process-wide state another test may not have installed yet.
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let world = World;
    let mut frame = boundary::Frame::default();
    frame.set_server_host(&world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: world and its frame remain live for this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let mut villager = mod_api::Compound::new();
    villager.insert(
        "id".into(),
        mod_api::Tag::String("minecraft:villager".into()),
    );
    let mut expected = villager.clone();
    expected.insert("upgraded_from".into(), mod_api::Tag::Int(1900));
    assert_eq!(
        guest::Guest.upgrade_entity_nbt(1900, villager.clone()),
        Ok(expected)
    );
    assert_eq!(
        guest::Guest.upgrade_block_entity_nbt(-1, villager),
        Err("data version -1 is negative".to_owned())
    );
    guest::set_transport(None);
}
