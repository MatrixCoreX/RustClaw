use super::{SkillKind, SkillsRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostToolDescriptor {
    pub name: &'static str,
}

/// Frozen catalog of in-process host adapters. Runtime dispatch still uses the
/// registry; this catalog only validates that every compiled host adapter has
/// one correctly typed registry entry.
pub const HOST_TOOL_DESCRIPTORS: &[HostToolDescriptor] = &[
    HostToolDescriptor { name: "run_cmd" },
    HostToolDescriptor { name: "code_index" },
    HostToolDescriptor { name: "fs_basic" },
    HostToolDescriptor {
        name: "config_basic",
    },
    HostToolDescriptor { name: "read_file" },
    HostToolDescriptor { name: "write_file" },
    HostToolDescriptor { name: "list_dir" },
    HostToolDescriptor { name: "make_dir" },
    HostToolDescriptor {
        name: "remove_file",
    },
    HostToolDescriptor { name: "schedule" },
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
