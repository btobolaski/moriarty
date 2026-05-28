//! Structured view of the raw `model` string carried by Claude log entries.
//!
//! Both `cost_analyzer` (for pricing) and `moriarty::api_pricing` (for
//! grouping and display) need to recognize Claude model families and versions
//! from the same `claude-…` identifiers. Centralizing the parser here keeps
//! the two consumers in sync without each maintaining its own copy.

use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModelFamily {
    Sonnet,
    Haiku,
    Opus,
    /// Claude Code's `<synthetic>` placeholder for harness-fabricated
    /// assistant turns that don't reach a real model. Carrying this as a real
    /// family variant lets the pricing layer skip it via the same match arm
    /// it uses for the billable families, instead of a separate substring
    /// check on the raw id.
    Synthetic,
}

impl ModelFamily {
    /// `Synthetic` returns `None` because the `<synthetic>` sentinel is
    /// detected by exact match in `from_model_string`, not by the keyword
    /// scan that this method feeds.
    fn keyword(self) -> Option<&'static str> {
        match self {
            Self::Sonnet => Some("sonnet"),
            Self::Haiku => Some("haiku"),
            Self::Opus => Some("opus"),
            Self::Synthetic => None,
        }
    }

    /// Production logs always carry a version, so the no-version branch of
    /// `Display` (the only caller) fires almost exclusively for the test-only
    /// `Model::family(...)` constructor and bare-family aliases like `"OPUS"`.
    fn label(self) -> &'static str {
        match self {
            Self::Sonnet => "Sonnet",
            Self::Haiku => "Haiku",
            Self::Opus => "Opus",
            Self::Synthetic => "<synthetic>",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModelVersion {
    pub major: u32,
    pub minor: Option<u32>,
}

/// Parsed view of a Claude `model` field. Carries the raw id so callers can
/// surface it in diagnostics, but the family and version are pre-computed so
/// downstream pricing and grouping never re-parse the string.
#[derive(Debug, Clone)]
pub struct Model {
    pub family: ModelFamily,
    pub version: Option<ModelVersion>,
    raw: String,
}

impl Model {
    /// Constructor that builds a `{family, version: None}` value. Production
    /// code reaches `Model` via `from_model_string`; this exists so unit tests
    /// in downstream crates can construct a bare-family bucket without an
    /// inline model id. The `raw` field is left empty — these instances are
    /// for grouping comparisons only, not for round-trip serialization.
    pub fn family(family: ModelFamily) -> Self {
        Self {
            family,
            version: None,
            raw: String::new(),
        }
    }

    /// Returns `Err` for ids that match no known family — including future
    /// Anthropic releases — so a new model surfaces as a loud parse failure
    /// (matching the strict-by-default policy elsewhere in the parser)
    /// instead of silently dropping out of cost reports. Pricing tiers within
    /// a family (e.g. Opus 3 vs Opus 4.x) are resolved later, by
    /// `ClaudeModelPricing::for_model`, from the parsed `version`.
    pub fn from_model_string(raw: impl Into<String>) -> Result<Self, UnknownModelError> {
        let raw = raw.into();
        let model_lower = raw.to_lowercase();

        // `<synthetic>` doesn't follow the `claude-…` substring pattern and
        // has no version to parse, so handle it before the keyword scan.
        if model_lower == "<synthetic>" {
            return Ok(Self {
                family: ModelFamily::Synthetic,
                version: None,
                raw,
            });
        }

        let family = match classify_family(&model_lower) {
            Some(family) => family,
            // Move `raw` into the error on the failure path so we don't pay
            // for a clone on the happy path either.
            None => return Err(UnknownModelError { raw }),
        };
        let version = parse_version(&model_lower, family);
        Ok(Self {
            family,
            version,
            raw,
        })
    }

    /// Preserved so callers can surface the exact wire string in diagnostics
    /// when `family` / `version` collapse two different ids into the same
    /// bucket.
    pub fn raw(&self) -> &str {
        &self.raw
    }
}

#[derive(Debug, Clone, PartialEq, Eq, miette::Diagnostic, thiserror::Error)]
#[error("unrecognized Claude model id `{raw}` — update `claude_logs::ModelFamily` if a new family has shipped")]
pub struct UnknownModelError {
    pub raw: String,
}

// `PartialEq` / `Eq` / `Hash` deliberately skip `raw`: dedup in
// `cost_analyzer::deduplicate_lines` and grouping in
// `moriarty::api_pricing::ModelMetricsMap` key on family + version so that
// two ids whose only difference is a release-date suffix
// (e.g. `claude-sonnet-4-20250514` vs `claude-sonnet-4-20250620`) collapse
// into the same bucket. Including `raw` would let those keys diverge.
impl PartialEq for Model {
    fn eq(&self, other: &Self) -> bool {
        self.family == other.family && self.version == other.version
    }
}

impl Eq for Model {}

impl Hash for Model {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.family.hash(state);
        self.version.hash(state);
    }
}

