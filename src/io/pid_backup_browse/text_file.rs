//! Reading a file of a backup as text, when it is text.
//!
//! Much of what a backup carries beside its drawings is plain text under a
//! name that hides it: the plant's rules (`RefData~4~680`), its insulation
//! specification, the data maps, the export log. This module reads such a
//! file -- from the backup folder, out of one of its archives, or from the
//! `Ref` tree -- and says whether it is text at all.
//!
//! * The encoding is taken from the bytes: a UTF-16 or UTF-8 byte-order
//!   mark, else UTF-8 when the bytes are valid UTF-8, else GBK (the plants
//!   this reads are Chinese Windows installations), else Windows-1252.
//! * At most [`TEXT_LIMIT`] bytes are read; a longer file is cut at its last
//!   whole line and says so.
//! * Lines are kept as they are, tabs widened; a line too long for a row is
//!   continued on rows of its own.
//! * What has the look of a credential is replaced by
//!   [`HIDDEN`](super::manifest::HIDDEN), and counted: a line of a manifest
//!   by the manifest's own rule (a copy of one can lie anywhere), elsewhere
//!   a value obfuscated the way a manifest's are, a `user/password@host`
//!   login, and the value of anything called a password (`Password=…`,
//!   `<Pwd>…</Pwd>`, `"password": "…"`, `key="DbPassword" value="…"`). A
//!   file is not expected to hold one, but the panel shows what it is given.

use std::borrow::Cow;
use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

use pid_parse::publish::backup_package::decode_zip_entry_name;

use super::manifest::{shown, HIDDEN};

/// How much of a file is read: twice the largest text file of the plants
/// seen so far (their data maps, 3.7 MB each).
pub const TEXT_LIMIT: usize = 8 << 20;

/// How much of a file is looked at to tell text from binary.
const SNIFF: usize = 8 << 10;

/// The longest row, in columns of a fixed-width font: a wide (CJK) character
/// takes two. A longer line continues on the next row.
pub const ROW_UNITS: usize = 160;

/// What separates the key and the fields of a manifest's line.
const MANIFEST_SEPARATOR: &str = "<<|>>";

/// A file read as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFile {
    /// The rows: the line's number (`None` on a row that continues the line
    /// above) and its text.
    pub rows: Vec<(Option<usize>, String)>,
    /// How many lines that is.
    pub line_count: usize,
    /// The encoding the bytes were read as.
    pub encoding: &'static str,
    /// Size of the whole file.
    pub bytes: u64,
    /// How much of it was read, when not all of it.
    pub cut_at: Option<u64>,
    /// How many credential-like values were replaced.
    pub hidden: usize,
}

impl TextFile {
    /// The lines whole: one continued over several rows is put together
    /// again.
    pub fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = Vec::with_capacity(self.line_count);
        for (number, text) in &self.rows {
            match (number, lines.last_mut()) {
                (None, Some(last)) => last.push_str(text),
                _ => lines.push(text.clone()),
            }
        }
        lines
    }
}

/// What a file turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readable {
    /// Text.
    Text(TextFile),
    /// Not text; its size.
    Binary(u64),
}

/// Read the file at `path`.
pub fn read_text_file(path: &Path) -> Result<Readable, String> {
    let fail = |error: std::io::Error| {
        crate::tf!("cannot read {}: {}", path.display(), error).into_owned()
    };
    let total = std::fs::metadata(path).map_err(fail)?.len();
    let file = std::fs::File::open(path).map_err(fail)?;
    read(file, total).map_err(fail)
}

/// Read what `reader` gives of a file `total` bytes long: its first
/// kilobytes, and the rest (up to [`TEXT_LIMIT`]) only when those are not a
/// binary file's. Most files of a backup are symbols and drawings, and a
/// click on one costs a glance at its start.
fn read(mut reader: impl Read, total: u64) -> std::io::Result<Readable> {
    let mut head = Vec::new();
    reader.by_ref().take(SNIFF as u64).read_to_end(&mut head)?;
    if starts_binary(&head) {
        return Ok(Readable::Binary(total));
    }
    reader
        .take((TEXT_LIMIT + 1 - head.len()) as u64)
        .read_to_end(&mut head)?;
    Ok(decode(&head, total))
}

/// Whether `head`, the start of a file, is a binary file's: no byte-order
/// mark, and a NUL among its first kilobytes.
fn starts_binary(head: &[u8]) -> bool {
    let marked = head.starts_with(&[0xFF, 0xFE])
        || head.starts_with(&[0xFE, 0xFF])
        || head.starts_with(&[0xEF, 0xBB, 0xBF]);
    !marked && head[..head.len().min(SNIFF)].contains(&0)
}

