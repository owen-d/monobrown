//! Data transformation — shared escape + rendering logic.

use crate::ingest::Record;

/// Escape a value for CSV (RFC 4180).
pub fn escape_csv(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Escape a value for JSON strings.
pub fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// Escape a value for TSV.
pub fn escape_tsv(s: &str) -> String {
    s.replace('\t', "\\t").replace('\n', "\\n")
}

/// Render records as delimited text (CSV or TSV).
///
/// Shared implementation: `separator` is `,` for CSV or `\t` for TSV,
/// `escape_fn` is the appropriate escaper.
pub fn render_delimited(
    records: &[Record],
    include_headers: bool,
    separator: char,
    escape_fn: fn(&str) -> String,
) -> String {
    let mut output = String::new();
    let sep = &separator.to_string();

    if include_headers {
        output.push_str(&["id", "fields", "source", "line"].join(sep));
        output.push('\n');
    }

    for record in records {
        let fields: Vec<String> = record.fields.iter().map(|f| escape_fn(f)).collect();
        output.push_str(&format!(
            "{}{}{}{}{}{}{}\n",
            escape_fn(&record.id), separator,
            fields.join(";"), separator,
            escape_fn(&record.source), separator,
            record.line_number,
        ));
    }
    output
}

/// Transform records to CSV format.
pub fn transform_to_csv(records: &[Record], include_headers: bool) -> String {
    render_delimited(records, include_headers, ',', escape_csv)
}

/// Transform records to TSV format.
pub fn transform_to_tsv(records: &[Record], include_headers: bool) -> String {
    render_delimited(records, include_headers, '\t', escape_tsv)
}

/// Transform records to JSON format.
pub fn transform_to_json(records: &[Record], pretty: bool) -> String {
    let mut output = String::from("[\n");
    for (i, record) in records.iter().enumerate() {
        let fields: Vec<String> = record.fields.iter().map(|f| format!("\"{}\"", escape_json(f))).collect();
        if pretty {
            output.push_str(&format!(
                "  {{\n    \"id\": \"{}\",\n    \"fields\": [{}],\n    \"source\": \"{}\",\n    \"line\": {}\n  }}",
                escape_json(&record.id), fields.join(", "), escape_json(&record.source), record.line_number,
            ));
        } else {
            output.push_str(&format!(
                "{{\"id\":\"{}\",\"fields\":[{}],\"source\":\"{}\",\"line\":{}}}",
                escape_json(&record.id), fields.join(","), escape_json(&record.source), record.line_number,
            ));
        }
        if i < records.len() - 1 { output.push_str(",\n"); } else { output.push('\n'); }
    }
    output.push(']');
    output
}
