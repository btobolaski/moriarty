use std::{collections::HashMap, fmt};

#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    /// Price per million input tokens
    pub input: f64,
    /// Price per million output tokens
    pub output: f64,
    /// Price per million prompt cache write tokens
    pub cache_write: f64,
    /// Price per million prompt cache read tokens
    pub cache_read: f64,
}

impl ModelPricing {
    /// Pricing for Sonnet models (effective as of 2025-10-23)
    /// Applies to: claude-sonnet-4-*, claude-3-5-sonnet-*
    pub const SONNET: Self = Self {
        input: 3.0,
        output: 15.0,
        cache_write: 3.75,
        cache_read: 0.30,
    };

    /// Pricing for Haiku models (effective as of 2025-10-23)
    /// Applies to: claude-haiku-*, claude-3-*-haiku-*
    pub const HAIKU: Self = Self {
        input: 1.0,
        output: 5.0,
        cache_write: 1.25,
        cache_read: 0.1,
    };

    /// Pricing for Opus 3 models (effective as of 2025-10-23)
    /// Applies to: claude-3-*-opus-*
    pub const OPUS: Self = Self {
        input: 15.0,
        output: 75.0,
        cache_write: 18.75,
        cache_read: 1.5,
    };

    /// Pricing for Opus 4 models (effective as of 2024-11-15)
    /// Applies to: claude-opus-4*
    pub const OPUS_4: Self = Self {
        input: 5.0,
        output: 25.0,
        cache_write: 6.25,
        cache_read: 0.50,
    };

    /// Calculate the cost for the given token counts
    pub fn calculate_cost(&self, usage: &TokenCounts) -> TokenCosts {
        TokenCosts {
            input: (usage.input_tokens as f64 / 1_000_000.0) * self.input,
            output: (usage.output_tokens as f64 / 1_000_000.0) * self.output,
            cache_write: (usage.cache_write_tokens as f64 / 1_000_000.0) * self.cache_write,
            cache_read: (usage.cache_read_tokens as f64 / 1_000_000.0) * self.cache_read,
        }
    }
}

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

    /// Detect model type from the model string.
    ///
    /// Matches known Claude model families based on substring detection.
    /// This is intentionally simple to handle model version changes.
    ///
    /// # Matching Rules
    /// - "sonnet" (case-insensitive) → Sonnet
    /// - "haiku" (case-insensitive) → Haiku
    /// - "opus-4" or "opus-45" (case-insensitive) → Opus4
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
        } else if model_lower.contains("opus-4") || model_lower.contains("opus-45") {
            // Check for Opus 4 BEFORE general opus check since all opus-4 strings
            // also contain "opus". This ordering is critical for correct classification.
            Self::Opus4
        } else if model_lower.contains("opus") {
            // Matches Opus 3 models (e.g., claude-3-opus-20240229)
            Self::Opus
        } else {
            Self::Unknown
        }
    }

    /// Returns the model's pricing, when known, plus the display name used in reports.
    ///
    /// The tuple is `(pricing, display_name)`. Unknown models have no pricing and
    /// therefore return `(None, "Unknown")`.
    fn pricing_and_display_name(&self) -> (Option<ModelPricing>, &'static str) {
        match self {
            Self::Sonnet => (Some(ModelPricing::SONNET), "Sonnet"),
            Self::Haiku => (Some(ModelPricing::HAIKU), "Haiku"),
            Self::Opus => (Some(ModelPricing::OPUS), "Opus"),
            Self::Opus4 => (Some(ModelPricing::OPUS_4), "Opus 4"),
            Self::Unknown => (None, "Unknown"),
        }
    }

    pub fn pricing(&self) -> Option<ModelPricing> {
        self.pricing_and_display_name().0
    }
}

impl fmt::Display for ModelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.pricing_and_display_name().1)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TokenCounts {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cache_write_tokens: usize,
    pub cache_read_tokens: usize,
}

