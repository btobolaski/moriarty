//! Display-side cost types for the API pricing report.
//!
//! After the migration to `cost_analyzer`, moriarty no longer prices token
//! counts locally. The actual pricing math lives in `cost_analyzer`'s
//! `ClaudeModelPricing`. This module keeps only what the report tables need:
//!
//! - [`ModelType`] — the four-bucket display grouping plus `Unknown`,
//! - [`TokenCosts`] — already-priced cost components (input / output / cache),
//! - [`ModelCostsMap`] — accumulator keyed by `ModelType` for grouped tables.
//!
//! Aggregation paths receive `cost_analyzer::LineWithCost.cost` values,
//! convert them to `TokenCosts`, and add them into a `ModelCostsMap` via
//! `ModelCostsMap::add`.

use std::{collections::HashMap, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelType {
    Sonnet,
    Haiku,
    Opus,
    Opus4,
    Unknown,
}

impl ModelType {
    /// Display order for cost reporting: highest-cost models first.
    pub const DISPLAY_ORDER: [(Self, &'static str); 4] = [
        (Self::Opus4, "Opus 4"),
        (Self::Opus, "Opus"),
        (Self::Sonnet, "Sonnet"),
        (Self::Haiku, "Haiku"),
    ];

    /// Detect model type from the model string for display grouping only.
    ///
    /// Pricing is performed in `cost_analyzer`; this function exists solely so
    /// the report can place each priced cost into its display bucket.
    ///
    /// # Matching Rules
    /// - "sonnet" (case-insensitive) → Sonnet
    /// - "haiku" (case-insensitive) → Haiku
    /// - "opus-4" (case-insensitive) → Opus4 (also catches "opus-4-5", "opus-45", etc.)
    /// - "opus" (case-insensitive) → Opus (for Opus 3)
    /// - Everything else → Unknown
    ///
    /// # Examples
    /// ```
    /// # use moriarty::api_pricing::pricing::ModelType;
    /// assert_eq!(ModelType::from_model_string("claude-sonnet-4-20250514"), ModelType::Sonnet);
    /// assert_eq!(ModelType::from_model_string("claude-3-haiku-20240307"), ModelType::Haiku);
    /// assert_eq!(ModelType::from_model_string("claude-opus-4"), ModelType::Opus4);
    /// assert_eq!(ModelType::from_model_string("claude-opus-4-5"), ModelType::Opus4);
    /// assert_eq!(ModelType::from_model_string("claude-3-opus-20240229"), ModelType::Opus);
    /// assert_eq!(ModelType::from_model_string("gpt-4"), ModelType::Unknown);
    /// ```
    pub fn from_model_string(model: &str) -> Self {
        let model_lower = model.to_lowercase();
        if model_lower.contains("sonnet") {
            Self::Sonnet
        } else if model_lower.contains("haiku") {
            Self::Haiku
        } else if model_lower.contains("opus-4") {
            // Check Opus 4 BEFORE the general opus check: every opus-4 model string
            // also contains "opus", so reversing these arms would misclassify Opus 4 as Opus 3.
            Self::Opus4
        } else if model_lower.contains("opus") {
            Self::Opus
        } else {
            Self::Unknown
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            Self::Sonnet => "Sonnet",
            Self::Haiku => "Haiku",
            Self::Opus => "Opus",
            Self::Opus4 => "Opus 4",
            Self::Unknown => "Unknown",
        }
    }
}

impl fmt::Display for ModelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// Already-priced cost components for a single model bucket.
///
/// All fields are in dollars and produced upstream by `cost_analyzer`'s
/// pricing tables; moriarty only sums them.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenCosts {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
}

impl TokenCosts {
    pub fn new(input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        Self {
            input,
            output,
            cache_write,
            cache_read,
        }
    }

    pub fn total(&self) -> f64 {
        self.input + self.output + self.cache_write + self.cache_read
    }

    /// Returns `(input, output, cache_write, cache_read)` so report-side row
    /// builders can pass cost components positionally without re-listing each
    /// field name at every construction site.
    pub fn as_components(&self) -> (f64, f64, f64, f64) {
        (self.input, self.output, self.cache_write, self.cache_read)
    }

    /// Adds another set of cost components into this one, in place.
    ///
    /// Used by `ModelCostsMap::add` to accumulate already-priced costs that
    /// arrive line-by-line from `cost_analyzer`. Token counts are intentionally
    /// not involved here; this path operates on already-computed cost amounts.
    pub fn add(&mut self, other: &TokenCosts) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
    }
}

/// Stores accumulated costs by model family for grouped report rendering.
#[derive(Debug, Clone, Default)]
pub struct ModelCostsMap {
    costs: HashMap<ModelType, TokenCosts>,
}

impl ModelCostsMap {
    /// Accumulates `costs` into the entry for `model_type`, summing components.
    ///
    /// This is the single entry point used by aggregation when consuming
    /// `cost_analyzer::LineWithCost` values: callers convert each line's
    /// `LlmCost` into `TokenCosts` and add it here without re-running
    /// per-token pricing.
    pub fn add(&mut self, model_type: ModelType, costs: TokenCosts) {
        self.costs.entry(model_type).or_default().add(&costs);
    }

    /// Returns costs for `model_type`, or zero-default if absent.
    #[cfg(test)]
    pub fn get(&self, model_type: ModelType) -> TokenCosts {
        self.costs.get(&model_type).copied().unwrap_or_default()
    }

