//! The menu bar icon: a single tape reel — the app icon's reel — that turns while a meeting
//! records, with a red REC light in its corner (owner request 2026-09-24).
//!
//! The artwork is drawn by `scripts/tray-icons/render.mjs`. Every reel frame is a macOS
//! *template* image, so the system tints it for the menu bar and the wallpaper behind it,
//! exactly like its neighbours. A template can't hold red, so the light is a small Core
//! Animation layer laid over the corner of the status item's button: red while recording,
//! amber and slowly blinking on HOLD (the in-app lamp's 1.8 s `hold-blink`), absent when
//! idle. A layer never takes clicks, so the menu opens from anywhere on the icon, and Core
//! Animation runs the blink without any work from us.
//!
//! Recording steps through [`REC_FRAMES`] frames 7.5° apart every [`FRAME_MS`], so the reel
//! turns at the in-app take-up hub's ~0.38 rev/s (`components/Transport/Reels.tsx`). Under
//! Reduce Motion it holds the first frame and the HOLD light stays lit, as in the app.
//! Paused stops the reel.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::image::Image;
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Runtime};

use crate::tray::RecordingState;

/// 7.5° a frame at ~18 fps ≈ 0.38 rev/s. The teeth repeat every 120°, so 16 frames loop.
/// (15° at ~9 fps read as choppy; 5° at ~27 fps cost WindowServer about 18% of a core
/// while recording — owner chose 18 fps, 2026-09-24.)
const FRAME_MS: u64 = 55;

const REC_FRAMES: [&[u8]; 16] = [
    include_bytes!("../icons/tray/rec-00.png"),
    include_bytes!("../icons/tray/rec-01.png"),
    include_bytes!("../icons/tray/rec-02.png"),
    include_bytes!("../icons/tray/rec-03.png"),
    include_bytes!("../icons/tray/rec-04.png"),
    include_bytes!("../icons/tray/rec-05.png"),
    include_bytes!("../icons/tray/rec-06.png"),
    include_bytes!("../icons/tray/rec-07.png"),
    include_bytes!("../icons/tray/rec-08.png"),
    include_bytes!("../icons/tray/rec-09.png"),
    include_bytes!("../icons/tray/rec-10.png"),
    include_bytes!("../icons/tray/rec-11.png"),
    include_bytes!("../icons/tray/rec-12.png"),
    include_bytes!("../icons/tray/rec-13.png"),
    include_bytes!("../icons/tray/rec-14.png"),
    include_bytes!("../icons/tray/rec-15.png"),
];

/// Bumped whenever the look changes; a running animation stops once it no longer owns the
/// current generation.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The look showing, so a repeated "recording" doesn't restart the animation.
static CURRENT: Mutex<Option<Look>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Idle,
    Recording,
    Paused,
}

impl Look {
    pub fn for_state(state: &RecordingState) -> Self {
        match state {
            RecordingState::Stopped | RecordingState::Stopping => Look::Idle,
            RecordingState::Starting | RecordingState::Recording | RecordingState::Resuming => {
                Look::Recording
            }
            RecordingState::Pausing | RecordingState::Paused => Look::Paused,
        }
    }
}

fn decode(bytes: &[u8]) -> Image<'static> {
    Image::from_bytes(bytes)
        .expect("bundled tray icon decodes")
        .to_owned()
}

/// The idle reel.
pub fn idle_icon() -> Image<'static> {
    decode(include_bytes!("../icons/tray/idle.png"))
}

/// Set a frame as a template image in one step. `TrayIcon::set_icon` alone clears the
/// template flag, which is how the old icons ended up black on a menu bar where every
/// other icon is white.
fn set_frame<R: Runtime>(tray: &TrayIcon<R>, icon: Image<'static>) {
    if let Err(e) = tray.set_icon_with_as_template(Some(icon), true) {
        log::warn!("Tray: Failed to set icon: {:?}", e);
    }
}

