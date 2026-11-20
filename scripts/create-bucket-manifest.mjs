import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { requireUpdaterCoverage, selectUpdaterBundles } from './shared/updater-targets.mjs'
import { buildUpdateManifest, readLocalSignature } from './shared/update-manifest.mjs'

export async function bucketManifest(directory, tag, base, minVersion) {
  const baseUrl = new URL(base.endsWith('/') ? base : `${base}/`)
  if (!['https:', 'http:'].includes(baseUrl.protocol) || baseUrl.search || baseUrl.hash
      || baseUrl.username || baseUrl.password) {
    throw new Error('The bucket base must be a public HTTP(S) directory URL')
  }
  const assets = fs.readdirSync(directory, { withFileTypes: true }).filter((entry) => entry.isFile())
  const bundles = await selectUpdaterBundles(assets, {
    signatureFor: (asset) => readLocalSignature(directory, asset),
  })
  requireUpdaterCoverage(bundles)
  if (!minVersion && assets.some(({ name }) => name === 'latest.json')) {
    minVersion = JSON.parse(fs.readFileSync(path.join(directory, 'latest.json'), 'utf8')).minVersion
  }
  return buildUpdateManifest({ tag, minVersion, bundles, urlFor: ({ name }) => new URL(encodeURIComponent(name), baseUrl).href })
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [directory, tag, output, publicBase = process.env.QOOKIX_BUCKET_UPDATE_URL] = process.argv.slice(2)
  if (!directory || !tag || !output || !publicBase) {
    throw new Error('Usage: create-bucket-manifest.mjs <artifacts-dir> <version-tag> <output.json> [public-base-url]')
  }
  const manifest = await bucketManifest(directory, tag, publicBase, process.env.QOOKIX_MIN_VERSION)
  fs.writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`)
  console.log(`Wrote ${output} for ${Object.keys(manifest.platforms).join(', ')}`)
}
