//! Binary PSM record-chain encoders for `igLine2d` (`0x0018`),
//! `igLineString2d` (`0x0084`), `igTextBox` (`0x004D`), `igSymbol2d` (`0x00CE`),
//! and `FreeFormAttrSet` (`0x0089`) (`L5-4` / `#129`).

const TYPE_IGLINE2D: u16 = 0x0018;
const TYPE_IGTEXTBOX: u16 = 0x004D;
const TYPE_JSHEET_LAYER: u16 = 0x0081;
const TYPE_IGLINESTRING2D: u16 = 0x0084;
const TYPE_FREEFORM_ATTR: u16 = 0x0089;
const TYPE_IGSYMBOL2D: u16 = 0x00CE;
const TYPE_JSHEET: u16 = 0x0114;

const IGSYMBOL2D_MATRIX_TAG: [u8; 4] = [0x02, 0x00, 0xA7, 0x50];

/// Convert paper millimetres to `.pid` sheet metres (`1 m = 1000 mm`).
pub fn mm_to_sheet_m(mm: f64) -> f64 {
    mm * 0.001
}

/// Parameters for encoding one `0x00CE igSymbol2d` placement record.
#[derive(Debug, Clone)]
pub struct SymbolRecordParams {
    pub oid: u32,
    pub sheet_layer_oid: u32,
    pub style_ref: u32,
    pub jsite_ref: u32,
    pub transform: [f64; 4],
    pub insertion_m: (f64, f64),
    pub uniform_scale: f64,
    pub definition_sheet_ref: u32,
    pub definition_site_ref: u32,
}

fn wrap_psm_record(type_code: u16, payload: &[u8]) -> Vec<u8> {
    let mut rec = Vec::with_capacity(6 + payload.len());
    rec.extend_from_slice(&type_code.to_le_bytes());
    rec.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    rec.extend_from_slice(payload);
    rec
}

fn check_coord_pair(pt: (f64, f64)) -> Result<(), String> {
    if !pt.0.is_finite() || !pt.1.is_finite() || pt.0.abs() > 1e9 || pt.1.abs() > 1e9 {
        return Err(format!("coordinate out of domain: ({}, {})", pt.0, pt.1));
    }
    Ok(())
}

/// Encode a full 56-byte `0x0018 igLine2d` record (6B PSM envelope + 50B payload).
pub fn encode_igline2d_record(
    oid: u32,
    sheet_layer_oid: u32,
    style_index: u32,
    start_m: (f64, f64),
    end_m: (f64, f64),
) -> Result<Vec<u8>, String> {
    check_coord_pair(start_m)?;
    check_coord_pair(end_m)?;
    if (end_m.0 - start_m.0).abs() < 1e-12 && (end_m.1 - start_m.1).abs() < 1e-12 {
        return Err(
            "degenerate zero-length igLine2d is rejected by native/pid-parse reader".into(),
        );
    }
    let mut payload = Vec::with_capacity(50);
    payload.extend_from_slice(&oid.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes()); // parent_ref
    payload.extend_from_slice(&sheet_layer_oid.to_le_bytes());
    payload.extend_from_slice(&1u16.to_le_bytes()); // sub_type_word
    payload.extend_from_slice(&style_index.to_le_bytes());
    payload.extend_from_slice(&start_m.0.to_le_bytes());
    payload.extend_from_slice(&start_m.1.to_le_bytes());
    payload.extend_from_slice(&end_m.0.to_le_bytes());
    payload.extend_from_slice(&end_m.1.to_le_bytes());
    Ok(wrap_psm_record(TYPE_IGLINE2D, &payload))
}

