//! One-off maintenance: re-queues failed meeting STT jobs so the running app
//! (with the model-warmup fix) can transcribe the real recordings that failed
//! before the fix. Equivalent to pressing the retry button per meeting.

use speak_app_lib::managers::meeting_store::open_meeting_database;

fn main() {
    let db = std::env::var("SPEAK_MEETINGS_DB").unwrap_or_else(|_| {
        std::env::var("APPDATA")
            .map(|p| format!("{p}\\com.haeni.speak\\meetings.db"))
            .unwrap()
    });
    let conn = open_meeting_database(std::path::Path::new(&db)).expect("open meetings db");

    let recoverable: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, title FROM meeting_sessions
                 WHERE state = 'recoverable_error' AND deleted_at IS NULL",
            )
            .expect("query");
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("map");
        rows.collect::<Result<Vec<_>, _>>().expect("rows")
    };
    println!("recoverable meetings: {}", recoverable.len());

    let mut requeued = 0usize;
    for (id, title) in &recoverable {
        let failed: usize = conn
            .prepare(
                "SELECT COUNT(*) FROM meeting_stt_jobs
                 WHERE meeting_id = ?1 AND state = 'failed'",
            )
            .expect("count")
            .query_row([id], |r| r.get(0))
            .expect("count row");
        if failed == 0 {
            continue;
        }
        conn.execute(
            "UPDATE meeting_stt_jobs SET state = 'queued', attempt_count = 0,
             next_attempt_at = NULL, last_error = NULL
             WHERE meeting_id = ?1 AND state = 'failed'",
            [id],
        )
        .expect("requeue");
        // Session back to transcribing so the queue settlement flow resumes.
        conn.execute(
            "UPDATE meeting_sessions SET state = 'transcribing', last_error = NULL
             WHERE id = ?1",
            [id],
        )
        .expect("session state");
        requeued += failed;
        println!("requeued {failed} job(s) for \"{title}\" ({id})");
    }
    println!("total requeued: {requeued}");
}
