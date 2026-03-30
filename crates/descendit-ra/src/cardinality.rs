//! Type and function cardinality extraction via rust-analyzer HIR.

use ra_ap_hir::db::HirDatabase;
use ra_ap_hir::{Adt, Field, Function, HasSource, HasVisibility};
use ra_ap_syntax::AstNode;
use ra_ap_vfs::Vfs;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::output::{ResolvedFunctionCardinality, ResolvedTypeCardinality, SemanticData};

/// Process an ADT (struct/enum) and compute its type cardinality.
pub(crate) fn process_adt(
    db: &dyn HirDatabase,
    adt: &Adt,
    file_path: &str,
    module_path: &str,
    data: &mut SemanticData,
) {
    let name = match adt {
        Adt::Struct(s) => s.name(db).as_str().to_owned(),
        Adt::Enum(e) => e.name(db).as_str().to_owned(),
        Adt::Union(_) => return, // conservative: skip unions
    };

    let mut cache = HashMap::new();
    let mut in_progress = HashSet::new();
    let cardinality = resolve_adt_cardinality(db, adt, &mut cache, &mut in_progress, 0);

    let log2 = if cardinality <= 1 {
        0.0
    } else {
        (cardinality as f64).log2()
    };

    data.type_cardinalities.push(ResolvedTypeCardinality {
        file: file_path.to_string(),
        module_path: module_path.to_string(),
        name,
        cardinality_log2: log2,
    });
}

/// Process a function and compute its internal-state cardinality from `let mut` bindings.
pub(crate) fn process_function(
    db: &dyn HirDatabase,
    _vfs: &Vfs,
    func: &Function,
    file_path: &str,
    module_path: &str,
    _workspace_root: &Path,
    data: &mut SemanticData,
) {
    // Skip functions without bodies (trait declarations, extern fns).
    if !func.has_body(db) {
        return;
    }

    let name = func.name(db).as_str().to_owned();
    let line = crate::function_line_number(_vfs, db, func);

    // Walk the function body to find `let mut` bindings.
    // Use the semantic API to resolve binding types.
    let mut cardinality: u64 = 1;

    // Get the body expression and walk it for local definitions.
    if let Some(body_source) = func.source(db) {
        let file_id = body_source.file_id.original_file(db);
        let sema = ra_ap_hir::Semantics::new_dyn(db);

        // Parse the file and find our function.
        let source_file = sema.parse(file_id);
        let syntax = source_file.syntax();

        // Find the function node by matching text range.
        let fn_syntax = body_source.value.syntax().clone();

        // Walk the function body for let-mut bindings.
        for node in fn_syntax.descendants() {
            if let Some(let_stmt) = ra_ap_syntax::ast::LetStmt::cast(node.clone())
                && let Some(pat) = let_stmt.pat()
                && is_mut_binding(&pat)
                && let Some(ty) = resolve_let_type(&sema, &let_stmt, syntax)
            {
                let mut cache = HashMap::new();
                let mut in_progress = HashSet::new();
                let ty_card = resolve_type_cardinality(db, &ty, &mut cache, &mut in_progress, 0);
                cardinality = cardinality.saturating_mul(ty_card);
            }
        }
    }

    if cardinality > 1 {
        data.function_cardinalities
            .push(ResolvedFunctionCardinality {
                file: file_path.to_string(),
                module_path: module_path.to_string(),
                name,
                line,
                internal_state_cardinality_log2: (cardinality as f64).log2(),
            });
    }
}

fn is_mut_binding(pat: &ra_ap_syntax::ast::Pat) -> bool {
    match pat {
        ra_ap_syntax::ast::Pat::IdentPat(ident) => ident.mut_token().is_some(),
        _ => false,
    }
}

fn resolve_let_type<'db>(
    sema: &ra_ap_hir::Semantics<'db, dyn HirDatabase>,
    let_stmt: &ra_ap_syntax::ast::LetStmt,
    _file_syntax: &ra_ap_syntax::SyntaxNode,
) -> Option<ra_ap_hir::Type<'db>> {
    let pat = let_stmt.pat()?;
    if let ra_ap_syntax::ast::Pat::IdentPat(ident_pat) = pat {
        let ty = sema.type_of_pat(&ident_pat.into())?;
        Some(ty.original)
    } else {
        None
    }
}

const MAX_RESOLUTION_DEPTH: usize = 64;

