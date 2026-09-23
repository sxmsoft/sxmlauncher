//! Skin uploading: pick a PNG in the launcher, apply it to the signed-in
//! provider.
//!
//! Provider support differs, and pretending otherwise would produce broken
//! buttons:
//!
//! * **Microsoft** — a real upload through the official endpoint
//!   `POST https://api.minecraftservices.com/minecraft/profile/skins`
//!   (`multipart/form-data`: `variant` = `classic|slim`, `file` = PNG). The
//!   bearer token is the *Minecraft* access token from the normal session
//!   refresh flow. A successful upload returns `200` with the profile JSON.
//! * **Ely.by** — the public API surface (auth server, skins system, account
//!   OAuth) exposes **no** skin-upload endpoint, and the documented OAuth
//!   scopes contain no skin-write permission; uploads happen on ely.by with a
//!   browser session cookie. So instead of a request that can never succeed,
//!   the launcher validates the PNG locally and hands the user a deep link to
//!   their Ely.by profile's skin page.
//! * **Offline** — nothing to upload to; rejected with a clear error.
//!
//! Every path validates the PNG first: Minecraft accepts only 64×64 (since
//! 1.8) or legacy 64×32 PNGs, and Ely.by's own uploader enforces the same, so
//! catching it here gives an immediate, local error instead of a round trip.

use crate::error::{AppError, AppResult};
use crate::models::account::{AccountProvider, SkinModel};

/// What [`super::AccountManager::upload_skin`] produced.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinUploadOutcome {
    /// `true` when the provider actually received the texture.
    pub uploaded: bool,
    /// Fresh profile read back from the provider after the upload.
    pub skin: crate::models::account::SkinProfile,
    /// Provider-specific follow-up the UI should surface.
    pub message: Option<String>,
    /// Deep link the UI may open in a browser. Ely.by sets this to the
    /// profile's skin page (where the upload completes); Microsoft needs no
    /// follow-up, so it is `None` there.
    pub url: Option<String>,
}

/// Size cap: a 64×64 RGBA PNG is a few kilobytes; anything near this is not a
/// skin (or is an HD skin, which Mojang rejects anyway).
pub const MAX_SKIN_BYTES: usize = 256 * 1024;

/// Exactly 8 bytes: `\x89PNG\r\n\x1a\n`.
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Outcome of the local PNG checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatedSkinPng {
    /// `64` for both accepted layouts.
    pub width: u32,
    /// `64` (modern) or `32` (pre-1.8, auto-scaled by the game).
    pub height: u32,
}

/// Validate that `bytes` are a PNG *image header* of a size Minecraft accepts.
///
/// This checks the signature and the IHDR dimensions without decoding pixels —
/// a full decoder just to read two integers would pull in an image crate for
/// one job. A corrupt body after the header still fails the upload itself,
/// which is the same failure a user would get from the website.
pub fn validate_skin_png(bytes: &[u8]) -> AppResult<ValidatedSkinPng> {
    if bytes.len() > MAX_SKIN_BYTES {
        return Err(AppError::Config(format!(
            "skin file is {} KiB — the limit is {} KiB",
            bytes.len() / 1024,
            MAX_SKIN_BYTES / 1024
        )));
    }

    let header = bytes
        .get(..8)
        .ok_or_else(|| AppError::Config("skin file is too small to be a PNG".to_string()))?;
    if header != PNG_SIGNATURE {
        return Err(AppError::Config(
            "skin file is not a PNG image".to_string(),
        ));
    }

    // IHDR must be the first chunk: 4-byte length (always 13), "IHDR", then
    // width and height as big-endian u32.
    let ihdr = bytes
        .get(..24)
        .ok_or_else(|| AppError::Config("skin file is truncated (no PNG header)".to_string()))?;
    if &ihdr[12..16] != b"IHDR" {
        return Err(AppError::Config(
            "skin file does not start with a PNG image header".to_string(),
        ));
    }
    let width = u32::from_be_bytes([
        *ihdr.get(16).ok_or_else(|| AppError::Config("truncated PNG".into()))?,
        *ihdr.get(17).ok_or_else(|| AppError::Config("truncated PNG".into()))?,
        *ihdr.get(18).ok_or_else(|| AppError::Config("truncated PNG".into()))?,
        *ihdr.get(19).ok_or_else(|| AppError::Config("truncated PNG".into()))?,
    ]);
    let height = u32::from_be_bytes([
        *ihdr.get(20).ok_or_else(|| AppError::Config("truncated PNG".into()))?,
        *ihdr.get(21).ok_or_else(|| AppError::Config("truncated PNG".into()))?,
        *ihdr.get(22).ok_or_else(|| AppError::Config("truncated PNG".into()))?,
        *ihdr.get(23).ok_or_else(|| AppError::Config("truncated PNG".into()))?,
    ]);

    // Minecraft skins: square 64×64, or the pre-1.8 64×32 layout which the
    // game still converts. Everything else is rejected up front.
    let ok = (width == 64 && height == 64) || (width == 64 && height == 32);
    if !ok {
        return Err(AppError::Config(format!(
            "skin must be 64×64 (or legacy 64×32) pixels — this one is {width}×{height}"
        )));
    }

    Ok(ValidatedSkinPng { width, height })
}

