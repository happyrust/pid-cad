//! Recover a DXF text style's TrueType typeface from its `ACAD` xdata.
//!
//! AutoCAD stores a TrueType STYLE record two ways at once: group 3 carries
//! the font *file* (often left empty, or naming a `.ttf` whose stem is not the
//! family name), and the typeface the style really means lives in extended
//! data — `1001 ACAD`, `1000 <typeface>`, `1071 <pitch/family/charset>`. The
//! acadrust DXF reader (cadcodec `7b4c112`, `read_textstyle_entry`) only
//! recognises the `AcadAnnotative` application and drops the `ACAD` record, so
//! `TextStyle::true_type_font` stays empty and `resolve_text_style` is left
//! with the style *name* — `-宋体`, `ST`, `HT`, `HZDX` — which names no
//! installed family. Every one of those styles then falls back to the default
//! stroke font for Latin and to whatever system font cosmic-text picks for
//! CJK, with advances that do not match the font AutoCAD laid the drawing out
//! in: on the twelve real P&ID sheets 74–90 % of the text goes through such a
//! style, and the mixed CJK / Latin lines overlap by a character or two.
//!
//! This pass re-reads only the STYLE table of the DXF text (it stops at that
//! table's `ENDTAB`, long before BLOCKS / ENTITIES) and fills
//! `true_type_font` where the record has a typeface and the style is not an
//! SHX one. Lines are decoded lossily: a pre-R2007 file in a legacy code page
//! whose typeface is a non-ASCII name comes out with replacement characters
//! and is skipped rather than guessed — matching by handle keeps the style
//! *name*'s encoding out of it. Binary DXF is not scanned.

use std::io::BufRead;

use acadrust::{CadDocument, Handle};

/// One STYLE record's typeface as written in its `ACAD` xdata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StyleTypeface {
    /// Group 5, when it parsed as hex; `NULL` otherwise (then `name` matches).
    pub handle: Handle,
    /// Group 2, as decoded from the file.
    pub name: String,
    /// Group 1000 under `1001 ACAD`.
    pub typeface: String,
}

/// Scan a DXF stream's STYLE table for typefaces kept only in `ACAD` xdata.
///
/// Reads pairs until the STYLE table ends (or the TABLES section does), so the
/// cost is bounded by the header and tables, not by the drawing. Returns an
/// empty list for binary DXF and for anything that is not a DXF at all.
pub(crate) fn scan_style_typefaces(reader: impl BufRead) -> Vec<StyleTypeface> {
    let mut found = Vec::new();
    let mut lines = reader.split(b'\n');
    let mut next_line = move || -> Option<String> {
        let raw = lines.next()?.ok()?;
        let text = String::from_utf8_lossy(&raw);
        Some(text.trim_end_matches(['\r', '\n']).to_string())
    };

    // Binary DXF starts with a sentinel; its pairs are not lines.
    let Some(first) = next_line() else {
        return found;
    };
    if first.starts_with("AutoCAD Binary DXF") {
        return found;
    }
    let mut pending = Some(first);

    let mut in_tables = false;
    let mut in_style_table = false;
    let mut entry: Option<(Handle, String, Option<String>)> = None;
    let mut xdata_app: Option<String> = None;

    loop {
        let code_line = match pending.take() {
            Some(line) => line,
            None => match next_line() {
                Some(line) => line,
                None => break,
            },
        };
        let Some(value) = next_line() else {
            break;
        };
        let Ok(code) = code_line.trim().parse::<i32>() else {
            continue;
        };
        let value = value.trim();

        if code == 0 {
            // Close the open STYLE entry, whatever comes next.
            if let Some((handle, name, typeface)) = entry.take() {
                if let Some(typeface) = typeface {
                    found.push(StyleTypeface { handle, name, typeface });
                }
            }
            xdata_app = None;
            match value {
                "STYLE" if in_style_table => entry = Some((Handle::NULL, String::new(), None)),
                "ENDTAB" if in_style_table => return found,
                "ENDSEC" if in_tables => return found,
                _ => {}
            }
            continue;
        }

        if code == 2 && entry.is_none() {
            // `2` names the SECTION or the TABLE we are entering. A file with
            // no TABLES section at all stops at the first drawing section, so
            // a malformed DXF never costs a scan of its entities.
            if !in_tables {
                if value.eq_ignore_ascii_case("BLOCKS")
                    || value.eq_ignore_ascii_case("ENTITIES")
                    || value.eq_ignore_ascii_case("OBJECTS")
                {
                    return found;
                }
                in_tables = value.eq_ignore_ascii_case("TABLES");
            } else if !in_style_table {
                in_style_table = value.eq_ignore_ascii_case("STYLE");
            }
            continue;
        }

        let Some((handle, name, typeface)) = entry.as_mut() else {
            continue;
        };
        match code {
            5 => {
                if let Ok(h) = u64::from_str_radix(value, 16) {
                    *handle = Handle::new(h);
                }
            }
            2 => *name = value.to_string(),
            1001 => xdata_app = Some(value.to_string()),
            1000 => {
                if xdata_app.as_deref() == Some("ACAD") && typeface.is_none() && !value.is_empty() {
                    *typeface = Some(value.to_string());
                }
            }
            _ => {}
        }
    }
    found
}

