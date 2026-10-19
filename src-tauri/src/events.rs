//! Sending events to the interface without depending on Tauri.
//!
//! The controller-management modules only need to "notify the interface": they receive an
//! `Events` value instead of an `AppHandle`. Only `lib.rs` knows about Tauri, which keeps the
//! logic testable without loading the graphical engine.

use serde::Serialize;
use std::sync::Arc;

type Sink = dyn Fn(&str, serde_json::Value) + Send + Sync;

#[derive(Clone)]
pub struct Events(Arc<Sink>);

impl Events {
    pub fn new(sink: impl Fn(&str, serde_json::Value) + Send + Sync + 'static) -> Self {
        Self(Arc::new(sink))
    }

    /// A receiver that ignores everything.
    #[cfg(test)]
    pub fn none() -> Self {
        Self::new(|_, _| {})
    }

    pub fn emit(&self, name: &str, payload: &impl Serialize) {
        if let Ok(value) = serde_json::to_value(payload) {
            (self.0)(name, value);
        }
    }
}
