//! CurseForge API client.
//!
//! Differences from Modrinth that shape this module:
//! * Every call needs an `x-api-key` header — the launcher ships **no** key, the
//!   user supplies their own in Settings (env `SXML_CURSEFORGE_API_KEY` works too).
//! * There is no "give me the version for game version X" endpoint; we fetch the
//!   file list and filter client-side.
//! * Installed-file detection uses **MurmurHash2** fingerprints of the jar with
//!   whitespace bytes stripped, not SHA-1.
//!
//! Without a key the client reports a clear, actionable error instead of
//! failing with a 403 somewhere deep in the UI.

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::models::modpack::{
    CurseForgeFile, CurseForgeMod, ModHashes, ModSearchHit, ModSearchQuery, ModSearchResults,
    ModSource, ModVersion,
};

/// CurseForge v1 base URL (the "core" API).
pub const CURSEFORGE_API: &str = "https://api.curseforge.com/v1";
/// Minecraft's game id on CurseForge (constant since launch).
pub const MINECRAFT_GAME_ID: u32 = 432;
/// Class ids for the modpack/mod categories we care about.
pub const CLASS_MOD: u32 = 6;
pub const CLASS_MODPACK: u32 = 4471;
pub const CLASS_RESOURCEPACK: u32 = 12;
pub const CLASS_SHADER: u32 = 6552;

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct Pagination {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    page_size: u32,
    #[serde(default)]
    total_count: u64,
}

#[derive(Debug, Deserialize)]
struct SearchEnvelope {
    data: Vec<CurseForgeMod>,
    pagination: Pagination,
}

#[derive(Debug, Deserialize)]
struct FilesEnvelope {
    data: Vec<CurseForgeFile>,
}

#[derive(Debug, Deserialize)]
struct FingerprintEnvelope {
    data: FingerprintMatches,
}

#[derive(Debug, Deserialize)]
struct FingerprintMatches {
    #[serde(default, rename = "exactMatches")]
    exact_matches: Vec<FingerprintMatch>,
}

/// One fingerprint hit. Only the resolved `file` is needed — the match's own
/// id is an internal CurseForge row identifier we never surface.
#[derive(Debug, Deserialize)]
struct FingerprintMatch {
    #[serde(default)]
    file: Option<CurseForgeFile>,
}

/// CurseForge client.
#[derive(Clone)]
pub struct CurseForgeClient {
    http: reqwest::Client,
    api_key: Option<String>,
}

impl CurseForgeClient {
    pub fn new(http: reqwest::Client, api_key: Option<String>) -> Self {
        Self {
            http,
            api_key: api_key.filter(|key| !key.trim().is_empty()),
        }
    }

    /// `true` when a key is configured; the UI gates the CurseForge tab on it.
    pub fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    pub fn with_api_key(mut self, api_key: Option<String>) -> Self {
        self.api_key = api_key.filter(|key| !key.trim().is_empty());
        self
    }

    /// Search mods/modpacks.
    pub async fn search(&self, query: &ModSearchQuery) -> AppResult<ModSearchResults> {
        let limit = query.limit.unwrap_or(24).clamp(1, 50);
        let index = query.index.unwrap_or(0);
        // Mirror the Modrinth sort names the browser uses.
        let sort_field = match query.sort.as_deref() {
            Some("updated" | "newest") => "3",
            Some("relevance") | None => "2",
            _ => "6", // downloads / follows both map to TotalDownloads
        };
        let class_id = match query.project_type.as_deref() {
            Some("modpack") => CLASS_MODPACK,
            Some("resourcepack") => CLASS_RESOURCEPACK,
            Some("shader") => CLASS_SHADER,
            _ => CLASS_MOD,
        };

        let mut url = url::Url::parse(&format!("{CURSEFORGE_API}/mods/search"))?;
        {
            let mut params = url.query_pairs_mut();
            params.append_pair("gameId", &MINECRAFT_GAME_ID.to_string());
            params.append_pair("classId", &class_id.to_string());
            params.append_pair("index", &index.to_string());
            params.append_pair("pageSize", &limit.to_string());
            // Field ids: 2 = Popularity, 3 = LastUpdated, 4 = Name, 6 = TotalDownloads.
            params.append_pair("sortField", sort_field);
            params.append_pair("sortOrder", "desc");
            if let Some(text) = query.query.as_deref().filter(|text| !text.is_empty()) {
                params.append_pair("searchFilter", text);
            }
            if let Some(version) = query.game_version.as_deref().filter(|v| !v.is_empty()) {
                params.append_pair("gameVersion", version);
            }
            // Mod loader filtering uses category ids; the mapping is remote, so
            // we filter with `modLoaderType` instead.
            if let Some(loader) = query.loader.as_deref().and_then(mod_loader_type) {
                params.append_pair("modLoaderType", &loader.to_string());
            }
        }

        let response: SearchEnvelope = self.get_json(url.as_str()).await?;
        Ok(ModSearchResults {
            total: response.pagination.total_count,
            offset: response.pagination.index,
            limit: response.pagination.page_size,
            source: ModSource::CurseForge,
            hits: response
                .data
                .into_iter()
                .map(|project| ModSearchHit {
                    id: project.id.to_string(),
                    slug: project.slug.clone().unwrap_or_default(),
                    title: project.name.clone(),
                    description: project.summary.clone().unwrap_or_default(),
                    source: ModSource::CurseForge,
                    project_type: query.project_type.clone().unwrap_or_else(|| "mod".into()),
                    icon_url: project.logo.and_then(|logo| logo.url),
                    downloads: project.download_count,
                    categories: project
                        .categories
                        .iter()
                        .map(|category| category.name.clone())
                        .collect(),
                    game_versions: project
                        .latest_files
                        .iter()
                        .flat_map(|file| file.game_versions.clone())
                        .collect(),
                    loaders: Vec::new(),
                    latest_version: project
                        .latest_files
                        .first()
                        .map(|file| file.display_name.clone()),
                })
                .collect(),
        })
    }