/// Fill `TextStyle::true_type_font` from the scanned xdata typefaces.
///
/// A style is matched by handle first and by name when the record had no
/// usable handle. Only styles that still have no typeface and are not SHX
/// (shape-file flag, or a font file other than `.ttf` / `.ttc` / `.otf`) are
/// touched; a typeface that decoded with replacement characters is skipped.
/// Returns how many styles were filled.
pub(crate) fn apply_style_typefaces(doc: &mut CadDocument, found: &[StyleTypeface]) -> usize {
    let mut filled = 0;
    for record in found {
        if record.typeface.is_empty() || record.typeface.contains('\u{FFFD}') {
            continue;
        }
        let style = doc.text_styles.iter_mut().find(|style| {
            if record.handle.is_valid() {
                style.handle == record.handle
            } else {
                !record.name.is_empty() && style.name.eq_ignore_ascii_case(&record.name)
            }
        });
        let Some(style) = style else {
            continue;
        };
        if !style.true_type_font.trim().is_empty() || style.is_shape_file {
            continue;
        }
        let file = style.font_file.trim().to_ascii_lowercase();
        let is_truetype_file = file.is_empty()
            || file.ends_with(".ttf")
            || file.ends_with(".ttc")
            || file.ends_with(".otf");
        if !is_truetype_file {
            continue;
        }
        style.true_type_font = record.typeface.clone();
        filled += 1;
    }
    filled
}

