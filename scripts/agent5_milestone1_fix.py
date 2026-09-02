from pathlib import Path

root = Path(__file__).resolve().parents[1]

path = root / "crates/swarm-protocol/src/canonical_modpack.rs"
text = path.read_text()
block = '''    if artifact.retrieval == CanonicalRetrievalV1::ProviderDownload
        && !algorithms.keys().any(|algorithm| {
            matches!(
                algorithm,
                CanonicalHashAlgorithmV1::Sha512
                    | CanonicalHashAlgorithmV1::Sha256
                    | CanonicalHashAlgorithmV1::Sha1
            )
        })
    {
        return Err(CanonicalModpackError::InvalidProviderHash(format!(
            "{} is marked provider_download but has only MD5/unsupported reacquisition proof",
            artifact.identity.display_key()
        )));
    }
'''
needle = block + "    for dependency in &artifact.dependencies {"
if needle not in text:
    raise SystemExit("misplaced provider-download hash block not found")
text = text.replace(needle, "    for dependency in &artifact.dependencies {", 1)
marker = "fn validate_provider_artifact(artifact: &CanonicalProviderArtifactV1) -> Result<(), CanonicalModpackError> {"
pos = text.index(marker)
head, tail = text[:pos], text[pos:]
needle = "    for dependency in &artifact.dependencies {"
if needle not in tail:
    raise SystemExit("provider validation dependency loop not found")
tail = tail.replace(needle, block + needle, 1)
path.write_text(head + tail)

# A provider JAR must also have a non-empty basename stem. `.jar` is not a useful
# portable artifact identity even though it is technically a single path component.
path = root / "apps/desktop/src-tauri/src/curseforge.rs"
text = path.read_text()
old = "    let portable = !value.is_empty()\n"
new = "    let portable = value.len() > 4\n"
if old not in text:
    raise SystemExit("safe provider filename predicate not found")
path.write_text(text.replace(old, new, 1))

print("Agent 5 milestone 1 anchors corrected")
