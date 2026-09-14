//! Start-time device resolution: preference → default → error/None.
//! Serves `start_recording_with_meeting_name` (the other start path,
//! `start_recording_with_devices_and_meeting`, parses explicit device names
//! directly). Extracted as a ratchet offset for the low-power-mode changes;
//! behavior unchanged.

use super::devices::{default_input_device, default_output_device, parse_audio_device, AudioDevice};
use log::{error, info, warn};
use std::sync::Arc;

/// Microphone: preference → default → Err (required).
pub fn resolve_microphone(preferred: Option<String>) -> Result<Arc<AudioDevice>, String> {
    match preferred {
        Some(pref_name) => {
            info!("🎤 Attempting to use preferred microphone: '{}'", pref_name);
            match parse_audio_device(&pref_name) {
                Ok(device) => {
                    info!("✅ Using preferred microphone: '{}'", device.name);
                    Ok(Arc::new(device))
                }
                Err(e) => {
                    warn!(
                        "⚠️ Preferred microphone '{}' not available: {}",
                        pref_name, e
                    );
                    warn!("   Falling back to system default microphone...");
                    match default_input_device() {
                        Ok(device) => {
                            info!("✅ Using default microphone: '{}'", device.name);
                            Ok(Arc::new(device))
                        }
                        Err(default_err) => {
                            error!(
                                "❌ No microphone available (preferred and default both failed)"
                            );
                            Err(format!(
                                "No microphone device available. Preferred device '{}' not found, and default microphone unavailable: {}",
                                pref_name, default_err
                            ))
                        }
                    }
                }
            }
        }
        None => {
            info!("🎤 No microphone preference set, using system default");
            match default_input_device() {
                Ok(device) => {
                    info!("✅ Using default microphone: '{}'", device.name);
                    Ok(Arc::new(device))
                }
                Err(e) => {
                    error!("❌ No default microphone available");
                    Err(format!("No microphone device available: {}", e))
                }
            }
        }
    }
}

/// System audio: preference → default → None (optional).
pub fn resolve_system_audio(preferred: Option<String>) -> Option<Arc<AudioDevice>> {
    match preferred {
        Some(pref_name) => {
            info!(
                "🔊 Attempting to use preferred system audio: '{}'",
                pref_name
            );
            match parse_audio_device(&pref_name) {
                Ok(device) => {
                    info!("✅ Using preferred system audio: '{}'", device.name);
                    Some(Arc::new(device))
                }
                Err(e) => {
                    warn!(
                        "⚠️ Preferred system audio '{}' not available: {}",
                        pref_name, e
                    );
                    warn!("   Falling back to system default...");
                    match default_output_device() {
                        Ok(device) => {
                            info!("✅ Using default system audio: '{}'", device.name);
                            Some(Arc::new(device))
                        }
                        Err(default_err) => {
                            warn!("⚠️ No system audio available (preferred and default both failed): {}", default_err);
                            warn!("   Recording will continue with microphone only");
                            None // System audio is optional
                        }
                    }
                }
            }
        }
        None => {
            info!("🔊 No system audio preference set, using system default");
            match default_output_device() {
                Ok(device) => {
                    info!("✅ Using default system audio: '{}'", device.name);
                    Some(Arc::new(device))
                }
                Err(e) => {
                    warn!("⚠️ No default system audio available: {}", e);
                    warn!("   Recording will continue with microphone only");
                    None // System audio is optional
                }
            }
        }
    }
}
