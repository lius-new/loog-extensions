//! Incremental project-marker discovery. Paths are opaque provider values;
//! walking and joining always happen through a scoped provider operation.
use crate::api::{Answer, Failure, FailureKind, FileKind, Operation, ProviderPath};

#[derive(Clone)]
pub(crate) struct Policy {
    pub markers: Vec<String>,
    pub outermost: bool,
    pub file_fallback: bool,
}
enum Step {
    Parent,
    Child,
    Stat,
}
pub(crate) struct Probe {
    policy: Policy,
    root: ProviderPath,
    directory: Option<ProviderPath>,
    fallback: Option<ProviderPath>,
    marked: Option<ProviderPath>,
    visited: Vec<ProviderPath>,
    marker: usize,
    step: Step,
    operation: Operation,
}
impl Probe {
    pub fn new(root: ProviderPath, file: ProviderPath, policy: Policy) -> Result<Self, String> {
        if policy.markers.len() > 32
            || policy
                .markers
                .iter()
                .any(|m| m.is_empty() || m.len() > 256 || m.contains(['\0', '/', '\\']))
        {
            return Err("Invalid project marker configuration".into());
        }
        Ok(Self {
            policy,
            root,
            directory: None,
            fallback: None,
            marked: None,
            visited: vec![],
            marker: 0,
            step: Step::Parent,
            operation: Operation::ParentPath { path: file },
        })
    }
    pub fn operation(&self) -> Operation {
        self.operation.clone()
    }
    fn selected(&self) -> ProviderPath {
        self.marked
            .clone()
            .or_else(|| {
                self.policy
                    .file_fallback
                    .then(|| self.fallback.clone())
                    .flatten()
            })
            .unwrap_or_else(|| self.root.clone())
    }
    fn parent(&mut self) -> Option<ProviderPath> {
        let directory = self.directory.as_ref().unwrap();
        if directory == &self.root {
            return Some(self.selected());
        }
        self.operation = Operation::ParentPath {
            path: directory.clone(),
        };
        self.step = Step::Parent;
        None
    }
    fn marker(&mut self) -> Option<ProviderPath> {
        let Some(name) = self.policy.markers.get(self.marker) else {
            return self.parent();
        };
        self.operation = Operation::ChildPath {
            parent: self.directory.clone().unwrap(),
            name: name.clone(),
        };
        self.step = Step::Child;
        None
    }
    pub fn receive(
        &mut self,
        result: Result<Answer, Failure>,
    ) -> Result<Option<ProviderPath>, String> {
        match (&self.step, result) {
            (
                Step::Parent,
                Ok(Answer::Parent {
                    path: Some(directory),
                }),
            ) => {
                if self.visited.contains(&directory) || self.visited.len() >= 64 {
                    return Err("Project parent traversal exceeded its boundary".into());
                }
                self.visited.push(directory.clone());
                self.fallback.get_or_insert_with(|| directory.clone());
                self.directory = Some(directory);
                self.marker = 0;
                if self.policy.markers.is_empty() {
                    return Ok(Some(self.selected()));
                }
                Ok(self.marker())
            }
            (Step::Parent, Ok(Answer::Parent { path: None })) => Ok(Some(self.selected())),
            (Step::Child, Ok(Answer::Path { path })) => {
                self.operation = Operation::Stat { path };
                self.step = Step::Stat;
                Ok(None)
            }
            (Step::Stat, Ok(Answer::Info { file })) => {
                if file.kind == FileKind::File {
                    self.marked = self.directory.clone();
                    if !self.policy.outermost {
                        return Ok(Some(self.selected()));
                    }
                    Ok(self.parent())
                } else {
                    self.marker += 1;
                    Ok(self.marker())
                }
            }
            (Step::Stat | Step::Child, Err(error)) if error.kind == FailureKind::NotFound => {
                self.marker += 1;
                Ok(self.marker())
            }
            (_, Err(error)) => Err(error.message),
            _ => Err("Unexpected project discovery reply".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn path(value: &str) -> ProviderPath {
        ProviderPath::new(value).unwrap()
    }
    fn probe(outermost: bool, file_fallback: bool) -> Probe {
        Probe::new(
            path("ROOT"),
            path("a\\b:中文"),
            Policy {
                markers: vec!["Cargo.toml".into(), "rust-project.json".into()],
                outermost,
                file_fallback,
            },
        )
        .unwrap()
    }
    fn parent(probe: &mut Probe, value: &str) -> Option<ProviderPath> {
        assert!(matches!(probe.operation(), Operation::ParentPath { .. }));
        probe
            .receive(Ok(Answer::Parent {
                path: Some(path(value)),
            }))
            .unwrap()
    }
    fn marker(probe: &mut Probe, found: bool) -> Option<ProviderPath> {
        assert!(matches!(probe.operation(), Operation::ChildPath { .. }));
        probe
            .receive(Ok(Answer::Path {
                path: path("opaque-marker"),
            }))
            .unwrap();
        assert!(matches!(probe.operation(), Operation::Stat { .. }));
        probe
            .receive(if found {
                Ok(Answer::Info {
                    file: crate::api::FileInfo {
                        path: path("opaque-marker"),
                        name: "marker".into(),
                        kind: FileKind::File,
                        size: 0,
                        revision: None,
                        readonly: false,
                    },
                })
            } else {
                Err(Failure::new(FailureKind::NotFound, "absent"))
            })
            .unwrap()
    }
    #[test]
    fn nearest_and_outermost_markers_obey_opaque_parent_chain() {
        let mut nearest = probe(false, true);
        assert!(parent(&mut nearest, "NEAR").is_none());
        assert_eq!(marker(&mut nearest, true), Some(path("NEAR")));
        let mut outer = probe(true, true);
        parent(&mut outer, "NEAR");
        assert!(marker(&mut outer, true).is_none());
        parent(&mut outer, "OUTER");
        assert!(marker(&mut outer, true).is_none());
        parent(&mut outer, "ROOT");
        marker(&mut outer, false);
        assert_eq!(marker(&mut outer, false), Some(path("OUTER")));
    }
    #[test]
    fn fallback_root_boundary_and_denied_reads_are_distinct() {
        for (fallback, expected) in [(true, "NEAR"), (false, "ROOT")] {
            let mut probe = probe(true, fallback);
            parent(&mut probe, "NEAR");
            marker(&mut probe, false);
            marker(&mut probe, false);
            parent(&mut probe, "ROOT");
            marker(&mut probe, false);
            assert_eq!(marker(&mut probe, false), Some(path(expected)));
        }
        let mut probe = probe(true, true);
        parent(&mut probe, "NEAR");
        assert!(
            probe
                .receive(Err(Failure::new(FailureKind::PermissionDenied, "denied")))
                .is_err()
        );
    }
    #[test]
    fn cycles_and_unbounded_parent_walks_fail_without_guessing_paths() {
        let mut cyclic = probe(true, false);
        parent(&mut cyclic, "same");
        marker(&mut cyclic, false);
        marker(&mut cyclic, false);
        assert!(
            cyclic
                .receive(Ok(Answer::Parent {
                    path: Some(path("same"))
                }))
                .is_err()
        );
        let mut deep = probe(true, false);
        for i in 0..64 {
            parent(&mut deep, &format!("node{i}"));
            marker(&mut deep, false);
            marker(&mut deep, false);
        }
        assert!(
            deep.receive(Ok(Answer::Parent {
                path: Some(path("node64"))
            }))
            .is_err()
        );
    }
}
