//! Server icon handling for the browser cards.
//!
//! Icons travel inside the Redis listing, so they are size-capped: a 5 MB PNG
//! pasted by a user would bloat every browse response for every client. The cap
//! mirrors what the server-list protocol effectively tolerates (64 KiB).

use base64::Engine;

use crate::config::MAX_ICON_BYTES;
use crate::error::{AppError, AppResult};

/// Reject anything that is not actually a PNG/JPEG before we advertise it.
pub fn validate_icon(bytes: &[u8]) -> AppResult<()> {
    if bytes.is_empty() {
        return Err(AppError::Config("the icon file is empty".to_string()));
    }
    if bytes.len() > MAX_ICON_BYTES {
        return Err(AppError::Config(format!(
            "server icons must be under {} KiB (this one is {} KiB)",
            MAX_ICON_BYTES / 1024,
            bytes.len() / 1024
        )));
    }

    let is_png = bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let is_jpeg = bytes.starts_with(&[0xff, 0xd8, 0xff]);
    if !is_png && !is_jpeg {
        return Err(AppError::Config(
            "server icons must be a PNG or JPEG image".to_string(),
        ));
    }
    Ok(())
}

/// Validate then base64-encode an icon for the listing payload.
pub fn encode_icon(bytes: &[u8]) -> AppResult<Option<String>> {
    validate_icon(bytes)?;
    Ok(Some(
        base64::engine::general_purpose::STANDARD.encode(bytes),
    ))
}

/// Decode an icon back to bytes (thumbnail cache, share previews).
pub fn decode_icon(encoded: &str) -> AppResult<Vec<u8>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|err| AppError::Config(format!("invalid icon payload: {err}")))?;
    validate_icon(&bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(size: usize) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.resize(size, 0);
        bytes
    }

    #[test]
    fn accepts_png_and_jpeg() {
        assert!(validate_icon(&png(64)).is_ok());
        assert!(validate_icon(&[0xff, 0xd8, 0xff, 0xe0]).is_ok());
    }

    #[test]
    fn rejects_wrong_type_empty_and_oversized_icons() {
        assert!(validate_icon(b"#!/bin/sh\nrm -rf /").is_err());
        assert!(validate_icon(&[]).is_err());
        assert!(validate_icon(&png(MAX_ICON_BYTES + 1)).is_err());
    }

    #[test]
    fn encoding_round_trips() {
        let original = png(128);
        let encoded = encode_icon(&original).expect("encode").expect("some");
        assert_eq!(decode_icon(&encoded).expect("decode"), original);
    }

    #[test]
    fn decoding_rejects_non_image_payloads() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"not an image");
        assert!(decode_icon(&encoded).is_err());
        assert!(decode_icon("!!!not base64!!!").is_err());
    }
}
