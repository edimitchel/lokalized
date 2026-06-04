mod app;

use std::path::PathBuf;

pub fn run(workspace: PathBuf) -> anyhow::Result<i32> {
    let snapshot = i18n_core::ProjectSnapshot::load(&workspace).map_err(|e| {
        anyhow::anyhow!("failed to load project at {}: {e}", workspace.display())
    })?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 700.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Lokalized",
        options,
        Box::new(|cc| Ok(Box::new(app::LokalizedApp::new(cc, workspace, snapshot)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(0)
}