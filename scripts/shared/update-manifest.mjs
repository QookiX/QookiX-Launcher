import fs from 'node:fs'
import path from 'node:path'
import { compareReleaseVersions, normalizeReleaseVersion } from './release-version.mjs'

export function readLocalSignature(dir, asset) {
  try {
    return fs.readFileSync(path.join(dir, `${asset.name}.sig`), 'utf8')
  } catch (error) {
    if (error.code === 'ENOENT') return undefined
    throw error
  }
}

export function buildUpdateManifest({ tag, notes = '', minVersion, bundles, urlFor }) {
  const version = normalizeReleaseVersion(tag)
  if (!version) throw new Error(`Invalid release version: ${tag}`)
  const manifest = { version, notes, pub_date: new Date().toISOString(), platforms: {} }
  if (minVersion) {
    const minimum = normalizeReleaseVersion(minVersion)
    if (!minimum || compareReleaseVersions(minimum, version) === 1) {
      throw new Error(`Invalid minimum update version ${minVersion} for ${version}`)
    }
    manifest.minVersion = minimum
  }
  for (const { asset, signature, platforms } of bundles) {
    const url = urlFor(asset)
    if (!url || !signature) throw new Error(`Missing updater URL or signature for ${asset.name}`)
    for (const platform of platforms) manifest.platforms[platform] = { url, signature }
  }
  return manifest
}
