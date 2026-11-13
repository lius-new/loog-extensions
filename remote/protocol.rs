//! Private SSH/Agent protocol; none of these details belong in the host.
use base64::{Engine, engine::general_purpose::STANDARD};
use loog_workspace_api::{Answer, Failure, FailureKind, Operation};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const AGENT: &[u8] = concat!(include_str!("search_agent.py"), "\n", include_str!("agent.py")).as_bytes();
#[cfg(target_arch = "wasm32")]
pub const SEARCH_BUNDLE: &[u8] = include_bytes!("search/target/bundle.gz");
#[cfg(not(target_arch = "wasm32"))]
pub const SEARCH_BUNDLE: &[u8] = &[];
#[cfg(target_arch = "wasm32")]
pub const SEARCH_SIZE: &str = env!("LOOG_SEARCH_SIZE");
#[cfg(not(target_arch = "wasm32"))]
pub const SEARCH_SIZE: &str = "0";
#[cfg(target_arch = "wasm32")]
pub const SEARCH_SHA256: &str = env!("LOOG_SEARCH_SHA256");
#[cfg(not(target_arch = "wasm32"))]
pub const SEARCH_SHA256: &str = "";
pub const MAX_FRAME: usize = 1024 * 1024;
pub const CHUNK: u32 = 128 * 1024;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub target: String,
    pub port: u16,
    pub root: String,
    #[serde(default)]
    pub config: String,
    /// A private key file to offer instead of the default keys; empty uses the defaults.
    #[serde(default)]
    pub key: String,
}
/// The remote folder that stands for the login directory of the SSH user.
pub const HOME: &str = "~";
impl Default for Profile {
    fn default() -> Self {
        Self { target: String::new(), port: 22, root: HOME.into(), config: String::new(), key: String::new() }
    }
}
impl Profile {
    /// A profile for a target that was listed rather than typed: a distribution or a container.
    #[cfg(not(transport = "ssh"))]
    pub fn for_target(target: &str) -> Result<Self, String> {
        let profile = Self { target: target.into(), ..Self::default() };
        profile.validate()?;
        Ok(profile)
    }
    /// Split what a person types for a host — `alias`, `user@host` or either with
    /// `:port` — into a target and a port. Bracketed IPv6 literals keep their colons.
    #[cfg(transport = "ssh")]
    pub fn from_input(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let (target, port) = match text.rsplit_once(':') {
            Some((target, port)) if !target.is_empty() && !port.is_empty() && !port.contains(']')
                && (!target.contains(':') || target.ends_with(']')) =>
                (target, port.parse::<u16>().map_err(|_| "The port is a number from 1 to 65535".to_owned())?),
            _ => (text, 22),
        };
        let profile = Self { target: target.into(), port, ..Self::default() };
        profile.validate()?;
        Ok(profile)
    }
    /// What is shown for the host: its target, with a port that is not the usual one.
    pub fn label(&self) -> String {
        if self.port == 22 || cfg!(not(transport = "ssh")) { self.target.clone() } else { format!("{}:{}", self.target, self.port) }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !crate::transport::target_valid(&self.target) {
            return Err(crate::transport::target_hint().into());
        }
        if self.port == 0 || self.root.len() > 4096 || self.root.contains('\0')
            || !(self.root == HOME || self.root.starts_with('/')) {
            return Err("Enter a valid port and an absolute Linux folder path".into());
        }
        if self.config.len() > 4096 || self.config.contains('\0') {
            return Err("Enter a valid SSH configuration file path".into());
        }
        if self.key.len() > 4096 || self.key.contains('\0') || self.key.starts_with('-') {
            return Err("Enter a valid private key file path".into());
        }
        Ok(())
    }
    pub fn identity(&self) -> String {
        #[cfg(transport = "ssh")]
        { serde_json::to_string(&(&self.target, self.port, &self.config)).unwrap() }
        #[cfg(not(transport = "ssh"))]
        { serde_json::to_string(&(&self.target,)).unwrap() }
    }
    /// The Python that checks the bundled Agent against its digest and runs it, as one
    /// argument for a channel that starts a program without a shell. It holds no quote
    /// character other than the single quote, so every channel passes it through as is.
    #[cfg(not(transport = "ssh"))]
    fn bootstrap_source() -> String {
        format!("import sys,hashlib;s=sys.stdin.buffer.read({});assert hashlib.sha256(s).hexdigest()=='{:x}';exec(compile(s,'<loog-agent>','exec'))",
            AGENT.len(), Sha256::digest(AGENT))
    }
    /// The program that opens the channel to the Agent and its arguments. Only the fixed
    /// bootstrap is sent; no target, folder or user text is interpolated into a shell.
    #[cfg(transport = "wsl")]
    pub fn launch(&self) -> Result<(&'static str, Vec<String>), String> {
        self.validate()?;
        Ok(("wsl.exe", ["-d", self.target.as_str(), "--exec", "python3", "-u", "-c", &Self::bootstrap_source()]
            .map(String::from).to_vec()))
    }
    #[cfg(transport = "docker")]
    pub fn launch(&self) -> Result<(&'static str, Vec<String>), String> {
        self.validate()?;
        Ok(("docker", ["exec", "-i", self.target.as_str(), "python3", "-u", "-c", &Self::bootstrap_source()]
            .map(String::from).to_vec()))
    }
    #[cfg(transport = "ssh")]
    pub fn launch(&self) -> Result<(&'static str, Vec<String>), String> {
        self.arguments().map(|arguments| ("ssh", arguments))
    }
    #[cfg(transport = "ssh")]
    pub fn arguments(&self) -> Result<Vec<String>, String> {
        self.validate()?;
        let digest = format!("{:x}", Sha256::digest(AGENT));
        // The fixed bootstrap verifies the bundled source before running it.
        // No target, folder or user text is interpolated into the remote shell.
        let bootstrap = format!(
            "python3 -u -c 'import sys,hashlib; s=sys.stdin.buffer.read({}); assert hashlib.sha256(s).hexdigest()==\"{}\"; exec(compile(s,\"<loog-agent>\",\"exec\"))'",
            AGENT.len(), digest);
        let mut arguments = vec!["-T".into(), "-o".into(), "BatchMode=yes".into(),
            // A host seen for the first time is trusted and recorded, as a person typing its
            // address expects; a host whose recorded key changed is still refused.
            "-o".into(), "StrictHostKeyChecking=accept-new".into(), "-o".into(), "ConnectTimeout=10".into(),
            "-o".into(), "ServerAliveInterval=10".into(), "-o".into(), "ServerAliveCountMax=3".into(),
            "-p".into(), self.port.to_string()];
        if !self.config.is_empty() { arguments.extend(["-F".into(), self.config.clone()]); }
        // Only the chosen key is offered: another key must not be tried first.
        if !self.key.is_empty() {
            arguments.extend(["-i".into(), self.key.clone(), "-o".into(), "IdentitiesOnly=yes".into()]);
        }
        arguments.extend(["--".into(), self.target.clone(), bootstrap]);
        Ok(arguments)
    }
}

