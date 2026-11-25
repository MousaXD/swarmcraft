use serde::Serialize;
use std::fmt;

/// Runtime contract declared by the Fabric bridge that ships with this build.
///
/// Keep this synchronized with `minecraft/fabric/src/main/resources/fabric.mod.json`.
/// A single shared contract prevents provider-valid but bridge-incompatible
/// Minecraft/Fabric tuples from becoming canonical worlds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeAdapterSupport {
    pub adapter_id: &'static str,
    pub minecraft_requirement: &'static str,
    pub minimum_fabric_loader: &'static str,
    pub minimum_java_major: u32,
}

pub const SHIPPED_RUNTIME_ADAPTER: RuntimeAdapterSupport = RuntimeAdapterSupport {
    adapter_id: "swarmcraft-fabric-v1",
    minecraft_requirement: "~26.1.2",
    minimum_fabric_loader: "0.19.3",
    minimum_java_major: 25,
};

/// A build's declared bridge range is not proof that every matching release
/// has been exercised. New-world selection is deliberately pinned to the
/// release-candidate baseline while field qualification remains outstanding.
pub const PINNED_MINECRAFT_VERSION: &str = "26.1.2";
pub const PINNED_FABRIC_LOADER_VERSION: &str = "0.19.3";
pub const PINNED_JAVA_MAJOR: u32 = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeQualification {
    PinnedBaseline,
    AdapterCompatibleUnqualified,
    UnsupportedAdapter,
}

impl RuntimeQualification {
    pub fn adapter_compatible(self) -> bool {
        self != Self::UnsupportedAdapter
    }

    pub fn selectable(self) -> bool {
        self == Self::PinnedBaseline
    }
}

