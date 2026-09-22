//! Modrinth API v2 client.
//!
//! Docs: <https://docs.modrinth.com/api-spec/>
//!
//! Notes that matter in practice:
//! * Modrinth requires a descriptive `User-Agent`; generic ones get rate limited.
//! * Search filters are expressed as a JSON `facets` array of OR-groups whose
//!   members are AND-ed, e.g. `[["project_type:mod"],["versions:1.20.1"]]`.
//! * Hash lookups are how "import my existing instance" works without a
//!   project id: POST `/version_files` with the SHA-1s of the jars on disk.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::modpack::{
    ModHashes, ModProject, ModSearchHit, ModSearchQuery, ModSearchResults, ModSource, ModVersion,
    PackDependencyKind,
};
use crate::store::Database;

/// Modrinth v2 base URL.
pub const MODRINTH_API: &str = "https://api.modrinth.com/v2";
/// Modrinth asks for a contact address in the agent string.
pub const USER_AGENT: &str = concat!(
    "sxmlauncher/",
    env!("CARGO_PKG_VERSION"),
    " (github.com/sxmlauncher/sxmlauncher)"
);
/// Search results are cheap to cache; projects change rarely.
const SEARCH_TTL_SECS: i64 = 300;
const PROJECT_TTL_SECS: i64 = 900;
/// Max facet OR-group size Modrinth accepts is generous, but a long URL can be
/// rejected by proxies, so we chunk version facets.
const FACET_CHUNK: usize = 40;
/// Mod categories Modrinth treats as *loaders*, not gameplay categories.
pub const LOADER_FACETS: [&str; 6] = ["fabric", "quilt", "forge", "neoforge", "rift", "liteloader"];

#[derive(Debug, Deserialize)]
struct SearchResponse {
    hits: Vec<SearchHit>,
    total_hits: u64,
    offset: u32,
    limit: u32,
}

#[derive(Debug, Deserialize)]
struct SearchHit {
    project_id: String,
    slug: String,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    versions: Vec<String>,
    #[serde(default)]
    project_type: String,
    #[serde(default)]
    latest_version: Option<String>,
    #[serde(default)]
    gallery: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ProjectResponse {
    id: String,
    slug: String,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    followers: u64,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    project_type: String,
    #[serde(default)]
    license: Option<LicenseObject>,
    #[serde(default)]
    updated: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    client_side: Option<String>,
    #[serde(default)]
    server_side: Option<String>,
    #[serde(default)]
    gallery: Vec<GalleryEntry>,
}

#[derive(Debug, Deserialize)]
struct LicenseObject {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GalleryEntry {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    featured: bool,
}

#[derive(Debug, Deserialize)]
struct VersionResponse {
    id: String,
    project_id: String,
    name: String,
    version_number: String,
    #[serde(default)]
    version_type: String,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    date_published: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    dependencies: Vec<DependencyResponse>,
    files: Vec<FileResponse>,
}

#[derive(Debug, Deserialize)]
struct FileResponse {
    #[serde(default)]
    hashes: HashesResponse,
    url: String,
    filename: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    primary: bool,
}

#[derive(Debug, Default, Deserialize)]
struct HashesResponse {
    #[serde(default)]
    sha1: Option<String>,
    #[serde(default)]
    sha512: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DependencyResponse {
    #[serde(default)]
    version_id: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    dependency_type: String,
}

/// Modrinth client with an optional SQLite metadata cache.
#[derive(Clone)]
pub struct ModrinthClient {
    http: reqwest::Client,
    cache: Option<Database>,
}

impl ModrinthClient {
    pub fn new(http: reqwest::Client, cache: Option<Database>) -> Self {
        Self { http, cache }
    }

