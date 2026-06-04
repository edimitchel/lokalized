//! End-to-end tests: ProjectConfig auto-detection + IndexBuilder against real fixtures.

use std::path::PathBuf;

use i18n_core::{resolve_value, IndexBuilder, Locale, LocaleLayout, ProjectConfig, ResolvedValue};

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
fn flat_yaml_project_builds_complete_index() {
    let root = fixture("flat_project_yaml");
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
    assert!(en_hello.file.ends_with("en.yml"));
    assert!(en_hello.range.end.offset > en_hello.range.start.offset);
}

#[test]
fn nested_yaml_project_builds_complete_index() {
    let root = fixture("nested_project_yaml");
    let config = ProjectConfig::auto_detect(&root);
    let index = IndexBuilder::new(&root, &config).build().expect("build");

    assert_eq!(index.layout, Some(LocaleLayout::Nested));
    let en_submit = index
        .lookup("common.actions.submit")
        .get(&Locale::new("en"))
        .copied()
        .expect("en submit");
    assert_eq!(en_submit.value, "Submit");
    assert!(en_submit.file.ends_with("common.yml"));

    let missing = index.missing_keys(&Locale::new("fr"));
    assert_eq!(missing, vec!["common.actions.cancel".to_string()]);
}

#[test]
fn monorepo_project_finds_front_locale_dir() {
    let root = fixture("monorepo_project");
    let config = ProjectConfig::auto_detect(&root);
    assert!(
        config
            .locale_paths
            .iter()
            .any(|p| p == "front/i18n/locales"),
        "locale_paths = {:?}",
        config.locale_paths
    );

    let index = IndexBuilder::new(&root, &config).build().expect("build");
    assert_eq!(index.source_locale.as_str(), "fr");
    let hello = index
        .lookup("app.hello")
        .get(&Locale::new("fr"))
        .copied()
        .expect("fr hello");
    assert_eq!(hello.value, "bonjour");
}

#[test]
fn namespace_false_keeps_json_root_keys_without_filename_prefix() {
    use std::fs;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let locales = dir.path().join("locales");
    fs::create_dir_all(locales.join("en")).unwrap();
    fs::write(
        locales.join("en/slots.json"),
        r#"{"slots":{"title":"Title","save":"Save"}}"#,
    )
    .unwrap();

    let config = ProjectConfig {
        locale_paths: vec!["locales".into()],
        namespace: Some(false),
        ..ProjectConfig::default()
    };
    let index = IndexBuilder::new(dir.path(), &config)
        .build()
        .expect("build");

    assert!(
        index.lookup("slots.title").contains_key(&Locale::new("en")),
        "expected slots.title without double prefix"
    );
    assert!(index.lookup("slots.slots.title").is_empty());
}

#[test]
fn namespace_true_avoids_double_prefix_when_json_is_self_wrapped() {
    use std::fs;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let locales = dir.path().join("locales");
    fs::create_dir_all(locales.join("en")).unwrap();
    fs::write(
        locales.join("en/slots.json"),
        r#"{"slots":{"title":"Title"}}"#,
    )
    .unwrap();

    let config = ProjectConfig {
        locale_paths: vec!["locales".into()],
        namespace: Some(true),
        ..ProjectConfig::default()
    };
    let index = IndexBuilder::new(dir.path(), &config)
        .build()
        .expect("build");

    assert!(index.lookup("slots.title").contains_key(&Locale::new("en")));
    assert!(index.lookup("slots.slots.title").is_empty());
}

#[test]
fn namespace_true_prepends_stem_for_flat_json_content() {
    use std::fs;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let locales = dir.path().join("locales");
    fs::create_dir_all(locales.join("en")).unwrap();
    fs::write(
        locales.join("en/common.json"),
        r#"{"actions":{"submit":"Submit"}}"#,
    )
    .unwrap();

    let config = ProjectConfig {
        locale_paths: vec!["locales".into()],
        namespace: Some(true),
        ..ProjectConfig::default()
    };
    let index = IndexBuilder::new(dir.path(), &config)
        .build()
        .expect("build");

    assert!(index
        .lookup("common.actions.submit")
        .contains_key(&Locale::new("en")));
}

#[test]
fn nuxt_per_locale_folder_indexes_global_cant_select() {
    use std::fs;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let fr = dir.path().join("locales/fr");
    fs::create_dir_all(&fr).unwrap();
    fs::write(
        fr.join("global.json"),
        r#"{"global":{"cantSelect":"nope"}}"#,
    )
    .unwrap();

    let config = ProjectConfig {
        locale_paths: vec!["locales/fr".into()],
        namespace: Some(true),
        source_locale: Some("fr".into()),
        ..ProjectConfig::default()
    };
    let index = IndexBuilder::new(dir.path(), &config)
        .build()
        .expect("build");
    assert_eq!(index.source_locale.as_str(), "fr");
    assert!(
        index
            .lookup("global.cantSelect")
            .contains_key(&Locale::new("fr")),
        "trees={:?}",
        index.trees.keys().collect::<Vec<_>>()
    );
}

#[test]
fn linked_messages_resolve_through_index() {
    let root = fixture("linked_messages");
    let config = ProjectConfig {
        locale_paths: vec!["locales".into()],
        namespace: Some(false),
        source_locale: Some("en".into()),
        ..Default::default()
    };
    let index = IndexBuilder::new(&root, &config).build().expect("build");

    let values = index.lookup("common.payfip");
    let alias = values.get(&Locale::new("en")).expect("alias");
    assert_eq!(alias.value, "@:common.providers.payfip");

    match resolve_value(&index, &Locale::new("en"), &alias.value) {
        ResolvedValue::Linked {
            display,
            target_key,
            ..
        } => {
            assert_eq!(target_key, "common.providers.payfip");
            assert_eq!(display, "PayFiP");
        }
        other => panic!("expected Linked, got {other:?}"),
    }

    let backlinks = index.keys_linking_to("common.providers.payfip");
    assert!(backlinks.iter().any(|k| k == "common.payfip"));
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

#[test]
fn wrong_locale_paths_from_front_subfolder_breaks_index() {
    let root = PathBuf::from("/Users/medighoffer/DEV/MG_SHOP/front");
    if !root.is_dir() {
        return;
    }
    let config = ProjectConfig::load(&PathBuf::from("/Users/medighoffer/DEV/MG_SHOP"));
    let err = IndexBuilder::new(&root, &config).build().unwrap_err();
    match err {
        i18n_core::IndexError::NoLocalesFound => {}
        other => panic!("expected NoLocalesFound, got {other:?}"),
    }
}
