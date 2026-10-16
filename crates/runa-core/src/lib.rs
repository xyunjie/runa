//! runa-core: the shared engine behind the `runa` CLI and the desktop GUI.
//!
//! Responsibilities:
//! - resolve which version of a tool applies to a directory (`resolver`)
//! - download, verify and unpack runtimes into isolated directories (`installer`)
//! - activate versions through shell hooks and shims (`activate`)
//! - detect and adopt runtimes that already exist on the machine (`detect`)

pub mod activate;
pub mod archive;
pub mod detect;
pub mod installer;
pub mod manager;
pub mod net;
pub mod paths;
pub mod platform;
pub mod plugins;
pub mod resolver;
pub mod settings;
pub mod toolversions;
pub mod version;

pub use manager::{EnvPlan, Manager, Resolved, ToolStatus};
pub use paths::Paths;
pub use platform::Target;
pub use resolver::{Source, Spec};
pub use settings::Settings;

/// Product name used for directories, env vars and markers.
pub const APP_NAME: &str = "runa";
