use std::collections::BTreeMap;

use cost_analyzer::PiModel;

/// Already-priced cost components for a single pi `(provider, model)` bucket.
///
/// Pi logs carry per-response money totals directly, so moriarty only needs a
/// small accumulator that can sum those values by bucket before rendering.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TokenCosts {
    pub(crate) input: f64,
    pub(crate) output: f64,
    pub(crate) cache_write: f64,
    pub(crate) cache_read: f64,
}

impl TokenCosts {
    pub(crate) fn new(input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        Self {
            input,
            output,
            cache_write,
            cache_read,
        }
    }

    pub(crate) fn total(&self) -> f64 {
        self.input + self.output + self.cache_write + self.cache_read
    }

    pub(crate) fn as_components(&self) -> (f64, f64, f64, f64) {
        (self.input, self.output, self.cache_write, self.cache_read)
    }

    pub(crate) fn add(&mut self, other: &TokenCosts) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
    }
}

/// A deterministic accumulator keyed by raw pi `(provider, model)` identity.
///
/// `PiModel` derives `Ord`, so using `BTreeMap` makes report rows stable across
/// runs without an extra sort step in the render path.
#[derive(Debug, Clone, Default)]
pub(crate) struct PiModelCostsMap {
    costs: BTreeMap<PiModel, TokenCosts>,
}

impl PiModelCostsMap {
    pub(crate) fn add(&mut self, model: PiModel, costs: TokenCosts) {
        self.costs.entry(model).or_default().add(&costs);
    }

    pub(crate) fn total(&self) -> f64 {
        self.costs.values().map(TokenCosts::total).sum()
    }

    pub(crate) fn model_costs(&self) -> impl Iterator<Item = (&PiModel, &TokenCosts)> {
        self.costs.iter()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.costs.len()
    }
}

#[cfg(test)]
mod tests {
    use pi_logs::Provider;

    use super::*;

    fn model(provider: Provider, model: &str) -> PiModel {
        PiModel {
            provider,
            model: model.to_string(),
        }
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
    fn pi_model_costs_map_accumulates_per_model() {
        let mut costs = PiModelCostsMap::default();
        let key = model(Provider::Anthropic, "claude-sonnet-4-5");
        costs.add(key.clone(), TokenCosts::new(1.0, 2.0, 0.0, 0.0));
        costs.add(key, TokenCosts::new(0.5, 0.5, 0.25, 0.0));

        let entries: Vec<_> = costs.model_costs().collect();
        assert_eq!(entries.len(), 1);
        assert!((entries[0].1.input - 1.5).abs() < 1e-10);
        assert!((entries[0].1.output - 2.5).abs() < 1e-10);
        assert!((entries[0].1.cache_write - 0.25).abs() < 1e-10);
    }

    #[test]
    fn pi_model_costs_map_uses_stable_provider_then_model_order() {
        let mut costs = PiModelCostsMap::default();
        costs.add(
            model(Provider::OpenAi, "gpt-5"),
            TokenCosts::new(1.0, 0.0, 0.0, 0.0),
        );
        costs.add(
            model(Provider::Anthropic, "claude-sonnet-4-5"),
            TokenCosts::new(1.0, 0.0, 0.0, 0.0),
        );
        costs.add(
            model(Provider::Anthropic, "claude-haiku-3-5"),
            TokenCosts::new(1.0, 0.0, 0.0, 0.0),
        );

        let ordered: Vec<_> = costs
            .model_costs()
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
}
