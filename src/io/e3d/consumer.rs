//! Pour an [`E3dSceneBatch`] into `Document` and `Scene`.
//!
//! Compiles on native and `wasm32`. The only input is the platform-neutral
//! batch (`contract`), so this module does not link `e3d-model`.
//!
//! A full batch (`is_incremental == false`) erases the previous ingest, then:
//! - one invisible proxy entity per geometry item, carrying `E3D_SEMANTICS`;
//! - one shared [`MeshLodSet`] per [`MeshKey`] (block-local), placed with
//!   [`MeshInstanceSource`] and `mesh_to_world_mm`;
//! - centreline polylines as line entities. `InvalidTubi` uses linetype
//!   `E3D_DASHED`; the other kinds stay continuous.
//!
//! Incremental `removed` / `upserted` application is left for Phase 4.

use std::collections::HashMap;

use codec::entities::{EntityType, Line, Point, Polyline3D, Vertex3DPolyline};
use codec::types::{Matrix4, Transform, Vector3};
use codec::xdata::{ExtendedDataRecord, XDataValue};
use codec::Document;

use crate::io::E3D_SEMANTICS_XDATA_APP;
use crate::scene::model::mesh_model::{MeshLodSet, MeshModel};
use crate::scene::model::wire_model::WireModel;
use crate::scene::Scene;

use super::contract::{
    CentrelineKind, CompactTriMesh, E3dCentrelineRecord, E3dInstanceRecord, E3dSceneBatch,
    GeometryIdKey, MeshKey,
};

/// Diagnostic dash registered on the document for broken implied tubes.
pub const E3D_DASH_LINETYPE: &str = "E3D_DASHED";

const DASH_MM: f64 = 8.0;
const GAP_MM: f64 = 4.0;

/// Phase 4 owns session-window deltas. Calling it now changes nothing.
pub const INCREMENTAL_DEFERRED: &str = "incremental E3D batch ingest is deferred to Phase 4";

/// Handles created by the last full ingest. Stored on [`Scene`] so the next
/// full batch can erase that source before pouring again.
#[derive(Debug, Clone, Default)]
pub struct E3dIngestIndex {
    /// Solid geometry item → proxy entity.
    pub by_geometry: HashMap<GeometryIdKey, HandleKey>,
    /// Centreline item → first line entity (extra segments are only in `handles`).
    pub centrelines: HashMap<GeometryIdKey, HandleKey>,
    /// Shared mesh → invisible pool anchor whose [`MeshLodSet`] lives in
    /// `Scene::block_meshes`.
    pub mesh_pool: HashMap<MeshKey, HandleKey>,
    /// Every entity this ingest created, erased together on the next full load.
    pub handles: Vec<HandleKey>,
    /// Centreline wires in the same dash/solid split as the line entities.
    pub centreline_wires: Vec<WireModel>,
    pub source_label: String,
}

/// Codec handle newtype alias kept local so this file reads as scene handles.
type HandleKey = codec::Handle;

/// Counts from one full ingest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct E3dIngestStats {
    pub proxies: usize,
    pub shared_meshes: usize,
    pub centrelines: usize,
    pub missing_meshes: usize,
}

/// Refuse an incremental manifest. Does not modify `scene`.
pub fn apply_incremental_batch(
    _scene: &mut Scene,
    batch: &E3dSceneBatch,
) -> Result<E3dIngestStats, String> {
    if !batch.manifest.is_incremental {
        return Err("apply_incremental_batch expects manifest.is_incremental = true".to_string());
    }
    let _ = batch;
    Err(INCREMENTAL_DEFERRED.to_string())
}

