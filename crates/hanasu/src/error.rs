/// Errors `hanasu` can produce.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The ONNX model could not be loaded (missing file, bad format, ort error).
    #[error("failed to load ONNX model: {0}")]
    ModelLoad(String),

    /// ONNX inference failed.
    #[error("ONNX inference failed: {0}")]
    Inference(String),

    /// `espeak-ng` could not be run, or returned an error.
    #[error("espeak-ng failed: {0}")]
    Espeak(String),

    /// The requested voice is not present in the voices file.
    #[error("voice {0:?} not found in voices file")]
    VoiceNotFound(String),

    /// The voices file could not be parsed.
    #[error("voices file is malformed: {0}")]
    VoicesParse(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Convenience alias for engine results.
pub type Result<T> = std::result::Result<T, Error>;
