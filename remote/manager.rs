//! Bounded independent SSH connections. Host tickets, scopes and process IDs
//! route to their owner; Agent handles are meaningful only inside that owner.
use super::*;

const MAX_CONNECTIONS: usize = 4;

pub(super) struct Connections {
    clients: Vec<Client>,
    selected: usize,
}
impl Default for Connections {
    fn default() -> Self {
        Self { clients: vec![Client::default()], selected: 0 }
    }
}
impl Connections {
    fn candidate(&mut self) -> Result<usize, String> {
        if let Some(index) = self.clients.iter().position(|client| {
            client.process.is_none() && client.environment.is_none()
                && client.tickets.is_empty() && client.pending.is_empty()
        }) { return Ok(index); }
        if self.clients.len() >= MAX_CONNECTIONS {
            return Err("At most four SSH environments can be connected; disconnect one first".into());
        }
        let index = self.clients.len();
        self.clients.push(Client { index, ..Client::default() });
        Ok(index)
    }
    fn show(&mut self, index: usize) -> Result<(), String> {
        for (other, client) in self.clients.iter_mut().enumerate() {
            if other != index { client.hide()?; }
        }
        self.selected = index;
        self.clients[index].show()
    }
    fn notice(&mut self, message: &str) {
        let _ = self.clients[self.selected].label(message);
        host::log(2, message);
    }
    fn message_owner(&self, message: &Inbound) -> Option<usize> {
        self.clients.iter().position(|client| match message {
            Inbound::TimerElapsed(timer) => client.heartbeat == Some(*timer),
            Inbound::ProcessOutput(output) => client.process == Some(output.process),
            Inbound::ProcessExited(exit) => client.process == Some(exit.process),
            Inbound::UiEvent(event) => client.controls.contains_key(&event.node),
            _ => false,
        })
    }
    pub(super) fn message(&mut self, message: Inbound) {
        if let Inbound::RunCommand(command) = &message {
            if command.command == "loog.remote.ssh.connect" {
                let result = self.candidate().and_then(|index| self.show(index));
                if let Err(error) = result { self.notice(&error); }
                return;
            }
            if command.command == "loog.remote.ssh.disconnect" {
                if let Err(error) = self.clients[self.selected].disconnect(true) { self.notice(&error); }
                return;
            }
        }
        let Some(index) = self.message_owner(&message) else { return };
        if let Inbound::UiEvent(event) = &message {
            let action = self.clients[index].controls.get(&event.node).copied();
            if matches!(event.payload, UiEventPayload::Click) {
                if action == Some("show") {
                    if let Err(error) = self.show(index) { self.notice(&error); }
                    return;
                }
                if action == Some("connect") {
                    if self.clients[index].process.is_some() || self.clients[index].environment.is_some() {
                        self.notice("This connection is active; use Remote SSH: Connect for another environment");
                        return;
                    }
                    let identity = self.clients[index].profile.identity();
                    if self.clients.iter().any(|client| client.connected.as_ref()
                        .is_some_and(|profile| profile.identity() == identity)) {
                        self.notice("This SSH environment is already connected; select its status entry");
                        return;
                    }
                }
            }
        }
        if let Err(error) = self.clients[index].message(message) { self.clients[index].fail(&error); }
    }
    fn event_owner(&self, event: &Event) -> Option<usize> {
        self.clients.iter().position(|client| match event {
            Event::Reply { ticket, .. } => client.tickets.contains_key(ticket),
            Event::Delivery { event: api::Delivery::Request(request) } => client.scope == Some(request.scope),
            Event::Delivery { event: api::Delivery::Release { scope, .. } } => client.scope == Some(*scope),
            Event::Delivery { event: api::Delivery::Cancel { request } } => client.pending.values().any(|pending|
                matches!(pending, Pending::Operation(operation) if operation.id == *request)),
            _ => false,
        })
    }
    pub(super) fn workspace(&mut self, event: Event) {
        if let Event::Restore { bookmark } = &event {
            let result = (|| {
                let profile: Profile = serde_json::from_str(&bookmark.state).map_err(|_| "Invalid saved connection profile".to_owned())?;
                profile.validate()?;
                let expected = api::ResourceRef::provided(api::EnvironmentKey::new(ID, profile.identity())?, profile.root.clone())?;
                if expected != bookmark.resource { return Err("Saved connection identity does not match its profile".into()); }
                if let Some(client) = self.clients.iter().find(|client| client.connected.as_ref()
                    .is_some_and(|active| active.identity() == profile.identity())) {
                    if client.connected.as_ref().unwrap().root != profile.root && client.canonical_root != profile.root {
                        return Err("This SSH environment already has another folder connected".into());
                    }
                    return Ok(());
                }
                let index = self.candidate()?;
                self.clients[index].profile = profile;
                self.show(index)?;
                if let Err(error) = self.clients[index].connect() {
                    self.clients[index].fail(&error);
                    return Err(error);
                }
                Ok(())
            })();
            if let Err(error) = result { self.notice(&error); }
            return;
        }
        let Some(index) = self.event_owner(&event) else { return };
        if let Err(error) = self.clients[index].workspace(event) { self.clients[index].fail(&error); }
    }
    pub(super) fn flush(&mut self) {
        for client in &mut self.clients {
            if let Err(error) = client.flush() { client.fail(&error); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_scopes_tickets_timers_and_cancellation_have_one_owner() {
        let mut connections = Connections::default();
        for index in 0..MAX_CONNECTIONS {
            assert_eq!(connections.candidate().unwrap(), index);
            let client = &mut connections.clients[index];
            client.process = Some(index as u64 + 10);
            client.scope = Some(index as u64 + 20);
            client.heartbeat = Some(index as u64 + 30);
            client.tickets.insert(index as u64 + 40, Ticket::Other);
            client.pending.insert(1, Pending::Operation(api::Request {
                id: index as u64 + 50, environment: index as u64 + 60,
                scope: index as u64 + 20, root: api::ProviderPath::new("/same").unwrap(),
                excluded_roots: vec![], operation: api::Operation::Stat { path: api::ProviderPath::new("/same/file").unwrap() },
            }));
        }
        assert!(connections.candidate().is_err());
        for index in 0..MAX_CONNECTIONS {
            assert_eq!(connections.message_owner(&Inbound::TimerElapsed(index as u64 + 30)), Some(index));
            assert_eq!(connections.event_owner(&Event::Reply { ticket: index as u64 + 40, result: Ok(Reply::Done) }), Some(index));
            assert_eq!(connections.event_owner(&Event::Delivery { event: api::Delivery::Cancel { request: index as u64 + 50 } }), Some(index));
            assert_eq!(connections.event_owner(&Event::Delivery { event: api::Delivery::Release {
                scope: index as u64 + 20, capability: api::Capability::Process, handle: 1,
            } }), Some(index));
        }
        assert_eq!(connections.message_owner(&Inbound::TimerElapsed(999)), None);
        connections.clients[2].process = None;
        assert!(connections.candidate().is_err(), "pending receipts prevent reusing an old connection slot");
        connections.clients[2].tickets.clear();
        connections.clients[2].pending.clear();
        assert_eq!(connections.candidate().unwrap(), 2);
    }
}
