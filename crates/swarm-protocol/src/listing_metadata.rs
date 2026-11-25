//! Optional, coarse listing metadata for new V1 worlds. No schema change:
//! language is a reserved signed presentation tag, not inferred identity.
use crate::{WorldPresentationV1, WorldVisibilityV1};

pub const LISTING_REGIONS: &[&str] =
    &["africa", "asia", "europe", "middle-east", "north-america", "oceania", "south-america"];

pub fn listing_region(value: &str) -> Result<String, &'static str> {
    let value = value.trim().to_ascii_lowercase();
    if LISTING_REGIONS.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err("choose a supported coarse listing region")
    }
}

pub fn listing_language_tag(value: &str) -> Result<String, &'static str> {
    let value = value.trim().to_ascii_lowercase();
    if (2..=3).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_lowercase()) {
        Ok(format!("lang:{value}"))
    } else {
        Err("language must be a two- or three-letter language code")
    }
}

/// Only deliberate creation-time metadata; never infer region from IP or
/// locale. A private world cannot accidentally retain public listing inputs.
pub fn new_listing_presentation(
    name: String,
    visibility: WorldVisibilityV1,
    region: Option<&str>,
    language: Option<&str>,
) -> Result<WorldPresentationV1, &'static str> {
    let region = region.map(str::trim).filter(|value| !value.is_empty());
    let language = language.map(str::trim).filter(|value| !value.is_empty());
    if visibility == WorldVisibilityV1::Private && (region.is_some() || language.is_some()) {
        return Err("private worlds cannot include public listing region or language");
    }
    Ok(WorldPresentationV1 {
        name,
        description: String::new(),
        tags: language.map(listing_language_tag).transpose()?.into_iter().collect(),
        icon_hash: None,
        approximate_region: region.map(listing_region).transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creation_metadata_is_explicit_coarse_and_signed_field_compatible() {
        let presentation =
            new_listing_presentation("Village".into(), WorldVisibilityV1::Public, Some(" Middle-East "), Some("AR"))
                .unwrap();
        assert_eq!(presentation.approximate_region.as_deref(), Some("middle-east"));
        assert_eq!(presentation.tags, ["lang:ar"]);
        assert!(new_listing_presentation("Private".into(), WorldVisibilityV1::Private, None, Some("en")).is_err());
        assert!(listing_region("25.123,55.123").is_err());
        assert!(listing_language_tag("ar:secret").is_err());
        assert!(listing_language_tag("en-US").is_err());
        assert!(new_listing_presentation("Private".into(), WorldVisibilityV1::Private, None, None)
            .unwrap()
            .tags
            .is_empty());
    }
}
