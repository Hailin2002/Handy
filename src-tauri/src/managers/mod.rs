// Meeting-mode repository seams (task-0134): compile-only trait/entity
// skeletons per repository-boundary-contract.md §2/§3 — no SQL, no wiring yet.
pub mod knowledge_store;
pub mod meeting_store;

pub mod audio;
pub mod gguf_meta;
pub mod history;
pub mod model;
pub mod model_capabilities;
pub mod transcription;
