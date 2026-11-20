import { spawnSync } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { bucketManifest } from './create-bucket-manifest.mjs'
import { compareReleaseVersions, normalizeReleaseVersion } from './shared/release-version.mjs'
import { isArtifactName } from './shared/updater-targets.mjs'

const updateBinary = /\.(?:exe|msi|zip|dmg|AppImage|deb|rpm|tar\.gz)(?:\.sig)?$/i
const auxiliary = ['release.json', 'changelog.json', 'SHA256SUMS.txt']

function runAws(args) {
  const endpoint = process.env.AWS_ENDPOINT_URL
  return spawnSync('aws', [...(endpoint ? ['--endpoint-url', endpoint] : []), ...args], {
    encoding: 'utf8',
    windowsHide: true,
    env: { ...process.env, AWS_PAGER: '', AWS_RETRY_MODE: process.env.AWS_RETRY_MODE || 'standard' },
  })
}

function readJson(directory, name) {
  const file = path.join(directory, name)
  if (!fs.statSync(file, { throwIfNoEntry: false })?.isFile()) return undefined
  return JSON.parse(fs.readFileSync(file, 'utf8'))
}

function requireSuccess(result, action) {
  if (result.status !== 0) throw new Error(`${action}: ${result.error?.message ?? result.stderr?.trim() ?? 'AWS CLI failed'}`)
  return result.stdout
}

function publishPointer(aws, bucket, key, file, version, stage) {
  const oldPath = path.join(stage, 'previous.json')
  for (let attempt = 0; attempt < 4; attempt++) {
    const args = ['--bucket', bucket, '--key', key]
    const fetched = aws(['s3api', 'get-object', ...args, oldPath])
    let condition
    if (fetched.status === 0) {
      const metadata = JSON.parse(fetched.stdout)
      const previous = JSON.parse(fs.readFileSync(oldPath, 'utf8'))
      const comparison = compareReleaseVersions(version, previous.version)
      if (!metadata.ETag || comparison == null) throw new Error('The current update manifest could not be verified')
      if (comparison < 0) return false
      condition = ['--if-match', metadata.ETag]
    } else if (/NoSuchKey|\(404\)|Not Found/.test(fetched.stderr ?? '')) {
      condition = ['--if-none-match', '*']
    } else {
      requireSuccess(fetched, 'Read current update manifest')
    }
    const published = aws(['s3api', 'put-object', ...args, '--body', file,
      '--content-type', 'application/json', '--cache-control', 'no-cache', ...condition])
    if (published.status === 0) return true
    if (!/PreconditionFailed|ConditionalRequestConflict|\((?:409|412)\)/.test(published.stderr ?? '')) {
      requireSuccess(published, 'Publish update manifest')
    }
  }
  throw new Error('The bucket update manifest changed repeatedly during publication')
}

export async function pushBucket({ directory, bucket, prefix, publicBase, tag, minVersion }, aws = runAws) {
  if (!directory || !bucket || !publicBase || !/^[a-zA-Z0-9_-]+(?:\/[a-zA-Z0-9_-]+)*$/.test(prefix ?? '')) {
    throw new Error('Provide the release directory, QOOKIX_S3_BUCKET, QOOKIX_S3_PREFIX and QOOKIX_BUCKET_UPDATE_URL')
  }
  if (!/^[a-zA-Z0-9.-]+$/.test(bucket)) throw new Error('Invalid bucket name')
  const metadataVersion = readJson(directory, 'release.json')?.tag_name ?? readJson(directory, 'latest.json')?.version
  const version = normalizeReleaseVersion(tag ?? metadataVersion)
  if (!version || (metadataVersion && normalizeReleaseVersion(metadataVersion) !== version)) {
    throw new Error('Specify a valid SemVer release matching the artifacts directory')
  }
  const uploadId = randomUUID()
  const releasePath = `releases/${version}/${uploadId}`
  const releasePrefix = `${prefix}/${releasePath}`
  const publicPath = `${publicBase.replace(/\/+$/, '')}/releases/${encodeURIComponent(version)}/${uploadId}`
  const manifest = await bucketManifest(directory, version, publicPath, minVersion)
  const names = fs.readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && isArtifactName(entry.name) && (updateBinary.test(entry.name) || auxiliary.includes(entry.name)))
    .map(({ name }) => name).sort()
  const stage = fs.mkdtempSync(path.join(os.tmpdir(), 'qookix-bucket-'))
  try {
    const manifestPath = path.join(stage, 'latest.json')
    fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'wx' })
    const ledger = path.join(stage, 'objects.json')
    const objects = [...names, 'latest.json', 'objects.json'].map((name) => `${releasePrefix}/${name}`)
    fs.writeFileSync(ledger, `${JSON.stringify({ version, uploadId, objects }, null, 2)}\n`, { flag: 'wx' })
    const upload = (file, key) => requireSuccess(
      aws(['s3', 'cp', file, `s3://${bucket}/${key}`, '--no-progress', '--checksum-algorithm', 'CRC32']),
      `Upload ${key}`,
    )
    for (const name of names) upload(path.join(directory, name), `${releasePrefix}/${name}`)
    upload(manifestPath, `${releasePrefix}/latest.json`)
    upload(ledger, `${releasePrefix}/objects.json`)
    const active = publishPointer(aws, bucket, `${prefix}/latest.json`, manifestPath, version, stage)
    if (active) {
      for (const name of auxiliary.filter((name) => names.includes(name))) upload(path.join(directory, name), `${prefix}/${name}`)
    }
    return { version, releasePrefix, active }
  } finally {
    fs.rmSync(stage, { recursive: true, force: true })
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [directory, tag] = process.argv.slice(2)
  const result = await pushBucket({
    directory, tag, bucket: process.env.QOOKIX_S3_BUCKET, prefix: process.env.QOOKIX_S3_PREFIX,
    publicBase: process.env.QOOKIX_BUCKET_UPDATE_URL, minVersion: process.env.QOOKIX_MIN_VERSION,
  })
  console.log(`Uploaded ${result.releasePrefix}${result.active ? ' and published latest.json' : ' (newer update already active)'}`)
}
