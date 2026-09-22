//! OAuth2 helpers shared by the Microsoft and Ely.by flows: PKCE (RFC 7636) and
//! a loopback redirect server (RFC 8252, the native-app best practice).
//!
//! The loopback server binds `127.0.0.1` on an ephemeral port and answers
//! exactly one authorization callback. It is intentionally dependency-free: a
//! full HTTP framework for one request would be dead weight, and hand-parsing
//! the request line keeps the surface that faces the browser tiny.

use std::net::SocketAddr;
use std::time::Duration;

use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::error::{AppError, AppResult};

/// How long we wait for the user to finish signing in.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
/// Hard cap on the request we are willing to parse.
const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// A PKCE verifier/challenge pair.
#[derive(Debug, Clone)]
pub struct PkceCode {
    pub verifier: String,
    pub challenge: String,
}

impl PkceCode {
    /// Generate a 43-character verifier from 32 random bytes (no padding).
    pub fn generate() -> AppResult<Self> {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let verifier = base64_url(&bytes, false);

        let digest = Sha256::digest(verifier.as_bytes());
        let challenge = base64_url(&digest, false);

        Ok(Self {
            verifier,
            challenge,
        })
    }

    /// RFC 7636 method identifier.
    pub fn method() -> &'static str {
        "S256"
    }
}

/// Base64url without padding, as required by PKCE and JWT-style fields.
pub fn base64_url(bytes: &[u8], pad: bool) -> String {
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let encoded = engine.encode(bytes);
    if pad {
        encoded
    } else {
        encoded.trim_end_matches('=').to_string()
    }
}

/// Random `state` value used for CSRF protection on the redirect.
pub fn random_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64_url(&bytes, false)
}

/// The result of a successful browser redirect.
#[derive(Debug, Clone)]
pub struct CallbackResult {
    pub code: String,
    pub state: String,
}

/// Loopback redirect listener for the authorization code flow.
pub struct LoopbackServer {
    listener: TcpListener,
    redirect_uri: String,
}

impl LoopbackServer {
    /// Bind `127.0.0.1:0` and derive the redirect URI.
    pub async fn bind() -> AppResult<Self> {
        Self::bind_on(0).await
    }

    /// Bind a *fixed* port.
    ///
    /// Needed by Ely.by: their application registration compares `redirect_uri`
    /// exactly, so a random ephemeral port can never match. The port is part of
    /// the Settings → Fixes value the user registered with them.
    pub async fn bind_on(port: u16) -> AppResult<Self> {
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
            .await
            .map_err(|err| {
                AppError::Account(format!(
                    "cannot bind the sign-in callback on 127.0.0.1:{port} ({err}). \
                     Another SXMLAUNCHER instance may already be listening there."
                ))
            })?;
        let bound = listener
            .local_addr()
            .map_err(|err| AppError::Account(format!("cannot read loopback port: {err}")))?
            .port();
        Ok(Self {
            listener,
            redirect_uri: format!("http://localhost:{bound}/callback"),
        })
    }

    /// The exact value that must be sent as `redirect_uri`.
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    pub fn port(&self) -> Option<u16> {
        self.listener.local_addr().ok().map(|addr| addr.port())
    }

    /// Wait for the browser to hit `/callback?code=...&state=...`.
    ///
    /// Runs until the timeout expires; browser noise (favicon requests, empty
    /// preflight connections) is answered and ignored.
    pub async fn wait_for_code(self, expected_state: &str) -> AppResult<CallbackResult> {
        self.wait_for_code_with_timeout(expected_state, CALLBACK_TIMEOUT)
            .await
    }

    pub async fn wait_for_code_with_timeout(
        self,
        expected_state: &str,
        timeout: Duration,
    ) -> AppResult<CallbackResult> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let accept = tokio::time::timeout_at(deadline, self.listener.accept()).await;
            let (mut socket, _peer) = match accept {
                Ok(Ok(connection)) => connection,
                Ok(Err(err)) => {
                    return Err(AppError::Account(format!("callback accept failed: {err}")))
                }
                Err(_) => {
                    return Err(AppError::Account(
                        "timed out waiting for the browser to complete sign-in".to_string(),
                    ))
                }
            };

