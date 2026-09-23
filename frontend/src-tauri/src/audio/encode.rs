use super::ffmpeg::find_ffmpeg_path; // Correct path to encode module
use super::AudioDevice;
use std::io::Write;
use std::sync::Arc;
use std::{
    path::Path,
    process::{Command, Stdio},
};
use tracing::{debug, error};

pub struct AudioInput {
    pub data: Arc<Vec<f32>>,
    pub sample_rate: u32,
    pub channels: u16,
    pub device: Arc<AudioDevice>,
}

/// AAC-LC bitrate of the meeting mix (`audio.mp4` and its checkpoints), specs/0072 W0: 64 kbps
/// scored the same WER as 192 kbps at a third of the size.
pub const MIX_AAC_BITRATE: u32 = 64_000;

pub fn encode_single_audio(
    data: &[u8],
    sample_rate: u32,
    channels: u16,
    bitrate: u32,
    output_path: &Path,
) -> anyhow::Result<()> {
    debug!(
        "Starting FFmpeg process for {} bytes of audio data",
        data.len()
    );

    if data.is_empty() {
        return Err(anyhow::anyhow!("No audio data provided for encoding"));
    }

    let ffmpeg_path = find_ffmpeg_path().ok_or_else(|| {
        anyhow::anyhow!("FFmpeg not found. Please install FFmpeg to save recordings.")
    })?;

    debug!("Using FFmpeg at: {:?}", ffmpeg_path);

    let mut command = Command::new(ffmpeg_path);
    command
        .args([
            "-f",
            "f32le",
            "-ar",
            &sample_rate.to_string(),
            "-ac",
            &channels.to_string(),
            "-i",
            "pipe:0",
            "-c:a",
            "aac",
            "-b:a",
            &bitrate.to_string(),
            "-profile:a",
            "aac_low", // Use AAC-LC profile for better compatibility
            "-movflags",
            "+faststart", // Optimize for web streaming
            "-f",
            "mp4",
            output_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Hide console window on Windows to prevent CMD popup during recording
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    debug!("FFmpeg command: {:?}", command);

    #[allow(clippy::zombie_processes)]
    let mut ffmpeg = command.spawn().expect("Failed to spawn FFmpeg process");
    debug!("FFmpeg process spawned");
    let mut stdin = ffmpeg.stdin.take().expect("Failed to open stdin");

    stdin.write_all(data)?;

    debug!("Dropping stdin");
    drop(stdin);
    debug!("Waiting for FFmpeg process to exit");
    let output = ffmpeg.wait_with_output().unwrap();
    let status = output.status;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    debug!("FFmpeg process exited with status: {}", status);
    debug!("FFmpeg stdout: {}", stdout);
    debug!("FFmpeg stderr: {}", stderr);

    if !status.success() {
        error!("FFmpeg process failed with status: {}", status);
        error!("FFmpeg stderr: {}", stderr);
        return Err(anyhow::anyhow!(
            "FFmpeg process failed with status: {}",
            status
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Acceptance 9 (specs/0072): the mix is written at `MIX_AAC_BITRATE`, not the old 192k.
    /// Noise makes the encoder spend its whole budget, so the file size measures the bitrate.
    #[test]
    fn the_mix_is_encoded_at_the_mix_bitrate() {
        if which::which("ffmpeg").is_err() {
            eprintln!("SKIP the_mix_is_encoded_at_the_mix_bitrate: no ffmpeg on PATH");
            return;
        }
        let secs = 6.0_f64;
        let mut seed = 0x2545_f491_u32;
        let noise: Vec<f32> = (0..(48_000.0 * secs) as usize)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 * 0.6 - 0.3
            })
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("audio.mp4");
        encode_single_audio(
            bytemuck::cast_slice(&noise),
            48_000,
            1,
            MIX_AAC_BITRATE,
            &out,
        )
        .unwrap();
        let kbps = std::fs::metadata(&out).unwrap().len() as f64 * 8.0 / secs / 1000.0;
        assert!(
            (48.0..96.0).contains(&kbps),
            "mix written at ~{kbps:.0} kbps"
        );
    }
}
