use chrono::{DateTime, Utc};
use miette::{Context, IntoDiagnostic};
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;

use pi_logs::{AssistantMessage, PiLogLine, Provider, RoleMessage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LlmCost {
    pub input: Decimal,
    pub cache_write: Decimal,
    pub cache_read: Decimal,
    pub output: Decimal,
}

impl Ord for LlmCost {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.total()
            .cmp(&other.total())
            .then_with(|| self.input.cmp(&other.input))
            .then_with(|| self.cache_write.cmp(&other.cache_write))
            .then_with(|| self.cache_read.cmp(&other.cache_read))
            .then_with(|| self.output.cmp(&other.output))
    }
}

impl PartialOrd for LlmCost {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl LlmCost {
    pub fn total(&self) -> Decimal {
        self.input + self.cache_write + self.cache_read + self.output
    }
}

pub trait IdentifierRequirements: std::fmt::Debug + Clone + Eq + Ord + std::hash::Hash {}

impl<T> IdentifierRequirements for T where T: std::fmt::Debug + Clone + Eq + Ord + std::hash::Hash {}

pub trait Identifier: IdentifierRequirements + Send + Sync + 'static {}

impl<T> Identifier for T where T: IdentifierRequirements + Send + Sync + 'static {}

pub(crate) fn parse_json_line<T: DeserializeOwned>(
    value: &str,
    context: &'static str,
) -> miette::Result<T> {
    serde_json::from_str(value)
        .into_diagnostic()
        .context(context)
}

pub trait AnalyzableLog: std::fmt::Debug + Clone + Send + Sync + 'static {
    type LogId: Identifier;
    type ModelId: Identifier;

    /// cost returns Option<LlmCost> because not all entries in the log have a cost i.e. entries from users
    fn cost(&self) -> Option<LlmCost>;
    fn timestamp(&self) -> DateTime<Utc>;
    fn identifier(&self) -> Self::LogId;
    /// model is only set on messages from the LLM so, this returns option
    fn model(&self) -> Option<Self::ModelId>;
    fn parse(value: &str) -> miette::Result<Self>;
}

#[derive(Debug, Clone)]
pub struct LineWithCost<Log>
where
    Log: AnalyzableLog,
{
    pub id: Log::LogId,
    pub model: Log::ModelId,
    pub timestamp: DateTime<Utc>,
    pub log: Box<Log>,
    pub cost: LlmCost,
}