            // A callback request is tiny; anything larger is not ours.
            let mut buffer = vec![0u8; MAX_REQUEST_BYTES];
            let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer)).await;
            let Ok(Ok(read)) = read else {
                continue;
            };
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();

            let Some(target) = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
            else {
                continue;
            };

            // Ignore anything that is not the callback path (favicon, probes).
            if !target.starts_with("/callback") {
                let _ = respond(&mut socket, 404, "Not found", "Waiting for sign-in…").await;
                continue;
            }

            // Parse query parameters from the request target.
            let parsed = url::Url::parse(&format!("http://127.0.0.1{target}")).map_err(|err| {
                AppError::Account(format!("malformed OAuth callback url: {err}"))
            })?;
            let mut code = None;
            let mut state = None;
            let mut error = None;
            for (key, value) in parsed.query_pairs() {
                match key.as_ref() {
                    "code" => code = Some(value.into_owned()),
                    "state" => state = Some(value.into_owned()),
                    "error" => error = Some(value.into_owned()),
                    "error_description" => {
                        let description = value.into_owned();
                        error = Some(match error.take() {
                            Some(previous) => format!("{previous}: {description}"),
                            None => description,
                        })
                    }
                    _ => {}
                }
            }

            if let Some(error) = error {
                let _ = respond(
                    &mut socket,
                    400,
                    "Sign-in failed",
                    &format!("The provider returned an error: {error}"),
                )
                .await;
                return Err(AppError::Account(format!(
                    "authorization denied by provider: {error}"
                )));
            }

            let (Some(code), Some(state)) = (code, state) else {
                let _ = respond(
                    &mut socket,
                    400,
                    "Malformed callback",
                    "The sign-in response was missing its code or state.",
                )
                .await;
                continue;
            };

            // Reject a forged/duplicated callback before exchanging the code.
            if state != expected_state {
                let _ = respond(
                    &mut socket,
                    400,
                    "State mismatch",
                    "This sign-in attempt did not originate from SXMLAUNCHER.",
                )
                .await;
                return Err(AppError::Account(
                    "OAuth state mismatch; the callback did not match this sign-in attempt"
                        .to_string(),
                ));
            }

            let _ = respond(
                &mut socket,
                200,
                "You can close this tab",
                "SXMLAUNCHER is finishing your sign-in…",
            )
            .await;

            return Ok(CallbackResult { code, state });
        }
    }
}

/// Minimal HTTP/1.1 response with a dark-styled landing page.
async fn respond(
    socket: &mut tokio::net::TcpStream,
    status: u16,
    title: &str,
    message: &str,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Not Found",
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <title>{title}</title></head>\
         <body style=\"margin:0;display:flex;align-items:center;justify-content:center;\
         height:100vh;background:#0b0b12;color:#e6e6f0;\
         font-family:system-ui,-apple-system,Segoe UI,sans-serif\">\
         <main style=\"text-align:center;padding:2rem;border-radius:1rem;\
         background:rgba(255,255,255,0.04);border:1px solid rgba(255,255,255,0.08)\">\
         <p style=\"margin:0 0 .75rem;letter-spacing:.16em;font-size:.7rem;opacity:.55\">SXMLAUNCHER</p>\
         <h1 style=\"font-size:1.25rem;font-weight:600;margin:0 0 .5rem\">{title}</h1>\
         <p style=\"margin:0;opacity:.7\">{message}</p></main></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await?;
    socket.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_verifier_and_challenge_are_well_formed() {
        let pkce = PkceCode::generate().expect("pkce");
        // RFC 7636 requires 43..=128 characters for the verifier.
        assert_eq!(pkce.verifier.len(), 43);
        assert!(!pkce.verifier.contains('='));
        assert_eq!(pkce.challenge.len(), 43);
        // The challenge must be the unpadded base64url of SHA-256(verifier).
        let expected = base64_url(&Sha256::digest(pkce.verifier.as_bytes()), false);
        assert_eq!(pkce.challenge, expected);
        assert_eq!(PkceCode::method(), "S256");
    }

    #[test]
    fn pkce_values_are_unique_per_attempt() {
        let first = PkceCode::generate().expect("pkce");
        let second = PkceCode::generate().expect("pkce");
        assert_ne!(first.verifier, second.verifier);
        assert_ne!(random_state(), random_state());
    }

    #[tokio::test]
    async fn loopback_server_captures_the_authorization_code() {
        let server = LoopbackServer::bind().await.expect("bind");
        let redirect = server.redirect_uri().to_string();
        let port = server.port().expect("port");
        // Microsoft's identity platform expects `http://localhost` for loopback
        // redirects (127.0.0.1 is the legacy spelling and gets AADSTS50011).
        assert!(redirect.starts_with("http://localhost:"));

        let handle = tokio::spawn(async move { server.wait_for_code("expected-state").await });

        // Simulate the browser hitting the callback.
        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        client
            .write_all(b"GET /callback?code=abc123&state=expected-state HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("write");
        let mut response = String::new();
        client.read_to_string(&mut response).await.expect("read");
        assert!(response.contains("200 OK"));

        let result = handle.await.expect("join").expect("callback");
        assert_eq!(result.code, "abc123");
        assert_eq!(result.state, "expected-state");
    }

    #[tokio::test]
    async fn loopback_server_rejects_a_state_mismatch() {
        let server = LoopbackServer::bind().await.expect("bind");
        let port = server.port().expect("port");
        let handle = tokio::spawn(async move { server.wait_for_code("expected-state").await });

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        client
            .write_all(b"GET /callback?code=abc123&state=attacker HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("write");

        let error = handle.await.expect("join").expect_err("must reject");
        assert!(error.to_string().contains("state mismatch"));
    }

    #[tokio::test]
    async fn loopback_server_times_out_without_a_callback() {
        let server = LoopbackServer::bind().await.expect("bind");
        let error = server
            .wait_for_code_with_timeout("state", Duration::from_millis(50))
            .await
            .expect_err("must time out");
        assert!(error.to_string().contains("timed out"));
    }
}
