use super::*;

fn seg(t: Option<f64>, speaker: Option<&str>, text: &str) -> Segment {
    Segment {
        t,
        speaker: speaker.map(String::from),
        text: text.to_string(),
    }
}

/// `n` blocks, each ~1000 chars, 40s apart so none merge; block `hit` mentions pricing.
fn long_blocks(n: usize, hit: Option<usize>) -> Vec<Block> {
    let segs: Vec<Segment> = (0..n)
        .map(|i| {
            let filler = format!("filler{i} ").repeat(140);
            let text = if Some(i) == hit {
                format!("we settled the pricing question {filler}")
            } else {
                filler
            };
            seg(Some(i as f64 * 40.0), Some("Ana"), &text)
        })
        .collect();
    blocks(&segs)
}

#[test]
fn blocks_merge_same_speaker_within_30s_and_label() {
    let segs = vec![
        seg(None, Some("Ana"), "dropped, nothing before it"),
        seg(Some(754.0), Some("Ana"), "first"),
        seg(Some(760.0), Some("Ana"), "second"),
        seg(Some(790.0), Some("Ana"), "past thirty seconds"),
        seg(Some(800.0), Some("Ben"), "other speaker"),
        seg(None, None, "untimed joins previous"),
    ];
    let b = blocks(&segs);
    assert_eq!(b.len(), 3);
    assert_eq!(b[0].label, "12:34");
    assert_eq!(b[0].t, 754.0);
    assert_eq!(b[0].line, "[12:34] Ana: first second");
    assert_eq!(b[1].line, "[13:10] Ana: past thirty seconds");
    assert_eq!(
        b[2].line,
        "[13:20] Ben: other speaker untimed joins previous"
    );
}

#[test]
fn cited_time_must_match_an_evidence_label() {
    let b = blocks(&[seg(Some(754.0), Some("Ana"), "hello")]);
    let ev = for_entry(&b, &[]);
    assert_eq!(ev.resolve("12:34"), Some(754.0));
    assert_eq!(ev.resolve("[12:34]"), Some(754.0));
    assert_eq!(ev.resolve(" [12:34] "), Some(754.0));
    assert_eq!(ev.resolve("12:3"), None);
    assert_eq!(ev.resolve("[99:99]"), None);
    // a real moment of the recording that is not a block start
    assert_eq!(ev.resolve("12:40"), None);
    assert_eq!(ev.resolve("754"), None);
}

#[test]
fn for_entry_under_budget_is_whole_transcript() {
    let b = blocks(&[
        seg(Some(0.0), Some("Ana"), "one"),
        seg(Some(60.0), Some("Ben"), "two"),
        seg(Some(120.0), Some("Ana"), "three"),
    ]);
    let ev = for_entry(&b, &["nothing".into()]);
    assert_eq!(
        ev.text,
        "[00:00] Ana: one\n[01:00] Ben: two\n[02:00] Ana: three"
    );
    assert!(!ev.text.contains('…'));
}

#[test]
fn for_entry_over_budget_keeps_hits_in_order() {
    let b = long_blocks(60, Some(30));
    let ev = for_entry(&b, &["pricing".to_string()]);
    assert!(ev.text.len() <= EVIDENCE_CHAR_BUDGET, "{}", ev.text.len());
    assert!(ev.text.contains(&b[0].line) && ev.text.contains(&b[2].line));
    assert!(ev.text.contains(&b[57].line) && ev.text.contains(&b[59].line));
    assert!(ev.text.contains(&b[30].line), "the hit block is kept");
    assert!(ev.text.contains("[…]"));
    let positions: Vec<usize> = b.iter().filter_map(|x| ev.text.find(&x.line)).collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "chronological");
    assert_eq!(ev.resolve(&b[30].label), Some(1200.0));
    // a block left out of the text cannot be cited
    let missing = b.iter().find(|x| !ev.text.contains(&x.line)).unwrap();
    assert_eq!(ev.resolve(&missing.label), None);
}

#[test]
fn for_mentions_empty_without_term_hits() {
    let b = long_blocks(5, None);
    let ev = for_mentions(&b, &["pricing".to_string()]);
    assert!(ev.text.is_empty());
    assert_eq!(ev.resolve("00:00"), None);
}

#[test]
fn for_mentions_keeps_hit_with_neighbours() {
    let b = long_blocks(10, Some(5));
    let ev = for_mentions(&b, &["pricing".to_string()]);
    for i in [4, 5, 6] {
        assert!(ev.text.contains(&b[i].line));
    }
    assert!(!ev.text.contains(&b[2].line));
    assert!(ev.text.starts_with("[…]"));
}

#[test]
fn topic_terms_drop_stopwords_and_short_tokens() {
    let t = topic_terms(
        "Atlas CRM rebuild",
        "What the team decided about the CRM and its data migration",
    );
    assert_eq!(t[0], "atlas crm rebuild");
    assert!(t.contains(&"atlas".to_string()));
    assert!(t.contains(&"rebuild".to_string()));
    assert!(t.contains(&"migration".to_string()));
    assert!(t.contains(&"data".to_string()));
    assert!(!t.contains(&"crm".to_string()), "short token");
    assert!(!t.contains(&"about".to_string()), "stopword");
    assert!(!t.contains(&"what".to_string()), "stopword");
    assert_eq!(
        t.iter().filter(|x| *x == "atlas").count(),
        1,
        "deduplicated"
    );
}
