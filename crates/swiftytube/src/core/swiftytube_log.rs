use std::sync::{Arc, RwLock};

/// `SwiftyTubeLog.Level`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Notice,
    Error,
}

pub type Handler = Arc<dyn Fn(&str, Level) + Send + Sync>;

static HANDLER: RwLock<Option<Handler>> = RwLock::new(None);

/// Library logging. Set a handler to capture, else the `log` crate (target "SwiftyTube.InnerTube").
pub struct SwiftyTubeLog;

impl SwiftyTubeLog {
    /// Swift `SwiftyTubeLog.handler = ...`.
    pub fn set_handler(handler: Option<Handler>) {
        *HANDLER.write().unwrap_or_else(|e| e.into_inner()) = handler;
    }

    pub fn debug(message: &str) {
        Self::emit(message, Level::Debug);
    }

    pub fn info(message: &str) {
        Self::emit(message, Level::Info);
    }

    pub fn notice(message: &str) {
        Self::emit(message, Level::Notice);
    }

    pub fn error(message: &str) {
        Self::emit(message, Level::Error);
    }

    fn emit(message: &str, level: Level) {
        let handler = HANDLER.read().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(handler) = handler {
            handler(message, level);
            return;
        }
        const TARGET: &str = "SwiftyTube.InnerTube";
        match level {
            Level::Debug => log::debug!(target: TARGET, "{message}"),
            Level::Info => log::info!(target: TARGET, "{message}"),
            // `log` has no notice level; warn is the closest above info.
            Level::Notice => log::warn!(target: TARGET, "{message}"),
            Level::Error => log::error!(target: TARGET, "{message}"),
        }
    }
}
