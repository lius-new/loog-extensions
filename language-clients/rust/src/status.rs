#[cfg(test)]
use crate::host::{motion, shared, tree};
#[cfg(not(test))]
use crate::loom::ui::{motion, shared, tree};
use crate::{host, loom::extension::types::*};
use std::collections::BTreeMap;

pub const SHARED_KEY: &str = "loom.language-services.status.v1";

// Material Design Icons (Pictogrammers), Apache-2.0. Paths stay plugin-owned;
// Loom only interprets the generic filled-path syntax.
pub const LOADING_ICON: &str = "svg-path:24x24:M12,4V2A10,10 0 0,0 2,12H4A8,8 0 0,1 12,4Z";
pub const READY_ICON: &str = "svg-path:24x24:M12 2C6.5 2 2 6.5 2 12S6.5 22 12 22 22 17.5 22 12 17.5 2 12 2M10 17L5 12L6.41 10.59L10 14.17L17.59 6.58L19 8L10 17Z";
pub const ERROR_ICON: &str = "svg-path:24x24:M13,13H11V7H13M13,17H11V15H13M12,2A10,10 0 0,0 2,12A10,10 0 0,0 12,22A10,10 0 0,0 22,12A10,10 0 0,0 12,2Z";
pub const STOPPED_ICON: &str = "svg-path:24x24:M12,2A10,10 0 0,0 2,12A10,10 0 0,0 12,22A10,10 0 0,0 22,12A10,10 0 0,0 12,2M12,4C16.41,4 20,7.59 20,12C20,16.41 16.41,20 12,20C7.59,20 4,16.41 4,12C4,7.59 7.59,4 12,4M9,9V15H15V9";

#[derive(Default)]
pub struct Widgets {
    entries: BTreeMap<String, Widget>,
    next: u64,
}
impl Widgets {
    pub fn update(&mut self, states: BTreeMap<String, (Status, Vec<String>)>) {
        self.entries.retain(|key, widget| {
            if states.contains_key(key) {
                true
            } else {
                widget.dispose();
                false
            }
        });
        for (key, (status, contexts)) in states {
            let widget = self.entries.entry(key).or_insert_with(|| {
                self.next += 1;
                Widget {
                    prefix: format!("language-status-{}", self.next),
                    ..Default::default()
                }
            });
            widget.update(&status, &contexts);
        }
    }
    pub fn dispose(&mut self) {
        for widget in self.entries.values_mut() {
            widget.dispose();
        }
        self.entries.clear();
    }
}

#[derive(Clone, Default)]
pub struct Status {
    pub rank: u8,
    pub detail: String,
}

#[derive(Default)]
pub struct Widget {
    prefix: String,
    root: Option<u64>,
    text: Option<u64>,
    icon: Option<u64>,
    animated_rank: Option<u8>,
    text_value: Option<String>,
    published: Option<(u8, Vec<String>)>,
}

fn compact(value: &str, limit: usize) -> String {
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > limit {
        text.chars()
            .take(limit - 1)
            .chain(std::iter::once('…'))
            .collect()
    } else {
        text
    }
}

pub fn description(id: &str, kind: UiKind) -> UiDescription {
    UiDescription {
        id: id.into(),
        kind,
        text: None,
        icon: None,
        foreground: None,
        background: None,
        width: None,
        height: Some(24.0),
        padding: 0.0,
        gap: 0.0,
        visible: true,
        label: None,
    }
}

fn animation(rank: u8) -> Option<motion::Animation> {
    use motion::{Animation, Direction, Easing, Keyframe, Property, Track};
    let track = |property, values: &[(f32, f32)]| Track {
        property,
        keyframes: values
            .iter()
            .map(|&(offset, value)| Keyframe { offset, value })
            .collect(),
        easing: Easing::Linear,
    };
    match rank {
        2 => Some(Animation {
            tracks: vec![track(Property::Rotation, &[(0.0, 0.0), (1.0, 360.0)])],
            duration_ms: 900,
            delay_ms: 0,
            iterations: None,
            direction: Direction::Normal,
            hold: false,
        }),
        _ => None,
    }
}

impl Widget {
    fn update_motion(&mut self, rank: u8) -> Result<(), String> {
        if self.animated_rank == Some(rank) {
            return Ok(());
        }
        let Some(icon) = self.icon else {
            return Ok(());
        };
        // One request per state transition; progress text never restarts the clock.
        motion::stop(icon)?;
        if let Some(animation) = animation(rank) {
            motion::animate(icon, &animation)?;
        }
        self.animated_rank = Some(rank);
        Ok(())
    }