/// Erase the previous full ingest and pour `batch` into `scene`.
///
/// `manifest.is_incremental` is rejected without clearing the current index.
pub fn ingest_batch(scene: &mut Scene, batch: &E3dSceneBatch) -> Result<E3dIngestStats, String> {
    if batch.manifest.is_incremental {
        return Err(INCREMENTAL_DEFERRED.to_string());
    }

    clear_ingested(scene);
    ensure_dashed_linetype(&mut scene.document);

    let mut index = E3dIngestIndex {
        source_label: batch.manifest.source_label.clone(),
        ..E3dIngestIndex::default()
    };
    let mut missing_meshes = 0usize;

    for instance in &batch.manifest.upserted {
        if index.by_geometry.contains_key(&instance.geometry_id) {
            continue;
        }
        let proxy = add_proxy(scene, instance);
        index.handles.push(proxy);
        index
            .by_geometry
            .insert(instance.geometry_id.clone(), proxy);

        let Some(mesh) = batch.meshes.get(&instance.mesh_key) else {
            missing_meshes += 1;
            continue;
        };
        let pool = pool_handle(scene, &mut index, &instance.mesh_key, mesh);
        place_instance(scene, proxy, pool, instance);
    }

    for record in &batch.manifest.centrelines {
        let handles = add_centreline(scene, &mut index, record);
        if handles.is_empty() {
            continue;
        }
        index
            .centrelines
            .insert(record.geometry_id.clone(), handles[0]);
        index.handles.extend(handles);
    }

    let stats = E3dIngestStats {
        proxies: index.by_geometry.len(),
        shared_meshes: index.mesh_pool.len(),
        centrelines: index.centrelines.len(),
        missing_meshes,
    };
    scene.e3d_index = index;
    Ok(stats)
}

fn clear_ingested(scene: &mut Scene) {
    let previous = std::mem::take(&mut scene.e3d_index);
    if previous.handles.is_empty() {
        return;
    }
    scene.erase_entities(&previous.handles);
    for handle in &previous.handles {
        scene.meshes.remove(handle);
        scene.block_meshes.remove(handle);
    }
}

fn ensure_dashed_linetype(document: &mut Document) {
    if document.line_types.contains(E3D_DASH_LINETYPE) {
        return;
    }
    let mut linetype = codec::tables::LineType::new(E3D_DASH_LINETYPE);
    linetype.handle = document.allocate_handle();
    linetype.description = "E3D invalid tube (dashed diagnostic)".to_string();
    linetype.add_element(codec::tables::LineTypeElement::dash(DASH_MM));
    linetype.add_element(codec::tables::LineTypeElement::space(GAP_MM));
    linetype.pattern_length = DASH_MM + GAP_MM;
    let _ = document.line_types.add(linetype);
}

fn add_proxy(scene: &mut Scene, instance: &E3dInstanceRecord) -> HandleKey {
    let [[x0, y0, z0], [x1, y1, z1]] = instance.world_aabb_mm;
    let location = if [x0, y0, z0, x1, y1, z1].iter().all(|v| v.is_finite()) {
        Vector3::new((x0 + x1) * 0.5, (y0 + y1) * 0.5, (z0 + z1) * 0.5)
    } else {
        let m = &instance.mesh_to_world_mm;
        Vector3::new(m[12], m[13], m[14])
    };
    let mut point = Point::at(location);
    point.common.invisible = true;
    point.common.layer = discipline_layer(&instance.noun).to_string();
    point.common.extended_data.add_record(semantics_record(
        &instance.geometry_id,
        &instance.refno,
        &instance.noun,
        instance.name.as_deref(),
        instance.owner_refno.as_deref(),
    ));
    scene.add_entity(EntityType::Point(point))
}

