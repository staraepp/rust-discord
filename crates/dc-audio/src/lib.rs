//! Device IO and Opus. Device enumeration/capture is never done by unit tests.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

pub const SAMPLE_RATE: u32 = 48_000;
pub const FRAME_SAMPLES: usize = 960;
pub const CHANNELS: usize = 2;
#[derive(Debug, thiserror::Error)]
#[error("Audio operation failed: {0}")]
pub struct AudioError(pub String);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    pub input: bool,
}
pub trait AudioDevices {
    fn devices(&self) -> Result<Vec<DeviceInfo>, AudioError>;
}
pub struct NativeDevices;
impl AudioDevices for NativeDevices {
    fn devices(&self) -> Result<Vec<DeviceInfo>, AudioError> {
        let host = cpal::default_host();
        let mut devices = Vec::new();
        for (input, list) in [(true, host.input_devices()), (false, host.output_devices())] {
            for device in list.map_err(|e| AudioError(e.to_string()))? {
                devices.push(DeviceInfo {
                    name: device
                        .description()
                        .map_err(|e| AudioError(e.to_string()))?
                        .name()
                        .to_owned(),
                    input,
                });
            }
        }
        Ok(devices)
    }
}
pub struct OpusEncoder(audiopus::coder::Encoder);
impl OpusEncoder {
    pub fn new() -> Result<Self, AudioError> {
        let mut encoder = audiopus::coder::Encoder::new(
            audiopus::SampleRate::Hz48000,
            audiopus::Channels::Stereo,
            audiopus::Application::Voip,
        )
        .map_err(|e| AudioError(e.to_string()))?;
        encoder
            .set_bitrate(audiopus::Bitrate::BitsPerSecond(64_000))
            .map_err(|e| AudioError(e.to_string()))?;
        encoder
            .set_inband_fec(true)
            .map_err(|e| AudioError(e.to_string()))?;
        Ok(Self(encoder))
    }
    pub fn encode(&self, pcm: &[f32]) -> Result<Vec<u8>, AudioError> {
        if pcm.len() != FRAME_SAMPLES * CHANNELS {
            return Err(AudioError("Expected one stereo 20 ms frame".into()));
        }
        let mut output = vec![0; 4000];
        let count = self
            .0
            .encode_float(pcm, &mut output)
            .map_err(|e| AudioError(e.to_string()))?;
        output.truncate(count);
        Ok(output)
    }
}
pub struct OpusDecoder(audiopus::coder::Decoder);
impl OpusDecoder {
    pub fn new() -> Result<Self, AudioError> {
        Ok(Self(
            audiopus::coder::Decoder::new(
                audiopus::SampleRate::Hz48000,
                audiopus::Channels::Stereo,
            )
            .map_err(|e| AudioError(e.to_string()))?,
        ))
    }
    pub fn decode(&mut self, packet: Option<&[u8]>) -> Result<Vec<f32>, AudioError> {
        let mut output = vec![0.; FRAME_SAMPLES * CHANNELS * 6];
        let packet = packet
            .map(audiopus::packet::Packet::try_from)
            .transpose()
            .map_err(|e| AudioError(e.to_string()))?;
        let samples = self
            .0
            .decode_float(
                packet,
                audiopus::MutSignals::try_from(output.as_mut_slice())
                    .map_err(|e| AudioError(e.to_string()))?,
                false,
            )
            .map_err(|e| AudioError(e.to_string()))?;
        output.truncate(samples * CHANNELS);
        Ok(output)
    }
}

