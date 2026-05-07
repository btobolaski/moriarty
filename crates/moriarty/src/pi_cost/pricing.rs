use std::collections::BTreeMap;

use cost_analyzer::PiModel;
use miette::WrapErr;

use crate::cost_report::{MetricComponents, MetricTotal, ReportMode};

/// A deterministic accumulator keyed by raw pi `(provider, model)` identity.
///
/// `PiModel` derives `Ord`, so using `BTreeMap` makes report rows stable across
/// runs without an extra sort step in the render path.
#[derive(Debug, Clone, Default)]
pub(crate) struct PiModelMetricsMap {
    metrics: BTreeMap<PiModel, MetricComponents>,
}

impl PiModelMetricsMap {
    pub(crate) fn add(
        &mut self,
        model: PiModel,
        metrics: impl Into<MetricComponents>,
    ) -> miette::Result<()> {
        let metrics = metrics.into();

        match self.metrics.entry(model.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(metrics);
                Ok(())
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => entry
                .get_mut()
                .checked_add_assign(metrics)
                .wrap_err_with(|| format!("failed to aggregate metrics for {}", model.model)),
        }
    }

    pub(crate) fn total(&self, report_mode: ReportMode) -> miette::Result<MetricTotal> {
        self.metrics
            .values()
            .try_fold(MetricTotal::zero(report_mode), |acc, metrics| {
                acc.checked_add(metrics.total())
            })
    }

    pub(crate) fn model_metrics(&self) -> impl Iterator<Item = (&PiModel, &MetricComponents)> {
        self.metrics.iter()
    }

    #[cfg(test)]
    pub(crate) fn model_costs(
        &self,
    ) -> impl Iterator<Item = (&PiModel, &crate::cost_report::CostComponents)> {
        self.metrics.iter().map(|(model, metrics)| {
            let MetricComponents::Cost(costs) = metrics else {
                unreachable!("cost tests requested token metrics")
            };
            (model, costs)
        })
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.metrics.len()
    }
}

#[cfg(test)]
pub(crate) type PiModelCostsMap = PiModelMetricsMap;

#[cfg(test)]
mod tests {
    use pi_logs::Provider;

    use crate::cost_report::{CostComponents, TokenCounts};

    use super::*;

    fn model(provider: Provider, model: &str) -> PiModel {
        PiModel {
            provider,
            model: model.to_string(),
        }
    }

    #[test]
    fn pi_model_metrics_map_accumulates_costs_per_model() {
        let mut metrics = PiModelMetricsMap::default();
        let key = model(Provider::Anthropic, "claude-sonnet-4-5");
        metrics
            .add(
                key.clone(),
                MetricComponents::Cost(CostComponents::new(1.0, 2.0, 0.0, 0.0)),
            )
            .unwrap();
        metrics
            .add(
                key,
                MetricComponents::Cost(CostComponents::new(0.5, 0.5, 0.25, 0.0)),
            )
            .unwrap();

        let entries: Vec<_> = metrics.model_metrics().collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            *entries[0].1,
            MetricComponents::Cost(CostComponents::new(1.5, 2.5, 0.25, 0.0))
        );
    }

    #[test]
    fn pi_model_metrics_map_accumulates_tokens_per_model() {
        let mut metrics = PiModelMetricsMap::default();
        let key = model(Provider::Anthropic, "claude-sonnet-4-5");
        metrics
            .add(
                key.clone(),
                MetricComponents::Tokens(TokenCounts::new(1, 2, 3, 4)),
            )
            .unwrap();
        metrics
            .add(key, MetricComponents::Tokens(TokenCounts::new(5, 6, 7, 8)))
            .unwrap();

        let entries: Vec<_> = metrics.model_metrics().collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            *entries[0].1,
            MetricComponents::Tokens(TokenCounts::new(6, 8, 10, 12))
        );
    }

    #[test]
    fn pi_model_metrics_map_rejects_mixed_metric_modes() {
        let mut metrics = PiModelMetricsMap::default();
        let key = model(Provider::Anthropic, "claude-sonnet-4-5");
        metrics
            .add(
                key.clone(),
                MetricComponents::Cost(CostComponents::new(1.0, 0.0, 0.0, 0.0)),
            )
            .unwrap();

        let error = metrics
            .add(key, MetricComponents::Tokens(TokenCounts::new(1, 0, 0, 0)))
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to aggregate metrics for claude-sonnet-4-5"));
    }

    #[test]
    fn pi_model_metrics_map_uses_stable_provider_then_model_order() {
        let mut metrics = PiModelMetricsMap::default();
        metrics
            .add(
                model(Provider::OpenAi, "gpt-5"),
                MetricComponents::Tokens(TokenCounts::new(1, 0, 0, 0)),
            )
            .unwrap();
        metrics
            .add(
                model(Provider::Anthropic, "claude-sonnet-4-5"),
                MetricComponents::Tokens(TokenCounts::new(1, 0, 0, 0)),
            )
            .unwrap();
        metrics
            .add(
                model(Provider::Anthropic, "claude-haiku-3-5"),
                MetricComponents::Tokens(TokenCounts::new(1, 0, 0, 0)),
            )
            .unwrap();

        let ordered: Vec<_> = metrics
            .model_metrics()
            .map(|(model, _)| (model.provider, model.model.as_str()))
            .collect();

        assert_eq!(
            ordered,
            vec![
                (Provider::Anthropic, "claude-haiku-3-5"),
                (Provider::Anthropic, "claude-sonnet-4-5"),
                (Provider::OpenAi, "gpt-5"),
            ]
        );
    }

    #[test]
    fn pi_model_metrics_map_total_sums_all_entries() {
        let mut metrics = PiModelMetricsMap::default();
        metrics
            .add(
                model(Provider::Anthropic, "claude-sonnet-4-5"),
                MetricComponents::Tokens(TokenCounts::new(1, 2, 3, 4)),
            )
            .unwrap();
        metrics
            .add(
                model(Provider::OpenAi, "gpt-5"),
                MetricComponents::Tokens(TokenCounts::new(5, 6, 7, 8)),
            )
            .unwrap();

        assert_eq!(
            metrics.total(ReportMode::Tokens).unwrap(),
            MetricTotal::Tokens(36)
        );
    }

    #[test]
    fn pi_model_metrics_map_rejects_token_component_overflow() {
        let mut metrics = PiModelMetricsMap::default();
        let key = model(Provider::Anthropic, "claude-sonnet-4-5");
        metrics
            .add(
                key.clone(),
                MetricComponents::Tokens(TokenCounts::new(u64::MAX, 0, 0, 0)),
            )
            .unwrap();

        let error = metrics
            .add(key, MetricComponents::Tokens(TokenCounts::new(1, 0, 0, 0)))
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to aggregate metrics for claude-sonnet-4-5"));
        assert!(format!("{error:?}").contains("token input total exceeded u64"));
    }
}
