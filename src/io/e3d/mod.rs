//! AVEVA E3D / PDMS Dabacon and `.rvm` 3D model integration (ADR-0001, ADR-0002).
//!
//! Architecture split:
//! - [`contract`] compiles on **all** targets (`native` and `wasm32`) and defines
//!   the platform-neutral **Scene Batch Manifest** (`E3dBatchManifest`),
//!   content-addressed mesh keys (`MeshKey`), **Geometry Item** identities
//!   (`GeometryIdKey`), and compact triangle meshes (`CompactTriMesh`).
//! - [`producer`] compiles on **native** targets only (`cfg(not(target_arch = "wasm32"))`),
//!   wrapping `e3d-model` (`generate_subtree_with_cache`, `increment_update`, and
//!   `rvm_read::RvmFile`) to assemble `E3dSceneBatch` payloads both for in-process
//!   desktop loading and for `OpenCADStudio --http` serving to Web clients.
//! - [`consumer`] compiles on **all** targets and pours an [`E3dSceneBatch`] into
//!   `Document` proxies plus `Scene` meshes and centreline wires. It does not
//!   link `e3d-model`.

pub mod consumer;
pub mod contract;

#[cfg(not(target_arch = "wasm32"))]
pub mod producer;

pub use consumer::{apply_incremental_batch, ingest_batch, E3dIngestIndex, E3dIngestStats};
pub use contract::{
    CanonicalShapeKey, CentrelineKind, CompactTriMesh, E3dBatchManifest, E3dCentrelineRecord,
    E3dInstanceRecord, E3dLedgerSummary, E3dSceneBatch, GeometryIdKey, MeshKey,
};
