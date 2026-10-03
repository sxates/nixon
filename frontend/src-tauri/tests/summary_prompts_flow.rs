//! specs/0079 saved-prompt flows through the public API (no Tauri runtime).

mod common;

use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::database::repositories::summary_prompt::{series_key_for, SummaryPromptRepository};
use app_lib::summary::prompts_resolve::{resolve_summary_prompt, PromptSource};
use common::fresh_db;

#[tokio::test]
async fn resolver_follows_series_across_meetings() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let default = SummaryPromptRepository::create(pool, "Default", "default body", true, true)
        .await
        .unwrap();
    let weekly = SummaryPromptRepository::create(pool, "Weekly", "weekly body", false, true)
        .await
        .unwrap();

    let first = MeetingsRepository::create_meeting(
        pool,
        Some("Weekly Sync".into()),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    // Occurrence 1 has no pick yet: default.
    let r = resolve_summary_prompt(pool, &first).await;
    assert_eq!(r.source, PromptSource::Default);
    assert_eq!(r.prompt_id.as_deref(), Some(default.id.as_str()));

    // Pick on occurrence 1 and remember it for the series.
    SummaryPromptRepository::set_meeting_prompt_id(pool, &first, Some(&weekly.id))
        .await
        .unwrap();
    let key = series_key_for(None, "Weekly Sync").unwrap();
    SummaryPromptRepository::set_series_prompt(pool, &key, &weekly.id)
        .await
        .unwrap();
    let r = resolve_summary_prompt(pool, &first).await;
    assert_eq!(r.source, PromptSource::Meeting);

    // Occurrence 2 (differently cased/spaced title) resolves to the series prompt.
    let second = MeetingsRepository::create_meeting(
        pool,
        Some("  weekly sync ".into()),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let r = resolve_summary_prompt(pool, &second).await;
    assert_eq!(r.source, PromptSource::Series);
    assert_eq!(r.prompt_id.as_deref(), Some(weekly.id.as_str()));
    assert_eq!(r.body, "weekly body");
    assert!(!r.extract_action_items);

    // An unrelated meeting is unaffected.
    let other = MeetingsRepository::create_meeting(
        pool,
        Some("Board review".into()),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let r = resolve_summary_prompt(pool, &other).await;
    assert_eq!(r.source, PromptSource::Default);
}