/// Read member `member` of the zip at `archive`. `name` is the member's
/// name as the index lists it: the member is taken by its position, and by
/// its name when the position holds another.
pub fn read_zip_member(archive: &Path, member: usize, name: &str) -> Result<Readable, String> {
    with_zip_member(archive, member, name, |entry, total| read(entry, total))
}

/// Find member `member` of the zip at `archive` the way [`read_zip_member`]
/// says, and hand it to `read` with the length it states. A reader of
/// something that is not text (a symbol file) finds its member the same
/// way. What fails is said with the archive's path.
pub(super) fn with_zip_member<T>(
    archive: &Path,
    member: usize,
    name: &str,
    read: impl FnOnce(&mut dyn Read, u64) -> std::io::Result<T>,
) -> Result<T, String> {
    let fail =
        |error: String| crate::tf!("cannot read {}: {}", archive.display(), error).into_owned();
    let file = std::fs::File::open(archive).map_err(|error| fail(error.to_string()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| fail(error.to_string()))?;
    let named = |zip: &mut zip::ZipArchive<std::fs::File>, at: usize| -> bool {
        zip.by_index_raw(at)
            .is_ok_and(|entry| decode_zip_entry_name(entry.name_raw()) == name)
    };
    let at = if named(&mut zip, member) {
        member
    } else {
        (0..zip.len())
            .find(|at| named(&mut zip, *at))
            .ok_or_else(|| crate::tf!("{} is not in {}", name, archive.display()).into_owned())?
    };
    let mut entry = zip.by_index(at).map_err(|error| fail(error.to_string()))?;
    let total = entry.size();
    read(&mut entry, total).map_err(|error| fail(error.to_string()))
}

/// Tell what `head`, the start of a file `total` bytes long, is, and read it
/// as text when it is text.
pub fn decode(head: &[u8], total: u64) -> Readable {
    let cut = head.len() > TEXT_LIMIT;
    let body = &head[..head.len().min(TEXT_LIMIT)];
    let (text, encoding) = match body {
        [0xFF, 0xFE, rest @ ..] => (utf16(rest, u16::from_le_bytes), "UTF-16 LE"),
        [0xFE, 0xFF, rest @ ..] => (utf16(rest, u16::from_be_bytes), "UTF-16 BE"),
        [0xEF, 0xBB, 0xBF, rest @ ..] => (String::from_utf8_lossy(rest).into_owned(), "UTF-8"),
        _ => {
            if starts_binary(body) {
                return Readable::Binary(total);
            }
            match std::str::from_utf8(body) {
                Ok(text) => (text.to_string(), "UTF-8"),
                // A cut may have split the last character.
                Err(error) if cut && error.valid_up_to() + 4 > body.len() => (
                    String::from_utf8_lossy(&body[..error.valid_up_to()]).into_owned(),
                    "UTF-8",
                ),
                Err(_) => legacy(body),
            }
        }
    };
    if !looks_like_text(&text) {
        return Readable::Binary(total);
    }
    // A cut file ends on its last whole line.
    let text = match (cut, text.rfind(['\n', '\r'])) {
        (true, Some(end)) => &text[..end],
        _ => text.as_str(),
    };
    let mut rows = Vec::new();
    let mut hidden = 0;
    let mut line_count = 0;
    // An empty file has no line, not one empty line.
    let lines = if text.is_empty() {
        Vec::new()
    } else {
        split_lines(text)
    };
    for line in lines {
        line_count += 1;
        let widened;
        let line = if line.contains('\t') {
            widened = line.replace('\t', "    ");
            widened.as_str()
        } else {
            line
        };
        let masked;
        let line = match hide_credentials(line) {
            Some((text, count)) => {
                hidden += count;
                masked = text;
                masked.as_str()
            }
            None => line,
        };
        let mut number = Some(line_count);
        for part in split_row(line) {
            rows.push((number.take(), part.to_string()));
        }
    }
    // What follows the line break a file ends in is not a line.
    if line_count > 1 && rows.last().is_some_and(|(_, text)| text.is_empty()) {
        rows.pop();
        line_count -= 1;
    }
    Readable::Text(TextFile {
        rows,
        line_count,
        encoding,
        bytes: total,
        cut_at: cut.then_some(TEXT_LIMIT as u64),
        hidden,
    })
}

/// The lines of `text`. A line ends at a line feed, at a carriage return, or
/// at the two together: a file whose lines end in carriage returns alone is
/// rare, but read as one long line its fields would not be where the rules
/// that hide a manifest's credentials look for them.
fn split_lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        let end = match bytes[at] {
            b'\n' => 1,
            b'\r' if bytes.get(at + 1) == Some(&b'\n') => 2,
            b'\r' => 1,
            _ => {
                at += 1;
                continue;
            }
        };
        lines.push(&text[start..at]);
        at += end;
        start = at;
    }
    lines.push(&text[start..]);
    lines
}

fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| unit([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Bytes that are not UTF-8, in the code page of the plants this reads; in
/// Windows-1252, which reads anything, when they are not that either.
fn legacy(bytes: &[u8]) -> (String, &'static str) {
    use document::io::dxf::code_page::try_encoding_from_code_page;
    if let Some(gbk) = try_encoding_from_code_page("ANSI_936") {
        let (text, had_errors) = gbk.decode_without_bom_handling(bytes);
        if !had_errors {
            return (text.into_owned(), "GBK");
        }
    }
    match try_encoding_from_code_page("ANSI_1252") {
        Some(latin) => (
            latin.decode_without_bom_handling(bytes).0.into_owned(),
            "Windows-1252",
        ),
        None => (String::from_utf8_lossy(bytes).into_owned(), "UTF-8"),
    }
}

/// Whether `text` reads as text: hardly any control or replacement
/// characters at its start.
fn looks_like_text(text: &str) -> bool {
    let mut total = 0usize;
    let mut odd = 0usize;
    for ch in text.chars().take(SNIFF) {
        total += 1;
        if ch == '\u{fffd}' || (ch.is_control() && !matches!(ch, '\n' | '\r' | '\t' | '\u{c}')) {
            odd += 1;
        }
    }
    odd * 50 <= total
}

/// Whether `word` has the look of a value a manifest obfuscates: long, and
/// mostly `#` around a few other characters. A rule of `#` alone is a
/// comment's, and a number format (`#,##0.00`) is short.
fn looks_obfuscated(word: &str) -> bool {
    let total = word.chars().count();
    let hashes = word.chars().filter(|ch| *ch == '#').count();
    total >= 32 && hashes * 2 >= total && hashes < total
}

/// `word` as `user/•••@host`, when it spells a database login the way a
/// command line does: a user name, a password free of the characters a path
/// or an expression would have there, and a host.
fn hide_login(word: &str) -> Option<String> {
    let (user, rest) = word.split_once('/')?;
    let (password, host) = rest.rsplit_once('@')?;
    let name = |text: &str, more: &str| {
        !text.is_empty()
            && text
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || more.contains(ch))
    };
    let plain = !password.is_empty()
        && !password
            .chars()
            .any(|ch| "/\\[](){}<>=\"',;".contains(ch) || ch.is_whitespace());
    (name(user, "_$#.") && plain && name(host, "_.:-/")).then(|| format!("{user}/{HIDDEN}@{host}"))
}

/// The words a key ends in when what it holds is a password.
const PASSWORD_WORDS: &str = "password|passwd|pwd|passphrase|密码|口令";

fn pattern(cell: &'static OnceLock<regex::Regex>, source: String) -> &'static regex::Regex {
    cell.get_or_init(|| regex::Regex::new(&source).expect("the pattern is a constant"))
}

/// Whether a line names a password at all: what decides, for nearly every
/// line of every file, that there is nothing more to look for.
fn names_a_password(line: &str) -> bool {
    static NAMED: OnceLock<regex::Regex> = OnceLock::new();
    pattern(&NAMED, format!("(?i){PASSWORD_WORDS}")).is_match(line)
}

/// The shapes a password takes in a settings file, a connection string, an
/// XML or a JSON document. Each has the value as its `value` group.
fn password_shapes() -> [&'static regex::Regex; 3] {
    static PAIR: OnceLock<regex::Regex> = OnceLock::new();
    static ELEMENT: OnceLock<regex::Regex> = OnceLock::new();
    static SETTING: OnceLock<regex::Regex> = OnceLock::new();
    // A key is letters, digits and `_ . -`, and ends in one of the words:
    // `Password`, `db_pwd`, `数据库密码`, but neither `PassValue` nor
    // `PasswordHint`.
    let key = format!(r"[\p{{L}}\p{{N}}_.\-]*(?:{PASSWORD_WORDS})");
    [
        // `Password=x`, `pwd: x`, `"password": "x"`, `DbPassword="x y"`.
        pattern(
            &PAIR,
            format!(r#"(?i){key}["']?\s*[=:]\s*(?P<value>"[^"]*"|'[^']*'|[^\s;,&<>"']+)"#),
        ),
        // `<Password>x</Password>`.
        pattern(
            &ELEMENT,
            format!(
                r"(?i)<[\p{{L}}\p{{N}}_.:\-]*(?:{PASSWORD_WORDS})(?:\s[^<>]*)?>(?P<value>[^<]+)<"
            ),
        ),
        // `<add key="DbPassword" value="x"/>`.
        pattern(
            &SETTING,
            format!(
                r#"(?i)\b(?:key|name)\s*=\s*"[^"]*(?:{PASSWORD_WORDS})"[^<>]*?\bvalue\s*=\s*(?P<value>"[^"]*")"#
            ),
        ),
    ]
}

/// `line` with the value of everything called a password replaced, counting
/// them in `hidden`.
fn hide_named_passwords<'a>(line: &'a str, hidden: &mut usize) -> Cow<'a, str> {
    let mut line = Cow::Borrowed(line);
    for shape in password_shapes() {
        let replaced = shape.replace_all(&line, |found: &regex::Captures<'_>| {
            let whole = found.get(0).expect("the whole match");
            let value = found.name("value").expect("every shape has a value");
            let quote = value
                .as_str()
                .chars()
                .next()
                .filter(|ch| matches!(ch, '"' | '\''))
                .map(String::from)
                .unwrap_or_default();
            let inner = value.as_str().trim_matches(['"', '\'']).trim();
            // Nothing there, or hidden already by the shape before.
            if inner.is_empty() || inner == HIDDEN {
                return whole.as_str().to_string();
            }
            *hidden += 1;
            let text = whole.as_str();
            format!(
                "{}{quote}{HIDDEN}{quote}{}",
                &text[..value.start() - whole.start()],
                &text[value.end() - whole.start()..],
            )
        });
        if let Cow::Owned(replaced) = replaced {
            line = Cow::Owned(replaced);
        }
    }
    line
}

