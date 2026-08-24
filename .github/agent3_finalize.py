from pathlib import Path
import re

path = Path("apps/desktop/src-tauri/src/curseforge.rs")
text = path.read_text()


def replace_once(old: str, new: str, label: str, already: str | None = None) -> None:
    global text
    if old in text:
        text = text.replace(old, new, 1)
        return
    marker = already or new
    if marker in text:
        return
    raise SystemExit(f"could not apply {label}")


def replace_regex(pattern: str, replacement: str, label: str, already: str) -> None:
    global text
    if already in text:
        return
    updated, count = re.subn(pattern, lambda _match: replacement, text, count=1, flags=re.S)
    if count != 1:
        raise SystemExit(f"could not apply {label}; matches={count}")
    text = updated


# Rust cannot infer the async block's error type from the outer match alone.
replace_regex(
    r'(?P<indent>\s*)Ok\(json!\(\{\n(?P<body>\s*"project_id": project_id\.to_string\(\),\n\s*"minecraft": target\.minecraft,\n\s*"loader": target\.loader,\n\s*"versions": versions,\n\s*\}\)\))',
    '        Ok::<Value, ProviderError>(json!({\n            "project_id": project_id.to_string(),\n            "minecraft": target.minecraft,\n            "loader": target.loader,\n            "versions": versions,\n        }))',
    "curseforge_versions result annotation",
    "Ok::<Value, ProviderError>(json!({",
)

# CurseForge gameVersions mixes Minecraft versions and loader-family tags. Keep them
# separate so the handoff matches Agent 2's provider-neutral semantics.
replace_regex(
    r'fn loader_tags\(file: &Value\) -> Vec<String> \{.*?\n\}\n\n(?=fn mapped_dependencies)',
    '''fn is_loader_tag(version: &str) -> bool {
    ["Fabric", "Forge", "NeoForge", "Quilt", "LiteLoader", "Cauldron"]
        .iter()
        .any(|loader| version.eq_ignore_ascii_case(loader))
}

fn minecraft_versions(file: &Value) -> Vec<String> {
    game_versions(file)
        .into_iter()
        .filter(|version| !is_loader_tag(version))
        .collect()
}

fn loader_tags(file: &Value) -> Vec<String> {
    game_versions(file)
        .into_iter()
        .filter(|version| is_loader_tag(version))
        .collect()
}

''',
    "Minecraft/loader metadata separation",
    "fn minecraft_versions(file: &Value) -> Vec<String>",
)
replace_once(
    "    let versions = game_versions(file);\n    let loaders = loader_tags(file);",
    "    let minecraft_versions = minecraft_versions(file);\n    let loaders = loader_tags(file);",
    "map_file version separation",
    "let minecraft_versions = minecraft_versions(file);",
)
replace_once(
    '        "minecraft_versions": versions,',
    '        "minecraft_versions": minecraft_versions,',
    "map_file clean minecraft_versions output",
)

replace_regex(
    r'fn validate_selected_file\(file: &Value, target: &Target\) -> Result<\(\), ProviderError> \{.*?\n\}\n\n(?=fn classify_version_gap)',
    '''fn validate_selected_file(file: &Value, target: &Target) -> Result<(), ProviderError> {
    let versions = minecraft_versions(file);
    if !versions.iter().any(|version| version == &target.minecraft) {
        return Err(ProviderError::new(
            "incompatible",
            "no_compatible_minecraft_version",
            format!("The selected CurseForge file is not tagged for Minecraft {}", target.minecraft),
        ));
    }
    let loaders = loader_tags(file);
    if !loaders.iter().any(|loader| loader.eq_ignore_ascii_case("fabric")) {
        return Err(ProviderError::new(
            "incompatible",
            "no_fabric_build",
            "The selected CurseForge file is not a Fabric build",
        ));
    }
    Ok(())
}

''',
    "selected file compatibility",
    "let versions = minecraft_versions(file);",
)

