//! Build hierarchical document-symbol trees from locale file entries.

use crate::position::Range;
use crate::LocalizedValue;

/// One node in a locale-file symbol tree (maps to LSP `DocumentSymbol`).
#[derive(Clone, Debug)]
pub struct DocumentSymbolNode {
    pub name: String,
    /// Translation preview for leaf keys.
    pub detail: Option<String>,
    pub key_range: Range,
    pub range: Range,
    pub children: Vec<DocumentSymbolNode>,
}

#[derive(Default)]
struct TrieNode<'a> {
    children: std::collections::BTreeMap<String, TrieNode<'a>>,
    leaf: Option<&'a LocalizedValue>,
}

/// Turn flat `(dotted_key, value)` pairs into a nested tree for Outline / document symbols.
pub fn build_document_symbol_tree(
    entries: &[(String, &LocalizedValue)],
) -> Vec<DocumentSymbolNode> {
    let mut root = TrieNode::default();
    for (key, value) in entries {
        let mut node = &mut root;
        for part in key.split('.') {
            node = node.children.entry(part.to_string()).or_default();
        }
        node.leaf = Some(value);
    }
    root.children
        .into_iter()
        .map(|(name, node)| trie_to_symbol(&name, node))
        .collect()
}

fn trie_to_symbol(name: &str, node: TrieNode<'_>) -> DocumentSymbolNode {
    let children: Vec<DocumentSymbolNode> = node
        .children
        .into_iter()
        .map(|(child_name, child)| trie_to_symbol(&child_name, child))
        .collect();

    let (key_range, range, detail) = if let Some(leaf) = node.leaf {
        let detail = Some(truncate_detail(&leaf.value));
        (leaf.key_range, leaf.range, detail)
    } else {
        (Range::default(), Range::default(), None)
    };

    if !children.is_empty() {
        let (kr, r) = union_child_ranges(&children, key_range, range);
        DocumentSymbolNode {
            name: name.to_string(),
            detail,
            key_range: kr,
            range: r,
            children,
        }
    } else {
        DocumentSymbolNode {
            name: name.to_string(),
            detail,
            key_range,
            range,
            children,
        }
    }
}

fn union_child_ranges(
    children: &[DocumentSymbolNode],
    fallback_key: Range,
    fallback_value: Range,
) -> (Range, Range) {
    let mut key_start = fallback_key.start;
    let mut key_end = fallback_key.end;
    let mut val_start = fallback_value.start;
    let mut val_end = fallback_value.end;

    for child in children {
        if child.key_range.start.offset < key_start.offset {
            key_start = child.key_range.start;
        }
        if child.key_range.end.offset > key_end.offset {
            key_end = child.key_range.end;
        }
        if child.range.start.offset < val_start.offset {
            val_start = child.range.start;
        }
        if child.range.end.offset > val_end.offset {
            val_end = child.range.end;
        }
    }

    (
        Range {
            start: key_start,
            end: key_end,
        },
        Range {
            start: val_start,
            end: val_end,
        },
    )
}

fn truncate_detail(value: &str) -> String {
    const MAX: usize = 48;
    if value.chars().count() <= MAX {
        return value.to_string();
    }
    let mut s: String = value.chars().take(MAX).collect();
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::{Position, Range};
    use std::path::PathBuf;

    fn leaf(value: &str, key_off: usize, val_off: usize) -> LocalizedValue {
        LocalizedValue {
            value: value.to_string(),
            file: PathBuf::from("/en.json"),
            key_range: Range {
                start: Position {
                    line: 0,
                    character: key_off as u32,
                    offset: key_off,
                },
                end: Position {
                    line: 0,
                    character: (key_off + 3) as u32,
                    offset: key_off + 3,
                },
            },
            range: Range {
                start: Position {
                    line: 0,
                    character: val_off as u32,
                    offset: val_off,
                },
                end: Position {
                    line: 0,
                    character: (val_off + value.len()) as u32,
                    offset: val_off + value.len(),
                },
            },
        }
    }

    #[test]
    fn builds_nested_tree() {
        let a = leaf("Hi", 1, 10);
        let b = leaf("Bye", 20, 30);
        let entries = vec![("hello".to_string(), &a), ("common.submit".to_string(), &b)];
        let tree = build_document_symbol_tree(&entries);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].name, "common");
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].name, "submit");
        assert_eq!(tree[1].name, "hello");
        assert_eq!(tree[1].detail.as_deref(), Some("Hi"));
    }
}