/// `line` with what looks like a credential replaced, and how many were;
/// `None` when it holds none, which is every line of nearly every file.
fn hide_credentials(line: &str) -> Option<(String, usize)> {
    let mut hidden = 0;
    // A line of a manifest: its fields by the manifest's own rule.
    if let Some((key, rest)) = line.split_once(MANIFEST_SEPARATOR) {
        let fields: Vec<String> = rest
            .split(MANIFEST_SEPARATOR)
            .enumerate()
            .map(|(at, field)| shown(key.trim(), at, field, &mut hidden))
            .collect();
        return (hidden > 0).then(|| {
            (
                format!(
                    "{key}{MANIFEST_SEPARATOR}{}",
                    fields.join(MANIFEST_SEPARATOR)
                ),
                hidden,
            )
        });
    }
    let named = names_a_password(line);
    if !(named || line.contains("#####") || (line.contains('@') && line.contains('/'))) {
        return None;
    }
    let line = if named {
        hide_named_passwords(line, &mut hidden)
    } else {
        Cow::Borrowed(line)
    };
    let words: Vec<String> = line
        .split(' ')
        .map(|word| {
            if looks_obfuscated(word) {
                hidden += 1;
                HIDDEN.to_string()
            } else if let Some(masked) = hide_login(word) {
                hidden += 1;
                masked
            } else {
                word.to_string()
            }
        })
        .collect();
    (hidden > 0).then(|| (words.join(" "), hidden))
}

/// The columns `ch` takes in a fixed-width font.
fn units(ch: char) -> usize {
    if ch.is_ascii() {
        1
    } else {
        2
    }
}

