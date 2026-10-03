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

#[tokio::test]
async fn final_system_prompt_embeds_resolved_one_off_prompt() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting =
        MeetingsRepository::create_meeting(pool, Some("Standup".into()), None, None, None, None)
            .await
            .unwrap();
    SummaryPromptRepository::set_meeting_custom_prompt(
        pool,
        &meeting,
        Some("Write exactly three bullets"),
        true,
    )
    .await
    .unwrap();

    let resolved = resolve_summary_prompt(pool, &meeting).await;
    assert_eq!(resolved.source, PromptSource::Custom);
    let system = app_lib::summary::prompts::build_final_synthesis_system_prompt(
        &resolved.body,
        "depth guidance",
        false,
        false,
        false,
    );
    assert!(system
        .contains("<summary_instructions>\nWrite exactly three bullets\n</summary_instructions>"));
}

/// A prompt that drops the title line must not rename the meeting or crash.
#[test]
fn markdown_without_title_line_yields_no_meeting_name() {
    assert_eq!(
        app_lib::summary::extract_meeting_name_from_markdown("## Decisions\n- ship it"),
        None
    );
}
