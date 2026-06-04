use std::path::PathBuf;
use i18n_core::{resolve_value, IndexBuilder, Locale, ProjectConfig, ResolvedValue};

#[test]
fn mg_shop_linked_payfip_resolves() {
    let root = PathBuf::from("/Users/medighoffer/DEV/MG_SHOP");
    if !root.is_dir() {
        return;
    }
    let config = ProjectConfig::load(&root);
    let index = IndexBuilder::new(&root, &config).build().expect("build");
    let fr = Locale::new("fr");
    let values = index.lookup("global.paymentModes.payfip");
    let alias = values.get(&fr).expect("alias");
    assert_eq!(alias.value, "@:global.providers.payfip");
    match resolve_value(&index, &fr, &alias.value) {
        ResolvedValue::Linked { display, .. } => assert_eq!(display, "PayFiP"),
        other => panic!("{other:?}"),
    }
    let display = index.display_for_locale(&fr, &alias.value);
    assert_eq!(display, "PayFiP");
}
