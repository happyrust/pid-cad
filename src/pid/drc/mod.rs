//! 设计规则检查（`SPEC-pid-drc.md`）。
//!
//! 现在有规则表达式（[`dsl`]：解析与三值求值）和 §3.4 的名字表（[`model`]）。
//! 规则包、检查器、豁免和保存门禁在后面的步骤里加。

pub mod dsl;
pub mod model;

pub use dsl::{parse, DslError, Expr, Fact, Facts, Truth, DSL_PARSE_ERROR};
pub use model::{Kind, Target};
