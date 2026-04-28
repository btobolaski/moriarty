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
    pub const DISPLAY_ORDER: [(Self, &'static str); 4] = [
        (Self::Opus4, "Opus 4"),
        (Self::Opus, "Opus"),
        (Self::Sonnet, "Sonnet"),
        (Self::Haiku, "Haiku"),
    ];

    /// Report rows intentionally collapse concrete Claude model ids into the
    /// four stable buckets shown in the table.
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

    pub fn as_components(&self) -> (f64, f64, f64, f64) {
        (self.input, self.output, self.cache_write, self.cache_read)
    }

    pub fn add(&mut self, other: &TokenCosts) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
    }
}

#[derive(Debug, Clone, Default)]
pub struct ModelCostsMap {
    costs: HashMap<ModelType, TokenCosts>,
}

impl ModelCostsMap {
    /// Unknown Claude models are dropped here because `cost_analyzer` already
    /// logged the pricing problem upstream and the report only has stable rows
    /// for the four named display buckets.
    pub fn add(&mut self, model_type: ModelType, costs: TokenCosts) {
        if model_type == ModelType::Unknown {
            return;
        }

        self.costs.entry(model_type).or_default().add(&costs);
    }

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
    fn model_costs_map_ignores_unknown_buckets() {
        let mut costs = ModelCostsMap::default();
        costs.add(ModelType::Unknown, TokenCosts::new(9.0, 8.0, 7.0, 6.0));

        assert_eq!(costs.total(), 0.0);
        assert_eq!(costs.model_costs().len(), 4);
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
