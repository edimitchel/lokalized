use std::collections::HashSet;
use std::path::PathBuf;

use egui::{Align, Layout, RichText, ScrollArea, Ui};
use i18n_core::{Locale, ProjectSnapshot};

use super::actions::{add_key_project, delete_key_project, rename_key_project, write_value};
use super::key_tree::{missing_key_set, KeyTreeRoot, TreeView};
use super::theme::{self, Palette};

pub struct LokalizedApp {
    workspace: PathBuf,
    snapshot: ProjectSnapshot,
    palette: Palette,
    search: String,
    locales: Vec<Locale>,
    key_tree: KeyTreeRoot,
    expanded_branches: HashSet<String>,
    selected_key: Option<String>,
    locale_drafts: Vec<(Locale, String, bool)>,
    unused_keys: HashSet<String>,
    missing_keys: HashSet<String>,
    key_count: usize,
    locale_count: usize,
    missing_count: usize,
    unused_count: usize,
    status_message: Option<String>,
    error_message: Option<String>,
    show_add_key: bool,
    new_key_path: String,
    new_key_value: String,
    rename_key_buffer: String,
    show_rename: bool,
}

impl LokalizedApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        workspace: PathBuf,
        snapshot: ProjectSnapshot,
    ) -> Self {
        let palette = theme::apply(&cc.egui_ctx);
        let mut app = Self {
            workspace,
            snapshot,
            palette,
            search: String::new(),
            locales: Vec::new(),
            key_tree: KeyTreeRoot::default(),
            expanded_branches: HashSet::new(),
            selected_key: None,
            locale_drafts: Vec::new(),
            unused_keys: HashSet::new(),
            missing_keys: HashSet::new(),
            key_count: 0,
            locale_count: 0,
            missing_count: 0,
            unused_count: 0,
            status_message: None,
            error_message: None,
            show_add_key: false,
            new_key_path: String::new(),
            new_key_value: String::new(),
            rename_key_buffer: String::new(),
            show_rename: false,
        };
        app.rebuild_from_snapshot();
        app
    }

    fn rebuild_from_snapshot(&mut self) {
        self.locales = self.snapshot.index.trees.keys().cloned().collect();
        self.key_tree = KeyTreeRoot::from_index(&self.snapshot.index);
        self.unused_keys = self.snapshot.unused_keys().into_iter().collect();
        self.missing_keys = missing_key_set(&self.snapshot.index);
        self.key_count = self.snapshot.index.all_keys().len();
        self.locale_count = self.locales.len();
        self.missing_count = self
            .snapshot
            .missing_by_locale(None)
            .values()
            .map(|v| v.len())
            .sum();
        self.unused_count = self.unused_keys.len();

        if let Some(key) = self.selected_key.clone() {
            if self.snapshot.index.lookup(&key).is_empty() {
                self.selected_key = None;
                self.locale_drafts.clear();
            } else {
                self.load_drafts_for_key(&key);
            }
        }
    }

    fn reload(&mut self) {
        match ProjectSnapshot::load(&self.workspace) {
            Ok(snap) => {
                self.snapshot = snap;
                self.rebuild_from_snapshot();
                self.error_message = None;
            }
            Err(e) => self.error_message = Some(e.to_string()),
        }
    }

    fn load_drafts_for_key(&mut self, key: &str) {
        self.locale_drafts = self
            .locales
            .iter()
            .map(|locale| {
                let text = self
                    .snapshot
                    .index
                    .lookup(key)
                    .get(locale)
                    .map(|v| v.value.clone())
                    .unwrap_or_default();
                let missing = !self.snapshot.index.lookup(key).contains_key(locale);
                (locale.clone(), text, missing)
            })
            .collect();
        self.rename_key_buffer = key.to_string();
    }

    fn select_key(&mut self, key: String) {
        self.selected_key = Some(key.clone());
        self.show_rename = false;
        self.load_drafts_for_key(&key);
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some(msg.into());
        self.error_message = None;
    }

    fn save_locale(&mut self, locale: &Locale, key: &str, value: &str) {
        match write_value(&self.snapshot.index, key, locale, value) {
            Ok(()) => {
                self.reload();
                self.set_status(format!("Saved · {locale}"));
            }
            Err(e) => self.error_message = Some(e.to_string()),
        }
    }

    fn save_all_locales(&mut self) {
        let Some(key) = self.selected_key.clone() else {
            return;
        };
        let drafts: Vec<_> = self
            .locale_drafts
            .iter()
            .map(|(l, v, _)| (l.clone(), v.clone()))
            .collect();
        let mut errors = Vec::new();
        for (locale, value) in drafts {
            if let Err(e) = write_value(&self.snapshot.index, &key, &locale, &value) {
                errors.push(format!("{locale}: {e}"));
            }
        }
        self.reload();
        if errors.is_empty() {
            self.set_status("All locales saved");
        } else {
            self.error_message = Some(errors.join("\n"));
        }
    }

    fn commit_rename(&mut self) {
        let Some(old) = self.selected_key.clone() else {
            return;
        };
        let new = self.rename_key_buffer.trim().to_string();
        match rename_key_project(&self.snapshot, &old, &new) {
            Ok(n) => {
                self.selected_key = Some(new);
                self.show_rename = false;
                self.reload();
                self.set_status(format!("Renamed in {n} file(s)"));
            }
            Err(e) => self.error_message = Some(e.to_string()),
        }
    }

    fn commit_delete(&mut self) {
        let Some(key) = self.selected_key.clone() else {
            return;
        };
        match delete_key_project(&self.snapshot, &key) {
            Ok(n) => {
                self.selected_key = None;
                self.locale_drafts.clear();
                self.reload();
                self.set_status(format!("Deleted from {n} file(s)"));
            }
            Err(e) => self.error_message = Some(e.to_string()),
        }
    }

    fn commit_add_key(&mut self) {
        let path = self.new_key_path.trim().to_string();
        let value = self.new_key_value.trim().to_string();
        match add_key_project(&self.snapshot, &path, &value) {
            Ok(n) => {
                self.show_add_key = false;
                self.new_key_path.clear();
                self.new_key_value.clear();
                self.reload();
                self.select_key(path);
                self.set_status(format!("Created in {n} locale(s)"));
            }
            Err(e) => self.error_message = Some(e.to_string()),
        }
    }

    fn fill_missing_from_source(&mut self) {
        let Some(key) = self.selected_key.clone() else {
            return;
        };
        let source = self.snapshot.index.source_locale.clone();
        let entries = self.snapshot.index.lookup(&key);
        let Some(value) = entries.get(&source).map(|v| v.value.clone()) else {
            self.error_message = Some("No value in source locale".to_string());
            return;
        };
        for (locale, draft, missing) in &mut self.locale_drafts {
            if *missing {
                match write_value(&self.snapshot.index, &key, locale, &value) {
                    Ok(()) => {
                        *draft = value.clone();
                        *missing = false;
                    }
                    Err(e) => {
                        self.error_message = Some(e.to_string());
                        return;
                    }
                }
            }
        }
        self.reload();
        self.set_status("Missing locales filled");
    }

    fn project_label(&self) -> String {
        self.workspace
            .file_name()
            .and_then(|s| s.to_str())
            .map(String::from)
            .unwrap_or_else(|| self.workspace.display().to_string())
    }
}

