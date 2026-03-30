//! Structural duplication detection for Rust functions.
//!
//! Detects structurally similar functions by comparing their AST "shape" --
//! ignoring identifiers but preserving control flow structure. Two functions
//! that do the same thing with different variable names are detected as
//! duplicates.
//!
//! # Approach
//!
//! 1. **Fingerprinting**: Walk each function body with `ShapeVisitor`, emitting
//!    a sequence of `ShapeToken`s that capture control flow and structure but
//!    not specific names or values.
//! 2. **Exact matching**: Hash the full token sequence and group functions with
//!    identical hashes.
//! 3. **Near-duplicate detection**: For non-trivial functions (>= 5 tokens),
//!    compute 3-gram sets and compare via Jaccard similarity. Only compare
//!    within size buckets (`min_len / max_len > 0.5`) for performance.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use serde::{Deserialize, Serialize};
use syn::visit::Visit;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Token representing a structural element of a function body.
///
/// These capture the "skeleton" of a function: control flow, binding patterns,
/// and expression kinds, but not specific identifiers, types, or values.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShapeToken {
    Let,
    LetMut,
    If,
    Else,
    Match,
    MatchArm,
    For,
    While,
    Loop,
    Return,
    Call,
    FieldAccess,
    IndexAccess,
    BinOp,
    UnaryOp,
    Assign,
    Block,
    Closure,
    Try,
    Macro,
    Literal,
    Reference,
    Await,
}

/// Structural fingerprint for a single function.
#[derive(Debug, Clone)]
pub struct FunctionFingerprint {
    pub name: String,
    pub file: String,
    pub line: usize,
    pub shape: Vec<ShapeToken>,
    pub scope_path: Vec<crate::metrics::ScopeSegment>,
}

/// Location of a function in the source tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionLocation {
    pub name: String,
    pub file: String,
    pub line: usize,
    #[serde(default)]
    pub scope_path: Vec<crate::metrics::ScopeSegment>,
}

/// A group of functions with exactly identical structural shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub shape_hash: u64,
    pub shape_length: usize,
    pub functions: Vec<FunctionLocation>,
}

/// A pair of functions with high but not exact structural similarity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearDuplicatePair {
    pub a: FunctionLocation,
    pub b: FunctionLocation,
    pub similarity: f64,
}

/// Complete duplication analysis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicationReport {
    /// Total functions fingerprinted.
    pub functions_fingerprinted: usize,
    /// Groups of exactly structurally identical functions.
    pub exact_duplicates: Vec<DuplicateGroup>,
    /// Pairs of near-duplicate functions (Jaccard > 0.8).
    pub near_duplicates: Vec<NearDuplicatePair>,
    /// Fraction of functions that are exact or near duplicates.
    pub duplication_score: f64,
}

// ---------------------------------------------------------------------------
// Fingerprinting
// ---------------------------------------------------------------------------

/// Generate a structural fingerprint for a function by visiting its body.
pub fn fingerprint_block(
    name: &str,
    file: &str,
    line: usize,
    block: &syn::Block,
    scope_path: Vec<crate::metrics::ScopeSegment>,
) -> FunctionFingerprint {
    let mut visitor = ShapeVisitor { tokens: Vec::new() };
    visitor.visit_block(block);

    FunctionFingerprint {
        name: name.to_string(),
        file: file.to_string(),
        line,
        shape: visitor.tokens,
        scope_path,
    }
}

/// AST visitor that emits shape tokens in depth-first order.
struct ShapeVisitor {
    tokens: Vec<ShapeToken>,
}

