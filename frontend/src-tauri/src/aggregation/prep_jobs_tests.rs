//! specs/0074 W3 — prep briefs as queued work: the pass plans before it executes, an
//! up-to-date pass records nothing, Retry regenerates one brief, and the pass announces
//! changes. No LLM is configured in these tests, so every brief that reaches generation
//! fails fast with "no summary model" — which is all the queue behaviour needs.

use super::*;
use crate::database::manager::DatabaseManager;
use crate::llm_activity::{LlmActivityView, LlmTaskRegistry};
use tauri::test::{mock_app, MockRuntime};
use tauri::Listener;

struct Fixture {
    _dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    pool: SqlitePool,
    registry: Arc<LlmTaskRegistry>,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new(
        dir.path().join("t.sqlite").to_str().unwrap(),
        dir.path().join("none.db").to_str().unwrap(),
    )
    .await
    .unwrap();
    let pool = db.pool().clone();
    let registry = Arc::new(LlmTaskRegistry::new());
    let app = mock_app();
    app.manage(AppState { db_manager: db });
    app.manage(LlmActivityState(Arc::clone(&registry)));
    Fixture {
        _dir: dir,
        app,
        pool,
        registry,
    }
}

/// A recorded meeting with content (a transcript row), optionally in a calendar series.
async fn recorded(pool: &SqlitePool, title: &str, at: &str, series: Option<&str>) -> String {
    let id = format!("meeting-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, origin, calendar_series_key) \
         VALUES (?, ?, ?, ?, 'recorded', ?)",
    )
    .bind(&id)
    .bind(title)
    .bind(at)
    .bind(at)
    .bind(series)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES (?, ?, 'we shipped it', ?)")
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&id)
        .bind(at)
        .execute(pool)
        .await
        .unwrap();
    id
}

fn event(series: &str, title: &str, starts_at: &str) -> UpcomingMeeting {
    UpcomingMeeting {
        id: format!("evt-{series}"),
        title: title.into(),
        starts_at: starts_at.into(),
        ends_at: starts_at.into(),
        calendar_name: "Work".into(),
        location: None,
        zoom_url: None,
        external_id: Some(series.into()),
    }
}

/// Three recurring meetings on the horizon, each with one prior occurrence.
async fn three_recurring(pool: &SqlitePool) -> Vec<UpcomingMeeting> {
    let mut events = Vec::new();
    for (i, title) in ["Standup", "Design review", "Planning"]
        .into_iter()
        .enumerate()
    {
        let series = format!("series-{i}-{}", uuid::Uuid::new_v4());
        recorded(pool, title, "2026-09-01T10:00:00Z", Some(&series)).await;
        events.push(event(&series, title, "2026-10-01T10:00:00Z"));
    }
    events
}

fn prep_rows_for(view: &LlmActivityView, meeting: &str) -> usize {
    let m = Some(meeting);
    view.queued
        .iter()
        .filter(|t| t.meeting_id.as_deref() == m)
        .count()
        + view
            .running
            .iter()
            .filter(|t| t.meeting_id.as_deref() == m)
            .count()
        + view
            .history
            .iter()
            .filter(|t| t.meeting_id.as_deref() == m)
            .count()
}

#[tokio::test]
async fn pass_enqueues_all_planned_before_first_starts() {
    let f = fixture().await;
    let events = three_recurring(&f.pool).await;

    let (planned, failed) = plan_pass(f.app.handle(), &f.pool, &events).await;
    assert_eq!(failed, 0);
    let view = f.registry.view();
    assert_eq!(
        view.queued.len(),
        3,
        "every planned brief waits in the queue"
    );
    assert!(view.running.is_empty(), "nothing has started yet");
    assert!(view.queued[0].label.starts_with("Preparing brief — "));

    let failed = execute_pass(f.app.handle(), &f.pool, planned).await;
    let view = f.registry.view();
    assert_eq!(failed, 3, "no model is configured, so each one fails");
    assert!(view.queued.is_empty() && view.running.is_empty());
    assert_eq!(
        view.history.len(),
        3,
        "each queued row became exactly one record"
    );
}

#[tokio::test]
async fn up_to_date_pass_registers_no_history() {
    let f = fixture().await;
    let events = three_recurring(&f.pool).await;
    // Mark every brief ready against its current input.
    for ev in &events {
        let start = parse_start(&ev.starts_at).unwrap();
        let target = MeetingsRepository::upsert_scheduled_meeting(
            &f.pool,
            &ev.id,
            ev.external_id.as_deref(),
            &ev.title,
            start,
        )
        .await
        .unwrap()
        .into_id();
        let BriefPlan::Generate { fingerprint, .. } =
            plan_brief(&f.pool, &target, false).await.unwrap()
        else {
            panic!("a never-generated brief must plan to generate");
        };
        MeetingBriefsRepository::upsert_ready(
            &f.pool,
            &target,
            "# brief",
            "[]",
            &fingerprint,
            "ollama",
            "m",
        )
        .await
        .unwrap();
    }

    let (planned, _) = plan_pass(f.app.handle(), &f.pool, &events).await;
    assert!(f.registry.view().queued.is_empty(), "nothing to wait for");
    execute_pass(f.app.handle(), &f.pool, planned).await;
    let view = f.registry.view();
    assert!(view.running.is_empty());
    assert!(
        view.history.is_empty(),
        "an up-to-date pass must not flood the history"
    );
}