impl TokenCounts {
    /// Creates token counts from raw values: input, output, cache-write, and cache-read.
    pub fn new(
        input_tokens: usize,
        output_tokens: usize,
        cache_write_tokens: usize,
        cache_read_tokens: usize,
    ) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_write_tokens,
            cache_read_tokens,
        }
    }

    pub fn add(&mut self, other: &TokenCounts) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TokenCosts {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
}

impl TokenCosts {
    pub fn total(&self) -> f64 {
        self.input + self.output + self.cache_write + self.cache_read
    }
}

/// Accumulates token usage by model family.
#[derive(Debug, Clone, Default)]
pub struct ModelUsageMap {
    counts: HashMap<ModelType, TokenCounts>,
}

impl ModelUsageMap {
    pub fn add(&mut self, model_type: ModelType, counts: TokenCounts) {
        self.counts.entry(model_type).or_default().add(&counts);
    }

    /// Returns counts for `model_type`, or zero-default if absent.
    #[cfg(test)]
    pub fn get(&self, model_type: ModelType) -> TokenCounts {
        self.counts.get(&model_type).copied().unwrap_or_default()
    }

    pub fn calculate_costs(&self) -> ModelCostsMap {
        let mut costs = ModelCostsMap::default();

        for (model_type, counts) in &self.counts {
            if let Some(pricing) = model_type.pricing() {
                costs.set(*model_type, pricing.calculate_cost(counts));
            }
        }

        costs
    }
}

/// Stores calculated costs by model family.
#[derive(Debug, Clone, Default)]
pub struct ModelCostsMap {
    costs: HashMap<ModelType, TokenCosts>,
}

