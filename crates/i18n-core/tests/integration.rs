//! End-to-end tests: ProjectConfig auto-detection + IndexBuilder against real fixtures.

use std::path::PathBuf;

use i18n_core::{IndexBuilder, Locale, LocaleLayout, ProjectConfig};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn nested_project_builds_complete_index() {
    let root = fixture("nested_project");
    let config = ProjectConfig::auto_detect(&root);

    assert_eq!(config.locale_paths, vec!["locales".to_string()]);

    let index = IndexBuilder::new(&root, &config).build().expect("build");

    assert_eq!(index.layout, Some(LocaleLayout::Nested));
    assert_eq!(index.source_locale.as_str(), "en");
    assert_eq!(index.trees.len(), 2);
    assert!(index.trees.contains_key(&Locale::new("en")));
    assert!(index.trees.contains_key(&Locale::new("fr")));

    // Keys are namespaced by the file stem (`common`)
    let en_submit = index
        .lookup("common.actions.submit")
        .get(&Locale::new("en"))
        .copied()
        .expect("en submit");
    assert_eq!(en_submit.value, "Submit");

    let fr_greeting = index
        .lookup("common.greeting")
        .get(&Locale::new("fr"))
        .copied()
        .expect("fr greeting");
    assert_eq!(fr_greeting.value, "Bonjour");

    // `common.actions.cancel` is only in English → missing for French
    let missing = index.missing_keys(&Locale::new("fr"));
    assert_eq!(missing, vec!["common.actions.cancel".to_string()]);
}

#[test]
fn flat_project_builds_complete_index() {
    let root = fixture("flat_project");
    let config = ProjectConfig::auto_detect(&root);
    let index = IndexBuilder::new(&root, &config).build().expect("build");

    assert_eq!(index.layout, Some(LocaleLayout::Flat));
    assert_eq!(index.trees.len(), 2);

    let en_hello = index
        .lookup("hello")
        .get(&Locale::new("en"))
        .copied()
        .expect("en hello");
    assert_eq!(en_hello.value, "Hi");
    // Range should point inside the locale file
    assert!(en_hello.range.end.offset > en_hello.range.start.offset);
    assert!(en_hello.file.ends_with("en.json"));

    assert_eq!(index.missing_keys(&Locale::new("fr")), Vec::<String>::new());
}

#[test]
fn missing_locale_dir_yields_no_locales_error() {
    let root = fixture("nonexistent");
    let config = ProjectConfig::auto_detect(&root);
    let err = IndexBuilder::new(&root, &config).build().unwrap_err();
    match err {
        i18n_core::IndexError::NoLocalesFound => {}
        other => panic!("unexpected error: {other:?}"),
    }
}
