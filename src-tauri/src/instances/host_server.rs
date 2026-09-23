//! Integrated host path: start a local Minecraft server the launcher owns.
//!
//! Hosting from the UI must never leave the user with "nothing listening on
//! 127.0.0.1:25565". This module:
//!
//! 1. Probes an existing listener when one was requested.
//! 2. Otherwise allocates a free TCP port and starts a dedicated server for the
//!    instance (vanilla server jar, or Fabric/Quilt Meta server jar).
//! 3. Waits until the port accepts connections before returning.
//!
//! The P2P layer still bridges guests to `127.0.0.1:<port>`; the punch socket
//! binds on `0.0.0.0` separately so friends can reach the host.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Child;
use tokio::sync::Mutex as AsyncMutex;

use crate::error::{AppError, AppResult};
use crate::models::instance::{Instance, LoaderKind};
use crate::models::progress::{JobKind, JobStage, ProgressEvent, ProgressSink};
use crate::models::version::VersionJson;
use crate::mods::java_runtime::JavaRuntime;
use crate::mods::{DownloadTask, ModEngine};

/// A dedicated server the launcher started for hosting.
pub struct HostedServer {
    pub address: SocketAddr,
    pub child: Arc<AsyncMutex<Child>>,
    pub log_file: PathBuf,
}

/// Pick a free TCP port on all interfaces, then release it for the JVM to bind.
pub async fn allocate_port() -> AppResult<u16> {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
        .await
        .map_err(|err| AppError::Network(format!("could not allocate a host port: {err}")))?;
    let port = listener
        .local_addr()
        .map_err(|err| AppError::Network(format!("could not read allocated port: {err}")))?
        .port();
    drop(listener);
    Ok(port)
}

/// `true` when something accepts TCP on `addr`.
pub async fn is_listening(addr: SocketAddr) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_millis(350), TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

/// Poll until `addr` accepts connections or `timeout` elapses.
pub async fn wait_for_listener(addr: SocketAddr, timeout: Duration) -> AppResult<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last_err = String::new();
    while tokio::time::Instant::now() < deadline {
        match TcpStream::connect(addr).await {
            Ok(_) => return Ok(()),
            Err(err) => last_err = err.to_string(),
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    Err(AppError::Network(format!(
        "nothing listened on {addr} within {}s ({last_err}). \
         The integrated host could not start the Minecraft server for this instance.",
        timeout.as_secs()
    )))
}

/// Probe for an existing server, otherwise start a dedicated one for the instance.
pub async fn resolve_or_start_host(
    instance: &Instance,
    version: &VersionJson,
    engine: &ModEngine,
    java: &JavaRuntime,
    preferred_port: Option<u16>,
    sink: Arc<dyn ProgressSink>,
) -> AppResult<(SocketAddr, Option<HostedServer>)> {
    let mut candidates: Vec<u16> = Vec::new();
    if let Some(port) = preferred_port {
        if port > 0 {
            candidates.push(port);
        }
    }
    candidates.push(crate::network::bridge::DEFAULT_SERVER_PORT);

    for port in candidates {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        if is_listening(addr).await {
            sink.report(
                ProgressEvent::started(JobKind::P2pHost, "Detected local Minecraft server")
                    .stage(JobStage::Registering)
                    .detail(addr.to_string()),
            )
            .await;
            return Ok((addr, None));
        }
    }

    let hosted =
        start_dedicated_server(instance, version, engine, java, preferred_port, sink).await?;
    Ok((hosted.address, Some(hosted)))
}

async fn start_dedicated_server(
    instance: &Instance,
    version: &VersionJson,
    engine: &ModEngine,
    java: &JavaRuntime,
    preferred_port: Option<u16>,
    sink: Arc<dyn ProgressSink>,
) -> AppResult<HostedServer> {
    let port = match preferred_port {
        Some(port) if port > 0 => port,
        _ => allocate_port().await?,
    };
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));

    sink.report(
        ProgressEvent::started(
            JobKind::P2pHost,
            format!("Starting integrated host on :{port}"),
        )
        .stage(JobStage::Launching),
    )
    .await;

    let server_root = engine
        .paths()
        .instance(instance.config.id)
        .root()
        .join("host-server");
    tokio::fs::create_dir_all(&server_root).await?;

    let jar = provision_server_jar(instance, version, engine, &server_root, sink.clone()).await?;
    write_server_files(&server_root, port, &instance.config.name).await?;
    maybe_link_world(
        &server_root,
        &engine.paths().instance(instance.config.id).root(),
    )
    .await?;

    // Copy instance mods so Fabric/Quilt packs load on the dedicated server.
    let mods_src = engine
        .paths()
        .instance(instance.config.id)
        .root()
        .join("mods");
    let mods_dst = server_root.join("mods");
    if mods_src.is_dir() {
        copy_dir_loose(&mods_src, &mods_dst).await?;
    }

    let log_file = engine
        .paths()
        .log_file(&format!("host-{}", instance.config.id));
    if let Some(parent) = log_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let memory = instance.config.memory.sanitized();
    let mut command = crate::process::command(&java.path);
    command
        .arg(format!("-Xms{}M", memory.min_mb.min(1024)))
        .arg(format!("-Xmx{}M", memory.max_mb))
        .arg("-jar")
        .arg(&jar)
        .arg("nogui")
        .current_dir(&server_root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|err| AppError::Java(format!("could not start the host server: {err}")))?;

    if let Some(stdout) = child.stdout.take() {
        let path = log_file.clone();
        tokio::spawn(async move {
            tee_lines(stdout, path).await;
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let path = log_file.clone();
        tokio::spawn(async move {
            tee_lines(stderr, path).await;
        });
    }

    sink.report(
        ProgressEvent::started(JobKind::P2pHost, "Waiting for Minecraft server")
            .stage(JobStage::Extracting)
            .detail(address.to_string()),
    )
    .await;

    let child = Arc::new(AsyncMutex::new(child));
    let wait = wait_for_listener(address, Duration::from_secs(120));
    tokio::pin!(wait);
    loop {
        tokio::select! {
            result = &mut wait => {
                result?;
                break;
            }
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                let mut guard = child.lock().await;
                if let Ok(Some(status)) = guard.try_wait() {
                    return Err(AppError::Java(format!(
                        "the integrated host exited before opening {address} (status {status}). \
                         Check logs at {}.",
                        log_file.display()
                    )));
                }
            }
        }
    }

    Ok(HostedServer {
        address,
        child,
        log_file,
    })
}

