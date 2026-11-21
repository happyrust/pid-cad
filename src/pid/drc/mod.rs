//! 设计规则检查（`SPEC-pid-drc.md`）。
//!
//! 现在有规则表达式（[`dsl`]：解析与三值求值）、§3.4 的名字表、规则包和检查结果的数据
//! （[`model`]）、内置包与项目包的读取和叠加（[`builtin_rules`]），以及检查器（[`runner`]）
//! 和它在摘要上取的事实（[`facts`]）。豁免和保存门禁在后面的步骤里加。

pub mod builtin_rules;
pub mod dsl;
pub mod facts;
pub mod model;
pub mod runner;

pub use dsl::{parse, DslError, Expr, Fact, Facts, Truth, DSL_PARSE_ERROR};
pub use model::{
    Category, CheckReport, CheckResult, DrcError, Kind, Notice, ObjectRef, Rule, RuleGroup,
    RuleScope, RuleSet, SaveGate, Scope, ScopeKind, Severity, Source, Summary, Target, Trigger,
    Waiver, REPORT_VERSION,
};
pub use runner::{check, CheckInput};