/// Scan and apply in one step; the caller hands over the DXF text it just read.
pub(crate) fn fix_dxf_text_style_typefaces(doc: &mut CadDocument, reader: impl BufRead) -> usize {
    let found = scan_style_typefaces(reader);
    if found.is_empty() {
        return 0;
    }
    apply_style_typefaces(doc, &found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use acadrust::tables::TextStyle;
    use std::io::Cursor;

    /// The STYLE table of `DWG-0100WS02-05`, trimmed to the records that matter,
    /// plus the section that follows so the scan is seen to stop.
    const SHEET_TABLES: &str = "  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\nAC1032\n  0\nENDSEC\n\
  0\nSECTION\n  2\nTABLES\n\
  0\nTABLE\n  2\nLTYPE\n  5\n5\n 70\n     3\n  0\nLTYPE\n  5\n14\n  2\nByBlock\n  0\nENDTAB\n\
  0\nTABLE\n  2\nSTYLE\n  5\n3\n 70\n     7\n\
  0\nSTYLE\n  5\n11\n  2\nStandard\n 70\n     0\n 41\n1.0\n  3\narial.ttf\n  4\n\n\
  0\nSTYLE\n  5\n319\n  2\n黑体\n 70\n     0\n 41\n0.8\n  3\nsimhei.ttf\n  4\n\n1001\nACAD\n1000\nSimHei\n1071\n    34306\n\
  0\nSTYLE\n  5\n31B\n  2\n-宋体\n 70\n     0\n 41\n0.8\n 42\n0.2\n  3\n\n  4\n\n1001\nACAD\n1000\nSimSun\n1071\n    34306\n\
  0\nSTYLE\n  5\n31C\n  2\n_TCH_DIM_T3\n 70\n     0\n 41\n0.7\n  3\nSIMPLEX\n  4\nGBCBIG\n\
  0\nSTYLE\n  5\n31D\n  2\nAnnotative\n 70\n     0\n  3\narial.ttf\n  4\n\n1001\nAcadAnnotative\n1000\nAnnotativeData\n1002\n{\n1070\n     1\n1070\n     1\n1002\n}\n\
  0\nENDTAB\n\
  0\nTABLE\n  2\nLAYER\n  0\nLAYER\n  2\nTWT_TEXT\n1001\nACAD\n1000\nNotAStyle\n  0\nENDTAB\n\
  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n  0\nTEXT\n  1\n室外排水管线设计坡度\n  7\n-宋体\n  0\nENDSEC\n  0\nEOF\n";

    #[test]
    fn scan_finds_acad_typefaces_and_stops_at_the_style_table() {
        let found = scan_style_typefaces(Cursor::new(SHEET_TABLES.replace('\n', "\r\n")));
        assert_eq!(
            found,
            vec![
                StyleTypeface {
                    handle: Handle::new(0x319),
                    name: "黑体".into(),
                    typeface: "SimHei".into(),
                },
                StyleTypeface {
                    handle: Handle::new(0x31B),
                    name: "-宋体".into(),
                    typeface: "SimSun".into(),
                },
            ],
            "only ACAD xdata on STYLE records counts; AcadAnnotative and the LAYER's xdata do not"
        );
    }

    #[test]
    fn scan_ignores_binary_dxf_and_non_dxf_input() {
        let binary = b"AutoCAD Binary DXF\r\n\x1a\0\0\0SECTION".to_vec();
        assert!(scan_style_typefaces(Cursor::new(binary)).is_empty());
        assert!(scan_style_typefaces(Cursor::new(b"not a dxf at all".to_vec())).is_empty());
        assert!(scan_style_typefaces(Cursor::new(Vec::<u8>::new())).is_empty());
    }

    fn style(handle: u64, name: &str, font_file: &str) -> TextStyle {
        let mut s = TextStyle::new(name);
        s.handle = Handle::new(handle);
        s.font_file = font_file.to_string();
        s
    }

    #[test]
    fn apply_fills_only_empty_non_shx_styles() {
        let mut doc = CadDocument::new();
        doc.text_styles.add_or_replace(style(0x31B, "-宋体", ""));
        doc.text_styles.add_or_replace(style(0x319, "黑体", "simhei.ttf"));
        let mut shx = style(0x31C, "_TCH_DIM_T3", "SIMPLEX");
        shx.big_font_file = "GBCBIG".into();
        doc.text_styles.add_or_replace(shx);
        let mut already = style(0x320, "HT", "");
        already.true_type_font = "FangSong".into();
        doc.text_styles.add_or_replace(already);

        let found = vec![
            StyleTypeface { handle: Handle::new(0x31B), name: "-宋体".into(), typeface: "SimSun".into() },
            StyleTypeface { handle: Handle::new(0x319), name: "黑体".into(), typeface: "SimHei".into() },
            // An SHX style would never carry this in a real file; it must still be refused.
            StyleTypeface { handle: Handle::new(0x31C), name: "_TCH_DIM_T3".into(), typeface: "SimSun".into() },
            // Already resolved: left alone.
            StyleTypeface { handle: Handle::new(0x320), name: "HT".into(), typeface: "SimSun".into() },
            // Decoded through the wrong code page: skipped, not guessed.
            StyleTypeface { handle: Handle::NULL, name: "ST".into(), typeface: "\u{FFFD}\u{FFFD}".into() },
            // Unknown record: ignored.
            StyleTypeface { handle: Handle::new(0x999), name: "nope".into(), typeface: "SimSun".into() },
        ];
        assert_eq!(apply_style_typefaces(&mut doc, &found), 2);
        assert_eq!(doc.text_styles.get("-宋体").unwrap().true_type_font, "SimSun");
        assert_eq!(doc.text_styles.get("黑体").unwrap().true_type_font, "SimHei");
        assert_eq!(doc.text_styles.get("_TCH_DIM_T3").unwrap().true_type_font, "");
        assert_eq!(doc.text_styles.get("HT").unwrap().true_type_font, "FangSong");
    }

    #[test]
    fn apply_matches_by_name_when_the_record_had_no_handle() {
        let mut doc = CadDocument::new();
        doc.text_styles.add_or_replace(style(0, "st", ""));
        let found = vec![StyleTypeface { handle: Handle::NULL, name: "ST".into(), typeface: "SimSun".into() }];
        assert_eq!(apply_style_typefaces(&mut doc, &found), 1);
        assert_eq!(doc.text_styles.get("st").unwrap().true_type_font, "SimSun");
    }

    #[test]
    fn end_to_end_on_the_sheet_tables() {
        let mut doc = CadDocument::new();
        doc.text_styles.add_or_replace(style(0x31B, "-宋体", ""));
        doc.text_styles.add_or_replace(style(0x319, "黑体", "simhei.ttf"));
        assert_eq!(fix_dxf_text_style_typefaces(&mut doc, Cursor::new(SHEET_TABLES.as_bytes())), 2);
        assert_eq!(doc.text_styles.get("-宋体").unwrap().true_type_font, "SimSun");
    }
}
