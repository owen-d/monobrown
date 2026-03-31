//! Cross-module call graph extraction via rust-analyzer HIR.

use ra_ap_hir::db::HirDatabase;
use ra_ap_hir::{Function, HasSource};
use ra_ap_syntax::AstNode;
use ra_ap_vfs::Vfs;

use std::collections::BTreeSet;
use std::path::Path;

use crate::output::{CallEdge, SemanticData};
use crate::{module_path_string, normalize_path};

/// Caller-side context for recording a call edge.
struct CallerContext<'a> {
    file: &'a str,
    module: &'a str,
    function: &'a str,
    line: usize,
}

/// Process a function and extract cross-module call edges.
pub(crate) fn process_function(
    db: &dyn HirDatabase,
    vfs: &Vfs,
    func: &Function,
    file_path: &str,
    module_path: &str,
    workspace_root: &Path,
    data: &mut SemanticData,
) {
    if !func.has_body(db) {
        return;
    }

    let name = func.name(db).as_str().to_owned();
    let line = crate::function_line_number(vfs, db, func);

    // Get the function body source for walking call expressions.
    let Some(body_source) = func.source(db) else {
        return;
    };

    // Skip functions defined inside macro expansions — their syntax nodes
    // belong to the macro's parse tree, not the original file's, so
    // Semantics cannot resolve paths within them.
    if body_source.file_id.file_id().is_none() {
        return;
    }

    let file_id = body_source.file_id.original_file(db);
    let sema = ra_ap_hir::Semantics::new_dyn(db);
    let _source_file = sema.parse(file_id);

    let fn_syntax = body_source.value.syntax().clone();

    let mut seen_edges: BTreeSet<CallEdge> = BTreeSet::new();

    // Walk for call expressions and method calls.
    for node in fn_syntax.descendants() {
        let caller = CallerContext {
            file: file_path,
            module: module_path,
            function: &name,
            line,
        };

        // Direct function calls.
        if let Some(call_expr) = ra_ap_syntax::ast::CallExpr::cast(node.clone())
            && let Some(callee_expr) = call_expr.expr()
            && let Some(path_expr) = ra_ap_syntax::ast::PathExpr::cast(callee_expr.syntax().clone())
            && let Some(path) = path_expr.path()
            && let Some(resolved) = sema.resolve_path(&path)
            && let ra_ap_hir::PathResolution::Def(ra_ap_hir::ModuleDef::Function(callee_fn)) =
                resolved
        {
            record_edge(
                db,
                vfs,
                &callee_fn,
                &caller,
                workspace_root,
                &mut seen_edges,
            );
        }

        // Method calls.
        if let Some(method_call) = ra_ap_syntax::ast::MethodCallExpr::cast(node.clone())
            && let Some(callee_fn) = sema.resolve_method_call(&method_call)
        {
            record_edge(
                db,
                vfs,
                &callee_fn,
                &caller,
                workspace_root,
                &mut seen_edges,
            );
        }
    }

    data.call_edges.extend(seen_edges);
}

fn record_edge(
    db: &dyn HirDatabase,
    vfs: &Vfs,
    callee_fn: &Function,
    caller: &CallerContext<'_>,
    workspace_root: &Path,
    edges: &mut BTreeSet<CallEdge>,
) {
    // Only track local (same-crate) calls.
    let callee_module_obj = callee_fn.module(db);
    if !callee_module_obj.krate(db).origin(db).is_local() {
        return;
    }

    let callee_module_path = module_path_string(db, callee_module_obj);

    // Skip intra-module calls.
    if caller.module == callee_module_path {
        return;
    }

    // Get callee file path.
    let callee_file = callee_module_obj
        .definition_source_file_id(db)
        .file_id()
        .and_then(|editioned_fid| {
            let fid = editioned_fid.file_id(db);
            let path = vfs.file_path(fid);
            let abs_path = path.as_path()?;
            Some(normalize_path(abs_path.as_ref(), workspace_root))
        })
        .unwrap_or_default();

    let edge = CallEdge {
        caller_module: caller.module.to_string(),
        caller_file: caller.file.to_string(),
        caller_function: caller.function.to_string(),
        caller_line: caller.line,
        callee_module: callee_module_path,
        callee_file,
    };

    edges.insert(edge);
}