/// Encode a `0x0084 igLineString2d` polyline record (`6 + 24 + vc*16` bytes).
pub fn encode_iglinestring2d_record(
    oid: u32,
    sheet_layer_oid: u32,
    style_index: u32,
    vertices_m: &[(f64, f64)],
) -> Result<Vec<u8>, String> {
    if vertices_m.len() < 2 {
        return Err("igLineString2d requires at least 2 vertices".into());
    }
    if sheet_layer_oid > 0xFFFF {
        return Err(format!(
            "igLineString2d sheet_layer_oid must be <= 0xFFFF, got {sheet_layer_oid}"
        ));
    }
    for &v in vertices_m {
        check_coord_pair(v)?;
    }
    let vc = vertices_m.len();
    let mut payload = Vec::with_capacity(24 + vc * 16);
    payload.extend_from_slice(&oid.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes()); // parent_ref
    payload.extend_from_slice(&sheet_layer_oid.to_le_bytes());
    payload.extend_from_slice(&1u16.to_le_bytes()); // sub_type_word
    payload.extend_from_slice(&style_index.to_le_bytes());
    payload.extend_from_slice(&(vc as u32).to_le_bytes());
    payload.push(0u8); // form = 0
    payload.push(0u8); // scope = 0
    for &(x, y) in vertices_m {
        payload.extend_from_slice(&x.to_le_bytes());
        payload.extend_from_slice(&y.to_le_bytes());
    }
    Ok(wrap_psm_record(TYPE_IGLINESTRING2D, &payload))
}

/// Encode a `0x004D igTextBox` shape-1 text annotation record.
pub fn encode_igtextbox_record(
    oid: u32,
    parent_ref: u32,
    sheet_layer_oid: u32,
    style_index: u32,
    text: &str,
    insertion_m: (f64, f64),
    rotation_rad: f64,
) -> Result<Vec<u8>, String> {
    check_coord_pair(insertion_m)?;
    if !rotation_rad.is_finite() {
        return Err("igTextBox rotation_rad must be finite".into());
    }
    let utf16: Vec<u16> = text.encode_utf16().collect();
    if utf16.len() > 1024 {
        return Err(format!(
            "igTextBox text exceeds 1024 UTF-16 units ({})",
            utf16.len()
        ));
    }
    // Shape 1: 22B header + 2B count + 2*N UTF-16LE + 36B placement tail
    let mut payload = Vec::with_capacity(24 + utf16.len() * 2 + 36);
    payload.extend_from_slice(&oid.to_le_bytes());
    payload.extend_from_slice(&parent_ref.to_le_bytes());
    payload.extend_from_slice(&sheet_layer_oid.to_le_bytes());
    payload.extend_from_slice(&1u16.to_le_bytes()); // sub_type_word
    payload.extend_from_slice(&style_index.to_le_bytes());
    payload.extend_from_slice(&1u16.to_le_bytes()); // text_sub_type = 1
    payload.extend_from_slice(&0u16.to_le_bytes()); // pad at +20..22
    payload.extend_from_slice(&(utf16.len() as u16).to_le_bytes()); // count at +22..24
    for u in utf16 {
        payload.extend_from_slice(&u.to_le_bytes());
    }
    // 36-byte placement tail: x, y, cos, sin, 4-byte trailer (anchor_code at +35)
    let (sin_v, cos_v) = rotation_rad.sin_cos();
    payload.extend_from_slice(&insertion_m.0.to_le_bytes());
    payload.extend_from_slice(&insertion_m.1.to_le_bytes());
    payload.extend_from_slice(&cos_v.to_le_bytes());
    payload.extend_from_slice(&sin_v.to_le_bytes());
    payload.extend_from_slice(&[0u8, 0, 0, 0x18]);
    Ok(wrap_psm_record(TYPE_IGTEXTBOX, &payload))
}

