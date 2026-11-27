use sunrise_protocol::entity_data::Value;
use sunrise_sim::mobs::Mob;

pub(in crate::game) fn unhappy_counter(
    m: &Mob,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
    if m.extra.ints[0] > 0 {
        meta.push((
            18,
            sunrise_protocol::entity_data::serializer::INT,
            Value::Int(m.extra.ints[0]),
        ));
    }
    defaults.push((
        18,
        sunrise_protocol::entity_data::serializer::INT,
        Value::Int(0),
    ));
}
