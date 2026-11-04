use crate::loom::extension::types::*;
use serde_json::{Value, json};
use std::rc::Rc;
pub type Result<T> = std::result::Result<T, String>;
pub fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
pub fn string(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}
pub fn optional(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}
pub fn number(value: &Value) -> u32 {
    value.as_u64().unwrap_or(0) as u32
}
pub fn yes(value: &Value) -> bool {
    !value.is_null() && value != false
}
pub fn normalize(path: &str) -> String {
    let p = path.replace('\\', "/");
    let p = if p
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("//?/UNC/"))
    {
        format!("//{}", &p[8..])
    } else if let Some(stripped) = p.strip_prefix("//?/") {
        stripped.to_owned()
    } else {
        p
    };
    if p == "/" || (p.len() == 3 && p.as_bytes()[1] == b':') {
        p
    } else {
        p.trim_end_matches('/').to_owned()
    }
}
fn encode(value: &str) -> String {
    let mut out = String::new();
    for c in value.bytes() {
        if c.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&c) {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}
pub fn uri(path: &str) -> String {
    let p = normalize(path);
    if let Some(unc) = p.strip_prefix("//") {
        let (authority, rest) = unc.split_once('/').unwrap_or((unc, ""));
        return format!(
            "file://{authority}/{}",
            rest.split('/').map(encode).collect::<Vec<_>>().join("/")
        );
    }
    let body = p
        .split('/')
        .enumerate()
        .map(|(i, s)| {
            if i == 0 && s.len() == 2 && s.as_bytes()[1] == b':' {
                s.to_owned()
            } else {
                encode(s)
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("file://{}{body}", if p.starts_with('/') { "" } else { "/" })
}
pub fn path(value: &str) -> Result<String> {
    let rest = value
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("file://"))
        .and_then(|_| value.get(7..))
        .ok_or("only file locations are supported")?;
    let (authority, body) = rest.split_once('/').ok_or("invalid file URI")?;
    let body = body.split(['?', '#']).next().unwrap_or_default();
    let bytes = body.as_bytes();
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = body.get(i + 1..i + 3).ok_or("invalid URI escape")?;
            decoded.push(u8::from_str_radix(hex, 16).map_err(|_| "invalid URI escape")?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    let body = String::from_utf8(decoded).map_err(|_| "invalid URI UTF-8")?;
    if !authority.is_empty() && authority != "localhost" {
        Ok(format!("//{authority}/{body}"))
    } else if body.as_bytes().get(1) == Some(&b':') {
        Ok(body)
    } else {
        Ok(format!("/{body}"))
    }
}
pub fn key(path: &str) -> String {
    let value = normalize(path);
    if value.starts_with("//") || value.as_bytes().get(1) == Some(&b':') {
        value.to_lowercase()
    } else {
        value
    }
}
pub fn inside(file: &str, folder: &str) -> bool {
    let folder = key(folder);
    let file = key(file);
    file.starts_with(&if folder.ends_with('/') {
        folder
    } else {
        folder + "/"
    })
}
pub fn markdown(value: &Value) -> Option<String> {
    if let Some(v) = value.as_str() {
        return Some(v.to_owned());
    }
    if let Some(v) = value.as_array() {
        return Some(
            v.iter()
                .filter_map(markdown)
                .collect::<Vec<_>>()
                .join("\n\n"),
        );
    }
    if let Some(language) = value["language"].as_str() {
        return Some(format!(
            "```{language}\n{}\n```",
            value["value"].as_str().unwrap_or_default()
        ));
    }
    optional(&value["value"])
}
#[derive(Clone)]
pub struct Text {
    pub value: Rc<str>,
    pub unit: ColumnUnit,
    lines: Rc<[usize]>,
}
impl Text {
    pub fn new(value: impl Into<Rc<str>>, unit: ColumnUnit) -> Self {
        let value = value.into();
        let lines = std::iter::once(0)
            .chain(
                value
                    .bytes()
                    .enumerate()
                    .filter_map(|(i, c)| (c == b'\n').then_some(i + 1)),
            )
            .collect::<Vec<_>>()
            .into();
        Self { value, unit, lines }
    }
    pub fn position(&self, offset: u64) -> Result<Value> {
        let offset = usize::try_from(offset).map_err(|_| "invalid byte offset")?;
        if offset > self.value.len() || !self.value.is_char_boundary(offset) {
            return Err("invalid byte offset".into());
        }
        let line = self
            .lines
            .partition_point(|&p| p <= offset)
            .saturating_sub(1);
        let part = &self.value[self.lines[line]..offset];
        Ok(
            json!({"line":line, "character":match self.unit { ColumnUnit::Utf8 => part.len(), ColumnUnit::Utf16 => part.encode_utf16().count() }}),
        )
    }
    pub fn offset(&self, position: &Value) -> Result<u64> {
        let line = position["line"].as_u64().ok_or("invalid line")? as usize;
        let column = position["character"].as_u64().ok_or("invalid column")? as usize;
        let Some(&start) = self.lines.get(line) else {
            return Ok(self.value.len() as u64);
        };
        let mut end = self
            .lines
            .get(line + 1)
            .map_or(self.value.len(), |&p| p - 1);
        if self.lines.get(line + 1).is_some()
            && end > start
            && self.value.as_bytes()[end - 1] == b'\r'
        {
            end -= 1;
        }
        let part = &self.value[start..end];
        if self.unit == ColumnUnit::Utf8 {
            let index = column.min(part.len());
            if !part.is_char_boundary(index) {
                return Err("position splits a character".into());
            }
            return Ok((start + index) as u64);
        }
        let mut units = 0;
        for (i, c) in part.char_indices() {
            if units == column {
                return Ok((start + i) as u64);
            }
            units += c.len_utf16();
            if units > column {
                return Err("position splits a surrogate pair".into());
            }
        }
        Ok(end as u64)
    }
    pub fn range(&self, value: &Value) -> Result<ByteRange> {
        let range = ByteRange {
            start: self.offset(&value["start"])?,
            end: self.offset(&value["end"])?,
        };
        if range.start > range.end {
            return Err("inverted range".into());
        }
        Ok(range)
    }
    pub fn edit(&self, value: &Value) -> Result<TextEdit> {
        Ok(TextEdit {
            range: self.range(&value["range"])?,
            text: string(&value["newText"]),
        })
    }
    pub fn file_range(&self, value: &Value) -> FileRange {
        FileRange {
            start: TextPosition {
                line: number(&value["start"]["line"]),
                column: number(&value["start"]["character"]),
            },
            end: TextPosition {
                line: number(&value["end"]["line"]),
                column: number(&value["end"]["character"]),
            },
            unit: self.unit,
        }
    }
    pub fn diff(&self, next: &str) -> Result<Value> {
        let old = self.value.as_bytes();
        let new = next.as_bytes();
        let mut start = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        while !self.value.is_char_boundary(start) || !next.is_char_boundary(start) {
            start -= 1;
        }
        let mut a = old.len();
        let mut b = new.len();
        while a > start && b > start && old[a - 1] == new[b - 1] {
            a -= 1;
            b -= 1;
        }
        while !self.value.is_char_boundary(a) || !next.is_char_boundary(b) {
            a += 1;
            b += 1;
        }
        Ok(
            json!({"range":{"start":self.position(start as u64)?,"end":self.position(a as u64)?},"text":&next[start..b]}),
        )
    }
}
