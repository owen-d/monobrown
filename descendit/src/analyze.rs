//! AST analysis engine for extracting structural metrics from Rust source.
//!
//! Walks directories of `.rs` files, parses each with `syn`, and extracts
//! function-level and type-level metrics. All analysis is deterministic:
//! the same source text always produces the same metrics.

use std::path::Path;

use syn::visit::Visit;
use walkdir::WalkDir;

use crate::duplication::{self, DuplicationReport, FunctionFingerprint, pat_has_mut};
use crate::metrics::{
    AnalysisReport, EntropyMetrics, FileEntropy, FunctionMetrics, ScopeSegment, Summary, TypeKind,
    TypeMetrics, module_path_from_scope,
};

// ---------------------------------------------------------------------------
// Extraction context types
// ---------------------------------------------------------------------------

/// Accumulated analysis results across all items in a file/crate.
///
/// Threaded mutably through the extraction functions so that each discovered
/// function, type, or fingerprint is appended in traversal order.
struct AnalysisContext {
    functions: Vec<FunctionMetrics>,
    types: Vec<TypeMetrics>,
    fingerprints: Vec<FunctionFingerprint>,
    macro_fn_count: usize,
    macro_export_fn_count: usize,
}

/// Immutable traversal state cloned-and-modified as we descend into nested scopes.
#[derive(Clone)]
struct TraversalState<'a> {
    source: &'a str,
    file: &'a str,
    scope: Vec<ScopeSegment>,
    in_test_module: bool,
    parent_is_pub: bool,
}

/// Analyze all Rust source files under `path` and produce an aggregate report.
///
/// If `path` is a file, analyzes that single file. If it is a directory,
/// recursively walks for `*.rs` files. Files that fail to parse are skipped
/// with a warning printed to stderr.
pub fn analyze_path(path: &Path) -> anyhow::Result<AnalysisReport> {
    // Enable span locations so we can extract line numbers from the AST.
    proc_macro2::fallback::force();

    let mut ctx = AnalysisContext {
        functions: Vec::new(),
        types: Vec::new(),
        fingerprints: Vec::new(),
        macro_fn_count: 0,
        macro_export_fn_count: 0,
    };
    let mut total_lines: usize = 0;
    let mut sources: Vec<(String, String)> = Vec::new();

    let entries = collect_rust_files(path);
    for file_path in &entries {
        total_lines += analyze_file(file_path, path, &mut ctx, &mut sources)?;
    }

    let duplication = duplication::detect_duplicates(&ctx.fingerprints);
    let mut summary = compute_summary(
        &ctx.functions,
        &ctx.types,
        &duplication,
        ctx.macro_fn_count,
        ctx.macro_export_fn_count,
    );
    // Override production_lines with comment-aware code line count from source text.
    summary.production_lines = sources.iter().map(|(_, src)| count_code_lines(src)).sum();
    let entropy = compute_entropy(&sources);
    let analysis_root = std::fs::canonicalize(path)
        .ok()
        .map(|canonical| canonical.to_string_lossy().to_string());

    Ok(AnalysisReport {
        analysis_root,
        files_analyzed: sources.len(),
        total_lines,
        functions: ctx.functions,
        types: ctx.types,
        entropy,
        duplication,
        summary,
        semantic: None,
    })
}

/// Collect all `.rs` file paths under `path`, sorted for deterministic order.
fn collect_rust_files(path: &Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<std::path::PathBuf> = if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        WalkDir::new(path)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "rs"))
            .map(walkdir::DirEntry::into_path)
            .collect()
    };
    // Sort for deterministic traversal order regardless of filesystem.
    files.sort();
    files
}

