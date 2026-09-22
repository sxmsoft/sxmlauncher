//! Domain models shared by every backend module and mirrored 1:1 by the
//! TypeScript interfaces in `src/types`.
//!
//! Serialization convention: `#[serde(rename_all = "camelCase")]` everywhere so
//! the IPC payloads match the frontend `camelCase` shape without adapters.

pub mod account;
pub mod instance;
pub mod modpack;
pub mod progress;
pub mod server;
pub mod version;

pub use account::{
    AccountProvider, AccountSummary, LaunchIdentity, MinecraftUuid, SkinModel, SkinProfile,
    TokenSet, UserAccount, UserType,
};
pub use instance::{
    CreateInstanceRequest, Instance, InstanceConfig, InstancePaths, InstanceStatus, JavaSettings,
    LoaderKind, MemorySettings, ModLoader, ResolutionSettings, UpdateInstanceRequest,
};
pub use modpack::{
    CurseForgeFile, CurseForgeMod, ModDependency, ModHashes, ModProject, ModSearchHit,
    ModSearchQuery, ModSearchResults, ModSource, ModVersion, MrpackManifest, PackDependencyKind,
    ResolvedMod, ResolvedPackPlan,
};
pub use progress::{JobKind, JobStage, ProgressEvent, ProgressSink};
pub use server::{
    ConnectionDescriptor, ConnectionMode, EndpointKind, ModpackRef, PeerEndpoint, PlayerCount,
    RelayDescriptor, ServerFilter, ServerHeartbeat, ServerListing, ServerListingSummary,
    ServerOwner, WhitelistPolicy,
};
pub use version::{
    Arguments, AssetIndexRef, DownloadArtifact, FeatureSet, JavaVersionRef, Library, Rule,
    VersionJson,
};
