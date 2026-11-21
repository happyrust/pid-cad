//! 规则包（`SPEC-pid-drc.md` §3.1、§3.2）：编进程序的内置包，图纸目录里的项目包叠在
//! 它上面；生效规则的版本号与摘要值。

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::dsl;
use super::model::{
    Category, DrcError, Rule, RuleGroup, RuleScope, RuleSet, SaveGate, Severity, Source, Target,
};

/// 内置包：§3.5 的 41 条规则，`基础规则`、`工程扩展` 两个规则集。
const BUILTIN_JSON: &str = include_str!("../../../assets/pid-drc-rules.json");

/// 项目包的文件名：放在图纸所在的目录里。
pub const PROJECT_PACK_FILE: &str = "pid-drc-rules.json";

/// 程序认识的规则包格式版本。
pub const PACK_VERSION: u64 = 1;

/// 规则名称最多这么多个字。
const NAME_LIMIT: usize = 64;

/// 规则包没写 `connect_tolerance` 时的容差，图纸单位。
const DEFAULT_TOLERANCE: f64 = 1e-6;

/// 编进程序的内置包（`assets/pid-drc-rules.json`）。内容由本模块的测试保证能读。
pub fn builtin() -> RuleSet {
    parse_json(BUILTIN_JSON)
        .and_then(|value| Pack::read(&value, None))
        .map(Pack::into_rule_set)
        .expect("内置 assets/pid-drc-rules.json 必须是合法的规则包")
}

/// 生效的规则：内置包；`drawing_dir` 里有项目包 [`PROJECT_PACK_FILE`] 时叠上它（§3.2）。
/// 项目包读不出来或校验不过时报错，不退回去只用内置包：静默退回会让人以为自己的规则在生效。
pub fn load(drawing_dir: Option<&Path>) -> Result<RuleSet, DrcError> {
    let base = builtin();
    let Some(dir) = drawing_dir else {
        return Ok(base);
    };
    let path = dir.join(PROJECT_PACK_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(base),
        Err(error) => {
            return Err(DrcError::Schema {
                at: path.display().to_string(),
                reason: format!("读不出来：{error}"),
            });
        }
    };
    // 开头的 UTF-8 BOM 不算错：Windows PowerShell 5.1 存 UTF-8 时会带。
    let value = parse_json(text.strip_prefix('\u{feff}').unwrap_or(&text))?;
    with_project_pack(base, &value, path)
}

/// 校验一个项目包：结构、取值、表达式，以及 §3.2 对内置规则的限制。全部通过才是 `Ok`。
pub fn validate_project_pack(value: &Value) -> Result<(), DrcError> {
    Pack::read(value, Some(&builtin())).map(|_| ())
}

/// 生效规则的摘要值（§3.2）：SHA-256 的前 16 位十六进制。算进去的是会改变检查结果的
/// 内容：每条规则的全部字段（含启停与参数）、规则集的启停、必填字段、回路角色和连接
/// 容差；版本号、保存门禁、图例标准和项目包的位置不算。对象的键排好序再算，同样的规则
/// 不管文件里怎么写，摘要都一样。
pub fn rule_set_digest(rules: &RuleSet) -> String {
    let mut text = String::new();
    write_canonical(&describe(rules), &mut text);
    let mut digest = format!("{:x}", Sha256::digest(text.as_bytes()));
    digest.truncate(16);
    digest
}

/// 内置包 `base` 叠上从 `path` 读来的项目包 `value`。
fn with_project_pack(base: RuleSet, value: &Value, path: PathBuf) -> Result<RuleSet, DrcError> {
    let pack = Pack::read(value, Some(&base))?;
    Ok(pack.overlay(base, path))
}

fn parse_json(text: &str) -> Result<Value, DrcError> {
    serde_json::from_str(text).map_err(|error| DrcError::Schema {
        at: String::new(),
        reason: format!("不是合法的 JSON：{error}"),
    })
}

// ── 读规则包 ─────────────────────────────────────────────────────────────────

/// 一个规则包文件读出来的样子。文件层除 `version`、`rule_set_version` 外都可以不写。
struct Pack {
    rule_set_version: String,
    legend_standard: Option<String>,
    save_gate: Option<SaveGate>,
    connect_tolerance: Option<f64>,
    required_fields: Option<BTreeMap<String, Vec<String>>>,
    loop_roles: Option<BTreeMap<String, Vec<String>>>,
    /// 规则集的名字和启停；没写 `enabled` 是 `None`。
    sets: Vec<(String, Option<bool>)>,
    /// 完整的规则：内置包的全部规则，或项目包新加的规则。
    rules: Vec<Rule>,
    /// 项目包对内置规则的改动。
    changes: Vec<Change>,
}

/// 项目包里写一条内置规则的一项：只能改启停和参数。
struct Change {
    id: String,
    enabled: Option<bool>,
    params: BTreeMap<String, Value>,
}

impl Pack {
    /// 读一个规则包。`base` 是项目包要叠上去的内置包；读内置包本身时是 `None`。
    fn read(value: &Value, base: Option<&RuleSet>) -> Result<Self, DrcError> {
        let top = object(value, "")?;
        read_version(top)?;
        if let Some(pack) = field(top, "pack") {
            text(pack, "pack")?;
        }
        let rule_set_version = field(top, "rule_set_version")
            .map(|value| text(value, "rule_set_version"))
            .transpose()?
            .unwrap_or_default();
        if rule_set_version.trim().is_empty() {
            return Err(schema("rule_set_version", "不能为空"));
        }
        let legend_standard = field(top, "legend_standard")
            .map(|value| text(value, "legend_standard").map(str::to_owned))
            .transpose()?;
        let save_gate = field(top, "save_gate").map(read_save_gate).transpose()?;
        let connect_tolerance = field(top, "connect_tolerance")
            .map(read_tolerance)
            .transpose()?;
        let required_fields = field(top, "required_fields")
            .map(|value| read_names(value, "required_fields"))
            .transpose()?;
        let loop_roles = field(top, "loop_roles")
            .map(|value| read_names(value, "loop_roles"))
            .transpose()?;
        let sets = field(top, "sets")
            .map(read_sets)
            .transpose()?
            .unwrap_or_default();

        let mut known: Vec<&str> = base.map_or_else(Vec::new, |base| {
            base.sets.iter().map(|set| set.name.as_str()).collect()
        });
        known.extend(sets.iter().map(|(name, _)| name.as_str()));
        let entries = field(top, "rules")
            .map(|value| array(value, "rules"))
            .transpose()?
            .unwrap_or_default();
        let mut rules = Vec::new();
        let mut changes = Vec::new();
        let mut seen = HashSet::new();
        for (index, item) in entries.iter().enumerate() {
            let at = format!("rules[{index}]");
            let entry = object(item, &at)?;
            let id = read_id(entry, &at)?;
            if !seen.insert(id.clone()) {
                return Err(DrcError::IdDuplicated {
                    at: format!("{at}.id"),
                    rule_id: id,
                });
            }
            match base {
                Some(base) if base.rule(&id).is_some() => {
                    changes.push(Change::read(entry, &at, id)?);
                }
                Some(_) if reserved(&id) => {
                    return Err(DrcError::IdReserved {
                        at: format!("{at}.id"),
                        rule_id: id,
                    });
                }
                _ => rules.push(read_rule(entry, &at, id, &known, base.is_some())?),
            }
        }

        Ok(Self {
            rule_set_version: rule_set_version.to_owned(),
            legend_standard,
            save_gate,
            connect_tolerance,
            required_fields,
            loop_roles,
            sets,
            rules,
            changes,
        })
    }

