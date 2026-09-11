//! P&ID ribbon tools.

use crate::modules::{IconKind, ModuleEvent, ToolDef};

pub fn legend_tool() -> ToolDef {
    ToolDef {
        id: "PIDLEGEND",
        label: "P&ID Legend",
        icon: IconKind::Svg(include_bytes!("../../../assets/icons/report.svg")),
        event: ModuleEvent::Command("PIDLEGEND LIST".to_string()),
    }
}

pub fn tag_tool() -> ToolDef {
    ToolDef {
        id: "PIDTAG",
        label: "Find P&ID Tag",
        icon: IconKind::Svg(include_bytes!("../../../assets/icons/find.svg")),
        event: ModuleEvent::Command("PIDTAG".to_string()),
    }
}

pub fn group_tool() -> ToolDef {
    ToolDef {
        id: "PIDGROUP",
        label: "P&ID Group",
        icon: IconKind::Svg(include_bytes!("../../../assets/icons/group.svg")),
        event: ModuleEvent::Command("PIDGROUP GROUP".to_string()),
    }
}

pub fn ungroup_tool() -> ToolDef {
    ToolDef {
        id: "PIDUNGROUP",
        label: "Ungroup P&ID Symbol",
        icon: IconKind::Svg(include_bytes!("../../../assets/icons/ungroup.svg")),
        event: ModuleEvent::Command("PIDGROUP OFF".to_string()),
    }
}
