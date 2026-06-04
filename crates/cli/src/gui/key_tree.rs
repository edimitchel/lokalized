//! Hierarchical key tree for the translation browser.

use std::collections::BTreeMap;

use egui::{Color32, Rect, Sense, Ui, Vec2};
use i18n_core::LocaleIndex;

use super::theme::Palette;

#[derive(Clone, Debug, Default)]
pub struct KeyTreeRoot {
    pub children: BTreeMap<String, KeyTreeNode>,
}

#[derive(Clone, Debug, Default)]
pub struct KeyTreeNode {
    pub leaf_key: Option<String>,
    pub children: BTreeMap<String, KeyTreeNode>,
}

impl KeyTreeRoot {
    pub fn from_index(index: &LocaleIndex) -> Self {
        let mut root = Self::default();
        let mut keys = index.all_keys();
        keys.sort();
        for key in keys {
            root.insert_key(&key);
        }
        root
    }

    fn insert_key(&mut self, key: &str) {
        let parts: Vec<&str> = key.split('.').collect();
        let mut level = &mut self.children;
        for (i, part) in parts.iter().enumerate() {
            let entry = level.entry((*part).to_string()).or_default();
            if i == parts.len() - 1 {
                entry.leaf_key = Some(key.to_string());
            }
            level = &mut entry.children;
        }
    }

    pub fn branch_paths(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, child) in &self.children {
            if !child.children.is_empty() {
                out.push(name.clone());
                out.extend(child.all_branch_paths(name));
            }
        }
        out
    }

    pub fn show(&self, ui: &mut Ui, mut view: TreeView<'_>) {
        for (name, node) in &self.children {
            show_node(ui, name, node, "", 0, &mut view);
        }
    }
}

fn collect_branch_paths_node(node: &KeyTreeNode, prefix: &str, out: &mut Vec<String>) {
    for (name, child) in &node.children {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        if !child.children.is_empty() {
            out.push(path.clone());
            collect_branch_paths_node(child, &path, out);
        }
    }
}

impl KeyTreeNode {
    fn all_branch_paths(&self, prefix: &str) -> Vec<String> {
        let mut out = Vec::new();
        collect_branch_paths_node(self, prefix, &mut out);
        out
    }
}

pub struct TreeView<'a> {
    pub expanded: &'a mut std::collections::HashSet<String>,
    pub selected: &'a mut Option<String>,
    pub search: &'a str,
    pub palette: &'a Palette,
    pub unused_keys: &'a std::collections::HashSet<String>,
    pub missing_keys: &'a std::collections::HashSet<String>,
}

fn show_node(
    ui: &mut Ui,
    segment: &str,
    node: &KeyTreeNode,
    prefix: &str,
    depth: usize,
    view: &mut TreeView<'_>,
) {
    let path = if prefix.is_empty() {
        segment.to_string()
    } else {
        format!("{prefix}.{segment}")
    };

    let has_children = !node.children.is_empty();
    let is_leaf = node.leaf_key.is_some();
    let needle = view.search.to_lowercase();

    if has_children {
        if !needle.is_empty() && !subtree_matches_search(node, &needle) {
            return;
        }
        if !needle.is_empty() {
            view.expanded.insert(path.clone());
        }

        branch_row(ui, segment, depth, &path, view);

        if view.expanded.contains(&path) {
            if is_leaf {
                if let Some(key) = &node.leaf_key {
                    if key_matches_search(key, &needle) {
                        leaf_row(ui, key, depth + 1, view);
                    }
                }
            }
            for (child_name, child) in &node.children {
                show_node(ui, child_name, child, &path, depth + 1, view);
            }
        }
    } else if is_leaf {
        if let Some(key) = &node.leaf_key {
            if key_matches_search(key, &needle) {
                leaf_row(ui, key, depth, view);
            }
        }
    }
}

fn branch_row(ui: &mut Ui, segment: &str, depth: usize, path: &str, view: &mut TreeView<'_>) {
    let open = view.expanded.contains(path);
    let indent = depth as f32 * 14.0;
    let row_h = 26.0;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::click());

    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, view.palette.surface_hover.linear_multiply(0.7));
    }

    let chevron = if open { "▾" } else { "▸" };
    let text_pos = rect.left_center() + Vec2::new(indent + 6.0, 0.0);
    ui.painter().text(
        rect.left_center() + Vec2::new(indent, 0.0),
        egui::Align2::LEFT_CENTER,
        chevron,
        egui::FontId::proportional(11.0),
        view.palette.accent_soft,
    );
    ui.painter().text(
        text_pos,
        egui::Align2::LEFT_CENTER,
        segment,
        egui::FontId::proportional(12.5),
        view.palette.text_secondary,
    );

    if response.clicked() {
        if open {
            view.expanded.remove(path);
        } else {
            view.expanded.insert(path.to_string());
        }
    }
}

fn leaf_row(ui: &mut Ui, key: &str, depth: usize, view: &mut TreeView<'_>) {
    let selected = view.selected.as_deref() == Some(key);
    let (dot_color, row_bg) = key_colors(key, view);
    let indent = depth as f32 * 14.0 + 2.0;
    let row_h = 28.0;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::click());

    let fill = if selected {
        view.palette.selection
    } else {
        row_bg
    };
    ui.painter().rect_filled(rect, 6.0, fill);

    if selected {
        let bar = Rect::from_min_size(rect.left_top(), Vec2::new(2.5, rect.height()));
        ui.painter().rect_filled(bar, 0.0, view.palette.accent);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, view.palette.surface_hover.linear_multiply(0.5));
    }

    let short = short_key_label(key);
    let text_x = rect.left_center().x + indent + 14.0;
    ui.painter().circle_filled(
        egui::pos2(rect.left_center().x + indent + 5.0, rect.center().y),
        3.5,
        dot_color,
    );
    ui.painter().text(
        egui::pos2(text_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        short,
        egui::FontId::proportional(12.5),
        if selected {
            view.palette.text
        } else {
            view.palette.text_secondary
        },
    );

    if response.clicked() {
        *view.selected = Some(key.to_string());
    }
}

fn key_colors(key: &str, view: &TreeView<'_>) -> (Color32, Color32) {
    if view.missing_keys.contains(key) {
        (view.palette.missing, view.palette.missing_surface)
    } else if view.unused_keys.contains(key) {
        (view.palette.unused, view.palette.unused_surface)
    } else {
        (view.palette.success, Color32::TRANSPARENT)
    }
}

fn key_matches_search(key: &str, needle: &str) -> bool {
    needle.is_empty() || key.to_lowercase().contains(needle)
}

fn subtree_matches_search(node: &KeyTreeNode, needle: &str) -> bool {
    if let Some(key) = &node.leaf_key {
        if key_matches_search(key, needle) {
            return true;
        }
    }
    node.children
        .values()
        .any(|c| subtree_matches_search(c, needle))
}

fn short_key_label(key: &str) -> String {
    key.rsplit('.').next().unwrap_or(key).to_string()
}

pub fn missing_key_set(index: &LocaleIndex) -> std::collections::HashSet<String> {
    let source = &index.source_locale;
    let mut set = std::collections::HashSet::new();
    for locale in index.trees.keys() {
        if locale == source {
            continue;
        }
        for k in index.missing_keys(locale) {
            set.insert(k);
        }
    }
    set
}