    pub fn update(&mut self, status: &Status, contexts: &[String]) {
        let detail = compact(&status.detail, 80);
        let text = match status.rank {
            1 => "Language Services: Ready",
            2 if detail == "Starting" || detail.starts_with("Restarting") => {
                "Language Services: Starting"
            }
            2 => "Language Services: Working",
            3 => "Language Services: Error",
            _ => "Language Services: Stopped",
        };
        let color = match status.rank {
            1 => 0x10b981,
            2 => 0x818cf8,
            3 => 0xf43f5e,
            _ => 0x8b919b,
        };
        if self.prefix.is_empty() {
            self.prefix = "language-status".into();
        }
        let mut root = description(&self.prefix, UiKind::Row);
        // The row describes the state; the text child supplies changing progress.
        // Keep the row and icon untouched while that state remains the same.
        root.label = Some("Language Services".into());
        let mut body = description(&format!("{}-text", self.prefix), UiKind::Text);
        body.text = Some(text.into());
        body.foreground = Some(0xa1a1aa);
        let mut icon = description(&format!("{}-icon", self.prefix), UiKind::Icon);
        icon.icon = Some(
            match status.rank {
                1 => READY_ICON,
                2 => LOADING_ICON,
                3 => ERROR_ICON,
                _ => STOPPED_ICON,
            }
            .into(),
        );
        icon.foreground = Some(color);
        icon.width = Some(14.0);
        icon.height = Some(14.0);

        let result = (|| -> Result<(), String> {
            if let Some(node) = self.root {
                if self.animated_rank != Some(status.rank) {
                    host::ui_update(node, &root).map_err(|e| e.message)?;
                    host::ui_update(self.icon.unwrap(), &icon).map_err(|e| e.message)?;
                }
                if self.text_value != body.text {
                    host::ui_update(self.text.unwrap(), &body).map_err(|e| e.message)?;
                    self.text_value = body.text.clone();
                }
            } else {
                let node = host::ui_create(&UiParent::Mount("ui.detached".into()), &root)
                    .map_err(|e| e.message)?;
                let children = (|| {
                    for (id, width) in [
                        ("language-status-leading", 10.0),
                        ("language-status-icon-gap", 6.0),
                    ] {
                        if id.ends_with("icon-gap") {
                            self.icon = Some(
                                host::ui_create(&UiParent::Node(node), &icon)
                                    .map_err(|e| e.message)?,
                            );
                        }
                        let mut space = description(&format!("{}-{id}", self.prefix), UiKind::Text);
                        space.text = Some(String::new());
                        space.width = Some(width);
                        host::ui_create(&UiParent::Node(node), &space).map_err(|e| e.message)?;
                    }
                    // The surrounding native layout owns spacing to the next item.
                    self.text =
                        Some(host::ui_create(&UiParent::Node(node), &body).map_err(|e| e.message)?);
                    tree::place(
                        node,
                        &tree::Placement {
                            target: tree::Target::PublicId("statusbar.cursor-position".into()),
                            relation: tree::Relation::Before,
                            order: 100,
                        },
                    )?;
                    Ok::<_, String>(())
                })();
                if let Err(error) = children {
                    let _ = host::ui_remove(node);
                    self.text = None;
                    self.icon = None;
                    return Err(error);
                }
                self.root = Some(node);
                self.text_value = body.text.clone();
            }
            self.update_motion(status.rank)?;
            let mut contexts = contexts.to_vec();
            contexts.sort();
            contexts.dedup();
            if self.published.as_ref() != Some(&(status.rank, contexts.clone())) {
                shared::publish(
                    self.root.unwrap(),
                    &shared::Contribution {
                        key: SHARED_KEY.into(),
                        contexts: Some(contexts.clone()),
                        priority: i32::from(status.rank),
                    },
                )?;
                self.published = Some((status.rank, contexts));
            }
            Ok(())
        })();
        if let Err(error) = result {
            host::log(1, &format!("Status widget: {error}"));
            // Never leave a partially published candidate as an independent entry.
            self.dispose();
        }
    }

    pub fn dispose(&mut self) {
        if let Some(icon) = self.icon.take() {
            let _ = motion::stop(icon);
        }
        if let Some(node) = self.root.take() {
            let _ = host::ui_remove(node);
        }
        self.text = None;
        self.text_value = None;
        self.animated_rank = None;
        self.published = None;
    }
}
