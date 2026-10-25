use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedAudio {
    pub sample_rate: u32,
    pub channels: usize,
    pub frames: u64,
    pub peaks: Vec<f32>,
}

pub fn decode_peaks(path: &Path, buckets: usize) -> Result<DecodedAudio, DecodeError> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }

    let mut format = symphonia::default::get_probe().probe(
        &hint,
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    )?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or(DecodeError::NoTrack)?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or(DecodeError::NoCodec)?;
    let mut sample_rate = params.sample_rate.unwrap_or(0);
    let mut channels = params
        .channels
        .as_ref()
        .map(|channels| channels.count())
        .unwrap_or(0);
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())?;

    let mut frames = 0u64;
    let mut raw_peaks = Vec::new();
    let mut bucket_peak = 0.0f32;
    let mut bucket_frames = 0usize;
    let mut interleaved: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) | Err(Error::ResetRequired) => break,
            Err(error) => return Err(error.into()),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                let spec = audio.spec();
                sample_rate = spec.rate();
                channels = spec.channels().count().max(1);
                interleaved.resize(audio.samples_interleaved(), 0.0);
                audio.copy_to_slice_interleaved(&mut interleaved);
                frames += audio.frames() as u64;
                for frame in interleaved.chunks_exact(channels) {
                    let peak = frame
                        .iter()
                        .fold(0.0f32, |maximum, sample| maximum.max(sample.abs()));
                    bucket_peak = bucket_peak.max(peak);
                    bucket_frames += 1;
                    if bucket_frames >= 2048 {
                        raw_peaks.push(bucket_peak);
                        bucket_peak = 0.0;
                        bucket_frames = 0;
                    }
                }
            }
            Err(Error::DecodeError(_)) | Err(Error::IoError(_)) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    if bucket_frames > 0 {
        raw_peaks.push(bucket_peak);
    }

    Ok(DecodedAudio {
        sample_rate,
        channels,
        frames,
        peaks: downsample(&raw_peaks, buckets.max(1)),
    })
}

fn downsample(peaks: &[f32], buckets: usize) -> Vec<f32> {
    if peaks.is_empty() {
        return vec![0.0];
    }
    if peaks.len() <= buckets {
        return peaks.to_vec();
    }
    (0..buckets)
        .map(|bucket| {
            let start = bucket * peaks.len() / buckets;
            let end = ((bucket + 1) * peaks.len() / buckets).max(start + 1);
            peaks[start..end].iter().copied().fold(0.0f32, f32::max)
        })
        .collect()
}

pub fn hash_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 65_536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(*hasher.finalize().as_bytes())
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Symphonia(#[from] symphonia::core::errors::Error),
    #[error("the file has no audio track")]
    NoTrack,
    #[error("the audio track has no codec parameters")]
    NoCodec,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::{WavSpec, write_wav};
    use tempfile::TempDir;

    #[test]
    fn decodes_a_wav_into_peaks() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("tone.wav");
        let frames = 4_800;
        let mut samples = Vec::with_capacity(frames * 2);
        for frame in 0..frames {
            let value = (frame as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5;
            samples.push(value);
            samples.push(value);
        }
        let file = File::create(&path).unwrap();
        write_wav(
            std::io::BufWriter::new(file),
            WavSpec {
                sample_rate: 48_000,
                channels: 2,
            },
            &samples,
        )
        .unwrap();

        let decoded = decode_peaks(&path, 128).unwrap();
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.frames, frames as u64);
        assert_eq!(decoded.peaks.len(), 3);
        assert!(decoded.peaks.iter().any(|peak| *peak > 0.4));
    }

    #[test]
    fn hashes_files_by_content() {
        let directory = TempDir::new().unwrap();
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        std::fs::write(&first, b"same").unwrap();
        std::fs::write(&second, b"same").unwrap();
        assert_eq!(hash_file(&first).unwrap(), hash_file(&second).unwrap());
        std::fs::write(&second, b"different").unwrap();
        assert_ne!(hash_file(&first).unwrap(), hash_file(&second).unwrap());
    }
}
