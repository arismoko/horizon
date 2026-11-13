//! Passage sync (#3775): the passages a native mod registered in a level, sent by a Horizon
//! server to the Horizon clients that hold their chunk, and their removal
//! (docs/modding/src/how-to/passages.md). Clientbound only: a client sends nothing back.
//!
//! A payload is a varint kind, then a varint count: for an addition that many passages
//! (the server's number as a long, the opening's centre and its two axes as three doubles
//! each, its width and height, the centre's destination, then the turn as a byte), for a
//! removal that many numbers.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};

/// The custom payload channel.
pub const CHANNEL: &str = "horizon:passage_sync";
/// Passages or numbers in one message at most: a level's cap.
pub const MAX_COUNT: usize = 256;

/// One passage as it travels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassageForm {
    /// The server's number for it.
    pub id: u64,
    pub origin: [f64; 3],
    pub axis_w: [f64; 3],
    pub axis_h: [f64; 3],
    pub width: f64,
    pub height: f64,
    pub destination: [f64; 3],
    /// Clockwise quarter turns about the vertical axis, 0 to 3.
    pub turn: u8,
}

/// A change to what a client holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// Passages to hold, replacing any with the same number.
    Add(Vec<PassageForm>),
    /// Numbers of passages to forget.
    Remove(Vec<u64>),
}

/// A message's payload; an error for one with more than [`MAX_COUNT`] entries or a turn
/// past 3.
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    match message {
        Message::Add(passages) => {
            check_count(passages.len())?;
            w.varint(0);
            w.varint(passages.len() as i32);
            for p in passages {
                if p.turn > 3 {
                    return Err(Error::new(format!("passage sync: turn {}", p.turn)));
                }
                w.i64(p.id as i64);
                for v in [p.origin, p.axis_w, p.axis_h] {
                    v.iter().for_each(|&c| w.f64(c));
                }
                w.f64(p.width);
                w.f64(p.height);
                p.destination.iter().for_each(|&c| w.f64(c));
                w.u8(p.turn);
            }
        }
        Message::Remove(ids) => {
            check_count(ids.len())?;
            w.varint(1);
            w.varint(ids.len() as i32);
            ids.iter().for_each(|&id| w.i64(id as i64));
        }
    }
    Ok(w.data)
}

fn check_count(n: usize) -> Result<()> {
    if n > MAX_COUNT {
        return Err(Error::new(format!("passage sync: {n} entries")));
    }
    Ok(())
}

/// A payload's message; an error for an unknown kind, more than [`MAX_COUNT`] entries, a
/// turn past 3, a cut payload or trailing bytes.
pub fn decode(data: &[u8]) -> Result<Message> {
    let mut r = Reader::new(data);
    let vec3 = |r: &mut Reader| -> Result<[f64; 3]> { Ok([r.f64()?, r.f64()?, r.f64()?]) };
    let message = match r.varint()? {
        0 => {
            let n = r.count(MAX_COUNT)?;
            let mut passages = Vec::with_capacity(n);
            for _ in 0..n {
                let id = r.i64()? as u64;
                let (origin, axis_w, axis_h) = (vec3(&mut r)?, vec3(&mut r)?, vec3(&mut r)?);
                let (width, height) = (r.f64()?, r.f64()?);
                let destination = vec3(&mut r)?;
                let turn = r.u8()?;
                if turn > 3 {
                    return Err(Error::new(format!("passage sync: turn {turn}")));
                }
                passages.push(PassageForm {
                    id,
                    origin,
                    axis_w,
                    axis_h,
                    width,
                    height,
                    destination,
                    turn,
                });
            }
            Message::Add(passages)
        }
        1 => {
            let n = r.count(MAX_COUNT)?;
            let mut ids = Vec::with_capacity(n);
            for _ in 0..n {
                ids.push(r.i64()? as u64);
            }
            Message::Remove(ids)
        }
        kind => return Err(Error::new(format!("passage sync: kind {kind}"))),
    };
    if r.remaining() > 0 {
        return Err(Error::new("passage sync: trailing bytes"));
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One passage's bytes, field by field, and a removal's, read back as they were.
    #[test]
    fn a_message_has_its_fields_in_order_and_reads_back() {
        let passage = PassageForm {
            id: 7,
            origin: [0.5, 65.0, 8.0],
            axis_w: [1.0, 0.0, 0.0],
            axis_h: [0.0, 1.0, 0.0],
            width: 2.0,
            height: 3.0,
            destination: [100.5, 70.0, -20.0],
            turn: 1,
        };
        let add = Message::Add(vec![passage]);
        let bytes = encode(&add).unwrap();
        // kind, count, the long, 14 doubles and the turn.
        assert_eq!(bytes.len(), 1 + 1 + 8 + 14 * 8 + 1);
        assert_eq!(&bytes[..10], [0, 1, 0, 0, 0, 0, 0, 0, 0, 7]);
        assert_eq!(&bytes[10..18], 0.5f64.to_be_bytes());
        assert_eq!(bytes[bytes.len() - 1], 1);
        assert_eq!(decode(&bytes).unwrap(), add);
        let remove = Message::Remove(vec![7, u64::MAX]);
        let bytes = encode(&remove).unwrap();
        assert_eq!(
            bytes,
            [
                1, 2, 0, 0, 0, 0, 0, 0, 0, 7, 255, 255, 255, 255, 255, 255, 255, 255
            ]
        );
        assert_eq!(decode(&bytes).unwrap(), remove);
    }

    /// An unknown kind, a turn past 3, too many entries, a cut payload and trailing
    /// bytes are refused both ways.
    #[test]
    fn a_bad_message_is_refused() {
        let turned = PassageForm {
            id: 1,
            origin: [0.0; 3],
            axis_w: [1.0, 0.0, 0.0],
            axis_h: [0.0, 1.0, 0.0],
            width: 1.0,
            height: 1.0,
            destination: [0.0; 3],
            turn: 4,
        };
        assert!(encode(&Message::Add(vec![turned])).is_err());
        assert!(encode(&Message::Remove(vec![0; 257])).is_err());
        let mut bytes = encode(&Message::Add(vec![PassageForm { turn: 3, ..turned }])).unwrap();
        assert!(decode(&bytes).is_ok());
        let last = bytes.len() - 1;
        bytes[last] = 4;
        assert!(decode(&bytes).is_err());
        assert!(decode(&bytes[..last]).is_err());
        assert!(decode(&[2, 0]).is_err());
        assert!(decode(&[1, 0, 0]).is_err());
        // A count of 257.
        assert!(decode(&[1, 0x81, 0x02]).is_err());
    }
}
