import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { fileDigest, githubReleases, matchesReleaseTag, releaseAssetUrl, sha256 } from './shared/github-releases.mjs'
import { requireUpdaterCoverage, selectUpdaterBundles } from './shared/updater-targets.mjs'
import { buildUpdateManifest, readLocalSignature } from './shared/update-manifest.mjs'
import { normalizeReleaseVersion } from './shared/release-version.mjs'

const manifestName = 'latest.json'
const checksumName = 'SHA256SUMS.txt'
const downloadFile = /\.(?:exe|msi|zip|dmg|AppImage|deb|rpm|tar\.gz)$/i

async function verifyDraftFiles(client, directory, assets) {
  const local = fs.readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && ![manifestName, checksumName].includes(entry.name))
  const localBundles = await selectUpdaterBundles(local, {
    signatureFor: (asset) => readLocalSignature(directory, asset),
  })
  requireUpdaterCoverage(localBundles)
  const matched = new Map()
  for (const { name } of local) {
    const normalized = name.replace(/\s+/g, '.')
    const candidates = assets.filter((asset) => asset.name === name || asset.name === normalized)
    if (candidates.length !== 1) throw new Error(`Expected exactly one uploaded copy of ${name}`)
    const [asset] = candidates
    if (matched.has(asset.name) || asset.state !== 'uploaded') throw new Error(`Invalid draft asset ${asset.name}`)
    const localFile = path.join(directory, name)
    const expected = await fileDigest(localFile)
    if (asset.size !== fs.statSync(localFile).size || await client.digest(asset) !== expected) {
      throw new Error(`Uploaded bytes differ from the verified build: ${asset.name}`)
    }
    matched.set(asset.name, expected)
  }
  if (assets.some(({ name }) => (downloadFile.test(name) || name.endsWith('.sig')) && !matched.has(name))) {
    throw new Error('The draft contains binary or signature assets outside the verified release directory')
  }
  return matched
}

async function writeDraftAsset(client, release, name, bytes) {
  const old = release.assets.find((asset) => asset.name === name)
  if (old && await client.digest(old) === await sha256([bytes])) return
  if (old) await client.remove(old.id)
  await client.upload(release.id, name, bytes)
}

export async function finalizeRelease(client, release, { tag, minVersion, directory, publish = false, draftId }) {
  if (!release?.id || !matchesReleaseTag(release, tag, draftId) || !Array.isArray(release.assets)) {
    throw new Error('The requested GitHub release is missing')
  }
  const { assets } = release
  const oldManifest = assets.find((asset) => asset.name === manifestName)
  if (!release.draft && (publish || oldManifest)) {
    throw new Error('This release is public. Prepare a draft release to publish a corrected update manifest.')
  }
  if (publish && !directory) throw new Error('The verified build directory is required to publish a draft')
  const matched = directory ? await verifyDraftFiles(client, directory, assets) : null

  const byName = new Map(assets.map((asset) => [asset.name, asset]))
  const bundles = await selectUpdaterBundles(assets.filter((asset) => asset.state === 'uploaded'), {
    signatureFor: (asset) => {
      const signature = byName.get(`${asset.name}.sig`)
      return signature?.state === 'uploaded' ? client.text(signature, 32 * 1024) : undefined
    },
  })
  requireUpdaterCoverage(bundles)
  if (matched && bundles.some(({ asset }) => !matched.has(asset.name) || !matched.has(`${asset.name}.sig`))) {
    throw new Error('An updater bundle differs from the verified artifacts')
  }

  if (!minVersion && oldManifest) {
    minVersion = JSON.parse(await client.text(oldManifest)).minVersion
  }
  const manifest = buildUpdateManifest({
    tag,
    notes: release.body ?? '',
    minVersion,
    bundles,
    urlFor: (asset) => releaseAssetUrl(asset, tag),
  })
  manifest.pub_date = release.created_at ?? manifest.pub_date
  if (matched) {
    const checksums = [...matched].filter(([name]) => downloadFile.test(name))
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([name, digest]) => `${digest.slice('sha256:'.length)}  ${name}`).join('\n')
    await writeDraftAsset(client, release, checksumName, Buffer.from(`${checksums}\n`))
  }
  await writeDraftAsset(client, release, manifestName, Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`))
  if (publish) {
    const published = await client.edit(release.id, {
      tag_name: tag, draft: false, make_latest: release.prerelease ? 'false' : 'true',
    })
    if (published.id !== release.id || published.tag_name !== tag || published.draft !== false) {
      throw new Error('Could not confirm release publication')
    }
  }
  return manifest
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const tag = process.env.GITHUB_TAG
  if (!normalizeReleaseVersion(tag)) throw new Error('GITHUB_TAG must identify a SemVer release')
  const args = process.argv.slice(2)
  const client = githubReleases(process.env.GITHUB_REPOSITORY, process.env.GH_TOKEN || process.env.GITHUB_TOKEN)
  if (args[0] === '--assert-unpublished') {
    const release = await client.find(tag)
    if (release && !release.draft) throw new Error(`Release ${tag} is already public`)
  } else {
    const publish = args[0] === '--publish-draft'
    if ((publish && args.length !== 2) || (!publish && args.length)) {
      throw new Error('Usage: fix-update-manifest.mjs [--assert-unpublished | --publish-draft <verified-artifacts>]')
    }
    const id = process.env.QOOKIX_RELEASE_ID
    const draftId = id === undefined ? undefined : Number(id)
    const release = draftId === undefined ? await client.find(tag) : await client.getDraft(tag, draftId)
    const manifest = await finalizeRelease(client, release, {
      tag, minVersion: process.env.QOOKIX_MIN_VERSION, directory: publish ? args[1] : undefined, publish, draftId,
    })
    console.log(`Verified updater manifest for ${tag}: ${Object.keys(manifest.platforms).join(', ')}`)
  }
}
