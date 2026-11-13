use super::*;
use crate::topics::evidence::{blocks, for_entry, Segment};

fn topic(id: &str, name: &str, description: &str) -> Topic {
    Topic {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        status: "open".into(),
        notes_markdown: None,
        notes_json: None,
        created_at: String::new(),
        updated_at: String::new(),
        closed_at: None,
    }
}

fn decision(n: &str, text: &str) -> OpenDecision {
    OpenDecision {
        entry_id: format!("te-{n}"),
        decision_id: format!("dec-{n}"),
        text: text.into(),
        occurred_at: "2026-10-01T09:00:00Z".into(),
    }
}

/// Evidence with block labels 00:10, 01:10, 02:10.
fn evidence() -> Evidence {
    let segs: Vec<Segment> = [10.0, 70.0, 130.0]
        .iter()
        .map(|t| Segment {
            t: Some(*t),
            speaker: Some("Ana".into()),
            text: "Atlas CRM rebuild talk.".into(),
        })
        .collect();
    for_entry(&blocks(&segs), &[])
}

#[test]
fn parse_entry_drops_bad_times_and_out_of_range_revisions() {
    let ds = [decision("a", "Ship in Q4"), decision("b", "Use Postgres")];
    let reply = r#"{"bullets":[
        {"text":"Scope agreed","t":"01:10"},
        {"text":"Hallucinated time","t":"[99:99]"},
        {"text":"Reformatted time","t":"1:1"},
        {"text":"No time","t":null},
        {"text":"  ","t":"01:10"}],
      "decisions":[{"text":"Cut the reporting module","t":"[02:10]"}],
      "revises":[
        {"decision":2,"note":"Moving to SQLite","t":"00:10"},
        {"decision":3,"note":"out of range","t":null},
        {"decision":0,"note":"zero","t":null},
        {"decision":-1,"note":"negative","t":null},
        {"decision":1,"note":"","t":null}]}"#;
    let p = parse_entry(reply, &evidence(), &ds).unwrap();
    let times: Vec<Option<f64>> = p.body.bullets.iter().map(|b| b.t).collect();
    assert_eq!(times, vec![Some(70.0), None, None, None]);
    assert_eq!(p.body.bullets[1].text, "Hallucinated time");
    assert_eq!(p.body.decisions.len(), 1);
    assert_eq!(p.body.decisions[0].t, Some(130.0));
    assert!(
        p.body.decisions[0].id.is_empty(),
        "ids are minted at insert"
    );
    assert_eq!(p.revisions.len(), 1);
    assert_eq!(p.revisions[0].0, ds[1]);
    assert_eq!(p.revisions[0].1, "Moving to SQLite");
    assert_eq!(p.revisions[0].2, Some(10.0));
}

#[test]
fn mention_without_valid_time_is_dropped() {
    let ts = [
        topic("t1", "Atlas CRM rebuild", ""),
        topic("t2", "Hiring", ""),
    ];
    let reply = r#"{"mentions":[
        {"topic":1,"summary":"Valid","t":"01:10"},
        {"topic":1,"summary":"Bad time","t":"99:99"},
        {"topic":1,"summary":"Null time","t":null},
        {"topic":1,"summary":"Missing time"},
        {"topic":5,"summary":"Out of range","t":"01:10"},
        {"topic":2,"summary":"Bracketed","t":"[02:10]"}]}"#;
    let m = parse_mentions(reply, &evidence(), &ts).unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!(
        (m[0].0.id.as_str(), m[0].1.as_str(), m[0].2),
        ("t1", "Valid", 70.0)
    );
    assert_eq!((m[1].0.id.as_str(), m[1].2), ("t2", 130.0));
}

#[test]
fn mentions_capped_per_topic() {
    let ts = [topic("t1", "Atlas", "")];
    let reply = r#"{"mentions":[
        {"topic":1,"summary":"a","t":"00:10"},{"topic":1,"summary":"b","t":"01:10"},
        {"topic":1,"summary":"c","t":"02:10"},{"topic":1,"summary":"d","t":"00:10"}]}"#;
    let m = parse_mentions(reply, &evidence(), &ts).unwrap();
    assert_eq!(m.len(), MAX_MENTIONS_PER_TOPIC);
    assert_eq!(m.last().unwrap().1, "c");
}

