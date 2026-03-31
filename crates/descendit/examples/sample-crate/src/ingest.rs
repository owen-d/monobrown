//! Data ingestion — split into focused functions.

use crate::config::{PipelineConfig, ValidationRules};

/// A single record from the input source.
pub struct Record {
    pub id: String,
    pub fields: Vec<String>,
    pub source: String,
    pub line_number: usize,
}

/// Result of ingesting a batch.
pub struct IngestResult {
    pub records: Vec<Record>,
    pub skipped: usize,
    pub errors: Vec<String>,
    pub total_bytes: usize,
}

/// Read raw input from disk.
pub fn read_input(path: &str, verbose: bool) -> Result<String, String> {
    let raw =
        std::fs::read_to_string(path).map_err(|e| format!("failed to read {}: {}", path, e))?;
    if verbose {
        eprintln!("Read {} bytes from {}", raw.len(), path);
    }
    Ok(raw)
}

/// Validate a single record against rules.
pub fn validate_record(record: &Record, rules: &ValidationRules) -> Vec<String> {
    let mut errors = Vec::new();

    if rules.content.text.utf8 {
        for field in &record.fields {
            if field.contains('\u{FFFD}') {
                errors.push(format!("record {}: invalid UTF-8 in field", record.id));
            }
        }
    }

    if rules.content.text.empty_fields {
        for (j, field) in record.fields.iter().enumerate() {
            if field.is_empty() && !rules.tolerance.allow_nulls {
                errors.push(format!("record {}: empty field at column {}", record.id, j));
            }
        }
    }

    if rules.content.values.numeric_ranges {
        for (j, field) in record.fields.iter().enumerate() {
            if let Ok(n) = field.parse::<f64>() {
                if n.is_nan() || n.is_infinite() {
                    errors.push(format!(
                        "record {}: invalid number at column {}",
                        record.id, j
                    ));
                }
            }
        }
    }

    errors
}

/// Parse a single line into a Record, returning None if skipped.
pub fn parse_line(
    line: &str,
    line_num: usize,
    source: &str,
    rules: &ValidationRules,
    strict: bool,
    records: &[Record],
    dedup: bool,
) -> Result<Option<Record>, String> {
    let trimmed = line.trim();

    if trimmed.is_empty() && !rules.tolerance.allow_empty_strings {
        return Ok(None);
    }

    let fields: Vec<String> = trimmed.split(',').map(|s| s.trim().to_string()).collect();
    if fields.is_empty() {
        return Ok(None);
    }

    // Validate the parsed fields
    let temp = Record {
        id: fields[0].clone(),
        fields: fields.clone(),
        source: source.to_string(),
        line_number: line_num,
    };
    let errs = validate_record(&temp, rules);
    if !errs.is_empty() && strict {
        return Err(errs[0].clone());
    }

    // Duplicate check
    if rules.content.values.duplicates {
        let id = &fields[0];
        if records.iter().any(|r: &Record| r.id == *id) {
            if dedup {
                return Ok(None);
            } else if strict {
                return Err(format!(
                    "strict mode: duplicate id '{}' at line {}",
                    id, line_num
                ));
            }
        }
    }

    Ok(Some(temp))
}

/// Remove duplicate records by id, returning the number removed.
pub fn dedup_records(records: &mut Vec<Record>) -> usize {
    let before = records.len();
    let mut seen = std::collections::HashSet::new();
    records.retain(|r| seen.insert(r.id.clone()));
    before - records.len()
}

/// Parse all data lines into records, collecting errors.
pub fn parse_all_lines(
    lines: &[&str],
    start: usize,
    source: &str,
    rules: &ValidationRules,
    strict: bool,
    dedup: bool,
) -> Result<(Vec<Record>, usize, Vec<String>), String> {
    let mut records = Vec::new();
    let mut skipped = 0usize;
    let mut errors = Vec::new();

    for (i, line) in lines.iter().enumerate().skip(start) {
        match parse_line(line, i + 1, source, rules, strict, &records, dedup) {
            Ok(Some(rec)) => records.push(rec),
            Ok(None) => skipped += 1,
            Err(e) => {
                if strict {
                    return Err(e);
                }
                errors.push(e);
            }
        }
    }

    Ok((records, skipped, errors))
}

/// Read, validate, and prepare records from input.
pub fn ingest_records(
    config: &PipelineConfig,
    rules: &ValidationRules,
) -> Result<IngestResult, String> {
    let raw_input = read_input(&config.input_path, config.run_mode.behavior.verbose)?;
    let total_bytes = raw_input.len();

    let lines: Vec<&str> = raw_input.lines().collect();
    if lines.is_empty() {
        return Err("input file is empty".to_string());
    }

    let start = if lines[0].starts_with('#') { 1 } else { 0 };
    let (mut records, mut skipped, errors) = parse_all_lines(
        &lines,
        start,
        &config.input_path,
        rules,
        config.run_mode.safety.strict_mode,
        config.processing.integrity.enable_dedup,
    )?;

    if config.processing.integrity.enable_dedup {
        skipped += dedup_records(&mut records);
    }

    if config.run_mode.behavior.verbose {
        eprintln!(
            "Ingested {} records, skipped {}, {} errors",
            records.len(),
            skipped,
            errors.len()
        );
    }

    Ok(IngestResult {
        records,
        skipped,
        errors,
        total_bytes,
    })
}
