pub mod app;
pub mod cleanup;
pub mod db;
pub mod duplicates;
pub mod export;
pub mod format;
pub mod insights;
pub mod platform;
pub mod scanner;
pub mod snapshot;
pub mod tree;
pub mod treemap;
pub mod watcher;

pub use app::{configure_theme, DiskMapApp};
