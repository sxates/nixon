//! specs/0073 (owner decision Q4) — recordings stay on this Mac's own drive.
//!
//! A recordings folder on a drive that can vanish (a USB stick, an SD card, a mounted disk
//! image, a network share) breaks recording and playback the moment it is unplugged or
//! unreachable. So both the plain "Change…" path and the recordings mover refuse such a
//! target: [`ensure_recordings_volume_allowed`] is the one check they share.
//!
//! iCloud Drive is on the internal disk but can evict files to the cloud, so it is not
//! refused; [`is_icloud_drive_path`] lets the caller warn instead.

use std::path::{Path, PathBuf};

/// The user-facing refusal. Exact copy from the spec.
pub const REMOVABLE_OR_NETWORK_REFUSAL: &str = "Recordings need to stay on this Mac's own \
     drive, so Nixon can't use a removable or network drive for them.";

/// The volume facts the decision needs. `None` = the OS didn't say.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VolumeProps {
    pub local: Option<bool>,
    pub internal: Option<bool>,
    pub removable: Option<bool>,
    pub ejectable: Option<bool>,
}

/// Pure decision: is a volume with these properties an acceptable recordings location?
///
/// Refused when it is removable, ejectable, known not to be internal, or known not to be
/// local (a network mount). Properties the OS didn't report don't refuse on their own.
pub fn volume_allowed(props: VolumeProps) -> bool {
    props.local != Some(false)
        && props.internal != Some(false)
        && props.removable != Some(true)
        && props.ejectable != Some(true)
}

/// The nearest existing ancestor of `path` (the path itself when it exists). A target that
/// will be created lives on the volume of the folder it will be created in.
fn nearest_existing(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|p| p.exists()).map(Path::to_path_buf)
}

/// Refuse a recordings location on a removable, ejectable or network volume.
///
/// `path` may not exist yet; its nearest existing ancestor decides. Returns the
/// user-facing refusal as the error.
pub fn ensure_recordings_volume_allowed(path: &Path) -> anyhow::Result<()> {
    let Some(existing) = nearest_existing(path) else {
        anyhow::bail!("Could not find the folder {}", path.display());
    };
    let props = volume_props(&existing);
    log::debug!(
        "recordings volume check for {}: {props:?}",
        existing.display()
    );
    if volume_allowed(props) {
        Ok(())
    } else {
        Err(anyhow::anyhow!(REMOVABLE_OR_NETWORK_REFUSAL))
    }
}

/// Is `path` inside iCloud Drive (`~/Library/Mobile Documents`)? Allowed, but worth a
/// warning: iCloud can evict a recording's audio to the cloud.
pub fn is_icloud_drive_path(path: &Path) -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    let icloud = home.join("Library").join("Mobile Documents");
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    canon(path).starts_with(canon(&icloud))
}

#[cfg(target_os = "macos")]
fn volume_props(path: &Path) -> VolumeProps {
    use core_foundation::base::{CFTypeRef, TCFType};
    use core_foundation::boolean::{CFBoolean, CFBooleanRef};
    use core_foundation::string::CFStringRef;
    use core_foundation::url::{
        kCFURLVolumeIsEjectableKey, kCFURLVolumeIsInternalKey, kCFURLVolumeIsLocalKey,
        kCFURLVolumeIsRemovableKey, CFURLRef, CFURL,
    };

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFURLCopyResourcePropertyForKey(
            url: CFURLRef,
            key: CFStringRef,
            value: *mut CFTypeRef,
            error: *mut *mut std::ffi::c_void,
        ) -> u8;
    }

    let Some(url) = CFURL::from_path(path, path.is_dir()) else {
        return VolumeProps::default();
    };
    let read = |key: CFStringRef| -> Option<bool> {
        let mut value: CFTypeRef = std::ptr::null();
        // SAFETY: `url` is a live CFURL, `key` is one of CoreFoundation's own constant
        // keys, and on success we own `value` (Copy rule), which `wrap_under_create_rule`
        // takes over and releases. A NULL value (the key doesn't apply) is `None`.
        unsafe {
            let ok = CFURLCopyResourcePropertyForKey(
                url.as_concrete_TypeRef(),
                key,
                &mut value,
                std::ptr::null_mut(),
            );
            if ok == 0 || value.is_null() {
                return None;
            }
            let b = CFBoolean::wrap_under_create_rule(value as CFBooleanRef);
            Some(bool::from(b))
        }
    };
    // SAFETY: reading CoreFoundation's exported constant key statics.
    unsafe {
        VolumeProps {
            local: read(kCFURLVolumeIsLocalKey),
            internal: read(kCFURLVolumeIsInternalKey),
            removable: read(kCFURLVolumeIsRemovableKey),
            ejectable: read(kCFURLVolumeIsEjectableKey),
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn volume_props(_path: &Path) -> VolumeProps {
    VolumeProps::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERNAL_DISK: VolumeProps = VolumeProps {
        local: Some(true),
        internal: Some(true),
        removable: Some(false),
        ejectable: Some(false),
    };

    #[test]
    fn the_internal_disk_is_allowed() {
        assert!(volume_allowed(INTERNAL_DISK));
    }

    #[test]
    fn removable_ejectable_external_and_network_volumes_are_refused() {
        let usb = VolumeProps {
            internal: Some(false),
            removable: Some(true),
            ejectable: Some(true),
            ..INTERNAL_DISK
        };
        let disk_image = VolumeProps {
            internal: Some(false),
            ejectable: Some(true),
            ..INTERNAL_DISK
        };
        let external_ssd = VolumeProps {
            internal: Some(false),
            ..INTERNAL_DISK
        };
        let network = VolumeProps {
            local: Some(false),
            internal: None,
            removable: None,
            ejectable: None,
        };
        for props in [usb, disk_image, external_ssd, network] {
            assert!(!volume_allowed(props), "{props:?} must be refused");
        }
    }

    #[test]
    fn unknown_properties_do_not_refuse_on_their_own() {
        assert!(volume_allowed(VolumeProps::default()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_temp_folder_on_the_boot_disk_is_allowed_even_before_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let props = volume_props(dir.path());
        assert_eq!(props.local, Some(true), "the boot disk reports as local");
        assert_eq!(
            props.internal,
            Some(true),
            "the boot disk reports as internal"
        );
        assert!(ensure_recordings_volume_allowed(&dir.path().join("not/yet/created")).is_ok());
    }

    #[test]
    fn icloud_drive_is_recognised() {
        let home = dirs::home_dir().unwrap();
        assert!(is_icloud_drive_path(
            &home.join("Library/Mobile Documents/com~apple~CloudDocs/Nixon")
        ));
        assert!(!is_icloud_drive_path(&home.join("Movies/nixon-recordings")));
    }
}
