//! Prints meeting session states plus transcript/summary counts.

use speak_app_lib::managers::meeting_store::open_meeting_database;

fn main() {
    let db = std::env::var("SPEAK_MEETINGS_DB").unwrap_or_else(|_| {
        std::env::var("APPDATA")
            .map(|p| format!("{p}\\com.haeni.speak\\meetings.db"))
            .unwrap()
    });
    let conn = open_meeting_database(std::path::Path::new(&db)).expect("open db");

    conn.execute(
        "UPDATE meeting_sessions SET state = 'completed', updated_at = ?1
         WHERE state = 'transcribing' AND deleted_at IS NULL
           AND NOT EXISTS (
             SELECT 1 FROM meeting_stt_jobs j
             WHERE j.meeting_id = meeting_sessions.id
               AND j.state IN ('queued','running','retry_wait','failed'))",
        [chrono::Utc::now().timestamp_millis()],
    )
    .expect("settle completed");
    let mut stmt = conn
        .prepare(
            "SELECT s.title, s.state,
                    (SELECT COUNT(*) FROM meeting_transcript_segments t
                     WHERE t.meeting_id = s.id AND t.deleted_at IS NULL),
                    (SELECT COUNT(*) FROM meeting_stt_jobs j
                     WHERE j.meeting_id = s.id AND j.state = 'failed')
             FROM meeting_sessions s
             WHERE s.deleted_at IS NULL
             ORDER BY s.created_at DESC",
        )
        .expect("query");
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .expect("map");
    println!(
        "{:<32} {:<20} {:>9} {:>7}",
        "Titel", "State", "Segmente", "Failed"
    );
    for row in rows.flatten() {
        println!("{:<32} {:<20} {:>9} {:>7}", row.0, row.1, row.2, row.3);
    }
}
