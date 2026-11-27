//! Velox download engine core: orchestration, segmentation, retry, integrity.
//!
//! Boundary rules: no UI code, no TLS/HTTP library choices (see `ports::HttpPort`),
//! no OS-specific APIs (see engine-storage for the cfg'd shim).

pub mod config;
pub mod error;
pub mod events;
pub mod extension;
pub mod ids;
pub mod inspect;
pub mod manager;
pub mod media;
pub mod naming;
pub mod paths;
pub mod plan;
pub mod ports;
pub mod retry;
pub mod segments;
pub mod speed;
pub mod task;
pub mod url_adapter;

pub use error::{EngineError, ErrorClass, ErrorKind};
pub use ids::DownloadId;
pub use manager::DownloadManager;
pub use ports::{BodyStream, ByteStream, HttpPort};
pub use task::TaskOutcome;
pub use url_adapter::{HttpUrlAdapter, UrlAdapter, UrlKind};