async fn provision_server_jar(
    instance: &Instance,
    version: &VersionJson,
    engine: &ModEngine,
    server_root: &Path,
    sink: Arc<dyn ProgressSink>,
) -> AppResult<PathBuf> {
    match instance.config.loader.kind {
        LoaderKind::Fabric | LoaderKind::Quilt => {
            fabric_like_server_jar(instance, engine, server_root, sink).await
        }
        _ => vanilla_server_jar(version, engine, server_root, sink).await,
    }
}

async fn vanilla_server_jar(
    version: &VersionJson,
    engine: &ModEngine,
    server_root: &Path,
    sink: Arc<dyn ProgressSink>,
) -> AppResult<PathBuf> {
    let artifact = version
        .downloads
        .as_ref()
        .and_then(|downloads| downloads.server.as_ref())
        .ok_or_else(|| {
            AppError::Config(format!(
                "version {} has no server download; pick another game version to host",
                version.id
            ))
        })?;

    let destination = server_root.join("server.jar");
    let mut task = DownloadTask::new(
        format!("{}-server.jar", version.id),
        artifact.url.clone(),
        destination.clone(),
    );
    if let Some(sha1) = artifact.sha1.as_ref() {
        task = task.with_sha1(sha1.clone());
    }
    if let Some(size) = artifact.size {
        task = task.with_size(size);
    }
    let tracker = Arc::new(crate::jobs::JobTracker::start(
        JobKind::P2pHost,
        "Minecraft server jar",
        JobStage::Downloading,
        sink,
    ));
    let fetched = engine.downloader().fetch(task, tracker.clone()).await;
    crate::mods::Downloader::settle(&tracker, fetched).await?;
    Ok(destination)
}

async fn fabric_like_server_jar(
    instance: &Instance,
    engine: &ModEngine,
    server_root: &Path,
    sink: Arc<dyn ProgressSink>,
) -> AppResult<PathBuf> {
    let loader = instance
        .config
        .loader
        .version
        .clone()
        .or_else(|| instance.config.loader.build.clone())
        .ok_or_else(|| {
            AppError::Config(
                "this Fabric/Quilt instance has no loader version recorded; \
                 reinstall the instance before hosting"
                    .into(),
            )
        })?;
    let game = &instance.config.game_version;
    let installer = latest_fabric_installer().await?;
    let kind = match instance.config.loader.kind {
        LoaderKind::Quilt => "quilt",
        _ => "fabric",
    };
    let url = if kind == "quilt" {
        format!(
            "https://meta.quiltmc.org/v3/versions/loader/{game}/{loader}/{installer}/server/jar"
        )
    } else {
        format!(
            "https://meta.fabricmc.net/v2/versions/loader/{game}/{loader}/{installer}/server/jar"
        )
    };

    let destination = server_root.join("fabric-server-launch.jar");
    let task = DownloadTask::new(
        format!("{kind}-server-{game}-{loader}.jar"),
        url,
        destination.clone(),
    )
    .without_cache();
    let tracker = Arc::new(crate::jobs::JobTracker::start(
        JobKind::P2pHost,
        format!("{kind} server jar"),
        JobStage::Downloading,
        sink,
    ));
    let fetched = engine.downloader().fetch(task, tracker.clone()).await;
    crate::mods::Downloader::settle(&tracker, fetched).await?;
    Ok(destination)
}

