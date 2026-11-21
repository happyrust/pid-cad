use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 层级选择：序号在哪一层共享/独立。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HierarchyMode {
    /// 当前类型依次递增：子类型从自身起点独立计数（契约默认）。
    #[serde(rename = "当前类型依次递增", alias = "TYPE", alias = "CurrentTypeIncrement")]
    CurrentTypeIncrement,
    /// 父类型递增：子类型接续父类型已用最大序号。
    #[serde(rename = "父类型递增", alias = "PARENT", alias = "ParentTypeIncrement")]
    ParentTypeIncrement,
}

impl Default for HierarchyMode {
    fn default() -> Self {
        Self::CurrentTypeIncrement
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagRule {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_auto_code")]
    pub auto_code: bool,
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub start: i64,
    #[serde(default = "default_step")]
    pub step: i64,
    #[serde(default)]
    pub suffix_increment: bool,
    #[serde(default)]
    pub level: Option<HierarchyMode>,
    #[serde(default)]
    pub join: Option<String>,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub pad: u32,
}

fn default_auto_code() -> bool {
    true
}

fn default_step() -> i64 {
    1
}

impl Default for TagRule {
    fn default() -> Self {
        Self {
            name: String::new(),
            auto_code: default_auto_code(),
            pattern: String::new(),
            start: 0,
            step: default_step(),
            suffix_increment: false,
            level: None,
            join: None,
            unique: false,
            pad: 0,
        }
    }
}

impl TagRule {
    pub const SEQUENCE_PLACEHOLDER: &'static str = "{序号}";
    pub const SUFFIX_PLACEHOLDER: &'static str = "{后缀}";

    pub fn new(pattern: impl Into<String>, start: i64) -> Self {
        Self {
            pattern: pattern.into(),
            start,
            ..Self::default()
        }
    }

    pub fn has_sequence(&self) -> bool {
        self.pattern.contains(Self::SEQUENCE_PLACEHOLDER)
    }

    pub fn has_suffix(&self) -> bool {
        self.pattern.contains(Self::SUFFIX_PLACEHOLDER)
    }

    pub fn hierarchy(&self) -> HierarchyMode {
        self.level.unwrap_or_default()
    }
}

/// `architecture.md` 附录 A.9 的默认位号规则表。
///
/// 类型库（`assets/pid-types.json`）只给根 `EQUIPMENT` 定义了规则：`VALVE`
/// 与 `INSTRUMENT` 不进那棵树（D4 / D16：`TypeLib` 强制单根，且
/// `VAL-SEM-015` 断言 `EQUIPMENT` 的子类型恰为 `["PUMP","VESSEL"]`），
/// 而符号库的 `symbol_type_map` 又会把它们作为作者类型编码写进
/// `PID_AUTHOR` 的 `type` 键。于是**库外的类型编码**在解析位号规则时按这张
/// 表兜底，库内的类型编码仍只认类型树（库里有节点但整条祖先链都没规则时
/// 仍报 `TAG_RULE_MISSING`，见 `empty_rule_reports_missing_rule`）。
///
/// 每条都是 `unique = true`：唯一键是 `(类型编码, 位号)` 复合键，同类型同
/// 位号即冲突。
pub fn default_tag_rule(type_code: &str) -> Option<TagRule> {
    let code = type_code.trim().to_ascii_uppercase();
    let rule = match code.as_str() {
        // 设备：`P-1001`、`P-1002`…
        "EQUIPMENT" => TagRule::new("P-{序号}", 1001),
        // 阀门：`V-1001` 起，与设备的序号空间互不相干。
        "VALVE" => TagRule::new("V-{序号}", 1001),
        // 仪表：前缀取 `仪表功能` 属性（`TI` / `PI` / `FI`…），故 `TI-1001`。
        "INSTRUMENT" => {
            let mut rule = TagRule::new("{仪表功能}-{序号}", 1001);
            rule.join = Some("仪表功能".to_string());
            rule
        }
        // 管线：`L-1001`…
        "PIPE_LINE" => TagRule::new("L-{序号}", 1001),
        // 管路：`{管线号}-01`、`{管线号}-02`…，后缀两位补零、递增。
        "PIPE_RUN" => {
            let mut rule = TagRule::new("{管线号}-{后缀}", 1);
            rule.join = Some("管线号".to_string());
            rule.suffix_increment = true;
            rule
        }
        _ => return None,
    };
    Some(TagRule {
        unique: true,
        ..rule
    })
}

/// 位号是不是照这条规则的模板写的（规则检查的 `位号合规`）。字面部分逐字比；`{序号}`、
/// `{后缀}` 对一串 ASCII 数字；其余占位符对同名属性的当前值，属性为空时对任意非空文本。
/// 后缀不递增时位号引擎把 `{后缀}` 连同紧挨在前面的分隔符一起去掉，所以不带后缀的也算。
/// 位号和属性值都去掉首尾空白再比。位号为空或模板为空时不知道，回 `None`。
pub fn matches(rule: &TagRule, tag: &str, props: &BTreeMap<String, String>) -> Option<bool> {
    let tag = tag.trim();
    if tag.is_empty() || rule.pattern.trim().is_empty() {
        return None;
    }
    let follows = |pattern: &str| fits(&pieces(pattern, props), tag);
    Some(
        follows(&rule.pattern)
            || (rule.has_suffix()
                && !rule.suffix_increment
                && follows(&without_suffix(&rule.pattern))),
    )
}

/// 模板的一段。
enum Piece {
    /// 照原样比的文字：字面部分，和有值的属性。
    Text(String),
    /// `{序号}`、`{后缀}`：一串数字。
    Digits,
    /// 属性为空的占位符：任意非空文本。
    Any,
}

/// 把模板拆成段，属性占位符按 `props` 填上。没有配对 `}` 的 `{` 当字面。
fn pieces(pattern: &str, props: &BTreeMap<String, String>) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|close| open + close) else {
            break;
        };
        text.push_str(&rest[..open]);
        let placeholder = &rest[open..=close];
        rest = &rest[close + 1..];
        let piece = if placeholder == TagRule::SEQUENCE_PLACEHOLDER
            || placeholder == TagRule::SUFFIX_PLACEHOLDER
        {
            Piece::Digits
        } else {
            let name = &placeholder[1..placeholder.len() - 1];
            match props.get(name).map(|value| value.trim()) {
                Some(value) if !value.is_empty() => {
                    text.push_str(value);
                    continue;
                }
                _ => Piece::Any,
            }
        };
        if !text.is_empty() {
            pieces.push(Piece::Text(std::mem::take(&mut text)));
        }
        pieces.push(piece);
    }
    text.push_str(rest);
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    pieces
}