# Make cycle termination explicit and directly testable without changing the existing
# deterministic selection semantics.
if "fn should_select_required_dependency" not in text:
    marker = "fn detect_impossible_relations(selected: &BTreeMap<u64, Value>) -> Result<(), ProviderError> {"
    if marker not in text:
        raise SystemExit("could not locate dependency relation checker")
    text = text.replace(
        marker,
        '''fn should_select_required_dependency(selected: &BTreeMap<u64, Value>, dependency_id: u64) -> bool {
    !selected.contains_key(&dependency_id)
}

''' + marker,
        1,
    )
replace_once(
    "                    if selected.contains_key(&dependency_id) {\n                        continue;\n                    }",
    "                    if !should_select_required_dependency(&selected, dependency_id) {\n                        continue;\n                    }",
    "required dependency cycle guard",
    "if !should_select_required_dependency(&selected, dependency_id)",
)

# Manual remediation must preserve the exact artifact identity and enough provider
# metadata for Agent 4 to verify a player-supplied JAR without inventing a URL or
# silently substituting another file.
replace_regex(
    r'fn manual_artifact_required\(file: &Value, project: &Value\) -> Result<Value, ProviderError> \{.*?\n\}\n\n(?=struct TempArtifact)',
    '''fn manual_artifact_required(file: &Value, project: &Value) -> Result<Value, ProviderError> {
    let project_id = required_u64(file, "modId")?;
    let file_id = required_u64(file, "id")?;
    let file_name = required_string(file, "fileName")?;
    let display_name = required_string(file, "displayName")?;
    let website_url = project
        .get("links")
        .and_then(|links| links.get("websiteUrl"))
        .cloned()
        .unwrap_or(Value::Null);
    let project_name = nonempty_string(project.get("name")).map(ToOwned::to_owned);
    let project_slug = nonempty_string(project.get("slug")).map(ToOwned::to_owned);
    let minecraft_versions = minecraft_versions(file);
    let loaders = loader_tags(file);
    let fabric_compatible = loaders
        .iter()
        .any(|loader| loader.eq_ignore_ascii_case("fabric"));
    let reason = "CurseForge did not provide an automatic download URL for this exact file. Desktop must ask the player to obtain this exact artifact and verify it before use.";
    Ok(json!({
        "status": "manual_artifact_required",
        "provider": "curseforge",
        "data": {
            "provider": "curseforge",
            "project_id": project_id.to_string(),
            "file_id": file_id.to_string(),
            "version_id": file_id.to_string(),
            "name": display_name,
            "file_name": file_name,
            "file_size": file.get("fileLength").cloned().unwrap_or(Value::Null),
            "hashes": provider_hashes(file),
            "minecraft_versions": minecraft_versions,
            "loaders": loaders,
            "fabric_compatible": fabric_compatible,
            "dependencies": mapped_dependencies(file),
            "project_url": website_url.clone(),
            "project": {
                "project_id": project_id.to_string(),
                "name": project_name,
                "slug": project_slug,
                "website_url": website_url,
                "allow_mod_distribution": project
                    .get("allowModDistribution")
                    .cloned()
                    .unwrap_or(Value::Null),
            },
            "remediation": {
                "kind": "manual_artifact_required",
                "provider": "curseforge",
                "automatic_retrieval_available": false,
                "reason_code": "provider_download_unavailable",
                "supply_exact_project_id": project_id.to_string(),
                "supply_exact_file_id": file_id.to_string(),
                "reason": reason,
            },
            "reason": reason,
        }
    }))
}

''',
    "manual artifact handoff",
    '"reason_code": "provider_download_unavailable"',
)

# Centralize declared-size validation so the provider-size bound is exercised without
# a network fixture.
if "fn validate_declared_artifact_size" not in text:
    marker = "fn verify_download_size(expected: u64, actual: u64) -> Result<(), ProviderError> {"
    if marker not in text:
        raise SystemExit("could not locate download size verifier")
    text = text.replace(
        marker,
        '''fn validate_declared_artifact_size(expected_size: u64) -> Result<(), ProviderError> {
    if expected_size == 0 || expected_size > MAX_ARTIFACT_BYTES {
        Err(ProviderError::new(
            "download_failed",
            "artifact_size_out_of_bounds",
            format!(
                "CurseForge declared artifact size {expected_size}; automatic downloads are limited to {MAX_ARTIFACT_BYTES} bytes"
            ),
        ))
    } else {
        Ok(())
    }
}

''' + marker,
        1,
    )