/// Encode a 119-byte `0x00CE igSymbol2d` symbol placement record (6B envelope + 113B payload).
pub fn encode_igsymbol2d_record(params: &SymbolRecordParams) -> Result<Vec<u8>, String> {
    check_coord_pair(params.insertion_m)?;
    if !params
        .transform
        .iter()
        .all(|v| v.is_finite() && v.abs() <= 1e9)
    {
        return Err("igSymbol2d transform matrix contains non-finite values".into());
    }
    let scale = if params.uniform_scale.is_finite() && params.uniform_scale > 0.0 {
        params.uniform_scale
    } else {
        1.0
    };

    let mut payload = Vec::with_capacity(113);
    payload.extend_from_slice(&params.oid.to_le_bytes()); // 0..4
    payload.extend_from_slice(&0u32.to_le_bytes()); // 4..8 parent_ref
    payload.extend_from_slice(&params.sheet_layer_oid.to_le_bytes()); // 8..12
    payload.extend_from_slice(&1u16.to_le_bytes()); // 12..14 sub_type_word
    payload.extend_from_slice(&[0u8; 11]); // 14..25
    payload.extend_from_slice(&params.style_ref.to_le_bytes()); // 25..29 style_ref
    payload.extend_from_slice(&params.jsite_ref.to_le_bytes()); // 29..33 jsite_ref
    payload.extend_from_slice(&IGSYMBOL2D_MATRIX_TAG); // 33..37
    for &m in &params.transform {
        payload.extend_from_slice(&m.to_le_bytes()); // 37..69
    }
    payload.extend_from_slice(&params.insertion_m.0.to_le_bytes()); // 69..77
    payload.extend_from_slice(&params.insertion_m.1.to_le_bytes()); // 77..85
    payload.extend_from_slice(&scale.to_le_bytes()); // 85..93
    payload.extend_from_slice(&0x0100_5001u32.to_le_bytes()); // 93..97 flags
    payload.extend_from_slice(&0u32.to_le_bytes()); // 97..101 has_membassy = 0
    payload.extend_from_slice(&0u32.to_le_bytes()); // 101..105
    payload.extend_from_slice(&params.definition_sheet_ref.to_le_bytes()); // 105..109
    payload.extend_from_slice(&params.definition_site_ref.to_le_bytes()); // 109..113
    debug_assert_eq!(payload.len(), 113);

    Ok(wrap_psm_record(TYPE_IGSYMBOL2D, &payload))
}

fn attr_frame(tag: u16, body: &[u8]) -> Result<Vec<u8>, String> {
    let total_len = body
        .len()
        .checked_add(4)
        .and_then(|n| u16::try_from(n).ok())
        .ok_or_else(|| "FreeFormAttrSet field exceeds u16 frame limit".to_string())?;
    let mut out = Vec::with_capacity(usize::from(total_len));
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&total_len.to_le_bytes());
    out.extend_from_slice(body);
    Ok(out)
}

/// Encode a `0x0089 FreeFormAttrSet` attribute row record (`P&IDAttributes` or custom class).
pub fn encode_freeform_attr_row_record(
    row_oid: u32,
    described_oid: u32,
    attr_class: &str,
    text_fields: &[(&str, &str)],
) -> Result<Vec<u8>, String> {
    if attr_class.is_empty()
        || attr_class.len() > 64
        || !attr_class.bytes().all(|b| (0x20..0x7F).contains(&b))
    {
        return Err(format!("invalid FreeFormAttrSet class name {attr_class:?}"));
    }

    let mut payload = Vec::new();
    payload.extend_from_slice(&row_oid.to_le_bytes()); // +0..4
    payload.extend_from_slice(&[0u8; 8]); // +4..12
    payload.extend_from_slice(&described_oid.to_le_bytes()); // +12..16
    payload.extend_from_slice(&[0xFF, 0xFF, 0x04, 0x01]); // +16..20

    // Set header at +20 (`tag = 0x0000`)
    let mut hdr_body = Vec::with_capacity(attr_class.len() + 2);
    hdr_body.push(0u8);
    hdr_body.extend_from_slice(attr_class.as_bytes());
    hdr_body.push(0u8);
    payload.extend_from_slice(&attr_frame(0x0000, &hdr_body)?);

    // Text fields (`tag = 0x8010`)
    for &(fname, fval) in text_fields {
        if fname.is_empty() || fname.len() > 64 || !fname.bytes().all(|b| (0x20..0x7F).contains(&b))
        {
            return Err(format!("invalid FreeFormAttrSet field name {fname:?}"));
        }
        if fval.as_bytes().contains(&0) {
            return Err(format!(
                "FreeFormAttrSet text value for {fname} contains NUL"
            ));
        }
        let mut fbody = Vec::with_capacity(1 + fname.len() + 1 + fval.len() + 1);
        fbody.push(1u8);
        fbody.extend_from_slice(fname.as_bytes());
        fbody.push(0u8);
        fbody.extend_from_slice(fval.as_bytes());
        fbody.push(0u8);
        payload.extend_from_slice(&attr_frame(0x8010, &fbody)?);
    }

    Ok(wrap_psm_record(TYPE_FREEFORM_ATTR, &payload))
}

