//! Minimal, dependency-free JSON and CSV field emission. `drmd` never needs
//! to *parse* JSON (the wire protocol in [`crate::serve`] is a simple
//! tab-separated line format), only to emit it reliably, so a full `serde`
//! dependency isn't pulled in for that alone.

pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

pub fn json_string(s: &str) -> String {
    format!("\"{}\"", json_escape(s))
}

pub fn json_string_array(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| json_string(s)).collect();
    format!("[{}]", inner.join(","))
}

pub fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn csv_line(fields: &[String]) -> String {
    fields.iter().map(|f| csv_escape(f)).collect::<Vec<_>>().join(",")
}
