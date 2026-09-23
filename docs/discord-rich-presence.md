# Discord Rich Presence

SXMLAUNCHER updates a Discord activity from the desktop app. Nothing is sent when Discord is closed, and nothing is sent until an application id is set.

## Application id

Create an application in the [Discord Developer Portal](https://discord.com/developers/applications). Copy its **Application ID** (digits only).

Set it in either place:

| Source | Name |
|--------|------|
| Environment variable | `SXML_DISCORD_APPLICATION_ID` |
| Settings → General | Discord Rich Presence → Application ID |

The environment variable wins when it is non-empty. A non-numeric value turns presence off. Do not commit a real id if you treat the portal application as private to your install; the id itself is not an OAuth secret.

## Art asset to upload

In the portal: **Rich Presence → Art Assets**.

| Key | Use | Suggested file |
|-----|-----|----------------|
| `sxmlauncher` | Large image on every activity | SXMLAUNCHER logo, 512×512 or 1024×1024 PNG |

The launcher sends hover text `SXMLAUNCHER` for that image. No other asset keys are required.

Discord can take a short while to publish a new asset. Until the key exists, presence still works and the image slot stays empty.

## What the status says

Text follows the launcher language (Turkish or English). It is a plain status, not a slogan.

| Moment | English | Türkçe |
|--------|---------|--------|
| Home | Browsing home | Ana menüde geziniyor |
| Library or an instance page | Browsing the library | Kütüphaneye bakıyor |
| Modrinth / CurseForge / custom packs | Browsing modpacks | Modpaketlerine bakıyor |
| Profile | Viewing profile | Profil ve hesaba bakıyor |
| Activity, nothing transferring | Viewing activity | Etkinliğe bakıyor |
| Download or install | Download in progress | İndirme durumu |
| Host page, or a host session before the game is up | Preparing multiplayer / Preparing a P2P host | Çok oyunculu hazırlığı / P2P sunucu hazırlıyor |
| Join in progress | Preparing to join | Sunucuya katılmaya hazırlanıyor |
| Game process running | Instance name, then loader and Minecraft version | Same facts, with Sunucu or Çok oyunculu when hosting or joining |

When the game process exits, presence returns to the current page. When the launcher closes, the activity is cleared.
