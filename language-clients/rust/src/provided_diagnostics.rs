//! Bounded pull diagnostics bind replies to the requested buffer revision.
use super::*;
use api::session::DocumentVersion;

#[derive(Default)]
pub(super) struct Pulls {
    queued: VecDeque<DocumentVersion>,
    pending: Option<(u64, DocumentVersion, u64)>,
}
impl Pulls {
    pub fn queue(&mut self, stamp: DocumentVersion) {
        if let Some(old) = self
            .queued
            .iter_mut()
            .find(|s| s.document == stamp.document)
        {
            *old = stamp;
        } else if self.queued.len() < 128 {
            self.queued.push_back(stamp);
        }
    }
    pub fn close(&mut self, document: u64) {
        self.queued.retain(|s| s.document != document);
    }
    pub fn next(&mut self, counter: &mut u64, now: u64) -> Option<(u64, DocumentVersion)> {
        if self.pending.is_some() {
            return None;
        }
        let stamp = self.queued.pop_front()?;
        *counter += 1;
        self.pending = Some((*counter, stamp, now.saturating_add(30_000)));
        Some((*counter, stamp))
    }
    pub fn finish(&mut self, id: u64) -> Option<DocumentVersion> {
        if self.pending.is_some_and(|(request, _, _)| request == id) {
            self.pending.take().map(|(_, stamp, _)| stamp)
        } else {
            None
        }
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn expired(&self, now: u64) -> bool {
        self.pending.is_some_and(|(_, _, deadline)| now >= deadline)
    }
}

pub(super) fn findings(doc: &mut Document, items: &Value) -> Vec<ResourceDiagnostic> {
    let mut retained = 0usize;
    doc.diagnostics = array(items)
        .iter()
        .take(64)
        .filter(|value| {
            retained += value.to_string().len();
            retained <= 16 * 1024
        })
        .cloned()
        .collect();
    let mut bytes = 0;
    array(items)
        .iter()
        .take(2048)
        .filter_map(|value| {
            let range = doc.text.range(&value["range"]).ok()?;
            let message = value["message"].as_str()?.to_owned();
            let source = value["source"]
                .as_str()
                .filter(|s| s.len() <= 1024)
                .map(str::to_owned);
            let code = value["code"]
                .as_str()
                .filter(|s| s.len() <= 1024)
                .map(str::to_owned);
            let size = message.len()
                + source.as_ref().map_or(0, String::len)
                + code.as_ref().map_or(0, String::len);
            if message.len() > 16 * 1024 || bytes + size > 384 * 1024 {
                return None;
            }
            bytes += size;
            Some(ResourceDiagnostic {
                start: range.start,
                end: range.end,
                severity: value["severity"]
                    .as_u64()
                    .filter(|v| (1..=4).contains(v))
                    .unwrap_or(1) as u8,
                message,
                source,
                code,
                documentation_url: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pulls_coalesce_edits_without_rebinding_late_responses_or_starving_other_documents() {
        let first = DocumentVersion {
            document: 1,
            lifecycle: 2,
            version: 3,
        };
        let second = DocumentVersion {
            document: 2,
            ..first
        };
        let updated = DocumentVersion {
            version: 4,
            ..first
        };
        let mut pulls = Pulls::default();
        let mut counter = 2;
        pulls.queue(first);
        let (request, stamp) = pulls.next(&mut counter, 10).unwrap();
        assert_eq!(stamp, first);
        pulls.queue(second);
        pulls.queue(first);
        pulls.queue(updated);
        assert!(pulls.next(&mut counter, 11).is_none());
        assert!(pulls.finish(request + 1).is_none());
        assert!(!pulls.expired(30_009));
        assert!(pulls.expired(30_010));
        assert_eq!(pulls.finish(request), Some(first));
        assert_eq!(pulls.next(&mut counter, 12).unwrap().1, second);
        assert_eq!(pulls.finish(counter), Some(second));
        assert_eq!(pulls.next(&mut counter, 13).unwrap().1, updated);
        pulls.queue(second);
        pulls.close(2);
        pulls.finish(counter);
        assert!(pulls.next(&mut counter, 14).is_none());
        for document in 0..256 {
            pulls.queue(DocumentVersion { document, ..first });
        }
        assert_eq!(pulls.queued.len(), 128);
    }
}