/// The manual path hands its waiting row to `generate_brief_for_target`; an up-to-date brief
/// must drop it without a trace, not record a "Done".
#[tokio::test]
async fn an_up_to_date_manual_trigger_leaves_no_record() {
    let f = fixture().await;
    let events = three_recurring(&f.pool).await;
    let (planned, _) = plan_pass(f.app.handle(), &f.pool, &events[..1]).await;
    let (target, handle) = planned.into_iter().next().unwrap();
    drop(handle);
    let BriefPlan::Generate { fingerprint, .. } =
        plan_brief(&f.pool, &target, false).await.unwrap()
    else {
        panic!("expected Generate");
    };
    MeetingBriefsRepository::upsert_ready(
        &f.pool,
        &target,
        "# b",
        "[]",
        &fingerprint,
        "ollama",
        "m",
    )
    .await
    .unwrap();

    let Slot::Queued(queued) = enqueue_brief(f.app.handle(), &f.pool, &target).await else {
        panic!("nothing else holds this brief");
    };
    let cancel = CancellationToken::new();
    let changed = generate_brief_for_target(
        f.app.handle(),
        &f.pool,
        &target,
        false,
        &cancel,
        |_| {},
        queued,
    )
    .await
    .unwrap();
    assert!(!changed);
    let view = f.registry.view();
    assert!(view.queued.is_empty() && view.running.is_empty() && view.history.is_empty());
}

#[tokio::test]
async fn pass_announces_briefs_it_changed() {
    let f = fixture().await;
    let events = three_recurring(&f.pool).await;
    let heard = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let h = Arc::clone(&heard);
    f.app.handle().listen_any(PREP_BRIEFS_UPDATED, move |_| {
        h.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });

    let (planned, _) = plan_pass(f.app.handle(), &f.pool, &events[..1]).await;
    execute_pass(f.app.handle(), &f.pool, planned).await;
    assert!(
        heard.load(std::sync::atomic::Ordering::SeqCst) >= 1,
        "Today must hear about it"
    );
}

/// The pass's queued row keeps a manual trigger away (one brief, one generation); the
/// manual trigger returns Busy and leaves the work to the pass.
#[tokio::test]
async fn a_brief_the_pass_queued_is_busy_for_a_manual_trigger() {
    let f = fixture().await;
    let events = three_recurring(&f.pool).await;
    let (planned, _) = plan_pass(f.app.handle(), &f.pool, &events[..1]).await;
    let target = planned[0].0.clone();
    assert!(matches!(
        enqueue_brief(f.app.handle(), &f.pool, &target).await,
        Slot::Busy
    ));
    drop(planned);
    assert!(f.registry.view().queued.is_empty());
}

/// Retry used to rerun the whole pass, which only finds calendar-keyed series and silently
/// no-ops while another pass holds its lock. It must now queue THIS brief — here one whose
/// only prior occurrence is linked by hand — and return while a pass is running.
#[tokio::test]
async fn retry_prep_regenerates_a_manually_linked_brief() {
    let f = fixture().await;
    let prior = recorded(
        &f.pool,
        "Kickoff with the vendor",
        "2026-09-01T10:00:00Z",
        None,
    )
    .await;
    let start = parse_start("2026-10-01T10:00:00Z").unwrap();
    let target = MeetingsRepository::upsert_scheduled_meeting(
        &f.pool,
        "evt-manual",
        None,
        "Vendor check-in",
        start,
    )
    .await
    .unwrap()
    .into_id();
    MeetingsRepository::link_meeting_to_series(&f.pool, &prior, &target)
        .await
        .unwrap();

    Arc::clone(&f.registry)
        .start_for(
            TaskKind::PrepBrief,
            Origin::Background,
            "Preparing brief",
            Some(target.clone()),
        )
        .finish(Err("provider timed out".into()));
    let failed_id = f.registry.view().history[0].id;

    let _pass = PASS_LOCK.lock().await; // a background pass is in flight
    let retried = tokio::time::timeout(
        Duration::from_secs(5),
        crate::llm_activity::retry::retry_task(f.app.handle(), failed_id),
    )
    .await
    .expect("Retry must return without waiting for the pass");
    assert!(retried.is_ok(), "{retried:?}");

    let view = f.registry.view();
    assert!(
        view.history.iter().all(|r| r.id != failed_id),
        "the failed row is gone"
    );
    assert_eq!(
        prep_rows_for(&view, &target),
        1,
        "one fresh row for this brief: {view:?}"
    );
}