/// Show `look` on the tray. The recording look starts the animation; any other look ends
/// it. Showing the recording look while it is already showing does nothing.
pub fn show<R: Runtime>(app: &AppHandle<R>, look: Look) {
    {
        let Ok(mut current) = CURRENT.lock() else {
            return;
        };
        if *current == Some(Look::Recording) && look == Look::Recording {
            return; // already turning
        }
        *current = Some(look);
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let Some(tray) = app.tray_by_id("main-tray") else {
        log::warn!("Tray: Could not find tray with id 'main-tray'");
        return;
    };

    match look {
        Look::Idle => {
            set_frame(&tray, idle_icon());
            set_light(&tray, Light::Off);
        }
        Look::Paused => {
            set_frame(&tray, decode(REC_FRAMES[0]));
            set_light(&tray, Light::Hold);
        }
        Look::Recording => {
            set_frame(&tray, decode(REC_FRAMES[0]));
            set_light(&tray, Light::Rec);
            tauri::async_runtime::spawn(async move {
                let mut frame = 0usize;
                loop {
                    tokio::time::sleep(Duration::from_millis(FRAME_MS)).await;
                    if GENERATION.load(Ordering::SeqCst) != generation {
                        break;
                    }
                    if reduce_motion() {
                        continue; // hold the frame on screen
                    }
                    frame = (frame + 1) % REC_FRAMES.len();
                    show_rec_frame(&tray, frame, generation);
                }
            });
        }
    }
}

/// Put recording frame `index` on the button. `TrayIcon::set_icon` re-encodes the image
/// as a PNG and rebuilds an NSImage on every call — at ~27 fps that cost ~80% of a core in
/// a debug build — so the frames are built as template NSImages once, on the main thread,
/// and each tick only swaps which one the button shows.
#[cfg(target_os = "macos")]
fn show_rec_frame<R: Runtime>(tray: &TrayIcon<R>, index: usize, generation: u64) {
    use objc2::rc::Retained;
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::NSImage;
    use objc2_foundation::{NSData, NSSize};
    use std::cell::OnceCell;

    thread_local! {
        static FRAMES: OnceCell<Vec<Retained<NSImage>>> = const { OnceCell::new() };
    }

    let _ = tray.with_inner_tray_icon(move |inner| {
        // The look may have changed while this hop to the main thread was queued.
        if GENERATION.load(Ordering::SeqCst) != generation {
            return None;
        }
        let mtm = MainThreadMarker::new()?;
        let button = inner.ns_status_item()?.button(mtm)?;
        FRAMES.with(|cell| {
            let frames = cell.get_or_init(|| {
                REC_FRAMES
                    .iter()
                    .filter_map(|bytes| {
                        let image =
                            NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))?;
                        // tray-icon's size for status item images: 18pt tall.
                        image.setSize(NSSize::new(ICON_PT, ICON_PT));
                        image.setTemplate(true);
                        Some(image)
                    })
                    .collect()
            });
            frames.get(index).map(|image| button.setImage(Some(image)))
        })
    });
}

