use bevy::prelude::*;
use bevy::scene::{DynamicScene, DynamicSceneRoot};
use bevy_egui::egui;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Spawn a loader entity to load a Bevy DynamicScene from an asset path.
/// The asset path should be AssetServer-relative (e.g., "app_state.scn.ron").
pub fn load_graph_from_file(commands: &mut Commands, asset_server: &AssetServer, file_path: impl Into<String>) -> Entity {
    let path: String = file_path.into();
    let handle: Handle<DynamicScene> = asset_server.load(path);
    commands
        .spawn((Name::new("State Machine (scene)"), DynamicSceneRoot(handle)))
        .id()
}

// ============
// Sidecar (.sm.ron) save
// ============
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    let tmp_path: PathBuf = {
        let mut p = path.to_path_buf();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            p.set_file_name(format!("{}.tmp", name));
        } else {
            p.set_file_name("sidecar.tmp");
        }
        p
    };

    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(contents.as_bytes())?;
        f.flush()?;
    }
    #[cfg(target_os = "windows")]
    {
        fs::rename(&tmp_path, path)?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = fs::remove_file(path);
        fs::rename(&tmp_path, path)
    }
}

fn to_sidecar_path(path_no_ext_or_full: impl AsRef<Path>) -> PathBuf {
    let p = path_no_ext_or_full.as_ref();
    let s = p.to_string_lossy();
    if s.ends_with(".sm.ron") { return p.to_path_buf(); }
    let mut out = p.to_path_buf();
    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
        // If user passed a base without extension, append .sm.ron
        if p.extension().is_none() || p.extension().and_then(|e| e.to_str()) != Some("sm.ron") {
            out.set_file_name(format!("{}.sm.ron", stem));
        }
    } else {
        out.set_file_name("state_machine.sm.ron");
    }
    out
}

pub fn save_sidecar_text(path_no_ext_or_full: impl AsRef<Path>, contents: &str) -> io::Result<()> {
    let path = to_sidecar_path(path_no_ext_or_full);
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    atomic_write(&path, contents)
}

// ======================
// Editor sidecar schema
// ======================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarViewport { pub pan: (f32, f32), pub zoom: f32 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeLayout { pub pos: (f32, f32), pub collapsed: bool }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EdgeLayout { pub pill_center: (f32, f32) }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Sidecar {
    pub schema_version: u32,
    pub scene_basename: Option<String>,
    pub scene_hash: Option<String>,
    pub graph_fingerprint: Option<String>,
    pub viewport: Option<SidecarViewport>,
    pub nodes: std::collections::HashMap<String, NodeLayout>,
    pub edges: std::collections::HashMap<String, EdgeLayout>,
}

impl Sidecar {
    pub fn new() -> Self {
        Self { schema_version: 1, scene_basename: None, scene_hash: None, graph_fingerprint: None, viewport: None, nodes: Default::default(), edges: Default::default() }
    }
}

pub fn save_sidecar(path: impl AsRef<Path>, sidecar: &Sidecar) -> io::Result<()> {
    let pretty = ron::ser::PrettyConfig::new();
    let text = ron::ser::to_string_pretty(sidecar, pretty).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("ron: {e}")))?;
    save_sidecar_text(path, &text)
}

pub fn load_sidecar(path: impl AsRef<Path>) -> io::Result<Sidecar> {
    let text = std::fs::read_to_string(path)?;
    let sidecar: Sidecar = ron::de::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("ron: {e}")))?;
    Ok(sidecar)
}

// ======================
// Key + fingerprint API
// ======================

use crate::editor::view_model::{GraphDoc, UiViewKind};
use crate::model::{EntityId, StateMachineGraph};

fn get_node_name(graph: &StateMachineGraph, id: &EntityId) -> String {
    graph
        .nodes
        .get(id)
        .and_then(|n| n.display_name.clone())
        .unwrap_or_else(|| format!("{}", id))
}