/// Read, parse, and extract items from a single Rust source file.
///
/// On success, pushes the `(relative_path, source)` pair into `sources` and
/// returns the raw line count. Parse failures emit a warning and return 0.
fn analyze_file(
    path: &Path,
    base_path: &Path,
    ctx: &mut AnalysisContext,
    sources: &mut Vec<(String, String)>,
) -> anyhow::Result<usize> {
    let source = std::fs::read_to_string(path)?;
    let line_count = source.lines().count();

    let relative = if base_path.is_file() {
        path.file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .to_string()
    } else {
        path.strip_prefix(base_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    };

    match syn::parse_file(&source) {
        Ok(syntax) => {
            let state = TraversalState {
                source: &source,
                file: &relative,
                scope: Vec::new(),
                in_test_module: false,
                parent_is_pub: true,
            };
            extract_items(&syntax.items, &state, ctx);
            sources.push((relative, source));
        }
        Err(err) => {
            eprintln!("warning: failed to parse {relative}: {err}");
        }
    }

    Ok(line_count)
}

/// Recursively extract function and type metrics from a list of syn items.
///
/// `state.scope` accumulates the nesting context as a stack of `ScopeSegment`s.
/// Items at the crate root have an empty scope.
/// `state.in_test_module` is `true` when we are inside a `#[cfg(test)]` module,
/// causing all extracted functions to be marked as test functions.
/// `state.parent_is_pub` tracks whether the enclosing module chain is publicly visible.
/// A function is only truly public if both it and all enclosing modules are `pub`.
fn extract_items(items: &[syn::Item], state: &TraversalState, ctx: &mut AnalysisContext) {
    for item in items {
        match item {
            syn::Item::Fn(item_fn) => {
                let fn_state = TraversalState {
                    in_test_module: state.in_test_module || has_attribute(&item_fn.attrs, "test"),
                    parent_is_pub: state.parent_is_pub
                        && matches!(item_fn.vis, syn::Visibility::Public(_)),
                    ..state.clone()
                };
                extract_function(&item_fn.sig, &item_fn.block, &fn_state, ctx);
            }
            syn::Item::Struct(item_struct) => {
                if let Some(m) = analyze_struct(item_struct, state.file) {
                    register_type(m, state, ctx);
                }
            }
            syn::Item::Enum(item_enum) => {
                register_type(analyze_enum(item_enum, state.file), state, ctx);
            }
            syn::Item::Impl(item_impl) => {
                let mut impl_scope = state.scope.clone();
                if let Some(type_name) = type_name_from_ty(&item_impl.self_ty) {
                    impl_scope.push(ScopeSegment::Type(type_name));
                }
                let impl_state = TraversalState {
                    scope: impl_scope,
                    ..state.clone()
                };
                extract_impl_items(&item_impl.items, &impl_state, ctx);
            }
            syn::Item::Mod(item_mod) => {
                if let Some((_, ref mod_items)) = item_mod.content {
                    let mut child_scope = state.scope.clone();
                    child_scope.push(ScopeSegment::Module(item_mod.ident.to_string()));
                    let child_state = TraversalState {
                        scope: child_scope,
                        in_test_module: state.in_test_module
                            || has_cfg_test_attribute(&item_mod.attrs),
                        parent_is_pub: state.parent_is_pub
                            && matches!(item_mod.vis, syn::Visibility::Public(_)),
                        ..state.clone()
                    };
                    extract_items(mod_items, &child_state, ctx);
                }
            }
            syn::Item::Trait(item_trait) => {
                let mut trait_scope = state.scope.clone();
                trait_scope.push(ScopeSegment::Type(item_trait.ident.to_string()));
                let trait_state = TraversalState {
                    scope: trait_scope,
                    ..state.clone()
                };
                extract_trait_items(&item_trait.items, &trait_state, ctx);
            }
            syn::Item::Macro(item_macro) => {
                count_macro_if_eligible(item_macro, state.in_test_module, ctx);
            }
            _ => {}
        }
    }
}

fn register_type(mut m: TypeMetrics, state: &TraversalState, ctx: &mut AnalysisContext) {
    m.module_path = module_path_from_scope(&state.scope);
    let mut type_scope = state.scope.clone();
    type_scope.push(ScopeSegment::Type(m.name.clone()));
    m.scope_path = type_scope;
    ctx.types.push(m);
}

/// Extract the type name from a `syn::Type` for scope tracking.
///
/// Returns `Some` only for concrete types, filtering single-character names
/// that are likely generic type parameters (e.g., `T`, `U`). This heuristic
/// cannot be perfect without type resolution.
fn type_name_from_ty(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(type_path) = ty
        && let Some(seg) = type_path.path.segments.last()
    {
        let name = seg.ident.to_string();
        if name.len() > 1 {
            return Some(name);
        }
    }
    None
}

/// Count a `macro_rules!` item if it has control flow and is not in a test module.
fn count_macro_if_eligible(
    item_macro: &syn::ItemMacro,
    in_test_module: bool,
    ctx: &mut AnalysisContext,
) {
    if !in_test_module
        && item_macro.ident.is_some()
        && item_macro.mac.path.is_ident("macro_rules")
        && macro_has_control_flow(&item_macro.mac.tokens)
    {
        ctx.macro_fn_count += 1;
        if has_attribute(&item_macro.attrs, "macro_export") {
            ctx.macro_export_fn_count += 1;
        }
    }
}

/// Extract functions from impl blocks.
fn extract_impl_items(items: &[syn::ImplItem], state: &TraversalState, ctx: &mut AnalysisContext) {
    for item in items {
        if let syn::ImplItem::Fn(method) = item {
            let fn_state = TraversalState {
                in_test_module: state.in_test_module || has_attribute(&method.attrs, "test"),
                parent_is_pub: state.parent_is_pub
                    && matches!(method.vis, syn::Visibility::Public(_)),
                ..state.clone()
            };
            extract_function(&method.sig, &method.block, &fn_state, ctx);
        }
    }
}

/// Extract default method implementations from trait blocks.
fn extract_trait_items(
    items: &[syn::TraitItem],
    state: &TraversalState,
    ctx: &mut AnalysisContext,
) {
    for item in items {
        if let syn::TraitItem::Fn(trait_fn) = item
            && let Some(ref block) = trait_fn.default
        {
            // Trait default methods are not standalone API surface.
            let fn_state = TraversalState {
                parent_is_pub: false,
                ..state.clone()
            };
            extract_function(&trait_fn.sig, block, &fn_state, ctx);
        }
    }
}

/// Analyze a function and generate both metrics and a structural fingerprint.
///
/// Also discovers nested items (inner functions, nested types/impls) inside
/// the function body and recurses into them via `extract_items`.
fn extract_function(
    sig: &syn::Signature,
    block: &syn::Block,
    state: &TraversalState,
    ctx: &mut AnalysisContext,
) {
    let name = sig.ident.to_string();
    let line = sig.ident.span().start().line;
    ctx.functions.push(analyze_function(
        sig,
        block,
        state.source,
        state.file,
        &state.scope,
        state.in_test_module,
        state.parent_is_pub,
    ));
    if !state.in_test_module {
        let mut fn_scope_for_fp = state.scope.clone();
        fn_scope_for_fp.push(ScopeSegment::Function(name.clone()));
        ctx.fingerprints.push(duplication::fingerprint_block(
            &name,
            state.file,
            line,
            block,
            fn_scope_for_fp,
        ));
    }

    // Discover nested items (inner functions, nested types) in the function body.
    for stmt in &block.stmts {
        if let syn::Stmt::Item(item) = stmt {
            let mut fn_scope = state.scope.clone();
            fn_scope.push(ScopeSegment::Function(sig.ident.to_string()));
            let nested_state = TraversalState {
                scope: fn_scope,
                parent_is_pub: false, // items inside functions can't be pub
                ..state.clone()
            };
            extract_items(std::slice::from_ref(item), &nested_state, ctx);
        }
    }
}

// ---------------------------------------------------------------------------
// Function analysis
// ---------------------------------------------------------------------------

/// Analyze a single function and produce metrics.
fn analyze_function(
    sig: &syn::Signature,
    block: &syn::Block,
    source: &str,
    file: &str,
    scope: &[ScopeSegment],
    is_test: bool,
    is_pub: bool,
) -> FunctionMetrics {
    let name = sig.ident.to_string();
    let line = sig.ident.span().start().line;
    let lines = count_block_lines(block, source);
    let params = count_params(sig);

    let mut nesting = NestingVisitor {
        current_depth: 0,
        max_depth: 0,
    };
    nesting.visit_block(block);

    let mut complexity = CyclomaticVisitor { count: 1 };
    complexity.visit_block(block);

    let mut mutability = MutableBindingVisitor {
        count: 0,
        cardinality_product: 1,
    };
    mutability.visit_block(block);

    let mut assertions = AssertionVisitor {
        count: 0,
        meaningful_count: 0,
    };
    assertions.visit_block(block);

    let internal_state_cardinality_log2 =
        if mutability.count == 0 || mutability.cardinality_product <= 1 {
            0.0
        } else {
            (mutability.cardinality_product as f64).log2()
        };

    let mut fn_scope = scope.to_vec();
    fn_scope.push(ScopeSegment::Function(name.clone()));

    FunctionMetrics {
        name,
        file: file.to_string(),
        module_path: module_path_from_scope(scope),
        scope_path: fn_scope,
        line,
        lines,
        params,
        nesting_depth: nesting.max_depth,
        cyclomatic: complexity.count,
        mutable_bindings: mutability.count,
        internal_state_cardinality_log2,
        assertions: assertions.count,
        meaningful_assertions: assertions.meaningful_count,
        is_test,
        is_pub,
    }
}

/// Count code lines in a block, excluding comments and blank lines.
///
/// Uses AST span info to extract the source slice for the function body,
/// then delegates to [`count_code_lines`] to count only executable lines.
fn count_block_lines(block: &syn::Block, source: &str) -> usize {
    let start = block.brace_token.span.open().start().line;
    let end = block.brace_token.span.close().end().line;

    // Span lines are 1-based. If they are valid and differ, extract the
    // source slice and count only non-comment, non-blank code lines.
    if end > start {
        let source_lines: Vec<&str> = source.lines().collect();
        let lo = start.saturating_sub(1).min(source_lines.len());
        // Exclude the closing-brace line from the count: the `}` is
        // structural scaffolding, not meaningful code.
        let hi = end.saturating_sub(1).min(source_lines.len());
        let block_text = source_lines[lo..hi].join("\n");
        return count_code_lines(&block_text).max(1);
    }

    // Fallback: count newlines in the source text between the byte
    // offsets of the opening and closing braces, or just use statement
    // count as a rough proxy.
    let open_offset = block.brace_token.span.open().start().column;
    let close_offset = block.brace_token.span.close().end().column;
    if close_offset > open_offset {
        source[open_offset..close_offset].lines().count().max(1)
    } else {
        // Last resort: count statements + 2 for the braces.
        block.stmts.len() + 2
    }
}

/// Count parameters, skipping `self` variants for methods.
fn count_params(sig: &syn::Signature) -> usize {
    sig.inputs
        .iter()
        .filter(|arg| matches!(arg, syn::FnArg::Typed(_)))
        .count()
}

// ---------------------------------------------------------------------------
// AST visitors
// ---------------------------------------------------------------------------

/// Tracks maximum nesting depth within a function body.
struct NestingVisitor {
    current_depth: usize,
    max_depth: usize,
}

impl NestingVisitor {
    /// Run the body of a nested construct, incrementing depth around it.
    fn enter_nested<F: FnOnce(&mut Self)>(&mut self, f: F) {
        self.current_depth += 1;
        if self.current_depth > self.max_depth {
            self.max_depth = self.current_depth;
        }
        f(self);
        self.current_depth -= 1;
    }
}

impl<'ast> Visit<'ast> for NestingVisitor {
    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        self.enter_nested(|this| syn::visit::visit_expr_if(this, node));
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        self.enter_nested(|this| {
            syn::visit::visit_expr_match(this, node);
        });
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.enter_nested(|this| {
            syn::visit::visit_expr_for_loop(this, node);
        });
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.enter_nested(|this| {
            syn::visit::visit_expr_while(this, node);
        });
    }

    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.enter_nested(|this| {
            syn::visit::visit_expr_loop(this, node);
        });
    }

    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        self.enter_nested(|this| {
            syn::visit::visit_expr_closure(this, node);
        });
    }
}

/// Counts cyclomatic complexity by walking the AST for branch points.
struct CyclomaticVisitor {
    /// Starts at 1 (the base path through the function).
    count: usize,
}

impl<'ast> Visit<'ast> for CyclomaticVisitor {
    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        // +1 for the `if` branch.
        self.count += 1;
        syn::visit::visit_expr_if(self, node);
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        // +1 for each arm beyond the first (one arm is the default path).
        if node.arms.len() > 1 {
            self.count += node.arms.len() - 1;
        }
        syn::visit::visit_expr_match(self, node);
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.count += 1;
        syn::visit::visit_expr_for_loop(self, node);
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.count += 1;
        syn::visit::visit_expr_while(self, node);
    }

    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.count += 1;
        syn::visit::visit_expr_loop(self, node);
    }

    fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
        // +1 for each short-circuit operator.
        match node.op {
            syn::BinOp::And(_) | syn::BinOp::Or(_) => {
                self.count += 1;
            }
            _ => {}
        }
        syn::visit::visit_expr_binary(self, node);
    }

    fn visit_expr_try(&mut self, node: &'ast syn::ExprTry) {
        // +1 for the `?` operator (early return on error).
        self.count += 1;
        syn::visit::visit_expr_try(self, node);
    }
}

/// Counts `let mut` bindings in a function body and tracks their inferred cardinalities.
///
/// Cardinality is inferred from the initializer expression:
/// - `true` / `false` → bool → 2
/// - `None` / `Some(...)` → Option → 2
/// - `Ok(...)` / `Err(...)` → Result → 2
/// - Everything else (integer literals, function calls, `Vec::new()`, etc.) → scalar → 1
struct MutableBindingVisitor {
    count: usize,
    /// Product of per-binding cardinalities (multiplicative accumulator).
    cardinality_product: u64,
}

impl<'ast> Visit<'ast> for MutableBindingVisitor {
    fn visit_local(&mut self, node: &'ast syn::Local) {
        if pat_has_mut(&node.pat) {
            self.count += 1;
            let card = infer_initializer_cardinality(node);
            self.cardinality_product = self.cardinality_product.saturating_mul(card);
        }
        syn::visit::visit_local(self, node);
    }
}

