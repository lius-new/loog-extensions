#[cfg(test)]
use crate::host::tree;
#[cfg(not(test))]
use crate::loom::ui::tree;
use crate::{NAME, host, loom::extension::types::*};
#[derive(Clone, Default)]
pub struct Status {
    pub rank: u8,
    pub detail: String,
}
#[derive(Default)]
pub struct Widget {
    root: Option<u64>,
    text: Option<u64>,
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
impl Widget {
    pub fn update(&mut self, status: &Status) {
        let detail = compact(&status.detail, 80);
        let text = match status.rank {
            1 => NAME.to_owned(),
            2 => format!(
                "{NAME}: {}",
                if detail.is_empty() {
                    "Working"
                } else {
                    &detail
                }
            ),
            3 => format!(
                "{NAME}: Error{}",
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(" · {detail}")
                }
            ),
            _ => format!("{NAME}: Stopped"),
        };
        let mut root = description("language-status", UiKind::Row);
        root.background = Some(match status.rank {
            1 => 0x10b981,
            2 => 0x818cf8,
            3 => 0xf43f5e,
            _ => 0x71717a,
        });
        root.label = Some(compact(
            &if status.rank == 1 {
                format!("{NAME}: Ready")
            } else {
                text.clone()
            },
            256,
        ));
        let mut body = description("language-status-text", UiKind::Text);
        body.text = Some(compact(&text, 96));
        body.foreground = Some(0xffffff);
        let result = (|| -> Result<(), String> {
            if let Some(node) = self.root {
                host::ui_update(node, &root).map_err(|e| e.message)?;
                host::ui_update(self.text.unwrap(), &body).map_err(|e| e.message)?;
            } else {
                let node = host::ui_create(&UiParent::Mount("ui.detached".into()), &root)
                    .map_err(|e| e.message)?;
                let children = (|| {
                    for id in ["language-status-leading", "language-status-trailing"] {
                        let mut space = description(id, UiKind::Text);
                        space.text = Some(String::new());
                        space.width = Some(10.0);
                        if id.ends_with("trailing") {
                            self.text = Some(
                                host::ui_create(&UiParent::Node(node), &body)
                                    .map_err(|e| e.message)?,
                            );
                        }
                        host::ui_create(&UiParent::Node(node), &space).map_err(|e| e.message)?;
                    }
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
                if let Err(e) = children {
                    let _ = host::ui_remove(node);
                    self.text = None;
                    return Err(e);
                }
                self.root = Some(node);
            }
            Ok(())
        })();
        if let Err(error) = result {
            host::log(1, &format!("Status widget: {error}"));
        }
    }
    pub fn dispose(&mut self) {
        if let Some(node) = self.root.take() {
            let _ = host::ui_remove(node);
        }
        self.text = None;
    }
}
