// A well-structured module: small public functions, no duplication, simple types.

/// Configuration for a build pipeline.
pub struct PipelineConfig {
    pub name: String,
    pub parallel: bool,
}

/// A single build step result.
pub enum StepResult {
    Success,
    Skipped,
    Failed(String),
}

/// Create a new pipeline configuration with the given name.
pub fn new_pipeline(name: &str) -> PipelineConfig {
    PipelineConfig {
        name: name.to_string(),
        parallel: false,
    }
}

/// Enable parallel execution for the pipeline.
pub fn with_parallel(mut config: PipelineConfig) -> PipelineConfig {
    config.parallel = true;
    config
}

/// Run a single step and report the outcome.
pub fn run_step(name: &str, enabled: bool) -> StepResult {
    if !enabled {
        return StepResult::Skipped;
    }
    if name.is_empty() {
        return StepResult::Failed("step name must not be empty".into());
    }
    StepResult::Success
}

/// Format a step result for display.
pub fn format_result(result: &StepResult) -> String {
    match result {
        StepResult::Success => "ok".into(),
        StepResult::Skipped => "skipped".into(),
        StepResult::Failed(msg) => format!("FAILED: {msg}"),
    }
}