impl eframe::App for LokalizedApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.top_bar(ctx);

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(300.0)
            .min_width(220.0)
            .frame(theme::sidebar_frame(self.palette))
            .show(ctx, |ui| self.sidebar(ui));

        egui::CentralPanel::default()
            .frame(theme::content_frame(self.palette))
            .show(ctx, |ui| self.main_content(ui));
    }
}

impl LokalizedApp {
    fn top_bar(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        egui::TopBottomPanel::top("top_bar")
            .frame(theme::panel_frame(p).stroke(egui::Stroke::new(1.0, p.border)))
            .show(ctx, |ui| {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("◆").size(18.0).color(p.accent));
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.label(theme::title("Lokalized", p));
                            ui.label(
                                RichText::new(self.project_label())
                                    .size(11.5)
                                    .color(p.text_muted),
                            );
                        });
                    });

                    ui.add_space(24.0);
                    theme::stat_chip(ui, &self.key_count.to_string(), "Keys", p.text, p);
                    theme::stat_chip(ui, &self.locale_count.to_string(), "Locales", p.accent, p);
                    theme::stat_chip(ui, &self.missing_count.to_string(), "Missing", p.missing, p);
                    theme::stat_chip(ui, &self.unused_count.to_string(), "Unused", p.unused, p);

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::primary_button(ui, "New key", p).clicked() {
                            self.show_add_key = true;
                        }
                        if theme::secondary_button(ui, "Refresh", p).clicked() {
                            self.reload();
                        }
                    });
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    theme::search_field(ui, &mut self.search, "Search keys…", p);
                });

                if let Some(err) = &self.error_message {
                    ui.add_space(6.0);
                    theme::inset_card(p).show(ui, |ui| {
                        ui.colored_label(p.danger, err);
                    });
                } else if let Some(msg) = &self.status_message {
                    ui.add_space(4.0);
                    ui.label(RichText::new(msg).size(11.5).color(p.success));
                }
                ui.add_space(8.0);
            });
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        let p = self.palette;
        ui.horizontal(|ui| {
            ui.label(RichText::new("KEYS").size(10.5).color(p.text_muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::ghost_button(ui, "Expand", p).clicked() {
                    for path in self.key_tree.branch_paths() {
                        self.expanded_branches.insert(path);
                    }
                }
                if theme::ghost_button(ui, "Collapse", p).clicked() {
                    self.expanded_branches.clear();
                }
            });
        });
        ui.add_space(8.0);
        theme::divider(ui, p);

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut selected = self.selected_key.clone();
                let view = TreeView {
                    expanded: &mut self.expanded_branches,
                    selected: &mut selected,
                    search: &self.search,
                    palette: &p,
                    unused_keys: &self.unused_keys,
                    missing_keys: &self.missing_keys,
                };
                self.key_tree.show(ui, view);
                if selected != self.selected_key {
                    if let Some(k) = selected {
                        self.select_key(k);
                    }
                }
            });
    }

    fn main_content(&mut self, ui: &mut Ui) {
        if self.show_add_key {
            self.new_key_panel(ui);
            ui.add_space(16.0);
        }

        let Some(key) = self.selected_key.clone() else {
            theme::empty_state(
                ui,
                "No key selected",
                "Pick a key in the sidebar or create one with New key",
                self.palette,
            );
            return;
        };

        self.key_detail(ui, &key);
    }

    fn new_key_panel(&mut self, ui: &mut Ui) {
        let p = self.palette;
        theme::card(p).show(ui, |ui| {
            ui.label(RichText::new("New translation key").strong().size(15.0));
            ui.add_space(14.0);
            theme::field_label(ui, "Key path", p);
            theme::singleline_field(ui, &mut self.new_key_path, "e.g. common.actions.save", p);
            ui.add_space(10.0);
            theme::field_label(ui, "Source value", p);
            theme::singleline_field(ui, &mut self.new_key_value, "Text in source locale", p);
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if theme::primary_button(ui, "Create", p).clicked() {
                    self.commit_add_key();
                }
                if theme::ghost_button(ui, "Cancel", p).clicked() {
                    self.show_add_key = false;
                }
            });
        });
    }

    fn key_detail(&mut self, ui: &mut Ui, key: &str) {
        let p = self.palette;
        theme::card(p).show(ui, |ui| {
            theme::key_breadcrumb(ui, key, p);
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                let (label, fg, bg) = if self.unused_keys.contains(key) {
                    ("Unused", p.unused, p.unused_surface)
                } else if self.missing_keys.contains(key) {
                    ("Incomplete", p.missing, p.missing_surface)
                } else {
                    ("Complete", p.success, p.success_surface)
                };
                theme::badge(ui, label, fg, bg);

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::danger_button(ui, "Delete", p).clicked() {
                        self.commit_delete();
                    }
                    if theme::secondary_button(
                        ui,
                        if self.show_rename { "Cancel" } else { "Rename" },
                        p,
                    )
                    .clicked()
                    {
                        self.show_rename = !self.show_rename;
                        self.rename_key_buffer = key.to_string();
                    }
                });
            });

            if self.show_rename {
                ui.add_space(14.0);
                theme::divider(ui, p);
                theme::field_label(ui, "New key path", p);
                theme::singleline_field(ui, &mut self.rename_key_buffer, key, p);
                ui.add_space(8.0);
                if theme::primary_button(ui, "Apply rename", p).clicked() {
                    self.commit_rename();
                }
            }
        });

        ui.add_space(14.0);
        self.translations_panel(ui, key);
    }

    fn translations_panel(&mut self, ui: &mut Ui, key: &str) {
        let p = self.palette;
        theme::card(p).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Translations").strong().size(15.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::primary_button(ui, "Save all", p).clicked() {
                        self.save_all_locales();
                    }
                    if theme::secondary_button(ui, "Fill missing", p).clicked() {
                        self.fill_missing_from_source();
                    }
                });
            });
            ui.add_space(12.0);
            theme::divider(ui, p);

            ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                let mut to_save: Option<(Locale, String)> = None;
                for (locale, draft, missing) in &mut self.locale_drafts {
                    ui.add_space(6.0);
                    let stroke_color = if *missing {
                        p.missing.linear_multiply(0.4)
                    } else {
                        p.border
                    };
                    theme::inset_card(p)
                        .stroke(egui::Stroke::new(1.0, stroke_color))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                theme::badge(
                                    ui,
                                    locale.as_str(),
                                    if *missing { p.missing } else { p.text },
                                    if *missing {
                                        p.missing_surface
                                    } else {
                                        p.accent_muted
                                    },
                                );
                                if *missing {
                                    theme::badge(ui, "Missing", p.missing, p.missing_surface);
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if theme::ghost_button(ui, "Save", p).clicked() {
                                        to_save = Some((locale.clone(), draft.clone()));
                                    }
                                });
                            });
                            ui.add_space(8.0);
                            theme::multiline_field(ui, draft, "Enter translation…", 3, p);
                        });
                }
                if let Some((locale, value)) = to_save {
                    self.save_locale(&locale, key, &value);
                }
            });
        });
    }
}