impl PartialOrd for Model {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// `Ord` mirrors `PartialEq` / `Hash`: it skips `raw` so the dedup HashMap's
// `Identifier` bound (which requires `Ord`) produces a stable order whose
// equality matches `==`.
impl Ord for Model {
    fn cmp(&self, other: &Self) -> Ordering {
        self.family
            .cmp(&other.family)
            .then_with(|| self.version.cmp(&other.version))
    }
}

impl Serialize for Model {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Emit the original id so JSON round-trips through deserialize +
        // serialize without losing the date suffix or any other detail the
        // parser is lossy about.
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for Model {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_model_string(raw).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.version {
            None => f.write_str(self.family.label()),
            Some(ModelVersion { major, minor: None }) => {
                write!(f, "{} {}", self.family.label(), major)
            }
            Some(ModelVersion {
                major,
                minor: Some(minor),
            }) => write!(f, "{} {}.{}", self.family.label(), major, minor),
        }
    }
}

fn classify_family(model_lower: &str) -> Option<ModelFamily> {
    // Sonnet runs first because ids containing both "sonnet" and "opus"
    // (e.g. `claude-opus-4-sonnet-preview`) must classify as Sonnet — the
    // experimental preview's substantive family is the trailing word.
    if model_lower.contains("sonnet") {
        Some(ModelFamily::Sonnet)
    } else if model_lower.contains("haiku") {
        Some(ModelFamily::Haiku)
    } else if model_lower.contains("opus") {
        // Opus 3 and Opus 4.x share the same family; the version major
        // drives the pricing tier in `ClaudeModelPricing::for_model`.
        Some(ModelFamily::Opus)
    } else {
        None
    }
}

fn parse_version(model_lower: &str, family: ModelFamily) -> Option<ModelVersion> {
    let keyword = family.keyword()?;
    let tokens: Vec<&str> = model_lower.split('-').collect();
    let family_idx = tokens.iter().position(|t| *t == keyword)?;

    // Modern naming puts the version AFTER the family token
    // (`claude-sonnet-4-5-...`, `claude-opus-4-1-...`); prefer it because the
    // legacy `claude-3-5-haiku` form is the falsy case.
    let after = collect_adjacent_digit_tokens(tokens[family_idx + 1..].iter().copied());
    if let Some(version) = version_from(after) {
        return Some(version);
    }

    // Legacy naming puts the version BEFORE the family token
    // (`claude-3-5-sonnet-...`, `claude-3-haiku-...`); scan leftward from the
    // family then restore left-to-right order for the major/minor split.
    let mut before = collect_adjacent_digit_tokens(tokens[..family_idx].iter().rev().copied());
    if !before.is_empty() {
        before.reverse();
        return version_from(before);
    }

    None
}

fn version_from(tokens: Vec<u32>) -> Option<ModelVersion> {
    let major = *tokens.first()?;
    Some(ModelVersion {
        major,
        minor: tokens.get(1).copied(),
    })
}

/// Collects up to two adjacent digit tokens, stopping at the first non-digit
/// token or at a date-shaped stamp (length > 2 or value > 99). The
/// length / value gate is what rejects `20250929` from being misread as a
/// version number. Two-digit numeric tokens like `45` survive the gate and
/// are taken as a single major version (so `claude-opus-45` displays as
/// "Opus 45"). That collapsed form is unconventional but appears in test
/// fixtures, and rendering it as the literal token surfaces malformed input
/// instead of silently splitting it into `4.5`.
fn collect_adjacent_digit_tokens<'a, I: Iterator<Item = &'a str>>(iter: I) -> Vec<u32> {
    let mut out = Vec::with_capacity(2);
    for token in iter {
        if token.is_empty() {
            break;
        }
        let Ok(value) = token.parse::<u32>() else {
            break;
        };
        if token.len() > 2 || value > 99 {
            break;
        }
        out.push(value);
        if out.len() == 2 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(id: &str) -> Model {
        Model::from_model_string(id).unwrap_or_else(|err| panic!("parse {id:?}: {err}"))
    }

    #[test]
    fn from_model_string_classifies_sonnet_family() {
        assert_eq!(parse("claude-sonnet-4").family, ModelFamily::Sonnet);
        assert_eq!(parse("SONNET").family, ModelFamily::Sonnet);
        assert_eq!(parse("Claude-Sonnet-3-5").family, ModelFamily::Sonnet);
    }

    #[test]
    fn from_model_string_classifies_haiku_family() {
        assert_eq!(parse("claude-haiku-3").family, ModelFamily::Haiku);
        assert_eq!(parse("HAIKU").family, ModelFamily::Haiku);
    }

    #[test]
    fn from_model_string_classifies_all_opus_versions_as_opus_family() {
        // Opus 3 and Opus 4.x share `ModelFamily::Opus`; the pricing tier
        // (OPUS vs OPUS_4) is resolved later from the parsed version major.
        for id in [
            "claude-3-opus-20240229",
            "claude-opus-4",
            "claude-opus-4-5",
            "CLAUDE-OPUS-4-20250514",
            "claude-opus-45",
        ] {
            assert_eq!(parse(id).family, ModelFamily::Opus, "id {id:?}");
        }
    }

    #[test]
    fn from_model_string_classifies_opus4_sonnet_preview_as_sonnet() {
        // The Sonnet arm runs first in `classify_family`, so this experimental
        // id classifies by its "sonnet" substring even though it also contains
        // "opus-4".
        assert_eq!(
            parse("claude-opus-4-sonnet-preview").family,
            ModelFamily::Sonnet
        );
    }

    #[test]
    fn from_model_string_classifies_synthetic_sentinel() {
        let model = parse("<synthetic>");
        assert_eq!(model.family, ModelFamily::Synthetic);
        assert_eq!(model.version, None);
        assert_eq!(model.raw(), "<synthetic>");
    }

    #[test]
    fn from_model_string_errors_for_ids_with_no_matching_family() {
        // Strict-by-default parsing surfaces unknown ids as `Err` so a new
        // Anthropic family release (or a non-Claude id slipping into a Claude
        // log) becomes a loud parse failure instead of silent miscounting.
        for id in ["gpt-4", "", "claude-mythos-1-20260301"] {
            let err = Model::from_model_string(id).unwrap_err();
            assert_eq!(err.raw, id, "id {id:?}");
        }
    }

    #[test]
    fn from_model_string_parses_modern_after_family_version() {
        let cases = [
            ("claude-sonnet-4-5-20250929", 4, Some(5)),
            ("claude-sonnet-4-20250514", 4, None),
            ("claude-opus-4", 4, None),
            ("claude-opus-4-1-20250805", 4, Some(1)),
            ("claude-opus-4-5", 4, Some(5)),
            ("claude-opus-4-7", 4, Some(7)),
            ("claude-haiku-4-5", 4, Some(5)),
            // The collapsed `45` form is treated as a single 2-digit major;
            // see the doc comment on `collect_adjacent_digit_tokens` for why
            // we don't split it into 4.5.
            ("claude-opus-45", 45, None),
        ];
        for (id, major, minor) in cases {
            assert_eq!(
                parse(id).version,
                Some(ModelVersion { major, minor }),
                "id {id:?}"
            );
        }
    }

    #[test]
    fn from_model_string_parses_legacy_before_family_version() {
        let cases = [
            ("claude-3-5-sonnet-20241022", 3, Some(5)),
            ("claude-3-haiku-20240307", 3, None),
            ("claude-3-5-haiku-20241022", 3, Some(5)),
            ("claude-3-opus-20240229", 3, None),
        ];
        for (id, major, minor) in cases {
            assert_eq!(
                parse(id).version,
                Some(ModelVersion { major, minor }),
                "id {id:?}"
            );
        }
    }

    #[test]
    fn from_model_string_rejects_date_stamps_as_versions() {
        // The 8-digit date suffix must not be misread as a version. Sonnet 4
        // with no minor still parses cleanly when the only "after" token is a
        // date.
        assert_eq!(
            parse("claude-sonnet-4-20250514").version,
            Some(ModelVersion {
                major: 4,
                minor: None
            })
        );
    }

    #[test]
    fn from_model_string_returns_no_version_for_bare_family_word() {
        for id in ["SONNET", "HAIKU", "OPUS"] {
            let parsed = parse(id);
            assert_eq!(parsed.version, None, "id {id:?}");
        }
    }

    #[test]
    fn from_model_string_leaves_malformed_opus_id_without_version() {
        // `claude-opus-4ab` has no parseable digit token adjacent to "opus";
        // the family still classifies as Opus, but the version stays `None`
        // and Display falls back to the bare family label ("Opus").
        let parsed = parse("claude-opus-4ab");
        assert_eq!(parsed.family, ModelFamily::Opus);
        assert_eq!(parsed.version, None);
        assert_eq!(parsed.to_string(), "Opus");
    }

    #[test]
    fn serde_roundtrip_preserves_raw_id() {
        // Display is lossy (drops the date suffix), but `Serialize` writes the
        // stored `raw` so reserializing yields the original wire string.
        let parsed = parse("claude-sonnet-4-5-20250929");
        let json = serde_json::to_string(&parsed).unwrap();
        assert_eq!(json, "\"claude-sonnet-4-5-20250929\"");

        let roundtripped: Model = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped, parsed);
        assert_eq!(roundtripped.raw(), "claude-sonnet-4-5-20250929");
    }

    #[test]
    fn serde_deserialize_surfaces_unknown_model_error() {
        // Strict-by-default parsing means an unknown id in a wire message
        // becomes a deserialization error naming the offending raw string,
        // not a silent fallback.
        let err = serde_json::from_str::<Model>("\"claude-mythos-1\"").unwrap_err();
        assert!(
            err.to_string().contains("claude-mythos-1"),
            "deserialize error should name the raw id: {err}"
        );
    }

    #[test]
    fn eq_and_hash_ignore_raw_difference() {
        use std::collections::hash_map::DefaultHasher;

        // The Sonnet 4 family with no minor: two distinct raw strings should
        // collapse into the same bucket key because dedup and grouping must
        // ignore date-suffix-only differences.
        let earlier = parse("claude-sonnet-4-20250514");
        let later = parse("claude-sonnet-4-20250620");
        assert_eq!(earlier, later);
        let mut earlier_hasher = DefaultHasher::new();
        earlier.hash(&mut earlier_hasher);
        let mut later_hasher = DefaultHasher::new();
        later.hash(&mut later_hasher);
        assert_eq!(earlier_hasher.finish(), later_hasher.finish());
        // raw is still observable; only eq/hash skip it.
        assert_ne!(earlier.raw(), later.raw());
    }

    #[test]
    fn display_combines_family_and_version() {
        let cases: Vec<(Model, &str)> = vec![
            (parse("claude-sonnet-4-5"), "Sonnet 4.5"),
            (parse("claude-sonnet-4-20250514"), "Sonnet 4"),
            (parse("claude-3-5-sonnet-20241022"), "Sonnet 3.5"),
            (parse("claude-3-haiku-20240307"), "Haiku 3"),
            (parse("claude-haiku-4-5"), "Haiku 4.5"),
            (parse("claude-3-opus-20240229"), "Opus 3"),
            (parse("claude-opus-4"), "Opus 4"),
            (parse("claude-opus-4-20250514"), "Opus 4"),
            (parse("claude-opus-4-5"), "Opus 4.5"),
            (parse("claude-opus-4-7"), "Opus 4.7"),
            (parse("claude-opus-45"), "Opus 45"),
            (parse("SONNET"), "Sonnet"),
            (parse("OPUS"), "Opus"),
            (parse("<synthetic>"), "<synthetic>"),
            (Model::family(ModelFamily::Haiku), "Haiku"),
            (Model::family(ModelFamily::Opus), "Opus"),
            (Model::family(ModelFamily::Synthetic), "<synthetic>"),
        ];
        for (model, expected) in cases {
            assert_eq!(model.to_string(), expected, "model {model:?}");
        }
    }
}