    /// 内置包本身生效时的样子：文件层没写的取 §3.1 的默认值。
    fn into_rule_set(self) -> RuleSet {
        RuleSet {
            version: self.rule_set_version,
            legend_standard: self.legend_standard,
            save_gate: self.save_gate.unwrap_or(SaveGate::Warn),
            connect_tolerance: self.connect_tolerance.unwrap_or(DEFAULT_TOLERANCE),
            required_fields: self.required_fields.unwrap_or_default(),
            loop_roles: self.loop_roles.unwrap_or_default(),
            sets: self
                .sets
                .into_iter()
                .map(|(name, enabled)| RuleGroup {
                    name,
                    enabled: enabled.unwrap_or(true),
                })
                .collect(),
            rules: self.rules,
            project_pack: None,
        }
    }

    /// 把这个项目包叠到内置包 `base` 上（§3.2）。
    fn overlay(self, mut base: RuleSet, path: PathBuf) -> RuleSet {
        base.version = format!("{}+{}", base.version, self.rule_set_version);
        if let Some(standard) = self.legend_standard {
            base.legend_standard = Some(standard);
        }
        if let Some(gate) = self.save_gate {
            base.save_gate = gate;
        }
        if let Some(tolerance) = self.connect_tolerance {
            base.connect_tolerance = tolerance;
        }
        if let Some(roles) = self.loop_roles {
            base.loop_roles = roles;
        }
        for (type_code, fields) in self.required_fields.unwrap_or_default() {
            if fields.is_empty() {
                base.required_fields.remove(&type_code);
            } else {
                base.required_fields.insert(type_code, fields);
            }
        }
        for (name, enabled) in self.sets {
            match base.sets.iter_mut().find(|set| set.name == name) {
                Some(set) => {
                    if let Some(enabled) = enabled {
                        set.enabled = enabled;
                    }
                }
                None => base.sets.push(RuleGroup {
                    name,
                    enabled: enabled.unwrap_or(true),
                }),
            }
        }
        for change in self.changes {
            if let Some(rule) = base.rules.iter_mut().find(|rule| rule.id == change.id) {
                if let Some(enabled) = change.enabled {
                    rule.enabled = enabled;
                }
                rule.params.extend(change.params);
            }
        }
        base.rules.extend(self.rules);
        base.project_pack = Some(path);
        base
    }
}

impl Change {
    /// 项目包里写内置规则 `id` 的一项：只认 `id`、`enabled`、`params`。带了别的键时报
    /// 字典序最小的那个。
    fn read(entry: &Map<String, Value>, at: &str, id: String) -> Result<Self, DrcError> {
        if let Some(key) = entry
            .keys()
            .filter(|key| !matches!(key.as_str(), "id" | "enabled" | "params"))
            .min()
        {
            return Err(DrcError::BuiltinReadonly {
                at: format!("{at}.{key}"),
                rule_id: id,
            });
        }
        let enabled = field(entry, "enabled")
            .map(|value| flag(value, &format!("{at}.enabled")))
            .transpose()?;
        let params = field(entry, "params")
            .map(|value| read_params(value, &format!("{at}.params")))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            id,
            enabled,
            params,
        })
    }
}

/// 一条完整的规则（§3.1 的规则表，按表里的顺序查）。`known` 是能用的规则集名。
/// `project` 为真时是项目包新加的规则，来源只能是复制修改或新建；否则是内置包的规则，
/// 来源只能是内置。
fn read_rule(
    entry: &Map<String, Value>,
    at: &str,
    id: String,
    known: &[&str],
    project: bool,
) -> Result<Rule, DrcError> {
    let name = required(entry, at, "name", &id)?;
    if name.chars().count() > NAME_LIMIT {
        return Err(schema(
            &format!("{at}.name"),
            format!("超过 {NAME_LIMIT} 个字"),
        ));
    }
    let set = required(entry, at, "set", &id)?;
    if !known.contains(&set) {
        return Err(schema(
            &format!("{at}.set"),
            format!("规则集 {set} 没有写在 sets 里"),
        ));
    }
    let found = required(entry, at, "category", &id)?;
    let category = Category::from_name(found).ok_or_else(|| DrcError::KindInvalid {
        at: format!("{at}.category"),
        rule_id: id.clone(),
        found: found.to_owned(),
    })?;
    let target = Target::parse(required(entry, at, "target", &id)?);
    let found = required(entry, at, "severity", &id)?;
    let severity = Severity::from_name(found).ok_or_else(|| DrcError::SeverityInvalid {
        at: format!("{at}.severity"),
        rule_id: id.clone(),
        found: found.to_owned(),
    })?;
    let expr = required(entry, at, "expr", &id)?;
    let tree = dsl::parse(expr, &target).map_err(|error| DrcError::Dsl {
        at: format!("{at}.expr"),
        rule_id: id.clone(),
        error,
    })?;
    let message = optional(entry, at, "message")?;
    let fix = optional(entry, at, "fix")?;
    let scope = read_scope(entry, at, &id)?;
    let enabled = field(entry, "enabled")
        .map(|value| flag(value, &format!("{at}.enabled")))
        .transpose()?
        .unwrap_or(true);
    let source_at = format!("{at}.source");
    let source = match Source::from_name(required(entry, at, "source", &id)?) {
        Some(Source::Builtin) if project => {
            return Err(schema(
                &source_at,
                "项目包新加的规则来源只能是复制修改或新建",
            ));
        }
        Some(source) if project || source == Source::Builtin => source,
        Some(_) => return Err(schema(&source_at, "内置包的规则来源只能是内置")),
        None => return Err(schema(&source_at, "应为内置、复制修改或新建")),
    };
    let params = field(entry, "params")
        .map(|value| read_params(value, &format!("{at}.params")))
        .transpose()?
        .unwrap_or_default();
    Ok(Rule {
        name: name.to_owned(),
        set: set.to_owned(),
        category,
        target,
        severity,
        expr: expr.to_owned(),
        tree,
        message: message.map(str::to_owned),
        fix: fix.map(str::to_owned),
        scope,
        enabled,
        source,
        params,
        id,
    })
}

