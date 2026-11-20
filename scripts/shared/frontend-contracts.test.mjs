import assert from 'node:assert/strict'
import fs from 'node:fs'
import test from 'node:test'
import ts from 'typescript'

const read = (path) => fs.readFileSync(new URL(path, import.meta.url), 'utf8')

function loadTypeScript(path, imports = {}, globals = {}, text = read(path)) {
  const source = text.replaceAll('import.meta.url', JSON.stringify(new URL(path, import.meta.url).href))
  const code = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, esModuleInterop: true },
  }).outputText
  const exports = {}
  const require = (specifier) => {
    assert.ok(Object.hasOwn(imports, specifier), `Unexpected import ${specifier}`)
    return imports[specifier]
  }
  new Function('require', 'exports', ...Object.keys(globals), code)(require, exports, ...Object.values(globals))
  return exports
}

function setupScript(path) {
  const source = read(path).match(/<script setup[^>]*>([\s\S]*?)<\/script>/)?.[1]
  assert.ok(source, `No setup script in ${path}`)
  return source
}

const ref = (value) => ({ value })
const computed = (get) => ({ get value() { return get() } })
const translations = { useI18n: () => ({ t: (key) => key }) }

function readLocale(path) {
  const source = ts.createSourceFile(path, read(path), ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  const declaration = source.statements.find(ts.isExportAssignment)
  assert.ok(declaration && ts.isObjectLiteralExpression(declaration.expression))
  return new Set(declaration.expression.properties.map((property) => property.name?.getText(source).replace(/^['"]|['"]$/g, '')))
}

test('keybind view static keys exist in both shipping locales', () => {
  const view = read('../../src/components/instance/KeybindTab.vue')
  const names = new Set(Array.from(view.matchAll(/\bt\(\s*['"]keybind\.([\w-]+)['"]/g), (match) => match[1]))
  assert.ok(names.has('none'))
  for (const language of ['en', 'zh']) {
    const keys = readLocale(`../../src/i18n/locales/_fragments/keybind.${language}.ts`)
    assert.deepEqual([...names].filter((name) => !keys.has(name)), [], language)
  }
})

test('only the latest independently scoped request may publish or settle', async () => {
  const source = read('../../src/composables/latestRequest.ts')
  const code = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 }
  }).outputText
  const { createLatestRequest } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)
  const list = createLatestRequest()
  const detail = createLatestRequest()
  const firstPage = list.start()
  const openDetail = detail.start()
  const filteredPage = list.start()
  assert.equal(firstPage(), false)
  assert.equal(filteredPage(), true)
  assert.equal(openDetail(), true)
  detail.invalidate()
  assert.equal(openDetail(), false)
  assert.equal(filteredPage(), true)
})

test('updating uses the confirmed decision across a second check, progress, and install', async () => {
  const first = { available: true, decisionId: 1, version: '1.1.0', source: 'bucket' }
  const second = { available: true, decisionId: 2, version: '1.2.0', source: 'github' }
  let newest = first
  let handleProgress
  let didRelaunch = false
  let releaseDownload
  const downloaded = new Promise((resolve) => { releaseDownload = resolve })
  const calls = []
  const samples = []
  const tasks = new Map()
  const mockedStore = {
    upsert(id, write) {
      const task = tasks.get(id) ?? { bytesTotal: 0, bytesDone: 0 }
      write(task)
      tasks.set(id, task)
    },
    sampleSpeed(_task, progress) { samples.push(progress) },
  }
  const updater = loadTypeScript('../../src/updater.ts', {
    vue: { ref: (value) => ({ value }) },
    '@tauri-apps/api/core': {
      async invoke(cmd, args) {
        calls.push([cmd, args])
        if (cmd === 'check_for_update') return newest
        if (cmd === 'download_update') return downloaded
        if (cmd === 'apply_app_update') return
        throw new Error(cmd)
      },
    },
    '@tauri-apps/api/event': { async listen(_event, handler) {
      handleProgress = handler
      return () => { handleProgress = null }
    } },
    '@tauri-apps/plugin-process': { async relaunch() { didRelaunch = true } },
    './stores/tasks': { useTasksStore: () => mockedStore },
    './utils/logger': { error() {} },
    '../CHANGELOG.md?releases': [],
    '../scripts/shared/release-version.mjs': {},
  })
  const offer = await updater.peekUpdate()
  const statuses = []
  const work = updater.downloadUpdate(offer, (status) => statuses.push(status))
  newest = second
  assert.equal((await updater.peekUpdate()).version, second.version)
  handleProgress({ payload: { decisionId: 2, downloaded: 99, total: 100 } })
  handleProgress({ payload: { decisionId: 1, downloaded: 25, total: 100 } })
  assert.deepEqual(statuses.map((status) => status.fraction), [0.25])
  releaseDownload({ version: first.version, decisionId: first.decisionId })
  assert.equal(await work, true)
  assert.equal(updater.updateReadyVersion.value, first.version)
  assert.equal(updater.isUpdateDownloaded(second), false)
  assert.equal(handleProgress, null)
  await updater.applyUpdateNow()
  assert.equal(didRelaunch, true)
  assert.deepEqual(calls.filter(([cmd]) => cmd !== 'check_for_update'), [
    ['download_update', { decisionId: 1 }],
    ['apply_app_update', { decisionId: 1 }],
  ])
  assert.equal(calls.filter(([cmd]) => cmd === 'check_for_update').length, 2)
})

test('primary instance and screenshot actions use native keyboard interaction', () => {
  const cards = read('../../src/components/InstanceCard.vue')
  const photos = read('../../src/views/TimeMachineView.vue')
  assert.match(cards, /<RouterLink class="name [^"]+" :to=/)
  assert.doesNotMatch(cards, /<div class="inst-card[^\n]+@click/)
  assert.match(cards, /\.name:focus-visible::after/)
  const buttons = photos.match(/<button type="button" class="shot-preview"[^>]+@click="openShot/g)
  assert.equal(buttons?.length, 2)
  assert.match(photos, /\.shot-preview:focus-visible/)
})

test('late Tauri registrations are released and stop dispatching after disposal', async () => {
  const { createTauriListenerScope } = loadTypeScript('../../src/composables/tauriListeners.ts', {
    vue: {},
    '@tauri-apps/api/event': {},
  })
  const scope = createTauriListenerScope()
  let publish
  let finishRegistration
  let detachments = 0
  const received = []
  const subscribing = scope.observe((next) => {
    publish = next
    return new Promise((resolve) => { finishRegistration = resolve })
  }, (event) => received.push(event))
  publish('before unmount')
  scope.dispose()
  publish('after unmount')
  finishRegistration(() => { detachments += 1 })
  const detach = await subscribing
  detach()
  assert.equal(detachments, 1)
  assert.deepEqual(received, ['before unmount'])
  await scope.observe(() => assert.fail('closed scope must not register'), () => {})
})

test('silent schematic workers reject initialization and queued meshes on timeout', async () => {
  const timers = new Map()
  let nextId = 0
  const workers = []
  class FakeWorker {
    constructor() { workers.push(this) }
    postMessage() {}
    terminate() { this.terminated = true }
  }
  const { createSchematicWorkerPool } = loadTypeScript('../../src/composables/useSchematicWorkerPool.ts', {}, {
    Worker: FakeWorker,
    setTimeout(callback) {
      const id = ++nextId
      timers.set(id, callback)
      return id
    },
    clearTimeout(id) { timers.delete(id) },
  })
  const firstPool = createSchematicWorkerPool(1)
  const initialization = assert.rejects(firstPool.init(), /initialization timed out/)
  timers.values().next().value()
  await initialization
  assert.equal(workers[0].terminated, true)
  const secondPool = createSchematicWorkerPool(1)
  const ready = secondPool.init()
  workers[1].onmessage({ data: { type: 'ready' } })
  await ready
  const running = assert.rejects(secondPool.buildMesh([0, 0, 0], new Uint32Array(1)), /mesh worker timed out/)
  const queued = assert.rejects(secondPool.buildMesh([1, 0, 0], new Uint32Array(1)), /mesh worker timed out/)
  timers.values().next().value()
  await Promise.all([running, queued])
  assert.equal(workers[1].terminated, true)
  assert.equal(timers.size, 0)
})

test('data directory confirmation schedules a move before exposing the new directory', async () => {
  const path = '../../src/views/settings/SettingsDataDirectory.vue'
  const store = { settings: { data_dir: 'old-directory' } }
  const calls = []
  const errors = []
  let fail = true
  const panel = loadTypeScript(path, {
    vue: { onMounted() {}, ref },
    'naive-ui': { useMessage: () => ({ error: (err) => errors.push(err) }) },
    '@tauri-apps/plugin-dialog': { async open() { return 'selected-directory' } },
    '@tauri-apps/plugin-process': { async relaunch() { calls.push('relaunch') } },
    '../../stores/settings': { useSettingsStore: () => store },
    'vue-i18n': translations,
    '../../api': { api: {
      async changeDataDir(dir, mode) {
        calls.push([dir, mode])
        if (fail) throw new Error('permission denied')
        return { scheduled: true, new_dir: dir }
      },
    } },
  }, {}, setupScript(path) + '\nexport { pickDataDir, confirmMigrate, relaunchNow, migratePhase, scheduled }')
  await panel.pickDataDir()
  await panel.confirmMigrate()
  assert.equal(panel.migratePhase.value, 'select')
  assert.match(errors[0], /permission denied/)
  fail = false
  await panel.confirmMigrate()
  assert.equal(panel.migratePhase.value, 'done')
  assert.equal(panel.scheduled.value, true)
  assert.equal(store.settings.data_dir, 'old-directory')
  await panel.relaunchNow()
  assert.deepEqual(calls, [
    ['selected-directory', 'move'], ['selected-directory', 'move'], 'relaunch',
  ])
})

test('two-step login keeps the challenge on failure and closes only after verification', async () => {
  const path = '../../src/components/QookixLoginDialog.vue'
  const tokens = []
  let reject = true
  const panel = loadTypeScript(path, {
    vue: { computed, ref },
    'vue-i18n': translations,
    'naive-ui': { useMessage: () => ({ success() {} }) },
    '@tauri-apps/plugin-opener': {},
    '../api': { api: {} },
    '../stores/qookix': {
      useQookixStore: () => ({
        async login() { return { challenge_token: 'fixture-challenge' } },
        async login2fa(token, code) {
          tokens.push([token, code])
          if (reject) throw new Error('invalid code')
        },
      }),
      qookixErrorMessage: (err) => err.message,
    },
  }, { defineExpose() {} }, setupScript(path) + '\nexport { open, account, password, mode, show, twofaCode, twofaChallenge, error, submitLogin, submitTwoFA }')
  panel.open()
  panel.account.value = 'fixture-user'
  panel.password.value = 'fixture-password'
  await panel.submitLogin()
  assert.equal(panel.mode.value, 'twofa')
  assert.equal(panel.show.value, true)
  panel.twofaCode.value = '123456'
  await panel.submitTwoFA()
  assert.equal(panel.error.value, 'invalid code')
  assert.equal(panel.show.value, true)
  reject = false
  await panel.submitTwoFA()
  assert.deepEqual(tokens, [['fixture-challenge', '123456'], ['fixture-challenge', '123456']])
  assert.equal(panel.show.value, false)
  assert.equal(panel.twofaChallenge.value, '')
  assert.equal(panel.password.value, '')
})

test('world edits validate before confirmation and cannot commit to a later world selection', async () => {
  const path = '../../src/views/NBTEditorView.vue'
  const setup = ts.createSourceFile(path, setupScript(path), ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  const save = setup.statements.find((statement) => ts.isFunctionDeclaration(statement) && statement.name?.text === 'save')
  assert.ok(save)
  const calls = []
  const current = ref({ dir: 'original-world' })
  let confirmed
  let invalid = true
  const patch = { levelName: 'Renamed' }
  const { save: saveWorld } = loadTypeScript(path, {}, {
    current,
    form: ref(patch),
    saving: ref(false),
    running: ref(false),
    editable: ref(true),
    instanceId: ref('instance'),
    savePlan: ref({ changes: [{ label: 'name', from: 'World', to: 'Renamed' }], patch }),
    api: {
      async nbtValidateWorldPatch(_instance, world, fields) {
        calls.push(['validate', world, fields])
        if (invalid) throw new Error('invalid')
      },
      async nbtCanEdit() { assert.fail('old confirmation must not check a different world') },
      async nbtSaveWorld() { assert.fail('old confirmation must not write') },
    },
    dialog: { warning(options) { confirmed = options.onPositiveClick } },
    message: { error() {} },
    t: (key) => key,
  }, `export ${save.getText(setup)}`)
  await saveWorld()
  assert.equal(confirmed, undefined)
  invalid = false
  await saveWorld()
  assert.equal(typeof confirmed, 'function')
  current.value = { dir: 'later-world' }
  await confirmed()
  assert.deepEqual(calls, [['validate', 'original-world', patch], ['validate', 'original-world', patch]])
})

test('installer launch failure exposes distinct retry and close commands', () => {
  const html = read('../../src-tauri/installer-ui/src/installer.html')
  const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1]
  assert.ok(script)
  const nodes = new Map()
  const newNode = () => ({
    style: {},
    classList: { add() {}, remove() {}, toggle() {} },
    setAttribute() {},
    addEventListener(event, callback) { this[event] = callback },
  })
  const document = {
    documentElement: {},
    createElement: newNode,
    querySelectorAll: () => [],
    querySelector(key) {
      if (!nodes.has(key)) nodes.set(key, newNode())
      return nodes.get(key)
    },
  }
  const calls = []
  const window = {
    __QOOKIX_INSTALLER__: { language: 'en', freshInstall: true },
    ipc: { postMessage: (message) => calls.push(JSON.parse(message)) },
  }
  new Function('document', 'window', script)(document, window)
  window.qookixInstaller.receive({ type: 'installFailed', exitCode: 3, message: 'Could not commit the requested data directory.' })
  assert.match(document.querySelector('#install-error').textContent, /Could not commit the requested data directory/)
  document.querySelector('#launch-after').checked = true
  window.qookixInstaller.receive({ type: 'launchFailed' })
  assert.equal(document.querySelector('#primary-label').textContent, 'Finish')
  assert.equal(document.querySelector('#secondary-action').textContent, 'Retry launch')
  document.querySelector('#secondary-action').click()
  window.qookixInstaller.receive({ type: 'launchFailed' })
  document.querySelector('#primary-action').click()
  assert.deepEqual(calls, [{ command: 'finish', launch: true }, { command: 'finish', launch: false }])
})
