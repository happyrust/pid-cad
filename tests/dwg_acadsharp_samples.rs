use codec::Document;
use std::path::PathBuf;
use OpenCADStudio::io::{load_file, save_as_version};

fn acadsharp_samples_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("ACadSharp")
        .join("samples");
    dir.is_dir().then_some(dir)
}

fn sample_path(name: &str) -> Option<PathBuf> {
    Some(acadsharp_samples_dir()?.join(name))
}

fn read_sample(name: &str) -> Option<Document> {
    let path = sample_path(name)?;
    Some(load_file(&path).unwrap_or_else(|e| panic!("load {}: {e}", path.display())))
}

fn assert_core_tables(doc: &Document, sample: &str) {
    // Some samples are legitimately entity-less: `empty.dwg` is an empty
    // drawing, and `geoloc.dwg` only carries GeoData objects (ACadSharp's own
    // test asserts just the GeoData dictionary entry). Core tables must still
    // load for those, but there are no entities to count.
    let entity_less = sample.ends_with("empty.dwg") || sample.ends_with("geoloc.dwg");
    if !entity_less {
        assert!(doc.entity_count() > 0, "{sample}: no entities read");
    }
    assert!(doc.layers.len() >= 1, "{sample}: no layers read");
    assert!(doc.line_types.len() >= 1, "{sample}: no linetypes read");
    assert!(doc.text_styles.len() >= 1, "{sample}: no text styles read");
    assert!(
        doc.block_records.len() >= 2,
        "{sample}: missing model/paper block records"
    );
    assert!(
        doc.objects.len() >= 1,
        "{sample}: no non-graphical objects read"
    );
    assert!(
        !doc.header.current_layer_name.is_empty(),
        "{sample}: empty CLAYER"
    );
}

#[test]
fn reads_acadsharp_sample_matrix() {
    let Some(samples) = acadsharp_samples_dir() else {
        eprintln!("Skipping: ../ACadSharp/samples not found");
        return;
    };

    for name in [
        "sample_AC1014.dwg",
        "sample_AC1015.dwg",
        "sample_AC1018.dwg",
        "sample_AC1021.dwg",
        "sample_AC1024.dwg",
        "sample_AC1027.dwg",
        "sample_AC1032.dwg",
    ] {
        let path = samples.join(name);
        let doc = load_file(&path).unwrap_or_else(|e| panic!("load {}: {e}", path.display()));
        assert_core_tables(&doc, name);
    }
}

#[test]
fn reads_acadsharp_feature_samples() {
    let Some(samples) = acadsharp_samples_dir() else {
        eprintln!("Skipping: ../ACadSharp/samples not found");
        return;
    };

    for rel in [
        "sample_base/empty.dwg",
        "sample_base/sample_base.dwg",
        "dynamic-blocks/BLOCKPOINTPARAMETER.dwg",
        "dynamic-blocks/BLOCKROTATIONPARAMETER.dwg",
        "dynamic-blocks/BLOCKVISIBILITYPARAMETER.dwg",
        "geolocation/geoloc.dwg",
        "aec_objects/AecObjects.dwg",
    ] {
        let path = samples.join(rel);
        if !path.is_file() {
            eprintln!("Skipping missing ACadSharp sample: {}", path.display());
            continue;
        }
        let doc = load_file(&path).unwrap_or_else(|e| panic!("load {}: {e}", path.display()));
        assert_core_tables(&doc, rel);
    }
}

#[test]
fn roundtrips_supported_acadsharp_samples() {
    let cases = [
        "sample_AC1014.dwg",
        "sample_AC1015.dwg",
        "sample_AC1018.dwg",
        "sample_AC1021.dwg",
        "sample_AC1024.dwg",
        "sample_AC1027.dwg",
        "sample_AC1032.dwg",
    ];

    for name in cases {
        let Some(doc) = read_sample(name) else {
            eprintln!("Skipping: ../ACadSharp/samples/{name} not found");
            return;
        };
        let out =
            std::env::temp_dir().join(format!("ocs_acadsharp_rt_{}_{}", std::process::id(), name));
        save_as_version(&doc, &out, doc.version)
            .unwrap_or_else(|e| panic!("write {}: {e}", out.display()));
        let reread =
            load_file(&out).unwrap_or_else(|e| panic!("load written {}: {e}", out.display()));
        assert!(
            reread.entity_count() >= doc.entity_count() / 2,
            "{name}: roundtrip lost too many entities: {} -> {}",
            doc.entity_count(),
            reread.entity_count()
        );
        let _ = std::fs::remove_file(out);
    }
}