#[derive(Debug, Serialize)]
pub struct RuntimeMatrixEntry {
    pub minecraft_version: &'static str,
    pub fabric_loader_version: &'static str,
    pub reference_java_major: u32,
    pub qualification: RuntimeQualification,
    pub field_acceptance: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RuntimeCompatibilityMatrix {
    pub adapter_id: &'static str,
    pub declared_minecraft_range: &'static str,
    pub declared_minimum_fabric_loader: &'static str,
    pub declared_minimum_java_major: u32,
    pub new_world_baselines: [RuntimeMatrixEntry; 1],
}

pub fn runtime_compatibility_matrix() -> RuntimeCompatibilityMatrix {
    RuntimeCompatibilityMatrix {
        adapter_id: SHIPPED_RUNTIME_ADAPTER.adapter_id,
        declared_minecraft_range: SHIPPED_RUNTIME_ADAPTER.minecraft_requirement,
        declared_minimum_fabric_loader: SHIPPED_RUNTIME_ADAPTER.minimum_fabric_loader,
        declared_minimum_java_major: SHIPPED_RUNTIME_ADAPTER.minimum_java_major,
        new_world_baselines: [RuntimeMatrixEntry {
            minecraft_version: PINNED_MINECRAFT_VERSION,
            fabric_loader_version: PINNED_FABRIC_LOADER_VERSION,
            reference_java_major: PINNED_JAVA_MAJOR,
            qualification: RuntimeQualification::PinnedBaseline,
            field_acceptance: "pending_real_fabric_launch_save_restore_and_authenticated_client",
        }],
    }
}

pub fn minecraft_qualification(minecraft_version: &str) -> RuntimeQualification {
    if minecraft_version == PINNED_MINECRAFT_VERSION {
        RuntimeQualification::PinnedBaseline
    } else if minecraft_supported_by_shipped_adapter(minecraft_version) {
        RuntimeQualification::AdapterCompatibleUnqualified
    } else {
        RuntimeQualification::UnsupportedAdapter
    }
}

pub fn runtime_qualification(
    minecraft_version: &str,
    fabric_loader_version: &str,
    java_major: Option<u32>,
) -> RuntimeQualification {
    if validate_runtime_selection(minecraft_version, fabric_loader_version, java_major).is_err() {
        RuntimeQualification::UnsupportedAdapter
    } else if minecraft_version == PINNED_MINECRAFT_VERSION
        && fabric_loader_version == PINNED_FABRIC_LOADER_VERSION
        && java_major.is_none_or(|major| major == PINNED_JAVA_MAJOR)
    {
        RuntimeQualification::PinnedBaseline
    } else {
        RuntimeQualification::AdapterCompatibleUnqualified
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeSupportError {
    InvalidInput(&'static str),
    UnsupportedMinecraft { actual: String, required: &'static str },
    UnsupportedFabricLoader { actual: String, minimum: &'static str },
    UnsupportedJava { actual: u32, minimum: u32 },
    UnqualifiedNewWorld { minecraft: String, fabric: String },
}

impl fmt::Display for RuntimeSupportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(label) => write!(formatter, "{label} must be a non-empty trimmed version token"),
            Self::UnsupportedMinecraft { actual, required } => write!(
                formatter,
                "Minecraft {actual} is not supported by the shipped SwarmCraft Fabric adapter ({required})"
            ),
            Self::UnsupportedFabricLoader { actual, minimum } => write!(
                formatter,
                "Fabric Loader {actual} is below the shipped SwarmCraft Fabric adapter requirement (>={minimum})"
            ),
            Self::UnsupportedJava { actual, minimum } => write!(
                formatter,
                "Java {actual} is below the shipped SwarmCraft Fabric adapter requirement (>={minimum})"
            ),
            Self::UnqualifiedNewWorld { minecraft, fabric } => write!(
                formatter,
                "Minecraft {minecraft} / Fabric {fabric} is within the declared adapter range but has not been qualified for new worlds. Use the pinned baseline Minecraft {PINNED_MINECRAFT_VERSION} / Fabric {PINNED_FABRIC_LOADER_VERSION}."
            ),
        }
    }
}

impl std::error::Error for RuntimeSupportError {}

/// Only the exact pinned development/release-candidate tuple is offered for
/// new playable worlds. Existing signed worlds keep their existing protocol
/// and runtime checks; this does not rewrite their compatibility fingerprints.
pub fn validate_new_world_selection(
    minecraft_version: &str,
    fabric_loader_version: &str,
) -> Result<&'static RuntimeAdapterSupport, RuntimeSupportError> {
    let support = validate_runtime_selection(minecraft_version, fabric_loader_version, None)?;
    if !runtime_qualification(minecraft_version, fabric_loader_version, None).selectable() {
        return Err(RuntimeSupportError::UnqualifiedNewWorld {
            minecraft: minecraft_version.to_owned(),
            fabric: fabric_loader_version.to_owned(),
        });
    }
    Ok(support)
}

pub fn validate_runtime_selection(
    minecraft_version: &str,
    fabric_loader_version: &str,
    java_major: Option<u32>,
) -> Result<&'static RuntimeAdapterSupport, RuntimeSupportError> {
    validate_token(minecraft_version, "Minecraft version")?;
    validate_token(fabric_loader_version, "Fabric Loader version")?;

    if !minecraft_matches_tilde_26_1_2(minecraft_version) {
        return Err(RuntimeSupportError::UnsupportedMinecraft {
            actual: minecraft_version.to_owned(),
            required: SHIPPED_RUNTIME_ADAPTER.minecraft_requirement,
        });
    }
    if !numeric_version_at_least(fabric_loader_version, SHIPPED_RUNTIME_ADAPTER.minimum_fabric_loader) {
        return Err(RuntimeSupportError::UnsupportedFabricLoader {
            actual: fabric_loader_version.to_owned(),
            minimum: SHIPPED_RUNTIME_ADAPTER.minimum_fabric_loader,
        });
    }
    if let Some(actual) = java_major {
        if actual < SHIPPED_RUNTIME_ADAPTER.minimum_java_major {
            return Err(RuntimeSupportError::UnsupportedJava {
                actual,
                minimum: SHIPPED_RUNTIME_ADAPTER.minimum_java_major,
            });
        }
    }
    Ok(&SHIPPED_RUNTIME_ADAPTER)
}

pub fn minecraft_supported_by_shipped_adapter(minecraft_version: &str) -> bool {
    minecraft_matches_tilde_26_1_2(minecraft_version)
}

fn validate_token(value: &str, label: &'static str) -> Result<(), RuntimeSupportError> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(RuntimeSupportError::InvalidInput(label));
    }
    Ok(())
}