replace_regex(
    r'    if expected_size == 0 \|\| expected_size > MAX_ARTIFACT_BYTES \{\n        return Err\(ProviderError::new\(\n            "download_failed",\n            "artifact_size_out_of_bounds",\n            format!\(\n                "CurseForge declared artifact size \{expected_size\}; automatic downloads are limited to \{MAX_ARTIFACT_BYTES\} bytes"\n            \),\n        \)\);\n    \}',
    "    validate_declared_artifact_size(expected_size)?;",
    "declared size helper use",
    "validate_declared_artifact_size(expected_size)?;",
)

# Keep publication as same-directory rename and make the exact publication operation
# directly testable. The file itself has already been flush()ed and sync_all()'d.
if "fn publish_verified_artifact" not in text:
    marker = "async fn download_artifact("
    if marker not in text:
        raise SystemExit("could not locate artifact downloader")
    text = text.replace(
        marker,
        '''fn publish_verified_artifact(
    temp: &mut TempArtifact,
    destination: &Path,
) -> Result<(), ProviderError> {
    std::fs::rename(&temp.path, destination).map_err(|error| {
        ProviderError::new(
            "download_failed",
            "atomic_publication_failed",
            format!("Could not atomically publish verified artifact: {error}"),
        )
    })?;
    temp.disarm();
    Ok(())
}

''' + marker,
        1,
    )
replace_regex(
    r'    tokio::fs::rename\(&temp\.path, &destination\)\n        \.await\n        \.map_err\(\|error\| \{\n            ProviderError::new\(\n                "download_failed",\n                "atomic_publication_failed",\n                format!\("Could not atomically publish verified artifact: \{error\}"\),\n            \)\n        \}\)\?;\n    temp\.disarm\(\);',
    "    publish_verified_artifact(&mut temp, &destination)?;",
    "testable atomic publication helper",
    "publish_verified_artifact(&mut temp, &destination)?;",
)

# Strengthen the existing manual-remediation fixture assertions in place.
replace_once(
    '''        let project = json!({
            "links": { "websiteUrl": "https://www.curseforge.com/minecraft/mc-mods/example" }
        });
        let response = manual_artifact_required(&fixture_file(), &project).unwrap();
        assert_eq!(response["status"], "manual_artifact_required");
        assert_eq!(response["data"]["project_id"], "123");
        assert_eq!(response["data"]["file_id"], "456");
        assert_eq!(response["data"]["file_name"], "example-1.2.3.jar");''',
    '''        let project = json!({
            "id": 123,
            "name": "Example",
            "slug": "example",
            "allowModDistribution": false,
            "links": { "websiteUrl": "https://www.curseforge.com/minecraft/mc-mods/example" }
        });
        let response = manual_artifact_required(&fixture_file(), &project).unwrap();
        assert_eq!(response["status"], "manual_artifact_required");
        assert_eq!(response["data"]["project_id"], "123");
        assert_eq!(response["data"]["file_id"], "456");
        assert_eq!(response["data"]["version_id"], "456");
        assert_eq!(response["data"]["file_name"], "example-1.2.3.jar");
        assert_eq!(response["data"]["file_size"], 3);
        assert_eq!(response["data"]["minecraft_versions"], json!(["1.21.1"]));
        assert_eq!(response["data"]["loaders"], json!(["Fabric"]));
        assert_eq!(response["data"]["fabric_compatible"], true);
        assert_eq!(response["data"]["hashes"].as_array().unwrap().len(), 2);
        assert_eq!(response["data"]["dependencies"][0]["project_id"], "700");
        assert_eq!(response["data"]["project"]["name"], "Example");
        assert_eq!(response["data"]["project"]["allow_mod_distribution"], false);
        assert_eq!(
            response["data"]["remediation"]["reason_code"],
            "provider_download_unavailable"
        );
        assert_eq!(
            response["data"]["remediation"]["supply_exact_file_id"],
            "456"
        );''',
    "manual artifact fixture assertions",
    'response["data"]["remediation"]["reason_code"]',
)

