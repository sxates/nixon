use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait};
use futures_util::{Stream, StreamExt};
use std::pin::Pin;
use std::task::{Context, Poll};

#[cfg(target_os = "macos")]
use super::core_audio::CoreAudioCapture;
#[cfg(target_os = "macos")]
use futures_channel::mpsc;
#[cfg(target_os = "macos")]
use log::info;

/// System audio capture using Core Audio tap (macOS) or CPAL (other platforms)
pub struct SystemAudioCapture {
    _host: cpal::Host,
}

impl SystemAudioCapture {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        Ok(Self { _host: host })
    }

    pub fn list_system_devices() -> Result<Vec<String>> {
        let host = cpal::default_host();
        let devices = host
            .output_devices()
            .map_err(|e| anyhow::anyhow!("Failed to enumerate output devices: {}", e))?;

        let mut device_names = Vec::new();
        for device in devices {
            if let Ok(name) = device.name() {
                device_names.push(name);
            }
        }

        Ok(device_names)
    }

    pub fn start_system_audio_capture(&self) -> Result<SystemAudioStream> {
        #[cfg(target_os = "macos")]
        {
            info!("Starting Core Audio system capture (macOS)");
            // Use Core Audio tap for system audio capture
            let core_audio = CoreAudioCapture::new()?;
            let core_audio_stream = core_audio.stream()?;
            let sample_rate = core_audio_stream.sample_rate();

            // Convert CoreAudioStream to SystemAudioStream
            let (tx, rx) = mpsc::unbounded::<Vec<f32>>();
            // specs/0028: stop the forwarding task promptly on drop. The previous
            // std::sync::mpsc stop flag was only polled at the top of each loop iteration, so a
            // silent/stalled Core Audio stream would park in `stream.next().await` forever and
            // never observe the stop signal — leaking the task AND keeping the Core Audio tap
            // alive. Use an async oneshot stop signal selected against the stream, and also
            // store the JoinHandle so Drop can hard-abort as a guarantee.
            let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();

            // Spawn task to forward Core Audio samples
            let forward_handle = tokio::spawn(async move {
                let mut stream = core_audio_stream;
                let mut buffer = Vec::new();
                let chunk_size = 1024;

                loop {
                    tokio::select! {
                        // Prefer the stop signal so we tear down immediately on drop.
                        biased;
                        _ = &mut stop_rx => {
                            break;
                        }
                        next = stream.next() => {
                            match next {
                                Some(sample) => {
                                    buffer.push(sample);
                                    if buffer.len() >= chunk_size {
                                        if tx.unbounded_send(buffer.clone()).is_err() {
                                            break;
                                        }
                                        buffer.clear();
                                    }
                                }
                                None => break,
                            }
                        }
                    }
                }

                // Send any remaining samples
                if !buffer.is_empty() {
                    let _ = tx.unbounded_send(buffer);
                }

                // Explicitly drop the Core Audio stream so its Drop signals tap termination
                // the moment forwarding ends (deterministic tap teardown, no leak).
                drop(stream);
            });

            let receiver = rx.map(futures_util::stream::iter).flatten();

            info!("Core Audio system capture started successfully");

            Ok(SystemAudioStream {
                stop_tx: Some(stop_tx),
                forward_handle,
                sample_rate,
                receiver: Box::pin(receiver),
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            // For non-macOS platforms, you would implement WASAPI/ALSA loopback here
            anyhow::bail!("System audio capture not yet implemented for this platform")
        }
    }

    pub fn check_system_audio_permissions() -> bool {
        // Check if we can enumerate audio devices
        cpal::default_host().output_devices().is_ok()
    }
}

pub struct SystemAudioStream {
    // specs/0028: async stop signal + owned task handle so the forwarding task (and the
    // Core Audio tap it holds) are torn down promptly and deterministically on drop.
    #[cfg(target_os = "macos")]
    stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
    #[cfg(target_os = "macos")]
    forward_handle: tokio::task::JoinHandle<()>,
    sample_rate: u32,
    receiver: Pin<Box<dyn Stream<Item = f32> + Send + Sync>>,
}

impl Drop for SystemAudioStream {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            // Ask the task to stop (flushes the tail), then hard-abort so it also unblocks if
            // it is parked awaiting a silent/stalled stream — guaranteeing the tap is dropped.
            if let Some(tx) = self.stop_tx.take() {
                let _ = tx.send(());
            }
            self.forward_handle.abort();
        }
    }
}

impl Stream for SystemAudioStream {
    type Item = f32;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.as_mut().poll_next_unpin(cx)
    }
}

impl SystemAudioStream {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// Public interface for system audio capture
pub async fn start_system_audio_capture() -> Result<SystemAudioStream> {
    let capture = SystemAudioCapture::new()?;
    capture.start_system_audio_capture()
}

pub fn list_system_audio_devices() -> Result<Vec<String>> {
    SystemAudioCapture::list_system_devices()
}

pub fn check_system_audio_permissions() -> bool {
    SystemAudioCapture::check_system_audio_permissions()
}