    /// Search projects with optional version/loader/category facets.
    pub async fn search(&self, query: &ModSearchQuery) -> AppResult<ModSearchResults> {
        let limit = query.limit.unwrap_or(24).clamp(1, 100);
        let offset = query.index.unwrap_or(0);
        let facets = build_facets(query);
        let sort = normalize_sort(query.sort.as_deref());        // Every axis that shapes the result must be in the key: paging or
        // re-sorting previously returned the *first* page again, so the
        // browser could never advance past page 1.
        let cache_key = format!(
            "modrinth:search:{}:{}:{}:{}:{}:{}:{}",
            query.query.as_deref().unwrap_or(""),
            query.project_type.as_deref().unwrap_or(""),
            query.game_version.as_deref().unwrap_or(""),
            query.loader.as_deref().unwrap_or(""),
            sort,
            query.index.unwrap_or(0),
            limit
        );
        if let Some(cached) = self.cached::<ModSearchResults>(&cache_key).await? {
            return Ok(cached);
        }

        let mut url = url::Url::parse(&format!("{MODRINTH_API}/search"))?;
        {
            let mut params = url.query_pairs_mut();
            params.append_pair("query", query.query.as_deref().unwrap_or(""));
            params.append_pair("limit", &limit.to_string());
            params.append_pair("offset", &offset.to_string());
            params.append_pair("index", sort);
            if !facets.is_empty() {
                params.append_pair("facets", &facets);
            }
        }

        let response = self.get_json::<SearchResponse>(url.as_str()).await?;
        let results = ModSearchResults {
            total: response.total_hits,
            offset: response.offset,
            limit: response.limit,
            source: ModSource::Modrinth,
            hits: response
                .hits
                .into_iter()
                .map(|hit| ModSearchHit {
                    id: hit.project_id,
                    slug: hit.slug,
                    title: hit.title,
                    description: hit.description,
                    source: ModSource::Modrinth,
                    project_type: hit.project_type,
                    icon_url: hit.icon_url.or_else(|| hit.gallery.first().cloned()),
                    downloads: hit.downloads,
                    categories: hit.categories,
                    game_versions: hit.versions,
                    loaders: Vec::new(),
                    latest_version: hit.latest_version,
                })
                .collect(),
        };

        self.store_cache(&cache_key, &results, SEARCH_TTL_SECS).await?;
        Ok(results)
    }

    /// Full project metadata (`id` may be an id or a slug).
    pub async fn project(&self, id: &str) -> AppResult<ModProject> {
        let cache_key = format!("modrinth:project:{id}");
        if let Some(cached) = self.cached::<ModProject>(&cache_key).await? {
            return Ok(cached);
        }

        let response = self
            .get_json::<ProjectResponse>(&format!("{MODRINTH_API}/project/{id}"))
            .await?;

        let project = ModProject {
            id: response.id,
            slug: response.slug,
            title: response.title,
            description: response.description,
            source: ModSource::Modrinth,
            project_type: response.project_type,
            icon_url: response.icon_url.or_else(|| {
                response
                    .gallery
                    .iter()
                    .find(|entry| entry.featured)
                    .and_then(|entry| entry.url.clone())
            }),
            downloads: response.downloads,
            followers: response.followers,
            categories: response.categories,
            game_versions: response.versions,
            loaders: response.loaders,
            license: response
                .license
                .and_then(|license| license.name.or(license.id)),
            updated_at: response.updated,
            client_side: response.client_side,
            server_side: response.server_side,
        };

        self.store_cache(&cache_key, &project, PROJECT_TTL_SECS).await?;
        Ok(project)
    }

    /// Versions of a project, newest first, optionally filtered by game version
    /// and loader.
    pub async fn versions(&self, id: &str) -> AppResult<Vec<ModVersion>> {
        let response = self
            .get_json::<Vec<VersionResponse>>(&format!("{MODRINTH_API}/project/{id}/version"))
            .await?;
        Ok(response.into_iter().map(version_from_response).collect())
    }

    /// All versions of a project compatible with an instance.
    pub async fn compatible_versions(
        &self,
        id: &str,
        game_version: &str,
        loader: Option<&str>,
    ) -> AppResult<Vec<ModVersion>> {
        let mut url = url::Url::parse(&format!("{MODRINTH_API}/project/{id}/version"))?;
        {
            let mut params = url.query_pairs_mut();
            params.append_pair("game_versions", &serde_json::to_string(&[game_version])?);
            if let Some(loader) = loader {
                params.append_pair("loaders", &serde_json::to_string(&[loader])?);
            }
        }
        let response = self.get_json::<Vec<VersionResponse>>(url.as_str()).await?;
        Ok(response.into_iter().map(version_from_response).collect())
    }