if "fn provider_metadata_separates_minecraft_versions_from_loader_tags" not in text:
    marker = "    #[test]\n    fn deterministic_file_selection_prefers_latest_then_highest_id() {"
    if marker not in text:
        raise SystemExit("could not locate deterministic selection test")
    tests = r'''    #[test]
    fn provider_metadata_separates_minecraft_versions_from_loader_tags() {
        let mapped = map_file(&fixture_file(), "server").unwrap();
        assert_eq!(mapped["minecraft_versions"], json!(["1.21.1"]));
        assert_eq!(mapped["loaders"], json!(["Fabric"]));
    }

    #[test]
    fn required_dependency_cycle_back_to_selected_project_terminates() {
        let mut selected = BTreeMap::new();
        selected.insert(123, fixture_file());
        assert!(!should_select_required_dependency(&selected, 123));
        assert!(should_select_required_dependency(&selected, 700));
    }

    #[test]
    fn local_sha256_is_exact_and_changes_with_artifact_bytes() {
        let expected = hex::encode(Sha256::digest(b"abc"));
        let different = hex::encode(Sha256::digest(b"abd"));
        assert_eq!(
            expected,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_ne!(expected, different);
    }

    #[test]
    fn declared_download_size_is_bounded() {
        assert!(validate_declared_artifact_size(1).is_ok());
        assert!(validate_declared_artifact_size(MAX_ARTIFACT_BYTES).is_ok());
        assert_eq!(
            validate_declared_artifact_size(0).unwrap_err().code,
            "artifact_size_out_of_bounds"
        );
        assert_eq!(
            validate_declared_artifact_size(MAX_ARTIFACT_BYTES + 1)
                .unwrap_err()
                .code,
            "artifact_size_out_of_bounds"
        );
    }

    #[test]
    fn partial_file_guard_cleans_up_interrupted_artifact() {
        let directory = std::env::temp_dir().join(format!(
            "swarmcraft-curseforge-partial-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let temporary = directory.join(".artifact.jar.part");
        std::fs::write(&temporary, b"partial").unwrap();
        let guard = TempArtifact::new(temporary.clone());
        drop(guard);
        assert!(!temporary.exists());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn successful_verified_artifact_publication_is_atomic_rename() {
        let directory = std::env::temp_dir().join(format!(
            "swarmcraft-curseforge-publish-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let temporary = directory.join(".artifact.jar.part");
        let destination = directory.join("artifact.jar");
        std::fs::write(&temporary, b"abc").unwrap();
        let mut guard = TempArtifact::new(temporary.clone());
        publish_verified_artifact(&mut guard, &destination).unwrap();
        assert!(!temporary.exists());
        assert_eq!(std::fs::read(&destination).unwrap(), b"abc");
        drop(guard);
        assert!(destination.exists());
        let _ = std::fs::remove_dir_all(directory);
    }

'''
    text = text.replace(marker, tests + marker, 1)

required = [
    "Ok::<Value, ProviderError>(json!({",
    "fn minecraft_versions(file: &Value) -> Vec<String>",
    '"reason_code": "provider_download_unavailable"',
    "fn should_select_required_dependency",
    "fn validate_declared_artifact_size",
    "fn publish_verified_artifact",
    "provider_metadata_separates_minecraft_versions_from_loader_tags",
    "partial_file_guard_cleans_up_interrupted_artifact",
    "successful_verified_artifact_publication_is_atomic_rename",
]
missing = [fragment for fragment in required if fragment not in text]
if missing:
    raise SystemExit(f"finalization incomplete: {missing}")

path.write_text(text)
