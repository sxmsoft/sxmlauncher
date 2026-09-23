//! Discord Rich Presence.
//!
//! A single background thread owns the IPC client. If Discord is not running,
//! or no application id is configured, updates are ignored and the launcher
//! keeps working.
//!
//! The large image key is [`LARGE_IMAGE_KEY`]. Upload that art asset in the
//! Discord Developer Portal (Rich Presence → Art Assets). The id comes from
//! `SXML_DISCORD_APPLICATION_ID` or Settings → General, never from a constant.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use discord_rich_presence::activity::{Activity, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use serde::{Deserialize, Serialize};

use crate::config::AppSettings;

/// Art asset key for the large image. Upload a SXMLAUNCHER logo under this name.
pub const LARGE_IMAGE_KEY: &str = "sxmlauncher";
/// Hover text on the large image.
pub const LARGE_IMAGE_TEXT: &str = "SXMLAUNCHER";

/// What the frontend wants Discord to show. Strings are already localized.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceActivity {
    pub details: String,
    pub state: String,
    /// Unix epoch milliseconds. Discord uses this for the elapsed timer.
    #[serde(default)]
    pub start_unix_ms: Option<i64>,
}

/// Whether an application id is configured. Discord may still be closed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceStatus {
    pub enabled: bool,
}

enum Cmd {
    Set {
        application_id: Option<String>,
        activity: PresenceActivity,
    },
    Clear,
    Shutdown {
        ack: Sender<()>,
    },
}

/// Handle to the presence thread. Cheap to clone the commands; the client is not shared.
pub struct Presence {
    tx: Mutex<Option<Sender<Cmd>>>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl Presence {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name("discord-presence".into())
            .spawn(move || worker(rx))
            .ok();
        Self {
            tx: Mutex::new(Some(tx)),
            join: Mutex::new(join),
        }
    }

    pub fn set(&self, application_id: Option<String>, activity: PresenceActivity) {
        self.send(Cmd::Set {
            application_id,
            activity,
        });
    }

    pub fn clear(&self) {
        self.send(Cmd::Clear);
    }

