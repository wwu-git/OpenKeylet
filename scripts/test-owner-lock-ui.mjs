import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import http from 'node:http';
import { once } from 'node:events';
import { chromium } from 'playwright';

const renderer = path.resolve(import.meta.dirname, '../renderer');
const server = http.createServer(async (request, response) => {
  const file = path.resolve(renderer, '.' + (request.url === '/' ? '/index.html' : request.url));
  if (!file.startsWith(renderer + path.sep)) { response.writeHead(403).end(); return; }
  try {
    const types = { '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' };
    response.setHeader('Content-Type', types[path.extname(file)] || 'text/html');
    response.end(await fs.readFile(file));
  } catch {
    response.writeHead(404).end();
  }
});
server.listen(0, '127.0.0.1');
await once(server, 'listening');
let browser;
try {
  browser = await chromium.launch({ headless: true, channel: process.env.OPENKEYLET_TEST_CHROME || undefined });
  const page = await browser.newPage({ viewport: { width: 720, height: 900 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const item = { id: '11111111-1111-4111-8111-111111111111', name: 'Synthetic private item', itemType: 'login', folder: 'test', enabled: true, updatedAt: 'fixture', modules: [{ id: '22222222-2222-4222-8222-222222222222', kind: 'password', configured: true, secret: true }], capabilities: [], passkeys: [], secret: { fields: [] } };
    const state = { settings: { language: 'en', closeBehavior: 'tray', browserEnabled: false, browserPort: 39272 }, connections: [item], activities: [], mcp: { status: 'ready', endpoint: 'fixture', stdioCommand: 'fixture' }, browserBridge: { status: 'offline' }, security: { encrypted: true, storage: 'fixture' } };
    const listeners = new Map();
    const fixture = window.fixture = { unlocked: true, touches: 0, reads: 0, holdSnapshot: false };
    function status() {
      return { pinConfigured: true, unlocked: fixture.unlocked, expiresInSeconds: fixture.unlocked ? 300 : 0 };
    }
    fixture.expire = () => {
      fixture.unlocked = false;
      listeners.get('owner-locked')?.();
    };
    window.__TAURI__ = {
      event: { listen: async (event, listener) => { listeners.set(event, listener); return () => listeners.delete(event); } },
      core: { invoke: async (command, args) => {
        if (command === 'get_state') {
          fixture.reads++;
          const snapshot = structuredClone(fixture.unlocked ? state : { ...state, connections: [] });
          if (fixture.holdSnapshot) {
            fixture.holdSnapshot = false;
            await new Promise(resolve => { fixture.releaseSnapshot = resolve; });
          }
          return snapshot;
        }
        if (command === 'owner_status') return status();
        if (command === 'owner_touch') { fixture.touches++; return status(); }
        if (command === 'owner_lock') { fixture.expire(); return status(); }
        if (command === 'owner_unlock') {
          if (args.pin !== '123456') throw Error('Too many PIN attempts; retry in 30 seconds');
          fixture.unlocked = true;
          return status();
        }
        if (command === 'owner_secret_view') {
          if (!fixture.releaseSecret) await new Promise(resolve => { fixture.releaseSecret = resolve; });
          return { fields: [{ name: 'password', value: 'synthetic-late-password' }] };
        }
        if (['owner_editor_drafts', 'agent_mcp_status', 'ssh_host_trust'].includes(command)) return [];
        if (command === 'github_backup_status') return { connected: false };
        if (command === 'system_integration_status') return { desktopShortcut: false, launchAtLogin: false };
        if (command === 'certificate_capabilities') return { enabled: false, grants: [], authorities: [] };
        return {};
      } },
    };
  });
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  const item = page.locator('#connections-list').getByText('Synthetic private item', { exact: true });
  await item.waitFor();
  await page.waitForFunction(() => fixture.reads > 1);
  await page.evaluate(() => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'A' })));
  assert.equal(await page.evaluate(() => fixture.touches), 0);
  await page.keyboard.press('Shift');
  await page.waitForFunction(() => fixture.touches === 1);
  await page.keyboard.press('Shift');
  assert.equal(await page.evaluate(() => fixture.touches), 1);

  await page.evaluate(() => { fixture.holdSnapshot = true; });
  await page.waitForFunction(() => fixture.releaseSnapshot);
  await item.click();
  await page.waitForFunction(() => fixture.releaseSecret);
  await page.evaluate(() => {
    fixture.expire();
    fixture.releaseSnapshot();
    fixture.releaseSecret();
  });
  await page.locator('#owner-lock-layer').waitFor();
  await item.waitFor({ state: 'detached' });
  assert.equal(await page.getByText('Synthetic private item', { exact: true }).count(), 0);
  assert.equal(await page.locator('#connection-modal').isHidden(), true);
  assert.equal(await page.locator('[data-secret-value]').count(), 0);
  assert.equal(await page.locator('.app-shell').evaluate(node => node.inert), true);

  async function enterPin(pin) {
    for (const [index, digit] of [...pin].entries()) await page.locator('[data-pin-group="owner-pin"] input').nth(index).fill(digit);
    await page.locator('#owner-unlock-action').click();
  }
  await enterPin('111111');
  await page.getByText('Too many PIN attempts. Try again in 30 seconds.', { exact: true }).waitFor();
  await enterPin('123456');
  await page.locator('#owner-lock-layer').waitFor({ state: 'hidden' });
  await item.waitFor();
  assert.equal(await page.locator('.app-shell').evaluate(node => node.inert), false);
  await item.click();
  const password = page.locator('[data-secret-value]');
  await password.waitFor();
  assert.equal(await password.inputValue(), 'synthetic-late-password');
  await page.locator('[data-toggle-module-secret]').click();
  assert.equal(await password.getAttribute('type'), 'text');
  await page.locator('#owner-lock-button').click();
  await page.locator('#owner-lock-layer').waitFor();
  assert.equal(await password.count(), 0);
  assert.equal(await page.locator('#connection-name').inputValue(), '');
  assert.deepEqual(errors, []);
  console.log('Owner lock renderer passed: trusted activity, stale replies, cooldown, and reload after unlock.');
} finally {
  await browser?.close();
  server.close();
}
