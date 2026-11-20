#!/usr/bin/env node
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { normalizeReleaseVersion } from './shared/release-version.mjs'

const scriptPath = fileURLToPath(import.meta.url)
const root = path.resolve(path.dirname(scriptPath), '..')
const RELEASE_FILES = [
  'package.json',
  'package-lock.json',
  'src-tauri/Cargo.toml',
  'src-tauri/Cargo.lock',
  'src-tauri/tauri.conf.json',
]

function requireVersion(version, label) {
  const parsed = normalizeReleaseVersion(version)
  if (!parsed) throw new Error(`${label}: invalid SemVer ${JSON.stringify(version)}`)
  return parsed
}

function jsonVersion(file, text, newVersion, keys) {
  const document = JSON.parse(text)
  const versions = []
  for (const [label, getOwner] of keys) {
    const owner = getOwner(document)
    const version = requireVersion(owner?.version, `${file} ${label}`)
    if (owner.version !== version) throw new Error(`${file} ${label}: version must be normalized`)
    versions.push([label, version])
    if (newVersion) owner.version = newVersion
  }
  if (!newVersion || versions.every(([, version]) => version === newVersion)) {
    return { versions, content: text }
  }
  const newline = text.includes('\r\n') ? '\r\n' : '\n'
  const content = JSON.stringify(document, null, 2).replaceAll('\n', newline) + newline
  return { versions, content }
}

function cargoVersion(file, text, newVersion) {
  const sections = text.split(/(?=^\[)/m)
  const candidates = sections.map((section, index) => ({ section, index })).filter(({ section }) =>
    file.endsWith('Cargo.toml')
      ? /^\[package\]\s*\r?\n/.test(section)
      : /^\[\[package\]\]\r?\n[^\[]*?^name\s*=\s*"qookix-launcher"\s*$/m.test(section))
  if (candidates.length !== 1) throw new Error(`${file}: expected exactly one QookiX Launcher package`)
  const { section, index } = candidates[0]
  const versionLine = /^(version\s*=\s*")([^"]+)(")/gm
  const versions = [...section.matchAll(versionLine)]
  if (versions.length !== 1) throw new Error(`${file}: expected exactly one package version`)
  const version = requireVersion(versions[0][2], file)
  if (version !== versions[0][2]) throw new Error(`${file}: version must be normalized`)
  if (newVersion) {
    sections[index] = section.replace(versionLine, (_, prefix, _previous, suffix) => prefix + newVersion + suffix)
  }
  return { versions: [['version', version]], content: sections.join('') }
}

function planVersionFiles(directory, newVersion) {
  return RELEASE_FILES.map((file) => {
    const filename = path.join(directory, file)
    const previous = fs.readFileSync(filename, 'utf8')
    let next
    if (file.endsWith('.json')) {
      const fields = [['version', (document) => document]]
      if (file === 'package-lock.json') {
        fields.push(['packages[""].version', (document) => document.packages?.['']])
      }
      next = jsonVersion(file, previous, newVersion, fields)
    } else {
      next = cargoVersion(file, previous, newVersion)
    }
    return { filename, file, previous, ...next }
  })
}

export function verifyReleaseVersion(directory, reference) {
  const files = planVersionFiles(directory)
  const version = reference == null ? files[0].versions[0][1] : requireVersion(reference, 'Release reference')
  const mismatches = files.flatMap(({ file, versions }) => versions
    .filter(([, value]) => value !== version)
    .map(([field, value]) => `${file} ${field}=${value}`))
  if (mismatches.length) throw new Error(`Expected ${version}. Mismatches: ${mismatches.join(', ')}`)
  return version
}

export function synchronizeReleaseVersion(directory, input) {
  const version = requireVersion(input, 'New version')
  const changes = planVersionFiles(directory, version).filter(({ previous, content }) => previous !== content)
  if (changes.length === 0) return version

  const staged = fs.mkdtempSync(path.join(directory, '.version-update-'))
  const committed = []
  let keepRecovery = false
  try {
    for (const [index, change] of changes.entries()) {
      const mode = fs.statSync(change.filename).mode & 0o777
      change.backup = path.join(staged, `${index}.original`)
      change.next = path.join(staged, `${index}.next`)
      fs.writeFileSync(change.backup, change.previous, { mode, flag: 'wx' })
      fs.writeFileSync(change.next, change.content, { mode, flag: 'wx' })
    }
    for (const change of changes) {
      if (fs.readFileSync(change.filename, 'utf8') !== change.previous) {
        throw new Error(`${change.file} was modified while preparing the update`)
      }
    }
    for (const change of changes) {
      fs.renameSync(change.next, change.filename)
      committed.push(change)
    }
    verifyReleaseVersion(directory, version)
  } catch (error) {
    const failed = []
    for (const change of committed.reverse()) {
      try {
        fs.renameSync(change.backup, change.filename)
      } catch {
        failed.push(change.file)
      }
    }
    if (failed.length) {
      keepRecovery = true
      throw new Error(`Rollback failed for ${failed.join(', ')}. Originals are retained at ${staged}`, { cause: error })
    }
    throw error
  } finally {
    if (!keepRecovery) fs.rmSync(staged, { recursive: true, force: true })
  }
  return version
}

if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  try {
    const [first, second, ...extra] = process.argv.slice(2)
    if (!first || extra.length || (first !== '--check' && second !== undefined)) {
      throw new Error('Usage: node scripts/update-version.mjs <version> | --check [tag]')
    }
    const version = first === '--check' ? verifyReleaseVersion(root, second) : synchronizeReleaseVersion(root, first)
    console.log(version)
  } catch (error) {
    console.error(error.message)
    process.exitCode = 1
  }
}
