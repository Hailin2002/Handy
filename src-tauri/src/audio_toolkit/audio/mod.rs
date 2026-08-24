// Re-export all audio components
mod device;
pub mod meeting_recorder;
pub mod meeting_tolerances;
mod recorder;
mod resampler;
mod utils;
mod visualizer;

pub use device::{list_input_devices, list_output_devices, CpalDeviceInfo};
pub use meeting_recorder::{
    CaptureBackend, CaptureBlock, CaptureStream, ChunkDescriptor, ChunkFs, ChunkId, ChunkPatch,
    ChunkState, ChunkStateStore, GapClass, MarkerKind, MeetingRecorder, MeetingRecorderImpl,
    PreflightCheck, PreflightOutcome, RecorderError, RecorderEvent, RecorderEventSink,
    RecorderSnapshot, RecorderState, RecoveryReport, SessionLayout, SessionSummary,
    SourceDeviceSet, SourceKind, SourceOpenRequest, SourceState, SourceStatus, StopReason,
    TimelineMarker, TimelineRange,
};
pub use recorder::{
    is_microphone_access_denied, is_no_input_device_error, AudioRecorder, VadPolicy,
};
pub use resampler::FrameResampler;
pub use utils::{read_wav_samples, save_wav_file, verify_wav_file};
pub use visualizer::AudioVisualiser;