/// 规则的 `id`：两个大写字母、连字符、三位数字。
fn read_id(entry: &Map<String, Value>, at: &str) -> Result<String, DrcError> {
    let at = format!("{at}.id");
    let id = field(entry, "id")
        .map(|value| text(value, &at))
        .transpose()?
        .unwrap_or_default();
    if id.trim().is_empty() {
        return Err(DrcError::RequiredFieldEmpty { at, rule_id: None });
    }
    let bytes = id.as_bytes();
    let well_formed = bytes.len() == 6
        && bytes[..2].iter().all(u8::is_ascii_uppercase)
        && bytes[2] == b'-'
        && bytes[3..].iter().all(u8::is_ascii_digit);
    if !well_formed {
        return Err(schema(
            &at,
            format!("{id} 应为两个大写字母、连字符、三位数字，如 DR-201"),
        ));
    }
    Ok(id.to_owned())
}

/// 编号落在留给内置规则的 `DR-001` – `DR-199` 里。
fn reserved(id: &str) -> bool {
    id.strip_prefix("DR-")
        .and_then(|number| number.parse::<u32>().ok())
        .is_some_and(|number| (1..=199).contains(&number))
}

/// `scope` 与 `scope_ref`：作用域不是项目时必须写 `scope_ref`。
fn read_scope(entry: &Map<String, Value>, at: &str, id: &str) -> Result<RuleScope, DrcError> {
    let name = required(entry, at, "scope", id)?;
    let reference = || -> Result<String, DrcError> {
        optional(entry, at, "scope_ref")?
            .map(str::to_owned)
            .ok_or_else(|| DrcError::RequiredFieldEmpty {
                at: format!("{at}.scope_ref"),
                rule_id: Some(id.to_owned()),
            })
    };
    match name {
        "项目" => Ok(RuleScope::Project),
        "PBS 节点" => Ok(RuleScope::Pbs(reference()?)),
        "图纸" => Ok(RuleScope::Drawing(reference()?)),
        "类型" => Ok(RuleScope::Type(reference()?)),
        _ => Err(schema(
            &format!("{at}.scope"),
            "应为项目、PBS 节点、图纸或类型",
        )),
    }
}

/// 规则的必填文本字段：没写、写成 `null` 或空串都报 `REQUIRED_FIELD_EMPTY`。
fn required<'a>(
    entry: &'a Map<String, Value>,
    at: &str,
    key: &str,
    id: &str,
) -> Result<&'a str, DrcError> {
    let at = format!("{at}.{key}");
    let value = field(entry, key)
        .map(|value| text(value, &at))
        .transpose()?
        .unwrap_or_default();
    if value.trim().is_empty() {
        Err(DrcError::RequiredFieldEmpty {
            at,
            rule_id: Some(id.to_owned()),
        })
    } else {
        Ok(value)
    }
}

/// 规则的可选文本字段（`message`、`fix`、`scope_ref`）：没写、`null` 和空串都是 `None`。
fn optional<'a>(
    entry: &'a Map<String, Value>,
    at: &str,
    key: &str,
) -> Result<Option<&'a str>, DrcError> {
    let value = field(entry, key)
        .map(|value| text(value, &format!("{at}.{key}")))
        .transpose()?;
    Ok(value.filter(|value| !value.trim().is_empty()))
}

fn read_params(value: &Value, at: &str) -> Result<BTreeMap<String, Value>, DrcError> {
    Ok(object(value, at)?
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect())
}

/// `version`：必须写，是程序认识的格式版本。
fn read_version(top: &Map<String, Value>) -> Result<(), DrcError> {
    match field(top, "version").map(Value::as_u64) {
        Some(Some(PACK_VERSION)) => Ok(()),
        Some(Some(found)) if found > PACK_VERSION => Err(DrcError::VersionUnsupported {
            at: "version".to_owned(),
            found,
        }),
        _ => Err(schema("version", format!("应为 {PACK_VERSION}"))),
    }
}

fn read_save_gate(value: &Value) -> Result<SaveGate, DrcError> {
    SaveGate::from_name(text(value, "save_gate")?)
        .ok_or_else(|| schema("save_gate", "应为 off、warn 或 block"))
}

fn read_tolerance(value: &Value) -> Result<f64, DrcError> {
    match value.as_f64() {
        Some(tolerance) if tolerance.is_finite() && tolerance >= 0.0 => Ok(tolerance),
        _ => Err(schema("connect_tolerance", "应为不小于 0 的数")),
    }
}

/// 名字 → 名字列表（`required_fields`、`loop_roles`）。
fn read_names(value: &Value, at: &str) -> Result<BTreeMap<String, Vec<String>>, DrcError> {
    object(value, at)?
        .iter()
        .map(|(key, list)| {
            let at = format!("{at}.{key}");
            let names = array(list, &at)?
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let at = format!("{at}[{index}]");
                    let name = text(item, &at)?;
                    if name.trim().is_empty() {
                        Err(schema(&at, "不能为空"))
                    } else {
                        Ok(name.to_owned())
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((key.clone(), names))
        })
        .collect()
}

/// `sets[]`：规则集的名字和启停，名字不能为空、不能重复。
fn read_sets(value: &Value) -> Result<Vec<(String, Option<bool>)>, DrcError> {
    let mut sets: Vec<(String, Option<bool>)> = Vec::new();
    for (index, item) in array(value, "sets")?.iter().enumerate() {
        let at = format!("sets[{index}]");
        let entry = object(item, &at)?;
        let name_at = format!("{at}.name");
        let name = field(entry, "name")
            .map(|value| text(value, &name_at))
            .transpose()?
            .unwrap_or_default();
        if name.trim().is_empty() {
            return Err(schema(&name_at, "不能为空"));
        }
        if sets.iter().any(|(seen, _)| seen == name) {
            return Err(schema(&name_at, format!("规则集 {name} 与前面的重复")));
        }
        let enabled = field(entry, "enabled")
            .map(|value| flag(value, &format!("{at}.enabled")))
            .transpose()?;
        sets.push((name.to_owned(), enabled));
    }
    Ok(sets)
}

fn schema(at: &str, reason: impl Into<String>) -> DrcError {
    DrcError::Schema {
        at: at.to_owned(),
        reason: reason.into(),
    }
}

/// 可以不写的键：写成 `null` 与没写一样。
fn field<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    object.get(key).filter(|value| !value.is_null())
}