/// Streams are owned by the voice worker and close on drop. Callbacks use
/// bounded queues and drop frames rather than blocking a real-time thread.
pub struct DuplexAudio {
    _input: cpal::Stream,
    _output: cpal::Stream,
    pub captured: mpsc::Receiver<Vec<f32>>,
    playback: Arc<Mutex<VecDeque<f32>>>,
    pub muted: Arc<AtomicBool>,
    pub deafened: Arc<AtomicBool>,
}
impl DuplexAudio {
    pub fn open(input_name: Option<&str>, output_name: Option<&str>) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let input = match input_name {
            Some(name) => host
                .input_devices()
                .map_err(|e| AudioError(e.to_string()))?
                .find(|d| d.description().is_ok_and(|s| s.name() == name)),
            None => host.default_input_device(),
        }
        .ok_or_else(|| AudioError("No microphone selected or available".into()))?;
        let output = match output_name {
            Some(name) => host
                .output_devices()
                .map_err(|e| AudioError(e.to_string()))?
                .find(|d| d.description().is_ok_and(|s| s.name() == name)),
            None => host.default_output_device(),
        }
        .ok_or_else(|| AudioError("No output device selected or available".into()))?;
        let input_config = choose_config(
            input
                .supported_input_configs()
                .map_err(|e| AudioError(e.to_string()))?,
        )?;
        let output_config = choose_config(
            output
                .supported_output_configs()
                .map_err(|e| AudioError(e.to_string()))?,
        )?;
        let (tx, captured) = mpsc::sync_channel(8);
        let muted = Arc::new(AtomicBool::new(false));
        let deafened = Arc::new(AtomicBool::new(false));
        let playback = Arc::new(Mutex::new(VecDeque::with_capacity(SAMPLE_RATE as usize)));
        let input = match input_config.sample_format() {
            SampleFormat::F32 => {
                input_stream::<f32>(&input, input_config.into(), tx, muted.clone())
            }
            SampleFormat::I16 => {
                input_stream::<i16>(&input, input_config.into(), tx, muted.clone())
            }
            SampleFormat::U16 => {
                input_stream::<u16>(&input, input_config.into(), tx, muted.clone())
            }
            _ => return Err(AudioError("Unsupported microphone sample format".into())),
        }?;
        let output = match output_config.sample_format() {
            SampleFormat::F32 => output_stream::<f32>(
                &output,
                output_config.into(),
                playback.clone(),
                deafened.clone(),
            ),
            SampleFormat::I16 => output_stream::<i16>(
                &output,
                output_config.into(),
                playback.clone(),
                deafened.clone(),
            ),
            SampleFormat::U16 => output_stream::<u16>(
                &output,
                output_config.into(),
                playback.clone(),
                deafened.clone(),
            ),
            _ => return Err(AudioError("Unsupported output sample format".into())),
        }?;
        input.play().map_err(|e| AudioError(e.to_string()))?;
        output.play().map_err(|e| AudioError(e.to_string()))?;
        Ok(Self {
            _input: input,
            _output: output,
            captured,
            playback,
            muted,
            deafened,
        })
    }
    pub fn play(&self, frame: &[f32]) {
        if self.deafened.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut buffer) = self.playback.lock() {
            if buffer.len() + frame.len() > SAMPLE_RATE as usize {
                buffer.clear();
            }
            buffer.extend(frame.iter().map(|s| s.clamp(-1., 1.)));
        }
    }
}
fn choose_config(
    configs: impl Iterator<Item = cpal::SupportedStreamConfigRange>,
) -> Result<cpal::SupportedStreamConfig, AudioError> {
    configs
        .filter(|c| {
            c.channels() <= 2
                && c.min_sample_rate() <= SAMPLE_RATE
                && c.max_sample_rate() >= SAMPLE_RATE
                && matches!(
                    c.sample_format(),
                    SampleFormat::F32 | SampleFormat::I16 | SampleFormat::U16
                )
        })
        .min_by_key(|c| (c.sample_format() != SampleFormat::F32, c.channels() != 2))
        .map(|c| c.with_sample_rate(SAMPLE_RATE))
        .ok_or_else(|| AudioError("Device does not expose a supported 48 kHz format".into()))
}
fn input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    tx: mpsc::SyncSender<Vec<f32>>,
    muted: Arc<AtomicBool>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let mut frame = Vec::with_capacity(FRAME_SAMPLES * CHANNELS);
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                for input in data.chunks_exact(channels) {
                    let left = f32::from_sample(input[0]);
                    let right = f32::from_sample(input[channels - 1]);
                    frame.push(left);
                    frame.push(right);
                    if frame.len() == FRAME_SAMPLES * CHANNELS {
                        if !muted.load(Ordering::Relaxed) {
                            let _ = tx.try_send(frame.clone());
                        }
                        frame.clear();
                    }
                }
            },
            |_| {},
            None,
        )
        .map_err(|e| AudioError(e.to_string()))
}
fn output_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    buffer: Arc<Mutex<VecDeque<f32>>>,
    deafened: Arc<AtomicBool>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let mut buffer = buffer.try_lock().ok();
                let deafened = deafened.load(Ordering::Relaxed);
                for out in data.chunks_mut(channels) {
                    let (left, right) = if deafened {
                        (0., 0.)
                    } else if let Some(buffer) = &mut buffer {
                        (
                            buffer.pop_front().unwrap_or(0.),
                            buffer.pop_front().unwrap_or(0.),
                        )
                    } else {
                        (0., 0.)
                    };
                    out[0] = T::from_sample(if channels == 1 {
                        (left + right) * 0.5
                    } else {
                        left
                    });
                    if channels == 2 {
                        out[1] = T::from_sample(right);
                    }
                }
            },
            |_| {},
            None,
        )
        .map_err(|e| AudioError(e.to_string()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opus_roundtrip_and_packet_loss_use_only_memory() {
        let encoder = OpusEncoder::new().unwrap();
        let frame: Vec<f32> = (0..FRAME_SAMPLES * CHANNELS)
            .map(|i| ((i / 2) as f32 * 0.04).sin() * 0.1)
            .collect();
        let packet = encoder.encode(&frame).unwrap();
        let mut decoder = OpusDecoder::new().unwrap();
        let decoded = decoder.decode(Some(&packet)).unwrap();
        assert_eq!(decoded.len(), FRAME_SAMPLES * CHANNELS);
        assert!(decoded.iter().all(|s| s.is_finite()));
        assert!(!decoder.decode(None).unwrap().is_empty());
        assert!(encoder.encode(&[0.]).is_err());
    }
}
