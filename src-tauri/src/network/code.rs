//! Shareable connection codes.
//!
//! A friend should be able to type (or paste) one short string instead of
//! hunting through a server browser. The code is a compact, self-contained
//! payload — no central lookup required:
//!
//! ```text
//! byte 0      version (1)
//! byte 1      mode (0 = direct, 1 = relay)
//! bytes 2..10 host peer id (8 bytes of the UUID)
//! bytes 10..14 IPv4 (direct) OR relay shard id (relay)
//! bytes 14..16 port
//! byte 16     flags (0b1 = password protected, 0b10 = whitelisted)
//! bytes 17..21 CRC32 of the previous bytes
//! ```
//!
//! Rendered as grouped base32 (34 characters: eight groups of four and a
//! final pair) so it survives being read aloud or retyped:
//! `SXM1-AEAI-RMQG-NTKC-WSQT-ELAX-3T6N-CYAB-FF6E-NQ`.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::server::ConnectionMode;

/// Human-visible prefix; also the version marker.
pub const CODE_PREFIX: &str = "SXM1";
/// Payload version byte.
pub const CODE_VERSION: u8 = 1;
/// Everything except the visible prefix and separators.
const PAYLOAD_LEN: usize = 21;

/// Flags carried by a connection code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeFlags {
    pub password_protected: bool,
    pub whitelisted: bool,
}

/// A decoded, directly connectable session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectCode {
    pub mode: ConnectionMode,
    /// Short peer id (first 8 bytes of the host's peer UUID).
    pub peer_id_prefix: [u8; 8],
    pub address: Option<SocketAddr>,
    pub relay_shard: Option<u16>,
    pub flags: CodeFlags,
}

impl ConnectCode {
    /// Build a direct-connect code for a publicly reachable host.
    pub fn direct(peer_id: Uuid, address: SocketAddr, flags: CodeFlags) -> AppResult<Self> {
        let SocketAddr::V4(v4) = address else {
            return Err(AppError::Transport(
                "connection codes only carry IPv4 addresses; use the server browser instead"
                    .to_string(),
            ));
        };
        Ok(Self {
            mode: ConnectionMode::DirectP2p,
            peer_id_prefix: peer_prefix(peer_id),
            address: Some(SocketAddr::V4(v4)),
            relay_shard: None,
            flags,
        })
    }

    /// Build a relay code (used when hole punching is impossible).
    pub fn relay(peer_id: Uuid, shard: u16, flags: CodeFlags) -> Self {
        Self {
            mode: ConnectionMode::Relay,
            peer_id_prefix: peer_prefix(peer_id),
            address: None,
            relay_shard: Some(shard),
            flags,
        }
    }

    /// Encode the payload as bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PAYLOAD_LEN);
        bytes.push(CODE_VERSION);
        bytes.push(match self.mode {
            ConnectionMode::DirectP2p => 0,
            ConnectionMode::Relay => 1,
            ConnectionMode::Dedicated => 2,
        });
        bytes.extend_from_slice(&self.peer_id_prefix);

        match self.mode {
            ConnectionMode::Relay | ConnectionMode::Dedicated => {
                // Bytes 10..12 carry the relay shard; the remaining four bytes
                // of the address/port region (12..16) are reserved and must be
                // emitted so the payload is exactly `PAYLOAD_LEN` long. Getting
                // this wrong makes `from_bytes` reject every relay code.
                let shard = self.relay_shard.unwrap_or(0);
                bytes.extend_from_slice(&shard.to_be_bytes());
                bytes.extend_from_slice(&[0u8; 4]);
            }
            ConnectionMode::DirectP2p => {
                let ip = self
                    .address
                    .map(|address| match address {
                        SocketAddr::V4(v4) => *v4.ip(),
                        SocketAddr::V6(_) => Ipv4Addr::UNSPECIFIED,
                    })
                    .unwrap_or(Ipv4Addr::UNSPECIFIED);
                bytes.extend_from_slice(&ip.octets());
                let port = self.address.map(|address| address.port()).unwrap_or(0);
                bytes.extend_from_slice(&port.to_be_bytes());
            }
        }

        let flags =
            (u8::from(self.flags.password_protected)) | (u8::from(self.flags.whitelisted) << 1);
        bytes.push(flags);

