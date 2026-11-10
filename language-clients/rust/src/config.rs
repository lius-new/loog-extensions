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

pub(crate) fn language_id<'a>(language: &'a str, variant: Option<&str>) -> &'a str {
    match (language, variant) {
        ("javascript", Some("jsx")) => "javascriptreact",
        ("typescript", Some("tsx")) => "typescriptreact",
        ("bash", _) => "shellscript",
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
