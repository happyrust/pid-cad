//! 设计规则检查（`SPEC-pid-drc.md`）。
//!
//! 现在有规则表达式（[`dsl`]：解析与三值求值）、§3.4 的名字表、规则包和检查结果的数据
//! （[`model`]）、内置包与项目包的读取和叠加（[`builtin_rules`]），以及检查器（[`runner`]）
//! 和它在摘要上取的事实（[`facts`]）。豁免记在图里（[`waivers`]、[`set_waivers`]），检查时按规则号
//! 和对象键对上结果。保存门禁在后面的步骤里加。

pub mod builtin_rules;
pub mod dsl;
pub mod facts;
pub mod model;
pub mod runner;

use document::Document;
use serde_json::{json, Value};

use crate::pid::graph_data;

pub use dsl::{parse, DslError, Expr, Fact, Facts, Truth, DSL_PARSE_ERROR};
pub use model::{
    Category, CheckReport, CheckResult, DrcError, Kind, Notice, ObjectRef, Rule, RuleGroup,
    RuleScope, RuleSet, SaveGate, Scope, ScopeKind, Severity, Source, Summary, Target, Trigger,
    Waiver, WaiverError, REPORT_VERSION, WAIVER_REASON_LIMIT,
};
pub use runner::{check, CheckInput};

/// 图级记录的名字：这张图上的豁免（§3.7）。
pub const PID_AUTHOR_DRC_WAIVERS: &str = "PID_AUTHOR_DRC_WAIVERS";

/// 图里记着的豁免。读不出来的项跳过；理由在写的时候查（[`Waiver::new`]），这里不再查。
pub fn waivers(doc: &Document) -> Vec<Waiver> {
    graph_data::load(doc, PID_AUTHOR_DRC_WAIVERS)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .collect()
}

/// 把图里的豁免整条改成 `waivers`，空表删掉记录。撤销快照由调用方压。
pub fn set_waivers(doc: &mut Document, waivers: &[Waiver]) {
    let items: Vec<Value> = waivers
        .iter()
        .map(|waiver| {
            json!({
                "rule_id": waiver.rule_id,
                "object_key": waiver.object_key,
                "reason": waiver.reason,
                "by": waiver.by,
                "at": waiver.at,
            })
        })
        .collect();
    graph_data::store(doc, PID_AUTHOR_DRC_WAIVERS, &items);
}

#[cfg(test)]
mod tests {
    use document::DxfVersion;

    use super::*;

    fn waiver(rule_id: &str, object_key: &str, reason: &str) -> Waiver {
        Waiver {
            rule_id: rule_id.to_owned(),
            object_key: object_key.to_owned(),
            reason: reason.to_owned(),
            by: "zhangsan".to_owned(),
            at: 1_794_800_000,
        }
    }

    /// 豁免记录写进图、存成 DWG 再打开，逐项相等（§3.7）。整条重写，空表不留记录；
    /// 读不出来的项跳过。这条记录不在四个图级 key 里（不进协同报文）。
    #[test]
    fn waivers_survive_a_dwg_save_and_reopen() {
        let mut doc = Document::new();
        assert!(waivers(&doc).is_empty());
        let written = vec![
            waiver("DR-014", "PIPE_LINE|L-1003", "放空管，端部通大气"),
            waiver("DR-008", "PUMP|P-101#N2", &"备用管口".repeat(50)),
            waiver("DR-113", "仪表回路|F-101", "控制器在 DCS 里"),
        ];
        set_waivers(&mut doc, &written);
        assert_eq!(waivers(&doc), written);
        let bytes =
            crate::io::save_to_bytes(&doc, "dwg", DxfVersion::AC1032).expect("save dwg 2018");
        let back = crate::io::load_bytes("waivers.dwg", bytes).expect("load dwg 2018");
        assert_eq!(waivers(&back), written);
        assert!(!graph_data::keys().contains(&PID_AUTHOR_DRC_WAIVERS));

        set_waivers(&mut doc, &written[1..]);
        assert_eq!(waivers(&doc), written[1..]);
        set_waivers(&mut doc, &[]);
        assert!(waivers(&doc).is_empty());
        assert_eq!(
            graph_data::load(&doc, PID_AUTHOR_DRC_WAIVERS),
            None,
            "空表删掉记录"
        );

        graph_data::store(
            &mut doc,
            PID_AUTHOR_DRC_WAIVERS,
            &[
                json!("不是对象"),
                json!({ "rule_id": "DR-001", "object_key": "PIPE_LINE|L-1003" }),
                json!({
                    "rule_id": "DR-014", "object_key": "PIPE_LINE|L-1003",
                    "reason": "放空管，端部通大气", "by": "zhangsan", "at": 1_794_800_000
                }),
            ],
        );
        assert_eq!(waivers(&doc), written[..1]);
    }
}
