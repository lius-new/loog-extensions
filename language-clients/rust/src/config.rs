//! Configuration is compiled into each package; no shared runtime or sessions.
#[derive(Clone, Copy)]
pub(crate) struct Configuration {
    pub languages: &'static [&'static str],
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub markers: &'static [&'static str],
    pub outermost: bool,
    pub file_fallback: bool,
}
impl Default for Configuration {
    fn default() -> Self {
        crate::CONFIG
    }
}

pub(crate) fn language_id<'a>(language: &'a str, variant: Option<&str>, path: &str) -> &'a str {
    match (language, variant) {
        ("javascript", Some("jsx")) => "javascriptreact",
        ("typescript", Some("tsx")) => "typescriptreact",
        ("bash", _) => "shellscript",
        ("objc", _) => "objective-c",
        // JSON and JSONC use the same Loog language and syntax grammar.
        ("json", _)
            if std::path::Path::new(path)
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonc")) =>
        {
            "jsonc"
        }
        _ => language,
    }
}

#[cfg(test)]
pub(crate) const GOPLS: Configuration = Configuration {
    languages: &["go"],
    program: "${extension}/gopls",
    args: &[],
    markers: &["go.work", "go.mod"],
    outermost: false,
    file_fallback: false,
};