fn pool_handle(
    scene: &mut Scene,
    index: &mut E3dIngestIndex,
    key: &MeshKey,
    mesh: &CompactTriMesh,
) -> HandleKey {
    if let Some(handle) = index.mesh_pool.get(key) {
        return *handle;
    }
    let mut anchor = Point::at(Vector3::new(0.0, 0.0, 0.0));
    anchor.common.invisible = true;
    anchor.common.layer = "E3D_MESH_POOL".to_string();
    let handle = scene.add_entity(EntityType::Point(anchor));
    index.handles.push(handle);
    index.mesh_pool.insert(key.clone(), handle);

    let mut lod = MeshLodSet::from_single(mesh_model(handle, mesh));
    lod.edge_verts = feature_edge_verts(mesh);
    lod.edge_verts_low = vec![[0.0; 3]; lod.edge_verts.len()];
    lod.prepare_instance_source(handle);
    scene.block_meshes.insert(handle, lod);
    handle
}

fn place_instance(
    scene: &mut Scene,
    proxy: HandleKey,
    pool: HandleKey,
    instance: &E3dInstanceRecord,
) {
    let Some(source) = scene
        .block_meshes
        .get(&pool)
        .and_then(|lod| lod.instance_source.clone())
    else {
        return;
    };
    let metrics = scene
        .block_meshes
        .get(&pool)
        .map(|lod| lod.metrics)
        .unwrap_or_default();
    let color = scene
        .block_meshes
        .get(&pool)
        .and_then(|lod| lod.display_color());
    let [[x0, y0, z0], [x1, y1, z1]] = instance.world_aabb_mm;
    let mut placed = MeshLodSet::from_lods(Vec::new());
    placed.complete = true;
    placed.metrics = metrics;
    placed.instance_source = Some(source);
    placed.instance_transform = Some(transform_from_cols(instance.mesh_to_world_mm));
    placed.instance_handle = Some(proxy);
    placed.instance_color = color;
    if [x0, y0, z0, x1, y1, z1].iter().all(|v| v.is_finite()) {
        placed.instance_aabb = Some([x0, y0, z0, x1, y1, z1]);
        placed.world_aabb = [x0 as f32, y0 as f32, x1 as f32, y1 as f32];
        placed.z_aabb = [z0 as f32, z1 as f32];
    }
    scene.meshes.insert(proxy, placed);
}

