//! SwiftyTube files the Trop parsers need before their own milestone ports
//! them into `crates/swiftytube` (`Sync/` in R5, `Streaming/` in R3). Same
//! paths as in SwiftyTube, so moving them is a `git mv` plus import changes.

pub mod streaming;
pub mod sync;
