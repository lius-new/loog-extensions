//! Private language-client source, compiled independently into each plugin.
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

use exports::loom::extension::guest::Guest;
#[cfg(not(test))]
use loom::extension::host;
use loom::extension::types::*;
mod client;
mod config;
mod findings;
#[cfg(test)]
mod host;
mod protocol;
mod results;
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
