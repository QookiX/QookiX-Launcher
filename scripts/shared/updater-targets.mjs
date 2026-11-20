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
export const UPDATER_TARGETS = [
  { platforms: ['windows-x86_64'], suffixes: ['_x64-setup.exe', '.nsis.zip'] },
  { platforms: ['darwin-aarch64'], suffixes: ['_aarch64.app.tar.gz', '_universal.app.tar.gz'] },
  { platforms: ['darwin-x86_64'], suffixes: ['_x64.app.tar.gz', '_universal.app.tar.gz'] },
  { platforms: ['linux-x86_64'], suffixes: ['_amd64.AppImage', '_amd64.AppImage.tar.gz'] },
  { platforms: ['linux-aarch64'], suffixes: ['_aarch64.AppImage', '_aarch64.AppImage.tar.gz'] },
]

export const RELEASE_UPDATE_PLATFORMS = [
  'windows-x86_64', 'darwin-aarch64', 'darwin-x86_64', 'linux-x86_64',
]

export function requireUpdaterCoverage(bundles, required = RELEASE_UPDATE_PLATFORMS) {
  const present = new Set(bundles.flatMap(({ platforms }) => platforms))
  const missing = required.filter((platform) => !present.has(platform))
  if (missing.length) throw new Error(`Missing updater artifacts: ${missing.join(', ')}`)
}

export function isArtifactName(name) {
  return typeof name === 'string' && name.length > 0 && !/[\\/:\u0000-\u001f]/.test(name)
}

/**
 * Pick one artifact per target. A higher-priority artifact without a signature
 * may fall back to a signed lower-priority format. Ambiguity is an error.
 */
export async function selectUpdaterBundles(assets, {
  targets = UPDATER_TARGETS,
  signatureFor,
} = {}) {
  const selected = []
  for (const target of targets) {
    for (const suffix of target.suffixes) {
      const candidates = []
      for (const asset of assets) {
        if (!isArtifactName(asset.name) || !asset.name.endsWith(suffix)) continue
        const signature = signatureFor ? (await signatureFor(asset))?.trim() : undefined
        if (!signatureFor || signature) candidates.push({ asset, signature })
      }
      if (candidates.length > 1) {
        throw new Error(`Ambiguous updater bundles for ${target.platforms.join('/')}: ${candidates.map(({ asset }) => asset.name).join(', ')}`)
      }
      if (candidates.length === 1) {
        selected.push({ platforms: target.platforms, ...candidates[0] })
        break
      }
    }
  }
  return selected
}
