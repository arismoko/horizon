use super::*;

pub(super) fn mod_ticket_to_nbt(ticket: &ModTicket, controller: &str) -> Tag {
    let owner = match ticket.owner {
        TicketOwner::Block(pos) => ("block", Tag::IntArray(pos.to_vec())),
        TicketOwner::Entity(uuid) => (
            "entity",
            Tag::IntArray((0..4).map(|i| (uuid >> (96 - 32 * i)) as i32).collect()),
        ),
    };
    Tag::compound([
        ("controller", Tag::String(controller.into())),
        ("owner", Tag::compound([owner])),
        (
            "chunk_pos",
            Tag::IntArray(vec![ticket.chunk.0, ticket.chunk.1]),
        ),
        (
            "natural_spawning",
            Tag::Byte(i8::from(ticket.natural_spawning)),
        ),
    ])
}
/// A saved ticket: the id of the controller that held it, and the ticket without its slot.
pub(super) fn mod_ticket_from_nbt(tag: &Tag) -> Option<(String, ModTicket)> {
    let Some(Tag::String(controller)) = tag.get("controller") else {
        return None;
    };
    let owner = tag.get("owner")?;
    let owner = match (owner.get("block"), owner.get("entity")) {
        (Some(Tag::IntArray(p)), _) if p.len() == 3 => TicketOwner::Block([p[0], p[1], p[2]]),
        (_, Some(Tag::IntArray(u))) if u.len() == 4 => TicketOwner::Entity(
            u.iter()
                .fold(0u128, |acc, &n| acc << 32 | u128::from(n as u32)),
        ),
        _ => return None,
    };
    let Some(Tag::IntArray(pos)) = tag.get("chunk_pos") else {
        return None;
    };
    let controller = controller.to_string().ok()?;
    (pos.len() == 2).then(|| {
        let ticket = ModTicket {
            controller: 0,
            owner,
            chunk: (pos[0], pos[1]),
            // Tickets saved without the flag don't force natural spawning.
            natural_spawning: matches!(tag.get("natural_spawning"), Some(Tag::Byte(b)) if *b != 0),
        };
        (controller, ticket)
    })
}
pub(super) fn is_forced_ticket(ticket: &Tag) -> bool {
    matches!(ticket.get("type"), Some(Tag::String(t)) if *t == JavaString::from("minecraft:forced"))
}
pub(super) fn is_portal_ticket(ticket: &Tag) -> bool {
    matches!(ticket.get("type"), Some(Tag::String(t)) if *t == JavaString::from("minecraft:portal"))
}
