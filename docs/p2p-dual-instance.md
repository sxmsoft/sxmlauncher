# Host and join

The check is cross-machine. The tester hosts on their own desktop. The other system joins with the share code. Do not validate this by opening two copies on the tester's PC.

No passwords belong in this file or in git.

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

On the other system (not a second window on the host PC):

1. Select the ready instance that matches the host's game version, then **Join with a code** and paste the code. Spaces, dashes, and letter case do not matter.
2. The launcher opens the local bridge and starts that instance into it. Minecraft 1.20 and newer receive `--quickPlayMultiplayer 127.0.0.1:<port>`. Older versions still receive `--server` and `--port`. The game should leave the title screen and join the world.
3. A bad checksum says the code is corrupted (TR and EN). An unknown or expired code says the host is offline or the invite lapsed. A code whose listing body disappeared says the host stopped refreshing — copy a fresh code.
4. Activity should end on **Connected to …** / the game joining, not a stuck download. After the host's server log prints `Done`, Activity leaves Starting / Downloading.

## Optional same-machine process

This is not the validation path. The app is single-instance, so a second launch on the same Windows user focuses the window that is already open.

To run a second process anyway, start it with `SXMLAUNCHER_ALLOW_MULTI=joiner` (any value other than `0`, `false`, `no`, or `off`). A name also stores that process under `profiles/<name>` so the two processes do not share one database. `SXMLAUNCHER_PROFILE` overrides the folder name. Both processes still need the same remote Redis URL.

A loopback punch with STUN and the relay down is covered by the automated test below.

## What the automated checks cover

- Canonical share-code keys (dashed, undashed, any case) and expiry versus a missing listing.
- A signaling offer published before the peer starts reading is still delivered (inbox, not a bare pub/sub race).
- Redis URLs are classified as shared vs localhost, and passwords are stripped from the label and from driver errors.
- A memory-directory host and guest complete a loopback punch from a sloppy code:

  `cargo test --manifest-path src-tauri/Cargo.toml --lib sloppy_share_code_joins_a_loopback_host`

- Activity hides an older host download once a newer “waiting for players” snapshot exists.

These do not open a real Upstash account and do not replace a two-machine NAT test.
