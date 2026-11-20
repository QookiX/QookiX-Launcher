// ============================================================================
//  create-update-manifest-local.mjs - Generate the Tauri updater `latest.json`
//  from LOCAL build artifacts only.
//
//  Unlike the release-API-based variant, asset URLs are derived directly from
//  the tag, so this works even before the GitHub Release exists — which makes
//  the publish step robust against release-state churn (deleted/untagged
//  releases, parallel runs, etc.).
//
//  Usage:
//    node scripts/create-update-manifest-local.mjs <artifacts-dir> <version-tag> <output.json>
//    env: GITHUB_REPOSITORY (e.g. "owner/repo")
//    exit code 0 = wrote manifest, 2 = nothing to write (caller may skip)
// ============================================================================
import fs from 'node:fs'
import { requireUpdaterCoverage, selectUpdaterBundles } from './shared/updater-targets.mjs'
import { buildUpdateManifest, readLocalSignature } from './shared/update-manifest.mjs'

const [dir, tag, outputPath] = process.argv.slice(2)
if (!dir || !tag || !outputPath) {
  throw new Error(
    'Usage: node create-update-manifest-local.mjs <artifacts-dir> <version-tag> <output.json>',
  )
}
const repo = process.env.GITHUB_REPOSITORY
if (!repo) throw new Error('GITHUB_REPOSITORY is not set')

const artifacts = fs.readdirSync(dir, { withFileTypes: true })
  .filter((entry) => entry.isFile())
  .map(({ name }) => ({ name }))
const bundles = await selectUpdaterBundles(artifacts, {
  signatureFor: (asset) => readLocalSignature(dir, asset),
})
if (!bundles.length) {
  console.warn('create-update-manifest-local: no signed updater bundles found')
  process.exit(2)
}
requireUpdaterCoverage(bundles)

const manifest = buildUpdateManifest({
  tag,
  minVersion: process.env.QOOKIX_MIN_VERSION,
  bundles,
  urlFor: ({ name }) => `https://github.com/${repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`,
})

fs.writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`)
console.log(
  `create-update-manifest-local: wrote ${outputPath} for ${Object.keys(manifest.platforms).length} platform(s)`,
)
