import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import { once } from 'node:events';
import { chromium } from 'playwright';
const renderer = path.resolve(import.meta.dirname, '../renderer');
const artifacts = await fs.mkdtemp(path.join(os.tmpdir(), 'openkeylet-cert-ui-'));
const server = http.createServer(async (req, res) => {
  const file = path.resolve(renderer, '.' + (req.url === '/' ? '/index.html' : req.url.split('?')[0]));
  if (!file.startsWith(renderer + path.sep)) { res.writeHead(403); res.end(); return; }
  try { res.setHeader('Content-Type', file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : file.endsWith('.svg') ? 'image/svg+xml' : 'text/html'); res.end(await fs.readFile(file)); }
  catch { res.writeHead(404); res.end(); }
});
server.listen(0, '127.0.0.1'); await once(server, 'listening');
const browser = await chromium.launch({ headless: true, channel: process.env.OPENKEYLET_TEST_CHROME || undefined });
try {
  const page = await browser.newPage({ viewport: { width: 720, height: 1000 } });
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const item = { id: '11111111-1111-4111-8111-111111111111', name: 'Synthetic CA', itemType: 'custom', folder: 'test', enabled: true, revision: 1, updatedAt: 'fixture', modules: [{ id: '22222222-2222-4222-8222-222222222222', kind: 'privateKey', agentVisible: false, configured: true, secret: true }, { id: '33333333-3333-4333-8333-333333333333', kind: 'caCertificate', value: 'synthetic certificate', agentVisible: true, configured: true }], capabilities: [], passkeys: [], secret: { fields: [] } };
    const jump = { ...item, id: '44444444-4444-4444-8444-444444444444', name: 'Synthetic certificate jump', itemType: 'ssh', host: '127.0.0.1', port: 22, sshAuthType: 'certificate', modules: [{kind:'username',configured:true},{kind:'privateKey',configured:true},{kind:'sshCertificate',configured:true,value:'synthetic'}] };
    const state = { settings: { language: 'zh', closeBehavior: 'tray', browserEnabled: false, browserPort: 39272 }, connections: [item, jump], activities: [], mcp: { status: 'ready', endpoint: 'fixture', stdioCommand: 'fixture' }, browserBridge: { status: 'offline' }, security: { encrypted: true, storage: 'fixture' } };
    const caps = { enabled: false, grants: [], authorities: [] };
    window.fixture = { calls: [], state, caps, saved: [] };
    window.__TAURI__ = { event: { listen: async () => () => {} }, core: { invoke: async (command, args = {}) => {
      fixture.calls.push({ command, args: structuredClone(args) });
      if (command === 'get_state') return structuredClone(state);
      if (command === 'owner_status') return { pinConfigured: false, unlocked: true };
      if (['owner_editor_drafts', 'agent_mcp_status', 'owner_trash', 'ssh_host_trust'].includes(command)) return [];
      if (command === 'github_backup_status') return { connected: false };
      if (command === 'system_integration_status') return { desktopShortcut: false, launchAtLogin: false };
      if (command === 'owner_secret_view') return { fields: [] };
      if (command === 'certificate_capabilities') return structuredClone(caps);
      if (command === 'set_certificate_authority') {
        item.certificateAuthority = args.binding;
        caps.authorities = args.binding ? [{ itemId: item.id, kind: args.binding.kind, configured: true, effective: false }] : [];
      }
      if (command === 'certificate_permissions') {
        if (args.enabled !== undefined) caps.enabled = args.enabled;
        else if (args.kinds) caps.grants = [{ itemId: args.id, kinds: args.kinds, maxTtlSeconds: args.maxTtlSeconds }];
        else caps.grants = caps.grants.filter(grant => grant.itemId !== args.id);
        caps.authorities.forEach(authority => { authority.effective = caps.enabled && caps.grants.some(grant => grant.itemId === authority.itemId); });
        return structuredClone(caps);
      }
      if (command === 'certificate_key_check') return { matches: true, algorithm: 'synthetic', fingerprint: 'SHA256:synthetic' };
      if (command === 'certificate_verify') return { verified: true, revocationChecked: false };
      if (command === 'certificate_issue') return { certificate: 'synthetic signed certificate', serial: '123', kind: args.request.kind };
      if (command === 'save_connection') { fixture.saved.push(args.input); return { ...args.input, id: crypto.randomUUID(), updatedAt: 'fixture', capabilities: [], passkeys: [], secret: { fields: [] } }; }
      if (command === 'save_editor_draft') return { id: 'fixture-draft', input: args.input };
      return {};
    } } };
  });
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  await page.locator('[data-page="settings"]').click();
  assert.equal(await page.locator('#certificate-enabled').isChecked(), false);
  const manage = page.locator('#certificate-ca-management');
  const issue = page.locator('#certificate-issue-panel');
  await issue.locator('summary').click();
  assert.equal(await issue.getByText('还没有注册的 CA。').isVisible(), true);
  assert.equal(await page.locator('#certificate-issue-item').isVisible(), false);
  await issue.getByRole('button', { name: '前往注册 CA' }).click();
  await page.waitForFunction(() => document.activeElement === document.querySelector('#certificate-item'));
  assert.equal(await manage.getAttribute('open'), '');
  assert.equal(await page.evaluate(() => fixture.calls.some(call => call.command === 'set_certificate_authority')), false);
  await page.locator('#certificate-item').selectOption('11111111-1111-4111-8111-111111111111');
  await page.locator('#certificate-ca-kind').selectOption('tls');
  await page.locator('#certificate-private-key').selectOption('22222222-2222-4222-8222-222222222222');
  await page.locator('#certificate-ca-module').selectOption('33333333-3333-4333-8333-333333333333');
  await page.locator('#certificate-register').click();
  await page.waitForFunction(() => fixture.state.connections[0].certificateAuthority?.kind === 'tls');
  await page.locator('#certificate-issue-item').selectOption('11111111-1111-4111-8111-111111111111');
  assert.equal(await issue.getByText('还没有注册的 CA。').isVisible(), false);
  await issue.locator('summary').click();
  await page.locator('#certificate-grant-kind').selectOption('tlsServer');
  await page.locator('#certificate-allow').click();
  await page.waitForFunction(() => fixture.caps.grants.length === 1);
  await page.locator('label:has(#certificate-enabled)').click();
  await page.waitForFunction(() => fixture.caps.enabled);
  await page.locator('[data-remove-signing]').click();
  await page.waitForFunction(() => fixture.caps.grants.length === 0);
  await page.locator('label:has(#certificate-enabled)').click();
  const match = page.locator('#certificate-match-panel');
  const verify = page.locator('#certificate-verify-panel');
  await match.locator('summary').click();
  await page.locator('#certificate-match-item').selectOption('11111111-1111-4111-8111-111111111111');
  await page.locator('#certificate-match-payload').fill('synthetic public material');
  await page.locator('#certificate-match').click();
  await page.waitForFunction(() => document.querySelector('#certificate-match-result').value.includes('"matches": true'));
  await match.locator('summary').click();

  await verify.locator('summary').click();
  assert.equal(await page.locator('#certificate-issue-item').isVisible(), false);
  assert.equal(await page.locator('#certificate-match-item').isVisible(), false);
  assert.equal(await page.locator('#certificate-intermediates').isVisible(), false);
  await page.locator('#certificate-verify-kind').selectOption('tlsServer');
  await page.locator('#certificate-verify-payload').fill('synthetic TLS certificate');
  await page.locator('#certificate-verify-identity').fill('localhost');
  await page.locator('#certificate-authority-input').fill('synthetic CA certificate');
  await page.locator('#certificate-intermediates').fill('-----BEGIN CERTIFICATE-----\nsynthetic-one\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\nsynthetic-two\n-----END CERTIFICATE-----');
  await page.locator('#certificate-verify').click();
  await page.waitForFunction(() => document.querySelector('#certificate-verify-result').value.includes('"verified": true'));
  assert.deepEqual(await page.evaluate(() => fixture.calls.find(call => call.command === 'certificate_verify').args.request), {
    kind: 'tlsServer', certificate: 'synthetic TLS certificate', identity: 'localhost', authorities: ['synthetic CA certificate'],
    intermediates: [
      '-----BEGIN CERTIFICATE-----\nsynthetic-one\n-----END CERTIFICATE-----',
      '-----BEGIN CERTIFICATE-----\nsynthetic-two\n-----END CERTIFICATE-----',
    ],
  });
  await page.locator('#certificate-verify-kind').selectOption('sshHost');
  assert.equal(await page.locator('#certificate-intermediates').isVisible(), false);
  await page.locator('#certificate-verify-payload').fill('synthetic SSH certificate');
  await page.locator('#certificate-verify-identity').fill('test.example');
  await page.locator('#certificate-authority-input').fill('synthetic SSH CA key');
  await page.locator('#certificate-verify').click();
  await page.waitForFunction(() => fixture.calls.filter(call => call.command === 'certificate_verify').length === 2);
  assert.deepEqual(await page.evaluate(() => fixture.calls.filter(call => call.command === 'certificate_verify').at(-1).args.request), {
    kind: 'sshHost', certificate: 'synthetic SSH certificate', identity: 'test.example', authorities: ['synthetic SSH CA key'], intermediates: [],
  });
  await verify.locator('summary').click();

  await issue.locator('summary').click();
  assert.equal(await page.locator('#certificate-authority-input').isVisible(), false);
  assert.equal(await page.locator('#certificate-intermediates').isVisible(), false);
  assert.equal(await page.locator('#certificate-key-id').isVisible(), true);
  await page.locator('#certificate-issue-item').selectOption('11111111-1111-4111-8111-111111111111');
  await page.locator('#certificate-key-id').fill('synthetic-key-id');
  await page.locator('#certificate-issue-kind').selectOption('tlsServer');
  assert.equal(await page.locator('#certificate-key-id').isVisible(), false);
  assert.equal(await page.locator('#certificate-issue-payload').inputValue(), '');
  await page.locator('#certificate-issue-payload').fill('synthetic CSR');
  await page.locator('#certificate-issue-names').fill('issuer.example, service.example');
  const requestId = await page.locator('#certificate-request-id').inputValue();
  await page.locator('#certificate-issue').click();
  await page.waitForFunction(() => document.querySelector('#certificate-issue-result').value.includes('synthetic signed certificate'));
  await page.locator('#certificate-issue').click();
  await page.waitForFunction(() => fixture.calls.filter(call => call.command === 'certificate_issue').length === 2);
  assert.deepEqual(await page.evaluate(() => fixture.calls.filter(call => call.command === 'certificate_issue').map(call => call.args.requestId)), [requestId, requestId]);
  assert.deepEqual(await page.evaluate(() => fixture.calls.find(call => call.command === 'certificate_issue').args.request), {
    kind: 'tlsServer', csr: 'synthetic CSR', names: ['issuer.example', 'service.example'],
  });
  assert.match(await page.locator('#certificate-verify-result').inputValue(), /"verified": true/);
  assert.match(await page.locator('#certificate-match-result').inputValue(), /"matches": true/);
  await page.locator('#certificate-new-request').click();
  assert.notEqual(await page.locator('#certificate-request-id').inputValue(), requestId);
  await page.locator('#certificate-issue-kind').selectOption('sshUser');
  await page.locator('#certificate-issue-payload').fill('synthetic public key');
  await page.locator('#certificate-issue-names').fill('test-user');
  await page.locator('#certificate-issue').click();
  await page.waitForFunction(() => fixture.calls.filter(call => call.command === 'certificate_issue').length === 3);
  assert.deepEqual(await page.evaluate(() => fixture.calls.filter(call => call.command === 'certificate_issue').at(-1).args.request), {
    kind: 'sshUser', publicKey: 'synthetic public key', principals: ['test-user'], keyId: 'synthetic-key-id', extensions: [],
  });
  await issue.locator('summary').click();
  await page.setViewportSize({ width: 460, height: 900 });
  await manage.locator('summary').click();
  await issue.locator('summary').click();
  await page.locator('#certificate-issue-kind').selectOption('tlsServer');
  await issue.screenshot({ path: path.join(artifacts, 'certificate-issue-460.png') });
  await issue.locator('summary').click();
  await verify.locator('summary').click();
  await page.locator('#certificate-verify-kind').selectOption('tlsServer');
  await verify.screenshot({ path: path.join(artifacts, 'certificate-verify-460.png') });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
  assert.equal(await page.locator('#certificate-panel button').evaluateAll(buttons => buttons.every(button => getComputedStyle(button).whiteSpace === 'nowrap')), true);
  await page.locator('[data-page="connections"]').click();
  await page.locator('#add-connection-button').click();
  await page.locator('[data-item-template="sshCertificate"]').click();
  assert.equal(await page.locator('.module-row[data-kind="sshCertificate"]').count(), 1);
  const row = page.locator('.module-row[data-kind="sshCertificate"]');
  await row.locator('[data-module-value]').fill('ssh-ed25519-cert-v01@openssh.com synthetic');
  assert.equal(await row.locator('[data-module-agent-visible]').getAttribute('aria-pressed'), 'true');
  await row.locator('[data-module-agent-visible]').click();
  assert.equal(await row.locator('[data-module-agent-visible]').getAttribute('aria-pressed'), 'false');
  assert.equal(await row.locator('[data-module-drag-handle]').count(), 1);
  assert.equal(await row.locator('[data-remove-module]').count(), 1);
  assert.equal(await page.locator('#ssh-auth-type').inputValue(), 'certificate');
  await page.locator('#ssh-route-mode').selectOption('jump');
  assert.equal(await page.locator('#ssh-jump-item option[value="44444444-4444-4444-8444-444444444444"]').count(), 1);
  await page.locator('#connection-modal').screenshot({ path: path.join(artifacts, 'certificate-modules-460.png') });
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ passed: true, artifacts, checks: ['CA empty-state navigation', 'CA binding', 'independent grant enable/revoke', 'separate certificate forms and results', 'TLS and SSH payloads', 'issue retry UUID', 'standard modules', '460px layout'] }));
} finally { await browser.close(); server.close(); }
