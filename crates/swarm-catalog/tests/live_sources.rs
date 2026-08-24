use swarm_catalog::{CatalogOrigin, CatalogService};
use tempfile::tempdir;

#[test]
#[ignore = "requires live HTTPS access to Mojang and Fabric Meta"]
fn official_mojang_and_fabric_sources_are_compatible() {
    let cache = tempdir().expect("temporary catalog cache");
    let service = CatalogService::http(cache.path().to_owned()).expect("HTTP catalog service");
    let minecraft = service
        .minecraft_versions(false, true)
        .expect("live Mojang release catalog");
    assert_eq!(minecraft.origin, CatalogOrigin::Network);
    assert!(!minecraft.versions.is_empty());

    let mut found_compatible_loader = false;
    for version in minecraft.versions.iter().take(5) {
        let loaders = service
            .fabric_loader_versions(&version.id, true)
            .expect("live Fabric loader catalog");
        if !loaders.versions.is_empty() {
            assert!(loaders
                .versions
                .iter()
                .all(|loader| loader.minecraft_version == version.id));
            found_compatible_loader = true;
            break;
        }
    }
    assert!(
        found_compatible_loader,
        "Fabric Meta returned no loaders for the five newest Mojang releases"
    );
}