/// `text` 从头到尾能不能对上这几段。数字和任意文本都可长可短，对不上就回头换一种分法。
fn fits(pieces: &[Piece], text: &str) -> bool {
    let Some((first, rest)) = pieces.split_first() else {
        return text.is_empty();
    };
    match first {
        Piece::Text(expected) => text
            .strip_prefix(expected.as_str())
            .is_some_and(|after| fits(rest, after)),
        Piece::Digits => {
            let digits = text.bytes().take_while(u8::is_ascii_digit).count();
            (1..=digits).rev().any(|end| fits(rest, &text[end..]))
        }
        Piece::Any => text
            .char_indices()
            .map(|(at, ch)| at + ch.len_utf8())
            .any(|end| fits(rest, &text[end..])),
    }
}

/// 后缀不递增时位号引擎写出来的样子：`{后缀}` 连同紧挨在前面的分隔符一起去掉，与引擎
/// 渲染位号时的做法一致。
fn without_suffix(pattern: &str) -> String {
    let mut pattern = pattern.to_owned();
    for glued in ["-", "_", ".", "/"] {
        pattern = pattern.replace(&format!("{glued}{}", TagRule::SUFFIX_PLACEHOLDER), "");
    }
    pattern.replace(TagRule::SUFFIX_PLACEHOLDER, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    /// 字面部分逐字比，`{序号}` 对一串数字；位号去掉首尾空白再比。
    #[test]
    fn literals_compare_exactly_and_the_sequence_takes_digits() {
        let rule = default_tag_rule("EQUIPMENT").expect("设备的默认规则");
        let none = props(&[]);
        for tag in ["P-1001", "P-7", " P-0012 "] {
            assert_eq!(matches(&rule, tag, &none), Some(true), "{tag}");
        }
        for tag in [
            "P-", "P-10A", "p-1001", "PP-1001", "P-1001-1", "P_1001", "P-１０",
        ] {
            assert_eq!(matches(&rule, tag, &none), Some(false), "{tag}");
        }
    }

    /// 别的占位符对同名属性的当前值（去掉首尾空白）；属性为空或没有时对任意非空文本。
    #[test]
    fn other_placeholders_take_the_property_or_any_text() {
        let rule = default_tag_rule("INSTRUMENT").expect("仪表的默认规则");
        let ti = props(&[("仪表功能", " TI ")]);
        assert_eq!(matches(&rule, "TI-1001", &ti), Some(true));
        assert_eq!(matches(&rule, "PI-1001", &ti), Some(false));
        assert_eq!(matches(&rule, "TIC-1001", &ti), Some(false));
        for blank in [props(&[]), props(&[("仪表功能", "  ")])] {
            assert_eq!(matches(&rule, "FIC-1001", &blank), Some(true));
            assert_eq!(matches(&rule, "A-B-1001", &blank), Some(true));
            assert_eq!(matches(&rule, "-1001", &blank), Some(false));
            assert_eq!(matches(&rule, "FIC1001", &blank), Some(false));
        }
    }

    /// 后缀递增时 `{后缀}` 要有数字；不递增时位号引擎把它连同紧挨的分隔符一起去掉，
    /// 带不带都算。
    #[test]
    fn suffixes_follow_what_the_tag_engine_writes() {
        let run = default_tag_rule("PIPE_RUN").expect("管路的默认规则");
        let line = props(&[("管线号", "L-1001")]);
        assert_eq!(matches(&run, "L-1001-01", &line), Some(true));
        assert_eq!(matches(&run, "L-1001", &line), Some(false));
        assert_eq!(matches(&run, "L-1002-01", &line), Some(false));

        let rule = TagRule::new("P-{序号}-{后缀}", 1001);
        let none = props(&[]);
        assert_eq!(matches(&rule, "P-1001", &none), Some(true));
        assert_eq!(matches(&rule, "P-1001-02", &none), Some(true));
        assert_eq!(matches(&rule, "P-1001-", &none), Some(false));
        assert_eq!(matches(&rule, "P--02", &none), Some(false));
    }

    /// 位号为空、模板为空时不知道。
    #[test]
    fn an_empty_tag_or_pattern_is_unknown() {
        let rule = default_tag_rule("EQUIPMENT").expect("设备的默认规则");
        assert_eq!(matches(&rule, "  ", &props(&[])), None);
        assert_eq!(matches(&TagRule::new(" ", 1), "P-1001", &props(&[])), None);
    }
}
