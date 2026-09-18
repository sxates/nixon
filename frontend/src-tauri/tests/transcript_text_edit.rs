// specs/0061 W5: editable transcript segment text, guarded by a `user_edited`
// flag and kept correct in the FTS5 index (specs/0033's external-content
// trigger trio already scopes its UPDATE trigger to `transcript`, so the
// edit's UPDATE re-triggers it — no hand-rolled index maintenance here).
mod common;

#[tokio::test]
async fn set_segment_text_updates_row_flags_it_and_reindexes_fts() {
    let (_d, db) = common::fresh_db().await;
    let pool = db.pool();
    let id = app_lib::database::repositories::transcript::TranscriptsRepository::save_transcript(
        pool,
        "T",
        &[common::segment("the quick brown fox", 0.0, 2.0)],
        None,
    )
    .await
    .unwrap();
    let (tid,): (String,) = sqlx::query_as("SELECT id FROM transcripts WHERE meeting_id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await
        .unwrap();
    app_lib::transcripts::set_segment_text_inner(pool, &id, &tid, "the quick brown ox")
        .await
        .unwrap();
    let (text, edited): (String, i64) =
        sqlx::query_as("SELECT transcript, user_edited FROM transcripts WHERE id = ?")
            .bind(&tid)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(text, "the quick brown ox");
    assert_eq!(edited, 1);
    let (hits,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM transcripts_fts WHERE transcripts_fts MATCH 'ox'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(hits, 1);
    let (old,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM transcripts_fts WHERE transcripts_fts MATCH 'fox'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(old, 0);
    assert_eq!(
        app_lib::transcripts::count_user_edited_inner(pool, &id)
            .await
            .unwrap(),
        1
    );
    assert!(
        app_lib::transcripts::set_segment_text_inner(pool, &id, &tid, "   ")
            .await
            .is_err()
    );
}