#[derive(Default)]
pub struct Frames { pending: Vec<u8> }
impl Frames {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>, String> {
        let mut frames = Vec::new();
        for segment in bytes.split_inclusive(|byte| *byte == b'\n') {
            if self.pending.len() + segment.len() > MAX_FRAME {
                return Err("Agent response exceeds the frame budget".into());
            }
            self.pending.extend_from_slice(segment);
            if segment.last() == Some(&b'\n') {
                let frame: Value = serde_json::from_slice(&self.pending).map_err(|_| "Invalid Agent response")?;
                if !frame.is_object() { return Err("Invalid Agent response".into()); }
                self.pending.clear();
                frames.push(frame);
                if frames.len() > 256 { return Err("Too many Agent responses in one dispatch".into()); }
            }
        }
        Ok(frames)
    }
}
pub fn frame(id: u64, method: &str, params: Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&json!({"id":id, "method":method, "params":params})).unwrap();
    bytes.push(b'\n');
    bytes
}
pub fn operation(root: u64, op: &Operation) -> Result<(&'static str, Value), Failure> {
    let (method, mut value) = match op {
        Operation::ParentPath { path } => ("parent_path", json!({"path":path})),
        Operation::ResourceUri { path } => ("resource_uri", json!({"path":path})),
        Operation::ResolveUri { uri } => ("resolve_uri", json!({"uri":uri})),
        Operation::ChildPath { parent, name } => ("child_path", json!({"parent":parent,"name":name})),
        Operation::ResolvePath { base, relative } => ("resolve_path", json!({"base":base,"relative":relative})),
        Operation::RelativePath { base, path } => ("relative_path", json!({"base":base,"path":path})),
        Operation::CreateFile { path } => ("create_file", json!({"path":path})),
        Operation::Relocate { source, target, tracked } =>
            ("relocate", json!({"source":source,"target":target,"tracked":tracked})),
        Operation::Stat { path } => ("stat", json!({"path":path})),
        Operation::List { path, cursor, limit } => ("list", json!({"path":path,"cursor":cursor,"limit":limit.min(&512)})),
        Operation::Read { path, offset, limit } => ("read", json!({"path":path,"offset":offset,"limit":limit.min(&CHUNK)})),
        Operation::Write { path, bytes, expected_revision } if bytes.len() <= 256 * 1024 =>
            ("write", json!({"path":path,"data":STANDARD.encode(bytes),"expected_revision":expected_revision})),
        Operation::BeginWrite { path, expected_revision, size } =>
            ("write_begin", json!({"path":path,"expected_revision":expected_revision,"size":size})),
        Operation::AppendWrite { write, offset, bytes } if bytes.len() <= 256 * 1024 =>
            ("write_append", json!({"write":write,"offset":offset,"data":STANDARD.encode(bytes)})),
        Operation::CommitWrite { write } => ("write_commit", json!({"write":write})),
        Operation::AbortWrite { write } => ("write_abort", json!({"write":write})),
        Operation::CreateDirectory { path } => ("mkdir", json!({"path":path})),
        Operation::Rename { source, target, overwrite } =>
            ("rename", json!({"source":source,"target":target,"overwrite":overwrite})),
        Operation::Remove { path, recursive } => ("remove", json!({"path":path,"recursive":recursive})),
        Operation::Watch { path } => ("watch", json!({"path":path})),
        Operation::Unwatch { watch } => ("unwatch", json!({"watch":watch})),
        Operation::Search { path, options } => ("search", json!({"path":path,"options":options})),
        Operation::SearchNext { search, limit } => ("search_next", json!({"search":search,"limit":limit.min(&32)})),
        Operation::SearchClose { search } => ("search_close", json!({"search":search})),
        Operation::Spawn { program, args, cwd, env } => ("spawn", json!({"program":program,"args":args,"cwd":cwd,"env":env})),
        Operation::Input { process, bytes } => ("process_input", json!({"process":process,"data":STANDARD.encode(bytes)})),
        Operation::CloseInput { process } => ("process_close_input", json!({"process":process})),
        Operation::Kill { process } => ("process_kill", json!({"process":process})),
        Operation::TerminalOpen { cwd, program, args, cols, rows } =>
            ("terminal_open", json!({"cwd":cwd,"program":program,"args":args,"cols":cols,"rows":rows})),
        Operation::TerminalInput { terminal, bytes } =>
            ("terminal_input", json!({"terminal":terminal,"data":STANDARD.encode(bytes)})),
        Operation::TerminalResize { terminal, cols, rows } =>
            ("terminal_resize", json!({"terminal":terminal,"cols":cols,"rows":rows})),
        Operation::TerminalClose { terminal } => ("terminal_close", json!({"terminal":terminal})),
        _ => return Err(Failure::new(FailureKind::Unsupported, "Operation is not available in this provider")),
    };
    value["root"] = root.into();
    Ok((method, value))
}
pub fn response(value: &Value) -> Result<Value, Failure> {
    if let Some(error) = value.get("error") {
        return Err(serde_json::from_value(error.clone()).map_err(|_| malformed())?);
    }
    value.get("result").cloned().ok_or_else(malformed)
}
pub fn notification(mut value: Value) -> Result<loog_workspace_api::Notification, Failure> {
    if value["kind"] == "output" || value["kind"] == "terminal_output" {
        let bytes = STANDARD.decode(value["data"].as_str().ok_or_else(malformed)?).map_err(|_| malformed())?;
        if bytes.len() > 16 * 1024 { return Err(malformed()); }
        value.as_object_mut().ok_or_else(malformed)?.remove("data");
        value["bytes"] = serde_json::to_value(bytes).unwrap();
    }
    serde_json::from_value(value).map_err(|_| malformed())
}
pub fn answer(op: &Operation, value: Value) -> Result<Answer, Failure> {
    let decode = |value| serde_json::from_value(value).map_err(|_| malformed());
    Ok(match op {
        Operation::ChildPath { .. } | Operation::ResolveUri { .. } | Operation::ResolvePath { .. } => Answer::Path { path: serde_json::from_value(value["path"].clone()).map_err(|_| malformed())? },
        Operation::RelativePath { .. } => {
            let components: Option<Vec<String>> = serde_json::from_value(value["components"].clone()).map_err(|_| malformed())?;
            if components.as_ref().is_some_and(|parts| parts.len() > 128
                || parts.iter().map(String::len).sum::<usize>() > 32 * 1024
                || parts.iter().any(|part| part.is_empty() || matches!(part.as_str(), "." | "..") || part.contains(['/', '\0']))) {
                return Err(malformed());
            }
            Answer::RelativePath { components }
        },
        Operation::ParentPath { .. } => Answer::Parent { path: serde_json::from_value(value["path"].clone()).map_err(|_| malformed())? },
        Operation::ResourceUri { .. } => {
            let uri = value["uri"].as_str().filter(|uri| !uri.is_empty() && uri.len() <= 128 * 1024 && !uri.contains('\0')).ok_or_else(malformed)?;
            Answer::Uri { uri: uri.into() }
        }
        Operation::CreateFile { .. } => Answer::Info { file: decode(value)? },
        Operation::Relocate { .. } => Answer::Relocated {
            entries: serde_json::from_value(value["entries"].clone()).map_err(|_| malformed())?,
        },
        Operation::Stat { .. } => Answer::Info { file: decode(value)? },
        Operation::List { .. } => Answer::Directory {
            entries: serde_json::from_value(value["entries"].clone()).map_err(|_| malformed())?,
            cursor: serde_json::from_value(value["cursor"].clone()).map_err(|_| malformed())?,
        },
        Operation::Read { limit, .. } => {
            let bytes = STANDARD.decode(value["data"].as_str().ok_or_else(malformed)?).map_err(|_| malformed())?;
            if bytes.len() > (*limit).min(CHUNK) as usize { return Err(malformed()); }
            Answer::Bytes {
                bytes, revision: value["revision"].as_str().map(String::from),
                eof: value["eof"].as_bool().ok_or_else(malformed)?,
            }
        }
        Operation::Write { .. } | Operation::CommitWrite { .. } => Answer::Written { revision: value["revision"].as_str().map(String::from) },
        Operation::BeginWrite { .. } => Answer::Started { handle: value["write"].as_u64().ok_or_else(malformed)? },
        Operation::Watch { .. } => Answer::Started { handle: value["watch"].as_u64().ok_or_else(malformed)? },
        Operation::Search { .. } => Answer::Started { handle: value["search"].as_u64().ok_or_else(malformed)? },
        Operation::SearchNext { .. } => Answer::SearchMatches {
            page: serde_json::from_value(value).map_err(|_| malformed())?,
        },
        Operation::Spawn { .. } => Answer::Started { handle: value["process"].as_u64().ok_or_else(malformed)? },
        Operation::TerminalOpen { .. } => Answer::Started { handle: value["terminal"].as_u64().ok_or_else(malformed)? },
        _ => Answer::Ok,
    })
}
fn malformed() -> Failure { Failure::new(FailureKind::Failed, "Invalid Agent response") }

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(transport = "ssh"))]
    #[test]
    fn the_channel_starts_with_one_fixed_argument_and_a_target_cannot_be_an_option() {
        let profile = Profile::for_target("Ubuntu-22.04").unwrap();
        let (program, args) = profile.launch().unwrap();
        assert_eq!(program, if cfg!(transport = "wsl") { "wsl.exe" } else { "docker" });
        let code = args.last().unwrap();
        // The Agent is checked against its digest, and nothing else is built into the code.
        assert!(code.contains(&format!("{:x}", Sha256::digest(AGENT))));
        assert!(!code.contains('"'), "only the single quote survives every channel");
        assert!(args.contains(&"Ubuntu-22.04".to_owned()));
        assert!(Profile::for_target("-oProxyCommand=bad").is_err());
        assert!(Profile::for_target("").is_err());
        assert!(Profile::for_target("a;b").is_err());
    }
    #[cfg(transport = "ssh")]
    #[test]
    fn shell_text_never_contains_the_user_folder_and_aliases_cannot_be_options() {
        let profile = Profile { target: "user@host".into(), port: 2222, root: "/work/';echo injected".into(), ..Profile::default() };
        let args = profile.arguments().unwrap();
        assert!(!args.last().unwrap().contains("injected"));
        assert!(Profile { target: "-oProxyCommand=bad".into(), ..profile }.arguments().is_err());
    }
    #[cfg(transport = "ssh")]
    #[test]
    fn typed_hosts_split_into_target_and_port() {
        let host = |text: &str| Profile::from_input(text).map(|profile| (profile.target, profile.port));
        assert_eq!(host("dev-box"), Ok(("dev-box".into(), 22)));
        assert_eq!(host(" liu@10.0.0.21:2222 "), Ok(("liu@10.0.0.21".into(), 2222)));
        assert_eq!(host("[::1]:2200"), Ok(("[::1]".into(), 2200)));
        assert_eq!(host("[::1]"), Ok(("[::1]".into(), 22)));
        assert!(host("host:99999").is_err());
        assert!(host("-oProxyCommand=x").is_err());
        assert!(host("").is_err());
        assert_eq!(Profile::from_input("a:2222").unwrap().label(), "a:2222");
        assert_eq!(Profile::from_input("a").unwrap().label(), "a");
    }
    #[cfg(transport = "ssh")]
    #[test]
    fn a_chosen_key_is_the_only_one_offered_and_cannot_be_an_option() {
        let profile = Profile { target: "host".into(), key: "C:/keys/id".into(), ..Profile::default() };
        let args = profile.arguments().unwrap();
        let at = args.iter().position(|arg| arg == "-i").unwrap();
        assert_eq!(args[at + 1], "C:/keys/id");
        assert!(args.contains(&"IdentitiesOnly=yes".to_owned()));
        assert!(Profile { key: "-oProxyCommand=bad".into(), ..profile }.arguments().is_err());
    }
    #[test]
    fn frames_accept_split_utf8_and_multiple_replies_and_bound_unterminated_input() {
        let mut frames = Frames::default();
        let encoded = "{\"id\":1,\"result\":\"中文\"}\n{\"id\":2}\n".as_bytes();
        assert!(frames.push(&encoded[..20]).unwrap().is_empty());
        assert_eq!(frames.push(&encoded[20..]).unwrap().len(), 2);
        assert!(frames.push(&vec![b'a'; MAX_FRAME + 1]).is_err());
    }
    #[test]
    fn binary_reads_keep_raw_bytes_and_refuse_oversized_provider_replies() {
        let op = Operation::Read { path: "/file".to_owned().try_into().unwrap(), offset: 0, limit: 3 };
        let result = answer(&op, json!({"data":STANDARD.encode([0,255,13]),"eof":true,"revision":"v1"})).unwrap();
        assert!(matches!(result, Answer::Bytes { bytes, .. } if bytes == [0,255,13]));
        assert!(answer(&op, json!({"data":STANDARD.encode([0,1,2,3]),"eof":true})).is_err());
    }
}
