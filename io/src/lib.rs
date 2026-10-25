pub mod decode;
pub mod wav;

pub use decode::{DecodeError, DecodedAudio, decode_peaks, hash_file};
pub use wav::{WavError, WavSpec, write_wav};