    /// Every published version of a project, newest first — the unfiltered
    /// history used by detail views (a pack can span many game versions).
    pub async fn all_versions(&self, id: &str) -> AppResult<Vec<ModVersion>> {
        let response = self
            .get_json::<Vec<VersionResponse>>(&format!("{MODRINTH_API}/project/{id}/version"))
            .await?;
        Ok(response.into_iter().map(version_from_response).collect())
    }

    pub async fn version(&self, version_id: &str) -> AppResult<ModVersion> {
        let cache_key = format!("modrinth:version:{version_id}");
        if let Some(cached) = self.cached::<ModVersion>(&cache_key).await? {
            return Ok(cached);
        }
        let response = self
            .get_json::<VersionResponse>(&format!("{MODRINTH_API}/version/{version_id}"))
            .await?;
        let version = version_from_response(response);
        self.store_cache(&cache_key, &version, PROJECT_TTL_SECS).await?;
        Ok(version)
    }

    /// Reverse-hash lookup: `sha1 -> version` for the jars already on disk.
    pub async fn versions_from_hashes(
        &self,
        hashes: &[String],
        algorithm: &str,
    ) -> AppResult<Vec<ModVersion>> {
        if hashes.is_empty() {
            return Ok(Vec::new());
        }
        let payload = serde_json::json!({
            "hashes": hashes,
            "algorithm": algorithm
        });
        let response = self
            .http
            .post(format!("{MODRINTH_API}/version_files"))
            .header("User-Agent", USER_AGENT)
            .json(&payload)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("Modrinth hash lookup failed: {err}")))?;

        if !response.status().is_success() {
            return Err(AppError::ModResolution(format!(
                "Modrinth hash lookup failed with HTTP {}",
                response.status()
            )));
        }

        // The endpoint answers with a map keyed by hash.
        let map: std::collections::HashMap<String, VersionResponse> = response
            .json()
            .await
            .map_err(|err| AppError::ModResolution(format!("unexpected hash response: {err}")))?;
        Ok(map.into_values().map(version_from_response).collect())
    }

    /// Latest version matching an instance's version + loader.
    pub async fn latest_compatible(
        &self,
        project_id: &str,
        game_version: &str,
        loader: Option<&str>,
    ) -> AppResult<Option<ModVersion>> {
        let mut versions = self
            .compatible_versions(project_id, game_version, loader)
            .await?;
        // Modrinth returns newest first, but be explicit rather than trusting it.
        versions.sort_by(|a, b| b.published_at.cmp(&a.published_at));
        Ok(versions.into_iter().next())
    }

    /// Fetch raw JSON with the required headers.
    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> AppResult<T> {
        let response = self
            .http
            .get(url)
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .map_err(|err| AppError::Network(format!("Modrinth request failed: {err}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(match status.as_u16() {
                404 => AppError::ModResolution("that project no longer exists on Modrinth".into()),
                429 => AppError::Network(
                    "Modrinth is rate limiting us; try again in a moment".into(),
                ),
                _ => AppError::Network(format!(
                    "Modrinth returned HTTP {status}: {}",
                    body.chars().take(200).collect::<String>()
                )),
            });
        }
        response
            .json::<T>()
            .await
            .map_err(|err| AppError::Network(format!("unexpected Modrinth payload: {err}")))
    }

    async fn cached<T: for<'de> Deserialize<'de>>(&self, key: &str) -> AppResult<Option<T>> {
        let Some(cache) = &self.cache else {
            return Ok(None);
        };
        let Some(payload) = cache.cache_get(key)? else {
            return Ok(None);
        };
        // A stale cache entry that no longer deserializes is treated as a miss
        // rather than an error.
        Ok(serde_json::from_str(&payload).ok())
    }

    async fn store_cache<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: i64,
    ) -> AppResult<()> {
        let Some(cache) = &self.cache else {
            return Ok(());
        };
        let payload = serde_json::to_string(value)?;
        cache.cache_put(key, &payload, ttl_secs)?;
        Ok(())
    }
}