impl<'ast> Visit<'ast> for ShapeVisitor {
    fn visit_local(&mut self, node: &'ast syn::Local) {
        if pat_has_mut(&node.pat) {
            self.tokens.push(ShapeToken::LetMut);
        } else {
            self.tokens.push(ShapeToken::Let);
        }
        syn::visit::visit_local(self, node);
    }

    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        self.tokens.push(ShapeToken::If);
        // Visit the condition and then-branch.
        self.visit_expr(&node.cond);
        self.visit_block(&node.then_branch);
        // Emit Else token and visit the else-branch if present.
        if let Some((_, ref else_branch)) = node.else_branch {
            self.tokens.push(ShapeToken::Else);
            self.visit_expr(else_branch);
        }
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        self.tokens.push(ShapeToken::Match);
        syn::visit::visit_expr_match(self, node);
    }

    fn visit_arm(&mut self, node: &'ast syn::Arm) {
        self.tokens.push(ShapeToken::MatchArm);
        syn::visit::visit_arm(self, node);
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.tokens.push(ShapeToken::For);
        syn::visit::visit_expr_for_loop(self, node);
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.tokens.push(ShapeToken::While);
        syn::visit::visit_expr_while(self, node);
    }

    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.tokens.push(ShapeToken::Loop);
        syn::visit::visit_expr_loop(self, node);
    }

    fn visit_expr_return(&mut self, node: &'ast syn::ExprReturn) {
        self.tokens.push(ShapeToken::Return);
        syn::visit::visit_expr_return(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        self.tokens.push(ShapeToken::Call);
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        self.tokens.push(ShapeToken::Call);
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_field(&mut self, node: &'ast syn::ExprField) {
        self.tokens.push(ShapeToken::FieldAccess);
        syn::visit::visit_expr_field(self, node);
    }

    fn visit_expr_index(&mut self, node: &'ast syn::ExprIndex) {
        self.tokens.push(ShapeToken::IndexAccess);
        syn::visit::visit_expr_index(self, node);
    }

    fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
        self.tokens.push(ShapeToken::BinOp);
        syn::visit::visit_expr_binary(self, node);
    }

    fn visit_expr_unary(&mut self, node: &'ast syn::ExprUnary) {
        self.tokens.push(ShapeToken::UnaryOp);
        syn::visit::visit_expr_unary(self, node);
    }

    fn visit_expr_assign(&mut self, node: &'ast syn::ExprAssign) {
        self.tokens.push(ShapeToken::Assign);
        syn::visit::visit_expr_assign(self, node);
    }

    fn visit_expr_block(&mut self, node: &'ast syn::ExprBlock) {
        self.tokens.push(ShapeToken::Block);
        syn::visit::visit_expr_block(self, node);
    }

    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        self.tokens.push(ShapeToken::Closure);
        syn::visit::visit_expr_closure(self, node);
    }

    fn visit_expr_try(&mut self, node: &'ast syn::ExprTry) {
        self.tokens.push(ShapeToken::Try);
        syn::visit::visit_expr_try(self, node);
    }

    fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
        self.tokens.push(ShapeToken::Macro);
        syn::visit::visit_expr_macro(self, node);
    }

    fn visit_expr_lit(&mut self, node: &'ast syn::ExprLit) {
        self.tokens.push(ShapeToken::Literal);
        syn::visit::visit_expr_lit(self, node);
    }

    fn visit_expr_reference(&mut self, node: &'ast syn::ExprReference) {
        self.tokens.push(ShapeToken::Reference);
        syn::visit::visit_expr_reference(self, node);
    }

    fn visit_expr_await(&mut self, node: &'ast syn::ExprAwait) {
        self.tokens.push(ShapeToken::Await);
        syn::visit::visit_expr_await(self, node);
    }
}