#[cfg(not(target_os = "macos"))]
fn show_rec_frame<R: Runtime>(_tray: &TrayIcon<R>, _index: usize, _generation: u64) {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Light {
    Off,
    Rec,
    Hold,
}

/// The app's lamp colours (`--lamp-red` / `--lamp-amber` in globals.css, light theme).
const LAMP_RED: (f64, f64, f64) = (232.0, 37.0, 23.0);
const LAMP_AMBER: (f64, f64, f64) = (242.0, 158.0, 13.0);
/// `hold-blink` in tailwind.config.js: 1 → 0.3 → 1 opacity over 1.8 s, ease-in-out.
const HOLD_BLINK_HALF_SECS: f64 = 0.9;
const HOLD_BLINK_DIM: f64 = 0.3;

/// The light's place in the 22×22 artwork box — keep in step with `LIGHT` in
/// `scripts/tray-icons/render.mjs`.
const LIGHT_X: f64 = 18.4;
const LIGHT_Y: f64 = 3.8;
const LIGHT_R: f64 = 2.5;
/// tray-icon draws the status item image 18pt tall, centred in the button.
const ICON_PT: f64 = 18.0;
const ART_BOX: f64 = 22.0;
/// The `name` of our light layer, so later calls find it again.
const LIGHT_LAYER: &str = "nixon-rec-light";

/// Where the light goes in the button, as (x, y, diameter) in the button's own points with
/// y measured from the top. The icon is `ICON_PT` square, centred in `button`.
fn light_frame(button_w: f64, button_h: f64) -> (f64, f64, f64) {
    let scale = ICON_PT / ART_BOX;
    let (ox, oy) = ((button_w - ICON_PT) / 2.0, (button_h - ICON_PT) / 2.0);
    let r = LIGHT_R * scale;
    (ox + LIGHT_X * scale - r, oy + LIGHT_Y * scale - r, 2.0 * r)
}

#[cfg(target_os = "macos")]
fn set_light<R: Runtime>(tray: &TrayIcon<R>, light: Light) {
    use objc2::MainThreadMarker;
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_core_graphics::CGColor;
    use objc2_foundation::NSNumber;
    use objc2_foundation::NSString;
    use objc2_quartz_core::{
        kCAMediaTimingFunctionEaseInEaseOut, CABasicAnimation, CALayer, CAMediaTiming,
        CAMediaTimingFunction,
    };

    let result = tray.with_inner_tray_icon(move |inner| {
        let mtm = MainThreadMarker::new()?;
        let item = inner.ns_status_item()?;
        let button = item.button(mtm)?;
        button.setWantsLayer(true);
        let host = button.layer()?;

        // Remove any light from an earlier call, then draw the new one (if any).
        let name = NSString::from_str(LIGHT_LAYER);
        // SAFETY: read on the main thread, and copied out before any layer is removed, so the
        // array isn't mutated while it is being walked.
        let ours: Vec<_> = unsafe { host.sublayers() }
            .map(|all| all.to_vec())
            .unwrap_or_default()
            .into_iter()
            .filter(|layer| layer.name().is_some_and(|n| n.isEqualToString(&name)))
            .collect();
        for layer in ours {
            layer.removeFromSuperlayer();
        }
        if light == Light::Off {
            return Some(());
        }

        let bounds = button.bounds();
        let (x, top, d) = light_frame(bounds.size.width, bounds.size.height);
        // A flipped host layer measures y from the top like `light_frame`; otherwise flip.
        let y = if host.isGeometryFlipped() || button.isFlipped() {
            top
        } else {
            bounds.size.height - top - d
        };

        let (r, g, b) = if light == Light::Hold {
            LAMP_AMBER
        } else {
            LAMP_RED
        };
        let colour = CGColor::new_srgb(r / 255.0, g / 255.0, b / 255.0, 1.0);
        let layer = CALayer::new();
        layer.setName(Some(&name));
        layer.setFrame(CGRect::new(CGPoint::new(x, y), CGSize::new(d, d)));
        layer.setCornerRadius(d / 2.0);
        layer.setBackgroundColor(Some(&colour));
        if light == Light::Hold && !reduce_motion() {
            let blink =
                CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str("opacity")));
            // SAFETY: `opacity` is a float property, and these are NSNumbers.
            unsafe {
                blink.setFromValue(Some(&NSNumber::new_f64(1.0)));
                blink.setToValue(Some(&NSNumber::new_f64(HOLD_BLINK_DIM)));
            }
            blink.setDuration(HOLD_BLINK_HALF_SECS);
            blink.setAutoreverses(true);
            blink.setRepeatCount(f32::INFINITY);
            // SAFETY: an AppKit-provided immutable constant.
            let ease = unsafe { kCAMediaTimingFunctionEaseInEaseOut };
            blink.setTimingFunction(Some(&CAMediaTimingFunction::functionWithName(ease)));
            layer.addAnimation_forKey(&blink, Some(&NSString::from_str("hold-blink")));
        }
        if let Some(window) = button.window() {
            layer.setContentsScale(window.backingScaleFactor());
        }
        host.addSublayer(&layer);
        Some(())
    });
    if !matches!(result, Ok(Some(()))) {
        log::warn!("Tray: could not draw the REC light ({light:?})");
    }
}

#[cfg(not(target_os = "macos"))]
fn set_light<R: Runtime>(_tray: &TrayIcon<R>, _light: Light) {}

/// System Settings → Accessibility → Display → Reduce motion.
#[cfg(target_os = "macos")]
fn reduce_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

#[cfg(not(target_os = "macos"))]
fn reduce_motion() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_frame_decodes_at_44px() {
        for bytes in REC_FRAMES {
            let img = decode(bytes);
            assert_eq!((img.width(), img.height()), (44, 44));
        }
        assert_eq!(idle_icon().width(), 44);
    }

    #[test]
    fn the_light_sits_in_the_icons_top_right_corner() {
        // A 30×22pt button: the 18pt icon starts at (6, 2).
        let (x, y, d) = light_frame(30.0, 22.0);
        let scale = 18.0 / 22.0;
        assert!((d - 2.0 * 2.5 * scale).abs() < 1e-9);
        assert!((x + d / 2.0 - (6.0 + 18.4 * scale)).abs() < 1e-9);
        assert!((y + d / 2.0 - (2.0 + 3.8 * scale)).abs() < 1e-9);
        // Inside the icon's box, near its top-right corner.
        assert!(x + d <= 6.0 + 18.0 && y >= 2.0);
    }

    #[test]
    fn recording_states_animate_and_paused_holds() {
        assert_eq!(Look::for_state(&RecordingState::Recording), Look::Recording);
        assert_eq!(Look::for_state(&RecordingState::Starting), Look::Recording);
        assert_eq!(Look::for_state(&RecordingState::Paused), Look::Paused);
        assert_eq!(Look::for_state(&RecordingState::Stopped), Look::Idle);
    }
}
