use super::{SkillKind, SkillsRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostToolAdapterKind {
    /// Dispatched by clawd's in-process builtin adapter.
    InProcessBuiltin,
    /// Registry-visible compatibility facade rewritten to a canonical runtime
    /// capability before execution.
    VirtualFacade,
    /// Executed inside the generic agent loop instead of the builtin adapter.
    AgentLoopInternal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostToolDescriptor {
    pub name: &'static str,
    pub adapter_kind: HostToolAdapterKind,
}

/// Frozen catalog of host-owned tools. Runtime dispatch still uses the
/// registry; this catalog validates that every compiled host tool has one
/// correctly typed registry entry and records which host execution surface
/// owns it.
pub const HOST_TOOL_DESCRIPTORS: &[HostToolDescriptor] = &[
    HostToolDescriptor {
        name: "browser_session",
        adapter_kind: HostToolAdapterKind::InProcessBuiltin,
    },
    HostToolDescriptor {
        name: "run_cmd",
        adapter_kind: HostToolAdapterKind::InProcessBuiltin,
    },
    HostToolDescriptor {
        name: "code_index",
        adapter_kind: HostToolAdapterKind::InProcessBuiltin,
    },
    HostToolDescriptor {
        name: "fs_basic",
        adapter_kind: HostToolAdapterKind::VirtualFacade,
    },
    HostToolDescriptor {
        name: "config_basic",
        adapter_kind: HostToolAdapterKind::VirtualFacade,
    },
    HostToolDescriptor {
        name: "read_file",
        adapter_kind: HostToolAdapterKind::InProcessBuiltin,
    },
    HostToolDescriptor {
        name: "write_file",
        adapter_kind: HostToolAdapterKind::InProcessBuiltin,
    },
    HostToolDescriptor {
        name: "list_dir",
        adapter_kind: HostToolAdapterKind::InProcessBuiltin,
    },
    HostToolDescriptor {
        name: "make_dir",
        adapter_kind: HostToolAdapterKind::InProcessBuiltin,
    },
    HostToolDescriptor {
        name: "remove_file",
        adapter_kind: HostToolAdapterKind::InProcessBuiltin,
    },
    HostToolDescriptor {
        name: "schedule",
        adapter_kind: HostToolAdapterKind::InProcessBuiltin,
    },
    HostToolDescriptor {
        name: "workspace_patch",
        adapter_kind: HostToolAdapterKind::InProcessBuiltin,
    },
    HostToolDescriptor {
        name: "task_plan",
        adapter_kind: HostToolAdapterKind::InProcessBuiltin,
    },
    HostToolDescriptor {
        name: "subagent",
        adapter_kind: HostToolAdapterKind::AgentLoopInternal,
    },
];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RegistryIntegrityReport {
    pub missing: Vec<String>,
    pub wrong_kind: Vec<String>,
}

impl RegistryIntegrityReport {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.wrong_kind.is_empty()
    }

    pub fn into_human_message(self) -> Option<String> {
        if self.is_clean() {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        if !self.missing.is_empty() {
            parts.push(format!("missing builtins: {}", self.missing.join(", ")));
        }
        if !self.wrong_kind.is_empty() {
            parts.push(format!(
                "builtins with wrong kind (expected kind=builtin): {}",
                self.wrong_kind.join(", ")
            ));
        }
        Some(parts.join("; "))
    }
}

impl SkillsRegistry {
    /// Validate required built-in registry entries and their kinds.
    pub fn integrity_report(&self) -> RegistryIntegrityReport {
        let mut missing: Vec<String> = Vec::new();
        let mut wrong_kind: Vec<String> = Vec::new();
        for descriptor in HOST_TOOL_DESCRIPTORS {
            let name = descriptor.name;
            match self.get(name) {
                None => missing.push(name.to_string()),
                Some(entry) if entry.kind != SkillKind::Builtin => {
                    wrong_kind.push(name.to_string());
                }
                Some(_) => {}
            }
        }
        RegistryIntegrityReport {
            missing,
            wrong_kind,
        }
    }
}
