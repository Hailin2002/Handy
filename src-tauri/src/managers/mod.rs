// Meeting-mode repository seams (task-0134): compile-only trait/entity
// skeletons per repository-boundary-contract.md §2/§3 — no SQL, no wiring yet.
// Append-only change_log writer shared by both stores (contract §3 row 16).
pub mod change_log;
pub mod knowledge_store;
// Concrete session persistence backing the wave-1 meeting journey.
pub mod meeting;
pub mod meeting_store;
// Local STT queue draining persistent meeting jobs (plan §13 Phase 5).
pub mod meeting_transcription;
// Cloud STT backends for meeting jobs (plan §13 Phase 6).
pub mod meeting_cloud;
pub mod meeting_export;
pub mod meeting_summary;
pub mod project;
pub mod source_ingestion;
pub mod whatsapp_importer;

pub mod audio;
pub mod gguf_meta;
pub mod history;
pub mod model;
pub mod model_capabilities;
pub mod transcription;
