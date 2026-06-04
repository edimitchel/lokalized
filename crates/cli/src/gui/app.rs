use std::collections::HashSet;
use std::path::PathBuf;

use egui::{Color32, RichText, ScrollArea, TextEdit, Ui};
use i18n_core::{set_key_json, Locale, ProjectSnapshot};

pub struct LokalizedApp {
    workspace: PathBuf,
    snapshot: ProjectSnapshot,
    search: String,
    locales: Vec<Locale>,
    keys: Vec<String>,
    selected_key: Option<String>,
    edit_buffer: String,
    editing: Option<EditTarget>,
    status_message: String,
    error_message: Option<String>,
    unused_keys: HashSet<String>,
}

#[derive(Clone)]
struct EditTarget {
    key: String,
    locale: Locale,
    file: PathBuf,
}

impl LokalizedApp {
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        workspace: PathBuf,
        snapshot: ProjectSnapshot,
    ) -> Self {
        let locales: Vec<_> = snapshot.index.trees.keys().cloned().collect();
        let mut keys = snapshot.index.all_keys();
        keys.sort();
        Self {
            workspace,
            snapshot,
            search: String::new(),
            locales,
            keys,
            selected_key: None,
            edit_buffer: String::new(),
            editing: None,
            status_message: String::new(),
            error_message: None,
            unused_keys: HashSet::new(),
        }
    }

    fn reload(&mut self) {
        match ProjectSnapshot::load(&self.workspace) {
            Ok(snap) => {
                self.locales = snap.index.trees.keys().cloned().collect();
                self.keys = snap.index.all_keys();
                self.keys.sort();
                self.snapshot = snap;
                self.unused_keys = self.snapshot.unused_keys().into_iter().collect();
                self.error_message = None;
                let missing: usize = self
                    .snapshot
                    .missing_by_locale(None)
                    .values()
                    .map(|v| v.len())
                    .sum();
                let unused = self.snapshot.unused_keys().len();
                self.status_message = format!(
                    "{} keys · {} missing · {} unused",
                    self.keys.len(),
                    missing,
                    unused
                );
            }
            Err(e) => self.error_message = Some(e.to_string()),
        }
    }

    fn filtered_keys(&self) -> Vec<&String> {
        let needle = self.search.to_lowercase();
        self.keys
            .iter()
            .filter(|k| needle.is_empty() || k.to_lowercase().contains(&needle))
            .collect()
    }

    fn start_edit(&mut self, key: String, locale: Locale) {
        let values = self.snapshot.index.lookup(&key);
        let Some(val) = values.get(&locale) else {
            self.error_message = Some(format!("no value for {key} in {locale}"));
            return;
        };
        self.edit_buffer = val.value.clone();
        self.editing = Some(EditTarget {
            key: key.clone(),
            locale,
            file: val.file.clone(),
        });
        self.selected_key = Some(key);
    }

    fn commit_edit(&mut self) {
        let Some(target) = self.editing.clone() else {
            return;
        };
        let ext = target
            .file
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !matches!(ext, "json" | "jsonc" | "json5") {
            self.error_message = Some(format!(
                "only JSON locale files can be edited (got .{ext})"
            ));
            return;
        }
        match std::fs::read_to_string(&target.file)
            .and_then(|content| {
                let segments: Vec<&str> = target.key.split('.').collect();
                set_key_json(&content, &segments, &self.edit_buffer).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                })
            })
            .and_then(|new_content| std::fs::write(&target.file, new_content))
        {
            Ok(()) => {
                self.editing = None;
                self.reload();
                self.status_message = format!("saved {} [{}]", target.key, target.locale);
            }
            Err(e) => self.error_message = Some(e.to_string()),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellState {
    Ok,
    Missing,
    Unused,
}

impl CellState {
    fn bg(self) -> Color32 {
        match self {
            Self::Ok => Color32::TRANSPARENT,
            Self::Missing => Color32::from_rgb(80, 30, 30),
            Self::Unused => Color32::from_rgb(50, 50, 55),
        }
    }
}

impl eframe::App for LokalizedApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.status_message.is_empty() {
            self.reload();
        }

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Lokalized");
                ui.separator();
                ui.label(format!("{}", self.workspace.display()));
                ui.separator();
                if ui.button("Refresh").clicked() {
                    self.reload();
                }
                ui.separator();
                ui.label(&self.status_message);
            });
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.add(TextEdit::singleline(&mut self.search).desired_width(280.0));
            });
            if let Some(err) = &self.error_message {
                ui.colored_label(Color32::LIGHT_RED, err);
            }
        });

        egui::SidePanel::left("keys")
            .resizable(true)
            .default_width(220.0)
            .show(ctx, |ui| {
                ui.heading("Keys");
                let keys: Vec<String> = self.filtered_keys().into_iter().cloned().collect();
                ScrollArea::vertical().show(ui, |ui| {
                    for key in &keys {
                        let selected = self.selected_key.as_deref() == Some(key.as_str());
                        if ui.selectable_label(selected, key.as_str()).clicked() {
                            self.selected_key = Some(key.clone());
                            self.editing = None;
                        }
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_table(ui);
            if self.editing.is_some() {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Edit value:");
                    ui.add(
                        TextEdit::multiline(&mut self.edit_buffer)
                            .desired_width(f32::INFINITY)
                            .desired_rows(3),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.commit_edit();
                    }
                    if ui.button("Cancel").clicked() {
                        self.editing = None;
                    }
                });
            }
        });
    }
}

impl LokalizedApp {
    fn render_table(&mut self, ui: &mut Ui) {
        let keys: Vec<String> = self.filtered_keys().into_iter().cloned().collect();
        let locales = self.locales.clone();
        let selected = self.selected_key.clone();
        let snapshot = self.snapshot.clone();
        let unused_keys = self.unused_keys.clone();

        ScrollArea::both().show(ui, |ui| {
            egui::Grid::new("translations")
                .striped(true)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("key").strong());
                    for loc in &locales {
                        ui.label(RichText::new(loc.as_str()).strong());
                    }
                    ui.end_row();

                    for key in &keys {
                        let row_selected = selected.as_deref() == Some(key.as_str());
                        if row_selected {
                            ui.label(RichText::new(key).strong());
                        } else {
                            ui.label(key);
                        }
                        for loc in &locales {
                            let state = cell_state(&snapshot, &unused_keys, key, loc);
                            let values = snapshot.index.lookup(key);
                            let text = values
                                .get(loc)
                                .map(|v| truncate(&v.value, 48))
                                .unwrap_or_else(|| "—".to_string());
                            let fill = state.bg();
                            let label = egui::Label::new(text).sense(egui::Sense::click());
                            let response = if fill != Color32::TRANSPARENT {
                                egui::Frame::new()
                                    .fill(fill)
                                    .inner_margin(4.0)
                                    .show(ui, |ui| ui.add(label))
                                    .inner
                            } else {
                                ui.add(label)
                            };
                            if response.clicked() && values.contains_key(loc) {
                                self.start_edit(key.clone(), loc.clone());
                            }
                        }
                        ui.end_row();
                    }
                });
        });
    }
}

fn cell_state(
    snapshot: &ProjectSnapshot,
    unused_keys: &HashSet<String>,
    key: &str,
    locale: &Locale,
) -> CellState {
    let values = snapshot.index.lookup(key);
    if !values.contains_key(locale) {
        return CellState::Missing;
    }
    if unused_keys.contains(key) {
        return CellState::Unused;
    }
    CellState::Ok
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}