fn minecraft_matches_tilde_26_1_2(value: &str) -> bool {
    let Some(parts) = parse_numeric_version(value) else {
        return false;
    };
    parts.len() == 3 && parts[0] == 26 && parts[1] == 1 && parts[2] >= 2
}

fn numeric_version_at_least(actual: &str, minimum: &str) -> bool {
    let (Some(mut actual), Some(mut minimum)) = (parse_numeric_version(actual), parse_numeric_version(minimum)) else {
        return false;
    };
    let width = actual.len().max(minimum.len());
    actual.resize(width, 0);
    minimum.resize(width, 0);
    actual >= minimum
}

fn parse_numeric_version(value: &str) -> Option<Vec<u64>> {
    if value.is_empty() {
        return None;
    }
    value
        .split('.')
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                None
            } else {
                part.parse::<u64>().ok()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_contract_matches_fabric_metadata() {
        let metadata = include_str!("../../../minecraft/fabric/src/main/resources/fabric.mod.json");
        assert!(metadata.contains("\"fabricloader\": \">=0.19.3\""));
        assert!(metadata.contains("\"minecraft\": \"~26.1.2\""));
        assert!(metadata.contains("\"java\": \">=25\""));
        let build = include_str!("../../../minecraft/fabric/gradle.properties");
        assert!(build.lines().any(|line| line == format!("minecraft_version={PINNED_MINECRAFT_VERSION}")));
        assert!(build.lines().any(|line| line == format!("loader_version={PINNED_FABRIC_LOADER_VERSION}")));
        assert_eq!(PINNED_JAVA_MAJOR, SHIPPED_RUNTIME_ADAPTER.minimum_java_major);
    }

    #[test]
    fn qualification_is_exact_and_does_not_relabel_declared_ranges_as_tested() {
        use RuntimeQualification::*;
        for (minecraft, fabric, java, expected) in [
            ("26.1.2", "0.19.3", None, PinnedBaseline),
            ("26.1.2", "0.19.3", Some(25), PinnedBaseline),
            ("26.1.2", "0.19.3", Some(26), AdapterCompatibleUnqualified),
            ("26.1.5", "0.19.3", Some(25), AdapterCompatibleUnqualified),
            ("26.1.2", "0.20.0", None, AdapterCompatibleUnqualified),
            ("26.1.2", "0.19.3.0", None, AdapterCompatibleUnqualified),
            ("26.01.02", "0.19.3", None, AdapterCompatibleUnqualified),
            ("26.1.2", "0.19.2", None, UnsupportedAdapter),
            ("26.1.2", "0.20.0-beta.1", None, UnsupportedAdapter),
            ("26.2", "0.19.3", None, UnsupportedAdapter),
            ("26.1.2", "0.19.3", Some(21), UnsupportedAdapter),
        ] {
            assert_eq!(runtime_qualification(minecraft, fabric, java), expected);
            if java.is_none() {
                assert_eq!(validate_new_world_selection(minecraft, fabric).is_ok(), expected.selectable());
            }
        }
        assert_eq!(minecraft_qualification("26.1.5"), AdapterCompatibleUnqualified);
        let matrix = runtime_compatibility_matrix();
        assert_eq!(matrix.new_world_baselines[0].qualification, PinnedBaseline);
        assert!(matrix.new_world_baselines[0].field_acceptance.starts_with("pending_"));
    }

    #[test]
    fn accepts_only_bridge_compatible_runtime_tuples() {
        assert!(validate_runtime_selection("26.1.2", "0.19.3", Some(25)).is_ok());
        assert!(validate_runtime_selection("26.1.5", "0.20.0", Some(25)).is_ok());
        assert!(matches!(
            validate_runtime_selection("26.2", "0.19.3", Some(25)),
            Err(RuntimeSupportError::UnsupportedMinecraft { .. })
        ));
        assert!(matches!(
            validate_runtime_selection("26.1.2", "0.19.2", Some(25)),
            Err(RuntimeSupportError::UnsupportedFabricLoader { .. })
        ));
        assert!(matches!(
            validate_runtime_selection("26.1.2", "0.19.3", Some(21)),
            Err(RuntimeSupportError::UnsupportedJava { .. })
        ));
    }
}
