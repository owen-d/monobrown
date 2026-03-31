//! Configuration types with reduced state cardinality.
//!
//! Booleans are grouped into focused sub-structs so no single type
//! carries more than a handful of fields.

/// Runtime performance knobs.
#[derive(Clone, Copy, Default)]
pub struct RuntimeBehavior {
    pub verbose: bool,
    pub parallel: bool,
}

/// Safety constraints on the run.
#[derive(Clone, Copy, Default)]
pub struct SafetyConstraints {
    pub dry_run: bool,
    pub strict_mode: bool,
}

/// Controls how the pipeline runs — split into behavior and safety.
#[derive(Clone, Copy, Default)]
pub struct RunMode {
    pub behavior: RuntimeBehavior,
    pub safety: SafetyConstraints,
}

/// Controls data-encoding features (compression, encryption).
#[derive(Clone, Copy, Default)]
pub struct EncodingFeatures {
    pub enable_compression: bool,
    pub enable_encryption: bool,
}

/// Controls data-integrity features (validation, dedup, retry).
#[derive(Clone, Copy, Default)]
pub struct IntegrityFeatures {
    pub skip_validation: bool,
    pub enable_dedup: bool,
    pub retry_on_failure: bool,
}

/// Controls data-processing features — split into encoding and integrity.
#[derive(Clone, Copy, Default)]
pub struct ProcessingFeatures {
    pub encoding: EncodingFeatures,
    pub integrity: IntegrityFeatures,
}

/// Controls output behavior.
#[derive(Clone, Copy, Default)]
pub struct OutputFlags {
    pub force_overwrite: bool,
    pub legacy_format: bool,
    pub emit_metrics: bool,
}

/// Pipeline configuration — booleans split into focused sub-structs.
pub struct PipelineConfig {
    pub input_path: String,
    pub output_path: String,
    pub run_mode: RunMode,
    pub processing: ProcessingFeatures,
    pub output_flags: OutputFlags,
    pub max_retries: Option<u32>,
    pub batch_size: Option<usize>,
    pub timeout_secs: Option<u64>,
}

/// Structural output includes (framing around data).
#[derive(Clone, Copy)]
pub struct StructuralIncludes {
    pub headers: bool,
    pub line_numbers: bool,
}

/// Metadata output includes (provenance info).
#[derive(Clone, Copy)]
pub struct MetadataIncludes {
    pub timestamps: bool,
    pub source: bool,
}

/// What to include in output formatting — split into structural and metadata.
#[derive(Clone, Copy)]
pub struct IncludeFields {
    pub structural: StructuralIncludes,
    pub metadata: MetadataIncludes,
}

/// Post-processing steps applied to output.
#[derive(Clone, Copy, Default)]
pub struct PostProcessing {
    pub sort_output: bool,
    pub deduplicate: bool,
}

/// Output format configuration — split from one big struct.
pub struct OutputConfig {
    pub pretty_print: bool,
    pub include: IncludeFields,
    pub colorize: bool,
    pub truncate_long_lines: bool,
    pub post_processing: PostProcessing,
}

/// Text-encoding content checks.
#[derive(Copy, Clone)]
pub struct TextChecks {
    pub utf8: bool,
    pub empty_fields: bool,
}

/// Data-value content checks.
#[derive(Copy, Clone)]
pub struct ValueChecks {
    pub duplicates: bool,
    pub numeric_ranges: bool,
}

/// Content-level validation — split into text and value checks.
#[derive(Copy, Clone)]
pub struct ContentChecks {
    pub text: TextChecks,
    pub values: ValueChecks,
}

/// Format-level validation (string shape).
#[derive(Copy, Clone, Default)]
pub struct FormatChecks {
    pub date_formats: bool,
    pub email_format: bool,
    pub url_format: bool,
}

/// What the validator tolerates.
#[derive(Copy, Clone, Default)]
pub struct ValidationTolerance {
    pub allow_nulls: bool,
    pub allow_empty_strings: bool,
    pub case_sensitive: bool,
}

/// Validation rules — composed from content checks, format checks, and tolerance.
#[derive(Copy, Clone)]
pub struct ValidationRules {
    pub content: ContentChecks,
    pub format: FormatChecks,
    pub tolerance: ValidationTolerance,
}

impl PipelineConfig {
    pub fn new(input_path: String, output_path: String) -> Self {
        Self {
            input_path,
            output_path,
            run_mode: RunMode::default(),
            processing: ProcessingFeatures::default(),
            output_flags: OutputFlags::default(),
            max_retries: None,
            batch_size: None,
            timeout_secs: None,
        }
    }
}

impl Default for TextChecks {
    fn default() -> Self {
        Self {
            utf8: true,
            empty_fields: true,
        }
    }
}

impl Default for ValueChecks {
    fn default() -> Self {
        Self {
            duplicates: true,
            numeric_ranges: false,
        }
    }
}

impl Default for ContentChecks {
    fn default() -> Self {
        Self {
            text: TextChecks::default(),
            values: ValueChecks::default(),
        }
    }
}

impl Default for ValidationRules {
    fn default() -> Self {
        Self {
            content: ContentChecks::default(),
            format: FormatChecks::default(),
            tolerance: ValidationTolerance::default(),
        }
    }
}

impl Default for StructuralIncludes {
    fn default() -> Self {
        Self {
            headers: true,
            line_numbers: false,
        }
    }
}

impl Default for MetadataIncludes {
    fn default() -> Self {
        Self {
            timestamps: false,
            source: false,
        }
    }
}

impl Default for IncludeFields {
    fn default() -> Self {
        Self {
            structural: StructuralIncludes::default(),
            metadata: MetadataIncludes::default(),
        }
    }
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            pretty_print: true,
            include: IncludeFields::default(),
            colorize: true,
            truncate_long_lines: false,
            post_processing: PostProcessing::default(),
        }
    }
}
