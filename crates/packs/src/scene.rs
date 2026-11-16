//! Placing a unit's scene into a document: fresh ids, bindings that travel
//! with their runs, and bytes the document does not declare reported rather
//! than invented.
//!
//! Shared by the editor's paste and the agent surface's `add_card`, so an
//! inserted card and a pasted one are the same operation.

use std::collections::{BTreeMap, BTreeSet};

use yunari_model::document::{Binding, Node, NodeKind, TextContent, TextRun, ThemeDocument};

use crate::package::Unit;

/// What an insertion added, and which declared assets the document is missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inserted {
    pub ids: Vec<String>,
    pub missing_assets: Vec<String>,
}

/// Puts a copy of `nodes` into `document`, shifted by `(dx, dy)` and with
/// fresh ids. `bindings` is keyed by the node id in `nodes` that carries them.
pub fn insert(
    document: &mut ThemeDocument,
    nodes: &[Node],
    bindings: &BTreeMap<String, Vec<Binding>>,
    dx: f64,
    dy: f64,
) -> Inserted {
    let mut ids = BTreeMap::new();
    for node in nodes {
        map_ids(document, node, &mut ids);
    }
    let mut taken = BTreeSet::new();
    for held in document.bindings.values() {
        for binding in held {
            taken.insert(binding.id.clone());
        }
    }
    let mut binding_ids = BTreeMap::new();
    let mut new_bindings = Vec::new();
    for (old, held) in bindings {
        let Some(new_node) = ids.get(old) else {
            continue;
        };
        let mut copied = Vec::new();
        for binding in held {
            let fresh = fresh_name(&taken, "binding");
            taken.insert(fresh.clone());
            binding_ids.insert(binding.id.clone(), fresh.clone());
            let mut binding = binding.clone();
            binding.id = fresh;
            copied.push(binding);
        }
        new_bindings.push((new_node.clone(), copied));
    }
    let mut inserted = Inserted::default();
    for node in nodes {
        let mut node = node.clone();
        let root = ids.get(&node.id).cloned();
        rename(&mut node, &ids, &binding_ids);
        node.transform.x += dx;
        node.transform.y += dy;
        if let Some(id) = root {
            inserted.ids.push(id);
        }
        if let Some(asset) = asset_of(&node)
            && !document.assets.iter().any(|held| held.id == asset)
            && !inserted.missing_assets.contains(&asset)
        {
            inserted.missing_assets.push(asset);
        }
        document.nodes.push(node);
    }
    for (id, held) in new_bindings {
        document.bindings.insert(id, held);
    }
    inserted
}

/// Places one unit's scene so its root corner lands at `(left, top)`. The
/// unit's palette and type tokens are adopted where the document lacks that
/// name, and each of its picture declarations takes the id this document
/// already holds for the same path.
pub fn place(document: &mut ThemeDocument, unit: &Unit, left: f64, top: f64) -> Inserted {
    if unit.document.nodes.is_empty() {
        return Inserted::default();
    }
    for (id, value) in &unit.document.palette {
        document
            .palette
            .entry(id.clone())
            .or_insert_with(|| value.clone());
    }
    for (id, token) in &unit.document.type_tokens {
        document
            .type_tokens
            .entry(id.clone())
            .or_insert_with(|| token.clone());
    }
    let mapping = asset_ids(document, unit);
    let mut assets = BTreeMap::new();
    for declaration in &unit.document.assets {
        let Some(fresh) = mapping.get(&declaration.id) else {
            continue;
        };
        if document.assets.iter().any(|held| held.id == *fresh) {
            continue;
        }
        if let Some(bytes) = unit.assets.get(&declaration.path) {
            assets.insert(declaration.path.clone(), bytes.clone());
        }
        let mut declaration = declaration.clone();
        declaration.id = fresh.clone();
        document.assets.push(declaration);
    }
    let mut nodes = unit.document.nodes.clone();
    for node in &mut nodes {
        remap_assets(node, &mapping);
    }
    let root = &unit.document.nodes[0];
    let dx = left - root.transform.x;
    let dy = top - root.transform.y;
    insert(document, &nodes, &unit.document.bindings, dx, dy)
}

fn asset_ids(document: &ThemeDocument, unit: &Unit) -> BTreeMap<String, String> {
    let mut mapping = BTreeMap::new();
    let mut taken: Vec<String> = document.assets.iter().map(|a| a.id.clone()).collect();
    for asset in &unit.document.assets {
        if let Some(held) = document.assets.iter().find(|held| held.path == asset.path) {
            mapping.insert(asset.id.clone(), held.id.clone());
            continue;
        }
        let mut id = asset.id.clone();
        let mut counter = 2;
        while taken.contains(&id) {
            id = format!("{asset_id}-{counter}", asset_id = asset.id);
            counter += 1;
        }
        taken.push(id.clone());
        mapping.insert(asset.id.clone(), id);
    }
    mapping
}

fn remap_assets(node: &mut Node, mapping: &BTreeMap<String, String>) {
    match &mut node.kind {
        NodeKind::Image { content } => {
            if let Some(fresh) = mapping.get(&content.asset_id) {
                content.asset_id = fresh.clone();
            }
        }
        NodeKind::Video { content } => {
            if let Some(fresh) = mapping.get(&content.asset_id) {
                content.asset_id = fresh.clone();
            }
        }
        _ => {}
    }
    for child in &mut node.children {
        remap_assets(child, mapping);
    }
}

fn asset_of(node: &Node) -> Option<String> {
    match &node.kind {
        NodeKind::Image { content } => Some(content.asset_id.clone()),
        NodeKind::Video { content } => Some(content.asset_id.clone()),
        _ => None,
    }
}

/// One fresh id per node in the copied subtree, keyed by the old id.
fn map_ids(document: &ThemeDocument, node: &Node, ids: &mut BTreeMap<String, String>) {
    let mut taken: BTreeSet<String> = document.nodes.iter().map(|node| node.id.clone()).collect();
    for node in &document.nodes {
        collect_ids(node, &mut taken);
    }
    taken.extend(ids.values().cloned());
    let fresh = fresh_name(&taken, &node.id);
    ids.insert(node.id.clone(), fresh);
    for child in &node.children {
        map_ids(document, child, ids);
    }
}

fn collect_ids(node: &Node, into: &mut BTreeSet<String>) {
    into.insert(node.id.clone());
    for child in &node.children {
        collect_ids(child, into);
    }
}

/// `base-1`, `base-2` … until nothing holds the name.
fn fresh_name(taken: &BTreeSet<String>, base: &str) -> String {
    let mut number = 1;
    loop {
        let id = format!("{base}-{number}");
        if !taken.contains(&id) {
            return id;
        }
        number += 1;
    }
}

fn rename(node: &mut Node, ids: &BTreeMap<String, String>, bindings: &BTreeMap<String, String>) {
    if let Some(fresh) = ids.get(&node.id) {
        node.id = fresh.clone();
    }
    if let NodeKind::Text {
        content: TextContent { runs, .. },
    } = &mut node.kind
    {
        for run in runs {
            if let TextRun::Value { binding_id, .. } = run
                && let Some(fresh) = bindings.get(binding_id)
            {
                *binding_id = fresh.clone();
            }
        }
    }
    for child in &mut node.children {
        rename(child, ids, bindings);
    }
}
