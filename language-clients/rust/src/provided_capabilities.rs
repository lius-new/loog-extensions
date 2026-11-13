//! Static server capabilities are published per process and exact resource,
//! without sharing the base client's native-path configuration.
use super::*;
use api::session::{LanguageMode as Mode, Reply, ResourceProviderConfiguration as Configuration};
pub const KINDS: &[&str] = &[
    "hover",
    "diagnostics",
    "completion",
    "definition",
    "references",
    "formatting",
    "format-on-type",
    "rename",
    "code-actions",
    "execute-command",
];
fn yes(value: &Value) -> bool {
    !value.is_null() && value != &Value::Bool(false)
}
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 256 && !s.contains('\0'))
        .take(64)
        .map(str::to_owned)
        .collect()
}
pub fn describe(
    kind: &str,
    capabilities: &Value,
    ready: bool,
    revision: u64,
    contexts: Vec<api::ResourceRef>,
) -> Configuration {
    let field = match kind {
        "hover" => "hoverProvider",
        "completion" => "completionProvider",
        "definition" => "definitionProvider",
        "references" => "referencesProvider",
        "formatting" => "documentFormattingProvider",
        "format-on-type" => "documentOnTypeFormattingProvider",
        "rename" => "renameProvider",
        "code-actions" => "codeActionProvider",
        "execute-command" => "executeCommandProvider",
        _ => "",
    };
    let options = &capabilities[field];
    let modes = match kind {
        "definition" => Some(vec![
            (Mode::Definition, "definitionProvider"),
            (Mode::Declaration, "declarationProvider"),
            (Mode::TypeDefinition, "typeDefinitionProvider"),
            (Mode::Implementation, "implementationProvider"),
        ]),
        "formatting" => Some(vec![
            (Mode::Full, "documentFormattingProvider"),
            (Mode::Range, "documentRangeFormattingProvider"),
        ]),
        _ => None,
    }
    .map(|modes| {
        modes
            .into_iter()
            .filter(|(_, field)| yes(&capabilities[*field]))
            .map(|(mode, _)| mode)
            .collect::<Vec<_>>()
    });
    let mut triggers = strings(&options["triggerCharacters"]);
    triggers.retain(|s| s.len() <= 16);
    if kind == "format-on-type" {
        triggers = options["firstTriggerCharacter"]
            .as_str()
            .filter(|v| v.chars().count() == 1)
            .map(str::to_owned)
            .into_iter()
            .collect();
        triggers.extend(
            strings(&options["moreTriggerCharacter"])
                .into_iter()
                .filter(|s| s.chars().count() == 1),
        );
        triggers.truncate(64);
    }
    Configuration {
        provider: format!("{ID}.{kind}"),
        contexts,
        enabled: ready
            && (kind == "diagnostics"
                || modes
                    .as_ref()
                    .map_or_else(|| yes(options), |m| !m.is_empty())),
        modes,
        revision,
        // Offers are always lazy: the adapter can resolve already-complete
        // server actions locally, even without a server resolve method.
        resolve: kind == "code-actions" || yes(&options["resolveProvider"]),
        prepare: yes(&options["prepareProvider"]),
        triggers,
        retriggers: strings(&options["retriggerCharacters"])
            .into_iter()
            .filter(|s| s.len() <= 16)
            .collect(),
        commands: strings(&options["commands"])
            .into_iter()
            .filter(|s| !s.contains(['\r', '\n']))
            .collect(),
        aliases: if kind == "formatting" {
            vec![
                crate::NAME.into(),
                ID.into(),
                ID.rsplit('.').next().unwrap_or(ID).into(),
            ]
        } else {
            vec![]
        },
    }
}
#[derive(Default)]
pub struct Configurations {
    last: Option<Vec<Configuration>>,
    next: Option<Vec<Configuration>>,
    pending: Option<(u64, Vec<Configuration>)>,
    oversized: bool,
}
impl Configurations {
    pub fn update(&mut self, configurations: Vec<Configuration>) {
        let oversized = configurations.len() > 128
            || serde_json::to_vec(&configurations).map_or(true, |v| v.len() > 512 * 1024);
        if oversized && !self.oversized {
            error("Resource capability metadata exceeds its budget; capabilities withdrawn");
        }
        self.oversized = oversized;
        self.next = Some(if oversized { vec![] } else { configurations });
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
        self.last = Some(self.pending.take().unwrap().1);
        if !matches!(result, Ok(Reply::Done)) {
            error("Resource capability configuration was rejected");
        }
        true
    }
    pub fn needs_drive(&self) -> bool {
        self.pending.is_none() && self.next.is_some() && self.next != self.last
    }
    pub fn drive(&mut self, submit: &mut impl FnMut(Command) -> Result<u64, String>) {
        if !self.needs_drive() {
            return;
        }
        if let Ok(ticket) = submit(Command::ConfigureLanguageProviders {
            configurations: self.next.as_ref().unwrap().clone(),
        }) {
            self.pending = Some((ticket, self.next.take().unwrap()));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capability_options_and_deferred_publications_remain_scoped_to_exact_resources() {
        let resource = |environment| {
            api::ResourceRef::provided(
                api::EnvironmentKey::new("provider", environment).unwrap(),
                "/same.rs",
            )
            .unwrap()
        };
        let capabilities = json!({"renameProvider":{"prepareProvider":true},"documentRangeFormattingProvider":true,
            "documentOnTypeFormattingProvider":{"firstTriggerCharacter":";","moreTriggerCharacter":["}"]}});
        let rename = describe("rename", &capabilities, true, 1, vec![resource("one")]);
        assert!(rename.enabled && rename.prepare);
        let format = describe("formatting", &capabilities, true, 1, vec![resource("one")]);
        assert_eq!(format.modes, Some(vec![Mode::Range]));
        assert!(format.enabled);
        assert_eq!(
            describe("format-on-type", &capabilities, true, 1, vec![]).triggers,
            vec![";", "}"]
        );
        assert!(!describe("rename", &capabilities, false, 1, vec![]).enabled);
        let mut states = Configurations::default();
        states.update(vec![]);
        assert!(states.needs_drive());
        states.drive(&mut |_| Err("busy".into()));
        assert!(states.needs_drive());
        states.drive(&mut |_| Ok(1));
        states.update(vec![rename]);
        states.drive(&mut |_| panic!("one outstanding"));
        assert!(states.receive(&Event::Reply {
            ticket: 1,
            result: Ok(Reply::Done)
        }));
        states.drive(&mut |command| {
            let Command::ConfigureLanguageProviders { configurations } = command else {
                panic!("configuration")
            };
            assert_eq!(configurations[0].contexts, vec![resource("one")]);
            assert_ne!(configurations[0].contexts, vec![resource("two")]);
            Ok(2)
        });
        states.receive(&Event::Reply {
            ticket: 2,
            result: Ok(Reply::Done),
        });
        states.update(vec![]);
        assert!(states.needs_drive());
    }
}
