//! P&ID 语义层。
//!
//! 目前只有类型库（[`type_lib`]）：类型树、属性分组、数据要素，以及沿树
//! 继承的位号 / 颜色规则。位号生成、属性面板、拓扑校验等后续 feature 都
//! 从这里取公共类型。
//!
//! 这一层是纯数据 + 纯函数，不依赖 iced / Scene，可以在无 GUI 环境下构造
//! 与序列化；只有 [`TypeLib::store`] / [`TypeLib::load`] 需要一张
//! [`document::Document`]。

pub mod collab_state;
pub mod dwg_roundtrip;
pub mod graph_data;
pub mod master_data;
pub mod perf_budget;
pub mod persist;
pub mod project_index;
pub mod propagate;
pub mod symbols;
pub mod tag_engine;
pub mod tag_rule;
pub mod topology;
pub mod type_lib;

pub use collab_state::{FileCollabState, QueuedOp, PID_AUTHOR_COLLAB, PID_AUTHOR_COLLAB_QUEUE};
pub use graph_data::{
    PID_AUTHOR_CONNECTORS, PID_AUTHOR_TAG_RULES, PID_AUTHOR_TOPOLOGY, PID_AUTHOR_TYPE_LIB,
};
pub use persist::{PidAuthor, PID_AUTHOR_APP, XDATA_KEYS, XDATA_SOFT_LIMIT};
pub use tag_engine::{TagError, TagView};
pub use tag_rule::{HierarchyMode, TagRule};
pub use type_lib::{
    DataElement, PropertyGroup, TypeLib, TypeLibError, TypeNode, ValueType, XRECORD_KEY,
};