/// `line` in rows of at most [`ROW_UNITS`] columns, broken after a space
/// where there is one in the last third of a row.
fn split_row(line: &str) -> Vec<&str> {
    // Bytes are never fewer than columns.
    if line.len() <= ROW_UNITS {
        return vec![line];
    }
    let mut rows = Vec::new();
    let mut rest = line;
    loop {
        let mut width = 0;
        let mut end = rest.len();
        let mut space = None;
        for (at, ch) in rest.char_indices() {
            if width + units(ch) > ROW_UNITS {
                end = at;
                break;
            }
            width += units(ch);
            if ch == ' ' && width >= ROW_UNITS * 2 / 3 {
                space = Some(at + 1);
            }
        }
        if end == rest.len() {
            rows.push(rest);
            return rows;
        }
        let end = space.unwrap_or(end);
        rows.push(&rest[..end]);
        rest = &rest[end..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn text(readable: Readable) -> TextFile {
        match readable {
            Readable::Text(file) => file,
            Readable::Binary(bytes) => panic!("read as binary ({bytes} bytes)"),
        }
    }

    fn lines(file: &TextFile) -> Vec<&str> {
        file.rows.iter().map(|(_, text)| text.as_str()).collect()
    }

    #[test]
    fn utf8_text_is_read_line_by_line() {
        let file = text(decode(
            "\"Begin Rules\",120\r\n\tindented\r\n末行\r\n".as_bytes(),
            40,
        ));
        assert_eq!(file.encoding, "UTF-8");
        assert_eq!(
            file.rows,
            [
                (Some(1), "\"Begin Rules\",120".to_string()),
                (Some(2), "    indented".to_string()),
                (Some(3), "末行".to_string()),
            ]
        );
        assert_eq!(
            (file.line_count, file.bytes, file.cut_at, file.hidden),
            (3, 40, None, 0)
        );
        // A file that does not end in a line break keeps its last line, and
        // an empty line in the middle stays one.
        assert_eq!(lines(&text(decode(b"a\n\nb", 4))), ["a", "", "b"]);
        // An empty file is text with no line; a lone line break is one
        // empty line.
        let empty = text(decode(b"", 0));
        assert_eq!((empty.rows.len(), empty.line_count), (0, 0));
        let blank = text(decode(b"\n", 1));
        assert_eq!((lines(&blank), blank.line_count), (vec![""], 1));
    }

    #[test]
    fn the_byte_order_mark_decides_the_encoding() {
        let utf8 = text(decode(b"\xEF\xBB\xBF<?xml?>\n<a/>", 15));
        assert_eq!(
            (utf8.encoding, lines(&utf8)),
            ("UTF-8", vec!["<?xml?>", "<a/>"])
        );

        let mut le = vec![0xFF, 0xFE];
        let mut be = vec![0xFE, 0xFF];
        for unit in "名称<<|>>值\r\n第二行".encode_utf16() {
            le.extend(unit.to_le_bytes());
            be.extend(unit.to_be_bytes());
        }
        let le = text(decode(&le, le.len() as u64));
        assert_eq!(
            (le.encoding, lines(&le)),
            ("UTF-16 LE", vec!["名称<<|>>值", "第二行"])
        );
        let be = text(decode(&be, be.len() as u64));
        assert_eq!(
            (be.encoding, lines(&be)),
            ("UTF-16 BE", vec!["名称<<|>>值", "第二行"])
        );
    }

    #[test]
    fn bytes_that_are_not_utf8_are_read_in_the_plants_code_page() {
        // "保温规范" in GBK, between two ASCII lines.
        let mut bytes = b"<Spec>\n".to_vec();
        bytes.extend([0xB1, 0xA3, 0xCE, 0xC2, 0xB9, 0xE6, 0xB7, 0xB6]);
        bytes.extend(b"\n</Spec>\n");
        let file = text(decode(&bytes, bytes.len() as u64));
        assert_eq!(file.encoding, "GBK");
        assert_eq!(lines(&file), ["<Spec>", "保温规范", "</Spec>"]);
        // Not GBK either (0xFF starts no GBK character): Windows-1252, which
        // reads every byte.
        let file = text(decode(b"caf\xE9 \xFF\n", 7));
        assert_eq!(file.encoding, "Windows-1252");
        assert_eq!(lines(&file), ["café ÿ"]);
    }

    #[test]
    fn a_binary_file_is_not_shown_as_text() {
        // A compound file starts with its signature and is full of NULs.
        let mut ole = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        ole.extend([0u8; 600]);
        assert_eq!(decode(&ole, 122_880), Readable::Binary(122_880));
        // No NUL, but nothing that reads as text either.
        let noise: Vec<u8> = (1u8..=31)
            .cycle()
            .take(4000)
            .filter(|byte| !matches!(byte, b'\n' | b'\r' | b'\t' | 0x0c))
            .collect();
        assert_eq!(decode(&noise, 4000), Readable::Binary(4000));
    }

    #[test]
    fn a_long_file_is_cut_at_its_last_whole_line() {
        let line = "0123456789abcdef0123456789abcde\n";
        let whole = TEXT_LIMIT / line.len();
        let mut bytes = line.repeat(whole + 10).into_bytes();
        let total = bytes.len() as u64;
        // What a reader hands over: one byte more than the limit, cutting
        // the line the limit falls in.
        bytes.truncate(TEXT_LIMIT + 1);
        let file = text(decode(&bytes, total));
        assert_eq!(file.cut_at, Some(TEXT_LIMIT as u64));
        assert_eq!(file.bytes, total);
        assert_eq!(file.line_count, whole);
        assert_eq!(file.rows.len(), whole);
        assert!(file
            .rows
            .iter()
            .all(|(_, text)| text.len() == line.len() - 1));
        // A file of exactly the limit is whole.
        let exact = text(decode(&bytes[..TEXT_LIMIT], TEXT_LIMIT as u64));
        assert_eq!(exact.cut_at, None);
    }

    #[test]
    fn a_line_too_long_for_a_row_continues_on_the_next() {
        let long = format!("{} {}", "x".repeat(150), "y".repeat(100));
        let file = text(decode(format!("short\n{long}\nend").as_bytes(), 0));
        assert_eq!(file.line_count, 3);
        assert_eq!(
            file.rows
                .iter()
                .map(|(number, _)| *number)
                .collect::<Vec<_>>(),
            [Some(1), Some(2), None, Some(3)]
        );
        // Broken after the space, nothing lost.
        assert_eq!(file.rows[1].1, format!("{} ", "x".repeat(150)));
        assert_eq!(file.rows[2].1, "y".repeat(100));
        // No space to break at: cut at the row's width.
        let solid = "z".repeat(ROW_UNITS * 2 + 5);
        assert_eq!(
            split_row(&solid)
                .iter()
                .map(|row| row.len())
                .collect::<Vec<_>>(),
            [ROW_UNITS, ROW_UNITS, 5]
        );
        // A wide character takes two columns, and is never split.
        let wide = "图".repeat(ROW_UNITS);
        let rows = split_row(&wide);
        assert_eq!(
            rows.iter()
                .map(|row| row.chars().count())
                .collect::<Vec<_>>(),
            [ROW_UNITS / 2, ROW_UNITS / 2]
        );
        assert_eq!(rows.concat(), wide);
        // A line of exactly a row stays one.
        assert_eq!(split_row(&"k".repeat(ROW_UNITS)).len(), 1);
        // Put together again, the rows are the lines they were cut from.
        assert_eq!(file.lines(), ["short", long.as_str(), "end"]);
    }

    /// What a credential looks like, made up for the tests.
    const OBFUSCATED: &str = "Ab######cd######ef#######gh#######ij######kl######mn######op####";
    const PASSWORD: &str = "made-up-pw";

    #[test]
    fn what_looks_like_a_credential_is_not_shown() {
        let source = format!(
            "plain line\nvalue {OBFUSCATED} next\nrun exp scott/{PASSWORD}@db.host:1521/orcl now\nDBPwds<<|>>{PASSWORD}<<|>>second\nSiteConnInfo<<|>>{PASSWORD}<<|>>1<<|>>ORCL\n"
        );
        let file = text(decode(source.as_bytes(), 0));
        assert_eq!(
            lines(&file),
            [
                "plain line".to_string(),
                format!("value {HIDDEN} next"),
                format!("run exp scott/{HIDDEN}@db.host:1521/orcl now"),
                format!("DBPwds<<|>>{HIDDEN}<<|>>{HIDDEN}"),
                format!("SiteConnInfo<<|>>{HIDDEN}<<|>>1<<|>>ORCL"),
            ]
        );
        assert_eq!(file.hidden, 5);
        let debug = format!("{file:?}");
        assert!(!debug.contains(PASSWORD) && !debug.contains(OBFUSCATED));
    }

    /// Every kind of credential a manifest holds, each a value made up for
    /// this test and spelled so that only the rule for its place can hide it.
    const MANIFEST_SECRETS: [&str; 6] = [
        "site-secret-one",
        "site-secret-two",
        "plant-secret",
        "tablespace-secret",
        "dbpwds-secret",
        "command-secret",
    ];

    fn manifest_copy(line_end: &str) -> String {
        [
            "Name<<|>>SQPlant",
            "SiteConnInfo<<|>>site-secret-one<<|>>1<<|>>ORCL<<|>>SQSite<<|>>site-secret-two<<|>>system",
            "PlantConnInfo<<|>>SQPlant<<|>>2<<|>>ORCL<<|>>SQPlant<<|>>plant-secret<<|>>system",
            "TableSpace<<|>>Permanent<<|>>tablespace-secret<<|>>USERS",
            "DBUids<<|>>SQPlantpid,SQPlant",
            "DBPwds<<|>>dbpwds-secret",
            "BackupCommand<<|>>Exp.exe system/command-secret@ORCL OWNER=(SQPlant) FILE=Export.dmp",
            "Table<<|>>SQPlant<<|>>T_PLANTGROUP",
            "",
        ]
        .join(line_end)
    }

    /// A manifest is never opened as text, but a copy of one can lie
    /// anywhere: under another name, inside an archive, among the reference
    /// files. Whatever its encoding and its line ends, it shows what the
    /// manifest's own branch shows and no more.
    #[test]
    fn a_copy_of_a_manifest_shows_none_of_its_credentials() {
        let utf16 = |text: &str| -> Vec<u8> {
            let mut bytes = vec![0xFF, 0xFE];
            for unit in text.encode_utf16() {
                bytes.extend(unit.to_le_bytes());
            }
            bytes
        };
        for (what, bytes) in [
            (
                "UTF-16, CRLF (as SmartPlant writes it)",
                utf16(&manifest_copy("\r\n")),
            ),
            ("UTF-8, LF", manifest_copy("\n").into_bytes()),
            ("UTF-8, CRLF", manifest_copy("\r\n").into_bytes()),
            ("UTF-8, CR alone", manifest_copy("\r").into_bytes()),
            ("UTF-16, CR alone", utf16(&manifest_copy("\r"))),
        ] {
            let file = text(decode(&bytes, bytes.len() as u64));
            let shown = lines(&file).join("\n");
            let leaking: Vec<usize> = MANIFEST_SECRETS
                .iter()
                .enumerate()
                .filter(|(_, secret)| shown.contains(**secret))
                .map(|(at, _)| at)
                .collect();
            assert!(
                leaking.is_empty(),
                "{what}: the credentials {leaking:?} of MANIFEST_SECRETS are shown"
            );
            assert_eq!(file.hidden, 6, "{what}");
            assert_eq!(file.line_count, 8, "{what}");
            // What is not a credential is all there.
            assert!(shown.contains("DBUids<<|>>SQPlantpid,SQPlant"), "{what}");
            assert!(
                shown.contains(&format!("Exp.exe system/{HIDDEN}@ORCL OWNER=(SQPlant)")),
                "{what}"
            );
        }
    }

    /// A password in a settings file, a connection string, an XML or JSON
    /// document: the value is hidden whatever the file is.
    #[test]
    fn a_password_in_any_text_file_is_not_shown() {
        let source = [
            "Password=made-up-01",
            "Provider=OraOLEDB.Oracle;User ID=plant;Password=made-up-02;Data Source=ORCL",
            "Driver={SQL Server};Server=db;Uid=sa;Pwd=made-up-03;",
            "<Connection user=\"plant\" DbPassword=\"made-up 04\" timeout=\"30\"/>",
            "  <Password>made-up 05</Password>",
            "  \"password\": \"made-up-06\",",
            "db_pwd : made-up-07",
            "数据库密码=made-up-08",
            "PASSPHRASE='made-up 09'",
            "sqlplus -s system/made-up-10@orcl",
            "<add key=\"DbPassword\" value=\"made-up-11\"/>",
        ]
        .join("\n");
        let file = text(decode(source.as_bytes(), source.len() as u64));
        let shown = lines(&file).join("\n");
        // Which line still shows its value is said; the value is not.
        let leaking: Vec<usize> = lines(&file)
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains("made-up"))
            .map(|(at, _)| at + 1)
            .collect();
        assert!(leaking.is_empty(), "lines {leaking:?} show their password");
        assert!(!shown.contains("made-up"));
        assert_eq!(file.hidden, 11);
        assert_eq!(
            lines(&file),
            [
                format!("Password={HIDDEN}"),
                format!(
                    "Provider=OraOLEDB.Oracle;User ID=plant;Password={HIDDEN};Data Source=ORCL"
                ),
                format!("Driver={{SQL Server}};Server=db;Uid=sa;Pwd={HIDDEN};"),
                format!("<Connection user=\"plant\" DbPassword=\"{HIDDEN}\" timeout=\"30\"/>"),
                format!("  <Password>{HIDDEN}</Password>"),
                format!("  \"password\": \"{HIDDEN}\","),
                format!("db_pwd : {HIDDEN}"),
                format!("数据库密码={HIDDEN}"),
                format!("PASSPHRASE='{HIDDEN}'"),
                format!("sqlplus -s system/{HIDDEN}@orcl"),
                format!("<add key=\"DbPassword\" value=\"{HIDDEN}\"/>"),
            ]
        );
    }

    /// Words that only start like "password", and a password key with
    /// nothing after it, hide nothing.
    #[test]
    fn a_key_that_is_not_a_password_keeps_its_value() {
        for line in [
            // Real: the validation rules of a plant (`ObjectAttrValidation.xml`).
            "<Attr Name=\"TagSeqNo\" PassValue=\"是\" FailValue=\"否\"/>",
            "<PassCount>3</PassCount>",
            "Bypass=1; Compass=north; PasswordHint=the usual",
            "Password=",
            "<Password></Password>",
            "password \"\" is not allowed",
            "Passwords are kept by the site administrator.",
        ] {
            let file = text(decode(line.as_bytes(), 0));
            assert_eq!(lines(&file), [line], "{line}");
            assert_eq!(file.hidden, 0, "{line}");
        }
    }

    #[test]
    fn a_line_ends_at_a_line_feed_a_carriage_return_or_both() {
        let file = text(decode(b"one\rtwo\r\nthree\nfour\r", 0));
        assert_eq!(lines(&file), ["one", "two", "three", "four"]);
        assert_eq!(file.line_count, 4);
        // Empty lines stay, whatever ends them.
        assert_eq!(
            lines(&text(decode(b"a\r\rb\n\r\nc", 0))),
            ["a", "", "b", "", "c"]
        );
    }

    #[test]
    fn what_only_resembles_a_credential_is_left_alone() {
        for line in [
            // A comment's rule, a number format, a short run of hashes.
            "################################################################",
            "format #,##0.00 and ####.## and #####",
            // Paths, expressions and addresses with a slash and an at sign.
            "select a/b[@id='1'] from //root/item[@name]",
            "see http://example.com/path@v2 and mail to:dev/ops@example.com,x",
            "ratio 1/2 @ 20C",
            // A manifest's line that holds no credential.
            "Table<<|>>SQPlant<<|>>T_DRAWING",
        ] {
            let file = text(decode(line.as_bytes(), 0));
            assert_eq!(lines(&file), [line], "{line}");
            assert_eq!(file.hidden, 0, "{line}");
        }
    }

    /// A file that counts what is read of it.
    struct Counted<'a> {
        bytes: &'a [u8],
        read: usize,
    }

    impl Read for Counted<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let rest = &self.bytes[self.read..];
            let count = rest.len().min(buffer.len());
            buffer[..count].copy_from_slice(&rest[..count]);
            self.read += count;
            Ok(count)
        }
    }

    #[test]
    fn a_binary_file_costs_a_glance_at_its_start() {
        // A megabyte of a drawing: only its first kilobytes are read.
        let mut drawing = vec![0xD0, 0xCF, 0x11, 0xE0];
        drawing.resize(1 << 20, 0);
        let mut file = Counted {
            bytes: &drawing,
            read: 0,
        };
        assert_eq!(
            read(&mut file, drawing.len() as u64).unwrap(),
            Readable::Binary(1 << 20)
        );
        assert_eq!(file.read, SNIFF);
        // Text of the same size is read to its end, and comes out the same
        // as when it is decoded in one piece.
        let lines = "a line of a text file\n".repeat((1 << 20) / 22);
        let mut file = Counted {
            bytes: lines.as_bytes(),
            read: 0,
        };
        let whole = read(&mut file, lines.len() as u64).unwrap();
        assert_eq!(file.read, lines.len());
        assert_eq!(whole, decode(lines.as_bytes(), lines.len() as u64));
        // A byte-order mark makes NULs text: UTF-16 is full of them.
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "<a/>\n".repeat(4000).encode_utf16() {
            utf16.extend(unit.to_le_bytes());
        }
        let mut file = Counted {
            bytes: &utf16,
            read: 0,
        };
        assert_eq!(text(read(&mut file, 0).unwrap()).line_count, 4000);
        assert_eq!(file.read, utf16.len());
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("ocs-pid-backup-text-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_file_and_an_archive_member_are_read_from_disk() {
        let root = scratch("read");
        let loose = root.join("Export.log");
        std::fs::write(
            &loose,
            "Connected to: Oracle\n. exporting table T_DRAWING 53 rows exported\n",
        )
        .unwrap();
        let file = text(read_text_file(&loose).unwrap());
        assert_eq!(file.line_count, 2);
        assert_eq!(file.bytes, std::fs::metadata(&loose).unwrap().len());

        let archive = root.join("RefData~4~809.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.add_directory("Maps/", options).unwrap();
        writer.start_file("Maps/first.xml", options).unwrap();
        writer.write_all(b"<first/>").unwrap();
        writer.start_file("Maps/second.xml", options).unwrap();
        writer.write_all(b"<second>\n</second>").unwrap();
        writer.start_file("Maps/symbol.sym", options).unwrap();
        writer
            .write_all(&[0xD0, 0xCF, 0x11, 0xE0, 0, 0, 0, 0])
            .unwrap();
        writer.finish().unwrap();
        assert_eq!(
            lines(&text(
                read_zip_member(&archive, 2, "Maps/second.xml").unwrap()
            )),
            ["<second>", "</second>"]
        );
        // The position holds another member: the name finds the right one.
        assert_eq!(
            lines(&text(
                read_zip_member(&archive, 0, "Maps/first.xml").unwrap()
            )),
            ["<first/>"]
        );
        assert_eq!(
            read_zip_member(&archive, 3, "Maps/symbol.sym").unwrap(),
            Readable::Binary(8)
        );
        // Neither the position nor the name: an error that says which.
        let error = read_zip_member(&archive, 1, "Maps/gone.xml").unwrap_err();
        assert!(error.contains("Maps/gone.xml"), "{error}");
        assert!(read_text_file(&root.join("absent.txt")).is_err());
        assert!(read_zip_member(&loose, 0, "x").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
