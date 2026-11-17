//! The `--demo` Topics and To-Dos scenario (dev_fixtures::scenario) seeds cleanly on top of
//! the embedded dataset, twice in a row, and every view has something to show.

mod common;

use app_lib::database::repositories::action_item_todo::{ActionItemTodoRepository, TodoView};
use app_lib::dev_fixtures::{dataset, scenario, seed};
use app_lib::topics::{ledger, links, repo::TopicsRepository};
use std::collections::HashMap;

async fn seed_once(pool: &sqlx::SqlitePool, ds: &dataset::Dataset, now: chrono::DateTime<chrono::Utc>) {
    seed::seed_all(pool, ds, &HashMap::new(), now).await.unwrap();
    seed::seed_manual_meetings(pool, &ds.manual_meetings, now).await.unwrap();
    scenario::seed_topics_and_todos(pool, ds, now).await.unwrap();
}

#[tokio::test]
async fn demo_scenario_fills_topics_and_todos_and_reseeds() {
    let (_dir, db) = common::fresh_db().await;
    let pool = db.pool();
    let ds = dataset::load_embedded().unwrap();
    let now = chrono::Utc::now();
    seed_once(pool, &ds, now).await;
    seed_once(pool, &ds, now).await; // a second --demo run starts clean

    let topics = TopicsRepository::list(pool, None).await.unwrap();
    let names: Vec<&str> = topics.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names.len(), 5, "{names:?}");
    assert_eq!(topics.iter().filter(|t| t.starred && t.status == "open").count(), 3);
    assert_eq!(topics.iter().filter(|t| t.status == "closed").count(), 1);

    let release = ledger::entries_for_topic(pool, scenario::TOPIC_RELEASE).await.unwrap();
    assert_eq!(release.len(), 4, "one entry per Product sync");
    assert!(release.iter().any(|e| !e.revisions.is_empty()), "the cloud-retry revision");
    let release_links = links::links_for_topic(pool, scenario::TOPIC_RELEASE).await.unwrap();
    assert_eq!(release_links.len(), 1);
    assert_eq!(release_links[0].kind, links::LinkKind::Series);
    let pending = ledger::pending_suggestions(pool, scenario::TOPIC_Q1).await.unwrap();
    assert_eq!(pending.len(), 2);

    let list = ActionItemTodoRepository::list_view(pool, TodoView::List).await.unwrap();
    let ids: Vec<&str> = list.iter().map(|r| r.item.id.as_str()).collect();
    assert_eq!(ids.first(), Some(&"ai-demo-01-0"), "{ids:?}");
    assert!(!ids.contains(&"ai-manual-comp"), "Maya's Topic is kept off the list");
    assert!(list.iter().any(|r| r.topic_id.is_none()), "some To-Dos have no Topic");
    let ble = &list[0];
    assert_eq!(ble.source_count, 2, "the BLE follow-up merged two Meetings");
    let inbox = ActionItemTodoRepository::list_view(pool, TodoView::Inbox).await.unwrap();
    assert!(inbox.len() >= 2, "{}", inbox.len());
    let done = ActionItemTodoRepository::list_view(pool, TodoView::Done).await.unwrap();
    assert!(done.len() >= 3, "{}", done.len());
}
