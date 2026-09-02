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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeSupportError {
    InvalidInput(&'static str),
    UnsupportedMinecraft { actual: String, required: &'static str },
    UnsupportedFabricLoader { actual: String, minimum: &'static str },
    UnsupportedJava { actual: u32, minimum: u32 },
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
        }
    }
}

impl std::error::Error for RuntimeSupportError {}

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