        let checksum = crc32(&bytes);
        bytes.extend_from_slice(&checksum.to_be_bytes());
        bytes
    }

    /// Parse a payload (without the human prefix).
    pub fn from_bytes(bytes: &[u8]) -> AppResult<Self> {
        if bytes.len() != PAYLOAD_LEN {
            return Err(AppError::Transport(format!(
                "a connection code payload must be {PAYLOAD_LEN} bytes, got {}",
                bytes.len()
            )));
        }
        if bytes[0] != CODE_VERSION {
            return Err(AppError::Transport(format!(
                "this code was made by a newer SXMLAUNCHER (payload version {})",
                bytes[0]
            )));
        }

        let expected = u32::from_be_bytes([bytes[17], bytes[18], bytes[19], bytes[20]]);
        let actual = crc32(&bytes[..17]);
        if expected != actual {
            return Err(AppError::Transport(
                "that connection code is corrupted — ask for it again".to_string(),
            ));
        }

        let mut prefix = [0u8; 8];
        prefix.copy_from_slice(&bytes[2..10]);
        let flags = CodeFlags {
            password_protected: bytes[16] & 0b1 != 0,
            whitelisted: bytes[16] & 0b10 != 0,
        };

        match bytes[1] {
            0 => {
                let ip = Ipv4Addr::new(bytes[10], bytes[11], bytes[12], bytes[13]);
                let port = u16::from_be_bytes([bytes[14], bytes[15]]);
                if ip.is_unspecified() || port == 0 {
                    return Err(AppError::Transport(
                        "this code has no valid address".to_string(),
                    ));
                }
                Ok(Self {
                    mode: ConnectionMode::DirectP2p,
                    peer_id_prefix: prefix,
                    address: Some(SocketAddr::V4(SocketAddrV4::new(ip, port))),
                    relay_shard: None,
                    flags,
                })
            }
            1 | 2 => {
                let shard = u16::from_be_bytes([bytes[10], bytes[11]]);
                Ok(Self {
                    mode: if bytes[1] == 1 {
                        ConnectionMode::Relay
                    } else {
                        ConnectionMode::Dedicated
                    },
                    peer_id_prefix: prefix,
                    address: None,
                    relay_shard: Some(shard),
                    flags,
                })
            }
            other => Err(AppError::Transport(format!(
                "unknown connection mode {other} in this code"
            ))),
        }
    }

    /// Encode as the human-shareable string.
    pub fn to_share_string(&self) -> String {
        format_code(&self.to_bytes())
    }

    /// Canonical `SXM1-…` form. Sloppy pastes (case, spaces, missing dashes)
    /// collapse to the same string the host stored, so Redis and MQTT keys match.
    pub fn canonical_share_string(input: &str) -> AppResult<String> {
        Ok(Self::parse(input)?.to_share_string())
    }

    /// Parse the human-shareable string (accepts the prefix, dashes and spaces).
    pub fn parse(input: &str) -> AppResult<Self> {
        let compact: String = input
            .trim()
            .to_uppercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();

        let without_prefix = compact.strip_prefix(CODE_PREFIX).ok_or_else(|| {
            AppError::Transport(format!("connection codes start with {CODE_PREFIX}"))
        })?;

        let bytes = base32_decode(without_prefix)?;
        Self::from_bytes(&bytes)
    }
}

/// Key form for a share code.
///
/// A full code is re-emitted in the canonical grouped form. Anything that does
/// not parse (tests, partial input) is only trimmed and upper-cased, which keeps
/// the historical `SXM1-ABCD` keys stable.
pub fn normalize_share_code(input: &str) -> String {
    ConnectCode::canonical_share_string(input).unwrap_or_else(|_| input.trim().to_uppercase())
}

/// First 8 bytes of a UUID, used as a short, collision-resistant peer handle.
fn peer_prefix(peer_id: Uuid) -> [u8; 8] {
    let bytes = peer_id.as_bytes();
    let mut prefix = [0u8; 8];
    prefix.copy_from_slice(&bytes[..8]);
    prefix
}

/// Format a payload as `SXM1-XXXX-XXXX-...`.
pub fn format_code(payload: &[u8]) -> String {
    let encoded = base32_encode(payload);
    let mut formatted = String::with_capacity(CODE_PREFIX.len() + encoded.len() + 8);
    formatted.push_str(CODE_PREFIX);
    for chunk in encoded.as_bytes().chunks(4) {
        formatted.push('-');
        formatted.push_str(&String::from_utf8_lossy(chunk));
    }
    formatted
}

