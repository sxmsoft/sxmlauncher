# Dual-instance host and join

Use this when two launchers need to share a world. No passwords belong in this file or in git.

## Same directory on both sides

Share codes live in the directory, not inside the code alone. Both launchers must use the **same** backend:

1. Settings → Network → **Redis URL**. Paste the shared `rediss://` URL (Upstash or another host that is not `localhost` / `127.0.0.1`).
2. Leave the password only in that field. The status tooltip shows the host with the user and password removed.
3. A remote Redis URL replaces the public MQTT broker. The localhost default does not: that path still uses the broker, and a code published there will not resolve against Upstash.
4. Directory must be enabled. If it is off, or the URL does not connect, hosting fails with a directory error instead of minting a code the other machine can never see.

Confirm the header directory tooltip matches on both machines (same host, no secret).

## Host

1. Open an instance and turn on **Host World to Friends**.
2. Activity should move through starting the local server, then settle on **Waiting for players** / **Oyuncular bekleniyor**. A finished server-jar download must not leave the strip on Downloading / İndiriliyor.
3. Copy the full `SXM1-…` code. It stays valid for the whole session: the host refreshes it about every 10 seconds. After the host stops, it expires (about five minutes).

If STUN cannot see a public address, the code is still issued. Direct punch is attempted, then the relay when one is configured. Activity names that failure instead of pretending the download is still running.

## Join

1. On the second launcher, **Join with a code** and paste the code. Spaces, dashes, and letter case do not matter.
2. A bad checksum says the code is corrupted (TR and EN). An unknown or expired code says the host is offline or the invite lapsed. A code whose listing body disappeared says the host stopped refreshing — copy a fresh code.
3. Activity should end on **Connected to …**, not a stuck download. The game connects to `127.0.0.1` on the bridge port from the toast.

## Same PC

The launcher is single-instance, so a second copy on the same Windows or Linux user focuses the window that is already open. Use two machines, or a second OS user, and put the same remote Redis URL in both Settings screens.

A loopback punch with STUN and the relay down is covered by the automated test below. That path does not need a second desktop process.

## What the automated checks cover

- Canonical share-code keys (dashed, undashed, any case) and expiry versus a missing listing.
- A signaling offer published before the peer starts reading is still delivered (inbox, not a bare pub/sub race).
- Redis URLs are classified as shared vs localhost, and passwords are stripped from the label and from driver errors.
- A memory-directory host and guest complete a loopback punch from a sloppy code:

  `cargo test --manifest-path src-tauri/Cargo.toml --lib sloppy_share_code_joins_a_loopback_host`

- Activity hides an older host download once a newer “waiting for players” snapshot exists.

These do not open a real Upstash account and do not replace a two-machine NAT test.