/// Build `JProperties` bytes carrying the normalized `.sym` relative path in both
/// ASCII (`SymbolPath=...`) and UTF-16LE so `parse_jproperties` recovers it cleanly.
pub(crate) fn encode_jproperties_for_sym_path(sym_rel_path: &str) -> Vec<u8> {
    let norm = sym_rel_path
        .trim()
        .trim_start_matches(['\\', '/'])
        .replace('/', "\\");
    let mut buf = Vec::new();
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(b"SymbolPath\0");
    buf.extend_from_slice(norm.as_bytes());
    buf.push(0);
    for u in norm.encode_utf16() {
        buf.extend_from_slice(&u.to_le_bytes());
    }
    buf.extend_from_slice(&[0u8; 4]);
    buf
}

/// Build a self-contained fallback embedded `JSite<site_id>` symbol cache
/// (`JProperties` + `PSMcluster0` with `JSheet 6`, `JSheetLayer 8 Default`, and
/// 2 crosshair `igLine2d` strokes) when an external `.sym` library file is absent.
pub(crate) fn build_fallback_jsite_streams(
    site_id: u32,
    sym_rel_path: &str,
) -> (u32, Vec<(String, Vec<u8>)>) {
    let def_sheet_oid = 6u32;
    let def_layer_oid = 8u32;

    // 0x0114 JSheet minimal record (`oid = 6`)
    let mut jsheet_payload = vec![0u8; 28 + 164];
    jsheet_payload[0..4].copy_from_slice(&def_sheet_oid.to_le_bytes());
    jsheet_payload[12..20].copy_from_slice(&1.0f64.to_le_bytes());
    let jsheet_rec = wrap_psm_record(TYPE_JSHEET, &jsheet_payload);

    // 0x0081 JSheetLayer (`oid = 8`, `object_count = 2`, `layer_number = 0`, `name = "Default"`)
    let mut layer_payload = Vec::new();
    layer_payload.extend_from_slice(&def_layer_oid.to_le_bytes()); // +0 oid
    layer_payload.extend_from_slice(&0u32.to_le_bytes()); // +4 parent_ref
    layer_payload.extend_from_slice(&0u32.to_le_bytes()); // +8 aux_hi
    layer_payload.extend_from_slice(&2u32.to_le_bytes()); // +12 object_count = 2
    layer_payload.extend_from_slice(&0u32.to_le_bytes()); // +16 layer_number = 0
    let name_u16: Vec<u16> = "Default".encode_utf16().collect();
    layer_payload.extend_from_slice(&(name_u16.len() as u32).to_le_bytes());
    for u in name_u16 {
        layer_payload.extend_from_slice(&u.to_le_bytes());
    }
    layer_payload.extend_from_slice(&0u32.to_le_bytes()); // empty secondary_name
    layer_payload.extend_from_slice(&0u32.to_le_bytes()); // trailing_word
    let layer_rec = wrap_psm_record(TYPE_JSHEET_LAYER, &layer_payload);

    let l1 = encode_igline2d_record(10, def_layer_oid, 1, (-0.002, -0.002), (0.002, 0.002))
        .expect("fallback stroke 1");
    let l2 = encode_igline2d_record(11, def_layer_oid, 1, (-0.002, 0.002), (0.002, -0.002))
        .expect("fallback stroke 2");

    let mut cluster = Vec::new();
    cluster.extend_from_slice(&0x6C90_F544u32.to_le_bytes());
    cluster.extend_from_slice(&4u32.to_le_bytes()); // 4 live records
    cluster.extend_from_slice(&jsheet_rec);
    cluster.extend_from_slice(&layer_rec);
    cluster.extend_from_slice(&l1);
    cluster.extend_from_slice(&l2);

    let jprops = encode_jproperties_for_sym_path(sym_rel_path);
    let streams = vec![
        (format!("/JSite{site_id}/JProperties"), jprops),
        (format!("/JSite{site_id}/PSMcluster0"), cluster),
    ];
    (def_sheet_oid, streams)
}
