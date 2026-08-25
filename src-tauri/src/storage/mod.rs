//! Persistence seam (plan §5): durable, content-addressed asset storage.
//!
//! Canonical home of `AssetStore` per `module-placement-map.md` row B11 and
//! rule E3: declared privately in `lib.rs`, selectively re-exported here.
//! No SQL may live under this module (`repository-boundary-contract.md` R1).

pub(crate) mod asset_store;

// Selective re-export (E3): consumed once `MeetingStore` / `KnowledgeStore`
// land; until then the seam is intentionally unwired.
#[allow(unused_imports)]
pub(crate) use asset_store::{
    AssetId, AssetKind, AssetStore, AssetStoreError, AudioSource, ContentHash, DeleteReport,
    LocalAssetStore, PutRequest, PutResult, PutSource, ReadVerification, StorageKey, VerifyReport,
};