/// Infer the state cardinality of a `let mut` binding from its initializer expression.
///
/// Returns 2 for bool-like / Option-like / Result-like initializers, 1 for everything else.
fn infer_initializer_cardinality(local: &syn::Local) -> u64 {
    let init = match &local.init {
        Some(init) => &*init.expr,
        None => return 1,
    };
    infer_expr_cardinality(init)
}

/// Classify an expression to infer state cardinality for its binding.
fn infer_expr_cardinality(expr: &syn::Expr) -> u64 {
    match expr {
        // `true` or `false` literal → bool → 2
        syn::Expr::Lit(expr_lit) => match &expr_lit.lit {
            syn::Lit::Bool(_) => 2,
            _ => 1,
        },
        // Path expressions: `None`, `true`, `false` as paths
        syn::Expr::Path(expr_path) if expr_path.qself.is_none() => {
            if let Some(ident) = expr_path.path.get_ident() {
                let name = ident.to_string();
                match name.as_str() {
                    "true" | "false" => 2,
                    "None" => 2,
                    _ => 1,
                }
            } else {
                1
            }
        }
        // `Some(...)` or `Ok(...)` or `Err(...)` → 2
        syn::Expr::Call(expr_call) => {
            if let syn::Expr::Path(path) = &*expr_call.func
                && let Some(ident) = path.path.get_ident()
            {
                let name = ident.to_string();
                match name.as_str() {
                    "Some" | "Ok" | "Err" => return 2,
                    _ => {}
                }
            }
            1
        }
        _ => 1,
    }
}

/// Counts assertion macro invocations in a function body.
///
/// Tracks both total assertions and "meaningful" assertions. An assertion is
/// meaningful if its token stream contains at least one `Ident` token that
/// is not `true` or `false` — i.e., it references a real variable or expression.
struct AssertionVisitor {
    count: usize,
    meaningful_count: usize,
}

/// Macro names that count as assertions for structural density tracking.
const ASSERTION_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "ensure",
    "bail",
];

impl AssertionVisitor {
    fn count_macro_assertion(&mut self, mac: &syn::Macro) {
        if is_assertion_macro(&mac.path) {
            self.count += 1;
            if has_meaningful_ident(&mac.tokens) {
                self.meaningful_count += 1;
            }
        }
    }
}

impl<'ast> Visit<'ast> for AssertionVisitor {
    fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
        self.count_macro_assertion(&node.mac);
        syn::visit::visit_expr_macro(self, node);
    }

    fn visit_stmt_macro(&mut self, node: &'ast syn::StmtMacro) {
        self.count_macro_assertion(&node.mac);
        syn::visit::visit_stmt_macro(self, node);
    }
}

/// Check whether a macro path's last segment matches an assertion macro name.
fn is_assertion_macro(path: &syn::Path) -> bool {
    path.segments
        .last()
        .is_some_and(|seg| ASSERTION_MACROS.contains(&seg.ident.to_string().as_str()))
}

