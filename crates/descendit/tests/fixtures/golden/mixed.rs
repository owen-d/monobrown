// A realistic module with a mix of problems: some duplication, some bloat, medium types.

pub struct Report {
    pub title: String,
    pub entries: Vec<Entry>,
    pub draft: bool,
    pub archived: bool,
    pub pinned: bool,
    pub author: Option<String>,
    pub reviewer: Option<String>,
}

pub struct Entry {
    pub label: String,
    pub value: f64,
}

pub enum Status {
    Pending,
    InProgress { assignee: String, priority: bool },
    Done { verified: bool },
    Cancelled { reason: String },
}

// --- Some bloat ---

/// Build a summary string from report entries (long function).
pub fn build_summary(report: &Report) -> String {
    let mut output = String::new();
    output.push_str("=== ");
    output.push_str(&report.title);
    output.push_str(" ===\n");

    let total: f64 = report.entries.iter().map(|e| e.value).sum();
    let count = report.entries.len();
    let average = if count > 0 { total / count as f64 } else { 0.0 };

    output.push_str(&format!("Total: {total:.2}\n"));
    output.push_str(&format!("Count: {count}\n"));
    output.push_str(&format!("Average: {average:.2}\n"));

    for (i, entry) in report.entries.iter().enumerate() {
        let pct = if total > 0.0 {
            entry.value / total * 100.0
        } else {
            0.0
        };
        output.push_str(&format!("  {}. {} = {:.2} ({:.1}%)\n", i + 1, entry.label, entry.value, pct));
    }

    if report.draft {
        output.push_str("[DRAFT]\n");
    }
    if report.archived {
        output.push_str("[ARCHIVED]\n");
    }
    if let Some(ref author) = report.author {
        output.push_str(&format!("Author: {author}\n"));
    }
    if let Some(ref reviewer) = report.reviewer {
        output.push_str(&format!("Reviewer: {reviewer}\n"));
    }

    output
}

// --- Some duplication ---

pub fn sort_entries_by_value(entries: &[Entry]) -> Vec<&Entry> {
    let mut sorted: Vec<&Entry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal));
    sorted
}

pub fn sort_entries_by_label(entries: &[Entry]) -> Vec<&Entry> {
    let mut sorted: Vec<&Entry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.label.partial_cmp(&b.label).unwrap_or(std::cmp::Ordering::Equal));
    sorted
}

// --- Private helpers adding overhead ---

fn compute_median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

fn format_pct(value: f64, total: f64) -> String {
    if total == 0.0 {
        return "N/A".to_string();
    }
    format!("{:.1}%", value / total * 100.0)
}

fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

fn truncate_label(label: &str, max_len: usize) -> String {
    if label.len() <= max_len {
        label.to_string()
    } else {
        format!("{}...", &label[..max_len.saturating_sub(3)])
    }
}

fn count_above_threshold(entries: &[Entry], threshold: f64) -> usize {
    entries.iter().filter(|e| e.value > threshold).count()
}

fn _unused_helper() -> bool {
    false
}
