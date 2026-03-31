// Types with high field cardinality, triggering the state_cardinality dimension.

/// A configuration type with many boolean/optional fields producing a massive state space.
pub struct AppConfig {
    pub debug_mode: bool,
    pub verbose: bool,
    pub dry_run: bool,
    pub force: bool,
    pub quiet: bool,
    pub recursive: bool,
    pub follow_symlinks: bool,
    pub preserve_permissions: bool,
    pub overwrite_existing: bool,
    pub create_backup: bool,
    pub strict_mode: bool,
    pub experimental: bool,
    pub max_retries: Option<u32>,
    pub timeout: Option<u64>,
    pub output_path: Option<String>,
}

/// A user profile with many optional fields.
pub struct UserProfile {
    pub username: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub bio: Option<String>,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
    pub is_verified: bool,
    pub is_suspended: bool,
    pub newsletter_opt_in: bool,
    pub two_factor_enabled: bool,
    pub dark_mode: bool,
    pub show_email: bool,
}

/// An event with many variant-specific data.
pub enum SystemEvent {
    UserCreated { id: u64, admin: bool, verified: bool },
    UserDeleted { id: u64, soft_delete: bool },
    UserUpdated { id: u64, field: String, admin: bool },
    LoginAttempt { user_id: u64, success: bool, two_factor: bool },
    PermissionChanged { user_id: u64, granted: bool, scope: String },
    ConfigUpdated { key: String, rollback: bool },
    SystemAlert { severity: u8, acknowledged: bool },
    BatchJob { id: u64, parallel: bool, dry_run: bool, retry: bool },
    AuditEntry { actor_id: u64, action: String, success: bool, admin: bool },
    Notification { user_id: u64, read: bool, urgent: bool, channel: String },
}

/// A pipeline stage descriptor.
pub enum PipelineStage {
    Parse { strict: bool, lenient: bool },
    Validate { schema: String, fail_fast: bool },
    Transform { in_place: bool, parallel: bool },
    Filter { inverted: bool, case_sensitive: bool },
    Aggregate { windowed: bool, distinct: bool },
    Emit { buffered: bool, compressed: bool },
    Checkpoint { durable: bool, async_write: bool },
    Finalize { cleanup: bool, notify: bool },
}

/// Create a default app configuration.
pub fn default_config() -> AppConfig {
    AppConfig {
        debug_mode: false,
        verbose: false,
        dry_run: false,
        force: false,
        quiet: false,
        recursive: false,
        follow_symlinks: false,
        preserve_permissions: true,
        overwrite_existing: false,
        create_backup: true,
        strict_mode: false,
        experimental: false,
        max_retries: None,
        timeout: None,
        output_path: None,
    }
}

/// Create a default user profile.
pub fn default_profile(username: &str) -> UserProfile {
    UserProfile {
        username: username.to_string(),
        display_name: None,
        email: None,
        bio: None,
        avatar_url: None,
        is_admin: false,
        is_verified: false,
        is_suspended: false,
        newsletter_opt_in: false,
        two_factor_enabled: false,
        dark_mode: false,
        show_email: false,
    }
}