/// Check whether a pattern contains a `mut` binding.
pub(crate) fn pat_has_mut(pat: &syn::Pat) -> bool {
    match pat {
        syn::Pat::Ident(pat_ident) => pat_ident.mutability.is_some(),
        syn::Pat::Tuple(pat_tuple) => pat_tuple.elems.iter().any(pat_has_mut),
        syn::Pat::TupleStruct(pat_ts) => pat_ts.elems.iter().any(pat_has_mut),
        syn::Pat::Struct(pat_struct) => pat_struct.fields.iter().any(|f| pat_has_mut(&f.pat)),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Duplicate detection
// ---------------------------------------------------------------------------

/// Minimum number of shape tokens for a function to participate in
/// near-duplicate analysis. Functions below this threshold are trivial.
const MIN_SHAPE_TOKENS: usize = 5;

/// Jaccard similarity threshold for near-duplicate detection.
const NEAR_DUPLICATE_THRESHOLD: f64 = 0.8;

/// Minimum length ratio for comparing two functions. Functions with very
/// different lengths cannot have high Jaccard similarity, so we skip them.
const MIN_LENGTH_RATIO: f64 = 0.5;

/// N-gram size for near-duplicate comparison.
const NGRAM_SIZE: usize = 3;

/// Detect exact and near-duplicate functions from a set of fingerprints.
pub fn detect_duplicates(fingerprints: &[FunctionFingerprint]) -> DuplicationReport {
    let (exact_duplicates, in_exact_group) = find_exact_duplicates(fingerprints);
    let near_duplicates = find_near_duplicates(fingerprints, &in_exact_group);
    let duplication_score =
        compute_duplication_score(fingerprints.len(), &exact_duplicates, &near_duplicates);

    DuplicationReport {
        functions_fingerprinted: fingerprints.len(),
        exact_duplicates,
        near_duplicates,
        duplication_score,
    }
}

/// Group functions by shape hash and return groups with 2+ members.
///
/// Also returns the set of fingerprint indices that belong to an exact group,
/// so that near-duplicate detection can exclude them.
fn find_exact_duplicates(
    fingerprints: &[FunctionFingerprint],
) -> (Vec<DuplicateGroup>, HashSet<usize>) {
    let mut hash_groups: HashMap<u64, Vec<usize>> = HashMap::new();
    for (index, fp) in fingerprints.iter().enumerate() {
        let hash = compute_shape_hash(&fp.shape);
        hash_groups.entry(hash).or_default().push(index);
    }

    // Collect groups with 2+ members AND shape_length >= MIN_SHAPE_TOKENS
    // as exact duplicates. Functions below the threshold are too simple
    // (e.g., empty bodies, trivial getters) to be meaningful duplication.
    let mut exact_duplicates: Vec<DuplicateGroup> = hash_groups
        .iter()
        .filter(|(_, indices)| {
            indices.len() >= 2 && fingerprints[indices[0]].shape.len() >= MIN_SHAPE_TOKENS
        })
        .map(|(&hash, indices)| {
            let mut functions: Vec<FunctionLocation> = indices
                .iter()
                .map(|&i| to_location(&fingerprints[i]))
                .collect();
            // Sort for deterministic output: by file, then line, then name.
            functions.sort_by(|a, b| {
                a.file
                    .cmp(&b.file)
                    .then(a.line.cmp(&b.line))
                    .then(a.name.cmp(&b.name))
            });
            DuplicateGroup {
                shape_hash: hash,
                shape_length: fingerprints[indices[0]].shape.len(),
                functions,
            }
        })
        .collect();

    // Sort groups deterministically: by shape length descending, then hash.
    exact_duplicates.sort_by(|a, b| {
        b.shape_length
            .cmp(&a.shape_length)
            .then(a.shape_hash.cmp(&b.shape_hash))
    });

    // Track which fingerprint indices are in exact-duplicate groups
    // (only those passing the minimum shape token threshold).
    let in_exact_group: HashSet<usize> = hash_groups
        .values()
        .filter(|indices| {
            indices.len() >= 2 && fingerprints[indices[0]].shape.len() >= MIN_SHAPE_TOKENS
        })
        .flat_map(|indices| indices.iter().copied())
        .collect();

    (exact_duplicates, in_exact_group)
}

/// Find near-duplicate pairs via 3-gram Jaccard similarity.
///
/// Only considers non-trivial functions (>= `MIN_SHAPE_TOKENS`) that are not
/// already in exact-duplicate groups. Uses a size-bucket optimization to skip
/// pairs with very different lengths.
fn find_near_duplicates(
    fingerprints: &[FunctionFingerprint],
    in_exact_group: &HashSet<usize>,
) -> Vec<NearDuplicatePair> {
    // Only consider non-trivial functions not already in exact groups.
    let ngram_sets: Vec<(usize, HashSet<Vec<ShapeToken>>)> = (0..fingerprints.len())
        .filter(|&i| {
            fingerprints[i].shape.len() >= MIN_SHAPE_TOKENS && !in_exact_group.contains(&i)
        })
        .map(|i| (i, ngrams(&fingerprints[i].shape, NGRAM_SIZE)))
        .collect();

    let mut near_duplicates: Vec<NearDuplicatePair> = Vec::new();

    for (ai, (idx_a, grams_a)) in ngram_sets.iter().enumerate() {
        let len_a = fingerprints[*idx_a].shape.len();

        for (idx_b, grams_b) in ngram_sets.iter().skip(ai + 1) {
            let len_b = fingerprints[*idx_b].shape.len();

            // Size-bucket optimization: skip pairs with very different lengths.
            let min_len = len_a.min(len_b);
            let max_len = len_a.max(len_b);
            if (min_len as f64 / max_len as f64) < MIN_LENGTH_RATIO {
                continue;
            }

            let similarity = jaccard_similarity(grams_a, grams_b);
            if similarity > NEAR_DUPLICATE_THRESHOLD {
                near_duplicates.push(NearDuplicatePair {
                    a: to_location(&fingerprints[*idx_a]),
                    b: to_location(&fingerprints[*idx_b]),
                    similarity,
                });
            }
        }
    }

    // Sort deterministically: by similarity descending, then by location.
    near_duplicates.sort_by(|x, y| {
        y.similarity
            .partial_cmp(&x.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(x.a.file.cmp(&y.a.file))
            .then(x.a.line.cmp(&y.a.line))
            .then(x.a.name.cmp(&y.a.name))
    });

    near_duplicates
}

/// Compute the fraction of functions that are exact or near duplicates.
fn compute_duplication_score(
    total_functions: usize,
    exact_duplicates: &[DuplicateGroup],
    near_duplicates: &[NearDuplicatePair],
) -> f64 {
    if total_functions == 0 {
        return 0.0;
    }

    let functions_in_exact: usize = exact_duplicates.iter().map(|g| g.functions.len()).sum();

    // Count unique functions in near-duplicate pairs.
    let mut near_dup_functions: HashSet<(String, String, usize)> = HashSet::new();
    for pair in near_duplicates {
        near_dup_functions.insert((pair.a.name.clone(), pair.a.file.clone(), pair.a.line));
        near_dup_functions.insert((pair.b.name.clone(), pair.b.file.clone(), pair.b.line));
    }

    let total_duplicated = functions_in_exact + near_dup_functions.len();
    total_duplicated as f64 / total_functions as f64
}

fn to_location(fp: &FunctionFingerprint) -> FunctionLocation {
    FunctionLocation {
        name: fp.name.clone(),
        file: fp.file.clone(),
        line: fp.line,
        scope_path: fp.scope_path.clone(),
    }
}

fn compute_shape_hash(shape: &[ShapeToken]) -> u64 {
    let mut hasher = DefaultHasher::new();
    shape.hash(&mut hasher);
    hasher.finish()
}

fn ngrams(shape: &[ShapeToken], n: usize) -> HashSet<Vec<ShapeToken>> {
    if shape.len() < n {
        return HashSet::new();
    }
    shape.windows(n).map(<[ShapeToken]>::to_vec).collect()
}

fn jaccard_similarity(a: &HashSet<Vec<ShapeToken>>, b: &HashSet<Vec<ShapeToken>>) -> f64 {
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Parse source and extract fingerprints for all functions.
    fn fingerprints_from_source(source: &str) -> Vec<FunctionFingerprint> {
        proc_macro2::fallback::force();
        let syntax =
            syn::parse_file(source).unwrap_or_else(|e| panic!("test source failed to parse: {e}"));

        let mut fingerprints = Vec::new();
        collect_fn_fingerprints(&syntax.items, "<test>", &mut fingerprints);
        fingerprints
    }

    /// Recursively extract function fingerprints from syn items.
    fn collect_fn_fingerprints(
        items: &[syn::Item],
        file: &str,
        out: &mut Vec<FunctionFingerprint>,
    ) {
        for item in items {
            match item {
                syn::Item::Fn(item_fn) => {
                    let name = item_fn.sig.ident.to_string();
                    let line = item_fn.sig.ident.span().start().line;
                    out.push(fingerprint_block(
                        &name,
                        file,
                        line,
                        &item_fn.block,
                        Vec::new(),
                    ));
                }
                syn::Item::Impl(item_impl) => {
                    for impl_item in &item_impl.items {
                        if let syn::ImplItem::Fn(method) = impl_item {
                            let name = method.sig.ident.to_string();
                            let line = method.sig.ident.span().start().line;
                            out.push(fingerprint_block(
                                &name,
                                file,
                                line,
                                &method.block,
                                Vec::new(),
                            ));
                        }
                    }
                }
                syn::Item::Mod(item_mod) => {
                    if let Some((_, ref mod_items)) = item_mod.content {
                        collect_fn_fingerprints(mod_items, file, out);
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    fn test_exact_duplicates() {
        // Two functions with the same structure but different names/variables.
        let source = r#"
fn process_a(x: i32) -> i32 {
    let result = x + 1;
    if result > 10 {
        return result * 2;
    }
    result
}

fn process_b(y: i32) -> i32 {
    let output = y + 1;
    if output > 10 {
        return output * 2;
    }
    output
}
"#;
        let fps = fingerprints_from_source(source);
        assert_eq!(fps.len(), 2);
        // The shape tokens should be identical.
        assert_eq!(fps[0].shape, fps[1].shape);

        let report = detect_duplicates(&fps);
        assert_eq!(report.exact_duplicates.len(), 1);
        assert_eq!(report.exact_duplicates[0].functions.len(), 2);
    }

    #[test]
    fn test_no_false_duplicates() {
        // Two functions with completely different structures.
        let source = r#"
fn linear(x: i32) -> i32 {
    let a = x + 1;
    let b = a * 2;
    b
}

fn branchy(x: i32) -> i32 {
    if x > 0 {
        match x {
            1 => 10,
            2 => 20,
            _ => 30,
        }
    } else {
        for i in 0..x {
            println!("{}", i);
        }
        0
    }
}
"#;
        let fps = fingerprints_from_source(source);
        assert_eq!(fps.len(), 2);
        // Shapes should differ.
        assert_ne!(fps[0].shape, fps[1].shape);

        let report = detect_duplicates(&fps);
        assert!(report.exact_duplicates.is_empty());
        assert!(report.near_duplicates.is_empty());
    }

    #[test]
    fn test_near_duplicates() {
        // Two functions that are mostly the same with one small structural
        // difference (an extra assignment in the second function).
        let source = r#"
fn version_a(x: i32) -> i32 {
    let a = x + 1;
    let b = a + 2;
    let c = b + 3;
    let d = c + 4;
    let e = d + 5;
    if e > 100 {
        return e;
    }
    e * 2
}

fn version_b(y: i32) -> i32 {
    let a = y + 1;
    let b = a + 2;
    let c = b + 3;
    let d = c + 4;
    let e = d + 5;
    let f = e + 6;
    if e > 100 {
        return e;
    }
    e * 2
}
"#;
        let fps = fingerprints_from_source(source);
        assert_eq!(fps.len(), 2);
        // Shapes should differ (not exact duplicates).
        assert_ne!(fps[0].shape, fps[1].shape);

        let report = detect_duplicates(&fps);
        assert!(report.exact_duplicates.is_empty());
        assert_eq!(report.near_duplicates.len(), 1);
        assert!(report.near_duplicates[0].similarity > NEAR_DUPLICATE_THRESHOLD);
    }

    #[test]
    fn test_trivial_functions_skipped() {
        // Functions with fewer than 5 shape tokens should not appear in
        // either exact-duplicate or near-duplicate analysis.
        let source = r#"
fn tiny_a(x: i32) -> i32 {
    x + 1
}

fn tiny_b(y: i32) -> i32 {
    y + 1
}

fn bigger(x: i32) -> i32 {
    let a = x + 1;
    let b = a * 2;
    let c = b + 3;
    if c > 10 {
        return c;
    }
    c
}
"#;
        let fps = fingerprints_from_source(source);
        assert_eq!(fps.len(), 3);

        // tiny_a and tiny_b have identical shapes but are below the threshold.
        assert_eq!(fps[0].shape, fps[1].shape);
        assert!(fps[0].shape.len() < MIN_SHAPE_TOKENS);

        let report = detect_duplicates(&fps);
        // Trivial functions are excluded from exact duplicate groups.
        assert!(report.exact_duplicates.is_empty());
        // No near-duplicates either (trivial excluded, bigger is unique).
        assert!(report.near_duplicates.is_empty());
        // No functions are counted as duplicated.
        assert_eq!(report.duplication_score, 0.0);
    }

    #[test]
    fn test_duplication_score() {
        // Create a scenario with known duplication to verify score calculation.
        let source = r#"
fn dup_a(x: i32) -> i32 {
    let a = x + 1;
    if a > 10 {
        return a * 2;
    }
    a
}

fn dup_b(y: i32) -> i32 {
    let b = y + 1;
    if b > 10 {
        return b * 2;
    }
    b
}

fn unique(x: i32) -> i32 {
    for i in 0..x {
        match i {
            0 => {}
            _ => {}
        }
    }
    x
}
"#;
        let fps = fingerprints_from_source(source);
        assert_eq!(fps.len(), 3);

        let report = detect_duplicates(&fps);
        // 2 out of 3 functions are exact duplicates.
        assert_eq!(report.exact_duplicates.len(), 1);
        assert_eq!(report.exact_duplicates[0].functions.len(), 2);
        // Score: 2 duplicated / 3 total.
        let expected_score = 2.0 / 3.0;
        assert!(
            (report.duplication_score - expected_score).abs() < 1e-10,
            "expected {expected_score}, got {}",
            report.duplication_score,
        );
    }

    #[test]
    fn test_empty_input() {
        let report = detect_duplicates(&[]);
        assert_eq!(report.functions_fingerprinted, 0);
        assert!(report.exact_duplicates.is_empty());
        assert!(report.near_duplicates.is_empty());
        assert_eq!(report.duplication_score, 0.0);
    }

    #[test]
    fn test_fingerprint_captures_structure() {
        // Verify that the shape visitor captures the expected tokens for a
        // known function body.
        let source = r#"
fn example(x: i32) -> i32 {
    let mut result = 0;
    if x > 0 {
        result = x + 1;
    }
    result
}
"#;
        let fps = fingerprints_from_source(source);
        assert_eq!(fps.len(), 1);

        let shape = &fps[0].shape;
        // Expected: LetMut, Literal, If, BinOp (x > 0), Assign, BinOp (x + 1), Literal
        assert!(shape.contains(&ShapeToken::LetMut));
        assert!(shape.contains(&ShapeToken::If));
        assert!(shape.contains(&ShapeToken::Assign));
        assert!(shape.contains(&ShapeToken::BinOp));
    }

    #[test]
    fn test_deterministic_output() {
        // Running detect_duplicates twice on the same input produces identical output.
        let source = r#"
fn alpha(x: i32) -> i32 {
    let a = x + 1;
    if a > 10 { return a; }
    a * 2
}

fn beta(y: i32) -> i32 {
    let b = y + 1;
    if b > 10 { return b; }
    b * 2
}
"#;
        let fps = fingerprints_from_source(source);

        let report1 = detect_duplicates(&fps);
        let report2 = detect_duplicates(&fps);

        // Serialize both to JSON for a thorough equality check.
        let json1 = serde_json::to_string(&report1).expect("serialize report1");
        let json2 = serde_json::to_string(&report2).expect("serialize report2");
        assert_eq!(json1, json2);
    }
}
