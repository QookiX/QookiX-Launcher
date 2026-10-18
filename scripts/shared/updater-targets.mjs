// ============================================================================
//  updater-targets.mjs - SINGLE source of truth for "which artifact is the
//  Tauri updater bundle on this platform".
//
//  Consumed by:
//    - create-update-manifest-local.mjs (generates latest.json from files)
//    - fix-update-manifest.mjs          (regenerates it from published assets)
//    - verify-artifacts.mjs             (fails the build if none was produced)
//
//  Keeping this in ONE place prevents the three from drifting apart, which
//  previously cost a failed release: the verifier demanded a
//  `*.AppImage.tar.gz` on Linux that Tauri never emits.
//
//  `suffixes` is ordered by PREFERENCE: the first suffix that matches wins.
// ============================================================================

// NOTE on Linux: `bundle.createUpdaterArtifacts` does NOT produce an
// `*.AppImage.tar.gz` — Tauri 2 signs and ships the AppImage binary ITSELF as
// the updater payload. Verified against the published assets of v0.3.8, v0.4.2
// and v0.4.3: each contains `QookiX.Launcher_<ver>_amd64.AppImage` + its
// `.sig`, and no `.AppImage.tar.gz`. The `.tar.gz` form is kept as a fallback
// only, in case a future Tauri release changes this.
//
// NOTE on Windows: likewise no `.nsis.zip` is produced; the updater payload is
// the `_x64-setup.exe` installer itself. Kept as a fallback for the same reason.
//
// `githubHosted`: the bundle is NOT mirrored to the object storage bucket.
// The AppImage is ~90 MB and the bucket endpoint rejects its multipart upload
// with `SlowDownWrite`, so the bucket manifest points it straight at the
// GitHub Release asset instead (the .sig is inlined in `latest.json`, so the
// updater still verifies it).
export const UPDATER_TARGETS = [
  { platforms: ['windows-x86_64'], suffixes: ['_x64-setup.exe', '.nsis.zip'] },
  { platforms: ['darwin-aarch64', 'darwin-x86_64'], suffixes: ['.app.tar.gz'] },
  {
    platforms: ['linux-x86_64'],
    suffixes: ['_amd64.AppImage', '_amd64.AppImage.tar.gz'],
    githubHosted: true,
  },
  {
    platforms: ['linux-aarch64'],
    suffixes: ['_aarch64.AppImage', '_aarch64.AppImage.tar.gz'],
    githubHosted: true,
  },
]

/** GitHub Release asset URL for a given file name (space-safe, like the API). */
export function githubReleaseAssetUrl(
  name,
  tag,
  repo = process.env.GITHUB_REPOSITORY || 'weimosheng/QookiX-Launcher',
) {
  return `https://github.com/${repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`
}

/** True when the asset is served from GitHub Releases instead of the bucket. */
export function isGithubHostedAsset(name) {
  return UPDATER_TARGETS.some(
    (t) => t.githubHosted && t.suffixes.some((suffix) => name.endsWith(suffix)),
  )
}