impl<Log> LineWithCost<Log>
where
    Log: AnalyzableLog,
{
    pub fn parse(value: &str) -> miette::Result<Option<Self>> {
        let log = Log::parse(value)?;
        Ok(match (log.cost(), log.model()) {
            (Some(cost), Some(model)) => Some(Self {
                id: log.identifier(),
                model,
                cost,
                timestamp: log.timestamp(),
                log: Box::new(log),
            }),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PiModel {
    pub provider: Provider,
    pub model: String,
}

fn pi_assistant_message(line: &PiLogLine) -> Option<&AssistantMessage> {
    match line {
        PiLogLine::Message(message) => match &message.message {
            RoleMessage::Assistant(assistant) => Some(assistant),
            _ => None,
        },
        _ => None,
    }
}

impl AnalyzableLog for PiLogLine {
    type LogId = String;
    type ModelId = PiModel;

    fn cost(&self) -> Option<LlmCost> {
        pi_assistant_message(self).map(|assistant| LlmCost {
            input: assistant.usage.cost.input,
            cache_read: assistant.usage.cost.cache_read,
            cache_write: assistant.usage.cost.cache_write,
            output: assistant.usage.cost.output,
        })
    }

    fn identifier(&self) -> String {
        match self {
            PiLogLine::Custom(custom) => custom.id.clone(),
            PiLogLine::CustomMessage(message) => message.id.clone(),
            PiLogLine::Message(message) => message.id.clone(),
            PiLogLine::ModelChange(model_change) => model_change.id.clone(),
            PiLogLine::Session(session) => session.id.to_string(),
            PiLogLine::ThinkingLevelChange(thinking_level) => thinking_level.id.clone(),
        }
    }

    fn model(&self) -> Option<PiModel> {
        pi_assistant_message(self).map(|assistant| PiModel {
            model: assistant.model.clone(),
            provider: assistant.provider,
        })
    }

    fn timestamp(&self) -> DateTime<Utc> {
        match self {
            PiLogLine::Custom(custom) => custom.timestamp,
            PiLogLine::CustomMessage(message) => message.timestamp,
            PiLogLine::Message(message) => message.timestamp,
            PiLogLine::ModelChange(model_change) => model_change.timestamp,
            PiLogLine::Session(session) => session.timestamp,
            PiLogLine::ThinkingLevelChange(thinking_level) => thinking_level.timestamp,
        }
    }

    fn parse(value: &str) -> miette::Result<Self> {
        parse_json_line(value, "failed to parse log line")
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use rust_decimal::Decimal;
    use serde_json::json;

    use super::*;

    fn timestamp() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-25T01:48:25.742Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn assistant_usage_json() -> serde_json::Value {
        json!({
            "input": 10,
            "output": 5,
            "cacheRead": 2,
            "cacheWrite": 1,
            "totalTokens": 18,
            "cost": {
                "input": "3",
                "output": "5",
                "cacheRead": "2",
                "cacheWrite": "1",
                "total": "11",
            },
        })
    }

    fn assistant_message_json() -> serde_json::Value {
        json!({
            "type": "message",
            "id": "a1",
            "parentId": "u1",
            "timestamp": "2026-04-25T01:48:25.742Z",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "hello"}],
                "api": "anthropic-messages",
                "provider": "anthropic",
                "model": "claude-sonnet-4-5",
                "usage": assistant_usage_json(),
                "stopReason": "stop",
                "timestamp": 1_700_000_000,
            },
        })
    }

    fn user_message_json() -> serde_json::Value {
        json!({
            "type": "message",
            "id": "u1",
            "parentId": "p1",
            "timestamp": "2026-04-25T01:48:25.742Z",
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": "hello"}],
                "timestamp": 1_700_000_000,
            },
        })
    }

    fn tool_result_message_json() -> serde_json::Value {
        json!({
            "type": "message",
            "id": "tr1",
            "parentId": "a1",
            "timestamp": "2026-04-25T01:48:25.742Z",
            "message": {
                "role": "toolResult",
                "toolCallId": "call_1",
                "toolName": "bash",
                "content": [{"type": "text", "text": "ok"}],
                "isError": false,
                "timestamp": 1_700_000_000,
            },
        })
    }

    fn session_json() -> serde_json::Value {
        json!({
            "type": "session",
            "version": 1,
            "id": "019dc252-e50e-766c-8182-d654b46881af",
            "timestamp": "2026-04-25T01:48:25.742Z",
            "cwd": "/home/brendan/src/moriarty",
        })
    }

    fn model_change_json() -> serde_json::Value {
        json!({
            "type": "model_change",
            "id": "m1",
            "parentId": null,
            "timestamp": "2026-04-25T01:48:25.742Z",
            "provider": "anthropic",
            "modelId": "claude-sonnet-4-5",
        })
    }

    fn thinking_level_change_json() -> serde_json::Value {
        json!({
            "type": "thinking_level_change",
            "id": "t1",
            "parentId": "m1",
            "timestamp": "2026-04-25T01:48:25.742Z",
            "thinkingLevel": "high",
        })
    }

    fn custom_json() -> serde_json::Value {
        json!({
            "type": "custom",
            "id": "c1",
            "parentId": "p1",
            "timestamp": "2026-04-25T01:48:25.742Z",
            "customType": "dcp-state",
            "data": {
                "compressionBlocks": [{
                    "id": 1,
                    "topic": "Test topic",
                    "summary": "Test summary",
                    "startTimestamp": 1777084923000_i64,
                    "endTimestamp": 1777084924000_i64,
                    "anchorTimestamp": 1777084924000_i64,
                    "active": true,
                    "summaryTokenEstimate": 100,
                    "createdAt": 1777084924500_i64
                }],
                "nextBlockId": 2,
                "prunedToolIds": ["call_1"],
                "tokensSaved": 1000,
                "totalPruneCount": 3,
                "manualMode": false,
            },
        })
    }

    fn custom_message_json() -> serde_json::Value {
        json!({
            "type": "custom_message",
            "id": "cm1",
            "parentId": "p1",
            "timestamp": "2026-04-25T01:48:25.742Z",
            "content": "Plan complete",
            "display": true,
            "customType": "plannotator-complete",
        })
    }

    fn parse_pi_log(value: serde_json::Value) -> PiLogLine {
        <PiLogLine as AnalyzableLog>::parse(&value.to_string()).unwrap()
    }

    #[test]
    fn llm_cost_total_sums_all_fields() {
        let cost = LlmCost {
            input: Decimal::new(3, 0),
            cache_write: Decimal::new(1, 0),
            cache_read: Decimal::new(2, 0),
            output: Decimal::new(5, 0),
        };

        assert_eq!(cost.total(), Decimal::new(11, 0));
    }

    #[test]
    fn llm_cost_ordering_prioritizes_total_before_field_tiebreakers() {
        let higher_total = LlmCost {
            input: Decimal::new(1, 0),
            cache_write: Decimal::ZERO,
            cache_read: Decimal::ZERO,
            output: Decimal::new(10, 0),
        };
        let lower_total = LlmCost {
            input: Decimal::new(5, 0),
            cache_write: Decimal::ZERO,
            cache_read: Decimal::ZERO,
            output: Decimal::new(3, 0),
        };
        let equal_total_different_shape = LlmCost {
            input: Decimal::new(0, 0),
            cache_write: Decimal::ZERO,
            cache_read: Decimal::ZERO,
            output: Decimal::new(11, 0),
        };

        assert!(higher_total > lower_total);
        assert_ne!(higher_total, equal_total_different_shape);
        assert_ne!(
            higher_total.cmp(&equal_total_different_shape),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn line_with_cost_parse_returns_some_for_assistant_messages() {
        let parsed = LineWithCost::<PiLogLine>::parse(&assistant_message_json().to_string())
            .unwrap()
            .unwrap();

        assert_eq!(parsed.id, "a1");
        assert_eq!(parsed.timestamp, timestamp());
        assert_eq!(
            parsed.model,
            PiModel {
                provider: Provider::Anthropic,
                model: "claude-sonnet-4-5".to_string(),
            }
        );
        assert_eq!(parsed.cost.total(), Decimal::new(11, 0));
    }

    #[test]
    fn line_with_cost_parse_returns_none_for_non_assistant_messages() {
        let cases = [
            user_message_json(),
            tool_result_message_json(),
            session_json(),
            model_change_json(),
            thinking_level_change_json(),
            custom_json(),
            custom_message_json(),
        ];

        for (index, value) in cases.into_iter().enumerate() {
            assert!(
                LineWithCost::<PiLogLine>::parse(&value.to_string())
                    .unwrap()
                    .is_none(),
                "case {index} should return None: {value}"
            );
        }
    }

    #[derive(Debug, Clone)]
    struct AsymmetricMockLog {
        timestamp: DateTime<Utc>,
        cost: Option<LlmCost>,
        model: Option<String>,
    }

    impl AnalyzableLog for AsymmetricMockLog {
        type LogId = String;
        type ModelId = String;

        fn cost(&self) -> Option<LlmCost> {
            self.cost
        }

        fn identifier(&self) -> Self::LogId {
            "asymmetric".to_string()
        }

        fn model(&self) -> Option<Self::ModelId> {
            self.model.clone()
        }

        fn timestamp(&self) -> DateTime<Utc> {
            self.timestamp
        }

        fn parse(value: &str) -> miette::Result<Self> {
            match value {
                "cost-only" => Ok(Self {
                    timestamp: timestamp(),
                    cost: Some(LlmCost {
                        input: Decimal::new(1, 0),
                        cache_write: Decimal::ZERO,
                        cache_read: Decimal::ZERO,
                        output: Decimal::ZERO,
                    }),
                    model: None,
                }),
                "model-only" => Ok(Self {
                    timestamp: timestamp(),
                    cost: None,
                    model: Some("model-a".to_string()),
                }),
                _ => panic!("unexpected asymmetric mock input: {value}"),
            }
        }
    }

    #[test]
    fn line_with_cost_parse_returns_none_when_only_cost_or_model_is_present() {
        assert!(LineWithCost::<AsymmetricMockLog>::parse("cost-only")
            .unwrap()
            .is_none());
        assert!(LineWithCost::<AsymmetricMockLog>::parse("model-only")
            .unwrap()
            .is_none());
    }

    #[test]
    fn line_with_cost_parse_returns_error_for_invalid_json() {
        let error = LineWithCost::<PiLogLine>::parse("not-json").unwrap_err();

        assert!(format!("{error}").contains("failed to parse log line"));
    }

    #[test]
    fn pi_log_line_trait_methods_cover_all_variants() {
        let cases = [
            (
                session_json(),
                "019dc252-e50e-766c-8182-d654b46881af",
                false,
                false,
            ),
            (model_change_json(), "m1", false, false),
            (thinking_level_change_json(), "t1", false, false),
            (custom_json(), "c1", false, false),
            (custom_message_json(), "cm1", false, false),
            (user_message_json(), "u1", false, false),
            (tool_result_message_json(), "tr1", false, false),
            (assistant_message_json(), "a1", true, true),
        ];

        for (value, expected_id, expect_cost, expect_model) in cases {
            let line = parse_pi_log(value);

            assert_eq!(line.identifier(), expected_id);
            assert_eq!(line.timestamp(), timestamp());
            assert_eq!(line.cost().is_some(), expect_cost);
            assert_eq!(line.model().is_some(), expect_model);
        }
    }
}
