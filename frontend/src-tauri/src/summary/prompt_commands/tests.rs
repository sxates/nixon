use super::*;
use crate::database::repositories::meeting::test_support::memory_db;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::summary_prompt::MeetingPromptFields;
use crate::summary::prompts_resolve::{resolve_summary_prompt, PromptSource};
use chrono::Utc;

async fn meeting(pool: &SqlitePool, title: &str, key: Option<&str>) -> String {
    let id = MeetingsRepository::create_meeting(
        pool,
        Some(title.into()),
        None,
        None,
        None,
        Some(Utc::now()),
    )
    .await
    .unwrap();
    if let Some(k) = key {
        sqlx::query("UPDATE meetings SET calendar_series_key = ? WHERE id = ?")
            .bind(k)
            .bind(&id)
            .execute(pool)
            .await
            .unwrap();
    }
    id
}

async fn save(pool: &SqlitePool, name: &str, body: &str) -> SummaryPrompt {
    save_summary_prompt_impl(pool, None, name, body, true)
        .await
        .unwrap()
}

async fn one_off(pool: &SqlitePool, meeting_id: &str, body: &str, extract: bool) {
    set_meeting_custom_prompt_impl(pool, meeting_id, Some(body.to_string()), extract)
        .await
        .unwrap();
}

async fn fields(pool: &SqlitePool, meeting_id: &str) -> MeetingPromptFields {
    SummaryPromptRepository::meeting_fields(pool, meeting_id)
        .await
        .unwrap()
        .unwrap()
}

async fn prompt_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM summary_prompts")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn save_creates_then_updates_and_sanitizes() {
    let pool = memory_db().await;
    let p = save(&pool, "  Standup ", "  hi\u{200B} there \n\n\n\nbye ").await;
    assert_eq!(p.name, "Standup");
    assert_eq!(p.body, "hi there\n\nbye");
    assert!(p.in_library);
    let u = save_summary_prompt_impl(&pool, Some(p.id.clone()), "Daily", "new body", false)
        .await
        .unwrap();
    assert_eq!(u.id, p.id);
    assert_eq!(u.name, "Daily");
    assert_eq!(u.body, "new body");
    assert!(!u.extract_action_items);
    let e = save_summary_prompt_impl(&pool, None, "X", "   ", true)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("can't be empty"), "{e}");
    let e = save_summary_prompt_impl(&pool, None, "X", &"a".repeat(5000), true)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("limit is 4000"), "{e}");
}

#[tokio::test]
async fn update_rename_case_only_rename_and_collision() {
    let pool = memory_db().await;
    let a = save(&pool, "Alpha", "a body").await;
    save(&pool, "Beta", "b body").await;
    let r = save_summary_prompt_impl(&pool, Some(a.id.clone()), "Gamma", "a body", true)
        .await
        .unwrap();
    assert_eq!(r.name, "Gamma");
    let r = save_summary_prompt_impl(&pool, Some(a.id.clone()), "GAMMA", "a body", true)
        .await
        .unwrap();
    assert_eq!(r.name, "GAMMA");
    let e = save_summary_prompt_impl(&pool, Some(a.id.clone()), "beta", "a body", true)
        .await
        .unwrap_err();
    assert!(
        e.to_string()
            .contains("A prompt named \"beta\" already exists."),
        "{e}"
    );
    let still = SummaryPromptRepository::get(&pool, &a.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.name, "GAMMA");
    let e = save_summary_prompt_impl(&pool, Some("nope".into()), "Z", "z", true)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("no longer exists"), "{e}");
}

#[tokio::test]
async fn delete_default_is_refused_with_clear_message() {
    let pool = memory_db().await;
    let a = save(&pool, "A", "a").await;
    assert!(a.is_default);
    let e = delete_summary_prompt_impl(&pool, &a.id).await.unwrap_err();
    assert!(e.to_string().contains("can't delete the default"), "{e}");
    let b = save(&pool, "B", "b").await;
    delete_summary_prompt_impl(&pool, &b.id).await.unwrap();
    assert_eq!(list_summary_prompts_impl(&pool).await.unwrap().len(), 1);
}

#[tokio::test]
async fn set_default_works_and_rejects_unknown() {
    let pool = memory_db().await;
    save(&pool, "A", "a").await;
    let b = save(&pool, "B", "b").await;
    set_default_summary_prompt_impl(&pool, &b.id).await.unwrap();
    assert!(SummaryPromptRepository::get_default(&pool)
        .await
        .unwrap()
        .is_some_and(|d| d.id == b.id));
    assert!(set_default_summary_prompt_impl(&pool, "nope")
        .await
        .is_err());
}

