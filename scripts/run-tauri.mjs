#!/usr/bin/env node
// Wrapper around the tauri CLI.
//
// On Windows it:
//   1. builds the `installer-ui` crate (the WebView2-based modern installer
//      frontend) when needed,
//   2. sets the `QOOKIX_INSTALLER_UI` environment variable to the built
//      `QookiXInstallerUI.exe` so the NSIS bundler embeds it into the
//      installer (see `src-tauri/nsis/installer.nsi`),
//   3. forwards every argument to the real `tauri` CLI.
//
// When the variable is unset the NSIS template falls back to the classic
// (dark-styled) wizard, so building still works.
import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const projectRoot = path.resolve(__dirname, '..')
const crateDir = path.join(projectRoot, 'src-tauri', 'installer-ui')

// Resolve cargo's real output directory. `CARGO_TARGET_DIR` (set on CI so both
// crates share ONE target dir and tao/wry/windows-sys are compiled only once)
// takes precedence over the default `<crate>/target`.
function cargoTargetDir() {
  const fromEnv = process.env.CARGO_TARGET_DIR
  return fromEnv ? path.resolve(projectRoot, fromEnv) : path.join(crateDir, 'target')
}

const uiExe = path.join(cargoTargetDir(), 'release', 'QookiXInstallerUI.exe')
const args = process.argv.slice(2)
const isBuild = args.includes('build')
const signScript = path.join(projectRoot, 'scripts', 'sign-windows.ps1')

// Code signing is enabled only when a certificate is configured. Without it
// every check below is skipped, so the normal dev/build flow is unaffected.
function shouldSign() {
  if (process.platform !== 'win32') return false
  const pfx = process.env.QOOKIX_CERT_PFX
  // CI used to set QOOKIX_CERT_PFX to a path unconditionally, so a missing
  // certificate looked like "signing is enabled" while sign-windows.ps1
  // silently skipped it. Check the file itself.
  return Boolean(pfx) && existsSync(pfx)
}

// Sign a list of files via scripts/sign-windows.ps1 (no-op unless a cert is set).
// File paths are passed over stdin (one per line) rather than as command-line
// arguments: Windows PowerShell 5.1's `-File` mode mangles quoted args that
// contain spaces (e.g. "QookiX Launcher_0.2.0_x64-setup.exe").
function signFiles(files) {
  if (!shouldSign()) {
    const pfx = process.env.QOOKIX_CERT_PFX
    if (process.platform === 'win32' && pfx && !existsSync(pfx)) {
      console.warn(`[run-tauri] WARNING: certificate ${pfx} is missing - build artifacts will be UNSIGNED`)
    }
    return
  }
  const real = files.filter((f) => f && existsSync(f))
  if (real.length === 0) return
  const r = spawnSync(
    'powershell',
    ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', signScript],
    { stdio: ['pipe', 'inherit', 'inherit'], input: real.join('\n') + '\n', env: process.env },
  )
  if (r.status !== 0) process.exit(r.status ?? 1)
}

if (process.platform === 'win32' && (isBuild || !existsSync(uiExe))) {
  const build = spawnSync(
    'cargo',
    ['build', '--release', '--manifest-path', path.join(crateDir, 'Cargo.toml')],
    { stdio: 'inherit' },
  )
  if (build.status !== 0) process.exit(build.status ?? 1)
  // Sign the installer UI BEFORE it is embedded into the NSIS installer, so the
  // copy extracted to $PLUGINSDIR at install time is already signed (unsigned
  // extracted-and-executed exes are a common antivirus false-positive trigger).
  signFiles([uiExe])
}

const env = { ...process.env }
if (process.platform === 'win32' && existsSync(uiExe)) {
  env.QOOKIX_INSTALLER_UI = uiExe
} else {
  delete env.QOOKIX_INSTALLER_UI
}

// 开发期凭证（client_id / secret 等）放在 gitignore 的 .env.local 里，避免提交到仓库。
// 这里在调用 tauri CLI 前把它们注入子进程环境，Rust 侧的 option_env! 即可在编译期读到。
// 仅覆盖未设置的变量：留空的占位行不会清掉已在 shell 里设好的同名变量。
const envLocal = path.join(projectRoot, '.env.local')
if (existsSync(envLocal)) {
  for (const line of readFileSync(envLocal, 'utf8').split(/\r?\n/)) {
    const trimmed = line.trim()
    if (!trimmed || trimmed.startsWith('#')) continue
    const eq = trimmed.indexOf('=')
    if (eq < 0) continue
    const key = trimmed.slice(0, eq).trim()
    const val = trimmed.slice(eq + 1).trim().replace(/^["']|["']$/g, '')
    if (key && val && env[key] === undefined) env[key] = val
  }
}

// The bundler signs Windows binaries before it packages and signs updater artifacts.
if (isBuild && shouldSign()) {
  const signCommand = {
    cmd: 'powershell',
    args: ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', signScript, '-Files', '%1'],
  }
  args.push('--config', JSON.stringify({ bundle: { windows: { signCommand } } }))
}

if (isBuild && !env.TAURI_SIGNING_PRIVATE_KEY && !args.includes('--no-sign')) {
  args.push('--no-sign')
  console.warn(
    '[run-tauri] WARNING: TAURI_SIGNING_PRIVATE_KEY is not set - skipping updater signature (.sig).\n' +
      '            Installers are still built; only builds with the trusted key can publish updater releases.',
  )
}

// Invoke the installed CLI directly so the structured Windows config and paths
// with spaces are passed as single arguments without a shell quoting round trip.
const tauriCli = path.join(projectRoot, 'node_modules', '@tauri-apps', 'cli', 'tauri.js')
const result = spawnSync(process.execPath, [tauriCli, ...args], { stdio: 'inherit', env })

process.exit(result.status ?? 1)
