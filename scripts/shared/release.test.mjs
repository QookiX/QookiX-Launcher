import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { parseChangelog } from './changelog.mjs'
import { compareReleaseVersions } from './release-version.mjs'
import { buildUpdateManifest } from './update-manifest.mjs'
import { selectUpdaterBundles } from './updater-targets.mjs'
import { requireUpdaterCoverage } from './updater-targets.mjs'
import { synchronizeReleaseVersion, verifyReleaseVersion } from '../update-version.mjs'
import { finalizeRelease } from '../fix-update-manifest.mjs'
import { fileDigest, githubReleases } from './github-releases.mjs'
import { pushBucket } from '../push-bucket.mjs'
import { fileURLToPath } from 'node:url'

test('uses the same ordering examples as the Rust updater', () => {
  const examples = JSON.parse(fs.readFileSync(new URL('./release-version.test.json', import.meta.url), 'utf8'))
  for (const [left, right, expected] of examples) {
    assert.equal(compareReleaseVersions(left, right), expected, `${left} vs ${right}`)
  }
})

test('falls back to a signed format and rejects ambiguous signed bundles', async () => {
  const artifacts = ['game_x64-setup.exe', 'game.nsis.zip'].map((name) => ({ name }))
  const signatureFor = ({ name }) => name.endsWith('.zip') ? ' signature ' : undefined
  const bundles = await selectUpdaterBundles(artifacts, { signatureFor })
  assert.equal(bundles.length, 1)
  assert.equal(bundles[0].asset.name, 'game.nsis.zip')
  assert.equal(bundles[0].signature, 'signature')

  await assert.rejects(
    selectUpdaterBundles([...artifacts, { name: 'second_x64-setup.exe' }], { signatureFor: () => 'signed' }),
    /Ambiguous updater bundles/,
  )
  const manifest = buildUpdateManifest({
    tag: 'v0.22.0', minVersion: '0.21.0', bundles, urlFor: (item) => `https://example.com/${item.name}`,
  })
  assert.equal(manifest.minVersion, '0.21.0')
  assert.deepEqual(manifest.platforms['windows-x86_64'], {
    signature: 'signature', url: 'https://example.com/game.nsis.zip',
  })
})

test('keeps unreleased notes out of the structured changelog', () => {
  const notes = [
    '## [v0.22.0] - 2026-11-19',
    '1. **Fixes**: Ready',
    '## [Unreleased]',
    '1. **Changes**: Draft',
    '## [0.21.0] - 2026-10-19',
    '1. **Changes**：Published',
  ].join('\r\n')
  assert.deepEqual(parseChangelog(notes, 1), [{
    version: '0.22.0', date: '2026-11-19', entries: [{ category: 'Fixes', text: 'Ready' }],
  }])
})

test('release versions validate every file before synchronizing both npm lock versions', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'qookix-version-test-'))
  try {
    fs.mkdirSync(path.join(root, 'src-tauri'))
    const write = (name, content) => fs.writeFileSync(path.join(root, name), content)
    const packageJson = '{"name":"qookix-launcher","version":"1.0.0"}\n'
    const npmLock = '{"version":"1.0.0","packages":{"":{"version":"1.0.0"}}}\n'
    write('package.json', packageJson)
    write('package-lock.json', npmLock)
    write('src-tauri/tauri.conf.json', '{"version":"1.0.0"}\n')
    write('src-tauri/Cargo.toml', '[package]\nname = "qookix-launcher"\nversion = "1.0.0"\n')
    write('src-tauri/Cargo.lock', '[[package]]\nname = "other"\nversion = "1.0.0"\n')
    assert.throws(() => synchronizeReleaseVersion(root, '2.0.0junk'), /invalid SemVer/)
    assert.throws(() => synchronizeReleaseVersion(root, '2.0.0'), /exactly one/)
    assert.equal(fs.readFileSync(path.join(root, 'package.json'), 'utf8'), packageJson)
    assert.equal(fs.readFileSync(path.join(root, 'package-lock.json'), 'utf8'), npmLock)

    write('src-tauri/Cargo.lock', '[[package]]\nname = "qookix-launcher"\nversion = "1.0.0"\n')
    assert.equal(synchronizeReleaseVersion(root, 'v2.0.0-rc.1'), '2.0.0-rc.1')
    assert.equal(verifyReleaseVersion(root, 'v2.0.0-rc.1'), '2.0.0-rc.1')
    assert.throws(() => verifyReleaseVersion(root, 'v2.0.0'), /Mismatches/)
  } finally {
    fs.rmSync(root, { force: true, recursive: true })
  }
})