/// Check whether a token stream contains at least one meaningful identifier.
///
/// An identifier is "meaningful" if it is not `true` or `false`. This filters
/// out trivial assertions like `assert!(true)` which only contain boolean
/// keywords and no real variable references.
fn has_meaningful_ident(tokens: &proc_macro2::TokenStream) -> bool {
    for token in tokens.clone() {
        match token {
            proc_macro2::TokenTree::Ident(ref ident) => {
                let name = ident.to_string();
                if name != "true" && name != "false" {
                    return true;
                }
            }
            proc_macro2::TokenTree::Group(ref group) if has_meaningful_ident(&group.stream()) => {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Check whether any attribute in the list has the given path identifier
/// (e.g., `"test"`, `"macro_export"`).
fn has_attribute(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}

/// Check whether a module has a `#[cfg(test)]` attribute.
///
/// Looks for an attribute with path `cfg` whose token stream contains the
/// identifier `test`.
fn has_cfg_test_attribute(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr.meta.require_list().ok().is_some_and(|list| {
                list.tokens
                    .clone()
                    .into_iter()
                    .any(|tok| matches!(tok, proc_macro2::TokenTree::Ident(ref id) if id == "test"))
            })
    })
}

/// Check whether a `macro_rules!` token body contains control flow keywords.
///
/// Walks all `TokenTree` items recursively (descending into `Group` delimiters).
/// Returns `true` if any `Ident` matches `if`, `match`, `while`, `for`, or `loop`.
fn macro_has_control_flow(tokens: &proc_macro2::TokenStream) -> bool {
    const CONTROL_FLOW: &[&str] = &["if", "match", "while", "for", "loop"];
    for token in tokens.clone() {
        match token {
            proc_macro2::TokenTree::Ident(ref ident)
                if CONTROL_FLOW.contains(&ident.to_string().as_str()) =>
            {
                return true;
            }
            proc_macro2::TokenTree::Group(ref group) if macro_has_control_flow(&group.stream()) => {
                return true;
            }
            _ => {}
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Type analysis
// ---------------------------------------------------------------------------

/// Recursively compute the structural cardinality of a type from its AST node.
///
/// Cardinality is the number of distinct values a type can represent,
/// approximated structurally from the syntax tree alone. Unknown named types
/// conservatively contribute 1 (a single opaque value).
///
/// Uses saturating arithmetic to avoid overflow on large composite types.
///
/// # Examples
///
/// ```
/// # use descendit::analyze::compute_type_cardinality;
/// // Bool soup: struct { a: bool, b: bool, c: bool }
/// // Each field is bool (2), product = 2 * 2 * 2 = 8.
/// let ty: syn::Type = syn::parse_str("bool").unwrap();
/// assert_eq!(compute_type_cardinality(&ty), 2);
///
/// // State machine: enum Conn { Disconnected, Connected { auth: bool }, Error(String) }
/// // Disconnected=1, Connected=2, Error=1 => sum = 4.
/// // Fewer states than three bools despite more variants — that's the point.
/// let option_bool: syn::Type = syn::parse_str("Option<bool>").unwrap();
/// assert_eq!(compute_type_cardinality(&option_bool), 3); // 1 + 2
/// ```
pub fn compute_type_cardinality(ty: &syn::Type) -> u64 {
    match ty {
        syn::Type::Path(type_path) if type_path.qself.is_none() => {
            let segments = &type_path.path.segments;
            if segments.len() != 1 {
                return 1;
            }
            let seg = &segments[0];
            let ident = seg.ident.to_string();

            match ident.as_str() {
                "bool" => 2,
                "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64"
                | "i128" | "isize" | "f32" | "f64" | "char" | "String" | "str" => 1,
                "Option" => {
                    if let syn::PathArguments::AngleBracketed(ref args) = seg.arguments
                        && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
                    {
                        return 1u64.saturating_add(compute_type_cardinality(inner));
                    }
                    1
                }
                "Result" => {
                    if let syn::PathArguments::AngleBracketed(ref args) = seg.arguments {
                        let mut iter = args.args.iter().filter_map(|a| {
                            if let syn::GenericArgument::Type(t) = a {
                                Some(t)
                            } else {
                                None
                            }
                        });
                        if let (Some(ok_ty), Some(err_ty)) = (iter.next(), iter.next()) {
                            return compute_type_cardinality(ok_ty)
                                .saturating_add(compute_type_cardinality(err_ty));
                        }
                    }
                    1
                }
                _ => 1, // Conservative: unknown named type
            }
        }
        syn::Type::Reference(type_ref) => compute_type_cardinality(&type_ref.elem),
        syn::Type::Tuple(type_tuple) => {
            if type_tuple.elems.is_empty() {
                return 1; // unit tuple
            }
            type_tuple.elems.iter().fold(1u64, |acc, elem| {
                acc.saturating_mul(compute_type_cardinality(elem))
            })
        }
        syn::Type::Array(_) | syn::Type::Slice(_) => 1,
        _ => 1,
    }
}

/// Analyze a struct with named fields. Returns `None` for unit/tuple structs.
fn analyze_struct(item: &syn::ItemStruct, file: &str) -> Option<TypeMetrics> {
    let fields = match &item.fields {
        syn::Fields::Named(named) => &named.named,
        // Skip unit structs and tuple structs (no named fields to analyze).
        _ => return None,
    };

    let name = item.ident.to_string();
    let line = item.ident.span().start().line;
    let total_fields = fields.len();

    let mut bool_fields: usize = 0;
    let mut option_fields: usize = 0;

    // Struct cardinality = product of field cardinalities.
    let mut state_cardinality: u64 = 1;

    for field in fields {
        if is_named_type(&field.ty, "bool") {
            bool_fields += 1;
        }
        if is_named_type(&field.ty, "Option") {
            option_fields += 1;
        }
        state_cardinality = state_cardinality.saturating_mul(compute_type_cardinality(&field.ty));
    }

    let state_cardinality_log2 = if state_cardinality <= 1 {
        0.0
    } else {
        (state_cardinality as f64).log2()
    };

    Some(TypeMetrics {
        name,
        file: file.to_string(),
        module_path: String::new(),
        scope_path: Vec::new(),
        line,
        kind: TypeKind::Struct,
        bool_fields,
        option_fields,
        total_fields,
        state_cardinality,
        state_cardinality_log2,
    })
}

/// Analyze an enum: count variants, compute state cardinality.
fn analyze_enum(item: &syn::ItemEnum, file: &str) -> TypeMetrics {
    let name = item.ident.to_string();
    let line = item.ident.span().start().line;
    let total_fields = item.variants.len();

    let mut bool_fields: usize = 0;
    let mut option_fields: usize = 0;

    // Enum cardinality = max per-variant product cardinality.
    //
    // Enum variants are domain-inherent complexity (all states valid by
    // construction), so variant *count* should not inflate cardinality.
    // Instead we score only the intra-variant boolean soup: a variant
    // with fields `{ a: bool, b: bool }` has cardinality 4, while a
    // unit variant has cardinality 1. The enum's cardinality is the
    // worst-case (maximum) across all variants.
    let mut state_cardinality: u64 = 1;

    for variant in &item.variants {
        let fields = variant_fields(variant);
        if fields.is_empty() {
            // Unit variant: cardinality 1 (cannot exceed current max).
        } else {
            let mut variant_card: u64 = 1;
            for field in &fields {
                if is_named_type(field, "bool") {
                    bool_fields += 1;
                }
                if is_named_type(field, "Option") {
                    option_fields += 1;
                }
                variant_card = variant_card.saturating_mul(compute_type_cardinality(field));
            }
            state_cardinality = state_cardinality.max(variant_card);
        }
    }

    let state_cardinality_log2 = if state_cardinality <= 1 {
        0.0
    } else {
        (state_cardinality as f64).log2()
    };

    TypeMetrics {
        name,
        file: file.to_string(),
        module_path: String::new(),
        scope_path: Vec::new(),
        line,
        kind: TypeKind::Enum,
        bool_fields,
        option_fields,
        total_fields,
        state_cardinality,
        state_cardinality_log2,
    }
}

/// Extract field types from a variant (handles unit, tuple, and struct variants).
fn variant_fields(variant: &syn::Variant) -> Vec<&syn::Type> {
    match &variant.fields {
        syn::Fields::Named(named) => named.named.iter().map(|f| &f.ty).collect(),
        syn::Fields::Unnamed(unnamed) => unnamed.unnamed.iter().map(|f| &f.ty).collect(),
        syn::Fields::Unit => Vec::new(),
    }
}

/// Check whether a type's outermost path segment matches `name`
/// (e.g., `"bool"`, `"Option"`).
fn is_named_type(ty: &syn::Type, name: &str) -> bool {
    if let syn::Type::Path(type_path) = ty {
        type_path.qself.is_none()
            && type_path.path.segments.len() == 1
            && type_path.path.segments[0].ident == name
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Comment-aware line counting
// ---------------------------------------------------------------------------

/// Count non-trivial code lines in source text.
///
/// Skips blank lines, single-line comment lines (`//`), and lines inside
/// block comments (`/* ... */`). A line that contains code after a block
/// comment close (`*/`) counts as a code line.
pub fn count_code_lines(source: &str) -> usize {
    let mut code_lines: usize = 0;
    let mut in_block_comment = false;

    for line in source.lines() {
        if in_block_comment {
            if let Some(pos) = line.find("*/") {
                in_block_comment = false;
                // Check if there is non-whitespace code after the `*/`.
                let after = &line[pos + 2..];
                let after_trimmed = after.trim();
                if !after_trimmed.is_empty() && !after_trimmed.starts_with("//") {
                    code_lines += 1;
                }
            }
            // Entire line is inside a block comment; skip it.
            continue;
        }

        let trimmed = line.trim();

        // Blank line.
        if trimmed.is_empty() {
            continue;
        }

        // Single-line comment.
        if trimmed.starts_with("//") {
            continue;
        }

        // Block comment opening on this line.
        if let Some(rest) = trimmed.strip_prefix("/*") {
            // Check if block comment closes on the same line.
            if let Some(close_pos) = rest.find("*/") {
                // Block comment is self-contained. Check for code after it.
                let after = &rest[close_pos + 2..];
                let after_trimmed = after.trim();
                if !after_trimmed.is_empty() && !after_trimmed.starts_with("//") {
                    code_lines += 1;
                }
            } else {
                in_block_comment = true;
            }
            continue;
        }

        // Non-trivial code line.
        code_lines += 1;
    }

    code_lines
}

// ---------------------------------------------------------------------------
// Entropy computation
// ---------------------------------------------------------------------------

/// Characters that delimit tokens for entropy analysis.
const DELIMITERS: &[char] = &[
    '{', '}', '(', ')', ';', ',', ':', '.', '<', '>', '[', ']', '=', '+', '-', '*', '/', '&', '|',
    '!', '?', '#', '@',
];

/// Tokenize source text by splitting on whitespace and delimiters.
///
/// Each delimiter is emitted as its own single-character token. Runs of
/// non-delimiter, non-whitespace characters form identifier/keyword tokens.
fn tokenize(source: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = None;

    for (i, ch) in source.char_indices() {
        if ch.is_whitespace() {
            if let Some(s) = start.take() {
                tokens.push(&source[s..i]);
            }
        } else if DELIMITERS.contains(&ch) {
            if let Some(s) = start.take() {
                tokens.push(&source[s..i]);
            }
            tokens.push(&source[i..i + ch.len_utf8()]);
        } else if start.is_none() {
            start = Some(i);
        }
    }
    // Flush any trailing token.
    if let Some(s) = start {
        tokens.push(&source[s..]);
    }
    tokens
}

/// Compute Shannon entropy from token frequencies.
///
/// Returns `(entropy_bits, normalized_entropy)`. When there are zero tokens
/// or only one unique token, entropy is 0.
fn shannon_entropy(
    frequencies: &std::collections::HashMap<&str, usize>,
    total: usize,
) -> (f64, f64) {
    let vocab = frequencies.len();
    if total == 0 || vocab <= 1 {
        return (0.0, 0.0);
    }

    let n = total as f64;
    let entropy: f64 = frequencies
        .values()
        .map(|&count| {
            let p = count as f64 / n;
            -p * p.log2()
        })
        .sum();

    let max_entropy = (vocab as f64).log2();
    let normalized = if max_entropy > 0.0 {
        entropy / max_entropy
    } else {
        0.0
    };

    (entropy, normalized)
}

/// Compute token-level entropy metrics for a set of source files.
///
/// `sources` is a slice of `(filename, content)` pairs. Per-file metrics
/// are sorted by normalized entropy ascending (most repetitive first).
fn compute_entropy(sources: &[(String, String)]) -> EntropyMetrics {
    let mut aggregate_freq: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    let mut aggregate_total: usize = 0;
    let mut per_file = Vec::with_capacity(sources.len());

    for (filename, content) in sources {
        let tokens = tokenize(content);
        let total = tokens.len();

        let mut freq: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for &tok in &tokens {
            *freq.entry(tok).or_insert(0) += 1;
        }

        let vocabulary = freq.len();
        let (entropy_bits, normalized_entropy) = shannon_entropy(&freq, total);

        per_file.push(FileEntropy {
            file: filename.clone(),
            tokens: total,
            vocabulary,
            entropy_bits,
            normalized_entropy,
        });

        // Accumulate into aggregate frequencies.
        for (&tok, &count) in &freq {
            *aggregate_freq.entry(tok).or_insert(0) += count;
        }
        aggregate_total += total;
    }

    // Sort per-file by normalized entropy ascending (most repetitive first).
    per_file.sort_by(|a, b| {
        a.normalized_entropy
            .partial_cmp(&b.normalized_entropy)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let vocabulary_size = aggregate_freq.len();
    let (entropy_bits, normalized_entropy) = shannon_entropy(&aggregate_freq, aggregate_total);

    EntropyMetrics {
        total_tokens: aggregate_total,
        vocabulary_size,
        entropy_bits,
        normalized_entropy,
        per_file,
    }
}

// ---------------------------------------------------------------------------
// Summary computation
// ---------------------------------------------------------------------------

/// Compute aggregate summary statistics from collected metrics.
fn compute_summary(
    functions: &[FunctionMetrics],
    types: &[TypeMetrics],
    duplication: &DuplicationReport,
    macro_fn_count: usize,
    macro_export_fn_count: usize,
) -> Summary {
    let mut summary = compute_function_summary(functions, macro_fn_count, macro_export_fn_count);
    summarize_types(types, &mut summary);
    summary.exact_duplicate_groups = duplication.exact_duplicates.len();
    summary.near_duplicate_pairs = duplication.near_duplicates.len();
    summary.duplication_score = duplication.duplication_score;
    summary
}

/// Compute function-level summary statistics.
fn compute_function_summary(
    functions: &[FunctionMetrics],
    macro_fn_count: usize,
    macro_export_fn_count: usize,
) -> Summary {
    let function_count = functions.len();
    if functions.is_empty() && macro_fn_count == 0 {
        return Summary {
            function_count: 0,
            ..Summary::default()
        };
    }

    let n = function_count as f64;
    let total_assert: usize = functions.iter().map(|f| f.assertions).sum();
    let total_meaningful: usize = functions.iter().map(|f| f.meaningful_assertions).sum();

    let is_nontrivial = |f: &FunctionMetrics| f.lines > 5 && f.cyclomatic > 1;
    let nontrivial_function_count = functions.iter().filter(|f| is_nontrivial(f)).count();
    let nontrivial_functions_under_2_assertions = functions
        .iter()
        .filter(|f| is_nontrivial(f) && f.assertions < 2)
        .count();

    let test_function_count = functions.iter().filter(|f| f.is_test).count();
    let production_function_count = function_count - test_function_count;
    let public_function_count = functions.iter().filter(|f| f.is_pub && !f.is_test).count();
    let function_overhead_ratio = compute_overhead_ratio(
        production_function_count,
        public_function_count,
        macro_fn_count,
        macro_export_fn_count,
    );

    Summary {
        function_count,
        max_function_lines: functions.iter().map(|f| f.lines).max().unwrap_or(0),
        mean_function_lines: mean_of(functions.iter().map(|f| f.lines), n),
        functions_over_70_lines: functions.iter().filter(|f| f.lines > 70).count(),
        max_nesting_depth: functions.iter().map(|f| f.nesting_depth).max().unwrap_or(0),
        mean_nesting_depth: mean_of(functions.iter().map(|f| f.nesting_depth), n),
        max_cyclomatic: functions.iter().map(|f| f.cyclomatic).max().unwrap_or(0),
        mean_cyclomatic: mean_of(functions.iter().map(|f| f.cyclomatic), n),
        max_params: functions.iter().map(|f| f.params).max().unwrap_or(0),
        total_mutable_bindings: functions.iter().map(|f| f.mutable_bindings).sum(),
        functions_under_2_assertions: functions.iter().filter(|f| f.assertions < 2).count(),
        nontrivial_functions_under_2_assertions,
        nontrivial_function_count,
        total_assertions: total_assert,
        mean_assertions_per_function: mean_of(functions.iter().map(|f| f.assertions), n),
        total_meaningful_assertions: total_meaningful,
        mean_meaningful_assertions_per_function: mean_of(
            functions.iter().map(|f| f.meaningful_assertions),
            n,
        ),
        test_function_count,
        production_function_count,
        public_function_count,
        macro_fn_count,
        macro_export_fn_count,
        function_overhead_ratio,
        test_density: compute_test_density(functions),
        total_production_cyclomatic: functions
            .iter()
            .filter(|f| !f.is_test)
            .map(|f| f.cyclomatic)
            .sum(),
        production_lines: functions
            .iter()
            .filter(|f| !f.is_test)
            .map(|f| f.lines)
            .sum(),
        ..Summary::default()
    }
}

/// Compute the mean of an iterator of usize values given a pre-computed count as f64.
fn mean_of(iter: impl Iterator<Item = usize>, n: f64) -> f64 {
    if n > 0.0 {
        iter.sum::<usize>() as f64 / n
    } else {
        0.0
    }
}

/// Compute the function overhead ratio: adjusted production / adjusted public.
///
/// Macros with control flow count as production functions; `#[macro_export]` ones as public.
fn compute_overhead_ratio(
    production_function_count: usize,
    public_function_count: usize,
    macro_fn_count: usize,
    macro_export_fn_count: usize,
) -> f64 {
    let adjusted_production = production_function_count + macro_fn_count;
    let adjusted_public = public_function_count + macro_export_fn_count;
    if adjusted_public == 0 {
        0.0
    } else {
        adjusted_production as f64 / adjusted_public as f64
    }
}

/// Compute test density: test assertions / production cyclomatic complexity.
///
/// Returns 0.0 when there are no production branches (vacuously untestable).
fn compute_test_density(functions: &[FunctionMetrics]) -> f64 {
    let test_assertions: usize = functions
        .iter()
        .filter(|f| f.is_test)
        .map(|f| f.assertions)
        .sum();
    let production_complexity: usize = functions
        .iter()
        .filter(|f| !f.is_test)
        .map(|f| f.cyclomatic)
        .sum();
    if production_complexity == 0 {
        0.0
    } else {
        test_assertions as f64 / production_complexity as f64
    }
}

/// Fill in type-level summary fields.
fn summarize_types(types: &[TypeMetrics], summary: &mut Summary) {
    summary.type_count = types.len();
    summary.total_bool_fields = types.iter().map(|t| t.bool_fields).sum();
    summary.total_option_fields = types.iter().map(|t| t.option_fields).sum();
    summary.max_state_cardinality_log2 = types
        .iter()
        .map(|t| t.state_cardinality_log2)
        .fold(0.0_f64, f64::max);
}

// ---------------------------------------------------------------------------
// Helpers for tests
// ---------------------------------------------------------------------------

/// Parse a Rust source string and extract all function metrics.
/// Useful for unit tests that operate on inline snippets.
#[cfg(test)]
fn analyze_source(source: &str) -> (Vec<FunctionMetrics>, Vec<TypeMetrics>) {
    let (fns, types, _, _) = analyze_source_with_macros(source);
    (fns, types)
}

/// Like `analyze_source`, but also returns macro counts.
#[cfg(test)]
fn analyze_source_with_macros(
    source: &str,
) -> (Vec<FunctionMetrics>, Vec<TypeMetrics>, usize, usize) {
    proc_macro2::fallback::force();
    let syntax =
        syn::parse_file(source).unwrap_or_else(|e| panic!("test source failed to parse: {e}"));
    let mut ctx = AnalysisContext {
        functions: Vec::new(),
        types: Vec::new(),
        fingerprints: Vec::new(),
        macro_fn_count: 0,
        macro_export_fn_count: 0,
    };
    let state = TraversalState {
        source,
        file: "<test>",
        scope: Vec::new(),
        in_test_module: false,
        parent_is_pub: true,
    };
    extract_items(&syntax.items, &state, &mut ctx);
    (
        ctx.functions,
        ctx.types,
        ctx.macro_fn_count,
        ctx.macro_export_fn_count,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_function_metrics() {
        let source = r#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);

        let f = &fns[0];
        assert_eq!(f.name, "add");
        assert_eq!(f.params, 2);
        // Simple function with no branches: cyclomatic = 1.
        assert_eq!(f.cyclomatic, 1);
        assert_eq!(f.nesting_depth, 0);
        assert_eq!(f.mutable_bindings, 0);
        // Body spans from `{` to `}` across multiple lines.
        assert!(f.lines >= 2, "expected at least 2 lines, got {}", f.lines);
    }

    #[test]
    fn test_nested_function() {
        let source = r#"
fn deeply_nested(x: i32) {
    if x > 0 {
        for i in 0..x {
            if i % 2 == 0 {
                match i {
                    0 => {}
                    _ => {}
                }
            }
        }
    }
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);

        let f = &fns[0];
        assert_eq!(f.name, "deeply_nested");
        // Nesting: if > for > if > match = depth 4.
        assert_eq!(f.nesting_depth, 4);
        assert_eq!(f.params, 1);
        // Cyclomatic: 1 base + 1 (if) + 1 (for) + 1 (if) + 1 (match arm beyond first) = 5.
        assert_eq!(f.cyclomatic, 5);
    }

    #[test]
    fn test_struct_bool_fields() {
        let source = r#"
struct Flags {
    enabled: bool,
    visible: bool,
    name: String,
    count: Option<usize>,
}
"#;
        let (_, types) = analyze_source(source);
        assert_eq!(types.len(), 1);

        let t = &types[0];
        assert_eq!(t.name, "Flags");
        assert_eq!(t.bool_fields, 2);
        assert_eq!(t.option_fields, 1);
        assert_eq!(t.total_fields, 4);
        // bool(2) * bool(2) * String(1) * Option<usize>(1+1=2) = 8.
        assert_eq!(t.state_cardinality, 8);
        assert!(
            (t.state_cardinality_log2 - 3.0).abs() < f64::EPSILON,
            "expected 3.0, got {}",
            t.state_cardinality_log2,
        );
    }

    #[test]
    fn test_enum_variants() {
        let source = r#"
enum Color {
    Red,
    Green,
    Blue,
    Custom(bool),
}
"#;
        let (_, types) = analyze_source(source);
        assert_eq!(types.len(), 1);

        let t = &types[0];
        assert_eq!(t.name, "Color");
        assert_eq!(t.total_fields, 4); // 4 variants
        assert_eq!(t.bool_fields, 1); // Custom(bool)
        // Max variant cardinality: max(1, 1, 1, 2) = 2 (from Custom(bool)).
        assert_eq!(t.state_cardinality, 2);
        assert!(
            (t.state_cardinality_log2 - 1.0).abs() < f64::EPSILON,
            "expected log2(2)=1.0, got {}",
            t.state_cardinality_log2,
        );
    }

    #[test]
    fn test_empty_file() {
        let source = "";
        let (fns, types) = analyze_source(source);
        assert!(fns.is_empty());
        assert!(types.is_empty());
    }

    #[test]
    fn test_analyze_path_single_file_uses_filename() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("single.rs");
        std::fs::write(
            &file_path,
            r#"
fn only_function() {
    let answer = 42;
    let _ = answer;
}
"#,
        )
        .unwrap();

        let report = analyze_path(&file_path).unwrap();

        assert_eq!(report.files_analyzed, 1);
        assert_eq!(report.functions.len(), 1);
        assert_eq!(report.functions[0].file, "single.rs");
    }

    #[test]
    fn test_cyclomatic_short_circuit() {
        let source = r#"
fn check(a: bool, b: bool, c: bool) -> bool {
    a && b || c
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        // 1 base + 1 (&&) + 1 (||) = 3.
        assert_eq!(fns[0].cyclomatic, 3);
    }

    #[test]
    fn test_question_mark_operator() {
        let source = r#"
fn fallible(x: Option<i32>) -> Option<i32> {
    let val = x?;
    Some(val + 1)
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        // 1 base + 1 (?) = 2.
        assert_eq!(fns[0].cyclomatic, 2);
    }

    #[test]
    fn test_mutable_bindings() {
        let source = r#"
fn mutate() {
    let mut x = 0;
    let y = 1;
    let mut z = 2;
    x += z;
    let _ = y;
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].mutable_bindings, 2);
    }

    #[test]
    fn test_method_self_not_counted() {
        let source = r#"
struct Foo;
impl Foo {
    fn method(&self, x: i32) -> i32 {
        x
    }
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        // `self` is not counted; only `x`.
        assert_eq!(fns[0].params, 1);
    }

    #[test]
    fn test_unit_struct_skipped() {
        let source = r#"
struct Marker;
"#;
        let (_, types) = analyze_source(source);
        // Unit structs are skipped (no named fields to analyze).
        assert!(types.is_empty());
    }

    #[test]
    fn test_enum_struct_variant() {
        let source = r#"
enum Message {
    Quit,
    Data { payload: Vec<u8>, compressed: bool },
}
"#;
        let (_, types) = analyze_source(source);
        assert_eq!(types.len(), 1);

        let t = &types[0];
        assert_eq!(t.total_fields, 2); // 2 variants
        assert_eq!(t.bool_fields, 1); // compressed: bool
        // Max variant cardinality: max(1, 1*2) = 2 (from Data{compressed:bool}).
        assert_eq!(t.state_cardinality, 2);
        assert!(
            (t.state_cardinality_log2 - 1.0).abs() < f64::EPSILON,
            "expected log2(2)=1.0, got {}",
            t.state_cardinality_log2,
        );
    }

    // -----------------------------------------------------------------------
    // Entropy tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_entropy_basic() {
        // Repetitive file: all the same token repeated.
        let repetitive = ("repetitive.rs".to_string(), "x x x x x x x x".to_string());
        // Diverse file: many unique tokens.
        let diverse = (
            "diverse.rs".to_string(),
            "alpha beta gamma delta epsilon zeta eta theta".to_string(),
        );

        let metrics = compute_entropy(&[repetitive, diverse]);
        assert_eq!(metrics.per_file.len(), 2);

        // Per-file results are sorted by normalized entropy ascending,
        // so the repetitive file should come first.
        assert_eq!(metrics.per_file[0].file, "repetitive.rs");
        assert_eq!(metrics.per_file[1].file, "diverse.rs");
        assert!(
            metrics.per_file[0].normalized_entropy < metrics.per_file[1].normalized_entropy,
            "repetitive file should have lower normalized entropy: {} vs {}",
            metrics.per_file[0].normalized_entropy,
            metrics.per_file[1].normalized_entropy,
        );
    }

    #[test]
    fn test_entropy_single_token() {
        // All tokens identical: entropy should be 0.
        let sources = vec![("mono.rs".to_string(), "aaa aaa aaa aaa".to_string())];
        let metrics = compute_entropy(&sources);

        assert_eq!(metrics.total_tokens, 4);
        assert_eq!(metrics.vocabulary_size, 1);
        assert!(
            metrics.entropy_bits.abs() < f64::EPSILON,
            "expected 0 entropy, got {}",
            metrics.entropy_bits,
        );
        assert!(
            metrics.normalized_entropy.abs() < f64::EPSILON,
            "expected 0 normalized entropy, got {}",
            metrics.normalized_entropy,
        );
    }

    #[test]
    fn test_entropy_empty_file() {
        let sources = vec![("empty.rs".to_string(), String::new())];
        let metrics = compute_entropy(&sources);

        assert_eq!(metrics.total_tokens, 0);
        assert_eq!(metrics.vocabulary_size, 0);
        assert!(metrics.entropy_bits.abs() < f64::EPSILON);
        assert!(metrics.normalized_entropy.abs() < f64::EPSILON);
    }

    // -----------------------------------------------------------------------
    // Assertion density tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_assertion_counting() {
        let source = r#"
fn well_guarded(x: i32) {
    assert!(x > 0);
    assert_eq!(x, 42);
    debug_assert!(x < 100);
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].assertions, 3);
    }

    #[test]
    fn test_assertion_counting_all_variants() {
        let source = r#"
fn full_coverage(a: i32, b: i32) {
    assert!(a > 0);
    assert_eq!(a, b);
    assert_ne!(a, 0);
    debug_assert!(b > 0);
    debug_assert_eq!(a, b);
    debug_assert_ne!(b, 0);
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].assertions, 6);
    }

    #[test]
    fn test_assertion_density_in_summary() {
        let source = r#"
fn no_asserts() {
    let _ = 1 + 1;
}

fn one_assert() {
    assert!(true);
}

fn two_asserts() {
    assert!(true);
    assert_eq!(1, 1);
}
"#;
        let (fns, types) = analyze_source(source);
        let empty_dup = DuplicationReport {
            functions_fingerprinted: 0,
            exact_duplicates: Vec::new(),
            near_duplicates: Vec::new(),
            duplication_score: 0.0,
        };
        let summary = compute_summary(&fns, &types, &empty_dup, 0, 0);

        // 3 functions total; 2 have fewer than 2 assertions.
        assert_eq!(summary.function_count, 3);
        assert_eq!(summary.functions_under_2_assertions, 2);
        assert_eq!(summary.total_assertions, 3);
        assert!(
            (summary.mean_assertions_per_function - 1.0).abs() < f64::EPSILON,
            "expected mean 1.0, got {}",
            summary.mean_assertions_per_function,
        );
        // All 3 functions are production (non-test), each cyclomatic=1 => total=3.
        assert_eq!(summary.total_production_cyclomatic, 3);
        // production_lines is the sum of body lines for non-test functions.
        assert!(summary.production_lines > 0);
    }

    // -----------------------------------------------------------------------
    // Meaningful assertion filter tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_trivial_assertion_not_meaningful() {
        let source = r#"
fn example(x: i32) {
    assert!(true);
    assert!(x > 0);
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        // Both count as assertions.
        assert_eq!(fns[0].assertions, 2);
        // Only assert!(x > 0) is meaningful (references ident `x`).
        assert_eq!(fns[0].meaningful_assertions, 1);
    }

    #[test]
    fn test_meaningful_assertion_filter() {
        let source = r#"
fn trivial_only() {
    assert!(true);
    assert!(false);
    assert_eq!(1, 1);
}

fn meaningful_only(a: i32, b: i32) {
    assert!(a > 0);
    assert_eq!(a, b);
    assert_ne!(a, 0);
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 2);

        let trivial = &fns[0];
        assert_eq!(trivial.assertions, 3);
        assert_eq!(trivial.meaningful_assertions, 0);

        let meaningful = &fns[1];
        assert_eq!(meaningful.assertions, 3);
        assert_eq!(meaningful.meaningful_assertions, 3);
    }

    // -----------------------------------------------------------------------
    // Test function detection tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_test_detection() {
        let source = r#"
fn regular() {
    let _ = 1;
}

#[test]
fn test_something() {
    assert!(true);
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 2);

        let regular = fns.iter().find(|f| f.name == "regular").unwrap();
        assert!(!regular.is_test, "regular function should not be a test");

        let test_fn = fns.iter().find(|f| f.name == "test_something").unwrap();
        assert!(test_fn.is_test, "#[test] function should be a test");
    }

    #[test]
    fn test_cfg_test_module() {
        let source = r#"
fn production_code() {
    let _ = 1;
}

#[cfg(test)]
mod tests {
    fn helper() {
        let _ = 2;
    }

    #[test]
    fn test_it() {
        assert!(true);
    }
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 3);

        let prod = fns.iter().find(|f| f.name == "production_code").unwrap();
        assert!(!prod.is_test, "production function should not be a test");

        let helper = fns.iter().find(|f| f.name == "helper").unwrap();
        assert!(
            helper.is_test,
            "function inside #[cfg(test)] module should be a test"
        );

        let test_it = fns.iter().find(|f| f.name == "test_it").unwrap();
        assert!(
            test_it.is_test,
            "#[test] function inside #[cfg(test)] module should be a test"
        );
    }

    // -----------------------------------------------------------------------
    // Test density computation test
    // -----------------------------------------------------------------------

    #[test]
    fn test_test_density_computation() {
        let source = r#"
fn prod_simple() {
    let _ = 1;
}

fn prod_branchy(x: i32) {
    if x > 0 {
        let _ = x;
    }
}

#[test]
fn test_a() {
    assert!(true);
    assert_eq!(1, 1);
    assert!(true);
}
"#;
        let (fns, types) = analyze_source(source);
        let empty_dup = DuplicationReport {
            functions_fingerprinted: 0,
            exact_duplicates: Vec::new(),
            near_duplicates: Vec::new(),
            duplication_score: 0.0,
        };
        let summary = compute_summary(&fns, &types, &empty_dup, 0, 0);

        assert_eq!(summary.test_function_count, 1);
        assert_eq!(summary.production_function_count, 2);

        // prod_simple: cyclomatic=1, prod_branchy: cyclomatic=2 => total=3
        // test_a: 3 assertions
        // test_density = 3 / 3 = 1.0
        assert!(
            (summary.test_density - 1.0).abs() < f64::EPSILON,
            "expected test_density 1.0, got {}",
            summary.test_density,
        );
        // total_production_cyclomatic = 1 + 2 = 3.
        assert_eq!(summary.total_production_cyclomatic, 3);
        // production_lines covers the two prod functions.
        assert!(summary.production_lines > 0);
    }

    // -----------------------------------------------------------------------
    // Type cardinality tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_type_cardinality_bool_soup_vs_state_machine() {
        // Bool soup struct: bool * bool * bool = 2 * 2 * 2 = 8
        let source = r#"
struct BoolSoup {
    a: bool,
    b: bool,
    c: bool,
}

enum StateMachine {
    Disconnected,
    Connected { auth: bool },
    Error(String),
}
"#;
        let (_, types) = analyze_source(source);
        assert_eq!(types.len(), 2);

        let soup = types.iter().find(|t| t.name == "BoolSoup").unwrap();
        assert_eq!(soup.state_cardinality, 8);
        assert!(
            (soup.state_cardinality_log2 - 3.0).abs() < f64::EPSILON,
            "expected log2(8)=3.0, got {}",
            soup.state_cardinality_log2,
        );

        // State machine: max(1, 2, 1) = 2 (from Connected{auth:bool}).
        let sm = types.iter().find(|t| t.name == "StateMachine").unwrap();
        assert_eq!(sm.state_cardinality, 2);
        assert!(
            (sm.state_cardinality_log2 - 1.0).abs() < f64::EPSILON,
            "expected log2(2)=1.0, got {}",
            sm.state_cardinality_log2,
        );
    }

    #[test]
    fn test_type_cardinality_option_recurse() {
        // Option<bool> = 1 + 2 = 3
        // Option<Option<bool>> = 1 + (1 + 2) = 4
        // Option<u32> = 1 + 1 = 2
        let source = r#"
struct Opt1 { x: Option<bool> }
struct Opt2 { x: Option<Option<bool>> }
struct Opt3 { x: Option<u32> }
"#;
        let (_, types) = analyze_source(source);
        assert_eq!(types.len(), 3);

        let opt1 = types.iter().find(|t| t.name == "Opt1").unwrap();
        assert_eq!(opt1.state_cardinality, 3); // 1 + 2

        let opt2 = types.iter().find(|t| t.name == "Opt2").unwrap();
        assert_eq!(opt2.state_cardinality, 4); // 1 + (1 + 2)

        let opt3 = types.iter().find(|t| t.name == "Opt3").unwrap();
        assert_eq!(opt3.state_cardinality, 2); // 1 + 1
    }

    #[test]
    fn test_type_cardinality_result_recurse() {
        // Result<bool, Option<u8>> = 2 + (1 + 1) = 4
        let source = r#"
struct Res { x: Result<bool, Option<u8>> }
"#;
        let (_, types) = analyze_source(source);
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].state_cardinality, 4);
    }

    #[test]
    fn test_type_cardinality_named_type_conservative() {
        // struct Buzz { a: Foo, b: Option<Foo>, c: String, d: u64 }
        // Foo=1, Option<Foo>=1+1=2, String=1, u64=1 => 1 * 2 * 1 * 1 = 2
        let source = r#"
struct Buzz {
    a: Foo,
    b: Option<Foo>,
    c: String,
    d: u64,
}
"#;
        let (_, types) = analyze_source(source);
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].state_cardinality, 2);
    }

    #[test]
    fn test_type_cardinality_reference() {
        // &bool should be 2 (reference is transparent)
        let source = r#"
struct RefBool { x: &'static bool }
"#;
        let (_, types) = analyze_source(source);
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].state_cardinality, 2);
    }

    #[test]
    fn test_type_cardinality_tuple() {
        // (bool, bool) should be 2 * 2 = 4
        // () should be 1
        let source = r#"
struct TuplePair { x: (bool, bool) }
struct TupleUnit { x: () }
"#;
        let (_, types) = analyze_source(source);
        assert_eq!(types.len(), 2);

        let pair = types.iter().find(|t| t.name == "TuplePair").unwrap();
        assert_eq!(pair.state_cardinality, 4);

        let unit = types.iter().find(|t| t.name == "TupleUnit").unwrap();
        assert_eq!(unit.state_cardinality, 1);
    }

    // -----------------------------------------------------------------------
    // Internal state cardinality tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_internal_state_cardinality_bool_and_scalar() {
        // `let mut done = false` → bool → 2
        // `let mut count = 0` → scalar → 1
        // product = 2 * 1 = 2, log2(2) = 1.0
        let source = r#"
fn process() {
    let mut done = false;
    let mut count = 0;
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].mutable_bindings, 2);
        assert!(
            (fns[0].internal_state_cardinality_log2 - 1.0).abs() < f64::EPSILON,
            "expected 1.0, got {}",
            fns[0].internal_state_cardinality_log2,
        );
    }

    #[test]
    fn test_internal_state_cardinality_no_mut() {
        // No mutable bindings → 0.0.
        let source = r#"
fn pure(x: i32) -> i32 {
    let y = x + 1;
    y
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].mutable_bindings, 0);
        assert!(
            fns[0].internal_state_cardinality_log2.abs() < f64::EPSILON,
            "expected 0.0, got {}",
            fns[0].internal_state_cardinality_log2,
        );
    }

    #[test]
    fn test_internal_state_cardinality_option_result() {
        // `let mut x = None` → Option → 2
        // `let mut y = Some(0)` → Option → 2
        // `let mut z = Ok(1)` → Result → 2
        // product = 2 * 2 * 2 = 8, log2(8) = 3.0
        let source = r#"
fn complex() {
    let mut x = None;
    let mut y = Some(0);
    let mut z = Ok(1);
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].mutable_bindings, 3);
        assert!(
            (fns[0].internal_state_cardinality_log2 - 3.0).abs() < f64::EPSILON,
            "expected 3.0, got {}",
            fns[0].internal_state_cardinality_log2,
        );
    }

    #[test]
    fn test_internal_state_cardinality_all_scalars() {
        // All scalar → product = 1, log2(1) = 0.0
        let source = r#"
fn all_scalars() {
    let mut a = 0;
    let mut b = Vec::new();
    let mut c = String::new();
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].mutable_bindings, 3);
        assert!(
            fns[0].internal_state_cardinality_log2.abs() < f64::EPSILON,
            "expected 0.0 for all-scalar bindings, got {}",
            fns[0].internal_state_cardinality_log2,
        );
    }

    // -----------------------------------------------------------------------
    // Comment-aware line counting tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_count_code_lines_mixed() {
        let source = "\
fn main() {
    // This is a comment
    let x = 1;

    /* block comment */
    let y = 2;
    /*
     * multi-line
     * block comment
     */
    let z = 3;
}
";
        // Code lines: `fn main() {`, `let x = 1;`, `let y = 2;`, `let z = 3;`, `}`
        // Blank: 1 empty line
        // Comments: `// This is a comment`, `/* block comment */`, 3 lines of multi-line block
        assert_eq!(count_code_lines(source), 5);
    }

    #[test]
    fn test_count_code_lines_all_comments() {
        let source = "\
// comment 1
// comment 2
/* block
   comment */
";
        assert_eq!(count_code_lines(source), 0);
    }

    #[test]
    fn test_count_code_lines_all_code() {
        let source = "\
fn add(a: i32, b: i32) -> i32 {
    a + b
}
";
        assert_eq!(count_code_lines(source), 3);
    }

    #[test]
    fn test_count_code_lines_empty() {
        assert_eq!(count_code_lines(""), 0);
    }

    #[test]
    fn test_count_code_lines_blanks_only() {
        assert_eq!(count_code_lines("   \n\n  \n"), 0);
    }

    #[test]
    fn test_function_lines_exclude_comments() {
        let source = r#"
fn example() {
    // line comment
    let x = 1;

    /* block comment */
    let y = 2;
    x + y
}
"#;
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        let f = &fns[0];
        // Only code lines within the body: `fn example() {`, `let x = 1;`,
        // `let y = 2;`, `x + y`. Comments and blanks are excluded.
        assert_eq!(f.lines, 4, "expected 4 code lines, got {}", f.lines);
    }

    // -----------------------------------------------------------------------
    // Macro counting tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_macro_has_control_flow() {
        // Macros with control flow keywords should be detected.
        let with_if: proc_macro2::TokenStream =
            "($x:expr) => { if $x { 1 } else { 0 } }".parse().unwrap();
        assert!(macro_has_control_flow(&with_if));

        let with_match: proc_macro2::TokenStream =
            "($x:expr) => { match $x { _ => {} } }".parse().unwrap();
        assert!(macro_has_control_flow(&with_match));

        let with_while: proc_macro2::TokenStream = "($x:expr) => { while $x {} }".parse().unwrap();
        assert!(macro_has_control_flow(&with_while));

        let with_for: proc_macro2::TokenStream = "($x:expr) => { for i in $x {} }".parse().unwrap();
        assert!(macro_has_control_flow(&with_for));

        let with_loop: proc_macro2::TokenStream =
            "($x:expr) => { loop { break; } }".parse().unwrap();
        assert!(macro_has_control_flow(&with_loop));

        // Simple macro without control flow should not be detected.
        let simple: proc_macro2::TokenStream = "($x:expr) => { $x + 1 }".parse().unwrap();
        assert!(!macro_has_control_flow(&simple));
    }

    #[test]
    fn test_macro_counted_in_code_economy() {
        let source = r#"
pub fn api_entry(x: i32) -> i32 {
    x + 1
}

macro_rules! process {
    ($val:expr) => {
        if $val > 0 {
            $val * 2
        } else {
            0
        }
    };
}
"#;
        let (fns, types, macro_fn, macro_export) = analyze_source_with_macros(source);
        assert_eq!(macro_fn, 1, "macro with control flow should be counted");
        assert_eq!(macro_export, 0, "non-exported macro should not be public");

        let empty_dup = DuplicationReport {
            functions_fingerprinted: 0,
            exact_duplicates: Vec::new(),
            near_duplicates: Vec::new(),
            duplication_score: 0.0,
        };
        let summary = compute_summary(&fns, &types, &empty_dup, macro_fn, macro_export);
        assert_eq!(summary.macro_fn_count, 1);
        assert_eq!(summary.macro_export_fn_count, 0);
        // 1 pub fn + 0 macro_export = 1 public, 1 prod fn + 1 macro = 2 adjusted production
        // overhead = 2.0 / 1.0 = 2.0
        assert!(
            (summary.function_overhead_ratio - 2.0).abs() < f64::EPSILON,
            "expected overhead 2.0, got {}",
            summary.function_overhead_ratio,
        );
    }

    #[test]
    fn test_macro_export_counted_as_public() {
        let source = r#"
pub fn api_a(x: i32) -> i32 {
    x + 1
}

#[macro_export]
macro_rules! exported_logic {
    ($val:expr) => {
        match $val {
            0 => "zero",
            _ => "nonzero",
        }
    };
}
"#;
        let (fns, types, macro_fn, macro_export) = analyze_source_with_macros(source);
        assert_eq!(macro_fn, 1);
        assert_eq!(
            macro_export, 1,
            "#[macro_export] macro should be counted as public"
        );

        let empty_dup = DuplicationReport {
            functions_fingerprinted: 0,
            exact_duplicates: Vec::new(),
            near_duplicates: Vec::new(),
            duplication_score: 0.0,
        };
        let summary = compute_summary(&fns, &types, &empty_dup, macro_fn, macro_export);
        assert_eq!(summary.macro_fn_count, 1);
        assert_eq!(summary.macro_export_fn_count, 1);
        // 1 pub fn + 1 macro_export = 2 public, 1 prod fn + 1 macro = 2 adjusted production
        // overhead = 2.0 / 2.0 = 1.0
        assert!(
            (summary.function_overhead_ratio - 1.0).abs() < f64::EPSILON,
            "expected overhead 1.0, got {}",
            summary.function_overhead_ratio,
        );
    }

    #[test]
    fn test_macro_in_test_module_not_counted() {
        let source = r#"
pub fn api(x: i32) -> i32 {
    x
}

#[cfg(test)]
mod tests {
    macro_rules! test_helper {
        ($val:expr) => {
            if $val > 0 { $val } else { 0 }
        };
    }
}
"#;
        let (_, _, macro_fn, macro_export) = analyze_source_with_macros(source);
        assert_eq!(macro_fn, 0, "macro in test module should not be counted");
        assert_eq!(macro_export, 0);
    }

    #[test]
    fn test_simple_macro_not_counted() {
        let source = r#"
pub fn api(x: i32) -> i32 {
    x
}

macro_rules! just_add {
    ($a:expr, $b:expr) => {
        $a + $b
    };
}
"#;
        let (_, _, macro_fn, macro_export) = analyze_source_with_macros(source);
        assert_eq!(
            macro_fn, 0,
            "macro without control flow should not be counted"
        );
        assert_eq!(macro_export, 0);
    }

    // -----------------------------------------------------------------------
    // Scope path tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_scope_path_top_level_function() {
        let source = "fn foo() {}";
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        assert_eq!(
            fns[0].scope_path,
            vec![ScopeSegment::Function("foo".into())]
        );
        assert_eq!(fns[0].module_path, "");
    }

    #[test]
    fn test_scope_path_function_in_module() {
        let source = "mod bar { fn foo() {} }";
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        assert_eq!(
            fns[0].scope_path,
            vec![
                ScopeSegment::Module("bar".into()),
                ScopeSegment::Function("foo".into()),
            ]
        );
        assert_eq!(fns[0].module_path, "bar");
    }

    #[test]
    fn test_scope_path_method_in_impl() {
        let source = "struct Foo; impl Foo { fn bar() {} }";
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        assert_eq!(
            fns[0].scope_path,
            vec![
                ScopeSegment::Type("Foo".into()),
                ScopeSegment::Function("bar".into()),
            ]
        );
        assert_eq!(fns[0].module_path, "");
    }

    #[test]
    fn test_scope_path_nested_modules() {
        let source = "mod a { mod b { fn c() {} } }";
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        assert_eq!(
            fns[0].scope_path,
            vec![
                ScopeSegment::Module("a".into()),
                ScopeSegment::Module("b".into()),
                ScopeSegment::Function("c".into()),
            ]
        );
        assert_eq!(fns[0].module_path, "a::b");
    }

    #[test]
    fn test_scope_path_trait_default_method() {
        let source = "trait MyTrait { fn default_method() {} }";
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 1);
        assert_eq!(
            fns[0].scope_path,
            vec![
                ScopeSegment::Type("MyTrait".into()),
                ScopeSegment::Function("default_method".into()),
            ]
        );
        assert_eq!(fns[0].module_path, "");
    }

    #[test]
    fn test_nested_function_discovered() {
        let source = "fn outer() { fn inner() { let x = 1; } }";
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 2);
        let inner = fns.iter().find(|f| f.name == "inner").expect("inner fn");
        assert_eq!(
            inner.scope_path,
            vec![
                ScopeSegment::Function("outer".into()),
                ScopeSegment::Function("inner".into()),
            ]
        );
        assert_eq!(inner.module_path, "");
    }

    #[test]
    fn test_nested_struct_in_function() {
        let source = "fn outer() { struct Local { a: bool, b: bool } }";
        let (_, types) = analyze_source(source);
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].name, "Local");
        assert_eq!(
            types[0].scope_path,
            vec![
                ScopeSegment::Function("outer".into()),
                ScopeSegment::Type("Local".into()),
            ]
        );
    }

    #[test]
    fn test_nested_impl_in_function() {
        // Use a multi-character name so type_name_from_ty does not filter it,
        // and named fields so analyze_struct does not skip it.
        let source = "fn outer() { struct Local { x: bool } impl Local { fn method() {} } }";
        let (fns, types) = analyze_source(source);
        assert_eq!(types.len(), 1);
        let method = fns.iter().find(|f| f.name == "method").expect("method");
        assert_eq!(
            method.scope_path,
            vec![
                ScopeSegment::Function("outer".into()),
                ScopeSegment::Type("Local".into()),
                ScopeSegment::Function("method".into()),
            ]
        );
    }

    #[test]
    fn test_deeply_nested_functions() {
        let source = "fn a() { fn b() { fn c() {} } }";
        let (fns, _) = analyze_source(source);
        assert_eq!(fns.len(), 3);
        let c = fns.iter().find(|f| f.name == "c").expect("fn c");
        assert_eq!(
            c.scope_path,
            vec![
                ScopeSegment::Function("a".into()),
                ScopeSegment::Function("b".into()),
                ScopeSegment::Function("c".into()),
            ]
        );
    }
}
