//! The app's logic: what the UI shows, and what it can do.
//!
//! Plain Rust on purpose — no FFI, no HarmonyOS, no file descriptor it did not
//! open itself. It builds and tests on a developer's machine, and the same code
//! runs in the app behind `rust/bridge`. That split is the whole point of this
//! repository: if a piece of logic cannot be tested without a phone, it is in
//! the wrong place.

pub mod file;
pub mod geo;
pub mod image;
pub mod level;
pub mod rows;
pub mod todos;
