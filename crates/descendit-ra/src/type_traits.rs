//! Type/trait relationship extraction via rust-analyzer HIR.

use ra_ap_hir::db::HirDatabase;
use ra_ap_hir::{HasSource, Impl, Trait};
use ra_ap_syntax::AstNode;
use ra_ap_vfs::Vfs;

use crate::output::{SemanticData, TypeTraitFact, TypeTraitFactKind};

pub(crate) fn process_trait(
    db: &dyn HirDatabase,
    vfs: &Vfs,
    trait_: Trait,
    file: &str,
    module_path: &str,
    data: &mut SemanticData,
) {
    let name = trait_name(db, trait_);
    let line = trait_line_number(db, vfs, trait_);
    data.type_trait_facts.push(TypeTraitFact {
        file: file.to_owned(),
        module_path: module_path.to_owned(),
        line,
        kind: TypeTraitFactKind::TraitDecl { name: name.clone() },
    });

    for supertrait in trait_.direct_supertraits(db) {
        data.type_trait_facts.push(TypeTraitFact {
            file: file.to_owned(),
            module_path: module_path.to_owned(),
            line,
            kind: TypeTraitFactKind::TraitImplTrait {
                subject: name.clone(),
                target: trait_name(db, supertrait),
            },
        });
    }
}

pub(crate) fn process_impl(
    db: &dyn HirDatabase,
    vfs: &Vfs,
    impl_: Impl,
    file: &str,
    module_path: &str,
    data: &mut SemanticData,
) {
    let Some(trait_) = impl_.trait_(db) else {
        return;
    };
    let Some(source) = impl_.source(db) else {
        return;
    };
    let Some(self_ty) = source.value.self_ty() else {
        return;
    };

    data.type_trait_facts.push(TypeTraitFact {
        file: file.to_owned(),
        module_path: module_path.to_owned(),
        line: impl_line_number(db, vfs, impl_),
        kind: TypeTraitFactKind::TypeImplTrait {
            ty: type_name(module_path, self_ty.syntax().text().to_string()),
            tr: trait_name(db, trait_),
        },
    });
}

fn trait_name(db: &dyn HirDatabase, trait_: Trait) -> String {
    let module = crate::module_path_string(db, trait_.module(db));
    qualify_name(&module, trait_.name(db).as_str())
}

fn type_name(module_path: &str, source_text: String) -> String {
    let compact = source_text.split_whitespace().collect::<String>();
    qualify_name(module_path, &compact)
}

fn qualify_name(module_path: &str, name: &str) -> String {
    if module_path.is_empty() {
        name.to_owned()
    } else {
        format!("{module_path}::{name}")
    }
}

fn trait_line_number(db: &dyn HirDatabase, vfs: &Vfs, trait_: Trait) -> usize {
    let Some(source) = trait_.source(db) else {
        return 0;
    };
    let offset = source
        .value
        .trait_token()
        .map(|token| usize::from(token.text_range().start()))
        .unwrap_or_else(|| usize::from(source.value.syntax().text_range().start()));
    crate::line_number_for_source(vfs, db, source.file_id, offset)
}

fn impl_line_number(db: &dyn HirDatabase, vfs: &Vfs, impl_: Impl) -> usize {
    let Some(source) = impl_.source(db) else {
        return 0;
    };
    let offset = source
        .value
        .impl_token()
        .map(|token| usize::from(token.text_range().start()))
        .unwrap_or_else(|| usize::from(source.value.syntax().text_range().start()));
    crate::line_number_for_source(vfs, db, source.file_id, offset)
}
