use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, SampleFormat, SizedSample, StreamConfig, SupportedStreamConfig};
use thiserror::Error;

pub trait AudioCallback: Send + 'static {
    fn process(&mut self, output: &mut [f32], channels: usize, sample_rate: u32);
}

pub struct AudioBackend {
    _stream: cpal::Stream,
    sample_rate: u32,
    channels: u16,
    sample_format: SampleFormat,
    error: Arc<AtomicBool>,
}

impl AudioBackend {
    pub fn start<C: AudioCallback>(
        callback: C,
        sample_rate: u32,
        channels: u16,
    ) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(AudioError::NoDevice)?;
        let supported = choose_config(&device, channels, sample_rate)?;
        let sample_format = supported.sample_format();
        let config = supported.config();
        eprintln!(
            "audio: {:?} {} ch {} Hz",
            sample_format, config.channels, config.sample_rate
        );
        let error = Arc::new(AtomicBool::new(false));
        let xruns = Arc::new(AtomicU32::new(0));

        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(
                &device,
                config,
                callback,
                Arc::clone(&error),
                Arc::clone(&xruns),
            ),
            SampleFormat::I16 => build_stream::<i16>(
                &device,
                config,
                callback,
                Arc::clone(&error),
                Arc::clone(&xruns),
            ),
            SampleFormat::U16 => build_stream::<u16>(
                &device,
                config,
                callback,
                Arc::clone(&error),
                Arc::clone(&xruns),
            ),
            other => return Err(AudioError::UnsupportedSampleFormat(other)),
        }?;
        stream.play()?;

        Ok(Self {
            _stream: stream,
            sample_rate,
            channels,
            sample_format,
            error,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn sample_format(&self) -> SampleFormat {
        self.sample_format
    }

    pub fn is_healthy(&self) -> bool {
        !self.error.load(Ordering::Acquire)
    }
}

fn choose_config(
    device: &Device,
    channels: u16,
    sample_rate: u32,
) -> Result<SupportedStreamConfig, AudioError> {
    let mut fallback = None;
    for range in device.supported_output_configs()? {
        if range.channels() != channels {
            continue;
        }
        if sample_rate < range.min_sample_rate() || sample_rate > range.max_sample_rate() {
            continue;
        }
        let config = range.with_sample_rate(sample_rate);
        if config.sample_format() == SampleFormat::F32 {
            return Ok(config);
        }
        if fallback.is_none() {
            fallback = Some(config);
        }
    }
    fallback.ok_or(AudioError::NoSupportedConfig {
        channels,
        sample_rate,
    })
}

fn build_stream<T>(
    device: &Device,
    config: StreamConfig,
    mut callback: impl AudioCallback,
    error: Arc<AtomicBool>,
    xruns: Arc<AtomicU32>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    let sample_rate = config.sample_rate;
    let mut scratch = vec![0.0f32; 65_536];
    let data_error = Arc::clone(&error);
    let data_xruns = Arc::clone(&xruns);
    let error_xruns = Arc::clone(&xruns);
    let error_callback = move |stream_error: cpal::Error| {
        match stream_error.kind() {
            // One Xrun is a glitch. A long run of them without a successful
            // callback usually means the device was suspended or rerouted;
            // make it visible so the app can rebuild the stream.
            cpal::ErrorKind::Xrun => {
                let count = error_xruns.fetch_add(1, Ordering::AcqRel) + 1;
                if count < 8 {
                    return;
                }
            }
            // A route change is handled by the backend.
            cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::RealtimeDenied => return,
            _ => {}
        }
        error.store(true, Ordering::Release);
    };

    Ok(device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            data_xruns.store(0, Ordering::Release);
            if data.len() > scratch.len() {
                data.fill(T::EQUILIBRIUM);
                data_error.store(true, Ordering::Release);
                return;
            }
            let length = data.len();
            let buffer = &mut scratch[..length];
            callback.process(buffer, channels, sample_rate);
            for (destination, source) in data.iter_mut().zip(buffer.iter().copied()) {
                *destination = T::from_sample(source);
            }
        },
        error_callback,
        None,
    )?)
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("no default audio output device is available")]
    NoDevice,
    #[error("no output configuration with {channels} channels at {sample_rate} Hz")]
    NoSupportedConfig { channels: u16, sample_rate: u32 },
    #[error("unsupported audio sample format {0:?}")]
    UnsupportedSampleFormat(SampleFormat),
    #[error(transparent)]
    Cpal(#[from] cpal::Error),
}
