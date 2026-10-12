use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use miette::{IntoDiagnostic, WrapErr};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Map, Value};
use tracing::warn;

use cost_analyzer::{LlmCost, PiModel};
use pi_logs::{AssistantUsage, Provider};

/// These rates are flattened into pi's base and tier prices, so strict unknown
/// fields would reject siblings such as tiers and inputTokensAbove. Ignore
/// pi-owned metadata while requiring and validating every rate; never execute
/// credential commands or load extensions from the catalog.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rates {
    input: Decimal,
    output: Decimal,
    cache_read: Decimal,
    cache_write: Decimal,
}

/// Pi owns evolving tier metadata; serde(flatten) cannot combine with
/// deny_unknown_fields. Required rates and the threshold remain typed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tier {
    input_tokens_above: u64,
    #[serde(flatten)]
    rates: Rates,
}

/// Pi's pricing shape evolves (e.g. tiers). Flattened rates prevent
/// deny_unknown_fields; unrelated catalog metadata does not affect estimates.
#[derive(Debug, Deserialize)]
struct ModelPrice {
    #[serde(flatten)]
    rates: Rates,
    #[serde(default)]
    tiers: Vec<Tier>,
}

impl ModelPrice {
    fn valid(&self) -> bool {
        std::iter::once(&self.rates)
            .chain(self.tiers.iter().map(|tier| &tier.rates))
            .all(|rates| {
                [
                    rates.input,
                    rates.output,
                    rates.cache_read,
                    rates.cache_write,
                ]
                .iter()
                .all(|rate| *rate >= Decimal::ZERO)
            })
    }

    fn estimate(&self, usage: &AssistantUsage) -> miette::Result<LlmCost> {
        let prompt =
            u128::from(usage.input) + u128::from(usage.cache_read) + u128::from(usage.cache_write);
        let rates = self
            .tiers
            .iter()
            .filter(|tier| prompt > u128::from(tier.input_tokens_above))
            .max_by_key(|tier| tier.input_tokens_above)
            .map_or(&self.rates, |tier| &tier.rates);
        let per_million = |tokens: u64, rate: Decimal| {
            Decimal::from(tokens)
                .checked_mul(rate)
                .and_then(|cost| cost.checked_div(Decimal::from(1_000_000)))
                .ok_or_else(|| miette::miette!("pi catalog cost exceeds Decimal range"))
        };
        let long_write = usage.cache_write1h.unwrap_or(0);
        let short_write = usage
            .cache_write
            .checked_sub(long_write)
            .ok_or_else(|| miette::miette!("pi cacheWrite1h exceeds cacheWrite"))?;
        // Pi prices 1h cache writes at twice input, independent of the short-write rate.
        let long_rate = rates
            .input
            .checked_mul(Decimal::from(2))
            .ok_or_else(|| miette::miette!("pi catalog rate exceeds Decimal range"))?;
        let cache_write = per_million(short_write, rates.cache_write)?
            .checked_add(per_million(long_write, long_rate)?)
            .ok_or_else(|| miette::miette!("pi cache-write cost exceeds Decimal range"))?;
        Ok(LlmCost {
            input: per_million(usage.input, rates.input)?,
            output: per_million(usage.output, rates.output)?,
            cache_read: per_million(usage.cache_read, rates.cache_read)?,
            cache_write,
        })
    }
}

#[derive(Default)]
pub(super) struct PricingCatalog {
    prices: BTreeMap<(String, String), Map<String, Value>>,
    parsed: BTreeMap<(String, String), ModelPrice>,
    missing: BTreeSet<PiModel>,
}