    /// Ask Discord to drop the activity, then close the socket.
    ///
    /// Waits briefly so a normal quit clears the status. If Discord never
    /// answers, the wait ends and the process is free to exit; Discord also
    /// drops presence when the socket closes.
    pub fn shutdown(&self) {
        let tx = self.tx.lock().take();
        if let Some(tx) = tx {
            let (ack_tx, ack_rx) = mpsc::channel();
            if tx.send(Cmd::Shutdown { ack: ack_tx }).is_ok() {
                let _ = ack_rx.recv_timeout(Duration::from_millis(400));
            }
        }
        let handle = self.join.lock().take();
        if let Some(handle) = handle {
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }

    fn send(&self, cmd: Cmd) {
        if let Some(tx) = self.tx.lock().as_ref() {
            let _ = tx.send(cmd);
        }
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Digits-only Discord snowflake. Anything else disables presence.
pub fn normalize_discord_application_id(raw: &str) -> Option<String> {
    let id = raw.trim();
    if (16..=20).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_digit()) {
        Some(id.to_string())
    } else {
        None
    }
}

/// Env wins when it is non-empty. An invalid env value disables presence
/// instead of silently falling back to the saved id.
pub fn resolve_discord_application_id(settings_id: &str, env_id: Option<&str>) -> Option<String> {
    if let Some(from_env) = env_id.map(str::trim).filter(|value| !value.is_empty()) {
        return normalize_discord_application_id(from_env);
    }
    normalize_discord_application_id(settings_id)
}

pub fn application_id_from_settings(settings: &AppSettings) -> Option<String> {
    let from_env = std::env::var("SXML_DISCORD_APPLICATION_ID").ok();
    resolve_discord_application_id(&settings.discord_application_id, from_env.as_deref())
}

fn clip(value: &str, fallback: &str) -> String {
    let trimmed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = if trimmed.chars().count() >= 2 {
        trimmed
    } else {
        fallback.to_string()
    };
    text.chars().take(128).collect()
}

fn worker(rx: Receiver<Cmd>) {
    let mut client: Option<DiscordIpcClient> = None;
    let mut connected_id: Option<String> = None;
    let mut desired_id: Option<String> = None;
    let mut desired: Option<PresenceActivity> = None;
    let mut applied: Option<String> = None;
    let mut warned = false;
    let mut retry_at = Instant::now();

    loop {
        let incoming = if desired.is_some() && applied.is_none() {
            rx.recv_timeout(Duration::from_secs(15))
        } else {
            match rx.recv() {
                Ok(cmd) => Ok(cmd),
                Err(_) => break,
            }
        };

        match incoming {
            Ok(cmd) => match drain(&rx, cmd) {
                Cmd::Shutdown { ack } => {
                    disconnect(&mut client);
                    let _ = ack.send(());
                    break;
                }
                Cmd::Clear => {
                    disconnect(&mut client);
                    connected_id = None;
                    desired_id = None;
                    desired = None;
                    applied = None;
                    warned = false;
                }
                Cmd::Set {
                    application_id,
                    activity,
                } => {
                    desired_id = application_id;
                    desired = Some(activity);
                }
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        let Some(activity) = desired.clone() else {
            continue;
        };
        let Some(application_id) = desired_id.clone() else {
            disconnect(&mut client);
            connected_id = None;
            desired = None;
            applied = Some("disabled".into());
            continue;
        };
        if Instant::now() < retry_at && applied.is_none() && connected_id.is_none() {
            continue;
        }

        let details = clip(&activity.details, LARGE_IMAGE_TEXT);
        let state = clip(&activity.state, LARGE_IMAGE_TEXT);
        let fingerprint = format!(
            "{application_id}|{details}|{state}|{:?}",
            activity.start_unix_ms
        );
        if applied.as_ref() == Some(&fingerprint) && connected_id.as_ref() == Some(&application_id)
        {
            continue;
        }

        if connected_id.as_ref() != Some(&application_id) || client.is_none() {
            disconnect(&mut client);
            connected_id = None;
            let mut next = DiscordIpcClient::new(&application_id);
            match next.connect() {
                Ok(()) => {
                    client = Some(next);
                    connected_id = Some(application_id);
                    warned = false;
                }
                Err(err) => {
                    if !warned {
                        eprintln!(
                            "[presence] Discord rich presence is off ({err}). The launcher keeps running."
                        );
                        warned = true;
                    }
                    retry_at = Instant::now() + Duration::from_secs(15);
                    continue;
                }
            }
        }

        let Some(ipc) = client.as_mut() else {
            continue;
        };
        let mut built = Activity::new().details(&details).state(&state).assets(
            Assets::new()
                .large_image(LARGE_IMAGE_KEY)
                .large_text(LARGE_IMAGE_TEXT),
        );
        if let Some(start) = activity.start_unix_ms {
            if start > 0 {
                built = built.timestamps(Timestamps::new().start(start));
            }
        }
        match ipc.set_activity(built) {
            Ok(()) => {
                applied = Some(fingerprint);
                warned = false;
            }
            Err(err) => {
                if !warned {
                    eprintln!("[presence] could not update Discord ({err}).");
                    warned = true;
                }
                disconnect(&mut client);
                connected_id = None;
                applied = None;
                retry_at = Instant::now() + Duration::from_secs(15);
            }
        }
    }
}

fn drain(rx: &Receiver<Cmd>, mut cmd: Cmd) -> Cmd {
    loop {
        match rx.try_recv() {
            Ok(next) => cmd = merge(cmd, next),
            Err(_) => return cmd,
        }
    }
}

fn merge(current: Cmd, next: Cmd) -> Cmd {
    match next {
        shutdown @ Cmd::Shutdown { .. } => shutdown,
        next => match current {
            shutdown @ Cmd::Shutdown { .. } => shutdown,
            _ => next,
        },
    }
}

fn disconnect(client: &mut Option<DiscordIpcClient>) {
    if let Some(ipc) = client.as_mut() {
        let _ = ipc.clear_activity();
        let _ = ipc.close();
    }
    *client = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_id_accepts_a_snowflake_only() {
        assert_eq!(normalize_discord_application_id(""), None);
        assert_eq!(normalize_discord_application_id("  "), None);
        assert_eq!(normalize_discord_application_id("not-an-id"), None);
        assert_eq!(normalize_discord_application_id("12345"), None);
        assert_eq!(
            normalize_discord_application_id(" 12345678901234567 "),
            Some("12345678901234567".into())
        );
    }

    #[test]
    fn env_overrides_the_saved_id_and_a_bad_env_disables_presence() {
        assert_eq!(
            resolve_discord_application_id("12345678901234567", Some("9876543210987654321")),
            Some("9876543210987654321".into())
        );
        assert_eq!(
            resolve_discord_application_id("12345678901234567", Some("  ")),
            Some("12345678901234567".into())
        );
        assert_eq!(
            resolve_discord_application_id("12345678901234567", Some("nope")),
            None
        );
        assert_eq!(resolve_discord_application_id("", None), None);
    }

    #[test]
    fn large_image_key_matches_the_portal_asset() {
        assert_eq!(LARGE_IMAGE_KEY, "sxmlauncher");
        assert_eq!(LARGE_IMAGE_TEXT, "SXMLAUNCHER");
    }

    #[test]
    fn missing_application_id_does_not_require_discord() {
        let presence = Presence::spawn();
        presence.set(
            None,
            PresenceActivity {
                details: "Browsing home".into(),
                state: "SXMLAUNCHER".into(),
                start_unix_ms: None,
            },
        );
        presence.clear();
        presence.shutdown();
    }

    #[test]
    fn status_text_is_clamped() {
        let long = "x".repeat(400);
        let clipped = clip(&long, "SXMLAUNCHER");
        assert_eq!(clipped.chars().count(), 128);
        assert_eq!(clip(" ", "SXMLAUNCHER"), "SXMLAUNCHER");
        assert_eq!(clip("  Fabric   1.21.1  ", "SXMLAUNCHER"), "Fabric 1.21.1");
    }
}
