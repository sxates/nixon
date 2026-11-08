use super::*;

const SUCCESS: &str = include_str!("../../tests/fixtures/cli/claude/success-stream.ndjson");
const SCHEMA: &str = include_str!("../../tests/fixtures/cli/claude/success-json-schema.ndjson");
const SIGNED_OUT: &str =
    include_str!("../../tests/fixtures/cli/claude/not-signed-in-stream.ndjson");
const LIMIT: &str =
    include_str!("../../tests/fixtures/cli/claude/usage-limit-stream.SYNTHETIC.ndjson");
const API_RETRY_AUTH: &str =
    include_str!("../../tests/fixtures/cli/claude/api-retry-auth.SYNTHETIC.ndjson");

/// The capture's `system/init` line (line 2 of every stream capture).
fn init_line() -> &'static str {
    SUCCESS.lines().nth(1).unwrap()
}

fn init() -> Value {
    serde_json::from_str(init_line()).unwrap()
}

/// A stream: the capture's init plus `lines`.
fn stream(lines: &[Value]) -> String {
    let mut s = format!("{}\n", init_line());
    for l in lines {
        s.push_str(&l.to_string());
        s.push('\n');
    }
    s
}

fn failed_result(text: &str) -> Value {
    json!({"type": "result", "subtype": "success", "is_error": true, "result": text})
}

fn run(stdout: &str, exit: i32, schema: bool) -> Result<String, CliError> {
    interpret(stdout.as_bytes(), exit, schema)
}

#[test]
fn success_capture_returns_the_result_text() {
    let reply = run(SUCCESS, 0, false).unwrap();
    assert!(
        reply.starts_with("**Summary (partial transcript)**"),
        "{reply}"
    );
    assert!(reply.contains("Exponential backoff"));
}

#[test]
fn schema_capture_returns_structured_output_as_json() {
    let reply = run(SCHEMA, 0, true).unwrap();
    let parsed: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(parsed["action_items"].as_array().unwrap().len(), 4);
}

#[test]
fn a_schema_run_without_structured_output_is_bad_output() {
    // init with tools [] is fine on a schema run; the missing output is the failure.
    assert_eq!(run(SUCCESS, 0, true), Err(CliError::BadOutput));
    let null_output =
        json!({"type": "result", "is_error": false, "result": "{}", "structured_output": null});
    assert_eq!(
        run(&stream(&[null_output]), 0, true),
        Err(CliError::BadOutput)
    );
}

#[test]
fn structured_output_tool_without_a_schema_is_an_isolation_failure() {
    assert_eq!(run(SCHEMA, 0, false), Err(CliError::IsolationFailed));
}

#[test]
fn signed_out_capture_is_not_signed_in_despite_subtype_success() {
    assert_eq!(run(SIGNED_OUT, 1, false), Err(CliError::NotSignedIn));
}

#[test]
fn usage_limit_capture_keeps_the_reset_time() {
    // Its assistant error says "rate_limit"; the plan wording must win.
    assert_eq!(
        run(LIMIT, 1, false),
        Err(CliError::UsageLimit {
            detail: "resets 3:45pm".into()
        })
    );
}

#[test]
fn api_retry_auth_failure_is_not_signed_in() {
    let s = format!("{}\n{API_RETRY_AUTH}", init_line());
    assert_eq!(run(&s, 1, false), Err(CliError::NotSignedIn));
    let mut wire = Tripwire::new(false);
    assert_eq!(wire.check_line(init_line().as_bytes()), Ok(()));
    assert_eq!(
        wire.check_line(API_RETRY_AUTH.trim().as_bytes()),
        Err(CliError::NotSignedIn)
    );
}

#[test]
fn auth_wording_variants_are_not_signed_in() {
    for text in [
        "Not logged in · Please run /login",
        "Login expired · Please run /login",
        "Authentication required",
        "OAuth token has expired. Please obtain a new token",
    ] {
        assert_eq!(
            run(&stream(&[failed_result(text)]), 1, false),
            Err(CliError::NotSignedIn),
            "{text}"
        );
    }
}