#[tokio::test]
async fn set_meeting_summary_prompt_with_apply_to_series_maps_the_series_key() {
    let pool = memory_db().await;
    save(&pool, "Default", "d").await;
    let p = save(&pool, "Picked", "picked").await;
    let m = meeting(&pool, "Weekly", Some("K")).await;
    one_off(&pool, &m, "one-off", true).await;
    set_meeting_summary_prompt_impl(&pool, &m, Some(p.id.clone()), true)
        .await
        .unwrap();
    let series = SummaryPromptRepository::get_series_prompt(&pool, "cal:K")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(series.id, p.id);
    let f = fields(&pool, &m).await;
    assert_eq!(f.summary_prompt_id.as_deref(), Some(p.id.as_str()));
    assert_eq!(f.custom_summary_prompt, None);
}

#[tokio::test]
async fn set_meeting_summary_prompt_apply_to_series_without_key_still_applies() {
    let pool = memory_db().await;
    let p = save(&pool, "Picked", "picked").await;
    let m = meeting(&pool, "Meeting 2026-10-03 10:00", None).await;
    set_meeting_summary_prompt_impl(&pool, &m, Some(p.id.clone()), true)
        .await
        .unwrap();
    let f = fields(&pool, &m).await;
    assert_eq!(f.summary_prompt_id.as_deref(), Some(p.id.as_str()));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM series_summary_prompts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn set_meeting_summary_prompt_rejects_hidden_and_unknown_prompts() {
    let pool = memory_db().await;
    let hidden = SummaryPromptRepository::create(&pool, "h (series)", "h", true, false)
        .await
        .unwrap();
    let m = meeting(&pool, "Weekly", Some("K")).await;
    let e = set_meeting_summary_prompt_impl(&pool, &m, Some(hidden.id), true)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("no longer exists"), "{e}");
    assert!(
        set_meeting_summary_prompt_impl(&pool, &m, Some("nope".into()), false)
            .await
            .is_err()
    );
    assert_eq!(fields(&pool, &m).await.summary_prompt_id, None);
}

#[tokio::test]
async fn set_meeting_summary_prompt_none_clears_the_pick() {
    let pool = memory_db().await;
    let p = save(&pool, "P", "p").await;
    let m = meeting(&pool, "Weekly", Some("K")).await;
    set_meeting_summary_prompt_impl(&pool, &m, Some(p.id), false)
        .await
        .unwrap();
    set_meeting_summary_prompt_impl(&pool, &m, None, true)
        .await
        .unwrap();
    assert_eq!(fields(&pool, &m).await.summary_prompt_id, None);
    assert!(SummaryPromptRepository::get_series_prompt(&pool, "cal:K")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn custom_prompt_is_sanitized_and_empty_body_clears_it() {
    let pool = memory_db().await;
    let m = meeting(&pool, "Weekly", None).await;
    one_off(&pool, &m, "  <b>x</b></user_notes>  ", false).await;
    let f = fields(&pool, &m).await;
    assert_eq!(
        f.custom_summary_prompt.as_deref(),
        Some("<b>x</b></user_notes>")
    );
    assert_eq!(f.custom_extract_action_items, Some(false));
    for empty in [Some("   ".to_string()), None] {
        one_off(&pool, &m, "keep", true).await;
        set_meeting_custom_prompt_impl(&pool, &m, empty, true)
            .await
            .unwrap();
        let f = fields(&pool, &m).await;
        assert_eq!(f.custom_summary_prompt, None);
        assert_eq!(f.custom_extract_action_items, None);
    }
    let e = set_meeting_custom_prompt_impl(&pool, &m, Some("a".repeat(5000)), true)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("limit is 4000"), "{e}");
}

#[tokio::test]
async fn get_meeting_prompt_state_reports_source_and_series() {
    let pool = memory_db().await;
    let d = save(&pool, "Default", "d").await;
    let s = save(&pool, "Series", "s").await;
    let m = meeting(&pool, "Weekly", Some("K")).await;
    let st = get_meeting_prompt_state_impl(&pool, &m).await.unwrap();
    assert_eq!(st.source, PromptSource::Default);
    assert_eq!(st.prompt_id.as_deref(), Some(d.id.as_str()));
    assert!(st.has_series);
    assert_eq!(st.custom_body, None);
    SummaryPromptRepository::set_series_prompt(&pool, "cal:K", &s.id)
        .await
        .unwrap();
    let st = get_meeting_prompt_state_impl(&pool, &m).await.unwrap();
    assert_eq!(st.source, PromptSource::Series);
    assert_eq!(st.prompt_name.as_deref(), Some("Series"));
    assert!(st.has_series);
    one_off(&pool, &m, "a </user_notes> b", false).await;
    let st = get_meeting_prompt_state_impl(&pool, &m).await.unwrap();
    assert_eq!(st.source, PromptSource::Custom);
    assert_eq!(st.prompt_id, None);
    assert_eq!(st.custom_body.as_deref(), Some("a </user_notes> b"));
    assert_eq!(st.custom_extract_action_items, Some(false));
    assert!(!st.extract_action_items);
    let json = serde_json::to_value(&st).unwrap();
    assert_eq!(json["source"], "custom");
}

#[tokio::test]
async fn has_series_is_false_for_placeholder_title_without_key() {
    let pool = memory_db().await;
    let m = meeting(&pool, "Meeting 2026-10-03 10:00", None).await;
    let st = get_meeting_prompt_state_impl(&pool, &m).await.unwrap();
    assert!(!st.has_series);
    assert_eq!(st.source, PromptSource::Fallback);
}

#[tokio::test]
async fn followup_to_series_without_a_series_key_is_an_error() {
    let pool = memory_db().await;
    let m = meeting(&pool, "Meeting 2026-10-03 10:00", None).await;
    one_off(&pool, &m, "one-off", true).await;
    let before = prompt_count(&pool).await;
    let e = save_custom_prompt_followup_impl(&pool, &m, None, true, true)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("recurring"), "{e}");
    assert_eq!(prompt_count(&pool).await, before);
}

