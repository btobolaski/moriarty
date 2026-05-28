use std::{cmp::Reverse, collections::HashMap};

use claude_logs::{Model, ModelFamily};

use crate::cost_report::{MetricComponents, MetricTotal, ReportMode};

/// Exhaustive `match` (rather than a const lookup table) so adding a new
/// `ModelFamily` variant fails to compile here until the ordering is
/// updated, instead of panicking at runtime with the previous
/// `expect("every ModelFamily variant must appear …")` shape. Newer /
/// typically more expensive families come first; Opus 3 vs Opus 4.x share
/// `ModelFamily::Opus` and are further ordered by the version-descending
/// tiebreak in `model_sort_key`. `Synthetic` is placed last as a defensive
/// fallback even though `priced_claude_assistant` keeps it out of
/// `ModelMetricsMap` in practice.
fn family_sort_key(family: ModelFamily) -> usize {
    match family {
        ModelFamily::Opus => 0,
        ModelFamily::Sonnet => 1,
        ModelFamily::Haiku => 2,
        ModelFamily::Synthetic => 3,
    }
}

/// Ordering tuple combining the family bucket order with descending version
/// so the newest Opus 4.x appears above older 4.x variants. The
/// `has_no_version` flag forces entries with no parsed digits to sort last
/// inside their family so e.g. a bare `"SONNET"` row sits below
/// `"Sonnet 4.5"` rather than ahead of it.
fn model_sort_key(model: &Model) -> (usize, Reverse<(u32, u32)>, u8) {
    let (major, minor, has_no_version) = match model.version {
        Some(version) => (version.major, version.minor.unwrap_or(0), 0u8),
        None => (0, 0, 1u8),
    };
    (
        family_sort_key(model.family),
        Reverse((major, minor)),
        has_no_version,
    )
}

#[derive(Debug, Clone, Default)]
pub struct ModelMetricsMap {
    metrics: HashMap<Model, MetricComponents>,
}

impl ModelMetricsMap {
    /// Unknown models are dropped silently because `cost_analyzer` already
    /// emitted a structured tracing error for them upstream; raising another
    /// error here would double-report the same condition to the user.
    pub fn add(
        &mut self,
        model: Model,
        metrics: impl Into<MetricComponents>,
    ) -> miette::Result<()> {
        if model.family == ModelFamily::Synthetic {
            // Synthetic assistant turns have no usage to bill; skip them
            // before storing so the report never shows a `<synthetic>` row.
            return Ok(());
        }

        let metrics = metrics.into();
        // Capture the display label before `entry(model)` consumes `model`,
        // so the error path doesn't need to clone or borrow it back out.
        let label = model.to_string();

        match self.metrics.entry(model) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(metrics);
                Ok(())
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => entry
                .get_mut()
                .checked_add_assign(metrics)
                .map_err(|error| error.wrap_err(format!("failed to aggregate {label} metrics"))),
        }
    }

    #[cfg(test)]
    pub fn get_metric(&self, model: Model, report_mode: ReportMode) -> MetricComponents {
        self.metrics
            .get(&model)
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

    /// Ordered view of every populated bucket. Empty buckets are not emitted,
    /// so the report's row count tracks the number of distinct model versions
    /// actually observed in the logs instead of a fixed family list.
    pub fn model_metrics(&self) -> Vec<(String, MetricComponents)> {
        let mut entries: Vec<(&Model, &MetricComponents)> = self.metrics.iter().collect();
        entries.sort_by(|left, right| model_sort_key(left.0).cmp(&model_sort_key(right.0)));
        entries
            .into_iter()
            .map(|(model, metrics)| (model.to_string(), *metrics))
            .collect()
    }
}

#[cfg(test)]
impl ModelMetricsMap {
    pub fn get(&self, model: Model) -> crate::cost_report::CostComponents {
        match self.get_metric(model, ReportMode::Cost) {
            MetricComponents::Cost(costs) => costs,
            MetricComponents::Tokens(_) => {
                unreachable!("map contains token metrics but cost access was requested")
            }
        }
    }

    pub fn get_tokens(&self, model: Model) -> crate::cost_report::TokenCounts {
        match self.get_metric(model, ReportMode::Tokens) {
            MetricComponents::Cost(_) => {
                unreachable!("map contains cost metrics but token access was requested")
            }
            MetricComponents::Tokens(tokens) => tokens,
        }
    }

    pub fn model_costs(&self) -> Vec<(String, crate::cost_report::CostComponents)> {
        self.model_metrics()
            .into_iter()
            .map(|(name, metrics)| match metrics {
                MetricComponents::Cost(costs) => (name, costs),
                MetricComponents::Tokens(_) => {
                    unreachable!("map contains token metrics but cost access was requested")
                }
            })
            .collect()
    }
}

#[cfg(test)]
pub type ModelCostsMap = ModelMetricsMap;

#[cfg(test)]
mod tests {
    use crate::cost_report::{CostComponents, TokenCounts};

    use super::*;

    fn sonnet() -> Model {
        Model::family(ModelFamily::Sonnet)
    }
    fn haiku() -> Model {
        Model::family(ModelFamily::Haiku)
    }

    #[test]
    fn model_metrics_map_add_accumulates_costs_per_bucket() {
        let mut map = ModelMetricsMap::default();
        map.add(
            sonnet(),
            MetricComponents::Cost(CostComponents::new(1.0, 2.0, 0.0, 0.0)),
        )
        .unwrap();
        map.add(
            sonnet(),
            MetricComponents::Cost(CostComponents::new(0.5, 0.5, 0.25, 0.0)),
        )
        .unwrap();

        let sonnet_costs = map.get(sonnet());
        assert_eq!(sonnet_costs, CostComponents::new(1.5, 2.5, 0.25, 0.0));
    }