#[test]
fn plan_limit_wording_variants_are_usage_limit() {
    let cases = [
        ("You've hit your Sonnet limit · resets 5pm", "resets 5pm"),
        (
            "You've hit your weekly limit. Resets Mon 9am.",
            "Resets Mon 9am",
        ),
        ("Credit balance is too low", ""),
    ];
    for (text, detail) in cases {
        assert_eq!(
            run(&stream(&[failed_result(text)]), 1, false),
            Err(CliError::UsageLimit {
                detail: detail.into()
            }),
            "{text}"
        );
    }
    let billing =
        json!({"type": "assistant", "error": "billing_error", "message": {"content": []}});
    assert!(matches!(
        run(&stream(&[billing, failed_result("API Error")]), 1, false),
        Err(CliError::UsageLimit { .. })
    ));
}

#[test]
fn a_rejected_rate_limit_event_is_usage_limit_with_its_reset_time() {
    let event = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "rejected", "resetsAt": 1_800_000_000_i64}});
    let Err(CliError::UsageLimit { detail }) =
        run(&stream(&[event, failed_result("API Error")]), 1, false)
    else {
        panic!("not a usage limit");
    };
    assert!(detail.starts_with("resets "), "{detail}");
    // An allowed event (every success capture has one) is no signal.
    assert_eq!(
        run(
            &stream(&[
                json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed"}}),
                failed_result("boom")
            ]),
            1,
            false
        ),
        Err(CliError::Unknown)
    );
}

#[test]
fn rate_limit_without_plan_wording_is_transient() {
    let overloaded = json!({"type": "assistant", "error": "overloaded", "message": {"content": [{"type": "text", "text": "API Error"}]}});
    let status_429 =
        json!({"type": "result", "is_error": true, "api_error_status": 429, "result": "API Error"});
    for lines in [
        vec![overloaded, failed_result("API Error")],
        vec![status_429],
    ] {
        assert_eq!(run(&stream(&lines), 1, false), Err(CliError::RateLimited));
    }
}

#[test]
fn classification_ignores_subtype() {
    // A successful answer with an alarming subtype is still the answer...
    let ok = json!({"type": "result", "subtype": "error_during_execution", "is_error": false, "result": "fine"});
    assert_eq!(run(&stream(&[ok]), 0, false), Ok("fine".into()));
    // ...and an unrecognised failure is Unknown, whatever its subtype says.
    let failed = failed_result("Something odd happened");
    assert_eq!(run(&stream(&[failed]), 1, false), Err(CliError::Unknown));
}

#[test]
fn missing_or_empty_answers_are_bad_output_or_a_crash() {
    let empty = json!({"type": "result", "is_error": false, "result": "  \n"});
    assert_eq!(run(&stream(&[empty]), 0, false), Err(CliError::BadOutput));
    // No result: exit 0 is unreadable output, non-zero is a crash.
    assert_eq!(run(&stream(&[]), 0, false), Err(CliError::BadOutput));
    assert_eq!(
        run(&stream(&[]), 3, false),
        Err(CliError::Crashed { code: 3 })
    );
    // Garbage, and an answer that never came with an init, are both discarded.
    assert_eq!(run("not json\n<html>", 0, false), Err(CliError::BadOutput));
    let tail: String = SUCCESS.lines().skip(2).map(|l| format!("{l}\n")).collect();
    assert_eq!(run(&tail, 0, false), Err(CliError::BadOutput));
    assert_eq!(run("", 2, false), Err(CliError::Crashed { code: 2 }));
}

#[test]
fn init_allowlist() {
    let mut bad_tools = init();
    bad_tools["tools"] = json!(["Bash"]);
    let mut extra_with_schema = init();
    extra_with_schema["tools"] = json!(["StructuredOutput", "Read"]);
    let mut mcp = init();
    mcp["mcp_servers"] = json!([{"name": "x", "status": "connected"}]);
    let mut user_plugin = init();
    user_plugin["plugins"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "mine", "path": "/Users/user/.claude/plugins/mine"}));
    let mut no_plugins_field = init();
    no_plugins_field.as_object_mut().unwrap().remove("plugins");
    let mut no_tools_field = init();
    no_tools_field.as_object_mut().unwrap().remove("tools");

    assert_eq!(check_init(&init(), false), Ok(()));
    assert_eq!(check_init(&init(), true), Ok(()));
    let mut structured = init();
    structured["tools"] = json!(["StructuredOutput"]);
    assert_eq!(check_init(&structured, true), Ok(()));
    for (bad, schema) in [
        (&structured, false),
        (&bad_tools, false),
        (&bad_tools, true),
        (&extra_with_schema, true),
        (&mcp, false),
        (&user_plugin, false),
        (&no_plugins_field, false),
        (&no_tools_field, false),
    ] {
        assert_eq!(
            check_init(bad, schema),
            Err(CliError::IsolationFailed),
            "{bad}"
        );
    }
}