    pub fn total(&self) -> f64 {
        self.costs.values().map(TokenCosts::total).sum()
    }

    fn get_or_default(&self, model_type: ModelType) -> TokenCosts {
        self.costs.get(&model_type).copied().unwrap_or_default()
    }

    /// Returns the four display-order buckets (Opus 4, Opus, Sonnet, Haiku),
    /// zero-filling any model that has no accumulated costs.
    pub fn model_costs(&self) -> [(&'static str, TokenCosts); 4] {
        ModelType::DISPLAY_ORDER.map(|(model_type, name)| (name, self.get_or_default(model_type)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_type_from_string_sonnet_variations() {
        assert_eq!(
            ModelType::from_model_string("claude-sonnet-4"),
            ModelType::Sonnet
        );
        assert_eq!(ModelType::from_model_string("SONNET"), ModelType::Sonnet);
        assert_eq!(
            ModelType::from_model_string("Claude-Sonnet-3.5"),
            ModelType::Sonnet
        );
    }

    #[test]
    fn model_type_from_string_haiku_variations() {
        assert_eq!(
            ModelType::from_model_string("claude-haiku-3"),
            ModelType::Haiku
        );
        assert_eq!(ModelType::from_model_string("HAIKU"), ModelType::Haiku);
    }

    #[test]
    fn model_type_from_string_opus_variations() {
        assert_eq!(
            ModelType::from_model_string("claude-3-opus-20240229"),
            ModelType::Opus
        );
        assert_eq!(ModelType::from_model_string("OPUS"), ModelType::Opus);
    }

    #[test]
    fn model_type_from_string_opus4_variations() {
        // Opus 4 must win against the more general Opus arm; this guards the
        // ordering inside `from_model_string`.
        assert_eq!(
            ModelType::from_model_string("claude-opus-4"),
            ModelType::Opus4
        );
        assert_eq!(
            ModelType::from_model_string("claude-opus-4-5"),
            ModelType::Opus4
        );
        assert_eq!(
            ModelType::from_model_string("CLAUDE-OPUS-4-20250514"),
            ModelType::Opus4
        );
        assert_eq!(
            ModelType::from_model_string("claude-opus-45"),
            ModelType::Opus4
        );
    }

    #[test]
    fn model_type_from_string_unknown() {
        assert_eq!(ModelType::from_model_string("gpt-4"), ModelType::Unknown);
        assert_eq!(ModelType::from_model_string(""), ModelType::Unknown);
    }

    #[test]
    fn model_type_display_matches_variant() {
        assert_eq!(ModelType::Sonnet.to_string(), "Sonnet");
        assert_eq!(ModelType::Haiku.to_string(), "Haiku");
        assert_eq!(ModelType::Opus.to_string(), "Opus");
        assert_eq!(ModelType::Opus4.to_string(), "Opus 4");
        assert_eq!(ModelType::Unknown.to_string(), "Unknown");
    }

    #[test]
    fn token_costs_total_sums_components() {
        let costs = TokenCosts::new(1.5, 2.5, 0.5, 0.25);

        assert!((costs.total() - 4.75).abs() < 1e-10);
    }

    #[test]
    fn token_costs_add_accumulates_each_component() {
        let mut costs = TokenCosts::new(1.0, 2.0, 0.5, 0.25);
        let other = TokenCosts::new(0.5, 1.0, 0.25, 0.1);

        costs.add(&other);

        assert!((costs.input - 1.5).abs() < 1e-10);
        assert!((costs.output - 3.0).abs() < 1e-10);
        assert!((costs.cache_write - 0.75).abs() < 1e-10);
        assert!((costs.cache_read - 0.35).abs() < 1e-10);
    }

    #[test]
    fn model_costs_map_add_accumulates_per_bucket() {
        let mut map = ModelCostsMap::default();
        map.add(ModelType::Sonnet, TokenCosts::new(1.0, 2.0, 0.0, 0.0));
        map.add(ModelType::Sonnet, TokenCosts::new(0.5, 0.5, 0.25, 0.0));

        let sonnet = map.get(ModelType::Sonnet);
        assert!((sonnet.input - 1.5).abs() < 1e-10);
        assert!((sonnet.output - 2.5).abs() < 1e-10);
        assert!((sonnet.cache_write - 0.25).abs() < 1e-10);
    }

    #[test]
    fn model_costs_map_get_absent_returns_default() {
        let costs = ModelCostsMap::default();
        assert_eq!(costs.get(ModelType::Sonnet).total(), 0.0);
    }

    #[test]
    fn model_costs_map_total_sums_all_entries() {
        let mut costs = ModelCostsMap::default();
        costs.add(ModelType::Sonnet, TokenCosts::new(1.0, 2.0, 0.0, 0.0));
        costs.add(ModelType::Haiku, TokenCosts::new(0.5, 1.0, 0.0, 0.0));

        assert!((costs.total() - 4.5).abs() < 1e-10);
    }

    #[test]
    fn model_costs_map_model_costs_zero_fills_absent_entries() {
        let costs = ModelCostsMap::default();
        let entries = costs.model_costs();

        assert_eq!(entries.len(), 4);
        for (_, model_costs) in &entries {
            assert_eq!(model_costs.total(), 0.0);
        }
    }

    #[test]
    fn model_costs_map_model_costs_returns_display_order() {
        let entries = ModelCostsMap::default().model_costs();

        let names: Vec<&str> = entries.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, vec!["Opus 4", "Opus", "Sonnet", "Haiku"]);
    }
}