impl ModelCostsMap {
    pub fn set(&mut self, model_type: ModelType, costs: TokenCosts) {
        self.costs.insert(model_type, costs);
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

    pub fn model_costs(&self) -> [(&'static str, TokenCosts); 4] {
        ModelType::DISPLAY_ORDER
            .map(|(model_type, name)| (name, self.get_or_default(model_type)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sonnet_pricing_constants() {
        let pricing = ModelPricing::SONNET;
        assert_eq!(pricing.input, 3.0);
        assert_eq!(pricing.output, 15.0);
        assert_eq!(pricing.cache_write, 3.75);
        assert_eq!(pricing.cache_read, 0.30);
    }

    #[test]
    fn test_haiku_pricing_constants() {
        let pricing = ModelPricing::HAIKU;
        assert_eq!(pricing.input, 1.0);
        assert_eq!(pricing.output, 5.0);
        assert_eq!(pricing.cache_write, 1.25);
        assert_eq!(pricing.cache_read, 0.1);
    }

    #[test]
    fn test_opus_pricing_constants() {
        let pricing = ModelPricing::OPUS;
        assert_eq!(pricing.input, 15.0);
        assert_eq!(pricing.output, 75.0);
        assert_eq!(pricing.cache_write, 18.75);
        assert_eq!(pricing.cache_read, 1.5);
    }

    #[test]
    fn test_opus4_pricing_constants() {
        let pricing = ModelPricing::OPUS_4;
        assert_eq!(pricing.input, 5.0);
        assert_eq!(pricing.output, 25.0);
        assert_eq!(pricing.cache_write, 6.25);
        assert_eq!(pricing.cache_read, 0.50);
    }

    #[test]
    fn test_calculate_cost_zero_tokens() {
        let pricing = ModelPricing::SONNET;
        let usage = TokenCounts::default();
        let costs = pricing.calculate_cost(&usage);

        assert_eq!(costs.input, 0.0);
        assert_eq!(costs.output, 0.0);
        assert_eq!(costs.cache_write, 0.0);
        assert_eq!(costs.cache_read, 0.0);
        assert_eq!(costs.total(), 0.0);
    }

    #[test]
    fn test_calculate_cost_one_million_tokens() {
        let pricing = ModelPricing::SONNET;
        let usage = TokenCounts {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_write_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
        };
        let costs = pricing.calculate_cost(&usage);

        assert_eq!(costs.input, 3.0);
        assert_eq!(costs.output, 15.0);
        assert_eq!(costs.cache_write, 3.75);
        assert_eq!(costs.cache_read, 0.30);
        assert_eq!(costs.total(), 22.05);
    }

    #[test]
    fn test_calculate_cost_fractional_tokens() {
        let pricing = ModelPricing::HAIKU;
        let usage = TokenCounts {
            input_tokens: 500,
            output_tokens: 1_000,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
        };
        let costs = pricing.calculate_cost(&usage);

        assert!((costs.input - 0.0005).abs() < 1e-10);
        assert!((costs.output - 0.005).abs() < 1e-10);
        assert!((costs.total() - 0.0055).abs() < 1e-10);
    }

    #[test]
    fn test_calculate_cost_opus_one_million_tokens() {
        let pricing = ModelPricing::OPUS;
        let usage = TokenCounts {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_write_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
        };
        let costs = pricing.calculate_cost(&usage);

        assert_eq!(costs.input, 15.0);
        assert_eq!(costs.output, 75.0);
        assert_eq!(costs.cache_write, 18.75);
        assert_eq!(costs.cache_read, 1.5);
        assert_eq!(costs.total(), 110.25);
    }

    #[test]
    fn test_calculate_cost_opus4_one_million_tokens() {
        let pricing = ModelPricing::OPUS_4;
        let usage = TokenCounts {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_write_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
        };
        let costs = pricing.calculate_cost(&usage);

        assert_eq!(costs.input, 5.0);
        assert_eq!(costs.output, 25.0);
        assert_eq!(costs.cache_write, 6.25);
        assert_eq!(costs.cache_read, 0.50);
        assert_eq!(costs.total(), 36.75);
    }

    #[test]
    fn test_model_type_from_string_sonnet_variations() {
        assert_eq!(
            ModelType::from_model_string("claude-sonnet-4"),
            ModelType::Sonnet
        );
        assert_eq!(ModelType::from_model_string("SONNET"), ModelType::Sonnet);
        assert_eq!(
            ModelType::from_model_string("Claude-Sonnet-3.5"),
            ModelType::Sonnet
        );
        assert_eq!(ModelType::from_model_string("sonnet"), ModelType::Sonnet);
    }

    #[test]
    fn test_model_type_from_string_haiku_variations() {
        assert_eq!(
            ModelType::from_model_string("claude-haiku-3"),
            ModelType::Haiku
        );
        assert_eq!(ModelType::from_model_string("HAIKU"), ModelType::Haiku);
        assert_eq!(
            ModelType::from_model_string("Claude-Haiku-3.5"),
            ModelType::Haiku
        );
        assert_eq!(ModelType::from_model_string("haiku"), ModelType::Haiku);
    }

    #[test]
    fn test_model_type_from_string_opus_variations() {
        assert_eq!(
            ModelType::from_model_string("claude-3-opus-20240229"),
            ModelType::Opus
        );
        assert_eq!(ModelType::from_model_string("OPUS"), ModelType::Opus);
        assert_eq!(
            ModelType::from_model_string("Claude-Opus-3.5"),
            ModelType::Opus
        );
        assert_eq!(ModelType::from_model_string("opus"), ModelType::Opus);
    }

    #[test]
    fn test_model_type_from_string_opus4_variations() {
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
    fn test_model_type_from_string_unknown() {
        assert_eq!(ModelType::from_model_string("gpt-4"), ModelType::Unknown);
        assert_eq!(ModelType::from_model_string(""), ModelType::Unknown);
    }

    #[test]
    fn test_model_type_pricing() {
        assert!(ModelType::Sonnet.pricing().is_some());
        assert!(ModelType::Haiku.pricing().is_some());
        assert!(ModelType::Opus.pricing().is_some());
        assert!(ModelType::Opus4.pricing().is_some());
        assert!(ModelType::Unknown.pricing().is_none());
    }

    #[test]
    fn test_token_counts_add() {
        let mut counts = TokenCounts {
            input_tokens: 100,
            output_tokens: 200,
            cache_write_tokens: 50,
            cache_read_tokens: 75,
        };

        let other = TokenCounts {
            input_tokens: 50,
            output_tokens: 100,
            cache_write_tokens: 25,
            cache_read_tokens: 10,
        };

        counts.add(&other);

        assert_eq!(counts.input_tokens, 150);
        assert_eq!(counts.output_tokens, 300);
        assert_eq!(counts.cache_write_tokens, 75);
        assert_eq!(counts.cache_read_tokens, 85);
    }

    #[test]
    fn test_token_costs_total() {
        let costs = TokenCosts {
            input: 1.5,
            output: 2.5,
            cache_write: 0.5,
            cache_read: 0.25,
        };

        assert!((costs.total() - 4.75).abs() < 1e-10);
    }

    #[test]
    fn test_model_type_display() {
        assert_eq!(format!("{}", ModelType::Sonnet), "Sonnet");
        assert_eq!(format!("{}", ModelType::Haiku), "Haiku");
        assert_eq!(format!("{}", ModelType::Opus), "Opus");
        assert_eq!(format!("{}", ModelType::Opus4), "Opus 4");
        assert_eq!(format!("{}", ModelType::Unknown), "Unknown");
    }

    #[test]
    fn test_model_usage_map_add_accumulates() {
        let mut map = ModelUsageMap::default();
        map.add(ModelType::Sonnet, TokenCounts::new(1000, 500, 0, 0));
        map.add(ModelType::Sonnet, TokenCounts::new(500, 250, 0, 0));

        let counts = map.get(ModelType::Sonnet);
        assert_eq!(counts.input_tokens, 1500);
        assert_eq!(counts.output_tokens, 750);
    }

    #[test]
    fn test_model_usage_map_get_absent_returns_default() {
        let map = ModelUsageMap::default();
        let counts = map.get(ModelType::Sonnet);

        assert_eq!(counts.input_tokens, 0);
        assert_eq!(counts.output_tokens, 0);
        assert_eq!(counts.cache_write_tokens, 0);
        assert_eq!(counts.cache_read_tokens, 0);
    }

    #[test]
    fn test_model_usage_map_calculate_costs_known_model() {
        let mut map = ModelUsageMap::default();
        map.add(
            ModelType::Sonnet,
            TokenCounts::new(1_000_000, 1_000_000, 0, 0),
        );

        let costs = map.calculate_costs();
        assert_eq!(costs.get(ModelType::Sonnet).input, 3.0);
        assert_eq!(costs.get(ModelType::Sonnet).output, 15.0);
        assert_eq!(costs.total(), 18.0);
    }

    #[test]
    fn test_model_usage_map_calculate_costs_unknown_produces_no_cost() {
        let mut map = ModelUsageMap::default();
        map.add(
            ModelType::Unknown,
            TokenCounts::new(1_000_000, 500_000, 0, 0),
        );

        let costs = map.calculate_costs();
        assert_eq!(costs.total(), 0.0);
        assert_eq!(costs.get(ModelType::Unknown).total(), 0.0);
    }

    #[test]
    fn test_model_costs_map_get_absent_returns_default() {
        let costs = ModelCostsMap::default();
        assert_eq!(costs.get(ModelType::Sonnet).total(), 0.0);
    }

    #[test]
    fn test_model_costs_map_total_sums_all_entries() {
        let mut costs = ModelCostsMap::default();
        costs.set(
            ModelType::Sonnet,
            TokenCosts {
                input: 1.0,
                output: 2.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
        );
        costs.set(
            ModelType::Haiku,
            TokenCosts {
                input: 0.5,
                output: 1.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
        );

        assert_eq!(costs.total(), 4.5);
    }

    #[test]
    fn test_model_costs_map_model_costs_zero_fills_absent_entries() {
        let costs = ModelCostsMap::default();
        let entries = costs.model_costs();

        assert_eq!(entries.len(), 4);
        for (_, model_costs) in &entries {
            assert_eq!(model_costs.total(), 0.0);
        }
    }
}