#[test]
fn parse_entry_accepts_bare_string_items() {
    // Small local models (the built-in qwen3.5:2b) ignore the item objects and reply with
    // plain strings; those used to be dropped, saving an empty entry.
    let reply = r#"{"bullets":["Maya will reproduce the drift on rev B.","  "],"decisions":["Freeze date is Sept 25."],"revises":[]}"#;
    let p = parse_entry(reply, &evidence(), &[]).unwrap();
    let bullets: Vec<_> = p
        .body
        .bullets
        .iter()
        .map(|b| (b.text.as_str(), b.t))
        .collect();
    assert_eq!(
        bullets,
        vec![("Maya will reproduce the drift on rev B.", None)]
    );
    let decisions: Vec<_> = p
        .body
        .decisions
        .iter()
        .map(|d| (d.text.as_str(), d.t))
        .collect();
    assert_eq!(decisions, vec![("Freeze date is Sept 25.", None)]);
}

#[test]
fn parse_entry_caps_bullets_and_decisions() {
    let many = |n: usize| {
        (0..n)
            .map(|i| format!("\"point {i}\""))
            .collect::<Vec<_>>()
            .join(",")
    };
    let reply = format!(
        r#"{{"bullets":[{}],"decisions":[{}],"revises":[]}}"#,
        many(12),
        many(9)
    );
    let p = parse_entry(&reply, &evidence(), &[]).unwrap();
    assert_eq!(p.body.bullets.len(), 8);
    assert_eq!(p.body.bullets[7].text, "point 7");
    assert_eq!(p.body.decisions.len(), 5);
}

#[test]
fn parse_tolerates_prose_around_json() {
    let ts = [topic("t1", "Atlas", "")];
    let wrapped = "<think>maybe {\"x\":1}</think>Sure! Here you go:\n```json\n{\"mentions\":[{\"topic\":1,\"summary\":\"s\",\"t\":\"00:10\"}]}\n```\nHope that helps.";
    assert_eq!(parse_mentions(wrapped, &evidence(), &ts).unwrap().len(), 1);
    let e = parse_entry(
        "Result: {\"bullets\":[{\"text\":\"x\",\"t\":null}]} done",
        &evidence(),
        &[],
    )
    .unwrap();
    assert_eq!(e.body.bullets.len(), 1);
    assert!(e.body.decisions.is_empty() && e.revisions.is_empty());
}

#[test]
fn parse_errors_without_json_and_never_quote_the_reply() {
    let err = parse_entry("no braces about SECRETWORD", &evidence(), &[]).unwrap_err();
    assert!(!err.to_string().contains("SECRETWORD"));
    let err = parse_mentions("{SECRETWORD: }", &evidence(), &[]).unwrap_err();
    assert!(!err.to_string().contains("SECRETWORD"));
}

#[test]
fn prompts_contain_pinned_instructions() {
    let t = topic("t1", "Atlas CRM rebuild", "Migration of the sales CRM");
    let ds = [decision("a", "Ship in Q4"), decision("b", "Use Postgres")];
    let ev = evidence();
    let (sys, user) = entry_prompt(&t, "Summary text", Some("my notes"), &ev, &ds);
    for s in [
        "Write only about the topic; ignore everything else in the meeting.",
        "If the meeting did not discuss the topic, return empty bullets and decisions.",
        "Write at most 8 bullets and at most 5 decisions, each different from the others; never repeat a point.",
        "Only list a revision when the meeting clearly changes a numbered decision.",
        "Cite times only as they appear in square brackets in the transcript",
    ] {
        assert!(sys.contains(s), "missing: {s}");
    }
    assert!(user.contains(
        "The user's description of the topic (used to recognise it): Migration of the sales CRM"
    ));
    assert!(user.contains("1. Ship in Q4\n2. Use Postgres"));
    assert!(user.contains("my notes") && user.contains("Summary text"));
    assert!(user.contains("[00:10] Ana:"));

    let (sys, user) = mention_prompt(&[t.clone(), topic("t2", "Hiring", "")], "Summary text", &ev);
    for s in [
        "Only report a mention when the meeting actually discusses the topic, not when a word merely matches. When unsure, report nothing.",
        "Cite times only as they appear in square brackets in the transcript",
    ] {
        assert!(sys.contains(s), "missing: {s}");
    }
    assert!(user.contains("1. Atlas CRM rebuild (the user's description, used to recognise the topic: Migration of the sales CRM)"));
    assert!(user.contains("2. Hiring\n"));
}

#[test]
fn schemas_are_strict() {
    for (schema, keys) in [
        (entry_schema(), vec!["bullets", "decisions", "revises"]),
        (mention_schema(), vec!["mentions"]),
    ] {
        assert_eq!(schema["additionalProperties"], false);
        let req: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(req, keys);
        for k in keys {
            let item = &schema["properties"][k]["items"];
            assert_eq!(item["additionalProperties"], false);
            assert_eq!(
                item["required"].as_array().unwrap().len(),
                item["properties"].as_object().unwrap().len()
            );
        }
    }
}