fn version_from_response(response: VersionResponse) -> ModVersion {
    // Prefer the file Modrinth marks as primary.
    let file = response
        .files
        .iter()
        .find(|file| file.primary)
        .or_else(|| response.files.first());

    ModVersion {
        id: response.id,
        project_id: response.project_id,
        name: response.name,
        version_number: response.version_number,
        version_type: if response.version_type.is_empty() {
            "release".to_string()
        } else {
            response.version_type
        },
        source: ModSource::Modrinth,
        game_versions: response.game_versions,
        loaders: response.loaders,
        downloads: response.downloads,
        file_name: file.map(|file| file.filename.clone()).unwrap_or_default(),
        download_url: file.map(|file| file.url.clone()).unwrap_or_default(),
        hashes: file
            .map(|file| ModHashes {
                sha1: file.hashes.sha1.clone(),
                sha512: file.hashes.sha512.clone(),
                murmur2: None,
            })
            .unwrap_or_default(),
        file_size: file.map(|file| file.size).unwrap_or(0),
        published_at: response.date_published,
        dependencies: response
            .dependencies
            .into_iter()
            .map(|dependency| crate::models::modpack::ModDependency {
                kind: match dependency.dependency_type.as_str() {
                    "optional" => PackDependencyKind::Optional,
                    "incompatible" => PackDependencyKind::Incompatible,
                    "embedded" => PackDependencyKind::Embedded,
                    _ => PackDependencyKind::Required,
                },
                project_id: dependency.project_id,
                version_id: dependency.version_id,
                file_name: dependency.file_name,
            })
            .collect(),
    }
}

/// Encode the search filters as Modrinth's `facets` JSON.
///
/// Each inner array is an OR-group; groups are AND-ed together.
pub fn build_facets(query: &ModSearchQuery) -> String {
    let mut groups: Vec<Vec<String>> = Vec::new();

    let project_types: Vec<String> = match query.project_type.as_deref() {
        Some("modpack") => vec!["project_type:modpack".to_string()],
        Some("mod") | None => vec!["project_type:mod".to_string()],
        Some(other) => vec![format!("project_type:{other}")],
    };
    groups.push(project_types);

    if let Some(version) = query.game_version.as_deref().filter(|v| !v.is_empty()) {
        groups.push(vec![format!("versions:{version}")]);
    }

    // The loader is both a category facet and a loader facet on Modrinth.
    let mut loader_group: Vec<String> = Vec::new();
    if let Some(loader) = query.loader.as_deref().filter(|l| !l.is_empty()) {
        loader_group.push(format!("categories:{loader}"));
    }
    if !loader_group.is_empty() {
        groups.push(loader_group);
    }

    // Gameplay categories are AND-ed as separate groups so "adventure" +
    // "optimization" cannot match a project that only has one of them.
    for category in query.categories.iter().filter(|c| !c.is_empty()) {
        if LOADER_FACETS.contains(&category.as_str()) {
            continue;
        }
        groups.push(vec![format!("categories:{category}")]);
    }

    serde_json::to_string(&groups).unwrap_or_else(|_| "[]".to_string())
}

/// Map the UI sort names onto Modrinth's `index` values.
pub fn normalize_sort(sort: Option<&str>) -> &'static str {
    match sort.unwrap_or("relevance") {
        "downloads" => "downloads",
        "follows" => "follows",
        "newest" | "date" => "newest",
        "updated" => "updated",
        _ => "relevance",
    }
}