    pub async fn project(&self, mod_id: u32) -> AppResult<CurseForgeMod> {
        let envelope: Envelope<CurseForgeMod> = self
            .get_json(&format!("{CURSEFORGE_API}/mods/{mod_id}"))
            .await?;
        Ok(envelope.data)
    }

    /// Files of a mod, newest first, in pages of 50 (the API maximum).
    pub async fn files(&self, mod_id: u32, game_version: Option<&str>) -> AppResult<Vec<CurseForgeFile>> {
        let mut collected = Vec::new();
        let mut index = 0u32;

        loop {
            let mut url = url::Url::parse(&format!("{CURSEFORGE_API}/mods/{mod_id}/files"))?;
            {
                let mut params = url.query_pairs_mut();
                params.append_pair("index", &index.to_string());
                params.append_pair("pageSize", "50");
                if let Some(version) = game_version {
                    params.append_pair("gameVersion", version);
                }
            }
            let envelope: FilesEnvelope = self.get_json(url.as_str()).await?;
            let received = envelope.data.len();
            collected.extend(envelope.data);
            if received < 50 {
                break;
            }
            index += 50;
            // Guard against a pathological project with thousands of files.
            if index >= 1000 {
                break;
            }
        }
        Ok(collected)
    }

    /// Files that support the requested game version and loader.
    ///
    /// CurseForge has no server-side "compatible with" filter for loaders: the
    /// loader name appears inside `gameVersions`, so we filter client-side.
    pub async fn compatible_files(
        &self,
        mod_id: u32,
        game_version: &str,
        loader: Option<&str>,
    ) -> AppResult<Vec<CurseForgeFile>> {
        let files = self.files(mod_id, Some(game_version)).await?;
        Ok(files
            .into_iter()
            .filter(|file| {
                file.game_versions.iter().any(|value| value == game_version)
                    && loader.is_none_or(|loader| file.supports_loader(loader))
                    && file.is_downloadable()
            })
            .collect())
    }

    /// Every downloadable file of a mod, unfiltered — the full history used by
    /// detail views (a project can ship for many game versions).
    pub async fn all_files(&self, mod_id: &u32) -> AppResult<Vec<CurseForgeFile>> {
        self.files(*mod_id, None).await
    }

    /// One file by project + file id (CurseForge pack manifests).
    pub async fn file(&self, mod_id: u32, file_id: u32) -> AppResult<CurseForgeFile> {
        let envelope: Envelope<CurseForgeFile> = self
            .get_json(&format!("{CURSEFORGE_API}/mods/{mod_id}/files/{file_id}"))
            .await?;
        Ok(envelope.data)
    }

    /// Resolve an installed jar back to its CurseForge file.
    pub async fn match_fingerprints(&self, fingerprints: &[u32]) -> AppResult<Vec<CurseForgeFile>> {
        if fingerprints.is_empty() {
            return Ok(Vec::new());
        }
        let response = self
            .post_json(
                &format!("{CURSEFORGE_API}/fingerprints"),
                &serde_json::json!({ "fingerprints": fingerprints }),
            )
            .await?;
        let envelope: FingerprintEnvelope = response;
        Ok(envelope
            .data
            .exact_matches
            .into_iter()
            .filter_map(|matched| matched.file)
            .collect())
    }

