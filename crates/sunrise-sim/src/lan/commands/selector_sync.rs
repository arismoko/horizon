//! Sunrise's selector-token metadata, sent beside the server command tree.
use std::collections::HashSet;
use sunrise_core::vanilla_bytes::Writer;
use sunrise_game_data::registries::SelectorToken;
use sunrise_protocol::{Error, Result, buf::Reader};

pub const CHANNEL: &str = "sunrise:selector_tokens";
/// Vanilla `ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE` (1 MiB).
const MAX_PAYLOAD_BYTES: usize = 1_048_576;
/// Vanilla `STRING_UTF8` maximum length in UTF-16 code units.
const MAX_STRING_UTF16: usize = 32_767;

#[derive(Debug, PartialEq, Eq)]
pub enum Snapshot {
    Tokens(Vec<SelectorToken>),
    Unsupported(u8),
}

fn varint_len(value: i32) -> usize {
    let bits = (u32::BITS - (value as u32).leading_zeros()).max(1) as usize;
    bits.div_ceil(7)
}

pub fn encode(tokens: &[SelectorToken]) -> Vec<u8> {
    let mut body = Writer::new();
    let mut entry = Writer::new();
    let mut count: i32 = 0;
    for token in tokens {
        if token.name.encode_utf16().count() > MAX_STRING_UTF16
            || token.tooltip.encode_utf16().count() > MAX_STRING_UTF16
        {
            log::warn!(
                "selector token @{}: skipped from {CHANNEL} because name or tooltip exceeds {MAX_STRING_UTF16} UTF-16 code units",
                token.name
            );
            continue;
        }
        entry.data.clear();
        entry.string(&token.name);
        entry.string(&token.tooltip);
        entry.bool(token.players_only);
        entry.bool(token.limit != usize::MAX);
        if token.limit != usize::MAX {
            entry.u32(token.limit as u32);
        }
        entry.u8(match token.sort {
            "nearest" => 1,
            "furthest" => 2,
            "random" => 3,
            _ => 0,
        });
        let Some(next_count) = count.checked_add(1) else {
            log::warn!(
                "selector token @{}: skipped from {CHANNEL} because token count overflowed",
                token.name
            );
            continue;
        };
        let header_bytes = 1 + varint_len(next_count);
        if header_bytes + body.data.len() + entry.data.len() > MAX_PAYLOAD_BYTES {
            log::warn!(
                "selector token @{}: skipped from {CHANNEL} because payload would exceed vanilla's {MAX_PAYLOAD_BYTES}-byte custom-payload limit",
                token.name
            );
            continue;
        }
        body.data.extend_from_slice(&entry.data);
        count = next_count;
    }
    let mut w = Writer::new();
    w.u8(1);
    w.varint(count);
    w.data.extend_from_slice(&body.data);
    w.data
}

