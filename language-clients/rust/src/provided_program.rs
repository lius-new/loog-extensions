//! Target-environment executable policy. Native package/runtime placeholders
//! never cross into a provider process request.
use super::*;

pub(super) fn language<'a>(language: &'a str, variant: &str, uri: &str) -> &'a str {
    match (language, variant) {
        ("javascript", "jsx") => "javascriptreact",
        ("typescript", "tsx") => "typescriptreact",
        ("bash", _) => "shellscript",
        ("objc", _) => "objective-c",
        ("json", _)
            if uri
                .split(['?', '#'])
                .next()
                .unwrap_or("")
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
                .ends_with(".jsonc") =>
        {
            "jsonc"
        }
        _ => language,
    }
}

pub(super) fn command(
    settings: &loog_language_workspace::Settings,
) -> Result<(String, Vec<String>), String> {
    let read = |name: &str| -> Result<Value, String> {
        let text = settings
            .get(&format!("{ID}.{name}"))
            .ok_or_else(|| format!("Missing environment setting {name}"))?;
        serde_json::from_str(text).map_err(|_| format!("Invalid environment setting {name}"))
    };
    let program = read("environmentProgram")?
        .as_str()
        .ok_or("Environment program must be a string")?
        .to_owned();
    let encoded = read("environmentArguments")?;
    let arguments: Value = serde_json::from_str(
        encoded
            .as_str()
            .ok_or("Environment arguments must be JSON text")?,
    )
    .map_err(|_| "Invalid environment argument JSON")?;
    let arguments = arguments
        .as_array()
        .ok_or("Environment arguments must be a JSON array")?;
    if program.is_empty() || program.len() > 4096 || arguments.len() > 64 {
        return Err("Environment command exceeds its budget".into());
    }
    let mut bytes = program.len();
    let args = arguments
        .iter()
        .map(|value| {
            let arg = value
                .as_str()
                .ok_or("Environment argument must be a string")?;
            bytes += arg.len();
            Ok(arg.to_owned())
        })
        .collect::<Result<Vec<_>, String>>()?;
    if bytes > 32 * 1024
        || std::iter::once(&program)
            .chain(args.iter())
            .any(|v| v.contains('\0') || v.contains("${"))
    {
        return Err("Invalid or unexpanded environment command".into());
    }
    Ok((program, args))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_language_uses_variants_and_server_uris_without_native_paths() {
        assert_eq!(
            language("json", "json", "file:///中文/SETTINGS.JSONC"),
            "jsonc"
        );
        assert_eq!(
            language("json", "json", "file:///a.json?name=.jsonc"),
            "json"
        );
        assert_eq!(
            language("typescript", "tsx", "file:///x.tsx"),
            "typescriptreact"
        );
        assert_eq!(
            language("javascript", "jsx", "file:///x.jsx"),
            "javascriptreact"
        );
        assert_eq!(language("rust", "rust", "opaque:///x"), "rust");
    }
    fn settings(program: &str, args: Value) -> loog_language_workspace::Settings {
        std::sync::Arc::new(BTreeMap::from([
            (
                format!("{ID}.environmentProgram"),
                json!(program).to_string(),
            ),
            (
                format!("{ID}.environmentArguments"),
                json!(args.to_string()).to_string(),
            ),
        ]))
    }
    #[test]
    fn target_runtime_and_script_paths_are_literal_and_bounded() {
        assert_eq!(
            command(&settings(
                "/target/node",
                json!(["/target/语言 server.js", "--stdio"])
            ))
            .unwrap(),
            (
                "/target/node".into(),
                vec!["/target/语言 server.js".into(), "--stdio".into()]
            )
        );
        assert_eq!(command(&settings("123", json!([]))).unwrap().0, "123");
        for (program, args) in [
            ("${extension}/server", json!([])),
            ("node", json!(["${runtime.node}/server"])),
            ("node", json!([0])),
            ("node", json!({})),
            ("node", json!(["nul\0"])),
            ("", json!([])),
            ("node", json!(vec!["x"; 65])),
            ("node", json!(["x".repeat(32 * 1024)])),
        ] {
            assert!(command(&settings(program, args)).is_err());
        }
        assert!(command(&Default::default()).is_err());
    }
}
