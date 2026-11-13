//! Parse only URI and coordinates here. Provider resolution supplies resource
//! identity; the local-path results::location adapter is deliberately unused.
use super::*;
use api::session::{ColumnEncoding, ResourcePosition, ResourceRange};

pub fn parse(value: &Value, unit: ColumnUnit) -> Result<Vec<(String, ResourceRange)>, String> {
    let values = match value {
        Value::Null => return Ok(vec![]),
        Value::Array(values) if values.len() <= 256 => values.as_slice(),
        Value::Object(_) => std::slice::from_ref(value),
        _ => return Err("Invalid or excessive language locations".into()),
    };
    fn position(value: &Value) -> Result<ResourcePosition, String> {
        Ok(ResourcePosition {
            line: value["line"]
                .as_u64()
                .and_then(|v| v.try_into().ok())
                .ok_or("Invalid location line")?,
            character: value["character"]
                .as_u64()
                .and_then(|v| v.try_into().ok())
                .ok_or("Invalid location column")?,
        })
    }
    values
        .iter()
        .map(|value| {
            let uri = value
                .get("targetUri")
                .or(value.get("uri"))
                .and_then(Value::as_str)
                .ok_or("Missing location URI")?;
            let range = value
                .get("targetSelectionRange")
                .or(value.get("targetRange"))
                .or(value.get("range"))
                .ok_or("Missing location range")?;
            let range = ResourceRange {
                start: position(&range["start"])?,
                end: position(&range["end"])?,
                encoding: match unit {
                    ColumnUnit::Utf8 => ColumnEncoding::Utf8,
                    ColumnUnit::Utf16 => ColumnEncoding::Utf16,
                },
            };
            if range.start > range.end {
                return Err("Reversed location range".into());
            }
            Ok((uri.to_owned(), range))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unopened_location_links_keep_uri_and_server_units_without_native_path_decoding() {
        let value = json!({"targetUri":"file:///work/a%5C%E4%B8%AD.rs", "targetRange":{
            "start":{"line":1,"character":0},"end":{"line":8,"character":0}},
            "targetSelectionRange":{"start":{"line":2,"character":3},"end":{"line":2,"character":8}}});
        let parsed = parse(&value, ColumnUnit::Utf16).unwrap();
        assert_eq!(parsed[0].0, "file:///work/a%5C%E4%B8%AD.rs");
        assert_eq!(parsed[0].1.start.line, 2);
        assert_eq!(parsed[0].1.encoding, ColumnEncoding::Utf16);
        assert!(parse(&json!(vec![value; 257]), ColumnUnit::Utf8).is_err());
        assert!(parse(&json!({"uri":"file:///x","range":{"start":{"line":0,"character":4294967296u64},"end":{"line":1,"character":0}}}), ColumnUnit::Utf8).is_err());
    }
}