    #[test]
    fn model_metrics_map_add_accumulates_tokens_per_bucket() {
        let mut map = ModelMetricsMap::default();
        map.add(
            sonnet(),
            MetricComponents::Tokens(TokenCounts::new(1, 2, 0, 0)),
        )
        .unwrap();
        map.add(
            sonnet(),
            MetricComponents::Tokens(TokenCounts::new(3, 4, 5, 6)),
        )
        .unwrap();

        let sonnet_tokens = map.get_tokens(sonnet());
        assert_eq!(sonnet_tokens, TokenCounts::new(4, 6, 5, 6));
    }

    #[test]
    fn model_metrics_map_rejects_mixed_metric_modes() {
        let mut map = ModelMetricsMap::default();
        map.add(
            sonnet(),
            MetricComponents::Cost(CostComponents::new(1.0, 0.0, 0.0, 0.0)),
        )
        .unwrap();

        let error = map
            .add(
                sonnet(),
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
        assert_eq!(metrics.get(sonnet()), CostComponents::default());
        assert_eq!(metrics.get_tokens(sonnet()), TokenCounts::default());
    }

    #[test]
    fn model_metrics_map_total_sums_all_cost_entries() {
        let mut metrics = ModelMetricsMap::default();
        metrics
            .add(
                sonnet(),
                MetricComponents::Cost(CostComponents::new(1.0, 2.0, 0.0, 0.0)),
            )
            .unwrap();
        metrics
            .add(
                haiku(),
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
                sonnet(),
                MetricComponents::Tokens(TokenCounts::new(1, 2, 3, 4)),
            )
            .unwrap();
        metrics
            .add(
                haiku(),
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
                sonnet(),
                MetricComponents::Tokens(TokenCounts::new(9_007_199_254_740_993, 1, 0, 0)),
            )
            .unwrap();
        metrics
            .add(
                haiku(),
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
                sonnet(),
                MetricComponents::Tokens(TokenCounts::new(u64::MAX, 0, 0, 0)),
            )
            .unwrap();

        let error = metrics
            .add(
                sonnet(),
                MetricComponents::Tokens(TokenCounts::new(1, 0, 0, 0)),
            )
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to aggregate Sonnet metrics"));
        assert!(format!("{error:?}").contains("token input total exceeded u64"));
    }

    #[test]
    fn model_metrics_map_ignores_synthetic_family() {
        // `<synthetic>` is the only non-billable family that can reach
        // `add` (unknown ids fail to parse upstream); the map must drop it
        // silently so the report doesn't grow a phantom `<synthetic>` row.
        let mut metrics = ModelMetricsMap::default();
        metrics
            .add(
                Model::family(ModelFamily::Synthetic),
                MetricComponents::Cost(CostComponents::new(9.0, 8.0, 7.0, 6.0)),
            )
            .unwrap();

        assert_eq!(
            metrics.total(ReportMode::Cost).unwrap(),
            MetricTotal::Cost(0.0)
        );
        assert!(metrics.model_metrics().is_empty());
    }

    #[test]
    fn model_metrics_map_empty_returns_no_entries() {
        let metrics = ModelMetricsMap::default();
        assert!(metrics.model_metrics().is_empty());
    }

    #[test]
    fn model_metrics_sorts_family_first_then_version_desc() {
        let mut metrics = ModelMetricsMap::default();
        let cases = [
            "claude-3-5-sonnet-20241022",
            "claude-sonnet-4-20250514",
            "claude-sonnet-4-5-20250929",
            "claude-opus-4-20250514",
            "claude-opus-4-5",
            "claude-opus-4-7",
            // Opus 3.5 (hypothetical id, not shipped by Anthropic) pins the
            // within-family ordering: a minor-bearing Opus 3 entry must sort
            // between "Opus 4" and the bare "Opus 3" major-only row.
            "claude-opus-3-5-20241022",
            "claude-3-opus-20240229",
            "claude-3-haiku-20240307",
            "claude-haiku-4-5",
        ];
        for id in cases {
            metrics
                .add(
                    Model::from_model_string(id).expect("fixture model id parses"),
                    MetricComponents::Cost(CostComponents::new(1.0, 0.0, 0.0, 0.0)),
                )
                .unwrap();
        }

        let labels: Vec<String> = metrics
            .model_metrics()
            .into_iter()
            .map(|(name, _)| name)
            .collect();

        assert_eq!(
            labels,
            vec![
                "Opus 4.7",
                "Opus 4.5",
                "Opus 4",
                "Opus 3.5",
                "Opus 3",
                "Sonnet 4.5",
                "Sonnet 4",
                "Sonnet 3.5",
                "Haiku 4.5",
                "Haiku 3",
            ]
        );
    }

    #[test]
    fn model_metrics_sorts_unversioned_entry_last_within_family() {
        // Exercises the `has_no_version` tiebreaker in `model_sort_key`: a
        // bare-family entry (e.g. `"SONNET"`) must sort below versioned
        // entries of the same family so reports group the "newest first"
        // versions on top and leave the unparseable row at the bottom.
        let mut metrics = ModelMetricsMap::default();
        for id in [
            "claude-sonnet-4-5-20250929",
            "claude-sonnet-4-20250514",
            "SONNET",
        ] {
            metrics
                .add(
                    Model::from_model_string(id).expect("fixture model id parses"),
                    MetricComponents::Cost(CostComponents::new(1.0, 0.0, 0.0, 0.0)),
                )
                .unwrap();
        }

        let labels: Vec<String> = metrics
            .model_metrics()
            .into_iter()
            .map(|(name, _)| name)
            .collect();

        assert_eq!(labels, vec!["Sonnet 4.5", "Sonnet 4", "Sonnet"]);
    }
}
