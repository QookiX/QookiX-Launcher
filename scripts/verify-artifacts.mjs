// ============================================================================
//  verify-artifacts.mjs - Fail the build EARLY if the collected release
//  artifacts are incomplete, instead of shipping a broken release.
//
//  This guards the bundle-target optimisations: whenever `bundle.targets` is
//  narrowed (e.g. dropping the MSI on Windows) there is a risk of also losing
//  the updater bundle, which would silently break in-app auto-updates.
//
//  Usage:
//    node scripts/verify-artifacts.mjs <artifacts-dir> <windows|macos|linux>
//    env: TAURI_SIGNING_PRIVATE_KEY (set => the updater bundle needs a .sig)
// ============================================================================
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { UPDATER_TARGETS, RELEASE_UPDATE_PLATFORMS, requireUpdaterCoverage, selectUpdaterBundles } from './shared/updater-targets.mjs'
import { readLocalSignature } from './shared/update-manifest.mjs'
import { normalizeReleaseVersion } from './shared/release-version.mjs'
import { macExecutableArchitectures } from './shared/macho.mjs'

const [dir, platform, expected] = process.argv.slice(2)
if (!dir || !platform) {
  throw new Error('Usage: verify-artifacts.mjs <artifacts-dir> <windows|macos|linux>')
}

// CI matrix value -> Tauri target-triple prefix used by UPDATER_TARGETS.
const TRIPLE_PREFIX = { windows: 'windows', macos: 'darwin', linux: 'linux' }
const prefix = TRIPLE_PREFIX[platform]
if (!prefix) throw new Error(`Unknown platform "${platform}" (expected windows|macos|linux)`)

const targets = UPDATER_TARGETS.filter((target) => target.platforms.some((name) => name.startsWith(prefix)))

// What a human can download and install by hand.
// NOTE: Tauri inserts the NSIS language code into MSI names
// (`QookiX.Launcher_0.4.3_x64_en-US.msi`), hence the wildcard before `.msi`.
const INSTALLERS = {
  windows: /(_x64-setup\.exe|_x64.*\.msi|_x64_portable\.zip)$/,
  macos: /\.dmg$/,
  linux: /\.(AppImage|deb|rpm)$/,
}

if (!fs.statSync(dir, { throwIfNoEntry: false })?.isDirectory()) {
  throw new Error(`Artifacts directory does not exist: ${dir}`)
}
const files = fs.readdirSync(dir, { withFileTypes: true }).filter((entry) => entry.isFile())
const installers = files.filter(({ name }) => INSTALLERS[platform].test(name))
if (!installers.length) throw new Error(`No installable artifact found for ${platform}`)

const signatureFor = process.env.TAURI_SIGNING_PRIVATE_KEY
  ? (asset) => readLocalSignature(dir, asset)
  : undefined
const bundles = await selectUpdaterBundles(files, { targets, signatureFor })
if (!bundles.length) throw new Error(`No ${signatureFor ? 'signed ' : ''}updater bundle found for ${platform}`)
requireUpdaterCoverage(bundles, RELEASE_UPDATE_PLATFORMS.filter((target) => target.startsWith(`${prefix}-`)))

if (signatureFor) {
  const config = JSON.parse(fs.readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'))
  const publicKey = config.plugins.updater.pubkey
  const manifest = fileURLToPath(new URL('./updater-signature-check/Cargo.toml', import.meta.url))
  const releases = [...new Set(bundles.map(({ asset }) => path.resolve(dir, asset.name)))]
  execFileSync('cargo', ['run', '--quiet', '--locked', '--manifest-path', manifest, '--', publicKey, ...releases], {
    stdio: 'inherit',
  })
}

const version = expected ? normalizeReleaseVersion(expected) : null
if (expected && !version) throw new Error(`Invalid expected artifact version: ${expected}`)
if (platform === 'macos') {
  const verified = new Set()
  for (const { asset } of bundles) {
    if (verified.has(asset.name)) continue
    verified.add(asset.name)
    verifyMacBundle(asset.name, version)
  }
}

if (version) {
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const suffixes = {
    windows: String.raw`(?:_x64(?:-setup\.exe|-setup\.nsis\.zip|_portable\.zip|(?:_[\w-]+)?\.msi|\.nsis\.zip)|\.nsis\.zip)`,
    macos: String.raw`(?:_(?:aarch64|arm64|x86_64|x64|universal))?\.dmg`,
    linux: String.raw`(?:_(?:amd64|aarch64|arm64|armhf|i386|x86_64)\.(?:AppImage(?:\.tar\.gz)?|deb)|-[0-9]+\.(?:x86_64|aarch64|i686)\.rpm)`,
  }
  const label = new RegExp(`(?:^|[-_.])${escaped}${suffixes[platform]}$`)
  for (const { name } of [...installers, ...bundles.map(({ asset }) => asset)]) {
    if (!name.endsWith('.app.tar.gz') && !label.test(name)) {
      throw new Error(`${name}: artifact filename does not match version ${version}`)
    }
  }
}

function verifyMacBundle(name, expectedVersion) {
  const filename = path.resolve(dir, name)
  const members = execFileSync('tar', ['-tf', filename], { encoding: 'utf8' }).split(/\r?\n/)
  const plists = members.filter((entry) => /\.app\/Contents\/Info\.plist$/.test(entry))
  if (plists.length !== 1) throw new Error(`Could not find the application Info.plist in ${name}`)
  const info = execFileSync('tar', ['-xOf', filename, '--', plists[0]], { encoding: 'utf8', maxBuffer: 256 * 1024 })
  const bundleVersions = [...info.matchAll(/<key>CFBundle(?:ShortVersionString|Version)<\/key>\s*<string>([^<]+)<\/string>/g)]
  if (expectedVersion && !bundleVersions.some(([, content]) => normalizeReleaseVersion(content) === expectedVersion)) {
    throw new Error(`${name}: application version does not match ${expectedVersion}`)
  }
  const executable = /<key>CFBundleExecutable<\/key>\s*<string>([^/<]+)<\/string>/.exec(info)?.[1]
  const program = plists[0].replace(/Info\.plist$/, `MacOS/${executable}`)
  if (!executable || members.filter((member) => member === program).length !== 1) {
    throw new Error(`Could not find exactly one application executable in ${name}`)
  }
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'qookix-mac-check-'))
  const binary = path.join(scratch, 'launcher')
  try {
    const output = fs.openSync(binary, 'wx')
    try {
      execFileSync('tar', ['-xOf', filename, '--', program], { stdio: ['ignore', output, 'inherit'] })
    } finally {
      fs.closeSync(output)
    }
    const actual = macExecutableArchitectures(binary)
    const expected = name.endsWith('_universal.app.tar.gz') ? ['x86_64', 'arm64']
      : name.endsWith('_aarch64.app.tar.gz') ? ['arm64'] : ['x86_64']
    if (!expected.every((arch) => actual.includes(arch))) {
      throw new Error(`${name}: expected ${expected.join('/')} but found ${actual.join('/')}`)
    }
  } finally {
    if (fs.existsSync(binary)) fs.unlinkSync(binary)
    fs.rmdirSync(scratch)
  }
}

console.log(`verify-artifacts: ${platform} OK`)
console.log(`  installers: ${installers.map(({ name }) => name).join(', ')}`)
console.log(`  updater   : ${bundles.map(({ asset }) => asset.name).join(', ')}`)
