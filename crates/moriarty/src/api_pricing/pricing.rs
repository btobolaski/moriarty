use std::{collections::HashMap, fmt};

use crate::cost_report::{MetricComponents, MetricTotal, ReportMode};

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

#[derive(Debug, Clone, Default)]
pub struct ModelMetricsMap {
    metrics: HashMap<ModelType, MetricComponents>,
}

impl ModelMetricsMap {
    /// Unknown Claude models are dropped here because `cost_analyzer` already
    /// logged the pricing problem upstream and the report only has stable rows
    /// for the four named display buckets.
    pub fn add(
        &mut self,
        model_type: ModelType,
        metrics: impl Into<MetricComponents>,
    ) -> miette::Result<()> {
        if model_type == ModelType::Unknown {
            return Ok(());
        }

        let metrics = metrics.into();

        match self.metrics.entry(model_type) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(metrics);
                Ok(())
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => entry
                .get_mut()
                .checked_add_assign(metrics)
                .map_err(|error| {
                    error.wrap_err(format!("failed to aggregate {model_type} metrics"))
                }),
        }
    }

    #[cfg(test)]
    pub fn get_metric(&self, model_type: ModelType, report_mode: ReportMode) -> MetricComponents {
        self.metrics
            .get(&model_type)
            .copied()
            .unwrap_or_else(|| MetricComponents::zero(report_mode))
    }

    pub fn total(&self, report_mode: ReportMode) -> miette::Result<MetricTotal> {
        self.metrics
            .values()
            .try_fold(MetricTotal::zero(report_mode), |acc, metrics| {
                acc.checked_add(metrics.total())
            })
    }

    fn get_or_default(&self, model_type: ModelType, report_mode: ReportMode) -> MetricComponents {
        self.metrics
            .get(&model_type)
            .copied()
            .unwrap_or_else(|| MetricComponents::zero(report_mode))
    }

    pub fn model_metrics(&self, report_mode: ReportMode) -> [(&'static str, MetricComponents); 4] {
        ModelType::DISPLAY_ORDER
            .map(|(model_type, name)| (name, self.get_or_default(model_type, report_mode)))
    }
}

#[cfg(test)]
impl ModelMetricsMap {
    pub fn get(&self, model_type: ModelType) -> crate::cost_report::CostComponents {
        match self.get_metric(model_type, ReportMode::Cost) {
            MetricComponents::Cost(costs) => costs,
            MetricComponents::Tokens(_) => unreachable!("cost tests requested token metrics"),
        }
    }

    pub fn get_tokens(&self, model_type: ModelType) -> crate::cost_report::TokenCounts {
        match self.get_metric(model_type, ReportMode::Tokens) {
            MetricComponents::Cost(_) => unreachable!("token tests requested cost metrics"),
            MetricComponents::Tokens(tokens) => tokens,
        }
    }

    pub fn model_costs(&self) -> [(&'static str, crate::cost_report::CostComponents); 4] {
        ModelType::DISPLAY_ORDER.map(|(model_type, name)| (name, self.get(model_type)))
    }
}

#[cfg(test)]
pub type ModelCostsMap = ModelMetricsMap;

#[cfg(test)]
mod tests {
    use crate::cost_report::{CostComponents, TokenCounts};

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
    fn model_metrics_map_add_accumulates_costs_per_bucket() {
        let mut map = ModelMetricsMap::default();
        map.add(
            ModelType::Sonnet,
            MetricComponents::Cost(CostComponents::new(1.0, 2.0, 0.0, 0.0)),
        )
        .unwrap();
        map.add(
            ModelType::Sonnet,
            MetricComponents::Cost(CostComponents::new(0.5, 0.5, 0.25, 0.0)),
        )
        .unwrap();

        let sonnet = map.get(ModelType::Sonnet);
        assert_eq!(sonnet, CostComponents::new(1.5, 2.5, 0.25, 0.0));
    }

    #[test]
    fn model_metrics_map_add_accumulates_tokens_per_bucket() {
        let mut map = ModelMetricsMap::default();
        map.add(
            ModelType::Sonnet,
            MetricComponents::Tokens(TokenCounts::new(1, 2, 0, 0)),
        )
        .unwrap();
        map.add(
            ModelType::Sonnet,
            MetricComponents::Tokens(TokenCounts::new(3, 4, 5, 6)),
        )
        .unwrap();

        let sonnet = map.get_tokens(ModelType::Sonnet);
        assert_eq!(sonnet, TokenCounts::new(4, 6, 5, 6));
    }

