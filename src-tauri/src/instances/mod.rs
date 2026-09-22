//! Instance management: isolation, installation and launching.

pub mod installer;
pub mod launch;
pub mod loader;
pub mod manager;

pub use installer::{
    asset_tasks, build_install_plan, InstallFile, InstallPlan, Installer, MojangClient,
};
pub use launch::{LaunchExtras, LaunchPlan, LaunchPlanner, RunningGame};
pub use loader::install_loader;
pub use manager::InstanceManager;