/// RFC 4648 base32, no padding. Digits are only `2–7`.
///
/// `0`, `1`, `8`, and `9` are not in this alphabet. The join box uses the same
/// set (`SHARE_CODE_PATTERN` in `src/services/network.ts`). The only `1` in a
/// code is the `SXM1` prefix.
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// `true` when `code` is exactly what the join box accepts:
/// `SXM1` + eight groups of four + a final pair, alphabet `A–Z2–7`.
pub fn matches_share_code_pattern(code: &str) -> bool {
    let Some(rest) = code.strip_prefix("SXM1-") else {
        return false;
    };
    let groups: Vec<&str> = rest.split('-').collect();
    if groups.len() != 9 {
        return false;
    }
    let group_ok = |group: &str, len: usize| {
        group.len() == len && group.bytes().all(|byte| ALPHABET.contains(&byte))
    };
    groups[..8].iter().all(|group| group_ok(group, 4)) && group_ok(groups[8], 2)
}

fn base32_encode(data: &[u8]) -> String {
    let mut output = String::new();
    let mut buffer: u32 = 0;
    let mut bits = 0u32;

    for byte in data {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            let index = ((buffer >> (bits - 5)) & 0x1f) as usize;
            output.push(ALPHABET[index] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0x1f) as usize;
        output.push(ALPHABET[index] as char);
    }
    output
}

fn base32_decode(input: &str) -> AppResult<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len() * 5 / 8 + 1);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;

    for character in input.chars() {
        let value = ALPHABET
            .iter()
            .position(|candidate| *candidate as char == character)
            .ok_or_else(|| {
                AppError::Transport(format!(
                    "`{character}` is not a valid connection code character"
                ))
            })? as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            output.push(((buffer >> (bits - 8)) & 0xff) as u8);
            bits -= 8;
        }
    }
    Ok(output)
}