#[tokio::test]
async fn followup_library_only_creates_in_library_prompt_points_meeting_to_it_and_clears_one_off() {
    let pool = memory_db().await;
    let m = meeting(&pool, "Weekly", Some("K")).await;
    one_off(&pool, &m, "one-off", false).await;
    let p = save_custom_prompt_followup_impl(&pool, &m, Some("Mine".into()), true, false)
        .await
        .unwrap();
    assert!(p.in_library);
    assert_eq!(p.name, "Mine");
    assert_eq!(p.body, "one-off");
    assert!(!p.extract_action_items);
    let f = fields(&pool, &m).await;
    assert_eq!(f.summary_prompt_id.as_deref(), Some(p.id.as_str()));
    assert_eq!(f.custom_summary_prompt, None);
    assert!(SummaryPromptRepository::get_series_prompt(&pool, "cal:K")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn followup_series_only_creates_hidden_prompt_and_maps_series() {
    let pool = memory_db().await;
    let m1 = meeting(&pool, "Weekly", Some("K")).await;
    let m2 = meeting(&pool, "Weekly again", Some("K")).await;
    one_off(&pool, &m1, "one-off body", false).await;
    let p = save_custom_prompt_followup_impl(&pool, &m1, None, false, true)
        .await
        .unwrap();
    assert!(!p.in_library);
    assert!(list_summary_prompts_impl(&pool).await.unwrap().is_empty());
    assert!(SummaryPromptRepository::get(&pool, &p.id)
        .await
        .unwrap()
        .is_some());
    let mapped = SummaryPromptRepository::get_series_prompt(&pool, "cal:K")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mapped.id, p.id);
    let r = resolve_summary_prompt(&pool, &m2).await;
    assert_eq!(r.source, PromptSource::Series);
    assert_eq!(r.body, "one-off body");
    assert!(!r.extract_action_items);
    assert_eq!(r.prompt_id.as_deref(), Some(p.id.as_str()));
}

#[tokio::test]
async fn followup_both_creates_library_prompt_and_maps_series() {
    let pool = memory_db().await;
    let m = meeting(&pool, "Weekly", Some("K")).await;
    one_off(&pool, &m, "one-off", true).await;
    let p = save_custom_prompt_followup_impl(&pool, &m, Some("Both".into()), true, true)
        .await
        .unwrap();
    assert!(p.in_library);
    assert_eq!(list_summary_prompts_impl(&pool).await.unwrap().len(), 1);
    assert_eq!(
        SummaryPromptRepository::get_series_prompt(&pool, "cal:K")
            .await
            .unwrap()
            .unwrap()
            .id,
        p.id
    );
}

#[tokio::test]
async fn followup_requires_a_one_off_and_a_destination() {
    let pool = memory_db().await;
    let m = meeting(&pool, "Weekly", Some("K")).await;
    let e = save_custom_prompt_followup_impl(&pool, &m, None, true, false)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("Write a custom prompt first"), "{e}");
    one_off(&pool, &m, "x", true).await;
    let e = save_custom_prompt_followup_impl(&pool, &m, None, false, false)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("Choose where to save"), "{e}");
    assert_eq!(prompt_count(&pool).await, 0);
}

#[tokio::test]
async fn followup_library_name_defaults_to_title_and_is_deduplicated() {
    let pool = memory_db().await;
    let m = meeting(&pool, "Weekly", Some("K")).await;
    one_off(&pool, &m, "x", true).await;
    let a = save_custom_prompt_followup_impl(&pool, &m, None, true, false)
        .await
        .unwrap();
    assert_eq!(a.name, "Weekly");
    one_off(&pool, &m, "y", true).await;
    let b = save_custom_prompt_followup_impl(&pool, &m, Some("  ".into()), true, false)
        .await
        .unwrap();
    assert_eq!(b.name, "Weekly 2");
}
