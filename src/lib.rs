mod tls;
mod proxy;
mod gui;
mod store;
mod util;

pub use util::*;

pub const ORDERING: std::sync::atomic::Ordering = std::sync::atomic::Ordering::Relaxed;