    /// Normalize a CurseForge file into the shared [`ModVersion`] shape.
    pub fn file_to_version(&self, file: &CurseForgeFile) -> ModVersion {
        ModVersion {
            id: file.id.to_string(),
            project_id: file.mod_id.to_string(),
            name: file.display_name.clone(),
            version_number: file.display_name.clone(),
            version_type: if file.release_type == 1 { "release" } else { "beta" }.to_string(),
            source: ModSource::CurseForge,
            game_versions: file.game_versions.clone(),
            loaders: Vec::new(),
            downloads: file.download_count,
            file_name: file.file_name.clone(),
            download_url: file.download_url.clone().unwrap_or_default(),
            hashes: ModHashes {
                sha1: file.hashes.iter().find(|h| h.algo == 1).and_then(|h| h.text()),
                sha512: None,
                murmur2: file.hashes.iter().find(|h| h.algo == 2).and_then(|h| h.number()),
            },
            file_size: file.file_length,
            published_at: file.file_date,
            dependencies: Vec::new(),
        }
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> AppResult<T> {
        let key = self.api_key.clone().ok_or_else(missing_key_error)?;
        let response = self
            .http
            .get(url)
            .header("x-api-key", key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|err| AppError::Network(format!("CurseForge request failed: {err}")))?;
        parse_response(response).await
    }

    async fn post_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> AppResult<T> {
        let key = self.api_key.clone().ok_or_else(missing_key_error)?;
        let response = self
            .http
            .post(url)
            .header("x-api-key", key)
            .json(body)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("CurseForge request failed: {err}")))?;
        parse_response(response).await
    }
}

fn missing_key_error() -> AppError {
    AppError::Config(
        "CurseForge needs a personal API key. Add one in Settings → Integrations \
         (or set SXML_CURSEFORGE_API_KEY)."
            .to_string(),
    )
}

async fn parse_response<T: for<'de> Deserialize<'de>>(response: reqwest::Response) -> AppResult<T> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(match status.as_u16() {
            401 | 403 => AppError::Config(
                "CurseForge rejected the API key. Check it in Settings → Integrations.".to_string(),
            ),
            404 => AppError::ModResolution("that project does not exist on CurseForge".to_string()),
            _ => AppError::Network(format!(
                "CurseForge returned HTTP {status}: {}",
                body.chars().take(200).collect::<String>()
            )),
        });
    }
    response
        .json::<T>()
        .await
        .map_err(|err| AppError::Network(format!("unexpected CurseForge payload: {err}")))
}

