use std::path::PathBuf;

use pi_logs::{
    parse_file, CustomMessagePayload, CustomPayload, PiLogLine, RoleMessage, ToolResultDetails,
    WebSearchResultsPayload,
};

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

#[test]
fn parse_recent_fixture_file() {
    let path = fixture_path("tests/fixtures/pi_sessions_ok/recent_session.jsonl");
    let lines = parse_file(&path).expect("expected recent pi fixture to parse");

    assert_eq!(lines.len(), 5);
    assert!(matches!(lines[0], PiLogLine::Session(_)));

    let PiLogLine::Custom(custom) = &lines[1] else {
        panic!("expected dcp-state custom payload")
    };
    let CustomPayload::DcpState(state) = &custom.payload else {
        panic!("expected dcp-state payload")
    };
    assert_eq!(state.compression_blocks[0].supersedes_block_ids, vec![1, 2]);

    let PiLogLine::CustomMessage(message) = &lines[2] else {
        panic!("expected subagent control notice")
    };
    let CustomMessagePayload::SubagentControlNotice(details) = &message.payload else {
        panic!("expected subagent control notice payload")
    };
    assert_eq!(
        details.notice_text,
        "Subagent needs attention: documentation-reviewer"
    );

    let PiLogLine::Message(message) = &lines[3] else {
        panic!("expected routed tool result message")
    };
    let RoleMessage::ToolResult(details_message) = &message.message else {
        panic!("expected tool result role message")
    };
    let Some(ToolResultDetails::Mcp(details)) = &details_message.details else {
        panic!("expected mcp tool result details")
    };
    assert_eq!(details.hint_server.as_deref(), Some("project-tools"));

    let PiLogLine::Custom(custom) = &lines[4] else {
        panic!("expected web-search-results custom payload")
    };
    let CustomPayload::WebSearchResults(results) = &custom.payload else {
        panic!("expected web-search-results payload")
    };
    match &results.payload {
        WebSearchResultsPayload::Fetch(fetch) => {
            assert_eq!(fetch.urls.len(), 1);
            assert_eq!(fetch.urls[0].error, None);
        }
        other => panic!("expected fetch payload, got {other:?}"),
    }
}