fn build_parent_path(graph: &StateMachineGraph, id: &EntityId) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = Some(*id);
    while let Some(cid) = cur {
        if let Some(node) = graph.nodes.get(&cid) {
            if let Some(p) = node.parent { cur = Some(p); parts.push(get_node_name(graph, &cid)); continue; } else { parts.push(get_node_name(graph, &cid)); break; }
        } else { break; }
    }
    if parts.len() >= 1 { parts.remove(0); }
    parts.reverse();
    parts.join("/")
}

pub fn node_key(graph: &StateMachineGraph, id: &EntityId) -> String {
    // Simplified variant without explicit state variant string for now
    let parent = graph.nodes.get(id).and_then(|n| n.parent);
    let parent_path = match parent { Some(pid) => build_parent_path(graph, &pid), None => String::new() };
    let name = get_node_name(graph, id);
    format!("{}|{}", parent_path, name)
}

pub fn edge_key(graph: &StateMachineGraph, eid: &EntityId) -> String {
    let e = match graph.edges.get(eid) { Some(e) => e, None => return format!("{:?}", eid) };
    let src = node_key(graph, &e.source);
    let dst = node_key(graph, &e.target);
    let label = e.display_label.clone().unwrap_or_else(|| "Edge".to_string());
    format!("{} -> {}|{}", src, dst, label)
}

pub fn compute_graph_fingerprint(graph: &StateMachineGraph) -> String {
    let mut node_keys: Vec<String> = graph.nodes.keys().map(|id| node_key(graph, id)).collect();
    node_keys.sort();
    let mut edge_keys: Vec<String> = graph.edges.keys().map(|id| edge_key(graph, id)).collect();
    edge_keys.sort();
    let canonical = format!("nodes:\n{}\nedges:\n{}", node_keys.join("\n"), edge_keys.join("\n"));
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let bytes = hasher.finalize();
    format!("sha256:{}", hex::encode(bytes))
}

// ===============
// Apply/Extract
// ===============

pub fn extract_sidecar_from_doc(doc: &GraphDoc) -> Sidecar {
    let mut sc = Sidecar::new();
    if let Some(graph) = &doc.graph {
        sc.graph_fingerprint = Some(compute_graph_fingerprint(graph));
        for (id, view) in doc.views.iter() {
            match view.kind {
                UiViewKind::Leaf | UiViewKind::Parent | UiViewKind::Parallel => {
                    let key = node_key(graph, id);
                    sc.nodes.insert(key, NodeLayout { pos: (view.rect.min.x, view.rect.min.y), collapsed: false });
                }
                UiViewKind::Edge { .. } => {
                    let key = edge_key(graph, id);
                    let center = view.pill.as_ref().map(|p| p.center).unwrap_or(view.rect.center());
                    sc.edges.insert(key, EdgeLayout { pill_center: (center.x, center.y) });
                }
            }
        }
    }
    sc.viewport = Some(SidecarViewport { pan: (doc.transform.pan.x, doc.transform.pan.y), zoom: doc.transform.zoom });
    sc
}

pub fn apply_sidecar_to_doc(doc: &mut GraphDoc, sidecar: &Sidecar) {
    if let Some(graph) = &doc.graph {
        for (id, view) in doc.views.iter_mut() {
            match view.kind {
                UiViewKind::Leaf | UiViewKind::Parent | UiViewKind::Parallel => {
                    let key = node_key(graph, id);
                    if let Some(n) = sidecar.nodes.get(&key) {
                        let size = view.rect.size();
                        view.rect = egui::Rect::from_min_size(egui::pos2(n.pos.0, n.pos.1), size);
                    }
                }
                UiViewKind::Edge { .. } => {
                    let key = edge_key(graph, id);
                    if let Some(e) = sidecar.edges.get(&key) {
                        let size = view.rect.size();
                        let min = egui::pos2(e.pill_center.0 - size.x * 0.5, e.pill_center.1 - size.y * 0.5);
                        view.rect = egui::Rect::from_min_size(min, size);
                        if let Some(p) = view.pill.as_mut() { p.center = egui::pos2(e.pill_center.0, e.pill_center.1); }
                    }
                }
            }
        }
    }
    if let Some(vp) = &sidecar.viewport { doc.transform.pan = egui::vec2(vp.pan.0, vp.pan.1); doc.transform.zoom = vp.zoom; }
}