fn object<'a>(value: &'a Value, at: &str) -> Result<&'a Map<String, Value>, DrcError> {
    value.as_object().ok_or_else(|| schema(at, "应为对象"))
}

fn array<'a>(value: &'a Value, at: &str) -> Result<&'a [Value], DrcError> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| schema(at, "应为数组"))
}

fn text<'a>(value: &'a Value, at: &str) -> Result<&'a str, DrcError> {
    value.as_str().ok_or_else(|| schema(at, "应为字符串"))
}

fn flag(value: &Value, at: &str) -> Result<bool, DrcError> {
    value
        .as_bool()
        .ok_or_else(|| schema(at, "应为 true 或 false"))
}

// ── 摘要值 ─────────────────────────────────────────────────────────────────────

/// 规则集和规则，照规则包文件的写法（§3.1）：`pid_check_rules` 列的就是这两样，摘要值也算它们。
pub fn listing(rules: &RuleSet) -> (Vec<Value>, Vec<Value>) {
    let sets: Vec<Value> = rules
        .sets
        .iter()
        .map(|set| serde_json::json!({ "name": set.name, "enabled": set.enabled }))
        .collect();
    let list: Vec<Value> = rules
        .rules
        .iter()
        .map(|rule| {
            serde_json::json!({
                "id": rule.id,
                "name": rule.name,
                "set": rule.set,
                "category": rule.category.name(),
                "target": rule.target.name(),
                "severity": rule.severity.name(),
                "expr": rule.expr,
                "message": rule.message,
                "fix": rule.fix,
                "scope": rule.scope.name(),
                "scope_ref": rule.scope.reference(),
                "enabled": rule.enabled,
                "source": rule.source.name(),
                "params": rule.params,
            })
        })
        .collect();
    (sets, list)
}

/// 摘要值算的内容，见 [`rule_set_digest`]。
fn describe(rules: &RuleSet) -> Value {
    let (sets, list) = listing(rules);
    serde_json::json!({
        "connect_tolerance": rules.connect_tolerance,
        "required_fields": rules.required_fields,
        "loop_roles": rules.loop_roles,
        "sets": sets,
        "rules": list,
    })
}