const bundleNames = [
  'QookiX Launcher_1.2.0_x64-setup.exe',
  'QookiX Launcher_universal.app.tar.gz',
  'QookiX Launcher_1.2.0_amd64.AppImage',
]

async function withReleaseAssets(action) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'qookix-release-test-'))
  try {
    const assets = []
    for (const name of bundleNames.flatMap((name) => [name, `${name}.sig`])) {
      const file = path.join(dir, name)
      const remote = name.replaceAll(' ', '.')
      const text = name.endsWith('.sig') ? `fixture-signature-${assets.length}` : `fixture-${name}`
      fs.writeFileSync(file, text)
      assets.push({
        name: remote, id: assets.length + 1, state: 'uploaded', text,
        size: Buffer.byteLength(text), digest: await fileDigest(file),
        browser_download_url: `https://github.com/demo/launcher/releases/download/v1.2.0/${remote}`,
      })
    }
    await action(dir, assets)
  } finally {
    fs.rmSync(dir, { recursive: true, force: true })
  }
}

test('an unlabeled macOS archive cannot represent either architecture', async () => {
  const assets = [{ name: 'QookiX.Launcher.app.tar.gz' }, { name: 'QookiX.Launcher_aarch64.app.tar.gz' }]
  const selected = await selectUpdaterBundles(assets, { signatureFor: () => 'fixture' })
  assert.deepEqual(selected.flatMap(({ platforms }) => platforms), ['darwin-aarch64'])
  assert.throws(() => requireUpdaterCoverage(selected, ['darwin-aarch64', 'darwin-x86_64']), /darwin-x86_64/)
})

