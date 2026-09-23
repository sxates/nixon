/**
 * Display formatting for filesystem paths shown in the UI.
 *
 * Nixon shows exactly one absolute path to the user — the recordings save location in
 * Settings — and it contained their macOS short username. That is a real leak the moment
 * anyone screen-shares Settings, files a bug report with a screenshot, or (as happened here)
 * commits a screenshot to a public repo. The path is still the real path everywhere it is
 * USED; only what we render is abbreviated.
 */

/** Matches a macOS/Linux home directory prefix: `/Users/<name>/` or `/home/<name>/`. */
const HOME_PREFIX = /^\/(?:Users|home)\/[^/]+(?=\/|$)/;

/**
 * Replace a home-directory prefix with `~`.
 *
 * `/Users/ada/Movies/nixon-recordings` → `~/Movies/nixon-recordings`
 *
 * Anything that isn't a home path (a volume, a relative path, an empty string) is returned
 * untouched — this never invents a path it wasn't given. Deliberately a pure string
 * transform rather than a `homeDir()` comparison: it needs no async Tauri call to render a
 * settings row, and the pattern is unambiguous on both platforms Nixon runs the check for.
 */
export function tildePath(path: string): string {
  if (!path) return path;
  return path.replace(HOME_PREFIX, '~');
}