fn resolve_adt_cardinality(
    db: &dyn HirDatabase,
    adt: &Adt,
    cache: &mut HashMap<String, u64>,
    in_progress: &mut HashSet<String>,
    depth: usize,
) -> u64 {
    if depth >= MAX_RESOLUTION_DEPTH {
        return 1;
    }

    let key = match adt {
        Adt::Struct(s) => format!("struct:{}", s.name(db).as_str()),
        Adt::Enum(e) => format!("enum:{}", e.name(db).as_str()),
        Adt::Union(_) => return 1,
    };

    if let Some(&cached) = cache.get(&key) {
        return cached;
    }
    if !in_progress.insert(key.clone()) {
        return 1; // cycle
    }

    let result = match adt {
        Adt::Struct(s) => {
            let mut card: u64 = 1;
            for field in s.fields(db) {
                if !is_field_visible(&field, db) {
                    continue;
                }
                let field_ty = field.ty(db).to_type(db);
                let field_card =
                    resolve_type_cardinality(db, &field_ty, cache, in_progress, depth + 1);
                card = card.saturating_mul(field_card);
            }
            card
        }
        Adt::Enum(e) => {
            let mut total: u64 = 0;
            for variant in e.variants(db) {
                let mut variant_card: u64 = 1;
                for field in variant.fields(db) {
                    let field_ty = field.ty(db).to_type(db);
                    let field_card =
                        resolve_type_cardinality(db, &field_ty, cache, in_progress, depth + 1);
                    variant_card = variant_card.saturating_mul(field_card);
                }
                total = total.saturating_add(variant_card);
            }
            total.max(1)
        }
        Adt::Union(_) => 1,
    };

    in_progress.remove(&key);
    cache.insert(key, result);
    result
}

fn is_field_visible(field: &Field, db: &dyn HirDatabase) -> bool {
    // In rust-analyzer, field visibility is available through the HasVisibility trait.
    // We treat private fields (only visible within the defining module) as
    // cardinality 1, since private fields are not part of the public state space.
    !matches!(field.visibility(db), ra_ap_hir::Visibility::Module(_, _))
}

fn resolve_type_cardinality(
    db: &dyn HirDatabase,
    ty: &ra_ap_hir::Type<'_>,
    cache: &mut HashMap<String, u64>,
    in_progress: &mut HashSet<String>,
    depth: usize,
) -> u64 {
    if depth >= MAX_RESOLUTION_DEPTH {
        return 1;
    }

    if ty.is_bool() {
        return 2;
    }

    // Primitives.
    if ty.is_int_or_uint() || ty.is_float() || ty.is_char() {
        return 1;
    }

    // str type — use the ADT name check or is_str if available.
    // Since Type has no display() with just db, we check via as_builtin.
    if ty.is_str() {
        return 1;
    }

    // References: cardinality of the referent.
    if ty.is_reference()
        && let Some(inner) = ty.as_reference()
    {
        return resolve_type_cardinality(db, &inner.0, cache, in_progress, depth + 1);
    }

    // Tuples: product of elements.
    if ty.is_tuple() {
        let mut card: u64 = 1;
        for field in ty.tuple_fields(db) {
            card = card.saturating_mul(resolve_type_cardinality(
                db,
                &field,
                cache,
                in_progress,
                depth + 1,
            ));
        }
        return card;
    }

    // ADT types.
    if let Some(adt) = ty.as_adt() {
        // Check for Option<T> / Result<T, E> by ADT name.
        let adt_name = match &adt {
            Adt::Struct(s) => s.name(db).as_str().to_owned(),
            Adt::Enum(e) => e.name(db).as_str().to_owned(),
            Adt::Union(_) => String::new(),
        };

        if adt_name == "Option" {
            let mut args = ty.type_arguments();
            if let Some(inner) = args.next() {
                let inner_card =
                    resolve_type_cardinality(db, &inner, cache, in_progress, depth + 1);
                return 1u64.saturating_add(inner_card);
            }
        }

        if adt_name == "Result" {
            let args: Vec<_> = ty.type_arguments().collect();
            if args.len() >= 2 {
                let ok_card = resolve_type_cardinality(db, &args[0], cache, in_progress, depth + 1);
                let err_card =
                    resolve_type_cardinality(db, &args[1], cache, in_progress, depth + 1);
                return ok_card.saturating_add(err_card);
            }
        }

        // Local ADT: recurse.
        if adt.module(db).krate(db).origin(db).is_local() {
            return resolve_adt_cardinality(db, &adt, cache, in_progress, depth + 1);
        }

        // External ADT: conservative.
        return 1;
    }

    // Arrays, slices, function pointers, closures, etc.: conservative.
    1
}