fn add_centreline(
    scene: &mut Scene,
    index: &mut E3dIngestIndex,
    record: &E3dCentrelineRecord,
) -> Vec<HandleKey> {
    let dashed = centreline_is_dashed(record.kind);
    let layer = centreline_layer(record.kind);
    let linetype = if dashed {
        E3D_DASH_LINETYPE
    } else {
        "Continuous"
    };
    let color = if dashed {
        [0.95, 0.35, 0.15, 1.0]
    } else {
        [0.95, 0.85, 0.25, 1.0]
    };
    let mut handles = Vec::new();
    for (ordinal, polyline) in record.polylines_mm.iter().enumerate() {
        let points: Vec<[f64; 3]> = polyline
            .iter()
            .copied()
            .filter(|p| p.iter().all(|v| v.is_finite()))
            .collect();
        if points.len() < 2 {
            continue;
        }
        let record_xdata = semantics_record(
            &record.geometry_id,
            &record.refno,
            &record.noun,
            record.name.as_deref(),
            None,
        );
        let handle = if points.len() == 2 {
            let mut line = Line::from_points(
                Vector3::new(points[0][0], points[0][1], points[0][2]),
                Vector3::new(points[1][0], points[1][1], points[1][2]),
            );
            line.common.layer = layer.to_string();
            line.common.linetype = linetype.to_string();
            line.common.extended_data.add_record(record_xdata);
            scene.add_entity(EntityType::Line(line))
        } else {
            let mut poly = Polyline3D::new();
            poly.vertices = points
                .iter()
                .map(|p| Vertex3DPolyline::from_xyz(p[0], p[1], p[2]))
                .collect();
            poly.common.layer = layer.to_string();
            poly.common.linetype = linetype.to_string();
            poly.common.extended_data.add_record(record_xdata);
            scene.add_entity(EntityType::Polyline3D(poly))
        };
        if handle.is_null() {
            continue;
        }
        handles.push(handle);
        let mut wire = WireModel::solid_f64(handle.value().to_string(), points, color, false);
        if dashed {
            wire.pattern_length = (DASH_MM + GAP_MM) as f32;
            wire.pattern = [DASH_MM as f32, -GAP_MM as f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        }
        wire.name = format!("{}:{ordinal}", record.geometry_id);
        index.centreline_wires.push(wire);
    }
    handles
}

fn mesh_model(handle: HandleKey, mesh: &CompactTriMesh) -> MeshModel {
    MeshModel {
        name: handle.value().to_string(),
        verts: mesh.positions.clone(),
        verts_low: Vec::new(),
        normals: mesh.normals.clone(),
        indices: mesh.indices.clone(),
        triangle_material_handles: Vec::new(),
        triangle_colors: Vec::new(),
        color: [0.62, 0.66, 0.70, 1.0],
        selected: false,
    }
}

fn feature_edge_verts(mesh: &CompactTriMesh) -> Vec<[f32; 3]> {
    let mut verts = Vec::with_capacity(mesh.feature_edges.len() * 2);
    for &[a, b] in &mesh.feature_edges {
        let (Some(p0), Some(p1)) = (
            mesh.positions.get(a as usize),
            mesh.positions.get(b as usize),
        ) else {
            continue;
        };
        verts.push(*p0);
        verts.push(*p1);
    }
    verts
}

fn transform_from_cols(cols: [f64; 16]) -> Transform {
    // `mesh_to_world_mm` is glam column-major (`DMat4::to_cols_array`).
    Transform::from_matrix(Matrix4 {
        m: [
            [cols[0], cols[4], cols[8], cols[12]],
            [cols[1], cols[5], cols[9], cols[13]],
            [cols[2], cols[6], cols[10], cols[14]],
            [cols[3], cols[7], cols[11], cols[15]],
        ],
    })
}

fn semantics_record(
    geometry_id: &GeometryIdKey,
    refno: &str,
    noun: &str,
    name: Option<&str>,
    owner: Option<&str>,
) -> ExtendedDataRecord {
    let mut record = ExtendedDataRecord::new(E3D_SEMANTICS_XDATA_APP);
    push_pair(&mut record, "geometry_id", &geometry_id.to_string());
    push_pair(&mut record, "refno", refno);
    push_pair(&mut record, "noun", noun);
    if let Some(name) = name {
        push_pair(&mut record, "name", name);
    }
    if let Some(owner) = owner {
        push_pair(&mut record, "owner", owner);
    }
    record
}

fn push_pair(record: &mut ExtendedDataRecord, key: &str, value: &str) {
    if !value.is_empty() {
        record.add_value(XDataValue::String(format!("{key}={value}")));
    }
}

/// Read one `key=` value from an entity's `E3D_SEMANTICS` record.
pub fn semantics_value(entity: &EntityType, key: &str) -> Option<String> {
    let record = entity
        .common()
        .extended_data
        .get_record(E3D_SEMANTICS_XDATA_APP)?;
    let prefix = format!("{key}=");
    for value in &record.values {
        let XDataValue::String(text) = value else {
            continue;
        };
        if let Some(rest) = text.strip_prefix(&prefix) {
            return Some(rest.to_string());
        }
    }
    None
}

/// Discipline layer for a solid geometry item. Unknown nouns land on `E3D_OTHER`.
pub fn discipline_layer(noun: &str) -> &'static str {
    match noun.trim().to_ascii_uppercase().as_str() {
        "PIPE" | "TUBI" | "ELBO" | "VALV" | "FLAN" | "GASK" | "TEE" | "REDU" | "BEND" | "ATTA"
        | "PCOM" | "WELD" | "FBLI" => "E3D_PIPE",
        "EQUI" | "NOZZ" | "SUBE" => "E3D_EQUI",
        "SCTN" | "STRU" | "PANE" | "SNOD" | "FRMW" | "GENSEC" | "WALL" => "E3D_STRU",
        "HVAC" | "DUCT" => "E3D_HVAC",
        _ => "E3D_OTHER",
    }
}