    #[test]
    fn model_metrics_map_rejects_mixed_metric_modes() {
        let mut map = ModelMetricsMap::default();
        map.add(
            ModelType::Sonnet,
            MetricComponents::Cost(CostComponents::new(1.0, 0.0, 0.0, 0.0)),
        )
        .unwrap();

        let error = map
            .add(
                ModelType::Sonnet,
                MetricComponents::Tokens(TokenCounts::new(1, 0, 0, 0)),
            )
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to aggregate Sonnet metrics"));
    }

    #[test]
    fn model_metrics_map_get_absent_returns_zero_for_mode() {
        let metrics = ModelMetricsMap::default();
        assert_eq!(metrics.get(ModelType::Sonnet), CostComponents::default());
        assert_eq!(
            metrics.get_tokens(ModelType::Sonnet),
            TokenCounts::default()
        );
    }

    #[test]
    fn model_metrics_map_total_sums_all_cost_entries() {
        let mut metrics = ModelMetricsMap::default();
        metrics
            .add(
                ModelType::Sonnet,
                MetricComponents::Cost(CostComponents::new(1.0, 2.0, 0.0, 0.0)),
            )
            .unwrap();
        metrics
            .add(
                ModelType::Haiku,
                MetricComponents::Cost(CostComponents::new(0.5, 1.0, 0.0, 0.0)),
            )
            .unwrap();

        assert_eq!(
            metrics.total(ReportMode::Cost).unwrap(),
            MetricTotal::Cost(4.5)
        );
    }

    #[test]
    fn model_metrics_map_total_sums_all_token_entries() {
        let mut metrics = ModelMetricsMap::default();
        metrics
            .add(
                ModelType::Sonnet,
                MetricComponents::Tokens(TokenCounts::new(1, 2, 3, 4)),
            )
            .unwrap();
        metrics
            .add(
                ModelType::Haiku,
                MetricComponents::Tokens(TokenCounts::new(5, 6, 7, 8)),
            )
            .unwrap();

        assert_eq!(
            metrics.total(ReportMode::Tokens).unwrap(),
            MetricTotal::Tokens(36)
        );
    }

    #[test]
    fn model_metrics_map_total_preserves_large_token_counts_exactly() {
        let mut metrics = ModelMetricsMap::default();
        metrics
            .add(
                ModelType::Sonnet,
                MetricComponents::Tokens(TokenCounts::new(9_007_199_254_740_993, 1, 0, 0)),
            )
            .unwrap();
        metrics
            .add(
                ModelType::Haiku,
                MetricComponents::Tokens(TokenCounts::new(2, 0, 0, 0)),
            )
            .unwrap();

        assert_eq!(
            metrics.total(ReportMode::Tokens).unwrap(),
            MetricTotal::Tokens(9_007_199_254_740_996)
        );
    }

    #[test]
    fn model_metrics_map_rejects_token_component_overflow() {
        let mut metrics = ModelMetricsMap::default();
        metrics
            .add(
                ModelType::Sonnet,
                MetricComponents::Tokens(TokenCounts::new(u64::MAX, 0, 0, 0)),
            )
            .unwrap();

        let error = metrics
            .add(
                ModelType::Sonnet,
                MetricComponents::Tokens(TokenCounts::new(1, 0, 0, 0)),
            )
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to aggregate Sonnet metrics"));
        assert!(format!("{error:?}").contains("token input total exceeded u64"));
    }

    #[test]
    fn model_metrics_map_ignores_unknown_buckets() {
        let mut metrics = ModelMetricsMap::default();
        metrics
            .add(
                ModelType::Unknown,
                MetricComponents::Cost(CostComponents::new(9.0, 8.0, 7.0, 6.0)),
            )
            .unwrap();

        assert_eq!(
            metrics.total(ReportMode::Cost).unwrap(),
            MetricTotal::Cost(0.0)
        );
        assert_eq!(metrics.model_metrics(ReportMode::Cost).len(), 4);
    }

    #[test]
    fn model_metrics_map_model_metrics_zero_fills_absent_entries() {
        let metrics = ModelMetricsMap::default();
        let entries = metrics.model_metrics(ReportMode::Tokens);

        assert_eq!(entries.len(), 4);
        for (_, model_metrics) in &entries {
            assert_eq!(
                *model_metrics,
                MetricComponents::Tokens(TokenCounts::default())
            );
        }
    }
}
