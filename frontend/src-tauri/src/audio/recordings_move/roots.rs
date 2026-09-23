//! Which recordings folders a move may read, move from, count in, and remove.
//!
//! A debug build ("Dev Nixon") knows about the PRODUCTION recordings folders: they are on
//! its read allow-list so dev meetings recorded before the 0070 dev root still play. That
//! allow-list must never become a move list. A dev build moving the owner's real
//! recordings into a dev folder, or removing a production folder it thinks it emptied,
//! would be destructive. So every root is sorted here, once, into what this build profile
//! owns and what it only reads:
//!
//! - `protected`: never moved from, never counted, never removed. Debug builds: the release
//!   build's recordings folder and the platform default folders. Release builds: none.
//! - `sources`: the current and earlier folders of THIS profile (minus protected and the
//!   target). Loose entries are counted only here.
//! - `removable`: the roots the end-of-run cleanup may remove once a move empties them:
//!   `sources`, plus, in a release build, the platform default folders (an install that
//!   changed folder before specs/0073 never recorded its old default as a previous root).

use std::path::{Path, PathBuf};

use crate::audio::meeting_folder::canonical_or_lexical;

/// The roots one move works with. See the module doc for what each list may be used for.
#[derive(Debug, Clone, Default)]
pub struct MoveRoots {
    /// Where every meeting ends up.
    pub target: PathBuf,
    /// This profile's current and earlier roots, minus `protected` and `target`.
    pub sources: Vec<PathBuf>,
    /// Every root Nixon knows about (the ownership check's list), including `target`.
    pub known: Vec<PathBuf>,
    /// Roots this build must never move from, count loose entries in, or remove.
    pub protected: Vec<PathBuf>,
    /// Roots the end-of-run cleanup may remove once the run has emptied them.
    pub removable: Vec<PathBuf>,
}

impl MoveRoots {
    /// Is `path` one of the protected roots or inside one?
    pub fn is_protected(&self, path: &Path) -> bool {
        self.protected.iter().any(|r| path.starts_with(r))
    }
}

/// Everything [`roots_for_profile`] needs, gathered by the caller.
#[derive(Debug, Clone, Default)]
pub struct ProfileRootsInput {
    /// A debug ("Dev Nixon") build.
    pub dev: bool,
    pub target: PathBuf,
    /// The current recordings root (before the change, for a Change…).
    pub current: PathBuf,
    /// The persisted `previous_save_folders`.
    pub previous: Vec<PathBuf>,
    /// The platform default folders (`meetily-recordings`, `nixon-recordings`).
    pub platform_defaults: Vec<PathBuf>,
    /// The folder a release build would record into.
    pub release_root: PathBuf,
    /// `known_recording_roots()`.
    pub known: Vec<PathBuf>,
}

fn push_unique(out: &mut Vec<PathBuf>, path: PathBuf) {
    if !out.contains(&path) {
        out.push(path);
    }
}

/// Sort the roots for one move by what this build profile owns. Pure apart from
/// canonicalizing paths that exist.
pub fn roots_for_profile(input: ProfileRootsInput) -> MoveRoots {
    let canon = |p: &Path| canonical_or_lexical(p);
    let target = canon(&input.target);

    let mut protected = Vec::new();
    if input.dev {
        for root in input
            .platform_defaults
            .iter()
            .chain(std::iter::once(&input.release_root))
        {
            push_unique(&mut protected, canon(root));
        }
    }
    let is_protected = |p: &Path| protected.iter().any(|r| p.starts_with(r));

    let mut sources = Vec::new();
    for root in std::iter::once(&input.current).chain(input.previous.iter()) {
        let root = canon(root);
        if root != target && !is_protected(&root) {
            push_unique(&mut sources, root);
        }
    }

    let mut removable = sources.clone();
    if !input.dev {
        for root in &input.platform_defaults {
            let root = canon(root);
            if root != target && !is_protected(&root) {
                push_unique(&mut removable, root);
            }
        }
    }

    let mut known = vec![target.clone()];
    for root in input.known.iter().chain(sources.iter()) {
        push_unique(&mut known, canon(root));
    }

    MoveRoots {
        target,
        sources,
        known,
        protected,
        removable,
    }
}

/// [`roots_for_profile`] for this running build, moving into `target`.
pub fn profile_roots(target: &Path) -> MoveRoots {
    use crate::audio::recording_preferences as prefs;
    roots_for_profile(ProfileRootsInput {
        dev: prefs::is_dev_build(),
        target: target.to_path_buf(),
        current: prefs::recordings_root(),
        previous: prefs::previous_recording_roots(),
        platform_defaults: prefs::platform_default_recordings_folders(),
        release_root: prefs::release_default_recordings_folder(),
        known: prefs::known_recording_roots(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(dev: bool) -> ProfileRootsInput {
        ProfileRootsInput {
            dev,
            target: "/r/new".into(),
            current: "/r/dev-now".into(),
            // Worst case: the release root ended up in the dev profile's previous list.
            previous: vec!["/r/dev-old".into(), "/m/nixon-recordings".into()],
            platform_defaults: vec!["/m/meetily-recordings".into(), "/m/nixon-recordings".into()],
            release_root: "/m/nixon-recordings".into(),
            known: vec!["/m/meetily-recordings".into(), "/m/nixon-recordings".into()],
        }
    }

    #[test]
    fn a_debug_build_owns_none_of_the_production_folders() {
        let roots = roots_for_profile(input(true));
        let prod = [
            PathBuf::from("/m/meetily-recordings"),
            PathBuf::from("/m/nixon-recordings"),
        ];
        for p in &prod {
            assert!(roots.is_protected(p), "{p:?} must be protected");
            assert!(!roots.sources.contains(p), "{p:?} must not be a source");
            assert!(!roots.removable.contains(p), "{p:?} must never be removed");
        }
        assert!(roots.is_protected(Path::new("/m/nixon-recordings/Some meeting")));
        assert_eq!(
            roots.sources,
            vec![PathBuf::from("/r/dev-now"), PathBuf::from("/r/dev-old")]
        );
        // Still readable: they stay in the ownership check's list.
        assert!(roots.known.contains(&prod[0]));
    }

    #[test]
    fn a_release_build_may_clean_up_the_legacy_default_folders() {
        let roots = roots_for_profile(input(false));
        assert!(roots.protected.is_empty());
        assert!(roots
            .removable
            .contains(&PathBuf::from("/m/meetily-recordings")));
        // …but counts loose entries only in its own current and earlier folders.
        assert!(!roots
            .sources
            .contains(&PathBuf::from("/m/meetily-recordings")));
    }

    #[test]
    fn the_target_is_never_a_source_or_removable() {
        let mut i = input(false);
        i.target = "/r/dev-old".into();
        let roots = roots_for_profile(i);
        assert!(!roots.sources.contains(&PathBuf::from("/r/dev-old")));
        assert!(!roots.removable.contains(&PathBuf::from("/r/dev-old")));
        assert_eq!(roots.known[0], PathBuf::from("/r/dev-old"));
    }
}