test('artifact verification reads both executable slices inside a macOS updater archive', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'qookix-mac-fixture-'))
  const cpu = { x64: 0x01000007, arm64: 0x0100000c }
  function thin(type, subtype) {
    const buffer = Buffer.alloc(40)
    const fields = [0xfeedfacf, type, subtype, 2, 1, 8]
    fields.forEach((value, index) => buffer.writeUInt32LE(value, index * 4))
    return buffer
  }
  function universal(secondCpu, secondOffset = 88) {
    const buffer = Buffer.alloc(128)
    const fields = [0xcafebabe, 2, cpu.x64, 3, 48, 40, 0, secondCpu, 0, secondOffset, 40, 0]
    fields.forEach((value, index) => buffer.writeUInt32BE(value, index * 4))
    thin(cpu.x64, 3).copy(buffer, 48)
    thin(cpu.arm64, 0).copy(buffer, 88)
    return buffer
  }

  try {
    const artifacts = path.join(root, 'artifacts')
    const contents = path.join(root, 'Fixture.app', 'Contents')
    fs.mkdirSync(artifacts)
    fs.mkdirSync(path.join(contents, 'MacOS'), { recursive: true })
    fs.writeFileSync(path.join(artifacts, 'Fixture_1.2.0_universal.dmg'), '')
    fs.writeFileSync(path.join(contents, 'Info.plist'),
      '<plist><dict><key>CFBundleExecutable</key><string>Fixture</string>' +
      '<key>CFBundleShortVersionString</key><string>1.2.0</string></dict></plist>')
    const verifier = fileURLToPath(new URL('../verify-artifacts.mjs', import.meta.url))
    const check = (bytes) => {
      fs.writeFileSync(path.join(contents, 'MacOS', 'Fixture'), bytes)
      execFileSync('tar', ['-czf', path.join(artifacts, 'Fixture_universal.app.tar.gz'), '-C', root, 'Fixture.app'])
      return spawnSync(process.execPath, [verifier, artifacts, 'macos', '1.2.0'], {
        encoding: 'utf8', env: { ...process.env, TAURI_SIGNING_PRIVATE_KEY: '' },
      })
    }
    const valid = check(universal(cpu.arm64))
    assert.equal(valid.status, 0, valid.stderr)
    const thinLabeledUniversal = check(thin(cpu.arm64, 0))
    assert.equal(thinLabeledUniversal.status, 1)
    assert.match(thinLabeledUniversal.stderr, /expected x86_64\/arm64 but found arm64/)
    const wrongArchitecture = check(universal(cpu.x64))
    assert.equal(wrongArchitecture.status, 1)
    assert.match(wrongArchitecture.stderr, /architecture does not match/)
    const invalidBounds = check(universal(cpu.arm64, 100))
    assert.equal(invalidBounds.status, 1)
    assert.match(invalidBounds.stderr, /Invalid Mach-O slice bounds/)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

test('draft publication verifies the actual signed uploads before exposing the manifest', async () => {
  await withReleaseAssets(async (directory, assets) => {
    const tag = 'v1.2.0'
    const draftTag = 'untagged-9eb48aafd1fa7a0dccf4'
    for (const asset of assets) {
      asset.browser_download_url = asset.browser_download_url.replace(`/download/${tag}/`, `/download/${draftTag}/`)
    }
    const release = { id: 4, tag_name: draftTag, draft: true, assets }
    const api = githubReleases('demo/launcher', 'fixture', (url) => {
      assert.equal(url, 'https://api.github.com/repos/demo/launcher/releases/4')
      return Response.json(release)
    })
    const draft = await api.getDraft(tag, release.id)
    const calls = []
    let rejectManifest = true
    const client = {
      async digest(asset) { return asset.digest },
      async text(asset) { return asset.text },
      async upload(_id, name, bytes) {
        calls.push(name)
        if (name === 'latest.json' && rejectManifest) throw new Error('upload failed')
        if (name === 'latest.json') {
          const manifest = JSON.parse(bytes)
          assert.equal(manifest.minVersion, '1.0.0')
          assert.match(manifest.platforms['windows-x86_64'].url, /\/download\/v1\.2\.0\/QookiX\.Launcher/)
          assert.deepEqual(manifest.platforms['darwin-aarch64'], manifest.platforms['darwin-x86_64'])
        }
      },
      async edit(id, patch) {
        calls.push('publish')
        assert.deepEqual(patch, { tag_name: tag, draft: false, make_latest: 'true' })
        return { id, tag_name: tag, draft: false }
      },
    }
    const options = { tag, directory, minVersion: '1.0.0', publish: true, draftId: 4 }
    await assert.rejects(finalizeRelease(client, draft, { ...options, draftId: 5 }), /release is missing/)
    await assert.rejects(finalizeRelease(client, draft, { ...options, draftId: undefined }), /release is missing/)
    await assert.rejects(finalizeRelease(client, draft, options), /upload failed/)
    assert.deepEqual(calls, ['SHA256SUMS.txt', 'latest.json'])
    calls.length = 0
    rejectManifest = false
    await finalizeRelease(client, draft, options)
    assert.deepEqual(calls, ['SHA256SUMS.txt', 'latest.json', 'publish'])
    calls.length = 0
    await assert.rejects(finalizeRelease(client, { ...release, tag_name: tag, draft: false }, {
      ...options, draftId: undefined,
    }), /public/)
    assert.equal(calls.length, 0)
  })
})

test('bucket publication scopes all writes and updates the pointer after its assets', async () => {
  await withReleaseAssets(async (directory) => {
    fs.writeFileSync(path.join(directory, 'unrelated.txt'), 'shared content')
    const calls = []
    const aws = (args) => {
      calls.push(args)
      if (args[1] === 'get-object') return { status: 254, stderr: '(NoSuchKey)' }
      if (args[1] === 'put-object') {
        assert.deepEqual(args.slice(-2), ['--if-none-match', '*'])
        const manifest = JSON.parse(fs.readFileSync(args[args.indexOf('--body') + 1], 'utf8'))
        assert.equal(manifest.version, '1.2.0')
        assert.ok(Object.values(manifest.platforms).every(({ url }) => url.includes('/managed/releases/1.2.0/')))
      }
      return { status: 0, stdout: '' }
    }
    const published = await pushBucket({
      directory, bucket: 'test-bucket', prefix: 'managed',
      publicBase: 'https://updates.example/managed', tag: 'v1.2.0',
    }, aws)
    assert.equal(published.active, true)
    const uploads = calls.filter((args) => args[0] === 's3')
    assert.ok(uploads.every((args) => args[1] === 'cp' && args[3].startsWith(`s3://test-bucket/${published.releasePrefix}/`)))
    assert.ok(uploads.every((args) => !args[2].endsWith('unrelated.txt')))
    assert.equal(calls.at(-1)[1], 'put-object')
    assert.equal(calls.at(-1)[5], 'managed/latest.json')
  })
})
