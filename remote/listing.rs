//! Reading what a channel's own tool says can be connected to: the WSL distributions or
//! the running containers. The SSH package lists nothing; a host is typed.
#![allow(dead_code)]

/// One thing the person can choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    /// One short line about it: its state, version or image.
    pub detail: String,
}

/// Most bytes of a listing that are read; a longer one is cut, not trusted.
pub const MAX_BYTES: usize = 256 * 1024;
const MAX_TARGETS: usize = 200;

/// The program and arguments that print the listing.
#[cfg(transport = "wsl")]
pub fn command() -> (&'static str, Vec<String>) {
    ("wsl.exe", vec!["--list".into(), "--verbose".into()])
}
#[cfg(transport = "docker")]
pub fn command() -> (&'static str, Vec<String>) {
    ("docker", vec!["ps".into(), "--format".into(), "{{.Names}}|{{.Image}}|{{.Status}}".into()])
}
#[cfg(transport = "ssh")]
pub fn command() -> (&'static str, Vec<String>) {
    ("", Vec::new())
}

/// `wsl.exe` writes UTF-16 with little-endian code units, whatever the console says; text
/// from other programs is UTF-8. Either becomes a string, and a stray byte never fails it.
pub fn decode(bytes: &[u8]) -> String {
    let wide = bytes.len() >= 2 && bytes.len() % 2 == 0 && bytes.iter().skip(1).step_by(2).filter(|byte| **byte == 0).count() * 2 >= bytes.len() / 2;
    if wide {
        let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        String::from_utf16_lossy(&units).trim_start_matches('\u{feff}').to_owned()
    } else {
        String::from_utf8_lossy(bytes).trim_start_matches('\u{feff}').to_owned()
    }
}

/// The distributions of `wsl --list --verbose`: a header, then one line each with an
/// optional `*` for the default, the name, the state and the WSL version. The header and
/// the state are in the language of the system, so neither is matched by its words.
pub fn parse_wsl(bytes: &[u8]) -> Vec<Target> {
    decode(bytes)
        .lines()
        .map(|line| line.trim_matches(|c: char| c.is_whitespace() || c == '\0'))
        .filter(|line| !line.is_empty())
        .skip(1)
        .filter_map(|line| {
            let mut words = line.trim_start_matches('*').split_whitespace();
            let name = words.next()?;
            let rest: Vec<&str> = words.collect();
            // Docker Desktop keeps its own distributions; nothing can be opened in them.
            if name.starts_with("docker-desktop") || !crate::transport::target_valid(name) {
                return None;
            }
            let version = rest.last().filter(|word| word.chars().all(|c| c.is_ascii_digit()));
            let state = rest.get(..rest.len().saturating_sub(usize::from(version.is_some()))).unwrap_or_default().join(" ");
            let detail = match version {
                Some(version) if !state.is_empty() => format!("{state} · WSL {version}"),
                Some(version) => format!("WSL {version}"),
                None => state,
            };
            Some(Target { name: name.into(), detail })
        })
        .take(MAX_TARGETS)
        .collect()
}

/// The containers of `docker ps`: name, image and status separated by `|`.
pub fn parse_docker(bytes: &[u8]) -> Vec<Target> {
    decode(bytes)
        .lines()
        .filter_map(|line| {
            let mut parts = line.trim().splitn(3, '|');
            let name = parts.next()?.trim();
            let image = parts.next()?.trim();
            let status = parts.next().unwrap_or_default().trim();
            crate::transport::target_valid(name).then(|| Target {
                name: name.into(),
                detail: if status.is_empty() { image.into() } else { format!("{image} · {status}") },
            })
        })
        .take(MAX_TARGETS)
        .collect()
}

#[cfg(transport = "wsl")]
pub fn parse(bytes: &[u8]) -> Vec<Target> { parse_wsl(bytes) }
#[cfg(transport = "docker")]
pub fn parse(bytes: &[u8]) -> Vec<Target> { parse_docker(bytes) }
#[cfg(transport = "ssh")]
pub fn parse(_bytes: &[u8]) -> Vec<Target> { Vec::new() }

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        std::iter::once(0xfeffu16).chain(text.encode_utf16()).flat_map(u16::to_le_bytes).collect()
    }

    #[cfg(transport = "wsl")]
    #[test]
    fn distributions_are_read_from_utf16_without_the_header_default_mark_or_docker() {
        let text = "  NAME                   STATE           VERSION\r\n* Ubuntu-22.04          Running         2\r\n  Debian                 Stopped         2\r\n  docker-desktop         Stopped         2\r\n";
        let targets = parse_wsl(&utf16(text));
        assert_eq!(targets, vec![
            Target { name: "Ubuntu-22.04".into(), detail: "Running · WSL 2".into() },
            Target { name: "Debian".into(), detail: "Stopped · WSL 2".into() },
        ]);
    }

    #[cfg(transport = "wsl")]
    #[test]
    fn a_listing_in_another_language_and_in_utf8_is_still_read() {
        let text = "  名称            状态           版本\n* Ubuntu         正在运行        2\n";
        for bytes in [utf16(text), text.as_bytes().to_vec()] {
            assert_eq!(parse_wsl(&bytes), vec![Target { name: "Ubuntu".into(), detail: "正在运行 · WSL 2".into() }]);
        }
        assert!(parse_wsl(b"").is_empty());
        assert!(parse_wsl(&utf16("NAME STATE VERSION\r\n")).is_empty());
    }

    #[cfg(transport = "docker")]
    #[test]
    fn containers_are_read_with_their_image_and_status() {
        let text = "loog-dev|rust:1.82|Up 3 hours\npostgres-16|postgres:16|Up 3 days (healthy)\nbad name|x|y\n\n";
        let targets = parse_docker(text.as_bytes());
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0], Target { name: "loog-dev".into(), detail: "rust:1.82 · Up 3 hours".into() });
        assert_eq!(targets[1].detail, "postgres:16 · Up 3 days (healthy)");
        assert!(parse_docker(b"").is_empty());
    }

    #[test]
    fn a_listing_is_bounded() {
        let many: String = (0..1000).map(|index| format!("c{index}|image|Up\n")).collect();
        assert!(parse_docker(many.as_bytes()).len() <= 200);
        assert!(utf16("x").len() > 2);
    }
}
