//! Output writing — refactored into smaller functions.

use crate::config::{OutputConfig, PipelineConfig};
use crate::ingest::Record;
use crate::transform;

/// Prepare the final record list (sort + dedup).
pub fn prepare_records<'a>(records: &'a [Record], config: &OutputConfig) -> Vec<&'a Record> {
    let mut result: Vec<&Record> = records.iter().collect();

    if config.post_processing.sort_output {
        result.sort_by(|a, b| a.id.cmp(&b.id));
    }

    if config.post_processing.deduplicate {
        let mut seen = std::collections::HashSet::new();
        result.retain(|r| seen.insert(&r.id));
    }

    result
}

/// Render records into the target format string.
pub fn render(records: &[Record], format: &str, config: &OutputConfig) -> String {
    let content = match format {
        "json" => transform::transform_to_json(records, config.pretty_print),
        "tsv" => transform::transform_to_tsv(records, config.include.structural.headers),
        _ => transform::transform_to_csv(records, config.include.structural.headers),
    };

    if config.truncate_long_lines {
        content
            .lines()
            .map(|line| if line.len() > 200 { format!("{}...", &line[..197]) } else { line.to_string() })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        content
    }
}

/// Detect output format from file extension.
pub fn detect_format(path: &str) -> &'static str {
    if path.ends_with(".json") {
        "json"
    } else if path.ends_with(".tsv") {
        "tsv"
    } else {
        "csv"
    }
}

/// Write records to output.
pub fn write_output(
    records: &[Record],
    pipeline_config: &PipelineConfig,
    output_config: &OutputConfig,
) -> Result<(), String> {
    if records.is_empty() {
        return Ok(());
    }

    let path = &pipeline_config.output_path;
    let format = detect_format(path);

    if !pipeline_config.output_flags.force_overwrite && std::path::Path::new(path).exists() {
        return Err(format!("output file {} already exists (use force_overwrite)", path));
    }

    let final_refs = prepare_records(records, output_config);
    let owned: Vec<Record> = final_refs
        .iter()
        .map(|r| Record {
            id: r.id.clone(),
            fields: r.fields.clone(),
            source: r.source.clone(),
            line_number: r.line_number,
        })
        .collect();

    let content = render(&owned, format, output_config);

    if pipeline_config.run_mode.safety.dry_run {
        println!("{}", content);
        return Ok(());
    }

    std::fs::write(path, &content)
        .map_err(|e| format!("failed to write {}: {}", path, e))?;

    if pipeline_config.output_flags.emit_metrics {
        eprintln!(
            "metrics: records={} bytes={} format={}",
            owned.len(), content.len(), format,
        );
    }

    Ok(())
}