async fn latest_fabric_installer() -> AppResult<String> {
    #[derive(serde::Deserialize)]
    struct InstallerEntry {
        version: String,
        #[serde(default)]
        stable: bool,
    }

    let http = crate::mods::modrinth::http_client()?;
    let entries: Vec<InstallerEntry> = http
        .get("https://meta.fabricmc.net/v2/versions/installer")
        .send()
        .await
        .map_err(|err| AppError::Network(format!("Fabric installer list failed: {err}")))?
        .error_for_status()
        .map_err(|err| AppError::Network(format!("Fabric installer list HTTP error: {err}")))?
        .json()
        .await
        .map_err(|err| AppError::Network(format!("Fabric installer list JSON error: {err}")))?;

    if let Some(entry) = entries.iter().find(|entry| entry.stable) {
        return Ok(entry.version.clone());
    }
    entries
        .into_iter()
        .next()
        .map(|entry| entry.version)
        .ok_or_else(|| AppError::Network("no Fabric installer version was published".into()))
}

async fn write_server_files(server_root: &Path, port: u16, motd: &str) -> AppResult<()> {
    tokio::fs::write(server_root.join("eula.txt"), "eula=true\n").await?;
    let safe_motd = motd.replace('\n', " ").replace('\r', "");
    let properties = format!(
        "server-port={port}\n\
         online-mode=false\n\
         max-players=16\n\
         motd={safe_motd}\n\
         spawn-protection=0\n\
         view-distance=10\n\
         sync-chunk-writes=true\n\
         enable-status=true\n\
         white-list=false\n"
    );
    tokio::fs::write(server_root.join("server.properties"), properties).await?;
    Ok(())
}

async fn maybe_link_world(server_root: &Path, instance_root: &Path) -> AppResult<()> {
    let saves = instance_root.join("saves");
    let Ok(mut entries) = tokio::fs::read_dir(&saves).await else {
        return Ok(());
    };
    let mut chosen: Option<PathBuf> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("world") {
            chosen = Some(path);
            break;
        }
        if chosen.is_none() {
            chosen = Some(path);
        }
    }
    let Some(world) = chosen else {
        return Ok(());
    };
    let target = server_root.join("world");
    if target.exists() {
        return Ok(());
    }
    copy_dir_loose(&world, &target).await
}

async fn copy_dir_loose(from: &Path, to: &Path) -> AppResult<()> {
    tokio::fs::create_dir_all(to).await?;
    let mut stack = vec![from.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let src = entry.path();
            let rel = src.strip_prefix(from).unwrap_or(&src);
            let dst = to.join(rel);
            if src.is_dir() {
                tokio::fs::create_dir_all(&dst).await?;
                stack.push(src);
            } else if let Some(parent) = dst.parent() {
                tokio::fs::create_dir_all(parent).await?;
                let _ = tokio::fs::copy(&src, &dst).await;
            }
        }
    }
    Ok(())
}

async fn tee_lines<R: tokio::io::AsyncRead + Unpin>(reader: R, path: PathBuf) {
    let mut lines = BufReader::new(reader).lines();
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .ok();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(file) = file.as_mut() {
            use tokio::io::AsyncWriteExt;
            let _ = file.write_all(line.as_bytes()).await;
            let _ = file.write_all(b"\n").await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allocate_port_returns_nonzero() {
        let port = allocate_port().await.expect("port");
        assert!(port > 0);
    }

    #[tokio::test]
    async fn wait_for_listener_succeeds_when_bound() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });
        wait_for_listener(addr, Duration::from_secs(2))
            .await
            .expect("listening");
    }

    #[tokio::test]
    async fn wait_for_listener_times_out_on_closed_port() {
        let port = allocate_port().await.expect("port");
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let err = wait_for_listener(addr, Duration::from_millis(600))
            .await
            .expect_err("should time out");
        let message = err.to_string();
        assert!(
            message.contains("nothing listened") || message.contains(&port.to_string()),
            "{message}"
        );
    }
}
