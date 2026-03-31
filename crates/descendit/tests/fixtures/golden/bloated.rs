// Functions that are excessively long, triggering the bloat dimension.

pub struct Record {
    pub id: u64,
    pub name: String,
    pub score: f64,
    pub active: bool,
}

pub struct Summary {
    pub total: u64,
    pub label: String,
}

/// A massively long function that processes records through many stages.
pub fn process_records(records: &[Record]) -> Vec<Summary> {
    let mut results = Vec::new();
    let mut running_total: u64 = 0;
    let mut max_score: f64 = 0.0;
    let mut active_count: u64 = 0;
    let mut inactive_names: Vec<String> = Vec::new();

    // Stage 1: initial scan
    for record in records {
        if record.active {
            active_count += 1;
            if record.score > max_score {
                max_score = record.score;
            }
        } else {
            inactive_names.push(record.name.clone());
        }
        running_total += record.id;
    }

    // Stage 2: bucketing by score
    let mut low_bucket = Vec::new();
    let mut mid_bucket = Vec::new();
    let mut high_bucket = Vec::new();
    for record in records {
        match record.score as u64 {
            0..=30 => low_bucket.push(record),
            31..=70 => mid_bucket.push(record),
            _ => high_bucket.push(record),
        }
    }

    // Stage 3: compute summaries per bucket
    let low_total: u64 = low_bucket.iter().map(|r| r.id).sum();
    let mid_total: u64 = mid_bucket.iter().map(|r| r.id).sum();
    let high_total: u64 = high_bucket.iter().map(|r| r.id).sum();

    results.push(Summary {
        total: low_total,
        label: format!("low ({} items)", low_bucket.len()),
    });
    results.push(Summary {
        total: mid_total,
        label: format!("mid ({} items)", mid_bucket.len()),
    });
    results.push(Summary {
        total: high_total,
        label: format!("high ({} items)", high_bucket.len()),
    });

    // Stage 4: adjust for inactive records
    if !inactive_names.is_empty() {
        let penalty = inactive_names.len() as u64;
        for summary in &mut results {
            if summary.total > penalty {
                summary.total -= penalty;
            }
        }
    }

    // Stage 5: final adjustment
    if active_count > 0 {
        let average = running_total / active_count;
        results.push(Summary {
            total: average,
            label: format!("average (max_score={max_score:.2})"),
        });
    }

    results
}

/// Another bloated function that validates and transforms configuration entries.
pub fn validate_config(entries: &[(String, String)]) -> Result<Vec<(String, u64)>, String> {
    let mut validated = Vec::new();
    let mut errors = Vec::new();
    let mut seen_keys: Vec<String> = Vec::new();
    let mut duplicate_count: u64 = 0;

    for (key, value) in entries {
        // Check for duplicates
        if seen_keys.contains(key) {
            duplicate_count += 1;
            errors.push(format!("duplicate key: {key}"));
            continue;
        }
        seen_keys.push(key.clone());

        // Validate key format
        if key.is_empty() {
            errors.push("empty key found".to_string());
            continue;
        }
        if key.starts_with('_') {
            errors.push(format!("key must not start with underscore: {key}"));
            continue;
        }
        if key.len() > 64 {
            errors.push(format!("key too long: {key}"));
            continue;
        }

        // Parse value
        let parsed: u64 = match value.parse() {
            Ok(v) => v,
            Err(_) => {
                errors.push(format!("invalid integer value for key {key}: {value}"));
                continue;
            }
        };

        // Range checks
        if parsed == 0 {
            errors.push(format!("zero value not allowed for key {key}"));
            continue;
        }
        if parsed > 1_000_000 {
            errors.push(format!("value too large for key {key}: {parsed}"));
            continue;
        }

        validated.push((key.clone(), parsed));
    }

    if duplicate_count > 3 {
        return Err(format!(
            "too many duplicates ({duplicate_count}); aborting: {}",
            errors.join(", ")
        ));
    }

    if errors.is_empty() {
        Ok(validated)
    } else {
        Err(errors.join("; "))
    }
}

/// A third bloated function that renders a report from summaries.
pub fn render_report(summaries: &[Summary], title: &str, verbose: bool) -> String {
    let mut output = String::new();
    let separator = "=".repeat(title.len() + 4);

    output.push_str(&separator);
    output.push('\n');
    output.push_str("  ");
    output.push_str(title);
    output.push_str("  \n");
    output.push_str(&separator);
    output.push('\n');
    output.push('\n');

    let grand_total: u64 = summaries.iter().map(|s| s.total).sum();

    for (index, summary) in summaries.iter().enumerate() {
        let pct = if grand_total > 0 {
            (summary.total as f64 / grand_total as f64) * 100.0
        } else {
            0.0
        };

        output.push_str(&format!(
            "  {}. {} -- total: {} ({:.1}%)\n",
            index + 1,
            summary.label,
            summary.total,
            pct,
        ));

        if verbose {
            let bar_width = (pct / 2.0) as usize;
            let bar: String = "#".repeat(bar_width);
            output.push_str(&format!("     [{bar:<50}]\n"));
        }
    }

    output.push('\n');
    output.push_str(&format!("  Grand total: {grand_total}\n"));

    if grand_total == 0 {
        output.push_str("  WARNING: no data\n");
    } else if grand_total > 1_000_000 {
        output.push_str("  NOTE: large dataset\n");
    }

    output.push_str(&separator);
    output.push('\n');

    output
}
