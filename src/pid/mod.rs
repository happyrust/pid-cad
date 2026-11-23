//! P&ID 语义层。
//!
//! 目前只有类型库（[`type_lib`]）：类型树、属性分组、数据要素，以及沿树
//! 继承的位号 / 颜色规则。位号生成、属性面板、拓扑校验等后续 feature 都
//! 从这里取公共类型。
//!
//! 这一层是纯数据 + 纯函数，不依赖 iced / Scene，可以在无 GUI 环境下构造
//! 与序列化；只有 [`TypeLib::store`] / [`TypeLib::load`] 需要一张
//! [`document::Document`]。

pub mod bom;
pub mod collab_state;
pub mod drc;
pub mod dwg_roundtrip;
pub mod graph_data;
pub mod master_data;
pub mod perf_budget;
pub mod persist;
pub mod project_index;
pub mod propagate;
pub mod publish_pkg;
pub mod snapshot;
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

/// FNV-1a 64, written as hex. Stable across compilers, unlike `DefaultHasher`.
fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Fingerprints of the three built-in libraries this program was compiled
/// with (#97). Sent on `hello` and compared with the editing sessions
/// already on the drawing.
pub(crate) fn builtin_libraries() -> ocs_collab::protocol::Libraries {
    ocs_collab::protocol::Libraries {
        type_library: fingerprint(type_lib::builtin_source().as_bytes()),
        symbol_library: fingerprint(symbols::builtin_source().as_bytes()),
        tag_rules: fingerprint(tag_rule::builtin_source().as_bytes()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn builtin_libraries_name_three_distinct_fingerprints() {
        let libraries = super::builtin_libraries();
        assert_eq!(libraries.type_library.len(), 16);
        assert_eq!(libraries.symbol_library.len(), 16);
        assert_eq!(libraries.tag_rules.len(), 16);
        assert_ne!(libraries.type_library, libraries.symbol_library);
        assert_ne!(libraries.symbol_library, libraries.tag_rules);
        assert_eq!(libraries, super::builtin_libraries(), "stable for one build");
    }
}
