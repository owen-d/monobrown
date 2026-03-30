//! Generic modal input dispatch for TUI applications.
//!
//! Decouples key events from semantic actions, enabling vim-like modal editing
//! without coupling to any specific application's types. Applications implement
//! [`ModalInput`] to define their contexts, bindings, and intent types. Apps
//! that also want grouped help panes implement [`ModalHelp`] to enumerate the
//! contexts that should appear in reference UIs. The library provides
//! [`resolve`], [`hints`], and [`all_hints`] as the resolution and display
//! extraction loops.
//!
//! Key bindings carry both display metadata (for footer/help rendering) and
//! matching logic (for intent resolution). This single source of truth makes
//! drift between displayed hints and actual bindings impossible.

use crossterm::event::KeyEvent;

/// A key binding that maps key events to intents with display metadata.
///
/// Generic over:
/// - `I`: the intent (semantic action) type
/// - `A`: the app state type (for guard evaluation)
pub struct KeyBinding<I, A> {
    /// Display label for the key (e.g. "q", "Ctrl+h"). Empty = hidden from hints.
    pub key_label: &'static str,
    /// Short action name for footer display.
    pub action: &'static str,
    /// Longer description for help overlays.
    pub description: &'static str,
    /// Resolve a key event to an intent. Returns None if no match.
    pub resolve: fn(&KeyEvent) -> Option<I>,
    /// Guard: binding only active when this returns true. None = always active.
    pub guard: Option<fn(&A) -> bool>,
}

/// Display metadata extracted from a key binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyHint {
    pub key: &'static str,
    pub action: &'static str,
    pub description: &'static str,
}

/// A labeled modal context that should appear in grouped help output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpContext<C> {
    pub title: &'static str,
    pub context: C,
}

/// One grouped section of help content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeySection {
    pub title: &'static str,
    pub hints: Vec<HotkeyHint>,
}

/// Trait for types that implement modal input dispatch.
///
/// The implementer defines contexts, bindings per context, and how to
/// determine the active context from app state. The library provides
/// the resolution loop via [`resolve`] and hint extraction via [`hints`].
pub trait ModalInput {
    /// The semantic action type.
    type Intent;
    /// The application state type.
    type App;
    /// The input context type (e.g. an enum of modes/overlays).
    type Context;

    /// Determine the active input context from app state.
    fn active_context(app: &Self::App) -> Self::Context;

    /// Return bindings for the given context.
    ///
    /// Returns `Vec` because bindings are constructed per-call (they contain
    /// `fn` pointers that are static, but the collection itself is assembled
    /// from per-context helper functions). This is the pragmatic choice;
    /// optimize with `LazyLock` if profiling shows it matters.
    fn bindings(ctx: &Self::Context) -> Vec<KeyBinding<Self::Intent, Self::App>>;
}

/// Optional extension trait for modals that can generate grouped help output.
///
/// This is intentionally separate from [`ModalInput`] so lightweight users can
/// keep implementing current-context dispatch only.
pub trait ModalHelp: ModalInput {
    /// Representative contexts that should appear in grouped help output.
    ///
    /// Runtime guards are intentionally not evaluated when building
    /// [`all_hints`]; the grouped help pane acts as a reference card showing
    /// everything the modal knows how to expose.
    fn help_contexts() -> Vec<HelpContext<Self::Context>>;
}

/// Resolve a key event to an intent using the modal input system.
///
/// Determines the active context, gets bindings, and returns the first match
/// (skipping guarded bindings whose guard returns false).
pub fn resolve<M: ModalInput>(app: &M::App, key: &KeyEvent) -> Option<M::Intent> {
    let ctx = M::active_context(app);
    let bindings = M::bindings(&ctx);
    debug_assert!(
        !bindings.is_empty(),
        "every context must have at least one binding"
    );
    for binding in &bindings {
        if let Some(guard) = binding.guard
            && !guard(app)
        {
            continue;
        }
        if let Some(intent) = (binding.resolve)(key) {
            return Some(intent);
        }
    }
    None
}

fn collect_hints<I, A>(bindings: Vec<KeyBinding<I, A>>, app: Option<&A>) -> Vec<HotkeyHint> {
    bindings
        .into_iter()
        .filter(|binding| !binding.key_label.is_empty())
        .filter(|binding| match app {
            Some(app) => binding.guard.is_none_or(|guard| guard(app)),
            None => true,
        })
        .map(|binding| HotkeyHint {
            key: binding.key_label,
            action: binding.action,
            description: binding.description,
        })
        .collect()
}

/// Extract visible hotkey hints for an explicit context.
///
/// Returns hints for bindings that have a non-empty `key_label` and whose
/// guard (if any) passes for the provided app state.
pub fn hints_for_context<M: ModalInput>(app: &M::App, ctx: &M::Context) -> Vec<HotkeyHint> {
    collect_hints(M::bindings(ctx), Some(app))
}

