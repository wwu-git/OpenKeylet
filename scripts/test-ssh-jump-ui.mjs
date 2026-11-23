import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import { once } from 'node:events';
import { chromium } from 'playwright';
const renderer = path.resolve(import.meta.dirname, '../renderer');
const artifacts = await fs.mkdtemp(path.join(os.tmpdir(), 'openkeylet-ssh-jump-ui-'));
const server = http.createServer(async (req, res) => {
  const pathname = decodeURIComponent(req.url.split('?')[0]);
  const file = path.resolve(renderer, '.' + (pathname === '/' ? '/index.html' : pathname));
  if (!file.startsWith(renderer + path.sep)) { res.writeHead(403); res.end(); return; }
  try { res.setHeader('Content-Type', file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : file.endsWith('.svg') ? 'image/svg+xml' : 'text/html'); res.end(await fs.readFile(file)); }
  catch { res.writeHead(404); res.end(); }
});
server.listen(0, '127.0.0.1'); await once(server, 'listening');
const browser = await chromium.launch({ headless: true, channel: process.env.OPENKEYLET_TEST_CHROME || undefined });
try {
  const page = await browser.newPage({ viewport: { width: 1120, height: 1000 } });
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const item = (id, name, extra = {}) => ({ id, name, itemType: 'ssh', folder: 'test', favorite: false, enabled: true, updatedAt: '2026-11-23T00:00:00Z', revision: 1, description: '', sshAuthType: 'password', host: id === 'b' ? 'private.test.invalid' : 'jump.test.invalid', port: 22, capabilities: ['ssh', 'fill'], canTest: true, passkeys: [], secret: { fields: [] }, modules: [ { id: `${id}-host`, kind: 'host', name: 'host', value: id === 'b' ? 'private.test.invalid' : 'jump.test.invalid', configured: true, agentVisible: true }, { id: `${id}-port`, kind: 'port', name: 'port', value: '22', configured: true, agentVisible: true }, { id: `${id}-user`, kind: 'username', name: 'username', value: '', configured: true, secret: true }, { id: `${id}-password`, kind: 'password', name: 'password', value: '', configured: true, secret: true } ], ...extra });
    const state = { settings: { language: 'zh', closeBehavior: 'tray', browserEnabled: false }, connections: [item('a', '公网跳板'), item('b', '内网目标', { sshJumpItemId: 'a' }), item('disabled', '已禁用', { enabled: false }), item('nested', '已有跳板', { sshJumpItemId: 'a' }), item('incomplete', '未配置目标', { host: '' })], activities: [], mcp: { status: 'ready', endpoint: 'fixture', stdioCommand: 'fixture' }, browserBridge: { status: 'disabled', enabled: false }, security: { encrypted: true, storage: 'fixture' } };
    if (sessionStorage.getItem('brokenJump')) { const b = state.connections.find(item => item.id === 'b'); b.sshJumpItemId = 'missing'; b.sshJumpUnresolved = true; }
    window.fixture = { state, saved: [], drafts: [], trust: [], trustWrites: [], probes: [] };
    window.__TAURI__ = { event: { listen: async () => () => {} }, core: { invoke: async (command, args = {}) => {
      if (command === 'get_state') return structuredClone(state);
      if (command === 'owner_status') return { pinConfigured: false, unlocked: true };
      if (command === 'owner_secret_view') return { fields: [{ name: 'username', value: 'synthetic-user' }, { name: 'password', value: 'synthetic-password' }] };
      if (command === 'owner_editor_drafts') return structuredClone(fixture.drafts);
      if (command === 'save_editor_draft') { const entry = { id: args.draftId || 'draft', input: structuredClone(args.input) }; fixture.drafts = [entry]; return entry; }
      if (command === 'delete_editor_draft') { fixture.drafts = []; return; }
      if (command === 'agent_mcp_status') return [];
      if (command === 'github_backup_status') return { connected: false };
      if (command === 'system_integration_status') return { desktopShortcut: false, launchAtLogin: false };
      if (command === 'save_connection') {
        fixture.saved.push(structuredClone(args.input));
        const index = state.connections.findIndex(item => item.id === args.input.id);
        const saved = { ...state.connections[index], ...args.input, revision: state.connections[index].revision + 1, updatedAt: new Date().toISOString(), modules: args.input.modules.map(module => ({ ...module, configured: true })) };
        state.connections[index] = saved; return saved;
      }
      if (command === 'ssh_host_trust') return structuredClone(fixture.trust);
      if (command === 'probe_ssh_host') { fixture.probes.push(args.id); return { host: state.connections.find(item => item.id === args.id).host, port: 22, fingerprint: 'SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' }; }
      if (command === 'set_ssh_host_trust') {
        fixture.trustWrites.push(structuredClone(args));
        fixture.trust = fixture.trust.filter(entry => entry.itemId !== args.id || entry.host !== args.host || entry.port !== args.port);
        if (args.fingerprint) fixture.trust.push({ itemId: args.id, host: args.host, port: args.port, fingerprint: args.fingerprint });
        return;
      }
      return {};
    } } };
  });
  const openB = async () => { await page.locator('.connection-card[data-item-id="b"] .item-disclosure').click(); await page.locator('#ssh-route-config').waitFor({ state: 'visible' }); };
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  await openB();
  assert.equal(await page.locator('#ssh-route-mode').inputValue(), 'jump');
  assert.equal(await page.locator('#ssh-jump-item').inputValue(), 'a');
  assert.deepEqual(await page.locator('#ssh-jump-item option').evaluateAll(nodes => nodes.map(node => node.value)), ['', 'a']);
  await page.locator('#ssh-route-mode').selectOption('direct');
  await page.locator('#connection-form .modal-footer [data-close-modal]').click();
  await page.locator('#connection-modal').waitFor({ state: 'hidden' });
  assert.equal(await page.evaluate(() => fixture.state.connections.find(item => item.id === 'b').sshJumpItemId), 'a');
  assert.equal(await page.evaluate(() => fixture.saved.length), 0);
  assert.equal(await page.evaluate(() => fixture.drafts[0].input.sshJumpItemId), null);
  await openB();
  assert.equal(await page.locator('#ssh-route-mode').inputValue(), 'direct');
  await page.locator('#ssh-route-mode').selectOption('jump');
  await page.locator('#ssh-jump-item').selectOption('a');
  await page.locator('#save-connection-button').click();
  await page.waitForFunction(() => fixture.saved.length === 1);
  await page.locator('#connection-modal').waitFor({ state: 'hidden' });
  assert.equal(await page.evaluate(() => fixture.saved[0].sshJumpItemId), 'a');
  assert.equal(await page.evaluate(() => fixture.saved[0].sshJumpUnresolved), false);
  await openB();
  await page.locator('#ssh-trust-details summary').click();
  await page.locator('#ssh-trust-probe').click();
  await page.waitForFunction(() => fixture.probes.length === 1);
  assert.equal(await page.evaluate(() => fixture.trustWrites.length), 0);
  await page.locator('#ssh-trust-save').click();
  assert.equal(await page.evaluate(() => fixture.trustWrites.length), 0);
  await page.locator('#ssh-trust-verified').check();
  await page.locator('#ssh-trust-save').click();
  await page.waitForFunction(() => fixture.trustWrites.length === 1);
  assert.equal(await page.evaluate(() => fixture.trust[0].itemId), 'b');
  await page.locator('#ssh-route-mode').selectOption('direct');
  assert.equal(await page.locator('#ssh-trust-save').isDisabled(), true);
  await page.locator('#connection-form .modal-footer [data-close-modal]').click();
  await page.locator('#connection-modal').waitFor({ state: 'hidden' });
  assert.equal(await page.evaluate(() => fixture.trust.length), 1);
  await page.evaluate(() => sessionStorage.setItem('brokenJump', '1'));
  await page.reload(); await openB();
  assert.equal(await page.locator('#ssh-route-mode').inputValue(), 'jump');
  assert.equal(await page.locator('#ssh-jump-item').inputValue(), 'unresolved:missing');
  assert.match(await page.locator('#ssh-route-path').textContent(), /不可用/);
  await page.locator('#ssh-jump-item').selectOption('a');
  await page.locator('#save-connection-button').click();
  await page.waitForFunction(() => fixture.saved.length === 1);
  await page.locator('#connection-modal').waitFor({ state: 'hidden' });
  assert.equal(await page.evaluate(() => fixture.saved[0].sshJumpItemId), 'a');
  assert.equal(await page.evaluate(() => fixture.saved[0].sshJumpUnresolved), false);
  await openB();
  await page.locator('#ssh-trust-details summary').click();
  await page.locator('#ssh-trust-fingerprint').fill('SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA');
  await page.waitForFunction(() => document.querySelectorAll('#toast-region .toast').length === 0);
  for (const width of [1120, 460]) {
    await page.setViewportSize({ width, height: 1000 });
    await page.locator('#ssh-route-config').scrollIntoViewIfNeeded();
    const geometry = await page.locator('#ssh-route-config').evaluate(element => ({ width: element.clientWidth, scroll: element.scrollWidth }));
    assert.ok(geometry.scroll <= geometry.width + 2, `SSH panel must fit at ${width}px: ${JSON.stringify(geometry)}`);
    const controls = await page.locator('#ssh-trust-details button, #ssh-trust-details input').evaluateAll(elements => elements.map(element => ({ left: element.getBoundingClientRect().left, right: element.getBoundingClientRect().right })));
    const buttons = await page.locator('#ssh-trust-details button').evaluateAll(elements => elements.map(element => ({ height: element.clientHeight, scroll: element.scrollHeight, whiteSpace: getComputedStyle(element).whiteSpace })));
    assert.ok(buttons.every(button => button.whiteSpace === 'nowrap' && button.scroll <= button.height + 1), `Trust button text must remain on one line at ${width}px`);
    const panel = await page.locator('#ssh-route-config').boundingBox();
    assert.ok(controls.every(control => control.left >= panel.x && control.right <= panel.x + panel.width + 1), `Expanded trust controls must fit at ${width}px`);
    await page.locator('#ssh-route-config').screenshot({ path: path.join(artifacts, `ssh-route-${width}.png`) });
  }
  assert.deepEqual(errors, []);
  console.log('SSH jump UI passed:', artifacts);
} finally { await browser.close(); server.close(); }
