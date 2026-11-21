//! 设计规则检查（`SPEC-pid-drc.md`）。
//!
//! 现在有规则表达式（[`dsl`]：解析与三值求值）、§3.4 的名字表和规则包的数据（[`model`]），
//! 以及内置包与项目包的读取和叠加（[`builtin_rules`]）。检查器、豁免和保存门禁在后面的
//! 步骤里加。

pub mod builtin_rules;
pub mod dsl;
pub mod model;

pub use dsl::{parse, DslError, Expr, Fact, Facts, Truth, DSL_PARSE_ERROR};
pub use model::{
    Category, DrcError, Kind, Rule, RuleGroup, RuleScope, RuleSet, SaveGate, Severity, Source,
    Target,
};
