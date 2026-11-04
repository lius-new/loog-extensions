//! Native UI fixture using the production Rust language client.
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

use exports::loom::extension::guest::Guest;

use loom::extension::types::*;
#[path = "../../../../rust/src/client.rs"]
mod client;
#[path = "../../../../rust/src/config.rs"]
mod config;
mod host {
    pub use crate::loom::extension::host::*;
    use crate::loom::extension::types::{HostError, ProcessRequest};

    // Test-only launch options exercise delayed and failed fake-server replies.
    pub fn process_spawn(request: &ProcessRequest) -> Result<u64, HostError> {
        let mut request = request.clone();
        let setting = get_setting("example.language-fixture.options").unwrap_or_default();
        let mut options: serde_json::Value = serde_json::from_str(&setting).unwrap_or_default();
        if let Some(text) = options.as_str() {
            options = serde_json::from_str(text).unwrap_or_default();
        }
        request.args = options["args"]
            .as_array()
            .map(|args| {
                args.iter()
                    .filter_map(|arg| arg.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        crate::loom::extension::host::process_spawn(&request)
    }
}
#[path = "../../../../rust/src/protocol.rs"]
mod protocol;
#[path = "../../../../rust/src/results.rs"]
mod results;
#[path = "../../../../rust/src/status.rs"]
mod status;

use std::cell::RefCell;
thread_local! { static CLIENT: RefCell<client::Client> = RefCell::new(client::Client::default()); }
struct Extension;
impl Guest for Extension {
    fn dispatch(batch: Vec<Inbound>) -> Result<RoundStatus, HostError> {
        CLIENT.with(|client| {
            let mut client = client.borrow_mut();
            for message in batch {
                if let Err(error) = client.message(message) {
                    host::log(2, &format!("{NAME}: {error}"));
                }
            }
            Ok(RoundStatus::Idle)
        })
    }
}
export!(Extension);