/// CurseForge `modLoaderType` ids.
pub fn mod_loader_type(loader: &str) -> Option<u32> {
    match loader.to_ascii_lowercase().as_str() {
        "forge" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

/// MurmurHash2 as CurseForge computes it for fingerprinting (seed `1`).
pub fn murmur2(data: &[u8]) -> u32 {
    const SEED: u32 = 1;
    const M: u32 = 0x5bd1e995;
    const R: u32 = 24;

    let mut hash = SEED ^ (data.len() as u32);
    let mut chunks = data.chunks_exact(4);

    for chunk in &mut chunks {
        let mut k = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);

        hash = hash.wrapping_mul(M);
        hash ^= k;
    }

    let remainder = chunks.remainder();
    match remainder.len() {
        3 => {
            hash ^= (remainder[2] as u32) << 16;
            hash ^= (remainder[1] as u32) << 8;
            hash ^= remainder[0] as u32;
            hash = hash.wrapping_mul(M);
        }
        2 => {
            hash ^= (remainder[1] as u32) << 8;
            hash ^= remainder[0] as u32;
            hash = hash.wrapping_mul(M);
        }
        1 => {
            hash ^= remainder[0] as u32;
            hash = hash.wrapping_mul(M);
        }
        _ => {}
    }

    hash ^= hash >> 13;
    hash = hash.wrapping_mul(M);
    hash ^= hash >> 15;
    hash
}

/// CurseForge fingerprint: MurmurHash2 over the file with whitespace removed.
///
/// Only `\t \n \r \x0b \x0c \x20` are stripped, and nothing else — removing
/// more would change the hash and break matching.
pub fn fingerprint(bytes: &[u8]) -> u32 {
    const WHITESPACE: [u8; 6] = [0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20];
    let filtered: Vec<u8> = bytes
        .iter()
        .copied()
        .filter(|byte| !WHITESPACE.contains(byte))
        .collect();
    murmur2(&filtered)
}

/// Fingerprint a jar on disk.
pub fn fingerprint_file(path: &std::path::Path) -> AppResult<u32> {
    let bytes = std::fs::read(path)?;
    Ok(fingerprint(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deliberately naive byte-index port of the canonical C function, used
    /// as an independent oracle. It shares no structure with the `chunks_exact`
    /// version under test, so it catches transcription slips in the loop.
    fn murmur2_by_hand(data: &[u8], seed: u32) -> u32 {
        const M: u32 = 0x5bd1e995;
        const R: u32 = 24;

        let mut h = seed ^ (data.len() as u32);
        let mut index = 0;
        while data.len() - index >= 4 {
            let mut k = (data[index] as u32)
                | ((data[index + 1] as u32) << 8)
                | ((data[index + 2] as u32) << 16)
                | ((data[index + 3] as u32) << 24);
            k = k.wrapping_mul(M);
            k ^= k >> R;
            k = k.wrapping_mul(M);
            h = h.wrapping_mul(M);
            h ^= k;
            index += 4;
        }

        // The C switch falls through, so the tail XORs accumulate and the
        // multiply happens exactly once.
        match data.len() - index {
            3 => {
                h ^= (data[index + 2] as u32) << 16;
                h ^= (data[index + 1] as u32) << 8;
                h ^= data[index] as u32;
                h = h.wrapping_mul(M);
            }
            2 => {
                h ^= (data[index + 1] as u32) << 8;
                h ^= data[index] as u32;
                h = h.wrapping_mul(M);
            }
            1 => {
                h ^= data[index] as u32;
                h = h.wrapping_mul(M);
            }
            _ => {}
        }

        h ^= h >> 13;
        h = h.wrapping_mul(M);
        h ^= h >> 15;
        h
    }

    #[test]
    fn murmur2_handles_the_seed_and_the_final_avalanche() {
        // Empty input skips every data-dependent step, so the result is just
        // the avalanche over `seed ^ len`. Written longhand here so the
        // expectation is derivable by inspection rather than trusted as a
        // magic constant.
        const M: u32 = 0x5bd1e995;
        let mut expected = 1u32; // seed ^ 0
        expected ^= expected >> 13;
        expected = expected.wrapping_mul(M);
        expected ^= expected >> 15;
        assert_eq!(murmur2(b""), expected);

        // Seed 0 with no input must stay zero: nothing perturbs the state.
        assert_eq!(murmur2_by_hand(b"", 0), 0);
    }

    #[test]
    fn murmur2_matches_an_independent_reference_on_every_tail_length() {
        // Lengths 0..=8 cover the empty case, all three tail lengths (1, 2, 3)
        // and multi-chunk inputs.
        let inputs: [&[u8]; 9] = [
            b"",
            b"a",
            b"ab",
            b"abc",
            b"abcd",
            b"abcde",
            b"abcdef",
            b"abcdefg",
            b"abcdefgh",
        ];
        for input in inputs {
            assert_eq!(
                murmur2(input),
                murmur2_by_hand(input, 1),
                "seed-1 mismatch for {input:?}"
            );
        }
    }

    #[test]
    fn murmur2_matches_reference_vectors() {
        // Regression vectors, captured from an independent byte-index
        // transcription of the canonical C function. They pin the exact output
        // so a future edit to the loop or the tail handling is caught.
        //
        // NOTE: these are *MurmurHash2* values. MurmurHash3's well-known
        // `0x514E28B7` for the empty string with seed 1 does not belong here.
        assert_eq!(murmur2(b""), 0x5BD15E36);
        assert_eq!(murmur2(b"hello world"), 0x83EA5DEE);
        assert_eq!(murmur2(b"abc"), 0x60A4FCC1);
        assert_eq!(murmur2(b"abcd"), 0xC93F7A16);
        assert_eq!(
            murmur2(b"The quick brown fox jumps over the lazy dog"),
            0x1E1049E7
        );
    }

    #[test]
    fn fingerprint_ignores_class_file_whitespace() {
        // Two jars that differ only in whitespace must fingerprint identically.
        assert_eq!(fingerprint(b"class A { }"), fingerprint(b"class A {\t\n}"));
        // Non-whitespace differences must change the hash.
        assert_ne!(fingerprint(b"class A { }"), fingerprint(b"class B { }"));
    }

    #[test]
    fn loader_ids_cover_every_supported_loader() {
        assert_eq!(mod_loader_type("forge"), Some(1));
        assert_eq!(mod_loader_type("Fabric"), Some(4));
        assert_eq!(mod_loader_type("quilt"), Some(5));
        assert_eq!(mod_loader_type("neoforge"), Some(6));
        assert_eq!(mod_loader_type("vanilla"), None);
    }

    #[test]
    fn missing_api_key_produces_an_actionable_error() {
        let client = CurseForgeClient::new(reqwest::Client::new(), None);
        assert!(!client.is_configured());
        let error = missing_key_error();
        assert!(error.to_string().contains("Settings"));
    }

    #[test]
    fn blank_api_keys_are_treated_as_missing() {
        let client = CurseForgeClient::new(reqwest::Client::new(), Some("   ".into()));
        assert!(!client.is_configured());
    }
}
