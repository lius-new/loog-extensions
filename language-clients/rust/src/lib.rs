//! Private language-client source, compiled independently into each plugin.
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

use exports::loog::extension::guest::Guest;
#[cfg(not(test))]
use loog::extension::host;
use loog::extension::types::*;
mod client;
mod config;
mod findings;
#[cfg(test)]
mod host;
mod protocol;
#[cfg(feature = "environment")]
mod provided;
mod results;
mod status;

use std::cell::RefCell;
thread_local! { static CLIENT: RefCell<client::Client> = RefCell::new(client::Client::default()); }
#[cfg(feature = "environment")]
thread_local! { static PROVIDED: RefCell<provided::Client> = RefCell::new(provided::Client::default()); }
struct Extension;
impl Guest for Extension {
    fn dispatch(batch: Vec<Inbound>) -> Result<RoundStatus, HostError> {
        #[cfg(feature = "environment")]
        let provided = PROVIDED.with(|client| client.borrow_mut().dispatch(&batch));
        CLIENT.with(|client| {
            let mut client = client.borrow_mut();
            for message in batch {
                #[cfg(feature = "environment")]
                if matches!(&message, Inbound::ProviderRequest(request) if provided.contains(&request.request)) { continue; }
                if let Err(error) = client.message(message) {
                    host::log(2, &format!("{NAME}: {error}"));
                }
            }
            Ok(RoundStatus::Idle)
        })
    }
}
export!(Extension);