#[test]
fn tripwire_skips_events_before_init_and_checks_only_the_first_init() {
    let mut wire = Tripwire::new(false);
    let ui = SUCCESS.lines().next().unwrap();
    assert_eq!(wire.check_line(ui.as_bytes()), Ok(()));
    assert_eq!(wire.check_line(b"not json"), Ok(()));
    let mut bad = init();
    bad["tools"] = json!(["Bash"]);
    assert_eq!(
        wire.check_line(bad.to_string().as_bytes()),
        Err(CliError::IsolationFailed)
    );
    let mut wire = Tripwire::new(false);
    for line in SUCCESS.lines() {
        assert_eq!(wire.check_line(line.as_bytes()), Ok(()));
    }
}

#[test]
fn argv_is_the_spec_order_and_the_schema_goes_last() {
    let want = [
        "-p",
        "--safe-mode",
        "--restricted",
        "--tools",
        "",
        "--disallowedTools",
        "mcp__*",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--permission-mode",
        "dontAsk",
        "--permission-prompts",
        "none",
        "--no-session-persistence",
        "--max-turns",
        "1",
        "--system-prompt-file",
        "{in}/system.txt",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--model",
        "sonnet",
    ];
    assert_eq!(args(None), want.map(OsString::from));
    let schema = json!({"type": "object", "required": ["a"]});
    let with = args(Some(&schema));
    assert_eq!(&with[..want.len()], want.map(OsString::from));
    assert_eq!(
        &with[want.len()..],
        [
            OsString::from("--json-schema"),
            OsString::from(r#"{"required":["a"],"type":"object"}"#)
        ]
    );
}

#[test]
fn stdin_is_one_escaped_stream_json_line() {
    let line = stdin_message("He said \"hi\"\nthen left");
    assert_eq!(line.last(), Some(&b'\n'));
    assert_eq!(line.iter().filter(|b| **b == b'\n').count(), 1);
    let v: Value = serde_json::from_slice(&line).unwrap();
    assert_eq!(
        v,
        json!({"type": "user", "message": {"role": "user", "content": "He said \"hi\"\nthen left"}})
    );
}

#[test]
fn reset_detail_stops_at_the_sentence_end_or_a_parenthesis() {
    // Never carry upgrade or marketing text into the user copy.
    let cases = [
        (
            "You've hit your limit · resets 3pm (America/New_York). Upgrade at example.com",
            "resets 3pm",
        ),
        ("You've hit your limit (resets 5pm)", "resets 5pm"),
        (
            "You've hit your weekly limit. Resets Mon 9am. Upgrade now",
            "Resets Mon 9am",
        ),
        (
            "You've hit your session limit · resets 3:45pm",
            "resets 3:45pm",
        ),
        (
            "You've hit your limit · resets 3.45pm! Get more usage",
            "resets 3.45pm",
        ),
    ];
    for (text, detail) in cases {
        assert_eq!(
            run(&stream(&[failed_result(text)]), 1, false),
            Err(CliError::UsageLimit {
                detail: detail.into()
            }),
            "{text}"
        );
    }
}

#[test]
fn plural_limits_are_a_plan_limit() {
    assert_eq!(
        run(
            &stream(&[failed_result("You've hit your usage limits · resets 5pm")]),
            1,
            false
        ),
        Err(CliError::UsageLimit {
            detail: "resets 5pm".into()
        })
    );
}

#[test]
fn hit_your_rate_limit_is_transient_not_a_plan_limit() {
    for text in [
        "You've hit your rate limit. Please wait a moment.",
        "You've hit your rate limits",
    ] {
        assert_eq!(
            run(&stream(&[failed_result(text)]), 1, false),
            Err(CliError::RateLimited),
            "{text}"
        );
    }
}
