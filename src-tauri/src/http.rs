//! Shared HTTP client for registries and sx.acc.
//!
//! Reqwest is built with `default-features = false`. That used to omit both
//! the OS certificate store and the Windows/macOS system proxy. A machine
//! running TLS inspection (Norton and similar filters install their own root
//! and often a local proxy) then fails every `send()` with
//! `error sending request` before the server can return a status. Public
//! webpki roots stay enabled so a normal network still verifies Let's Encrypt,
//! and the OS store is loaded beside them so an installed filter root is
//! trusted too.

use std::time::Duration;

/// Build the process-wide client. Callers that need a custom timeout clone
/// the builder from [`client_builder`].
pub fn client() -> reqwest::Client {
    client_builder()
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(concat!("SXMLauncher/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(45))
        .pool_max_idle_per_host(8)
}

/// Walk `source()` so a reqwest "error sending request" keeps the TLS, DNS,
/// or proxy reason that `Display` hides.
pub fn error_chain(err: &dyn std::error::Error) -> String {
    let mut parts = Vec::new();
    let mut current: Option<&dyn std::error::Error> = Some(err);
    while let Some(item) = current {
        let text = item.to_string();
        if !parts.iter().any(|seen: &String| seen == &text) {
            parts.push(text);
        }
        current = item.source();
        if parts.len() == 5 {
            break;
        }
    }
    parts.join(": ")
}
