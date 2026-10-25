use std::io::{self, Write};

use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavSpec {
    pub sample_rate: u32,
    pub channels: u16,
}

pub fn write_wav<W: Write>(mut writer: W, spec: WavSpec, samples: &[f32]) -> Result<(), WavError> {
    if spec.sample_rate == 0 {
        return Err(WavError::ZeroSampleRate);
    }
    if spec.channels == 0 {
        return Err(WavError::ZeroChannels);
    }
    let channels = usize::from(spec.channels);
    if samples.len() % channels != 0 {
        return Err(WavError::SampleCount {
            samples: samples.len(),
            channels,
        });
    }

    let block_align = spec.channels.checked_mul(2).ok_or(WavError::TooLarge)?;
    let byte_rate = spec
        .sample_rate
        .checked_mul(u32::from(block_align))
        .ok_or(WavError::TooLarge)?;
    let data_bytes = samples
        .len()
        .checked_mul(2)
        .and_then(|bytes| u32::try_from(bytes).ok())
        .ok_or(WavError::TooLarge)?;
    let riff_size = 36u32.checked_add(data_bytes).ok_or(WavError::TooLarge)?;

    writer.write_all(b"RIFF")?;
    write_u32(&mut writer, riff_size)?;
    writer.write_all(b"WAVE")?;
    writer.write_all(b"fmt ")?;
    write_u32(&mut writer, 16)?;
    write_u16(&mut writer, 1)?;
    write_u16(&mut writer, spec.channels)?;
    write_u32(&mut writer, spec.sample_rate)?;
    write_u32(&mut writer, byte_rate)?;
    write_u16(&mut writer, block_align)?;
    write_u16(&mut writer, 16)?;
    writer.write_all(b"data")?;
    write_u32(&mut writer, data_bytes)?;

    for sample in samples {
        let value = if sample.is_nan() {
            0.0
        } else {
            sample.clamp(-1.0, 1.0)
        };
        let pcm = (value * f32::from(i16::MAX)).round() as i16;
        writer.write_all(&pcm.to_le_bytes())?;
    }

    Ok(())
}

fn write_u16<W: Write>(writer: &mut W, value: u16) -> Result<(), io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u32<W: Write>(writer: &mut W, value: u32) -> Result<(), io::Error> {
    writer.write_all(&value.to_le_bytes())
}

#[derive(Debug, Error)]
pub enum WavError {
    #[error("failed to write WAV data: {0}")]
    Io(#[from] io::Error),
    #[error("sample rate must be greater than zero")]
    ZeroSampleRate,
    #[error("channel count must be greater than zero")]
    ZeroChannels,
    #[error("{samples} samples do not divide evenly into {channels} channels")]
    SampleCount { samples: usize, channels: usize },
    #[error("the WAV file would exceed the 32-bit RIFF size limit")]
    TooLarge,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
    }

    fn u32_at(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    }

    #[test]
    fn writes_a_valid_pcm_header() {
        let mut bytes = Vec::new();
        write_wav(
            &mut bytes,
            WavSpec {
                sample_rate: 48_000,
                channels: 2,
            },
            &[0.0, 0.0],
        )
        .unwrap();

        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32_at(&bytes, 4), 36 + 4);
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(u32_at(&bytes, 16), 16);
        assert_eq!(u16_at(&bytes, 20), 1);
        assert_eq!(u16_at(&bytes, 22), 2);
        assert_eq!(u32_at(&bytes, 24), 48_000);
        assert_eq!(u32_at(&bytes, 28), 192_000);
        assert_eq!(u16_at(&bytes, 32), 4);
        assert_eq!(u16_at(&bytes, 34), 16);
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(u32_at(&bytes, 40), 4);
    }

    #[test]
    fn converts_samples_to_signed_pcm() {
        let mut bytes = Vec::new();
        write_wav(
            &mut bytes,
            WavSpec {
                sample_rate: 48_000,
                channels: 1,
            },
            &[0.0, 0.5, -0.5, 1.0, -1.0, f32::NAN],
        )
        .unwrap();

        let samples: Vec<i16> = bytes[44..]
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        assert_eq!(samples, vec![0, 16_384, -16_384, 32_767, -32_767, 0]);
    }

    #[test]
    fn rejects_unaligned_and_empty_layouts() {
        let mut bytes = Vec::new();
        assert!(matches!(
            write_wav(
                &mut bytes,
                WavSpec {
                    sample_rate: 48_000,
                    channels: 2,
                },
                &[0.0],
            ),
            Err(WavError::SampleCount { .. })
        ));
        assert!(matches!(
            write_wav(
                &mut bytes,
                WavSpec {
                    sample_rate: 48_000,
                    channels: 0,
                },
                &[],
            ),
            Err(WavError::ZeroChannels)
        ));
        assert!(matches!(
            write_wav(
                &mut bytes,
                WavSpec {
                    sample_rate: 0,
                    channels: 1,
                },
                &[],
            ),
            Err(WavError::ZeroSampleRate)
        ));
    }
}