pub fn centreline_layer(kind: CentrelineKind) -> &'static str {
    if centreline_is_dashed(kind) {
        "E3D_INVALID_TUBI"
    } else {
        "E3D_CENTRELINE"
    }
}

pub fn centreline_is_dashed(kind: CentrelineKind) -> bool {
    matches!(kind, CentrelineKind::InvalidTubi)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::super::contract::{
        CanonicalShapeKey, CentrelineKind, CompactTriMesh, E3dBatchManifest, E3dCentrelineRecord,
        E3dInstanceRecord, E3dSceneBatch, GeometryIdKey, MeshKey,
    };
    use super::*;

    fn translation(x: f64, y: f64, z: f64) -> [f64; 16] {
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, z, 1.0,
        ]
    }

    fn triangle() -> CompactTriMesh {
        CompactTriMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]],
            indices: vec![0, 1, 2],
            feature_edges: vec![[0, 1], [1, 2], [2, 0]],
            local_aabb_mm: [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
        }
    }

    fn instance(refno: &str, x: f64) -> E3dInstanceRecord {
        E3dInstanceRecord {
            geometry_id: GeometryIdKey::Element {
                refno: refno.to_string(),
            },
            refno: refno.to_string(),
            owner_refno: Some("26229/1509".to_string()),
            noun: "VALV".to_string(),
            name: Some("/100-B-1".to_string()),
            mesh_key: MeshKey::Canonical(CanonicalShapeKey::BoxV1),
            mesh_to_world_mm: translation(x, 0.0, 0.0),
            world_aabb_mm: [[x - 0.5, -0.5, -0.5], [x + 0.5, 0.5, 0.5]],
        }
    }

    fn centreline(kind: CentrelineKind, refno: &str) -> E3dCentrelineRecord {
        E3dCentrelineRecord {
            geometry_id: GeometryIdKey::Element {
                refno: refno.to_string(),
            },
            refno: refno.to_string(),
            noun: "TUBI".to_string(),
            name: None,
            kind,
            polylines_mm: vec![vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]]],
        }
    }

    fn batch(
        instances: Vec<E3dInstanceRecord>,
        centrelines: Vec<E3dCentrelineRecord>,
    ) -> E3dSceneBatch {
        let key = MeshKey::Canonical(CanonicalShapeKey::BoxV1);
        let mut meshes = BTreeMap::new();
        meshes.insert(key.clone(), triangle());
        E3dSceneBatch {
            manifest: E3dBatchManifest {
                source_label: "synthetic".to_string(),
                is_incremental: false,
                upserted: instances,
                centrelines,
                removed: Vec::new(),
                required_meshes: vec![key],
                ledger: Default::default(),
            },
            meshes,
        }
    }

    fn tessellated_pattern(scene: &Scene, handle: HandleKey) -> f32 {
        let entity = scene
            .document
            .get_entity(handle)
            .expect("centreline entity");
        let wires = crate::scene::tessellate_entity(
            &scene.document,
            &rustc_hash::FxHashSet::default(),
            None,
            [0.0, 0.0, 0.0, 1.0],
            1.0,
            None,
            entity,
            None,
            None,
            None,
            false,
        );
        wires.iter().map(|wire| wire.pattern_length).sum()
    }

    #[test]
    fn shared_canonical_mesh_and_dashed_invalid_tubi() {
        let mut scene = Scene::new();
        let full = batch(
            vec![
                instance("24381/47065", 1000.0),
                instance("24381/47066", 2000.0),
            ],
            vec![
                centreline(CentrelineKind::InvalidTubi, "bad-tube"),
                centreline(CentrelineKind::ProfileCentreline, "sctn-line"),
            ],
        );
        let stats = ingest_batch(&mut scene, &full).expect("full ingest");
        assert_eq!(stats.proxies, 2);
        assert_eq!(stats.shared_meshes, 1);
        assert_eq!(stats.centrelines, 2);
        assert_eq!(stats.missing_meshes, 0);
        assert_eq!(scene.e3d_index.mesh_pool.len(), 1);
        assert_eq!(scene.block_meshes.len(), 1);

        let left = scene.e3d_index.by_geometry[&GeometryIdKey::Element {
            refno: "24381/47065".to_string(),
        }];
        let right = scene.e3d_index.by_geometry[&GeometryIdKey::Element {
            refno: "24381/47066".to_string(),
        }];
        let left_mesh = scene.meshes.get(&left).expect("left instance");
        let right_mesh = scene.meshes.get(&right).expect("right instance");
        assert!(
            left_mesh.lods.is_empty(),
            "instance must not copy triangles"
        );
        assert!(
            right_mesh.lods.is_empty(),
            "instance must not copy triangles"
        );
        let left_source = left_mesh.instance_source.clone().expect("left source");
        let right_source = right_mesh.instance_source.clone().expect("right source");
        assert!(Arc::ptr_eq(&left_source, &right_source));
        assert_eq!(left_source.lods[0].indices, vec![0, 1, 2]);
        assert_eq!(left_source.edge_verts.len(), 6);

        let left_tx = left_mesh.instance_transform.expect("left transform");
        assert!((left_tx.matrix.m[0][3] - 1000.0).abs() < 1e-9);

        let left_entity = scene.document.get_entity(left).expect("proxy");
        assert_eq!(
            semantics_value(left_entity, "refno").as_deref(),
            Some("24381/47065")
        );
        assert_eq!(
            semantics_value(left_entity, "noun").as_deref(),
            Some("VALV")
        );
        assert_eq!(
            semantics_value(left_entity, "name").as_deref(),
            Some("/100-B-1")
        );
        assert_eq!(
            semantics_value(left_entity, "owner").as_deref(),
            Some("26229/1509")
        );
        assert_eq!(left_entity.common().layer, "E3D_PIPE");

        let dashed = scene.e3d_index.centrelines[&GeometryIdKey::Element {
            refno: "bad-tube".to_string(),
        }];
        let solid = scene.e3d_index.centrelines[&GeometryIdKey::Element {
            refno: "sctn-line".to_string(),
        }];
        assert_eq!(
            scene.document.get_entity(dashed).unwrap().common().layer,
            "E3D_INVALID_TUBI"
        );
        assert_eq!(
            scene.document.get_entity(solid).unwrap().common().layer,
            "E3D_CENTRELINE"
        );
        assert!(
            scene
                .e3d_index
                .centreline_wires
                .iter()
                .any(|wire| wire.pattern_length > 0.0),
            "stored InvalidTubi wire is dashed"
        );
        assert!(tessellated_pattern(&scene, dashed) > 0.0);
        assert_eq!(tessellated_pattern(&scene, solid), 0.0);

        let incremental = E3dSceneBatch {
            manifest: E3dBatchManifest {
                is_incremental: true,
                ..full.manifest.clone()
            },
            meshes: full.meshes.clone(),
        };
        let err = apply_incremental_batch(&mut scene, &incremental).unwrap_err();
        assert!(err.contains("Phase 4"));
        assert!(scene.document.get_entity(left).is_some());

        let again = batch(vec![instance("9/1", 0.0)], Vec::new());
        let stats = ingest_batch(&mut scene, &again).expect("replace");
        assert_eq!(stats.proxies, 1);
        assert_eq!(stats.shared_meshes, 1);
        assert!(scene.document.get_entity(left).is_none());
        assert!(scene.document.get_entity(dashed).is_none());
        assert_eq!(scene.e3d_index.by_geometry.len(), 1);
        assert!(scene.meshes.contains_key(
            &scene.e3d_index.by_geometry[&GeometryIdKey::Element {
                refno: "9/1".to_string(),
            }]
        ));
    }
}