/// Where a provider's skin is managed in a browser.
pub fn provider_skin_page(provider: AccountProvider, username: &str) -> Option<String> {
    match provider {
        // Signed-in Minecraft profile page: has the upload form.
        AccountProvider::Microsoft => Some("https://www.minecraft.net/msaprofile/mygames/editskin".to_string()),
        // Ely.by profile skin manager (uploads need the website session).
        AccountProvider::ElyBy => Some(format!(
            "https://ely.by/u{username}/skin?username={username}"
        )),
        // sx.acc has no fixed website. Uploads go to `{BASE}/v1/profile/skin`
        // when that route exists; there is no host to link to otherwise.
        AccountProvider::SxAcc => None,
        AccountProvider::Offline => None,
    }
}

/// Upload the skin to Mojang for a Microsoft account.
///
/// `model` chooses the arm shape (`classic` = wide, `slim` = Alex-style).
/// The response body is the full profile JSON; we only need success/failure
/// here because the caller re-reads the skin through the session server.
pub async fn upload_skin_to_mojang(
    http: &reqwest::Client,
    mc_access_token: &str,
    model: SkinModel,
    png: Vec<u8>,
) -> AppResult<()> {
    const MC_SKIN_URL: &str = "https://api.minecraftservices.com/minecraft/profile/skins";

    let variant = model.as_str(); // "classic" | "slim" — matches the API
    let file_name = format!("skin-{}.png", variant);

    let part = reqwest::multipart::Part::bytes(png)
        .file_name(file_name)
        .mime_str("image/png")
        .map_err(|err| AppError::Network(format!("multipart build failed: {err}")))?;
    let form = reqwest::multipart::Form::new()
        .text("variant", variant)
        .part("file", part);

    let response = http
        .post(MC_SKIN_URL)
        .bearer_auth(mc_access_token)
        .multipart(form)
        .send()
        .await
        .map_err(|err| AppError::Network(format!("skin upload failed: {err}")))?;

    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    // 401 = session expired → the caller should sign in again.
    if status.as_u16() == 401 {
        return Err(AppError::Unauthorized);
    }

    let body = response.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .or_else(|| value.get("errorMessage"))
                .and_then(|message| message.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                format!("HTTP {status}")
            } else {
                trimmed.chars().take(300).collect()
            }
        });

    Err(AppError::Network(format!("Mojang rejected the skin: {detail}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal-but-valid PNG header with the given dimensions.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&PNG_SIGNATURE);
        bytes.extend_from_slice(&13u32.to_be_bytes()); // IHDR length
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]); // depth, colour, compression, filter, interlace
        bytes.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder — not validated
        bytes
    }

    #[test]
    fn a_64x64_png_is_accepted() {
        let png = png_header(64, 64);
        let validated = validate_skin_png(&png).expect("valid");
        assert_eq!(validated, ValidatedSkinPng { width: 64, height: 64 });
    }

    #[test]
    fn the_legacy_64x32_layout_is_accepted() {
        let png = png_header(64, 32);
        let validated = validate_skin_png(&png).expect("valid");
        assert_eq!(validated, ValidatedSkinPng { width: 64, height: 32 });
    }

    #[test]
    fn wrong_dimensions_are_rejected_with_the_actual_size() {
        let err = validate_skin_png(&png_header(128, 128)).unwrap_err();
        assert!(err.to_string().contains("128×128"), "{err}");
    }

    #[test]
    fn a_jpeg_named_png_is_rejected() {
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0];
        let err = validate_skin_png(&jpeg).unwrap_err();
        assert!(err.to_string().contains("not a PNG"), "{err}");
    }

    #[test]
    fn an_oversized_file_is_rejected_before_parsing() {
        let big = vec![0u8; MAX_SKIN_BYTES + 1];
        let err = validate_skin_png(&big).unwrap_err();
        assert!(err.to_string().contains("limit"), "{err}");
    }

    #[test]
    fn a_truncated_header_is_rejected_cleanly() {
        let err = validate_skin_png(&PNG_SIGNATURE).unwrap_err();
        assert!(err.to_string().contains("truncated"), "{err}");
    }

    #[test]
    fn only_online_providers_have_a_skin_page() {
        assert!(provider_skin_page(AccountProvider::Microsoft, "x").is_some());
        assert!(provider_skin_page(AccountProvider::ElyBy, "Erick").is_some());
        assert!(provider_skin_page(AccountProvider::Offline, "x").is_none());
    }

    #[test]
    fn elyby_deep_link_carries_the_username() {
        let link = provider_skin_page(AccountProvider::ElyBy, "Erick").expect("link");
        assert!(link.contains("username=Erick"), "{link}");
    }

    #[tokio::test]
    async fn mojang_upload_without_a_session_maps_to_unauthorized() {
        // A token Mojang will always reject: the endpoint answers 401 before
        // looking at the body, so this exercises the status mapping without
        // needing real credentials.
        let http = reqwest::Client::new();
        let png = png_header(64, 64);
        let result = upload_skin_to_mojang(&http, "not-a-real-token", SkinModel::Classic, png).await;
        assert!(matches!(result, Err(AppError::Unauthorized)), "{result:?}");
    }
}