pub fn decode(data: &[u8]) -> Result<Snapshot> {
    if data.len() > MAX_PAYLOAD_BYTES {
        return Err(Error::new(
            "selector tokens: vanilla 1 MiB custom-payload limit exceeded",
        ));
    }
    let mut r = Reader::new(data);
    let revision = r.u8()?;
    if revision != 1 {
        return Ok(Snapshot::Unsupported(revision));
    }
    let count = r.varint()?;
    if count < 0 || count as usize > r.remaining() {
        return Err(Error::new("selector tokens: invalid count"));
    }
    let mut tokens = Vec::new();
    let mut names = HashSet::new();
    for _ in 0..count {
        let name = r.string(MAX_STRING_UTF16)?;
        if name.is_empty()
            || !name.bytes().all(sunrise_command::is_unquoted)
            || ["p", "a", "r", "s", "e", "n"].contains(&name.as_str())
            || !names.insert(name.clone())
        {
            return Err(Error::new("selector tokens: invalid or duplicate name"));
        }
        let tooltip = r.string(MAX_STRING_UTF16)?;
        let players_only = r.bool()?;
        let limit = if r.bool()? {
            r.u32()? as usize
        } else {
            usize::MAX
        };
        let sort = match r.u8()? {
            0 => "arbitrary",
            1 => "nearest",
            2 => "furthest",
            3 => "random",
            _ => return Err(Error::new("selector tokens: invalid sort")),
        };
        tokens.push(SelectorToken {
            name,
            tooltip,
            players_only,
            limit,
            sort,
        });
    }
    if r.remaining() != 0 {
        return Err(Error::new("selector tokens: trailing bytes"));
    }
    Ok(Snapshot::Tokens(tokens))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_tokens_install_literal_metadata_and_reject_partial_snapshots() {
        let tokens = vec![
            SelectorToken {
                name: "Allies_1.0+team-a".into(),
                tooltip: "test.allies".into(),
                players_only: true,
                limit: 1,
                sort: "nearest",
            },
            SelectorToken {
                name: "Zero-Limit.v2".into(),
                tooltip: "test.zero".into(),
                players_only: false,
                limit: 0,
                sort: "arbitrary",
            },
        ];
        let data = encode(&tokens);
        assert_eq!(data[0], 1);
        assert_eq!(decode(&data).unwrap(), Snapshot::Tokens(tokens.clone()));
        for end in 0..data.len() {
            assert!(decode(&data[..end]).is_err());
        }
        let mut trailing = data;
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        assert!(decode(&encode(&[tokens[0].clone(), tokens[0].clone()])).is_err());
        assert_eq!(decode(&encode(&[])).unwrap(), Snapshot::Tokens(Vec::new()));
        assert_eq!(decode(&[2, 255, 255]).unwrap(), Snapshot::Unsupported(2));
        assert!(sunrise_protocol::packets::custom_payload::is_reserved_channel(CHANNEL));
    }

    #[test]
    fn protocol_framing_allows_up_to_one_mib_and_skips_oversized_entries() {
        // Nine 32,767-byte tooltips exceed the removed 256 KiB cap but fit well within 1 MiB.
        let over_256k: Vec<_> = (0..9)
            .map(|n| SelectorToken {
                name: format!("Token_{n}+v1"),
                tooltip: "x".repeat(MAX_STRING_UTF16),
                players_only: false,
                limit: 0,
                sort: "nearest",
            })
            .collect();
        let encoded_256k = encode(&over_256k);
        assert!(encoded_256k.len() > 256 * 1024);
        assert!(encoded_256k.len() <= MAX_PAYLOAD_BYTES);
        assert_eq!(decode(&encoded_256k).unwrap(), Snapshot::Tokens(over_256k));

        // Oversized individual tooltips are skipped without dropping valid siblings.
        let good_before = SelectorToken {
            name: "Before_1".into(),
            tooltip: "test.before".into(),
            players_only: true,
            limit: 1,
            sort: "nearest",
        };
        let oversized_tooltip = SelectorToken {
            name: "TooLongTooltip".into(),
            tooltip: "x".repeat(MAX_STRING_UTF16 + 1),
            players_only: false,
            limit: 1,
            sort: "arbitrary",
        };
        let good_after = SelectorToken {
            name: "After_2".into(),
            tooltip: "test.after".into(),
            players_only: false,
            limit: 0,
            sort: "random",
        };
        let filtered = encode(&[good_before.clone(), oversized_tooltip, good_after.clone()]);
        assert_eq!(
            decode(&filtered).unwrap(),
            Snapshot::Tokens(vec![good_before, good_after])
        );

        // Thirty-three ~32 KiB entries exceed 1 MiB: encode keeps the prefix that fits <= 1 MiB.
        let many: Vec<_> = (0..33)
            .map(|n| SelectorToken {
                name: format!("t{n}"),
                tooltip: "x".repeat(MAX_STRING_UTF16),
                players_only: false,
                limit: 1,
                sort: "nearest",
            })
            .collect();
        let framed = encode(&many);
        assert!(framed.len() <= MAX_PAYLOAD_BYTES);
        let Snapshot::Tokens(decoded) = decode(&framed).unwrap() else {
            panic!("expected revision-1 tokens");
        };
        assert_eq!(decoded.len(), 31);
        assert_eq!(decoded[0].name, "t0");
        assert_eq!(decoded[30].name, "t30");

        // Decode rejects any payload exceeding vanilla's 1 MiB custom-payload bound.
        let too_big = vec![1; MAX_PAYLOAD_BYTES + 1];
        assert!(decode(&too_big).is_err());

        // Invalid token names (e.g. spaces) and vanilla names are still rejected on decode.
        let bad_name = encode(&[SelectorToken {
            name: "bad name".into(),
            tooltip: "test.bad".into(),
            players_only: false,
            limit: 1,
            sort: "arbitrary",
        }]);
        assert!(decode(&bad_name).is_err());
    }
}
