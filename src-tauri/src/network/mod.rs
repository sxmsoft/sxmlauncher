//! P2P networking: server browser, NAT traversal, tunneling and session control.
//!
//! Two independent paths, deliberately:
//!
//! | path          | needs a server? | works             |
//! |---------------|-----------------|-------------------|
//! | `localnet`    | no              | LAN / same Wi-Fi  |
//! | directory+P2P | yes (Redis)     | internet friends  |
//!
//! The LAN path is always available, so the launcher is useful with zero setup.
//!
//! ```text
//!   LanManager  ── UDP broadcast beacons (no infrastructure)
//!
//!   SessionManager ──┬── RedisDirectory      (browse / signaling / share codes)
//!                    ├── TransportRegistry   (direct UDP then relay TCP)
//!                    │      ├── DirectTransport -> holepunch (STUN + punch)
//!                    │      │                        └── UdpTunnel (framing)
//!                    │      └── RelayTransport (framed handshake + byte pipe)
//!                    └── bridge              (Minecraft TCP <-> tunnel)
//! ```

pub mod bridge;
pub mod code;
pub mod directory;
pub mod holepunch;
pub mod icon;
pub mod localnet;
pub mod mqtt;
pub mod relay;
pub mod session;
pub mod transport;

pub use bridge::{BridgeHandle, BridgeStats, HandshakeInfo};
pub use code::{ConnectCode, CodeFlags};
pub use directory::{Directory, MemoryDirectory, RedisDirectory, CODE_TTL_SECS, KEY_PREFIX};
pub use holepunch::{NatBehavior, PunchConfig, PublicMapping};
pub use localnet::{LanBeacon, LanDiscovery, LanHost, LanManager, LanWorld};
pub use relay::{RelayHello, RelayRole, RelayTransport};
pub use session::{
    GuestConnection, GuestSession, HostOptions, HostSession, JoinTarget, SessionEvent,
    SessionManager,
};
pub use transport::{
    ConnectedPeer, DirectTransport, DuplexStream, Frame, FrameKind, Transport, TransportRegistry,
    UdpTunnel,
};
