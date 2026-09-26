import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import vm from 'node:vm';
import { transform } from 'esbuild';

const require = createRequire(import.meta.url);
const { code } = await transform(readFileSync(new URL('../src/updater.ts', import.meta.url), 'utf8'), { loader: 'ts', format: 'cjs' });
function setup(check, invoke = async () => {}) {
  const exports = {};
  const storage = new Map();
  const context = {
    exports, module: { exports },
    require: (name) => name === '@tauri-apps/plugin-updater' ? { check }
      : name === '@tauri-apps/api/core' ? { invoke } : require(name),
    localStorage: { getItem: (key) => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) },
    window: { setTimeout: () => 1, clearTimeout: () => {} },
  };
  vm.runInNewContext(code, context);
  return context.module.exports;
}
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
const update = (downloadAndInstall = async () => {}) => ({ version: '0.2.3', close: async () => {}, downloadAndInstall });

test('manual no-update check always shows latest toast', async () => {
  const { useUpdater: store } = setup(async () => null);
  assert.equal(store.getState().checked, false);
  await store.getState().check(false);
  assert.equal(store.getState().toast, '当前已是最新版本');
  assert.equal(store.getState().checked, true);
  assert.equal(store.getState().phase, 'idle');
});

test('404/network failure never claims latest', async () => {
  for (const error of ['404 Not Found', 'network timeout', 'invalid signature metadata']) {
    const { useUpdater: store } = setup(async () => { throw new Error(error); });
    await store.getState().check(false);
    assert.equal(store.getState().checked, false);
    assert.match(store.getState().toast, /检查更新失败/);
    assert.equal(store.getState().phase, 'idle');
  }
});

test('manual new-version check waits for install click', async () => {
  let downloads = 0;
  const { useUpdater: store } = setup(async () => update(async () => { downloads++; }));
  await store.getState().check(false);
  assert.equal(store.getState().hasNew, true);
  assert.match(store.getState().toast, /0.2.3/);
  assert.equal(downloads, 0);
});

test('auto install stays busy through download and blocks duplicate operations', async () => {
  const gate = deferred();
  let checks = 0, downloads = 0, restarts = 0;
  let store;
  ({ useUpdater: store } = setup(async () => { checks++; return update(async (event) => {
    downloads++;
    event({ event: 'Started', data: { contentLength: 100 } });
    event({ event: 'Progress', data: { chunkLength: 40 } });
    await gate.promise;
    event({ event: 'Finished' });
    assert.equal(store.getState().phase, 'installing');
  }); }, async (command) => { assert.equal(command, 'restart_app'); restarts++; }));
  const checking = store.getState().check(true);
  await new Promise(r => setImmediate(r));
  assert.equal(store.getState().phase, 'downloading');
  assert.equal(store.getState().progress, 0.4);
  await store.getState().check(false);
  await store.getState().install();
  assert.equal(checks, 1);
  assert.equal(downloads, 1);
  gate.resolve();
  await checking;
  assert.equal(restarts, 1);
  assert.equal(store.getState().phase, 'restarting');
});

test('signature/download failure is visible, never restarts or opens browser', async () => {
  let invokes = 0;
  const { useUpdater: store } = setup(async () => update(async () => { throw new Error('signature mismatch'); }), async () => { invokes++; });
  await store.getState().check(true);
  assert.equal(store.getState().phase, 'idle');
  assert.match(store.getState().toast, /signature mismatch/);
  assert.equal(invokes, 0);
});

test('restart retry does not download/install again', async () => {
  let downloads = 0, restarts = 0;
  const { useUpdater: store } = setup(async () => update(async () => { downloads++; }), async () => { if (++restarts === 1) throw new Error('restart failed'); });
  await store.getState().check(true);
  assert.equal(store.getState().installed, true);
  assert.equal(store.getState().phase, 'idle');
  await store.getState().install();
  assert.equal(downloads, 1);
  assert.equal(restarts, 2);
});

test('deferred startup check runs once when version arrives', async () => {
  let checks = 0;
  const { useUpdater: store, startAutoUpdate } = setup(async () => { checks++; return null; });
  startAutoUpdate();
  store.getState().setAppVersion('0.2.2');
  startAutoUpdate();
  await new Promise(r => setImmediate(r));
  assert.equal(checks, 1);
  assert.equal(store.getState().toast, null);
});

test('turning auto update off before version arrives prevents deferred check', async () => {
  let checks = 0;
  const { useUpdater: store, startAutoUpdate } = setup(async () => { checks++; return null; });
  startAutoUpdate();
  store.getState().setAutoCheck(false);
  store.getState().setAppVersion('0.2.2');
  await new Promise(r => setImmediate(r));
  assert.equal(checks, 0);
});

test('turning auto update off during a check prevents automatic installation', async () => {
  const gate = deferred();
  let downloads = 0;
  const { useUpdater: store } = setup(() => gate.promise);
  const running = store.getState().check(true);
  store.getState().setAutoCheck(false);
  gate.resolve(update(async () => { downloads++; }));
  await running;
  assert.equal(downloads, 0);
  assert.equal(store.getState().hasNew, true);
});