/// CRC-32 (IEEE 802.3) — the same polynomial ZIP and PNG use.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_codes_round_trip() {
        let address: SocketAddr = "203.0.113.7:25565".parse().expect("addr");
        let code = ConnectCode::direct(
            Uuid::new_v4(),
            address,
            CodeFlags {
                password_protected: true,
                whitelisted: false,
            },
        )
        .expect("code");

        let shared = code.to_share_string();
        assert!(shared.starts_with("SXM1-"));
        assert_eq!(shared.len(), CODE_PREFIX.len() + 1 + 34 + 8);

        let parsed = ConnectCode::parse(&shared).expect("parse");
        assert_eq!(parsed.mode, ConnectionMode::DirectP2p);
        assert_eq!(parsed.address, Some(address));
        assert!(parsed.flags.password_protected);
        assert!(!parsed.flags.whitelisted);
        assert_eq!(parsed.peer_id_prefix, code.peer_id_prefix);
    }

    #[test]
    fn every_mode_encodes_a_full_payload() {
        // Regression guard: the relay branch once emitted two bytes too few, so
        // relay codes could be produced but never parsed back.
        let direct = ConnectCode::direct(
            Uuid::new_v4(),
            "203.0.113.7:25565".parse().expect("addr"),
            CodeFlags {
                password_protected: false,
                whitelisted: false,
            },
        )
        .expect("direct");
        let relay = ConnectCode::relay(
            Uuid::new_v4(),
            12,
            CodeFlags {
                password_protected: true,
                whitelisted: true,
            },
        );

        for code in [&direct, &relay] {
            let bytes = code.to_bytes();
            assert_eq!(
                bytes.len(),
                PAYLOAD_LEN,
                "wrong payload length for {code:?}"
            );
            // And the shareable form must survive a full round trip.
            assert_eq!(
                &ConnectCode::parse(&code.to_share_string()).expect("parse"),
                code
            );
        }
    }

    #[test]
    fn sloppy_pastes_share_one_canonical_key() {
        let code = ConnectCode::relay(
            Uuid::new_v4(),
            7,
            CodeFlags {
                password_protected: false,
                whitelisted: false,
            },
        );
        let canonical = code.to_share_string();
        let sloppy = canonical.to_lowercase().replace('-', "");
        assert_eq!(normalize_share_code(&sloppy), canonical);
        assert_eq!(normalize_share_code(&format!("  {canonical}  ")), canonical);
        // Partial codes stay upper-case and do not grow a fake payload.
        assert_eq!(normalize_share_code("sxm1-abcd"), "SXM1-ABCD");
    }

    #[test]
    fn parsing_tolerates_case_spaces_and_missing_dashes() {
        let code = ConnectCode::relay(
            Uuid::new_v4(),
            12,
            CodeFlags {
                password_protected: false,
                whitelisted: true,
            },
        );
        let shared = code.to_share_string();
        let sloppy = format!(" {}  ", shared.to_lowercase().replace('-', " "));
        let parsed = ConnectCode::parse(&sloppy).expect("parse");
        assert_eq!(parsed.mode, ConnectionMode::Relay);
        assert_eq!(parsed.relay_shard, Some(12));
        assert!(parsed.flags.whitelisted);
    }

    #[test]
    fn corrupted_codes_are_rejected() {
        let code = ConnectCode::relay(
            Uuid::new_v4(),
            1,
            CodeFlags {
                password_protected: false,
                whitelisted: false,
            },
        );
        let mut bytes = code.to_bytes();
        bytes[3] ^= 0xff;
        let error = ConnectCode::from_bytes(&bytes).expect_err("must reject");
        assert!(error.to_string().contains("corrupted"));
    }

    #[test]
    fn wrong_prefix_and_unknown_version_are_rejected() {
        let code = ConnectCode::relay(
            Uuid::new_v4(),
            1,
            CodeFlags {
                password_protected: false,
                whitelisted: false,
            },
        );

        assert!(ConnectCode::parse("AAA-1234").is_err());
        assert!(ConnectCode::parse("").is_err());

        let mut bytes = code.to_bytes();
        bytes[0] = 9;
        assert!(ConnectCode::from_bytes(&bytes).is_err());
    }

    #[test]
    fn truncated_payloads_are_rejected() {
        let code = ConnectCode::relay(
            Uuid::new_v4(),
            1,
            CodeFlags {
                password_protected: false,
                whitelisted: false,
            },
        );
        let bytes = code.to_bytes();
        assert!(ConnectCode::from_bytes(&bytes[..10]).is_err());
    }

    #[test]
    fn base32_round_trips_arbitrary_bytes() {
        let payload: Vec<u8> = (0..PAYLOAD_LEN as u8).collect();
        let encoded = base32_encode(&payload);
        assert_eq!(base32_decode(&encoded).expect("decode"), payload);
        assert!(
            encoded.bytes().all(|byte| ALPHABET.contains(&byte)),
            "encoder emitted a character outside A-Z2-7: {encoded}"
        );
    }

    #[test]
    fn generated_codes_match_the_join_box_pattern() {
        let flags = CodeFlags {
            password_protected: false,
            whitelisted: false,
        };
        let mut samples = Vec::new();
        for index in 0..256u32 {
            let id = Uuid::from_u128(u128::from(index).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            samples.push(ConnectCode::relay(id, index as u16, flags).to_share_string());
            let address: SocketAddr =
                SocketAddr::from(([10, (index >> 8) as u8, index as u8, 7], 20000 + index as u16));
            if let Ok(direct) = ConnectCode::direct(id, address, flags) {
                samples.push(direct.to_share_string());
            }
        }
        // Every 21-byte payload length is fixed, so one hostile buffer is enough
        // to prove the encoder cannot invent a 0 or a 1.
        samples.push(format_code(&[0x00, 0x11, 0x01, 0x10, 0x08, 0x09, 0xff].repeat(3)));

        assert!(samples.len() > 500);
        for code in samples {
            assert!(
                matches_share_code_pattern(&code),
                "generated code is not a SHARE_CODE_PATTERN match: {code}"
            );
            let payload: String = code.chars().skip(4).filter(|ch| *ch != '-').collect();
            assert_eq!(payload.len(), 34, "{code}");
            assert!(
                !payload.contains(['0', '1', '8', '9']),
                "payload contains a digit the join box rejects: {code}"
            );
        }
    }

    #[test]
    fn crc32_matches_the_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn ipv6_hosts_cannot_produce_a_direct_code() {
        let address: SocketAddr = "[2001:db8::1]:25565".parse().expect("addr");
        assert!(ConnectCode::direct(
            Uuid::new_v4(),
            address,
            CodeFlags {
                password_protected: false,
                whitelisted: false,
            }
        )
        .is_err());
    }
}
