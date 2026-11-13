//! The language plugin chooses pinned tools; the broker only spawns a process.
use super::*;
const SOURCE: &str = include_str!("../environment_tools.py");
const START: &str = "import json,sys;n=int(sys.argv[1]);exec(compile(''.join(sys.argv[2:2+n]),'<loog-language-tools>','exec'),{'__name__':'__main__','OPTIONS':json.loads(''.join(sys.argv[2+n:]))})";

fn chunks(mut text: &str) -> Vec<String> {
    let mut chunks = vec![];
    while !text.is_empty() {
        let mut end = text.len().min(4096);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push(text[..end].into());
        text = &text[end..];
    }
    chunks
}

pub(super) fn command(
    settings: &loog_language_workspace::Settings,
    args: Vec<String>,
) -> Result<(String, Vec<String>), String> {
    let string = |name: &str| -> Result<String, String> {
        let text = settings
            .get(&format!("{ID}.{name}"))
            .ok_or_else(|| format!("Missing tool setting {name}"))?;
        serde_json::from_str::<String>(text).map_err(|_| format!("Invalid tool setting {name}"))
    };
    let python = string("environmentPython")?;
    let cache = string("environmentCache")?;
    if python.is_empty()
        || python.len() > 4096
        || cache.len() > 4096
        || [&python, &cache]
            .iter()
            .any(|v| v.contains('\0') || v.contains("${"))
    {
        return Err("Invalid target tool preparation settings".into());
    }
    let spec: Value = serde_json::from_str(
        crate::ENVIRONMENT_TOOLS.ok_or("This language package has no pinned target tools")?,
    )
    .map_err(|_| "Invalid compiled target tool recipe")?;
    let payload = json!({"spec":spec,"cache":cache,"args":args}).to_string();
    if payload.len() > 64 * 1024 {
        return Err("Target tool preparation exceeds its budget".into());
    }
    let code = chunks(SOURCE);
    let mut arguments = vec![
        "-I".into(),
        "-u".into(),
        "-c".into(),
        START.into(),
        code.len().to_string(),
    ];
    arguments.extend(code);
    arguments.extend(chunks(&payload));
    Ok((python, arguments))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparation_uses_compiled_pins_isolated_python_and_bounded_unicode_arguments() {
        let settings = std::sync::Arc::new(BTreeMap::from([
            (
                format!("{ID}.environmentPython"),
                json!("/target/python3").to_string(),
            ),
            (
                format!("{ID}.environmentCache"),
                json!("/target/cache-中文").to_string(),
            ),
        ]));
        let expected = vec!["--stdio".to_owned(), "中😀".repeat(2000)];
        let (program, args) = command(&settings, expected.clone()).unwrap();
        assert_eq!(program, "/target/python3");
        assert_eq!(&args[..3], ["-I", "-u", "-c"]);
        assert!(args.len() < 128 && args.iter().all(|arg| arg.len() <= 8192));
        let count = args[4].parse::<usize>().unwrap();
        assert_eq!(args[5..5 + count].join(""), SOURCE);
        let value: Value = serde_json::from_str(&args[5 + count..].join("")).unwrap();
        assert_eq!(value["spec"]["id"], ID);
        assert_eq!(value["cache"], "/target/cache-中文");
        assert_eq!(value["args"], json!(expected));
        assert!(
            value["spec"]["platforms"]["linux-x86_64"]["sha256"]
                .as_str()
                .unwrap()
                .len()
                == 64
        );
    }
}
