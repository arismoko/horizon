//! Negotiated server boundary: capability and replay state belong to one
//! actual player connection, never to a brand or global owner.
#[path = "settings_placement.rs"]
mod placement;
use super::{
    BookFlags, CustomBooks,
    placement_sync::ResourceId,
    settings_codec,
    settings_sync::{Authority, Message},
};
use std::collections::BTreeSet;

#[derive(Default)]
pub struct Connection {
    capability: bool,
    pub authority: Option<Authority>,
}
pub struct Reply {
    pub message: Message,
    pub changed: Option<(ResourceId, BookFlags)>,
}
impl Connection {
    /// Called only after resolving the actual placed player connection.
    /// Malformed or unexpected payloads report once and leave existing state intact.
    pub fn receive(
        &mut self,
        data: &[u8],
        books: &mut CustomBooks,
        registered: impl FnOnce() -> BTreeSet<ResourceId>,
        epoch: impl FnOnce() -> u64,
    ) -> Option<Reply> {
        let Some(message) = settings_codec::decode(data) else {
            log::warn!(
                "[recipe book] ignored malformed settings payload ({} bytes)",
                data.len()
            );
            return None;
        };
        let mut changed = None;
        let message = match message {
            Message::Hello => {
                if self.capability {
                    log::warn!("[recipe book] ignored duplicate settings hello");
                    return None;
                }
                self.capability = true;
                Message::Ready
            }
            Message::Offer(keys) => {
                if !self.capability || self.authority.is_some() {
                    log::warn!("[recipe book] ignored unexpected settings offer");
                    return None;
                }
                let authority = Authority::negotiate(epoch(), keys, &registered());
                let snapshot = authority.snapshot(books);
                self.authority = Some(authority);
                snapshot
            }
            Message::Request {
                epoch,
                number,
                key,
                flags,
            } => {
                let Some(authority) = self.authority.as_mut() else {
                    log::warn!("[recipe book] ignored settings request before negotiation");
                    return None;
                };
                let (reply, accepted) = authority.request(books, epoch, number, &key, flags);
                if accepted {
                    changed = Some((key, flags));
                }
                reply
            }
            other => {
                log::warn!(
                    "[recipe book] ignored unexpected serverbound settings message: {other:?}"
                );
                return None;
            }
        };
        Some(Reply { message, changed })
    }
}

#[cfg(test)]
mod tests {
    use super::super::settings_codec::encode;
    use super::*;
    #[test]
    fn real_boundary_negotiates_before_snapshot_and_notifies_only_accepted_changes() {
        let pot = ResourceId::try_from("demo:pot".to_owned()).unwrap();
        let oven = ResourceId::try_from("demo:oven".to_owned()).unwrap();
        let registered: BTreeSet<_> = [pot.clone(), oven.clone()].into();
        let mut connection = Connection::default();
        let mut books = CustomBooks::default();
        let offer = encode(&Message::Offer(vec![pot.clone()])).unwrap();
        assert!(
            connection
                .receive(&offer, &mut books, || registered.clone(), || 7)
                .is_none()
        );
        let hello = encode(&Message::Hello).unwrap();
        assert_eq!(
            connection
                .receive(&hello, &mut books, || registered.clone(), || 7)
                .unwrap()
                .message,
            Message::Ready
        );
        assert!(
            matches!(connection.receive(&offer,&mut books,|| registered.clone(),|| 7).unwrap().message,Message::Snapshot { entries,.. } if entries==vec![(pot.clone(),BookFlags::default())])
        );
        let request = encode(&Message::Request {
            epoch: 7,
            number: 1,
            key: pot.clone(),
            flags: BookFlags {
                open: true,
                filtering: true,
            },
        })
        .unwrap();
        assert_eq!(
            connection
                .receive(&request, &mut books, || registered.clone(), || 8)
                .unwrap()
                .changed,
            Some((
                pot.clone(),
                BookFlags {
                    open: true,
                    filtering: true
                }
            ))
        );
        // Replayed request does not mutate or notify again.
        assert!(
            connection
                .receive(&request, &mut books, || registered.clone(), || 8)
                .unwrap()
                .changed
                .is_none()
        );
        assert_eq!(books.get("demo:oven"), BookFlags::default());
        // Repeated offers, removed retry/busy tags, and unnegotiated books leave
        // the negotiated authority intact and never latch later valid intent out.
        assert!(
            connection
                .receive(&offer, &mut books, || registered.clone(), || 8)
                .is_none()
        );
        assert!(
            connection
                .receive(&[4], &mut books, || registered.clone(), || 8)
                .is_none()
        );
        assert!(
            connection
                .receive(&[7], &mut books, || registered.clone(), || 8)
                .is_none()
        );
        let unnegotiated = encode(&Message::Request {
            epoch: 7,
            number: 2,
            key: oven,
            flags: BookFlags {
                open: true,
                filtering: false,
            },
        })
        .unwrap();
        assert!(
            connection
                .receive(&unnegotiated, &mut books, || registered.clone(), || 8)
                .unwrap()
                .changed
                .is_none()
        );
        assert_eq!(connection.authority.as_ref().unwrap().epoch, 7);
        // Later valid toggles apply sequentially on the ordered play stream with no per-second cap.
        for number in 2..=26 {
            let flags = BookFlags {
                open: number % 2 == 0,
                filtering: false,
            };
            let next = encode(&Message::Request {
                epoch: 7,
                number,
                key: pot.clone(),
                flags,
            })
            .unwrap();
            let reply = connection
                .receive(&next, &mut books, || registered.clone(), || 8)
                .unwrap();
            assert_eq!(reply.changed, Some((pot.clone(), flags)));
            assert!(matches!(
                reply.message,
                Message::Update {
                    epoch: 7,
                    revision,
                    ..
                } if revision == number
            ));
        }
    }
}