/// 键排好序的紧凑 JSON：不管 `serde_json` 开没开 `preserve_order`，同样的值写出同样的文本。
fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_unstable_by_key(|(key, _)| *key);
            out.push('{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::from(key.as_str()).to_string());
                out.push(':');
                write_canonical(item, out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;
    use crate::pid::drc::model::Kind;

    /// §3.5 的两张表：编号、名称、作用对象、严重度、表达式。
    const SECTION_3_5: [(&str, &str, &str, &str, &str); 41] = [
        (
            "DR-001",
            "管线缺管道等级",
            "管线",
            "错误",
            "管道等级 = NULL",
        ),
        (
            "DR-002",
            "管线缺公称直径",
            "管线",
            "错误",
            "公称直径 = NULL",
        ),
        (
            "DR-003",
            "设备无位号",
            "设备",
            "错误",
            r#"位号 = NULL OR 位号 = """#,
        ),
        (
            "DR-004",
            "位号重复",
            "对象实例",
            "错误",
            "位号.同类型重复数 > 1",
        ),
        (
            "DR-005",
            "仪表未分配回路",
            "仪表",
            "警告",
            "仪表回路 = NULL",
        ),
        (
            "DR-006",
            "页间连接符对端缺失",
            "页间连接符",
            "错误",
            "对端图纸 = NULL OR 对端位号 = NULL",
        ),
        (
            "DR-007",
            "阀门压力等级高于管道等级",
            "阀门",
            "错误",
            "压力等级 > 所属管线.管道等级.最大公称压力",
        ),
        ("DR-008", "管口未连接", "管口", "提示", "连接状态 = #未连接"),
        (
            "DR-009",
            "图纸未设图例标准",
            "图纸",
            "错误",
            "图例标准 = NULL",
        ),
        (
            "DR-010",
            "属性必填项为空",
            "对象实例",
            "错误",
            "必填项缺失数 > 0",
        ),
        (
            "DR-011",
            "管路位号后缀缺失",
            "管路",
            "错误",
            r#"后缀 = NULL OR 后缀 = """#,
        ),
        ("DR-012", "管段长度非法", "管段", "警告", "长度 <= 0"),
        (
            "DR-013",
            "仪表回路仪表数不足",
            "仪表回路",
            "提示",
            "仪表数 < 2",
        ),
        (
            "DR-014",
            "管线端口未闭合",
            "管线",
            "错误",
            "端口未连接数 > 0",
        ),
        (
            "DR-015",
            "信号线端点缺失",
            "信号线",
            "错误",
            "起点仪表 = NULL OR 终点仪表 = NULL",
        ),
        (
            "DR-016",
            "标注文字为空",
            "标注",
            "提示",
            r#"文字 = NULL OR 文字 = """#,
        ),
        (
            "DR-017",
            "阀门开度越界",
            "阀门",
            "警告",
            "开度 > 100 OR 开度 < 0",
        ),
        (
            "DR-018",
            "位号格式不符",
            "对象实例",
            "警告",
            "位号合规 = FALSE",
        ),
        (
            "DR-019",
            "位号长度超限",
            "对象实例",
            "提示",
            "位号.长度 > 32",
        ),
        (
            "DR-020",
            "管口位号前缀不继承设备",
            "管口",
            "错误",
            "位号前缀 != 所属设备.位号",
        ),
        (
            "DR-021",
            "介质与管道等级材质不兼容",
            "管线",
            "错误",
            "流体介质 IN (#氯气, #氢氟酸) AND 管道等级.材质 = #碳钢",
        ),
        (
            "DR-022",
            "管道等级不在项目主数据",
            "管线",
            "错误",
            "管道等级 NOT IN (项目.管道等级表)",
        ),
        (
            "DR-023",
            "设备管口数不足",
            "设备",
            "警告",
            "管口数 < 2 AND NOT (设备类别 = #储罐)",
        ),
        (
            "DR-024",
            "符号实例状态非法",
            "符号实例",
            "提示",
            "符号.状态 != #有效",
        ),
        (
            "DR-025",
            "高温管线缺绝热等级",
            "管线",
            "警告",
            "(设计温度 > 200 OR 流体介质 = #蒸汽) AND 绝热等级 = NULL",
        ),
        (
            "DR-101",
            "仪表或阀门无位号",
            "对象实例",
            "错误",
            r#"种类 IN (#仪表, #阀门) AND (位号 = NULL OR 位号 = "")"#,
        ),
        (
            "DR-102",
            "跨图纸位号重复",
            "对象实例",
            "错误",
            "位号.跨图重复数 > 0",
        ),
        (
            "DR-103",
            "公称直径不在项目主数据",
            "对象实例",
            "错误",
            "种类 IN (#管线, #管路) AND 公称直径 NOT IN (项目.公称直径表)",
        ),
        (
            "DR-104",
            "流体介质不在项目主数据",
            "管线",
            "错误",
            "流体介质 NOT IN (项目.流体介质表)",
        ),
        (
            "DR-105",
            "管径突变处没有异径管",
            "管路",
            "警告",
            "直连异径管路数 > 0",
        ),
        (
            "DR-106",
            "阀门公称直径与所在管路不一致",
            "阀门",
            "警告",
            "公称直径 != 所属管路.公称直径",
        ),
        (
            "DR-107",
            "阀门或管件的管道等级与所在管线不一致",
            "对象实例",
            "警告",
            "种类 IN (#阀门, #管件) AND 管道等级 != 所属管线.管道等级",
        ),
        (
            "DR-108",
            "信号线接到了设备管口",
            "信号线",
            "警告",
            "接到设备管口数 > 0",
        ),
        (
            "DR-109",
            "单向阀或调节阀流向与管路相反",
            "阀门",
            "错误",
            "流向相反 = TRUE",
        ),
        ("DR-110", "管段重叠", "管段", "警告", "重叠管段数 > 0"),
        (
            "DR-111",
            "阀门或管件有端口没接上",
            "对象实例",
            "警告",
            "种类 IN (#阀门, #管件) AND 未连接端口数 > 0",
        ),
        (
            "DR-112",
            "配对连接符两侧管道等级或公称直径不一致",
            "页间连接符",
            "警告",
            "对端线.管道等级 != 所在线.管道等级 OR 对端线.公称直径 != 所在线.公称直径",
        ),
        (
            "DR-113",
            "仪表回路缺控制器或测量元件",
            "仪表回路",
            "警告",
            "控制器数 = 0 OR 测量元件数 = 0",
        ),
        (
            "DR-114",
            "对象不在受控图层",
            "对象实例",
            "提示",
            "图层 NOT IN (参数.允许图层)",
        ),
        (
            "DR-115",
            "管路属性与管线不同却没有覆盖标记",
            "管路",
            "警告",
            "未标记覆盖数 > 0",
        ),
        (
            "DR-116",
            "连接符的对端在项目里找不到",
            "页间连接符",
            "错误",
            "配对状态 IN (#one_sided, #target_missing)",
        ),
    ];

    /// 测试用的临时目录，测完删掉。
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            // 测试并行跑：计数器把同一纳秒里建的两个目录分开。
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "ocs-drc-rules-{}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_nanos())
            ));
            fs::create_dir(&path).expect("scratch directory");
            Self(path)
        }

        /// 这个目录里的项目包。
        fn pack(&self) -> PathBuf {
            self.0.join(PROJECT_PACK_FILE)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn group(name: &str, enabled: bool) -> RuleGroup {
        RuleGroup {
            name: name.to_owned(),
            enabled,
        }
    }

    /// 文件层只写了必有的键的项目包，再加上 `extra` 里的键。
    fn project(extra: Value) -> Value {
        let mut pack = json!({ "version": 1, "pack": "某项目的规则", "rule_set_version": "P1" });
        if let (Some(pack), Value::Object(extra)) = (pack.as_object_mut(), extra) {
            pack.extend(extra);
        }
        pack
    }

    /// 一条新规则 `XY-001`，放在基础规则集里；`changes` 里的键取代这些默认值。
    fn new_rule(changes: Value) -> Value {
        let mut rule = json!({
            "id": "XY-001",
            "name": "泵没有流量",
            "set": "基础规则",
            "category": "设计类",
            "target": "设备",
            "severity": "警告",
            "expr": "流量 = NULL",
            "scope": "项目",
            "source": "新建"
        });
        if let (Some(rule), Value::Object(changes)) = (rule.as_object_mut(), changes) {
            rule.extend(changes);
        }
        rule
    }

    /// 内置包叠上项目包。
    fn overlaid(pack: &Value) -> Result<RuleSet, DrcError> {
        with_project_pack(builtin(), pack, PathBuf::from(PROJECT_PACK_FILE))
    }

    /// 项目包校验不过的原因。
    fn failure(pack: &Value) -> DrcError {
        match validate_project_pack(pack) {
            Ok(()) => panic!("{pack} 应当校验不过"),
            Err(error) => error,
        }
    }

    /// 内置包就是 §3.5 的 41 条：都能解析（读包时就解析了），基础规则恰好 25 条、编号
    /// 连续，工程扩展 16 条；`DR-114` 默认停用；文件层的值照 §3.1 和 §9 的 D6。
    #[test]
    fn the_builtin_pack_is_the_41_rules_of_section_3_5() {
        let rules = builtin();
        let listed: Vec<(&str, &str, &str, &str, &str)> = rules
            .rules
            .iter()
            .map(|rule| {
                (
                    rule.id.as_str(),
                    rule.name.as_str(),
                    rule.target.name(),
                    rule.severity.name(),
                    rule.expr.as_str(),
                )
            })
            .collect();
        assert_eq!(listed, SECTION_3_5);

        let ids = |set: &str| -> Vec<&str> {
            rules
                .rules
                .iter()
                .filter(|rule| rule.set == set)
                .map(|rule| rule.id.as_str())
                .collect()
        };
        let basic: Vec<String> = (1..=25).map(|n| format!("DR-{n:03}")).collect();
        let extension: Vec<String> = (101..=116).map(|n| format!("DR-{n:03}")).collect();
        assert_eq!(ids("基础规则"), basic);
        assert_eq!(ids("工程扩展"), extension);

        for rule in &rules.rules {
            let id = rule.id.as_str();
            assert_eq!(rule.category, Category::Design, "{id}");
            assert_eq!(rule.scope, RuleScope::Project, "{id}");
            assert_eq!(rule.source, Source::Builtin, "{id}");
            assert_eq!(rule.enabled, id != "DR-114", "{id}");
            assert_eq!(rules.is_enabled(rule), id != "DR-114", "{id}");
            assert!(rule.message.is_some() && rule.fix.is_some(), "{id}");
            let params = if id == "DR-114" {
                BTreeMap::from([("允许图层".to_owned(), json!([]))])
            } else {
                BTreeMap::new()
            };
            assert_eq!(rule.params, params, "{id}");
        }

        assert_eq!(rules.version, "1.0.0");
        assert_eq!(rules.legend_standard, None);
        assert_eq!(rules.save_gate, SaveGate::Warn);
        assert_eq!(rules.connect_tolerance, 1e-6);
        assert_eq!(
            rules.required_fields,
            BTreeMap::from([
                ("EQUIPMENT".to_owned(), strings(&["名称"])),
                ("PIPE_LINE".to_owned(), strings(&["流体介质"])),
            ])
        );
        assert_eq!(
            rules.loop_roles,
            BTreeMap::from([
                ("控制器".to_owned(), strings(&["controller"])),
                (
                    "测量元件".to_owned(),
                    strings(&["transmitter", "indicator"])
                ),
            ])
        );
        assert_eq!(
            rules.sets,
            [group("基础规则", true), group("工程扩展", true)]
        );
        assert_eq!(rules.project_pack, None);
    }

    /// §3.2 第一行：`save_gate`、`connect_tolerance`、`loop_roles` 取代内置的值；没写的照旧。
    #[test]
    fn a_project_pack_replaces_the_save_gate_the_tolerance_and_the_loop_roles() {
        let rules = overlaid(&project(json!({
            "save_gate": "block",
            "connect_tolerance": 0.01,
            "loop_roles": { "控制器": ["pid_controller"] }
        })))
        .expect("合法的项目包");
        assert_eq!(rules.save_gate, SaveGate::Block);
        assert_eq!(rules.connect_tolerance, 0.01);
        assert_eq!(
            rules.loop_roles,
            BTreeMap::from([("控制器".to_owned(), strings(&["pid_controller"]))])
        );

        let base = builtin();
        let plain = overlaid(&project(json!({}))).expect("合法的项目包");
        assert_eq!(plain.save_gate, base.save_gate);
        assert_eq!(plain.connect_tolerance, base.connect_tolerance);
        assert_eq!(plain.loop_roles, base.loop_roles);
        assert_eq!(plain.required_fields, base.required_fields);
        assert_eq!(plain.sets, base.sets);
        assert_eq!(plain.rules, base.rules);
    }

    /// §3.2 第二行：`required_fields` 按类型编码取代内置的列表，空列表等于取消；没写的类型
    /// 照旧。
    #[test]
    fn required_fields_are_replaced_type_by_type_and_an_empty_list_cancels() {
        let rules = overlaid(&project(json!({
            "required_fields": {
                "EQUIPMENT": ["名称", "设计压力"],
                "PIPE_LINE": [],
                "PUMP": ["流量"]
            }
        })))
        .expect("合法的项目包");
        assert_eq!(
            rules.required_fields,
            BTreeMap::from([
                ("EQUIPMENT".to_owned(), strings(&["名称", "设计压力"])),
                ("PUMP".to_owned(), strings(&["流量"])),
            ])
        );

        let rules = overlaid(&project(json!({ "required_fields": { "PUMP": ["流量"] } })))
            .expect("合法的项目包");
        assert_eq!(
            rules.required_fields,
            BTreeMap::from([
                ("EQUIPMENT".to_owned(), strings(&["名称"])),
                ("PIPE_LINE".to_owned(), strings(&["流体介质"])),
                ("PUMP".to_owned(), strings(&["流量"])),
            ])
        );
    }

    /// §3.2 第三行：与内置同名的规则集只取 `enabled`，整组启停；规则本身不变。
    #[test]
    fn a_set_named_like_a_builtin_one_only_switches_it() {
        let rules = overlaid(&project(json!({
            "sets": [{ "name": "工程扩展", "enabled": false }]
        })))
        .expect("合法的项目包");
        assert_eq!(
            rules.sets,
            [group("基础规则", true), group("工程扩展", false)]
        );
        assert_eq!(rules.rules, builtin().rules);
        for rule in &rules.rules {
            assert_eq!(
                rules.is_enabled(rule),
                rule.set == "基础规则",
                "{}",
                rule.id
            );
        }

        // 没写 `enabled` 的同名规则集不改启停。
        let rules = overlaid(&project(json!({
            "sets": [{ "name": "工程扩展", "enabled": true }, { "name": "基础规则" }]
        })))
        .expect("合法的项目包");
        assert_eq!(rules.sets, builtin().sets);
    }

    /// §3.2 第四行：新名字的规则集接在后面，没写 `enabled` 算启用；新规则可以放进去。
    /// 规则的 `set` 哪个包都没声明过是结构错误。
    #[test]
    fn a_set_with_a_new_name_is_added() {
        let rules = overlaid(&project(json!({
            "sets": [{ "name": "本项目", "enabled": true }, { "name": "试用" }],
            "rules": [
                new_rule(json!({ "set": "本项目" })),
                new_rule(json!({ "id": "XY-002", "set": "试用" }))
            ]
        })))
        .expect("合法的项目包");
        assert_eq!(
            rules.sets,
            [
                group("基础规则", true),
                group("工程扩展", true),
                group("本项目", true),
                group("试用", true),
            ]
        );
        let added: Vec<(&str, &str)> = rules
            .rules
            .iter()
            .skip(41)
            .map(|rule| (rule.id.as_str(), rule.set.as_str()))
            .collect();
        assert_eq!(added, [("XY-001", "本项目"), ("XY-002", "试用")]);

        let error = failure(&project(json!({
            "rules": [new_rule(json!({ "set": "没有这个集" }))]
        })));
        assert_eq!(
            (error.code(), error.at()),
            ("RULESET_SCHEMA_ERROR", "rules[0].set")
        );
    }

    /// §3.2 第五行：与内置同编号的条目只能带 `id`、`enabled`、`params`：启停这条内置规则、
    /// 改它的参数（按参数名逐个取代）。带了别的键报 `RULE_BUILTIN_READONLY`，条件和严重度
    /// 改不了。
    #[test]
    fn a_builtin_rule_can_only_be_switched_and_given_params() {
        let rules = overlaid(&project(json!({
            "rules": [
                { "id": "DR-114", "enabled": true, "params": { "允许图层": ["PID-设备", "PID-管线"] } },
                { "id": "DR-014", "enabled": false },
                { "id": "DR-001", "params": { "备注": "只加参数" } }
            ]
        })))
        .expect("合法的项目包");
        let dr114 = rules.rule("DR-114").expect("DR-114");
        assert!(dr114.enabled);
        assert_eq!(
            dr114.params,
            BTreeMap::from([("允许图层".to_owned(), json!(["PID-设备", "PID-管线"]))])
        );
        assert!(!rules.rule("DR-014").expect("DR-014").enabled);
        let dr001 = rules.rule("DR-001").expect("DR-001");
        assert!(dr001.enabled);
        assert_eq!(
            dr001.params,
            BTreeMap::from([("备注".to_owned(), json!("只加参数"))])
        );
        let base = builtin();
        assert_eq!(rules.rules.len(), base.rules.len());
        for (rule, builtin) in rules.rules.iter().zip(&base.rules) {
            assert_eq!(
                (&rule.id, &rule.expr, rule.severity, &rule.target),
                (
                    &builtin.id,
                    &builtin.expr,
                    builtin.severity,
                    &builtin.target
                )
            );
        }

        // 参数按名字逐个取代：没提到的内置参数还在。
        let rules = overlaid(&project(json!({
            "rules": [{ "id": "DR-114", "params": { "备注": 1 } }]
        })))
        .expect("合法的项目包");
        assert_eq!(
            rules.rule("DR-114").expect("DR-114").params,
            BTreeMap::from([
                ("允许图层".to_owned(), json!([])),
                ("备注".to_owned(), json!(1)),
            ])
        );

        for (key, value) in [
            ("expr", json!("管道等级 = NULL")),
            ("severity", json!("提示")),
            ("name", json!("改个名字")),
            ("target", json!("管路")),
            ("message", json!("另一种说法")),
            ("source", json!("复制修改")),
        ] {
            let mut entry = json!({ "id": "DR-001", "enabled": true });
            entry[key] = value;
            let error = failure(&project(json!({ "rules": [entry] })));
            let at = format!("rules[0].{key}");
            assert_eq!(
                (error.code(), error.at(), error.rule_id()),
                ("RULE_BUILTIN_READONLY", at.as_str(), Some("DR-001"))
            );
        }
    }

    /// §3.2 第六行：新编号的规则接在内置规则后面，来源是复制修改或新建；编号落在内置号段
    /// 报 `RULE_ID_RESERVED`，文件里重复报 `RULE_ID_DUPLICATED`。内置规则一条不少、一条不变。
    #[test]
    fn a_rule_with_a_new_id_is_added_after_the_builtin_ones() {
        let rules = overlaid(&project(json!({
            "rules": [
                new_rule(json!({})),
                new_rule(json!({
                    "id": "DR-201",
                    "source": "复制修改",
                    "severity": "错误",
                    "enabled": false,
                    "message": "{位号} 没有流量",
                    "scope": "图纸",
                    "scope_ref": "PID-1101",
                    "params": { "上限": 5 }
                }))
            ]
        })))
        .expect("合法的项目包");
        assert_eq!(rules.rules.len(), 43);
        assert_eq!(rules.rules[..41], builtin().rules[..]);

        let first = &rules.rules[41];
        assert_eq!(first.id, "XY-001");
        assert_eq!(first.target, Target::Kind(Kind::Equipment));
        assert_eq!(first.source, Source::New);
        assert!(first.enabled);
        assert_eq!(first.scope, RuleScope::Project);
        assert_eq!(first.message, None);
        assert!(first.params.is_empty());

        let second = &rules.rules[42];
        assert_eq!(
            (
                second.id.as_str(),
                second.source,
                second.severity,
                second.enabled
            ),
            ("DR-201", Source::Modified, Severity::Error, false)
        );
        assert_eq!(second.scope, RuleScope::Drawing("PID-1101".to_owned()));
        assert_eq!(second.message.as_deref(), Some("{位号} 没有流量"));
        assert_eq!(
            second.params,
            BTreeMap::from([("上限".to_owned(), json!(5))])
        );

        let error = failure(&project(json!({
            "rules": [new_rule(json!({ "id": "DR-150" }))]
        })));
        assert_eq!(
            (error.code(), error.at(), error.rule_id()),
            ("RULE_ID_RESERVED", "rules[0].id", Some("DR-150"))
        );

        let error = failure(&project(json!({
            "rules": [new_rule(json!({})), new_rule(json!({ "name": "另一条" }))]
        })));
        assert_eq!(
            (error.code(), error.at(), error.rule_id()),
            ("RULE_ID_DUPLICATED", "rules[1].id", Some("XY-001"))
        );
        let error = failure(&project(json!({
            "rules": [{ "id": "DR-014", "enabled": false }, { "id": "DR-014" }]
        })));
        assert_eq!(
            (error.code(), error.at()),
            ("RULE_ID_DUPLICATED", "rules[1].id")
        );

        let error = failure(&project(json!({
            "rules": [new_rule(json!({ "source": "内置" }))]
        })));
        assert_eq!(
            (error.code(), error.at()),
            ("RULESET_SCHEMA_ERROR", "rules[0].source")
        );
    }

    /// `load`：没有图纸目录、目录里没有项目包时就是内置包；有项目包就叠上去，记下文件。
    /// 开头带 UTF-8 BOM（Windows PowerShell 5.1 存 UTF-8 时会带）的照样读。
    #[test]
    fn load_lays_the_project_pack_next_to_the_drawing_over_the_builtin_one() {
        let base = builtin();
        assert_eq!(load(None), Ok(base.clone()));
        let scratch = Scratch::new();
        assert_eq!(load(Some(&scratch.0)), Ok(base));

        let text = project(json!({
            "save_gate": "off",
            "rules": [{ "id": "DR-014", "enabled": false }]
        }))
        .to_string();
        fs::write(scratch.pack(), &text).expect("写项目包");
        let rules = load(Some(&scratch.0)).expect("合法的项目包");
        assert_eq!(rules.version, "1.0.0+P1");
        assert_eq!(rules.save_gate, SaveGate::Off);
        assert!(!rules.rule("DR-014").expect("DR-014").enabled);
        assert_eq!(rules.project_pack, Some(scratch.pack()));

        fs::write(scratch.pack(), format!("\u{feff}{text}")).expect("写项目包");
        assert_eq!(load(Some(&scratch.0)), Ok(rules));
    }

    /// 项目包读不出来或校验不过时 `load` 报错，不退回去只用内置包（§3.2）。
    #[test]
    fn load_reports_a_bad_project_pack_instead_of_falling_back() {
        let scratch = Scratch::new();
        fs::write(scratch.pack(), "{ \"version\": 1, ").expect("写项目包");
        let error = load(Some(&scratch.0)).expect_err("不是 JSON");
        assert_eq!((error.code(), error.at()), ("RULESET_SCHEMA_ERROR", ""));

        let text = project(json!({
            "rules": [{ "id": "DR-001", "expr": "管道等级 = NULL" }]
        }))
        .to_string();
        fs::write(scratch.pack(), text).expect("写项目包");
        assert_eq!(
            load(Some(&scratch.0)).map_err(|error| error.code()),
            Err("RULE_BUILTIN_READONLY")
        );

        // 同名的是个目录：读不出来。
        let other = Scratch::new();
        fs::create_dir(other.pack()).expect("同名目录");
        let error = load(Some(&other.0)).expect_err("读不出来");
        assert_eq!(error.code(), "RULESET_SCHEMA_ERROR");
        assert_eq!(error.at(), other.pack().display().to_string());
    }

    /// 版本号：没有项目包时是内置包的，有项目包时是 `<内置版本>+<项目包版本>`。摘要值是
    /// 生效规则的 SHA-256 前 16 位十六进制：同样的规则同样的摘要；启停、参数、规则集、必填
    /// 字段、回路角色、容差、新规则一变就变；只改保存门禁不变（不影响检查结果）。
    #[test]
    fn the_version_and_the_digest_follow_the_effective_rules() {
        let base = builtin();
        let digest = rule_set_digest(&base);
        assert_eq!(digest.len(), 16);
        assert!(
            digest
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{digest}"
        );
        assert_eq!(rule_set_digest(&builtin()), digest);

        let same = overlaid(&project(json!({}))).expect("合法的项目包");
        assert_eq!(base.version, "1.0.0");
        assert_eq!(same.version, "1.0.0+P1");
        assert_eq!(rule_set_digest(&same), digest);
        let gate = overlaid(&project(json!({ "save_gate": "block" }))).expect("合法的项目包");
        assert_eq!(rule_set_digest(&gate), digest);

        for change in [
            json!({ "rules": [{ "id": "DR-014", "enabled": false }] }),
            json!({ "rules": [{ "id": "DR-114", "params": { "允许图层": ["PID"] } }] }),
            json!({ "sets": [{ "name": "工程扩展", "enabled": false }] }),
            json!({ "required_fields": { "PUMP": ["流量"] } }),
            json!({ "loop_roles": {} }),
            json!({ "connect_tolerance": 0.01 }),
            json!({ "rules": [new_rule(json!({}))] }),
        ] {
            let rules = overlaid(&project(change.clone())).expect("合法的项目包");
            assert_ne!(rule_set_digest(&rules), digest, "{change}");
        }

        // 参数里对象的键写成什么顺序，摘要都一样。
        let digest_with = |params: &str| -> String {
            let params: Value = serde_json::from_str(params).expect("JSON");
            let rules = overlaid(&project(json!({
                "rules": [{ "id": "DR-114", "params": params }]
            })))
            .expect("合法的项目包");
            rule_set_digest(&rules)
        };
        assert_eq!(
            digest_with(r#"{"允许图层": [], "范围": {"甲": 1, "乙": 2}}"#),
            digest_with(r#"{"范围": {"乙": 2, "甲": 1}, "允许图层": []}"#)
        );
    }

    /// §3.8 里规则包类的错误码各有反例：错误指出位置，知道是哪条规则时也指出规则。
    #[test]
    fn every_pack_error_code_has_a_counterexample() {
        let with_rules = |rules: Value| project(json!({ "rules": rules }));
        let cases = [
            (json!([]), "RULESET_SCHEMA_ERROR", "", None),
            (
                project(json!({ "rules": {} })),
                "RULESET_SCHEMA_ERROR",
                "rules",
                None,
            ),
            (
                project(json!({ "save_gate": "sometimes" })),
                "RULESET_SCHEMA_ERROR",
                "save_gate",
                None,
            ),
            (
                with_rules(json!([new_rule(json!({ "id": "D-1" }))])),
                "RULESET_SCHEMA_ERROR",
                "rules[0].id",
                None,
            ),
            (
                with_rules(json!([new_rule(json!({ "name": "长".repeat(65) }))])),
                "RULESET_SCHEMA_ERROR",
                "rules[0].name",
                None,
            ),
            (
                json!({ "version": 2, "pack": "新格式", "rule_set_version": "P2" }),
                "RULESET_VERSION_UNSUPPORTED",
                "version",
                None,
            ),
            (
                with_rules(json!([new_rule(json!({ "name": "" }))])),
                "REQUIRED_FIELD_EMPTY",
                "rules[0].name",
                Some("XY-001"),
            ),
            (
                with_rules(json!([new_rule(json!({ "expr": null }))])),
                "REQUIRED_FIELD_EMPTY",
                "rules[0].expr",
                Some("XY-001"),
            ),
            (
                with_rules(json!([new_rule(json!({ "scope": "图纸" }))])),
                "REQUIRED_FIELD_EMPTY",
                "rules[0].scope_ref",
                Some("XY-001"),
            ),
            (
                with_rules(json!([{ "name": "没有编号" }])),
                "REQUIRED_FIELD_EMPTY",
                "rules[0].id",
                None,
            ),
            (
                with_rules(json!([new_rule(json!({ "category": "检查类" }))])),
                "RULE_KIND_INVALID",
                "rules[0].category",
                Some("XY-001"),
            ),
            (
                with_rules(json!([new_rule(json!({ "severity": "严重" }))])),
                "SEVERITY_INVALID",
                "rules[0].severity",
                Some("XY-001"),
            ),
            (
                with_rules(json!([new_rule(json!({ "id": "DR-150" }))])),
                "RULE_ID_RESERVED",
                "rules[0].id",
                Some("DR-150"),
            ),
            (
                with_rules(json!([new_rule(json!({})), new_rule(json!({}))])),
                "RULE_ID_DUPLICATED",
                "rules[1].id",
                Some("XY-001"),
            ),
            (
                with_rules(json!([new_rule(json!({ "expr": "流量 ==" }))])),
                "DSL_PARSE_ERROR",
                "rules[0].expr",
                Some("XY-001"),
            ),
        ];
        for (pack, code, at, rule_id) in cases {
            let error = failure(&pack);
            assert_eq!(
                (error.code(), error.at(), error.rule_id()),
                (code, at, rule_id),
                "{pack}"
            );
            assert!(
                error.to_string().starts_with(&format!("{code}: ")),
                "{error}"
            );
        }

        // 表达式的错误带着解析器给的偏移和期望。
        match failure(&with_rules(json!([new_rule(json!({ "expr": "流量 ==" }))]))) {
            DrcError::Dsl { error, .. } => {
                assert_eq!((error.offset, error.expected.as_str()), (4, "值或路径"));
            }
            other => panic!("{other}"),
        }
        assert_eq!(
            validate_project_pack(&with_rules(json!([new_rule(json!({}))]))),
            Ok(())
        );
    }
}