/// Request timeout used by the shared HTTP client.
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Build the shared HTTP client (kept here so every registry uses the same
/// timeouts and connection pool).
pub fn http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(HTTP_TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(8)
        .build()
        .map_err(|err| AppError::Network(format!("cannot build http client: {err}")))
}

/// `facets` chunking helper for very long version lists.
pub fn chunk_facets(values: &[String]) -> Vec<Vec<String>> {
    values.chunks(FACET_CHUNK).map(<[String]>::to_vec).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(loader: Option<&str>, version: Option<&str>, categories: Vec<&str>) -> ModSearchQuery {
        ModSearchQuery {
            query: Some("sodium".into()),
            source: ModSource::Modrinth,
            project_type: Some("mod".into()),
            game_version: version.map(str::to_string),
            loader: loader.map(str::to_string),
            categories: categories.into_iter().map(str::to_string).collect(),
            sort: None,
            index: None,
            limit: None,
        }
    }

    #[test]
    fn facets_encode_project_type_version_and_loader() {
        let facets = build_facets(&query(Some("fabric"), Some("1.20.1"), vec![]));
        let parsed: Vec<Vec<String>> = serde_json::from_str(&facets).expect("valid facets json");
        assert!(parsed.contains(&vec!["project_type:mod".to_string()]));
        assert!(parsed.contains(&vec!["versions:1.20.1".to_string()]));
        assert!(parsed.contains(&vec!["categories:fabric".to_string()]));
    }

    #[test]
    fn facets_and_gameplay_categories_instead_of_oring_them() {
        let facets = build_facets(&query(Some("fabric"), None, vec!["adventure", "optimization"]));
        let parsed: Vec<Vec<String>> = serde_json::from_str(&facets).expect("valid facets");
        assert!(parsed.contains(&vec!["categories:adventure".to_string()]));
        assert!(parsed.contains(&vec!["categories:optimization".to_string()]));
        // A loader passed as a category must not be duplicated as a gameplay tag.
        let duplicated = parsed
            .iter()
            .filter(|group| group.contains(&"categories:fabric".to_string()))
            .count();
        assert_eq!(duplicated, 1);
    }

    #[test]
    fn modpack_search_uses_the_modpack_project_type() {
        let mut request = query(None, Some("1.20.1"), vec![]);
        request.project_type = Some("modpack".into());
        let facets = build_facets(&request);
        assert!(facets.contains("project_type:modpack"));
        assert!(!facets.contains("project_type:mod\""));
    }

    #[test]
    fn sort_names_map_to_modrinth_indexes() {
        assert_eq!(normalize_sort(None), "relevance");
        assert_eq!(normalize_sort(Some("downloads")), "downloads");
        assert_eq!(normalize_sort(Some("newest")), "newest");
        assert_eq!(normalize_sort(Some("nonsense")), "relevance");
    }

    #[test]
    fn version_response_prefers_the_primary_file() {
        let response: VersionResponse = serde_json::from_value(serde_json::json!({
            "id": "v1",
            "project_id": "p1",
            "name": "Sodium 0.5.8",
            "version_number": "0.5.8",
            "version_type": "release",
            "game_versions": ["1.20.1"],
            "loaders": ["fabric"],
            "downloads": 42,
            "files": [
                { "url": "https://cdn/secondary.jar", "filename": "secondary.jar", "primary": false, "size": 10, "hashes": { "sha1": "bb" } },
                { "url": "https://cdn/primary.jar", "filename": "sodium.jar", "primary": true, "size": 20, "hashes": { "sha1": "aa", "sha512": "cc" } }
            ],
            "dependencies": []
        }))
        .expect("version fixture");

        let version = version_from_response(response);
        assert_eq!(version.file_name, "sodium.jar");
        assert_eq!(version.download_url, "https://cdn/primary.jar");
        assert_eq!(version.hashes.sha1.as_deref(), Some("aa"));
        assert_eq!(version.file_size, 20);
    }

    #[test]
    fn version_without_files_degrades_gracefully() {
        let response: VersionResponse = serde_json::from_value(serde_json::json!({
            "id": "v1",
            "project_id": "p1",
            "name": "x",
            "version_number": "1.0",
            "files": []
        }))
        .expect("version fixture");
        let version = version_from_response(response);
        assert!(version.download_url.is_empty());
        assert_eq!(version.version_type, "release");
    }

    #[test]
    fn require_hashes_maps_dependency_types() {
        let response: VersionResponse = serde_json::from_value(serde_json::json!({
            "id": "v1",
            "project_id": "p1",
            "name": "x",
            "version_number": "1.0",
            "files": [{ "url": "u", "filename": "f.jar", "hashes": {} }],
            "dependencies": [
                { "project_id": "req", "dependency_type": "required" },
                { "project_id": "opt", "dependency_type": "optional" },
                { "project_id": "bad", "dependency_type": "incompatible" },
                { "project_id": "emb", "dependency_type": "embedded" }
            ]
        }))
        .expect("version fixture");

        let version = version_from_response(response);
        let kinds: Vec<PackDependencyKind> = version.dependencies.iter().map(|d| d.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PackDependencyKind::Required,
                PackDependencyKind::Optional,
                PackDependencyKind::Incompatible,
                PackDependencyKind::Embedded
            ]
        );
    }

    #[test]
    fn facet_chunking_splits_long_version_lists() {
        let values: Vec<String> = (0..FACET_CHUNK + 5).map(|i| format!("v{i}")).collect();
        let chunks = chunk_facets(&values);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), FACET_CHUNK);
        assert_eq!(chunks[1].len(), 5);
    }
}
