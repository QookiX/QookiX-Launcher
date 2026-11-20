// ============================================================================
//  create-update-manifest.mjs - Generate the Tauri updater `latest.json` from
//  a GitHub release's assets + the minisign `.sig` signatures produced by
//  `tauri build` (when TAURI_SIGNING_PRIVATE_KEY is set).
//
//  Mirrors the format used by the Tauri updater plugin so the launcher can
//  self-update from GitHub Releases. Every release platform needs a signed bundle.
//
//  Usage:
//    node scripts/create-update-manifest.mjs <release.json> <signatures-dir> <version-tag> <output.json>
//    exit code 0 = wrote manifest, 2 = nothing to write (caller may skip upload)
// ============================================================================
import fs from 'node:fs'
import { requireUpdaterCoverage, selectUpdaterBundles } from './shared/updater-targets.mjs'
import { buildUpdateManifest, readLocalSignature } from './shared/update-manifest.mjs'

const [releasePath, signaturesPath, tag, outputPath] = process.argv.slice(2)
if (!releasePath || !signaturesPath || !tag || !outputPath) {
  throw new Error(
    'Usage: node create-update-manifest.mjs <release.json> <signatures-dir> <version-tag> <output.json>',
  )
}

const release = JSON.parse(fs.readFileSync(releasePath, 'utf8'))
const assets = release.assets
if (!Array.isArray(assets)) {
  throw new Error('Release metadata does not contain an assets array')
}

const bundles = await selectUpdaterBundles(assets, {
  signatureFor: (asset) => readLocalSignature(signaturesPath, asset),
})
if (!bundles.length) {
  console.warn('create-update-manifest: no signed updater bundles found')
  process.exit(2)
}
requireUpdaterCoverage(bundles)

const manifest = buildUpdateManifest({
  tag,
  notes: release.body ?? '',
  minVersion: process.env.QOOKIX_MIN_VERSION,
  bundles,
  urlFor: (asset) => asset.browser_download_url ?? asset.url,
})

fs.writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`)
console.log(
  `create-update-manifest: wrote ${outputPath} for ${Object.keys(manifest.platforms).length} platform(s)`,
)