/// Extract visible hotkey hints for the current context.
///
/// Returns hints for bindings that have a non-empty `key_label` and whose
/// guard (if any) passes for the current app state.
pub fn hints<M: ModalInput>(app: &M::App) -> Vec<HotkeyHint> {
    let ctx = M::active_context(app);
    hints_for_context::<M>(app, &ctx)
}

/// Extract grouped help sections across all enumerated contexts.
///
/// Unlike [`hints`], this does not evaluate runtime guards. The result is
/// intended for help panes and reference cards that show the full surface area
/// of the modal system.
pub fn all_hints<M: ModalHelp>() -> Vec<HotkeySection> {
    M::help_contexts()
        .into_iter()
        .map(|help_ctx| HotkeySection {
            title: help_ctx.title,
            hints: collect_hints(M::bindings(&help_ctx.context), None),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    // -- Test domain types --

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestContext {
        A,
        B,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestIntent {
        Quit,
        Action1,
        Action2,
    }

    struct TestApp {
        context_b_active: bool,
    }

    struct TestModal;

    impl ModalInput for TestModal {
        type Intent = TestIntent;
        type App = TestApp;
        type Context = TestContext;

        fn active_context(app: &TestApp) -> TestContext {
            if app.context_b_active {
                TestContext::B
            } else {
                TestContext::A
            }
        }

        fn bindings(ctx: &TestContext) -> Vec<KeyBinding<TestIntent, TestApp>> {
            match ctx {
                TestContext::A => context_a_bindings(),
                TestContext::B => context_b_bindings(),
            }
        }
    }

    impl ModalHelp for TestModal {
        fn help_contexts() -> Vec<HelpContext<TestContext>> {
            vec![
                HelpContext {
                    title: "Context A",
                    context: TestContext::A,
                },
                HelpContext {
                    title: "Context B",
                    context: TestContext::B,
                },
            ]
        }
    }

    fn context_a_bindings() -> Vec<KeyBinding<TestIntent, TestApp>> {
        vec![
            KeyBinding {
                key_label: "q",
                action: "quit",
                description: "Quit the application",
                resolve: |key| matches!(key.code, KeyCode::Char('q')).then_some(TestIntent::Quit),
                guard: None,
            },
            KeyBinding {
                key_label: "a",
                action: "action1",
                description: "Perform action 1",
                resolve: |key| {
                    matches!(key.code, KeyCode::Char('a')).then_some(TestIntent::Action1)
                },
                guard: None,
            },
            // Hidden binding (empty key_label).
            KeyBinding {
                key_label: "",
                action: "",
                description: "",
                resolve: |key| {
                    (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                    .then_some(TestIntent::Quit)
                },
                guard: None,
            },
        ]
    }

    fn context_b_bindings() -> Vec<KeyBinding<TestIntent, TestApp>> {
        vec![
            KeyBinding {
                key_label: "Esc",
                action: "back",
                description: "Go back",
                resolve: |key| matches!(key.code, KeyCode::Esc).then_some(TestIntent::Quit),
                guard: None,
            },
            // Guarded binding: only active when context_b_active is true
            // (which it always is when we're in context B, but we use a
            // secondary check here to test the guard mechanism).
            KeyBinding {
                key_label: "x",
                action: "action2",
                description: "Perform action 2 (guarded)",
                resolve: |key| {
                    matches!(key.code, KeyCode::Char('x')).then_some(TestIntent::Action2)
                },
                guard: Some(|app| app.context_b_active),
            },
        ]
    }

    // -- Helpers --

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_with(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn app_context_a() -> TestApp {
        TestApp {
            context_b_active: false,
        }
    }

    fn app_context_b() -> TestApp {
        TestApp {
            context_b_active: true,
        }
    }

    // -- resolve tests --

    #[test]
    fn resolve_context_a_matches_a_bindings() {
        let app = app_context_a();
        assert_eq!(
            resolve::<TestModal>(&app, &key(KeyCode::Char('q'))),
            Some(TestIntent::Quit),
        );
        assert_eq!(
            resolve::<TestModal>(&app, &key(KeyCode::Char('a'))),
            Some(TestIntent::Action1),
        );
    }

    #[test]
    fn resolve_context_a_does_not_match_b_bindings() {
        let app = app_context_a();
        // Esc is a context B binding, should not resolve in context A.
        assert_eq!(resolve::<TestModal>(&app, &key(KeyCode::Esc)), None);
        // 'x' is a context B binding.
        assert_eq!(resolve::<TestModal>(&app, &key(KeyCode::Char('x'))), None,);
    }

    #[test]
    fn resolve_context_b_matches_b_bindings() {
        let app = app_context_b();
        assert_eq!(
            resolve::<TestModal>(&app, &key(KeyCode::Esc)),
            Some(TestIntent::Quit),
        );
        assert_eq!(
            resolve::<TestModal>(&app, &key(KeyCode::Char('x'))),
            Some(TestIntent::Action2),
        );
    }

    #[test]
    fn resolve_context_b_does_not_match_a_bindings() {
        let app = app_context_b();
        // 'q' is a context A binding, should not resolve in context B.
        assert_eq!(resolve::<TestModal>(&app, &key(KeyCode::Char('q'))), None,);
    }

    #[test]
    fn resolve_hidden_binding_matches() {
        // Ctrl+c is a hidden binding in context A (empty key_label).
        let app = app_context_a();
        assert_eq!(
            resolve::<TestModal>(&app, &key_with(KeyCode::Char('c'), KeyModifiers::CONTROL),),
            Some(TestIntent::Quit),
        );
    }

    #[test]
    fn resolve_unbound_key_returns_none() {
        let app = app_context_a();
        assert_eq!(resolve::<TestModal>(&app, &key(KeyCode::Char('z'))), None,);
    }

    // -- Guard tests --

    /// A modal system where the guard rejects the binding even though
    /// the context is active.
    struct GuardTestModal;

    impl ModalInput for GuardTestModal {
        type Intent = TestIntent;
        type App = TestApp;
        type Context = TestContext;

        fn active_context(_app: &TestApp) -> TestContext {
            // Always context A for this test.
            TestContext::A
        }

        fn bindings(_ctx: &TestContext) -> Vec<KeyBinding<TestIntent, TestApp>> {
            vec![
                KeyBinding {
                    key_label: "g",
                    action: "guarded",
                    description: "Only when context_b_active",
                    resolve: |key| {
                        matches!(key.code, KeyCode::Char('g')).then_some(TestIntent::Action1)
                    },
                    guard: Some(|app| app.context_b_active),
                },
                // Fallback binding (no guard) for the same key.
                KeyBinding {
                    key_label: "g",
                    action: "fallback",
                    description: "Always active fallback",
                    resolve: |key| {
                        matches!(key.code, KeyCode::Char('g')).then_some(TestIntent::Action2)
                    },
                    guard: None,
                },
            ]
        }
    }

    impl ModalHelp for GuardTestModal {
        fn help_contexts() -> Vec<HelpContext<TestContext>> {
            vec![HelpContext {
                title: "Guarded",
                context: TestContext::A,
            }]
        }
    }

    #[test]
    fn resolve_skips_guarded_binding_when_guard_fails() {
        // Guard requires context_b_active, but it's false.
        let app = app_context_a();
        assert_eq!(
            resolve::<GuardTestModal>(&app, &key(KeyCode::Char('g'))),
            Some(TestIntent::Action2), // Falls through to the unguarded binding.
        );
    }

    #[test]
    fn resolve_uses_guarded_binding_when_guard_passes() {
        let app = app_context_b();
        assert_eq!(
            resolve::<GuardTestModal>(&app, &key(KeyCode::Char('g'))),
            Some(TestIntent::Action1), // Guarded binding matches first.
        );
    }

    // -- hints tests --

    #[test]
    fn hints_returns_only_visible_bindings() {
        // Context A has 3 bindings, but one is hidden (empty key_label).
        let app = app_context_a();
        let h = hints::<TestModal>(&app);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].key, "q");
        assert_eq!(h[0].action, "quit");
        assert_eq!(h[1].key, "a");
        assert_eq!(h[1].action, "action1");
    }

    #[test]
    fn hints_filters_by_guard() {
        // GuardTestModal has two bindings for 'g'. When the guard fails,
        // only the unguarded one appears.
        let app = app_context_a();
        let h = hints::<GuardTestModal>(&app);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].action, "fallback");
    }

    #[test]
    fn hints_includes_guarded_binding_when_guard_passes() {
        let app = app_context_b();
        let h = hints::<GuardTestModal>(&app);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].action, "guarded");
        assert_eq!(h[1].action, "fallback");
    }

    #[test]
    fn hints_description_is_populated() {
        let app = app_context_a();
        let h = hints::<TestModal>(&app);
        assert_eq!(h[0].description, "Quit the application");
        assert_eq!(h[1].description, "Perform action 1");
    }

    #[test]
    fn hints_for_context_uses_explicit_context() {
        let app = app_context_a();
        let h = hints_for_context::<TestModal>(&app, &TestContext::B);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].key, "Esc");
        assert_eq!(h[0].action, "back");
    }

    #[test]
    fn all_hints_groups_contexts_without_hidden_bindings() {
        let groups = all_hints::<TestModal>();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].title, "Context A");
        assert_eq!(groups[0].hints.len(), 2);
        assert_eq!(groups[1].title, "Context B");
        assert_eq!(groups[1].hints.len(), 2);
    }

    #[test]
    fn all_hints_ignores_runtime_guards() {
        let groups = all_hints::<GuardTestModal>();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title, "Guarded");
        assert_eq!(groups[0].hints.len(), 2);
        assert_eq!(groups[0].hints[0].action, "guarded");
        assert_eq!(groups[0].hints[1].action, "fallback");
    }
}