impl PricingCatalog {
    pub(super) fn load() -> Self {
        let agent_dir = env::var_os("PI_CODING_AGENT_DIR")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".pi/agent")));
        agent_dir.map_or_else(Self::default, |dir| Self::from_dir(&dir))
    }

    fn from_dir(dir: &Path) -> Self {
        let mut catalog = Self::default();
        for (filename, overrides) in [("models-store.json", false), ("models.json", true)] {
            let path = dir.join(filename);
            let value = match fs::read_to_string(&path) {
                Ok(text) => match serde_json::from_str::<Value>(&text) {
                    Ok(value) => value,
                    Err(error) => {
                        warn!(path = %path.display(), %error, "cannot read pi pricing catalog");
                        continue;
                    }
                },
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    warn!(path = %path.display(), %error, "cannot read pi pricing catalog");
                    continue;
                }
            };
            catalog.merge(&value, overrides);
        }
        catalog
    }

    fn merge(&mut self, value: &Value, overrides: bool) {
        let providers = if overrides {
            &value["providers"]
        } else {
            value
        };
        let Some(providers) = providers.as_object() else {
            return;
        };
        for (provider, data) in providers {
            if let Some(models) = data["models"].as_array() {
                for model in models {
                    if let Some(id) = model["id"].as_str() {
                        self.merge_price(provider, id, &model["cost"]);
                    }
                }
            }
            if overrides && let Some(models) = data["modelOverrides"].as_object() {
                for (id, model) in models {
                    self.merge_price(provider, id, &model["cost"]);
                }
            }
        }
    }

    fn merge_price(&mut self, provider: &str, id: &str, price: &Value) {
        let Some(fields) = price.as_object() else {
            return;
        };
        let key = (provider.to_owned(), id.to_owned());
        self.prices.entry(key).or_default().extend(fields.clone());
    }

    pub(super) fn estimate(
        &mut self,
        model: &PiModel,
        usage: &AssistantUsage,
    ) -> miette::Result<Option<LlmCost>> {
        let Value::String(provider) = serde_json::to_value(model.provider).into_diagnostic()?
        else {
            return Err(miette::miette!(
                "pi provider did not serialize as a wire name"
            ));
        };
        let mut key = (provider.to_owned(), model.model.clone());
        if !self.prices.contains_key(&key) && model.provider == Provider::OpenAiCodex {
            key.0 = "openai".to_owned();
        }
        if let Some(raw) = self.prices.get(&key) {
            if !self.parsed.contains_key(&key) {
                let price: ModelPrice = serde_json::from_value(Value::Object(raw.clone()))
                    .into_diagnostic()
                    .wrap_err_with(|| {
                        format!("invalid pi catalog pricing for {provider}/{}", model.model)
                    })?;
                if !price.valid() {
                    return Err(miette::miette!(
                        "negative pi catalog pricing for {provider}/{}",
                        model.model
                    ));
                }
                self.parsed.insert(key.clone(), price);
            }
            return self.parsed[&key].estimate(usage).map(Some);
        }
        if self.missing.insert(model.clone()) {
            warn!(provider, model = %model.model, "no pi catalog price for zero-cost usage; API-equivalent cost remains unestimated (refresh pi's models-store.json or configure models.json)");
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::test_helpers::TestEnvVarGuard;

    fn usage() -> AssistantUsage {
        serde_json::from_value(json!({"input":100,"output":10,"cacheRead":20,"cacheWrite":4,"cacheWrite1h":1,"totalTokens":134,
            "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}})).unwrap()
    }

    fn model() -> PiModel {
        PiModel {
            provider: Provider::OpenAiCodex,
            model: "example".to_owned(),
        }
    }

    #[test]
    fn load_reads_catalog_under_home_when_agent_dir_is_unset() {
        let home = TempDir::new().unwrap();
        let _agent_guard = TestEnvVarGuard::unset("PI_CODING_AGENT_DIR");
        let _home_guard = TestEnvVarGuard::set("HOME", home.path());
        let agent = home.path().join(".pi/agent");
        fs::create_dir_all(&agent).unwrap();
        fs::write(
            agent.join("models-store.json"),
            json!({"openai-codex":{"models":[{"id":"example","cost":{
            "input":2,"output":12,"cacheRead":0.2,"cacheWrite":2.5}}]}})
            .to_string(),
        )
        .unwrap();
        let price = PricingCatalog::load()
            .estimate(&model(), &usage())
            .unwrap()
            .unwrap();
        assert_eq!(price.input, Decimal::new(2, 4));
    }

    #[test]
    fn load_has_no_prices_without_an_agent_dir_or_home() {
        let _agent_guard = TestEnvVarGuard::unset("PI_CODING_AGENT_DIR");
        let _home_guard = TestEnvVarGuard::unset("HOME");
        assert!(PricingCatalog::load().prices.is_empty());
    }

    #[test]
    fn unusable_catalogs_leave_prices_unavailable_without_failing() {
        for (case, store, overrides) in [
            ("unreadable", None, "not JSON"),
            ("invalid JSON", Some("not JSON"), "not JSON"),
            ("non-object providers", Some("[]"), r#"{"providers":[]}"#),
            (
                "non-array models",
                Some(r#"{"openai-codex":{"models":{}}}"#),
                "{}",
            ),
            (
                "non-object cost",
                Some(r#"{"openai-codex":{"models":[{"id":"example","cost":[]}]}}"#),
                "{}",
            ),
        ] {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("models-store.json");
            match store {
                Some(text) => fs::write(&path, text).unwrap(),
                None => fs::create_dir(&path).unwrap(),
            }
            fs::write(dir.path().join("models.json"), overrides).unwrap();
            let price = PricingCatalog::from_dir(dir.path())
                .estimate(&model(), &usage())
                .unwrap();
            assert!(price.is_none(), "{case}");
        }
    }

    #[test]
    fn cached_catalog_and_partial_overrides_price_tiers_and_long_cache_writes() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("models-store.json"), json!({"openai-codex":{"models":[{"id":"example","cost":{
            "input":2,"output":12,"cacheRead":0.2,"cacheWrite":2.5,
            "tiers":[{"inputTokensAbove":124,"input":4,"output":18,"cacheRead":0.4,"cacheWrite":5}]}}]}}).to_string()).unwrap();
        fs::write(dir.path().join("models.json"), json!({"providers":{"openai-codex":{"modelOverrides":{"example":{"cost":{"output":10}}}}}}).to_string()).unwrap();
        let mut catalog = PricingCatalog::from_dir(dir.path());
        let mut usage = usage();
        let base = catalog.estimate(&model(), &usage).unwrap().unwrap();
        assert_eq!(base.input, Decimal::new(2, 4));
        assert_eq!(base.output, Decimal::new(1, 4));
        assert_eq!(base.cache_read, Decimal::new(4, 6));
        assert_eq!(base.cache_write, Decimal::new(115, 7));
        usage.input += 1;
        let tier = catalog.estimate(&model(), &usage).unwrap().unwrap();
        assert_eq!(tier.input, Decimal::new(404, 6));
        assert_eq!(tier.output, Decimal::new(18, 5));
        assert_eq!(tier.cache_write, Decimal::new(23, 6));
        assert_eq!(catalog.parsed.len(), 1);
    }

    #[test]
    fn codex_can_use_api_rates_but_unknown_models_are_not_guessed() {
        let mut catalog = PricingCatalog::default();
        catalog.merge(&json!({"providers":{"openai":{"models":[{"id":"example","cost":{"input":2,"output":12,"cacheRead":0.2,"cacheWrite":2.5}}]}}}), true);
        assert!(catalog.estimate(&model(), &usage()).unwrap().is_some());
        let unknown = PiModel {
            model: "unknown".to_owned(),
            ..model()
        };
        assert!(catalog.estimate(&unknown, &usage()).unwrap().is_none());
        assert!(catalog.estimate(&unknown, &usage()).unwrap().is_none());
        assert_eq!(catalog.missing.len(), 1);
    }

    #[test]
    fn negative_catalog_rates_fail_instead_of_fabricating_cost() {
        let mut catalog = PricingCatalog::default();
        catalog.merge_price(
            "openai-codex",
            "example",
            &json!({"input":-1,"output":12,"cacheRead":0.2,"cacheWrite":2.5}),
        );
        let error = catalog.estimate(&model(), &usage()).unwrap_err();
        assert!(error.to_string().contains("negative pi catalog pricing"));
    }

    #[test]
    fn excessive_long_cache_writes_fail_instead_of_fabricating_cost() {
        let mut catalog = PricingCatalog::default();
        catalog.merge_price(
            "openai-codex",
            "example",
            &json!({"input":2,"output":12,"cacheRead":0.2,"cacheWrite":2.5}),
        );
        let mut bad_usage = usage();
        bad_usage.cache_write1h = Some(5);
        let error = catalog.estimate(&model(), &bad_usage).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("cacheWrite1h exceeds cacheWrite")
        );
    }
}
