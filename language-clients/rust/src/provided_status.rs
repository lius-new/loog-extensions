//! Coalesce resource service reports until the host acknowledges publication.
use loog_language_workspace::api::session::{Command, Event, Reply, ResourceServiceReport};

#[derive(Default)]
pub(super) struct Reports {
    last: Vec<ResourceServiceReport>,
    next: Option<Vec<ResourceServiceReport>>,
    pending: Option<(u64, Vec<ResourceServiceReport>)>,
}
impl Reports {
    pub fn update(&mut self, mut reports: Vec<ResourceServiceReport>) {
        // Metadata is bounded independently of document text. Keep whole
        // identities; never shorten a path or make a scoped report global.
        let mut bytes = 0usize;
        for report in &mut reports {
            if let Some(contexts) = &mut report.contexts {
                contexts.truncate(128);
                contexts.retain(|resource| {
                    let size = serde_json::to_vec(resource).map_or(usize::MAX, |v| v.len());
                    if bytes.saturating_add(size) > 192 * 1024 {
                        return false;
                    }
                    bytes += size;
                    true
                });
            }
        }
        self.next = Some(reports);
    }
    pub fn receive(&mut self, event: &Event) -> bool {
        let Event::Reply { ticket, result } = event else {
            return false;
        };
        if self
            .pending
            .as_ref()
            .is_none_or(|(expected, _)| expected != ticket)
        {
            return false;
        }
        // A permanent refusal must not spin. A changed status may try again.
        self.last = self.pending.take().unwrap().1;
        if !matches!(result, Ok(Reply::Done)) {
            crate::host::log(2, "Resource service report was rejected");
        }
        true
    }
    pub fn needs_drive(&self) -> bool {
        self.pending.is_none() && self.next.as_ref().is_some_and(|next| next != &self.last)
    }
    pub fn drive(&mut self, submit: &mut impl FnMut(Command) -> Result<u64, String>) {
        if !self.needs_drive() {
            return;
        }
        let reports = self.next.as_ref().unwrap();
        if let Ok(ticket) = submit(Command::ReportServices {
            reports: reports.clone(),
        }) {
            self.pending = Some((ticket, self.next.take().unwrap()));
        }
    }
}

pub(super) fn detail(message: &str) -> String {
    let mut text = String::new();
    for ch in message.chars() {
        let ch = if ch.is_control() { ' ' } else { ch };
        if text.len() + ch.len_utf8() > 200 {
            break;
        }
        text.push(ch);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use loog_language_workspace::api::{EnvironmentKey, ResourceRef, session::ServiceState};
    #[test]
    fn status_updates_coalesce_across_receipts_and_keep_resource_identity() {
        let resource = |environment| {
            ResourceRef::provided(
                EnvironmentKey::new("provider", environment).unwrap(),
                "same\\中",
            )
            .unwrap()
        };
        let report = |environment| ResourceServiceReport {
            key: "session".into(),
            state: ServiceState::Ready,
            detail: Some("Ready".into()),
            contexts: Some(vec![resource(environment)]),
        };
        let mut reports = Reports::default();
        reports.update(vec![report("one")]);
        reports.drive(&mut |_| Err("busy".into()));
        assert!(reports.needs_drive());
        reports.drive(&mut |_| Ok(1));
        reports.update(vec![report("two")]);
        reports.drive(&mut |_| panic!("one pending publication"));
        assert!(reports.receive(&Event::Reply {
            ticket: 1,
            result: Ok(Reply::Done)
        }));
        reports.drive(&mut |command| {
            let Command::ReportServices { reports } = command else {
                panic!("reports")
            };
            assert_eq!(reports[0].contexts, Some(vec![resource("two")]));
            Ok(2)
        });
        reports.receive(&Event::Reply {
            ticket: 2,
            result: Ok(Reply::Done),
        });
        reports.update(vec![report("two")]);
        assert!(!reports.needs_drive());
        reports.update(vec![]);
        assert!(reports.needs_drive());
        assert!(detail(&"中\n".repeat(100)).len() <= 200);
        assert!(!detail("a\nb\0").contains(['\n', '\0']));
    }
}
