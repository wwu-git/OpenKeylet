// Isolated renderer fixture: no desktop vault or real credentials are used.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import { once } from 'node:events';
import { chromium } from 'playwright';
import { auditControlGeometry } from './fixtures/control-geometry.mjs';
import { auditControlStates } from './fixtures/control-states.mjs';

const renderer = path.resolve(import.meta.dirname, '../renderer');
const artifacts = await fs.mkdtemp(path.join(os.tmpdir(), 'openkeylet-inline-ui-'));
const server = http.createServer(async (req, res) => {
  const pathname = decodeURIComponent(req.url.split('?')[0]);
  const file = path.resolve(renderer, '.' + (pathname === '/' ? '/index.html' : pathname));
  if (!file.startsWith(renderer + path.sep)) { res.writeHead(403); res.end(); return; }
  try {
    res.setHeader('Content-Type', file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : file.endsWith('.svg') ? 'image/svg+xml' : 'text/html');
    res.end(await fs.readFile(file));
  } catch { res.writeHead(404); res.end(); }
});
server.listen(0, '127.0.0.1'); await once(server, 'listening');
const browser = await chromium.launch({ headless: true, channel: process.env.OPENKEYLET_TEST_CHROME || undefined });
try {
  const page = await browser.newPage({ viewport: { width: 460, height: 690 }, deviceScaleFactor: 1.5 });
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    window.setInterval = () => 0;
    const item = (id, name, capabilities) => ({
      id, name, capabilities, itemType: 'custom', description: 'Hidden synthetic description',
      enabled: true, canTest: true, updatedAt: 'revision-1', passkeys: [], folder: '', favorite: id === 'alpha',
      modules: [{ kind: 'url', value: 'https://example.test', agentVisible: true }, { kind: 'password', value: '', configured: true, agentVisible: false }],
      secret: { fields: [] }, passwordHistoryCount: 1,
    });
    const state = {
      settings: { language: 'en', closeBehavior: 'tray', browserEnabled: true, browserPort: 39272 },
      connections: [item('alpha', 'Synthetic service with a very long name', ['http']), item('beta', 'Synthetic server', ['ssh']), { ...item('gamma', 'Synthetic password', ['fill']), itemType: 'login' }],
      activities: [
        { id: 'ok', action: 'Synthetic success', connectionName: 'Synthetic server', status: 'success', source: 'mcp', durationMs: 42, time: new Date().toISOString() },
        { id: 'error', action: 'Synthetic failure', connectionName: 'Synthetic API', status: 'error', source: 'mcp', durationMs: 60, error: 'Fixture failure', time: new Date().toISOString() },
      ], mcp: { status: 'ready', endpoint: 'fixture', stdioCommand: 'fixture' },
      browserBridge: { status: 'listening', enabled: true, connected: true, paired: true, endpoint: 'fixture' },
      security: { encrypted: true, storage: 'fixture' },
    };
    window.fixture = { state, drafts: [], saved: [], orders: [], orderRequests: 0, revision: 1, failDraft: false, failOrder: false,
      owner: { pinConfigured: false, unlocked: true }, integrations: { desktopShortcut: false, launchAtLogin: false },
      integrationWrites: [], settingsWrites: [], failIntegration: false, pinDisables: 0 };
    window.__TAURI__ = { event: { listen: async () => () => {} }, core: { invoke: async (command, args = {}) => {
      if (command === 'get_state') return structuredClone(state);
      if (command === 'owner_status') return structuredClone(fixture.owner);
      if (command === 'owner_disable_pin') { fixture.pinDisables++; fixture.owner.pinConfigured = false; return structuredClone(fixture.owner); }
      if (command === 'owner_editor_drafts') return structuredClone(fixture.drafts);
      if (command === 'agent_mcp_status') return [
        { clientId: 'fixture-ready', displayName: 'Synthetic agent', state: 'available', canRegister: true },
        { clientId: 'fixture-stale', displayName: 'Synthetic stale agent', state: 'stale', canRepair: true, canRemove: true },
      ];
      if (command === 'github_backup_status') return { connected: false };
      if (command === 'system_integration_status') return structuredClone(fixture.integrations);
      if (command === 'set_desktop_shortcut' || command === 'set_launch_at_login') {
        fixture.integrationWrites.push({ command, enabled: args.enabled });
        if (fixture.failIntegration) throw Error('Synthetic integration failure');
        fixture.integrations[command === 'set_desktop_shortcut' ? 'desktopShortcut' : 'launchAtLogin'] = args.enabled;
        return structuredClone(fixture.integrations);
      }
      if (command === 'update_settings') { fixture.settingsWrites.push(args.patch); Object.assign(state.settings, args.patch); return state.settings; }
      if (command === 'owner_secret_view' && args.id === 'oci') return { fields: Object.entries(fixture.ociSecrets || {}).map(([name, value]) => ({ name, value })) };
      if (command === 'owner_secret_view' && fixture.keyActionSecret) return { fields: [{name:'privateKey',value:fixture.keyActionSecret}] };
      if (command === 'copy_owner_value') { fixture.copiedValue = args.value; return; }
      if (command === 'owner_secret_view') return { fields: [{ name: 'password', value: 'synthetic-password' }] };
      if (command === 'generate_password') return 'Aa1!'.repeat(32).slice(0, args.options.length);
      if (command === 'stage_private_key') {
        (fixture.keySources ??= []).push(args.source);
        if (args.source === 'file') return null;
        if (args.source === 'clipboard') throw Error('Invalid test key');
        return { id: 'staged-key', name: 'Ed25519', fingerprint: 'SHA256:test' };
      }
      if (command === 'copy_ssh_public_key') { fixture.publicKeyCopy = args; return; }
      if (command === 'discard_private_key') { fixture.discardedKey = args.id; return; }
      if (command === 'api_service_catalog') {
        const catalog = [
          {id:'profile-github',name:'GitHub',category:'developer',urlTemplate:'https://api.github.com/',parameters:[],requirements:'Bearer token',authMode:'bearer'},
          {id:'profile-openai',name:'OpenAI',category:'ai',urlTemplate:'https://api.openai.com/v1/',parameters:[],requirements:'Bearer token',authMode:'bearer'},
          {id:'profile-cloudflare',name:'Cloudflare',category:'cloud',urlTemplate:'https://api.cloudflare.com/client/v4/',parameters:[],requirements:'Bearer token',authMode:'bearer'},
          {id:'template-oci',name:'OCI Object Storage',category:'cloud',urlTemplate:'https://objectstorage.<region>.oraclecloud.com/',parameters:['region'],requirements:'OCI request signing',authMode:'oci-signature'},
          {id:'template-aws',name:'AWS EC2',category:'cloud',urlTemplate:'https://ec2.<region>.amazonaws.com/',parameters:['region'],requirements:'AWS SigV4 signing',authMode:'external'},
          {id:'template-workspace',name:'Model Studio Workspace',category:'ai',urlTemplate:'https://<workspace-id>.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/',parameters:['workspace-id'],requirements:'Bearer token',authMode:'bearer'},
        ];
        if (fixture.deferApiCatalog) return new Promise(resolve => { fixture.resolveApiCatalog = () => resolve(catalog); });
        return catalog;
      }
      if (command === 'oci_regions') return ['ap-tokyo-1', 'ap-osaka-1'];
      if (command === 'stage_oci_key') {
        (fixture.ociSources ??= []).push({source:args.source,hasText:typeof args.text==='string'});
        if (fixture.cancelOciFile && args.source==='file') return null;
        if (fixture.ociKeyError) throw Error(fixture.ociKeyError);
        if (fixture.rejectOciKey) throw Error('Invalid RSA PEM');
        const id = `test-oci-key-${fixture.ociSequence=(fixture.ociSequence||0)+1}`;
        if (fixture.deferOciKey) return new Promise(resolve=>{fixture.resolveOciKey=()=>{fixture.pendingOciKey=id;resolve(id);};});
        fixture.pendingOciKey=id;
        return id;
      }
      if (command === 'discard_oci_key') { fixture.discardedOciKey = args.id; if (fixture.pendingOciKey===args.id) fixture.pendingOciKey=null; return; }
      if (command === 'test_connection' && args.id === 'oci') {
        (fixture.ociChecks ??= []).push(args.id);
        if (fixture.ociCheckFailure) throw Error('OCI_CHECK_FAILED: HTTP 403; request ID: denied-fixture');
        return 'OCI_CHECK_OK: HTTP 200; request ID: check-fixture';
      }
      if (command === 'save_editor_draft') {
        if (fixture.failDraft) throw Error('Synthetic draft failure');
        const draft = { id: args.draftId || `draft-${args.input.id || 'new'}`, input: structuredClone(args.input) };
        fixture.drafts = [draft, ...fixture.drafts.filter(d => d.id !== draft.id)]; return draft;
      }
      if (command === 'delete_editor_draft') { fixture.drafts = fixture.drafts.filter(d => d.id !== args.id); return; }
      if (command === 'save_connection') {
        const index = state.connections.findIndex(i => i.id === args.input.id);
        const existing = state.connections[index];
        if (existing && args.expectedUpdatedAt !== existing.updatedAt) throw Error('Stale edit');
        if (fixture.failOciSave && (args.input.httpAuthType === 'oci-signature' || args.input.id === 'oci')) throw Error('Synthetic save rejected');
        if (args.pendingOciKeyId && args.pendingOciKeyId !== fixture.pendingOciKey) throw Error('Missing pending key');
        fixture.saved.push(structuredClone(args.input));
        const saved = { ...existing, ...args.input, id: args.input.id || 'oci', updatedAt: `revision-${++fixture.revision}` };
        if (saved.id === 'oci') {
          fixture.savedOci = structuredClone(args);
          const next = { ...args.input.secrets.namedSecrets, privateKey: args.input.secrets.privateKey || fixture.ociSecrets?.privateKey || '' };
          if (args.pendingOciKeyId) {
            const module = saved.modules.find(module => module.id === args.pendingOciKeyModuleId);
            next[module.kind === 'customSecret' ? module.name : module.kind] = 'SYNTHETIC-RSA-PEM';
            fixture.pendingOciKey = null;
          }
          fixture.ociSecrets = Object.fromEntries(saved.modules.map(module => module.kind === 'customSecret' ? module.name : module.kind).filter(name => name in next).map(name => [name, next[name]]));
          saved.modules = saved.modules.map(module => ({ ...module, configured: Boolean(fixture.ociSecrets[module.kind === 'customSecret' ? module.name : module.kind]) }));
          saved.canTest = true;
          saved.capabilities = ['http'];
        }
        if (index < 0) state.connections.push(saved); else state.connections[index] = saved;
        return saved;
      }
      if (command === 'delete_connection') { state.connections = state.connections.filter(i => i.id !== args.id); return; }
      if (command === 'reorder_connections') {
        fixture.orderRequests += 1;
        if (fixture.failOrder) throw Error('Synthetic reorder failure');
        if (JSON.stringify(state.connections.map(i => i.id)) !== JSON.stringify(args.expectedOrder)) throw Error('Stale order');
        if (new Set(args.ids).size !== state.connections.length || args.ids.some(id => !state.connections.some(i => i.id === id))) throw Error('Invalid order');
        state.connections = args.ids.map(id => state.connections.find(i => i.id === id));
        fixture.orders.push(args.ids); return;
      }
      return {};
    } } };
  });
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  await page.locator('.connection-card').first().waitFor();
  const row = id => page.locator(`.connection-card[data-item-id="${id}"]`);
  const toggle = id => row(id).locator('.item-disclosure');
  const editor = page.locator('#connection-modal');
  if (process.argv.includes('--field-labels')) {
    const results = [];
    for (const mode of ['modal', 'inline']) for (const width of [690, 460]) for (const size of ['standard', 'large']) for (const language of ['zh', 'en']) {
      await page.setViewportSize({width, height: 900});
      await page.locator('[data-page="settings"]').click();
      await page.locator('#text-size').selectOption(size);
      await page.locator(language === 'zh' ? '[data-language="zh"]' : '[data-language="en"]').click();
      await page.locator('[data-page="connections"]').click();
      if (mode === 'modal') await page.locator('#add-connection-button').click();
      else await toggle('alpha').click();
      await page.waitForFunction(() => !document.querySelector('#connection-modal').classList.contains('is-folding'));
      await editor.locator('.identity-grid').scrollIntoViewIfNeeded();
      const labels = await editor.locator('.identity-grid > .field > span, .vault-metadata > .field > span').evaluateAll(elements => elements.map(element => {
        const bounds = element.getBoundingClientRect();
        const control = element.parentElement.querySelector('input, select').getBoundingClientRect();
        const css = getComputedStyle(element);
        const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
        const glyphs = [];
        while (walker.nextNode()) {
          if (!walker.currentNode.textContent.trim()) continue;
          const range = document.createRange(); range.selectNodeContents(walker.currentNode);
          glyphs.push(...[...range.getClientRects()].filter(rect => rect.width && rect.height));
        }
        return {text: element.textContent.trim(), height: bounds.height, fontSize: parseFloat(css.fontSize), lineHeight: parseFloat(css.lineHeight), glyphClipped: css.overflow === 'hidden' && glyphs.some(rect => rect.top < bounds.top - 0.5 || rect.bottom > bounds.bottom + 0.5), overlapsControl: glyphs.some(rect => rect.bottom > control.top - 1), fitsWidth: element.scrollWidth <= element.clientWidth + 1};
      }));
      const result = {mode, width, size, language, labels}; results.push(result);
      await fs.writeFile(path.join(artifacts, 'field-labels.json'), JSON.stringify(results, null, 2));
      assert.ok(labels.length === 4 && labels.every(label => !label.glyphClipped && !label.overlapsControl && label.fitsWidth && label.lineHeight >= label.fontSize * 1.2), `Field labels must remain complete and above inputs: ${JSON.stringify(result)}`);
      if (mode === 'modal' && language === 'zh') await editor.locator('.identity-grid').screenshot({path: path.join(artifacts, `field-labels-${width}-${size}.png`)});
      await editor.locator('.modal-footer [data-close-modal]').click();
      await editor.waitFor({state: 'hidden'});
      await page.evaluate(async () => { fixture.drafts = []; await refreshDrafts(); });
    }
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({passed: true, cases: results.length, artifacts}));
  } else if (process.argv.includes('--key-actions')) {
    const keyRow=()=>editor.locator('[data-kind="privateKey"]');
    const checkActions=async label=>{
      await page.waitForFunction(()=>!document.querySelector('#connection-modal').classList.contains('is-folding'));
      await page.locator('#toast-region').evaluate(region=>region.replaceChildren());
      for (const button of await keyRow().locator('[data-copy-public-key], [data-cancel-pending-key]').all()) {
        await button.scrollIntoViewIfNeeded();
        const geometry=await button.evaluate(button=>{
          const r=button.getBoundingClientRect(), parent=button.closest('.module-control').getBoundingClientRect();
          const range=document.createRange();
          range.selectNodeContents(button.querySelector('.lettering-ink')||button);
          const lines=[...range.getClientRects()].filter(r=>r.width&&r.height).map(r=>Math.round(r.top));
          const hit=[[r.left+2,r.top+2],[r.right-2,r.top+2],[r.left+2,r.bottom-2],[r.right-2,r.bottom-2]].every(([x,y])=>button.contains(document.elementFromPoint(x,y)));
          return {text:button.textContent,lines:[...new Set(lines)],width:r.width,height:r.height,inside:r.left>=parent.left-1&&r.right<=parent.right+1,overflow:button.scrollWidth>button.clientWidth+1,hit};
        });
        assert.equal(geometry.lines.length,1,`Private key action label must use one line: ${JSON.stringify({label,...geometry})}`);
        assert.equal(geometry.inside&&!geometry.overflow,true,JSON.stringify({label,...geometry}));
        assert.equal(geometry.hit,true,`Private key action corners must be clickable: ${JSON.stringify({label,...geometry})}`);
        await button.click({trial:true});
      }
    };
    await page.evaluate(async()=>{
      fixture.keyActionSecret='SYNTHETIC-ORIGINAL-PRIVATE-KEY';
      Object.assign(fixture.state.connections.find(item=>item.id==='beta'),{privateKeyName:'fixture.pem',modules:[{kind:'privateKey',configured:true,agentVisible:false}]});
      await refresh();
    });
    for (const mode of ['list','grid','modal']) for (const width of [690,460]) for (const size of ['standard','large']) for (const language of ['en','zh']) {
      await page.setViewportSize({width,height:800});
      await page.locator('[data-page="settings"]').click();
      await page.locator('#item-layout').selectOption(mode==='grid'?'grid':'list');
      await page.locator('#text-size').selectOption(size);
      await page.locator(`[data-language="${language}"]`).click();
      await page.locator('[data-page="connections"]').click();
      if (mode==='modal') {
        await page.locator('#add-connection-button').click();
        await page.locator('#add-module-button').click();
        await page.locator('[data-add-module="privateKey"]').click();
      } else {
        await toggle('beta').click();
        await checkActions(`${mode}-${width}-${size}-${language}-configured`);
        await keyRow().locator('[data-copy-public-key]').click();
        assert.deepEqual(await page.evaluate(()=>fixture.publicKeyCopy),{pendingId:null,itemId:'beta'});
      }
      const original=await keyRow().locator('[data-secret-value]').inputValue();
      await keyRow().locator('.key-source-menu summary').click();
      await keyRow().locator('[data-stage-key="generate"]').click();
      await keyRow().locator('[data-cancel-pending-key]').waitFor();
      await checkActions(`${mode}-${width}-${size}-${language}-pending`);
      if (width===460&&size==='large'&&language==='zh') await keyRow().screenshot({path:path.join(artifacts,`key-actions-${mode}.png`)});
      await keyRow().locator('[data-copy-public-key]').click();
      assert.deepEqual(await page.evaluate(()=>fixture.publicKeyCopy),{pendingId:'staged-key',itemId:mode==='modal'?null:'beta'});
      await keyRow().locator('[data-cancel-pending-key]').click();
      assert.equal(await keyRow().locator('[data-cancel-pending-key]').count(),0);
      assert.equal(await keyRow().locator('[data-secret-value]').inputValue(),original,'Cancel replacement preserves the original private key');
      assert.equal(await page.evaluate(()=>fixture.discardedKey),'staged-key');
      await editor.locator('.modal-footer [data-close-modal]').click();
      await editor.waitFor({state:'hidden'});
      await page.evaluate(async()=>{fixture.drafts=[];await refreshDrafts();});
    }
    assert.equal(await page.evaluate(()=>fixture.saved.length),0);
    assert.deepEqual(errors,[]);
    console.log(`Key actions passed: ${artifacts}`);
  } else if (process.argv.includes('--editor-footer')) {
    const results=[];
    for (const mode of ['list','grid','modal']) for (const width of [690,460]) for (const size of ['standard','large']) for (const language of ['en','zh']) {
      await page.setViewportSize({width,height:760});
      await page.locator('[data-page="settings"]').click();
      await page.locator('#item-layout').selectOption(mode==='grid'?'grid':'list');
      await page.locator('#text-size').selectOption(size);
      await page.locator(`[data-language="${language}"]`).click();
      await page.locator('[data-page="connections"]').click();
      if (mode==='modal') await page.locator('#add-connection-button').click();
      else await toggle('alpha').click();
      await editor.waitFor({state:'visible'});
      await page.waitForFunction(()=>!document.querySelector('#connection-modal').classList.contains('is-folding'));
      const footer=editor.locator('.modal-footer');
      await footer.scrollIntoViewIfNeeded();
      const geometry=await footer.evaluate(footer=>{
        const bounds=el=>{const r=el.getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom};};
        return {footer:bounds(footer),buttons:[...footer.querySelectorAll('.button')].map(button=>({button:bounds(button),well:bounds(button.closest('.command-well'))}))};
      });
      results.push({mode,width,size,language,...geometry});
      for (const {button,well} of geometry.buttons) {
        assert.ok(button.top>=well.top-1&&button.bottom<=well.bottom+1&&button.left>=well.left-1&&button.right<=well.right+1,`Footer button must fit inside its well: ${JSON.stringify(results.at(-1))}`);
        assert.ok(well.top>=geometry.footer.top-1&&well.bottom<=geometry.footer.bottom+1,`Footer must contain its button wells: ${JSON.stringify(results.at(-1))}`);
      }
      for (const button of await footer.locator('.button').all()) {
        await button.scrollIntoViewIfNeeded();
        const visible=await button.evaluate(button=>{
          const r=button.getBoundingClientRect();
          return [[r.left+2,r.top+2],[r.right-2,r.top+2],[r.left+2,r.bottom-2],[r.right-2,r.bottom-2]].every(([x,y])=>x>=0&&x<innerWidth&&y>=0&&y<innerHeight&&button.contains(document.elementFromPoint(x,y)));
        });
        assert.equal(visible,true,`Every footer button corner must be visible and clickable: ${JSON.stringify({mode,width,size,language})}`);
        await button.click({trial:true});
      }
      await footer.screenshot({path:path.join(artifacts,`editor-footer-${mode}-${width}-${size}-${language}.png`)});
      await footer.locator('[data-close-modal]').click();
      await editor.waitFor({state:'hidden'});
    }
    assert.deepEqual(errors,[]);
    console.log(JSON.stringify(results));
    console.log(`Editor footer passed: ${artifacts}`);
  } else if (process.argv.includes('--name-toggle')) {
    const name=id=>row(id).locator('.connection-name');
    const settled=async open=>{
      await editor.waitFor({state:open?'visible':'hidden'});
      await page.waitForFunction(()=>!document.querySelector('#connection-modal').classList.contains('is-folding'));
    };
    const expanded=async(id,open)=>{
      assert.equal(await name(id).getAttribute('aria-expanded'),String(open));
      assert.equal(await toggle(id).getAttribute('aria-expanded'),String(open));
    };
    await page.setViewportSize({width:690,height:900});
    for (const layout of ['list','grid']) {
      await page.locator('[data-page="settings"]').click();
      await page.locator('#item-layout').selectOption(layout);
      await page.locator('[data-page="connections"]').click();
      await name('alpha').locator('.lettering-ink').click();
      await settled(true);
      await expanded('alpha',true);
      await name('alpha').click();
      await settled(false);
      await expanded('alpha',false);
      assert.equal(await name('alpha').evaluate(el=>el===document.activeElement),true,'Closing from the name returns focus to the name');
      await name('alpha').press('Enter');
      await settled(true);
      await name('alpha').press('Space');
      await settled(false);
      await expanded('alpha',false);
      const small=await name('alpha').boundingBox();
      await page.mouse.move(small.x+10,small.y+10);
      await page.mouse.down();
      await page.mouse.move(small.x+12,small.y+12);
      await page.mouse.up();
      await settled(true);
      await name('alpha').click();
      await settled(false);
      const from=await name('alpha').boundingBox(), target=await row('beta').boundingBox();
      await page.mouse.move(from.x+from.width/2,from.y+from.height/2);
      await page.mouse.down();
      await page.mouse.move(layout==='grid'?target.x+target.width-5:from.x+from.width/2,layout==='grid'?target.y+target.height/2:target.y+target.height-2,{steps:8});
      await page.mouse.up();
      await page.waitForFunction(()=>!document.querySelector('#connections-list').hasAttribute('aria-busy'));
      assert.deepEqual(await page.locator('.connection-card').evaluateAll(cards=>cards.map(card=>card.dataset.itemId)),['beta','alpha','gamma']);
      assert.equal(await editor.isHidden(),true,'Dragging the name must not toggle editing');
      await name('alpha').press('Home');
      await page.waitForFunction(()=>!document.querySelector('#connections-list').hasAttribute('aria-busy'));
      assert.deepEqual(await page.locator('.connection-card').evaluateAll(cards=>cards.map(card=>card.dataset.itemId)),['alpha','beta','gamma']);
      assert.equal(await editor.isHidden(),true,'Keyboard reorder must not toggle editing');
      const cancelStart=await name('alpha').boundingBox();
      await page.mouse.move(cancelStart.x+20,cancelStart.y+12);
      await page.mouse.down();
      await page.mouse.move(cancelStart.x+30,cancelStart.y+12,{steps:3});
      await page.keyboard.press('Escape');
      await page.waitForTimeout(30);
      await page.mouse.up();
      assert.equal(await editor.isHidden(),true,'Canceled name drag must not toggle editing');
      await name('alpha').press('Enter');
      await settled(true);
      await name('alpha').press('Space');
      await settled(false);
      await page.mouse.move(cancelStart.x+20,cancelStart.y+12);
      await page.mouse.down();
      await page.mouse.move(2,2,{steps:4});
      await page.mouse.up();
      assert.equal(await editor.isHidden(),true,'Dropping a name outside the list must not toggle editing');
      await name('alpha').click();
      await settled(true);
      await page.locator('#connection-description').fill(`Unsaved ${layout} name edit`);
      await page.evaluate(()=>{fixture.failDraft=true;});
      await name('beta').click();
      await page.getByText('Synthetic draft failure',{exact:true}).waitFor();
      assert.equal(await editor.evaluate(el=>el.parentElement.dataset.itemId),'alpha');
      await expanded('alpha',true);
      await expanded('beta',false);
      assert.equal(await page.locator('#connection-description').inputValue(),`Unsaved ${layout} name edit`);
      await page.evaluate(()=>{fixture.failDraft=false;});
      await name('alpha').click();
      await settled(false);
      assert.equal(await page.evaluate(()=>fixture.drafts.find(draft=>draft.input.id==='alpha').input.description),`Unsaved ${layout} name edit`);
      await name('alpha').click();
      await settled(true);
      assert.equal(await page.locator('#connection-description').inputValue(),`Unsaved ${layout} name edit`);
      await name('alpha').click();
      await settled(false);
    }
    assert.equal(await page.evaluate(()=>fixture.saved.length),0);
    assert.deepEqual(errors,[]);
    console.log(`Name toggle passed: ${artifacts}`);
  } else if (process.argv.includes('--grid-editor')) {
    await page.locator('[data-page="settings"]').click();
    await page.locator('#item-layout').selectOption('grid');
    await page.locator('[data-page="connections"]').click();
    const results=[];
    for (const count of [3,81]) for (const width of [690,460]) for (const size of ['standard','large']) {
      await page.setViewportSize({width,height:900});
      await page.locator('#text-size').evaluate((select,value)=>{select.value=value;select.dispatchEvent(new Event('change',{bubbles:true}));},size);
      await page.evaluate(async count => {
        const sample=structuredClone(fixture.state.connections[0]);
        sample.modules=sample.modules.filter(module=>module.kind!=='privateKey');
        sample.modules.push({kind:'privateKey',configured:true,agentVisible:false});
        fixture.state.connections=Array.from({length:count},(_,index)=>({...structuredClone(sample),id:`grid-${index}`,name:`Synthetic grid item ${index}`}));
        await refresh();
      },count);
      await toggle('grid-0').click();
      await page.waitForFunction(() => !document.querySelector('#connection-modal').classList.contains('is-folding'));
      const geometry=await page.evaluate(() => {
        const bounds=el=>{const r=el.getBoundingClientRect();return {top:r.top,bottom:r.bottom,height:r.height,width:r.width};};
        const card=document.querySelector('.connection-card.is-editing');
        return {card:bounds(card),editor:bounds(document.querySelector('#connection-modal')),form:bounds(document.querySelector('#connection-form')),next:bounds(card.nextElementSibling)};
      });
      results.push({count,width,size,...geometry});
      assert.ok(geometry.card.bottom>=geometry.editor.bottom-1,`Expanded card must contain the editor: ${JSON.stringify(results.at(-1))}`);
      assert.ok(geometry.next.top>=geometry.editor.bottom-1,`Following grid cards must start below the editor: ${JSON.stringify(results.at(-1))}`);
      await page.screenshot({path:path.join(artifacts,`grid-editor-${count}-${width}-${size}.png`)});
      const save=page.locator('#save-connection-button');
      await save.scrollIntoViewIfNeeded();
      assert.equal(await save.evaluate(button=>{const r=button.getBoundingClientRect();return button.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));}),true,'Save must be reachable above subsequent grid cards');
      await save.click({trial:true});
      await editor.locator('.modal-footer [data-close-modal]').click();
      await editor.waitFor({state:'hidden'});
      await toggle('grid-0').click();
      await page.waitForFunction(() => !document.querySelector('#connection-modal').classList.contains('is-folding'));
      assert.equal(await editor.isVisible(),true,'Collapsed grid editor reopens');
      await editor.locator('.modal-footer [data-close-modal]').click();
      await editor.waitFor({state:'hidden'});
    }
    assert.deepEqual(errors,[]);
    console.log(JSON.stringify(results));
    console.log(`Grid editor passed: ${artifacts}`);
  } else if (process.argv.includes('--grid')) {
    const order = () => page.locator('.connection-card').evaluateAll(rows => rows.map(row => row.dataset.itemId));
    const switchLayout = async layout => page.locator('#item-layout').evaluate((select, value) => { select.value = value; select.dispatchEvent(new Event('change', {bubbles:true})); }, layout);
    const search = async text => {
      if (!await page.locator('#title-search-input').isVisible()) await page.locator('#title-search-toggle').click();
      await page.locator('#title-search-input').fill(text);
    };
    const drag = async (from, target, after, cancel = false, vertical = false) => {
      await row(from).locator('.connection-name').scrollIntoViewIfNeeded();
      const start = await row(from).locator('.connection-name').boundingBox();
      const end = await row(target).boundingBox();
      await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2);
      await page.mouse.down();
      await page.mouse.move(vertical ? end.x+end.width/2 : after ? end.x+end.width-5 : end.x+5, vertical ? after ? end.y+end.height-5 : end.y+5 : end.y+end.height/2, {steps:8});
      if (cancel) await page.keyboard.press('Escape');
      await page.mouse.up();
      await page.waitForFunction(() => !document.querySelector('#connections-list').hasAttribute('aria-busy'));
    };
    assert.equal(await page.locator('html').getAttribute('data-item-layout'), 'list');
    await page.locator('[data-page="settings"]').click();
    await page.locator('#item-layout').selectOption('grid');
    assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('openkeylet.appearance.v1')).itemLayout), 'grid');
    await page.reload();
    await page.locator('.connection-card').first().waitFor();
    assert.equal(await page.locator('html').getAttribute('data-item-layout'), 'grid');
    await page.setViewportSize({width:690,height:1000});
    const a = await row('alpha').boundingBox(), b = await row('beta').boundingBox();
    assert.ok(Math.abs(a.y-b.y)<1 && b.x>a.x, 'Grid puts two cards on the same row at 690px');
    await drag('alpha','beta',true);
    assert.deepEqual(await order(), ['beta','alpha','gamma']);
    await drag('gamma','beta',false);
    assert.deepEqual(await order(), ['gamma','beta','alpha']);
    await row('alpha').locator('.connection-name').press('Home');
    await page.waitForFunction(() => !document.querySelector('#connections-list').hasAttribute('aria-busy'));
    assert.deepEqual(await order(), ['alpha','gamma','beta']);
    const startEnd=await row('alpha').locator('.connection-name').boundingBox(), lastEnd=await row('beta').boundingBox();
    await page.mouse.move(startEnd.x+10,startEnd.y+10);
    await page.mouse.down();
    await page.mouse.move(lastEnd.x+5,lastEnd.y+lastEnd.height+15,{steps:8});
    await page.mouse.up();
    await page.waitForFunction(() => !document.querySelector('#connections-list').hasAttribute('aria-busy'));
    assert.deepEqual(await order(),['gamma','beta','alpha'],'Bottom whitespace appends after the last tile');
    await row('alpha').locator('.connection-name').press('Home');
    await page.waitForFunction(() => !document.querySelector('#connections-list').hasAttribute('aria-busy'));
    await drag('alpha','beta',true,true);
    assert.deepEqual(await order(), ['alpha','gamma','beta']);
    await page.evaluate(() => { fixture.failOrder=true; });
    await drag('alpha','beta',true);
    assert.deepEqual(await order(), ['alpha','gamma','beta']);
    await page.evaluate(async () => {
      fixture.failOrder=false;
      fixture.state.connections.push({...structuredClone(fixture.state.connections[0]),id:'delta',name:'Second synthetic API',folder:'test'});
      fixture.state.connections.find(item=>item.id==='alpha').folder='test';
      fixture.state.connections.find(item=>item.id==='beta').enabled=false;
      await refresh();
    });
    await page.locator('[data-item-type-filter="api"]').click();
    await drag('delta','alpha',false);
    assert.deepEqual(await page.evaluate(() => fixture.state.connections.map(item=>item.id)), ['delta','gamma','beta','alpha']);
    await search('synthetic');
    await page.locator('#connection-folder-filter').selectOption('folder:test');
    await page.locator('#connection-status-filter').selectOption('enabled');
    assert.deepEqual(await order(), ['delta','alpha']);
    await page.locator('#title-search-toggle').click();
    await page.locator('[data-item-type-filter="ssh"]').click();
    assert.equal(await page.locator('.empty-state').isVisible(),true);
    await page.locator('.empty-state [data-action="clear-connection-filters"]').click();
    await toggle('alpha').click();
    await page.locator('#connection-description').fill('Unsaved grid edit');
    await editor.locator('[data-secret-value]').fill('SYNTHETIC-UNSAVED');
    await page.evaluate(() => { window.gridEditorNode=document.querySelector('#connection-modal'); });
    await switchLayout('list');
    await switchLayout('grid');
    assert.equal(await page.evaluate(() => window.gridEditorNode===document.querySelector('#connection-modal')),true);
    assert.equal(await page.locator('#connection-description').inputValue(),'Unsaved grid edit');
    assert.equal(await editor.locator('[data-secret-value]').inputValue(),'SYNTHETIC-UNSAVED');
    assert.equal(await page.evaluate(() => fixture.saved.length),0);
    const editing = await row('alpha').boundingBox(), list = await page.locator('#connections-list').boundingBox();
    assert.ok(Math.abs(editing.width-list.width)<2,'Expanded grid editor spans the full list');
    await search('not-a-match');
    assert.deepEqual(await order(),['alpha']);
    assert.equal(await page.locator('#connection-description').inputValue(),'Unsaved grid edit');
    await search('');
    await page.locator('#title-search-toggle').click();
    await toggle('alpha').click();
    await editor.waitFor({state:'hidden'});
    await page.keyboard.press('Escape');
    await page.setViewportSize({width:460,height:1000});
    const narrowA=await row('delta').boundingBox(), narrowB=await row('gamma').boundingBox();
    assert.ok(Math.abs(narrowA.x-narrowB.x)<1&&narrowB.y>narrowA.y,'460px grid uses one column');
    await drag('delta','gamma',true,false,true);
    assert.deepEqual(await order(),['gamma','delta','beta','alpha'],'Single-column downward drag uses vertical position');
    await drag('delta','gamma',false,false,true);
    assert.deepEqual(await order(),['delta','gamma','beta','alpha'],'Single-column upward drag uses vertical position');
    for (const width of [1000,690,460]) {
      await page.setViewportSize({width,height:1000});
      for (const size of ['standard','large']) {
        await page.locator('#text-size').evaluate((select, value) => {select.value=value;select.dispatchEvent(new Event('change',{bubbles:true}));},size);
        for (const language of ['en','zh']) {
          await page.locator(`[data-language="${language}"]`).click();
          for (const theme of ['dark','light']) {
            if (await page.locator('html').getAttribute('data-theme')!==theme) await page.locator('#theme-toggle').click();
            const geometry=await page.locator('.connection-card').evaluateAll(cards=>cards.map(card=>{
              const outer=card.getBoundingClientRect();
              return {width:outer.width,overflow:card.scrollWidth>card.clientWidth+1,controls:[...card.querySelectorAll('button')].map(button=>{const r=button.getBoundingClientRect();return r.left>=outer.left&&r.right<=outer.right+1;})};
            }));
            assert.ok(geometry.every(card=>!card.overflow&&card.controls.every(Boolean)),JSON.stringify(geometry));
            assert.equal(await page.evaluate(() => document.documentElement.scrollWidth<=innerWidth),true);
            await page.screenshot({path:path.join(artifacts,`grid-${width}-${size}-${language}-${theme}.png`)});
          }
        }
      }
    }
    await switchLayout('list');
    await page.reload();
    await page.locator('.connection-card').first().waitFor();
    assert.equal(await page.locator('html').getAttribute('data-item-layout'),'list');
    await page.evaluate(() => localStorage.setItem('openkeylet.appearance.v1',JSON.stringify({itemLayout:'invalid'})));
    await page.reload();
    await page.locator('.connection-card').first().waitFor();
    assert.equal(await page.locator('html').getAttribute('data-item-layout'),'list');
    await page.addInitScript(() => {
      Object.defineProperty(Storage.prototype,'getItem',{value(){throw Error('Unavailable test storage');}});
      Object.defineProperty(Storage.prototype,'setItem',{value(){throw Error('Unavailable test storage');}});
    });
    await page.reload();
    await page.locator('.connection-card').first().waitFor();
    assert.equal(await page.locator('html').getAttribute('data-item-layout'),'list');
    await switchLayout('grid');
    assert.equal(await page.locator('html').getAttribute('data-item-layout'),'grid','Unavailable storage still permits session-only layout changes');
    assert.deepEqual(errors,[]);
    console.log(`Grid display passed: ${artifacts}`);
  } else if (process.argv.includes('--api-services')) {
    const picker = page.locator('#api-service-modal');
    const urls = () => editor.locator('[data-kind="url"]');
    const addresses = () => urls().locator('[data-module-value], [data-module-alternate-value]');
    const openPicker = async name => {
      await urls().locator('[data-select-api-service]').click();
      await picker.waitFor({state:'visible'});
      if (name) await picker.locator('[data-api-service]').filter({has:page.locator('strong',{hasText:name})}).click();
    };
    const closeEditor = async () => {
      await editor.locator('.modal-footer [data-close-modal]').click();
      await editor.waitFor({state:'hidden'});
    };
    await page.locator('#add-connection-button').click();
    await page.locator('[data-item-template="api"]').click();
    assert.equal(await urls().count(),1);
    assert.equal(await urls().locator('[data-select-api-service]').evaluate(el=>getComputedStyle(el).flexBasis),'auto');
    assert.equal(await urls().locator('[data-select-api-service]').evaluate(el=>getComputedStyle(el).whiteSpace),'nowrap');
    await openPicker('GitHub');
    assert.equal(await picker.locator('#api-service-preview').inputValue(),'https://api.github.com/');
    await picker.locator('#api-service-cancel').click();
    assert.equal(await addresses().first().inputValue(),'');
    assert.equal(await page.evaluate(()=>fixture.saved.length),0);
    await closeEditor();
    await toggle('alpha').click();
    await openPicker('GitHub');
    assert.equal(await addresses().first().inputValue(),'https://example.test');
    assert.match(await picker.locator('#api-service-replace').innerText(),/Replace/i);
    await picker.locator('#api-service-replace').click();
    assert.equal(await addresses().first().inputValue(),'https://api.github.com/');
    await page.locator('#save-connection-button').click();
    await editor.waitFor({state:'hidden'});
    const savedBefore = await page.evaluate(()=>JSON.stringify(fixture.saved.at(-1)));
    const saved = JSON.parse(savedBefore);
    assert.equal(saved.httpAuthType,'auto');
    assert.equal(saved.secrets.password,'synthetic-password');
    assert.equal(saved.modules.find(module=>module.kind==='password').agentVisible,false);
    await toggle('alpha').click();
    await addresses().first().fill('api.github.com');
    await openPicker('GitHub');
    assert.equal(await picker.locator('#api-service-target').evaluate(el=>el===document.activeElement),true);
    await picker.locator('#api-service-add').click();
    assert.match(await picker.locator('#api-service-error').innerText(),/already exists/);
    await page.keyboard.press('Escape');
    await addresses().first().fill('https://api.github.com/');
    await urls().locator('[data-add-address]').click();
    await addresses().nth(1).fill('https://alternate.test/');
    await openPicker('Cloudflare');
    await picker.locator('#api-service-target').selectOption('1');
    await picker.locator('#api-service-replace').click();
    assert.equal(await addresses().first().inputValue(),'https://api.github.com/');
    assert.equal(await addresses().nth(1).inputValue(),'https://api.cloudflare.com/client/v4/');
    await openPicker('OpenAI');
    await picker.locator('#api-service-add').click();
    assert.equal(await addresses().count(),3);
    assert.equal(await addresses().first().inputValue(),'https://api.github.com/');
    await openPicker('OpenAI');
    await picker.locator('#api-service-add').click();
    assert.match(await picker.locator('#api-service-error').innerText(),/already exists/);
    assert.equal(await addresses().count(),3);
    await page.keyboard.press('Escape');
    await picker.waitFor({state:'hidden'});
    assert.equal(await editor.isVisible(),true);
    assert.equal(await urls().locator('[data-select-api-service]').evaluate(el=>el===document.activeElement),true);
    await openPicker();
    await picker.locator('#api-service-search').fill('workspace');
    assert.equal(await picker.locator('[data-api-service]').count(),1);
    await picker.locator('#api-service-category').selectOption('cloud');
    assert.equal(await picker.locator('[data-api-service]').count(),0);
    await picker.locator('#api-service-category').selectOption('ai');
    assert.equal(await picker.locator('[data-api-service]').count(),1);
    await picker.locator('[data-api-service="template-workspace"]').click();
    assert.equal(await picker.locator('#api-service-add').isDisabled(),true);
    await picker.locator('[data-api-service-parameter="workspace-id"]').fill('bad.example/');
    assert.equal(await picker.locator('#api-service-add').isDisabled(),true);
    for (const invalid of ['bad label','bad/label','bad-label-','a'.repeat(64)]) {
      await picker.locator('[data-api-service-parameter="workspace-id"]').fill(invalid);
      assert.equal(await picker.locator('#api-service-add').isDisabled(),true);
    }
    await picker.locator('[data-api-service-parameter="workspace-id"]').fill('a'.repeat(63));
    assert.equal(await picker.locator('#api-service-add').isEnabled(),true);
    await picker.locator('[data-api-service-parameter="workspace-id"]').fill('workspace-demo');
    assert.equal(await picker.locator('#api-service-preview').inputValue(),'https://workspace-demo.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/');
    await picker.locator('#api-service-add').click();
    assert.equal(await addresses().count(),4);
    await openPicker('OCI Object Storage');
    assert.match(await picker.locator('#api-service-requirements').innerText(),/needs OCI signing/);
    await picker.locator('[data-api-service-parameter="region"]').fill('ap-tokyo-1');
    await picker.locator('#api-service-add').click();
    assert.equal(await page.locator('#http-auth-type').inputValue(),'auto');
    await openPicker('AWS EC2');
    assert.match(await picker.locator('#api-service-requirements').innerText(),/Address only/);
    await page.keyboard.press('Escape');
    await openPicker('GitHub');
    await page.evaluate(()=>{
      const input=document.querySelector('[data-kind="url"] [data-module-value]');
      input.value='https://changed-while-open.test/';input.dispatchEvent(new Event('input',{bubbles:true}));
    });
    await picker.locator('#api-service-replace').click();
    assert.match(await picker.locator('#api-service-error').innerText(),/changed/);
    assert.equal(await addresses().first().inputValue(),'https://changed-while-open.test/');
    await picker.locator('#api-service-refresh-target').click();
    assert.equal(await picker.locator('#api-service-current').inputValue(),'https://changed-while-open.test/');
    await picker.locator('#api-service-replace').click();
    assert.equal(await addresses().first().inputValue(),'https://api.github.com/');
    await openPicker('Cloudflare');
    await page.evaluate(()=>document.querySelector('[data-kind="url"] [data-default-address="1"]').click());
    await picker.locator('#api-service-replace').click();
    assert.match(await picker.locator('#api-service-error').innerText(),/changed/);
    await page.keyboard.press('Escape');
    assert.equal(await page.evaluate(()=>JSON.stringify(fixture.saved.at(-1))),savedBefore);
    await closeEditor();
    assert.equal(await page.evaluate(()=>JSON.stringify(fixture.saved.at(-1))),savedBefore);
    await page.locator('#add-connection-button').click();
    await page.locator('[data-item-template="oci"]').click();
    await editor.locator('[data-fill-oci-region]').selectOption('ap-tokyo-1');
    const before = await page.evaluate(()=>JSON.stringify({auth:editorHttpAuthType,bindings:editorOciBindings,secrets:currentModules.filter(module=>module.kind!=='url')}));
    await openPicker('OCI Object Storage');
    assert.equal(await picker.locator('[data-api-service-parameter="region"]').inputValue(),'ap-tokyo-1');
    await picker.locator('[data-api-service-parameter="region"]').fill('ap-osaka-1');
    assert.equal(await picker.locator('#api-service-replace').isDisabled(),true);
    assert.match(await picker.locator('#api-service-error').innerText(),/bound region/);
    await picker.locator('[data-api-service="profile-github"]').click();
    assert.equal(await picker.locator('#api-service-replace').isDisabled(),true);
    await picker.locator('[data-api-service="template-oci"]').click();
    await picker.locator('#api-service-replace').click();
    assert.equal(await addresses().first().inputValue(),'https://objectstorage.ap-tokyo-1.oraclecloud.com/');
    assert.equal(await page.evaluate(()=>JSON.stringify({auth:editorHttpAuthType,bindings:editorOciBindings,secrets:currentModules.filter(module=>module.kind!=='url')})),before);
    await openPicker('OCI Object Storage');
    await page.evaluate(()=>{const module=currentModules.find(module=>module.id===editorOciBindings.region);module.secretValue='';renderModules();});
    await picker.locator('[data-api-service="template-oci"]').click();
    assert.match(await picker.locator('#api-service-error').innerText(),/Complete|bound/);
    await page.keyboard.press('Escape');
    await closeEditor();
    await toggle('beta').click();
    await page.evaluate(()=>{apiServiceCatalog=null;fixture.deferApiCatalog=true;});
    await openPicker();
    await page.waitForFunction(()=>typeof fixture.resolveApiCatalog==='function');
    await page.keyboard.press('Escape');
    await closeEditor();
    await toggle('alpha').click();
    await page.evaluate(()=>fixture.resolveApiCatalog());
    assert.equal(await picker.isHidden(),true);
    await openPicker('GitHub');
    await page.evaluate(async()=>{fixture.owner={pinConfigured:true,unlocked:false};await refreshOwnerLock(false);});
    assert.equal(await picker.isHidden(),true);
    assert.equal(await editor.isHidden(),true);
    await page.evaluate(async()=>{fixture.owner={pinConfigured:false,unlocked:true};await refreshOwnerLock(false);fixture.deferApiCatalog=false;document.querySelector('#toast-region').replaceChildren();});
    await toggle('alpha').click();
    await openPicker('AWS EC2');
    await picker.locator('[data-api-service-parameter="region"]').fill('us-east-1');
    await picker.locator('#api-service-replace').focus();
    await page.keyboard.press('Tab');
    assert.equal(await picker.locator('[data-close-service-picker]').first().evaluate(el=>el===document.activeElement),true);
    await page.keyboard.press('Shift+Tab');
    assert.equal(await picker.locator('#api-service-replace').evaluate(el=>el===document.activeElement),true);
    for (const width of [690,460]) {
      await page.setViewportSize({width,height:760});
      for (const language of ['en','zh']) {
        await page.locator(`[data-language="${language}"]`).evaluate(button=>button.click());
        for (const theme of ['dark','light']) {
          if (await page.locator('html').getAttribute('data-theme')!==theme) await page.locator('#theme-toggle').evaluate(button=>button.click());
          assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
          assert.equal(await picker.evaluate(el=>el.scrollWidth<=el.clientWidth+1),true);
          const buttonGeometry=await picker.locator('button').evaluateAll(buttons=>buttons.map(button=>({text:button.textContent,width:button.clientWidth,scrollWidth:button.scrollWidth,height:button.clientHeight,scrollHeight:button.scrollHeight})));
          assert.ok(buttonGeometry.every(button=>button.scrollWidth<=button.width+1&&button.scrollHeight<=button.height+1),JSON.stringify(buttonGeometry));
          assert.equal(await picker.evaluate(el=>el.querySelector('#api-service-refresh-target').getBoundingClientRect().bottom<=el.querySelector('#api-service-current').closest('label').getBoundingClientRect().top+1),true);
          await picker.screenshot({path:path.join(artifacts,`api-services-${width}-${language}-${theme}.png`)});
        }
      }
    }
    assert.deepEqual(errors,[]);
    console.log(`API service picker passed: ${artifacts}`);
  } else if (process.argv.includes('--oci')) {
    const rows = editor.locator('#module-list .module-row');
    const module = name => rows.filter({ has: page.locator(`[data-module-name][value="${name}"]`) });
    const keyRow = () => editor.locator('[data-kind="privateKey"]');
    const chooseSource = async source => {
      const menu = keyRow().locator('.key-source-menu');
      if (!await menu.evaluate(el => el.open)) await menu.locator('summary').click();
      await menu.locator(`[data-stage-key="${source}"]`).click();
    };
    const save = page.locator('#save-connection-button');
    await page.locator('#add-connection-button').click();
    await page.locator('[data-item-template="oci"]').click();
    await page.locator('#oci-bindings-summary').click();
    assert.equal(await rows.count(), 6);
    assert.equal(await editor.locator('#oci-config').count(), 0);
    assert.equal(await page.locator('#connection-type').isEnabled(), true);
    await page.locator('#connection-name').fill('OCI modules');
    await module('region').locator('[data-fill-oci-region]').selectOption('ap-tokyo-1');
    assert.equal(await module('region').locator('[data-secret-value]').inputValue(), 'ap-tokyo-1');
    assert.equal(await editor.locator('[data-kind="url"] [data-module-value]').inputValue(), 'https://iaas.ap-tokyo-1.oraclecloud.com/20160918/');
    await module('tenancy_ocid').locator('[data-secret-value]').fill('ocid1.tenancy.oc1..test');
    await module('user_ocid').locator('[data-secret-value]').fill('ocid1.user.oc1..test');
    await module('fingerprint').locator('[data-secret-value]').fill(Array(16).fill('00').join(':'));
    for (const row of await rows.all()) {
      assert.equal(await row.locator('[data-module-agent-visible]').count(), 1);
      assert.equal(await row.locator('[data-remove-module]').count(), 1);
      assert.equal(await row.locator('[data-module-drag-handle]').count(), 1);
    }
    assert.equal(await module('region').locator('[data-module-agent-visible]').getAttribute('aria-pressed'), 'false');
    const regionId = await module('region').getAttribute('data-module-id');
    await module('region').locator('[data-toggle-module-secret]').click();
    assert.equal(await module('region').locator('[data-secret-value]').getAttribute('type'), 'text');
    await module('region').locator('[data-copy-module-secret]').click();
    assert.equal(await page.evaluate(() => fixture.copiedValue), 'ap-tokyo-1');
    await module('region').locator('[data-module-name]').fill('oci_region');
    assert.equal(await editor.locator('[data-oci-binding="region"]').inputValue(), regionId);
    await keyRow().locator('[data-module-agent-visible]').click();
    assert.equal(await keyRow().locator('[data-module-agent-visible]').getAttribute('aria-pressed'), 'true');
    await page.evaluate(() => { fixture.cancelOciFile = true; });
    await chooseSource('file');
    assert.equal(await keyRow().getAttribute('data-pending'), '');
    await page.evaluate(() => { fixture.cancelOciFile = false; fixture.rejectOciKey = true; });
    await chooseSource('clipboard');
    await page.getByText('Invalid RSA PEM', { exact: true }).last().waitFor();
    assert.equal(await keyRow().getAttribute('data-pending'), '');
    await chooseSource('input');
    await keyRow().locator('[data-key-input]').fill('SYNTHETIC-INVALID-PEM');
    assert.equal(await save.isDisabled(), true);
    await keyRow().locator('[data-confirm-key-input]').click();
    await page.waitForFunction(() => !document.querySelector('[data-confirm-key-input]').disabled);
    assert.equal(await keyRow().locator('[data-key-input]').inputValue(), 'SYNTHETIC-INVALID-PEM');
    await page.locator('#http-auth-type').selectOption('auto');
    assert.equal(await keyRow().locator('[data-key-input]').isVisible(), true);
    await keyRow().locator('[data-key-input]').press('Escape');
    assert.equal(await save.isEnabled(), true);
    await page.locator('#http-auth-type').selectOption('oci-signature');
    await page.evaluate(() => { fixture.rejectOciKey = false; });
    await chooseSource('input');
    await keyRow().locator('[data-key-input]').fill('SYNTHETIC-RSA-PEM');
    await keyRow().locator('[data-confirm-key-input]').click();
    await page.waitForFunction(() => Boolean(document.querySelector('[data-kind="privateKey"]').dataset.pending));
    await page.evaluate(() => { fixture.deferOciKey = true; });
    await chooseSource('clipboard');
    await page.waitForFunction(() => typeof fixture.resolveOciKey === 'function');
    await editor.locator('[data-oci-binding="privateKey"]').selectOption('');
    await page.evaluate(() => { fixture.resolveOciKey(); fixture.deferOciKey = false; });
    await page.waitForFunction(() => !document.querySelector('#save-connection-button').disabled);
    const pending = await keyRow().getAttribute('data-pending');
    assert.equal(await page.evaluate(() => fixture.pendingOciKey), pending);
    const keyId = await keyRow().getAttribute('data-module-id');
    await editor.locator('[data-oci-binding="privateKey"]').selectOption('');
    await page.evaluate(() => { fixture.failOciSave = true; });
    await save.click();
    await page.getByText('Synthetic save rejected', { exact: true }).waitFor();
    assert.equal(await keyRow().getAttribute('data-pending'), pending);
    await page.evaluate(() => { fixture.failOciSave = false; });
    await save.click();
    await editor.waitFor({ state: 'hidden' });
    assert.equal(await page.evaluate(() => fixture.savedOci.pendingOciKeyModuleId), keyId);
    assert.equal(await page.evaluate(() => fixture.savedOci.pendingOciKeyId), pending);
    assert.equal(await page.evaluate(() => fixture.savedOci.input.ociBindings.privateKey), null);
    await toggle('oci').click();
    assert.equal(await keyRow().locator('[data-toggle-module-secret]').isEnabled(), true);
    await keyRow().locator('[data-toggle-module-secret]').click();
    assert.equal(await keyRow().locator('[data-secret-value]').evaluate(el => !el.classList.contains('secret-masked')), true);
    await keyRow().locator('[data-copy-module-secret]').click();
    assert.equal(await page.evaluate(() => fixture.copiedValue), 'SYNTHETIC-RSA-PEM');
    await editor.locator('[data-oci-binding="privateKey"]').selectOption(keyId);
    await module('oci_region').locator('[data-module-drag-handle]').focus();
    await page.keyboard.press('ArrowDown');
    assert.equal(await editor.locator('[data-oci-binding="region"]').inputValue(), regionId);
    await page.locator('#add-module-button').click();
    assert.equal(await page.locator('#module-menu [data-add-module="url"]').isVisible(), true);
    await page.locator('#module-menu [data-add-module="customSecret"]').click();
    const extra = rows.last();
    await extra.locator('[data-module-name]').fill('auth_token');
    await extra.locator('[data-secret-value]').fill('SYNTHETIC-EXTRA-TOKEN');
    await save.click();
    await editor.waitFor({ state: 'hidden' });
    assert.equal(await page.evaluate(() => fixture.savedOci.input.secrets.namedSecrets.auth_token), 'SYNTHETIC-EXTRA-TOKEN');
    await toggle('oci').click();
    const before = await page.evaluate(() => JSON.stringify(fixture.savedOci));
    await module('oci_region').locator('[data-secret-value]').fill('ap-osaka-1');
    await editor.locator('.modal-footer [data-close-modal]').click();
    await editor.waitFor({ state: 'hidden' });
    assert.equal(await page.evaluate(() => JSON.stringify(fixture.savedOci)), before);
    await toggle('oci').click();
    await page.locator('#add-module-button').click();
    await page.locator('#module-menu [data-add-module="username"]').click();
    await page.evaluate(() => { document.querySelector('#toast-region').replaceChildren(); document.querySelector('#oci-bindings').open = false; });
    for (const width of [690, 460]) {
      await page.setViewportSize({ width, height: 1000 });
      for (const language of ['en', 'zh']) {
        await page.locator(`[data-language="${language}"]`).evaluate(button => button.click());
        for (const theme of ['dark', 'light']) {
          if (await page.locator('html').getAttribute('data-theme') !== theme) await page.locator('#theme-toggle').evaluate(button => button.click());
          await page.evaluate(() => { document.documentElement.dataset.textSize = 'large'; });
          assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
          await module('oci_region').scrollIntoViewIfNeeded();
          await page.screenshot({ path: path.join(artifacts, `oci-modules-${width}-${language}-${theme}.png`) });
        }
      }
    }
    await module('fingerprint').locator('[data-remove-module]').click();
    await page.locator('#confirmation-accept').click();
    await save.click();
    await editor.waitFor({ state: 'hidden' });
    assert.equal(await page.evaluate(() => fixture.savedOci.input.modules.some(module => module.name === 'fingerprint')), false);
    assert.ok(await page.evaluate(() => fixture.savedOci.input.ociBindings.fingerprint));
    for (const language of ['en', 'zh']) {
      await page.locator(`[data-language="${language}"]`).click();
      for (const failed of [false, true]) {
        await page.evaluate(failed => { fixture.ociCheckFailure = failed; document.querySelector('#toast-region').replaceChildren(); }, failed);
        await row('oci').locator('[data-action="test"]').click();
        const expected = language === 'zh' ? (failed ? '检查失败' : '检查成功') : (failed ? 'check failed' : 'check successful');
        await page.locator('#toast-region').filter({ hasText: expected }).waitFor();
      }
    }
    await toggle('oci').click();
    await page.evaluate(() => { fixture.deferOciKey = true; delete fixture.resolveOciKey; });
    await chooseSource('clipboard');
    await page.waitForFunction(() => typeof fixture.resolveOciKey === 'function');
    await editor.locator('.modal-footer [data-close-modal]').click();
    await editor.waitFor({ state: 'hidden' });
    await toggle('oci').click();
    assert.equal(await keyRow().locator('[data-stage-key="clipboard"]').isDisabled(), true);
    await page.evaluate(() => { fixture.resolveOciKey(); fixture.deferOciKey = false; });
    await page.waitForFunction(() => fixture.pendingOciKey === null);
    assert.equal(await keyRow().getAttribute('data-pending'), '');
    await chooseSource('clipboard');
    await page.waitForFunction(() => Boolean(document.querySelector('[data-kind="privateKey"]').dataset.pending));
    assert.equal(await page.evaluate(() => fixture.pendingOciKey), await keyRow().getAttribute('data-pending'));
    await editor.locator('.modal-footer [data-close-modal]').click();
    await editor.waitFor({ state: 'hidden' });
    assert.equal(await page.evaluate(() => fixture.drafts.some(draft => JSON.stringify(draft).includes('test-oci-key-'))), false);
    await page.evaluate(async () => { fixture.state.connections.find(item => item.id === 'alpha').httpAuthType = 'basic'; await refresh(false); });
    await toggle('alpha').click();
    assert.equal(await page.locator('#http-auth-type').inputValue(), 'basic');
    await page.locator('#http-auth-type').selectOption('oci-signature');
    await page.locator('#http-auth-type').selectOption('basic');
    assert.equal(await page.locator('#http-auth-type').inputValue(), 'basic');
    assert.deepEqual(errors, []);
    console.log(`OCI standard modules passed: ${artifacts}`);
  } else if (process.argv.includes('--readability')) {
    await page.locator('[data-page="settings"]').click();
    const size = page.locator('#text-size');
    assert.equal(await size.inputValue(), 'standard');
    await size.selectOption('large');
    await page.reload();
    assert.equal(await size.inputValue(), 'large');
    await page.locator('[data-page="connections"]').click();
    for (const language of ['en', 'zh']) {
      await page.locator(`[data-language="${language}"]`).evaluate(button=>button.click());
      for (const [width, height] of [[720, 820], [460, 600]]) {
        await page.setViewportSize({ width, height });
        await page.locator('#add-connection-button').click();
        await page.locator('[data-item-template="ssh"]').click();
        await page.locator('#connection-name').fill('A long synthetic project name for the narrow layout');
        const field = page.locator('#connection-name');
        assert.equal(await field.evaluate(el => getComputedStyle(el).fontSize), '16px');
        const columns = await editor.locator('.identity-grid').evaluate(el => getComputedStyle(el).gridTemplateColumns.split(' ').length);
        assert.equal(columns, width === 460 ? 1 : 2);
        await editor.locator('#add-module-button').click();
        if (await editor.locator('[data-kind="privateKey"]').count() === 0) {
          await page.locator('[data-add-module="privateKey"]').click();
        } else {
          await page.keyboard.press('Escape');
        }
        const keyMenu = editor.locator('.key-source-menu');
        await keyMenu.locator('summary').click();
        const clip = await keyMenu.locator('.key-source-options').boundingBox();
        assert.ok(clip && clip.x >= 0 && clip.x + clip.width <= width);
        await keyMenu.locator('summary').click();
        const save = await page.locator('#save-connection-button').boundingBox();
        assert.ok(save && save.y + save.height <= height);
        assert.equal(await editor.locator('.form-scroll').evaluate(el => el.scrollWidth <= el.clientWidth), true);
        await page.locator('#connection-name').press('Escape');
        await editor.waitFor({ state: 'hidden' });
      }
    }
    assert.deepEqual(errors, []);
    console.log('English and Chinese readability checks passed at 460×600 and 720×820');
  } else if (process.argv.includes('--ssh-keys')) {
    await toggle('beta').click();
    await page.locator('#add-module-button').click();
    await page.locator('[data-add-module="privateKey"]').click();
    const keyRow = page.locator('[data-kind="privateKey"]');
    const keySource = async (name) => {
      await keyRow.locator('summary').click();
      await keyRow.locator(`[data-stage-key="${name}"]`).click();
    };
    await keySource('generate');
    await page.waitForFunction(() => document.querySelector('[data-kind="privateKey"]').dataset.pending === 'staged-key');
    await keyRow.locator('[data-copy-public-key]').click();
    assert.equal(await page.evaluate(() => fixture.publicKeyCopy.pendingId), 'staged-key');
    await keySource('file');
    await keySource('clipboard');
    assert.equal(await keyRow.getAttribute('data-pending'), 'staged-key', 'Cancelled or invalid import retains a valid candidate');
    assert.deepEqual(await page.evaluate(() => fixture.keySources), ['generate', 'file', 'clipboard']);
    await page.locator('#connection-name').press('Escape');
    await editor.waitFor({ state: 'hidden' });
    await page.waitForFunction(() => fixture.discardedKey === 'staged-key');
    assert.equal(await page.evaluate(() => fixture.drafts[0].input.privateKeyImportPath), '');
    assert.equal(await page.evaluate(() => fixture.saved.some(item => item.privateKeyImportPath)), false);
    await toggle('beta').click();
    await keySource('generate');
    await page.waitForFunction(() => document.querySelector('[data-kind="privateKey"]').dataset.pending === 'staged-key');
    await page.locator('#save-connection-button').click();
    await editor.waitFor({ state: 'hidden' });
    assert.equal(await page.evaluate(() => fixture.saved.at(-1).privateKeyImportPath), 'staged-key');
    assert.deepEqual(errors, []);
    console.log('SSH key editor checks passed');
  } else {
  const search = page.locator('#title-search-input');
  const fillSearch = async value => {
    if (!await search.isVisible()) await page.locator('#title-search-toggle').click();
    await search.fill(value);
  };
  const slideFilter = async (selector, from, to, cancel = false, expected = cancel ? from : to) => {
    const strip = page.locator(selector), buttons = strip.locator('button');
    const start = await buttons.nth(from).boundingBox(), end = await buttons.nth(to).boundingBox();
    await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2); await page.mouse.down();
    await page.mouse.move(end.x + end.width / 2, end.y + end.height / 2, { steps: 8 });
    assert.equal(await strip.evaluate(el => el.classList.contains('is-dragging')), true);
    assert.equal(await buttons.nth(from).getAttribute('aria-pressed'), 'true', 'Dragging previews without changing the filter before release');
    if (cancel) { await page.keyboard.press('Escape'); await page.waitForTimeout(60); }
    await page.mouse.up();
    await page.waitForFunction(({ selector, index }) => document.querySelector(selector).querySelectorAll('button')[index].getAttribute('aria-pressed') === 'true' && !document.querySelector(selector).hasAttribute('aria-busy'), { selector, index: expected });
    assert.equal(await strip.evaluate(el => el.style.getPropertyValue('--drag-type')), '');
  };
  const dragSwitch = async (selector, from, to, { count = 2, cancel = false, settle = true } = {}) => {
    const control = page.locator(selector);
    await control.scrollIntoViewIfNeeded();
    const box = await control.boundingBox();
    const inset = await control.evaluate(el => parseFloat(getComputedStyle(el).paddingLeft));
    const cell = (box.width - 2 * inset) / count, y = box.y + box.height / 2;
    await page.mouse.move(box.x + inset + cell * (from + .5), y); await page.mouse.down();
    await page.mouse.move(box.x + inset + cell * (to + .5), y, { steps: 8 });
    assert.equal(await control.evaluate(el => el.classList.contains('is-dragging')), true, `${selector} must drag its cap`);
    if (cancel) { await page.keyboard.press('Escape'); await page.waitForTimeout(60); }
    await page.mouse.up();
    if (settle) await page.waitForFunction(selector => !document.querySelector(selector).hasAttribute('aria-busy'), selector);
  };
  const hover = async locator => {
    await page.mouse.move(0, 0); await locator.scrollIntoViewIfNeeded();
    const face = el => { const s = getComputedStyle(el); return [s.backgroundColor, s.backgroundImage, s.boxShadow, el.offsetWidth, el.offsetHeight]; };
    const before = await locator.evaluate(face);
    await locator.hover(); await page.waitForTimeout(150);
    assert.deepEqual(await locator.evaluate(face), before, 'Hover must only illuminate the ink');
  };
  const headerMeasurements = [];
  const geometryMeasurements = [];
  const stateMeasurements = [];
  const display = await page.context().newCDPSession(page);
  for (const language of ['en', 'zh']) for (const theme of ['dark', 'light']) {
    await page.locator(`[data-language="${language}"]`).evaluate(button=>button.click());
    if (await page.locator('html').getAttribute('data-theme') !== theme) await page.locator('#theme-toggle').evaluate(button=>button.click());
    await page.mouse.move(0, 0); await page.waitForTimeout(170);
    const measure = await page.evaluate(() => {
      const rect = el => el.getBoundingClientRect();
      const center = el => { const r = rect(el); return r.y + r.height / 2; };
      return {
        centers: [...document.querySelectorAll('.title-tools svg, .app-mark-logo, .app-mark strong .lettering-ink')].map(center),
        wells: [...document.querySelectorAll('.title-switch, .language-switch, .window-control-well')].map(el => {
          const style = getComputedStyle(el), cap = el.matches('.title-switch') ? getComputedStyle(el, '::before') : null;
          return { height: rect(el).height, padding: [style.paddingTop, style.paddingRight, style.paddingBottom, style.paddingLeft], cap: cap && [cap.top, cap.left, cap.bottom, cap.width] };
        }),
        colors: [...document.querySelectorAll('.title-switch-option')].map(el => getComputedStyle(el).color),
        inactiveColor: getComputedStyle(document.querySelector('.language-switch button:not(.active)')).color,
        inactiveIcons: [...document.querySelectorAll('.title-switch')].map(el => getComputedStyle(el.querySelector(`[data-state="${el.getAttribute('aria-checked') === 'false'}"]`)).color),
        cards: [...document.querySelectorAll('.connection-card')].map(el => {
          const children = [...el.children].map(rect), outer = rect(el);
          return { height: outer.height, leftmost: children[0].right <= children[1].left, within: children.every(r => r.left >= outer.left && r.right <= outer.right), centers: children.map(r => r.y + r.height / 2) };
        }),
        overflow: document.documentElement.scrollWidth > innerWidth,
        recesses: [...document.querySelectorAll('.title-search, .title-switch, .language-switch, .window-control-well, .nav, .metric-grid, #item-type-switch, .creation-control, .connection-name, .connection-type, .item-disclosure, .card-actions')].map(el => {
          const s = getComputedStyle(el); return { readout: el.matches('.metric-grid'), compact: el.matches('.connection-name, .connection-type, .item-disclosure, .card-actions'), height: rect(el).height, material: [s.backgroundColor, s.boxShadow] };
        }),
      };
    });
    assert.equal(measure.overflow, false);
    assert.equal(new Set(measure.centers).size, 1, 'Header icons and wordmark must share a centerline');
    assert.ok(measure.inactiveIcons.every(color => color === measure.inactiveColor), `Unselected switch icons must use neutral language-button ink: ${JSON.stringify([measure.inactiveIcons, measure.inactiveColor])}`);
    for (const card of measure.cards) {
      assert.equal(card.height, 38); assert.ok(card.leftmost && card.within);
      assert.ok(Math.max(...card.centers) - Math.min(...card.centers) < 1);
    }
    for (const well of measure.wells) {
      assert.equal(well.height, 32); assert.deepEqual(well.padding, ['4px', '4px', '4px', '4px']);
      if (well.cap) assert.deepEqual(well.cap, ['4px', '4px', '4px', '24px']);
    }
    for (const recess of measure.recesses) {
      assert.equal(recess.height, recess.readout ? 48 : recess.compact ? 28 : 32, 'Content controls use the compact density; header and readout retain their spacing');
      assert.deepEqual(recess.material, measure.recesses[0].material, 'Every well shares the same floor and edge profile');
    }
    headerMeasurements.push({ language, theme, ...measure });
    for (const scale of [1, 1.25, 1.5, 2]) {
      await display.send('Emulation.setDeviceMetricsOverride', { width: 460, height: 690, deviceScaleFactor: scale, mobile: false });
      geometryMeasurements.push(await auditControlGeometry(page));
    }
    await display.send('Emulation.setDeviceMetricsOverride', { width: 460, height: 690, deviceScaleFactor: 1.5, mobile: false });
    await hover(toggle('alpha'));
    await hover(row('alpha').locator('.connection-name'));
    await hover(row('alpha').locator('.connection-type'));
    const selectedInk = await page.locator('.nav-item.active').evaluate(el => getComputedStyle(el).color);
    for (const selector of ['.window-control.close', '#add-connection-button', '[data-action="copy-name"]']) {
      const action = page.locator(selector).first(); await action.hover(); await page.waitForTimeout(150);
      assert.equal(await action.evaluate(el => getComputedStyle(el).color), selectedInk, 'Ordinary actions share the selected-state accent');
    }
    const destructive = row('alpha').locator('[data-action="delete"]');
    await destructive.hover(); await page.waitForTimeout(150);
    assert.notEqual(await destructive.evaluate(el => getComputedStyle(el).color), selectedInk, 'Destructive actions retain distinct warning ink');
    await page.mouse.move(0, 0);
    await page.screenshot({ path: path.join(artifacts, `compact-${language}-${theme}.png`) });
    await page.locator('[data-page="settings"]').click();
    await page.locator('[data-settings-filter="all"]').click();
    await page.locator('.settings-layout').evaluate(el => { el.scrollTop = 0; });
    geometryMeasurements.push(await auditControlGeometry(page));
    const settingsIssues = await page.evaluate(() => {
      const issues = [];
      const material = el => { const s = getComputedStyle(el); return [s.backgroundColor, s.boxShadow]; };
      const floor = JSON.stringify(material(document.querySelector('.nav')));
      for (const el of document.querySelectorAll('#page-settings :is(.input, .command-well, .status-strip, .extension-drag-card, details > summary)')) {
        if (!el.getBoundingClientRect().width) continue;
        if (JSON.stringify(material(el)) !== floor) issues.push(`${el.id || el.className}: inconsistent recessed material`);
      }
      for (const card of document.querySelectorAll('.settings-card')) {
        if (card.scrollWidth > card.clientWidth + 1) issues.push(`${card.dataset.module}: horizontal overflow`);
        const r = card.getBoundingClientRect();
        for (const el of card.querySelectorAll('button, input, select, summary, .agent-state, .settings-heading')) {
          const b = el.getBoundingClientRect();
          if (b.width && (b.left < r.left || b.right > r.right)) issues.push(`${el.id || el.className}: outside section`);
        }
      }
      return issues;
    });
    assert.deepEqual(settingsIssues, [], `SYSTEM layout in ${language}/${theme}`);
    await page.screenshot({ path: path.join(artifacts, `system-${language}-${theme}.png`) });
    if ((language === 'en' && theme === 'dark') || (language === 'zh' && theme === 'light')) {
      for (const section of ['MCP', 'APP', 'WEB', 'CLOUD', 'SAFE']) {
        await page.locator(`[data-settings-filter="${section}"]`).click();
        if (section === 'MCP') await page.locator('.manual-mcp-config summary').click();
        if (section === 'WEB') await page.locator('.browser-install-fallback summary').click();
        geometryMeasurements.push(await auditControlGeometry(page));
        await page.mouse.move(0, 0);
        await page.screenshot({ path: path.join(artifacts, `system-${section}-${language}-${theme}.png`) });
        if (section === 'MCP') await page.locator('.manual-mcp-config summary').click();
        if (section === 'WEB') await page.locator('.browser-install-fallback summary').click();
      }
    }
    await page.locator('[data-settings-filter="all"]').click();
    await page.locator('[data-page="connections"]').click();
  }
  assert.notDeepEqual(headerMeasurements[0].colors, headerMeasurements[1].colors, 'Switch icons must follow the theme');
  // A failed historical call must not report healthy current services as failed.
  assert.equal(await page.locator('.lcd-primary .seven-number').getAttribute('aria-label'), '3');
  assert.equal(await page.locator('.lcd-secondary .seven-number').getAttribute('aria-label'), '3');
  assert.deepEqual(await page.locator('.lcd-service strong').allTextContents(), ['READY', 'ON', 'OFF']);
  assert.equal(await page.locator('.lcd-status.fault').count(), 0);
  await page.evaluate(async () => { fixture.state.connections[1].enabled = false; await refresh(false); });
  assert.equal(await page.locator('.lcd-primary .seven-number').getAttribute('aria-label'), '3');
  assert.equal(await page.locator('.lcd-secondary .seven-number').getAttribute('aria-label'), '2');
  await page.evaluate(async () => { fixture.state.connections[1].enabled = true; await refresh(false); });
  for (const [status, paired, connected, expected] of [
    ['listening', false, false, 'PAIR'], ['listening', true, false, 'WAIT'],
    ['off', true, false, 'OFF'], ['error', true, false, 'ERROR'],
  ]) {
    await page.evaluate(async values => {
      Object.assign(fixture.state.browserBridge, values); await refresh(false);
    }, { status, paired, connected });
    assert.equal(await page.locator('.lcd-service').nth(1).locator('strong').textContent(), expected);
    assert.equal(await page.locator('.lcd-service').first().locator('strong').textContent(), 'READY');
  }
  await page.evaluate(async () => {
    Object.assign(fixture.state.browserBridge, { status: 'listening', paired: true, connected: true });
    await refresh(false);
  });
  // All travelling caps share drag, cancel and single-commit behavior.
  await dragSwitch('#theme-toggle', 1, 0);
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'dark');
  await dragSwitch('#theme-toggle', 0, .3);
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'dark', 'Returning to the same gear must not toggle it');
  await dragSwitch('#theme-toggle', 0, 1, { cancel: true });
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'dark');
  await dragSwitch('#sound-toggle', 0, 1);
  assert.equal(await page.locator('#sound-toggle').getAttribute('aria-checked'), 'true');
  assert.deepEqual(await page.evaluate(() => JSON.parse(localStorage.getItem('openkeylet.appearance.v1'))), { theme: 'dark', sound: true });
  await page.reload(); await page.locator('.connection-card').first().waitFor();
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'dark');
  assert.equal(await page.locator('#sound-toggle').getAttribute('aria-checked'), 'true');
  await dragSwitch('#sound-toggle', 1, 0);
  await dragSwitch('#theme-toggle', 0, 1);
  await dragSwitch('#language-switch', 0, 1);
  assert.equal(await page.locator('html').getAttribute('lang'), 'zh-CN');
  await dragSwitch('#language-switch', 1, 0);
  assert.equal(await page.locator('html').getAttribute('lang'), 'en');
  assert.equal(await page.evaluate(() => fixture.state.settings.language), 'en');
  await dragSwitch('.nav', 0, 2, { count: 3 });
  assert.equal(await page.locator('.page.active').getAttribute('id'), 'page-settings');
  geometryMeasurements.push(await auditControlGeometry(page));
  stateMeasurements.push(await auditControlStates(page));
  await dragSwitch('.switch:has(#desktop-shortcut-enabled)', 0, 1);
  assert.equal(await page.locator('#desktop-shortcut-enabled').isChecked(), true);
  assert.equal(await page.evaluate(() => fixture.integrationWrites.length), 1, 'A drag must not also fire the label click');
  await dragSwitch('.switch:has(#desktop-shortcut-enabled)', 1, 0);
  assert.equal(await page.locator('#desktop-shortcut-enabled').isChecked(), false);
  await page.evaluate(() => fixture.failIntegration = true);
  await dragSwitch('.switch:has(#launch-at-login-enabled)', 0, 1);
  assert.equal(await page.locator('#launch-at-login-enabled').isChecked(), false, 'A failed save restores the switch');
  await page.evaluate(() => fixture.failIntegration = false);
  await dragSwitch('.switch:has(#launch-at-login-enabled)', 0, 1);
  await dragSwitch('.switch:has(#launch-at-login-enabled)', 1, 0);
  await dragSwitch('.switch:has(#browser-enabled)', 1, 0);
  assert.equal(await page.evaluate(() => fixture.state.settings.browserEnabled), false);
  await dragSwitch('.switch:has(#browser-enabled)', 0, 1);
  assert.equal(await page.evaluate(() => fixture.state.settings.browserEnabled), true);
  await page.evaluate(async () => { fixture.owner.pinConfigured = true; await refreshOwnerLock(); renderSettings(); });
  await dragSwitch('.switch:has(#pin-enabled)', 1, 0, { settle: false });
  await page.locator('#confirmation-modal').waitFor({ state: 'visible' });
  assert.equal(await page.evaluate(() => fixture.pinDisables), 0, 'PIN drag retains confirmation before removal');
  await page.locator('#confirmation-cancel').click();
  await page.waitForFunction(() => !document.querySelector('#pin-enabled').closest('.switch').hasAttribute('aria-busy'));
  assert.equal(await page.locator('#pin-enabled').isChecked(), true);
  await dragSwitch('.switch:has(#pin-enabled)', 1, 0, { settle: false });
  await page.locator('#confirmation-accept').click();
  await page.waitForFunction(() => !document.querySelector('#pin-enabled').closest('.switch').hasAttribute('aria-busy'));
  assert.equal(await page.evaluate(() => fixture.pinDisables), 1);
  assert.equal(await page.locator('#pin-enabled').isChecked(), false);
  await dragSwitch('.nav', 2, 1, { count: 3 });
  assert.equal(await page.locator('.page.active').getAttribute('id'), 'page-activity');
  await dragSwitch('.nav', 1, 0, { count: 3 });
  await dragSwitch('.creation-options', 0, 1);
  assert.equal(await editor.isHidden(), true, 'The unavailable draft gear springs back');
  assert.equal(await page.locator('#add-connection-button').getAttribute('aria-pressed'), 'true');
  await page.locator('#add-connection-button').click();
  geometryMeasurements.push(await auditControlGeometry(page));
  await page.locator('#connection-name').fill('Switch draft');
  await page.locator('#connection-name').press('Escape'); await editor.waitFor({ state: 'hidden' });
  geometryMeasurements.push(await auditControlGeometry(page));
  await dragSwitch('.creation-options', 0, 1);
  assert.equal(await page.locator('#connection-name').inputValue(), 'Switch draft');
  await page.locator('#connection-name').press('Escape'); await editor.waitFor({ state: 'hidden' });
  await dragSwitch('.creation-options', 1, 0);
  assert.equal(await page.locator('#connection-name').inputValue(), '');
  await page.locator('#connection-name').press('Escape'); await editor.waitFor({ state: 'hidden' });
  await page.evaluate(async () => { fixture.drafts = []; await refreshDrafts(); document.querySelectorAll('.toast').forEach(el => el.remove()); });
  assert.equal(await page.locator('.connection-address, .connection-description, .module-strip, .card-actions [data-action="edit"]').count(), 0);
  assert.deepEqual(await page.locator('.connection-type').evaluateAll(elements => elements.map(el => el.dataset.itemType)), ['api', 'ssh', 'login']);
  assert.equal(await page.locator('.connection-type svg').count(), 3);
  assert.deepEqual(await page.locator('.connection-type').allTextContents(), ['', '', '']);
  assert.equal(await search.isHidden(), true, 'Search starts as a header icon');
  await slideFilter('#item-type-switch', 0, 3, true);
  assert.equal(await page.locator('.connection-card').count(), 3);
  await slideFilter('#item-type-switch', 0, 1);
  assert.deepEqual(await page.locator('.connection-card').evaluateAll(rows => rows.map(row => row.dataset.itemId)), ['beta']);
  await slideFilter('#item-type-switch', 1, 0);
  await page.locator('[data-item-type-filter="ssh"]').click();
  assert.deepEqual(await page.locator('.connection-card').evaluateAll(rows => rows.map(row => row.dataset.itemId)), ['beta']);
  await page.locator('[data-item-type-filter="login"]').click();
  assert.deepEqual(await page.locator('.connection-card').evaluateAll(rows => rows.map(row => row.dataset.itemId)), ['gamma']);
  await page.locator('[data-item-type-filter="api"]').click();
  assert.deepEqual(await page.locator('.connection-card').evaluateAll(rows => rows.map(row => row.dataset.itemId)), ['alpha']);
  await page.locator('[data-item-type-filter="api"]').press('Home');
  assert.equal(await page.locator('.connection-card').count(), 3);
  // All three pages share the header search and independent filter strips.
  await page.locator('[data-page="activity"]').click();
  assert.equal(await search.getAttribute('data-page-search'), 'activity');
  assert.equal(await page.locator('#page-activity .page-search').count(), 0);
  await slideFilter('#activity-filter-switch', 0, 2);
  assert.match(await page.locator('.activity-title').innerText(), /success/);
  await slideFilter('#activity-filter-switch', 2, 0);
  await page.locator('[data-activity-filter="error"]').click();
  assert.equal(await page.locator('.activity-item').count(), 1);
  assert.match(await page.locator('.activity-title').innerText(), /failure/);
  await page.locator('[data-activity-filter="success"]').click();
  assert.match(await page.locator('.activity-title').innerText(), /success/);
  await page.locator('[data-activity-filter="all"]').click();
  await fillSearch('failure');
  assert.equal(await page.locator('.activity-item').count(), 1);
  await search.press('Escape');
  assert.equal(await page.locator('.activity-item').count(), 2);
  geometryMeasurements.push(await auditControlGeometry(page));
  await page.screenshot({ path: path.join(artifacts, 'activity-filter-strip.png') });
  await page.locator('[data-page="settings"]').click();
  assert.equal(await search.getAttribute('data-page-search'), 'settings');
  assert.equal(await page.locator('#page-settings .page-search').count(), 0);
  await slideFilter('#settings-filter-switch', 0, 2);
  assert.equal(await page.locator('.settings-card:visible').count(), 1);
  await slideFilter('#settings-filter-switch', 2, 0);
  await page.locator('[data-settings-filter="MCP"]').click();
  assert.equal(await page.locator('.settings-card:visible').count(), 1);
  assert.equal(await page.locator('.settings-card:visible').getAttribute('data-module'), 'MCP / 02');
  await fillSearch('Agent');
  await page.locator('[data-page="activity"]').click();
  await page.locator('#title-search-toggle').click();
  assert.equal(await search.inputValue(), '');
  await page.locator('[data-page="settings"]').click();
  await page.locator('#title-search-toggle').click();
  assert.equal(await search.inputValue(), 'Agent');
  await search.press('Escape');
  await page.locator('[data-settings-filter="all"]').click();
  await page.screenshot({ path: path.join(artifacts, 'settings-filter-strip.png') });
  await page.locator('[data-page="connections"]').click();
  const order = () => page.locator('.connection-card').evaluateAll(rows => rows.map(row => row.dataset.itemId));
  const dragItem = async (from, target, after, release = true) => {
    const handle = row(from).locator('.connection-name');
    await handle.scrollIntoViewIfNeeded();
    const start = await handle.boundingBox(), end = await row(target).boundingBox();
    await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2);
    await page.mouse.down();
    await page.mouse.move(start.x + start.width / 2, after ? end.y + end.height - 1 : end.y + 1, { steps: 8 });
    if (release) await page.mouse.up();
  };
  await dragItem('alpha', 'gamma', true, false);
  assert.equal(await page.locator('.item-drag-preview').count(), 1);
  assert.equal(await page.locator('.item-drop-after').count(), 1);
  await page.screenshot({ path: path.join(artifacts, 'name-drag-feedback.png') });
  await page.mouse.up();
  await page.waitForFunction(() => fixture.orders.length === 1 && !document.querySelector('#connections-list').hasAttribute('aria-busy'));
  assert.deepEqual(await order(), ['beta', 'gamma', 'alpha']);
  await row('alpha').locator('.connection-name').press('Home');
  await page.waitForFunction(() => fixture.orders.length === 2 && !document.querySelector('#connections-list').hasAttribute('aria-busy'));
  assert.deepEqual(await order(), ['alpha', 'beta', 'gamma']);
  await dragItem('alpha', 'gamma', true, false); await page.keyboard.press('Escape'); await page.mouse.up();
  assert.equal(await page.evaluate(() => fixture.orders.length), 2);
  assert.equal(await page.locator('.item-drag-preview').count(), 0);
  await page.evaluate(() => fixture.failOrder = true);
  await dragItem('alpha', 'gamma', true);
  await page.waitForFunction(() => fixture.orderRequests === 3 && !document.querySelector('#connections-list').hasAttribute('aria-busy'));
  assert.deepEqual(await order(), ['alpha', 'beta', 'gamma'], 'A failed reorder must preserve the saved order');
  await page.evaluate(async () => {
    fixture.failOrder = false;
    fixture.state.connections.push({ ...structuredClone(fixture.state.connections[0]), id: 'delta', name: 'Second synthetic API' });
    await refresh();
  });
  await page.locator('[data-item-type-filter="api"]').click();
  await dragItem('delta', 'alpha', false);
  await page.waitForFunction(() => fixture.orders.length === 3 && !document.querySelector('#connections-list').hasAttribute('aria-busy'));
  assert.deepEqual(await page.evaluate(() => fixture.state.connections.map(i => i.id)), ['delta', 'beta', 'gamma', 'alpha'], 'Filtered dragging preserves the hidden item slots');
  await page.locator('[data-item-type-filter="all"]').click();
  await page.evaluate(async () => {
    fixture.state.connections = ['alpha', 'beta', 'gamma'].map(id => fixture.state.connections.find(i => i.id === id));
    await refresh();
  });
  // The pointer can cross a long list without releasing the name handle.
  await page.evaluate(async () => {
    fixture.state.connections.push(...Array.from({ length: 20 }, (_, index) => ({ ...structuredClone(fixture.state.connections[0]), id: `scroll-${index}`, name: `Scroll item ${index}` })));
    await refresh();
  });
  const beforeScrollDrag = await page.evaluate(() => fixture.orderRequests);
  const scrollHandle = await row('alpha').locator('.connection-name').boundingBox();
  const scrollList = await page.locator('#connections-list').boundingBox();
  await page.mouse.move(scrollHandle.x + 20, scrollHandle.y + 12); await page.mouse.down();
  await page.mouse.move(scrollHandle.x + 20, scrollList.y + scrollList.height - 5, { steps: 8 });
  await page.waitForFunction(() => document.querySelector('#connections-list').scrollTop > 100);
  await page.mouse.move(scrollList.x - 5, scrollList.y + 50);
  assert.equal(await page.locator('.item-drop-before, .item-drop-after').count(), 0);
  await page.mouse.up();
  assert.equal(await page.evaluate(() => fixture.orderRequests), beforeScrollDrag, 'Dropping outside the list cancels the reorder');
  assert.equal(await page.locator('.item-drag-preview').count(), 0);
  await page.evaluate(async () => {
    fixture.state.connections = fixture.state.connections.filter(item => !item.id.startsWith('scroll-'));
    await refresh();
    document.querySelector('#connections-list').scrollTop = 0;
    document.querySelectorAll('.toast').forEach(el => el.remove());
  });
  await fillSearch('server');
  assert.equal(await page.locator('.connection-card').count(), 1);
  geometryMeasurements.push(await auditControlGeometry(page));
  await page.screenshot({ path: path.join(artifacts, 'expanded-header-search.png') });
  await page.locator('#title-search-toggle').click();
  assert.equal(await search.isHidden(), true);
  assert.equal(await page.locator('.connection-card').count(), 1, 'Collapsing search keeps its query');
  const projectSearch = page.locator('#title-search-input');
  await page.keyboard.press('Control+f');
  assert.equal(await projectSearch.evaluate(el => document.activeElement === el), true);
  assert.equal(await projectSearch.inputValue(), 'server');
  await projectSearch.press('Escape');
  assert.equal(await projectSearch.inputValue(), '');
  assert.equal(await page.locator('.connection-card').count(), 3);
  await page.locator('[data-language="en"]').click();
  await toggle('alpha').click();
  await page.waitForFunction(() => document.querySelector('#connection-modal').classList.contains('is-folding'));
  const opening = await editor.evaluate(el => {
    const animation = el.getAnimations()[0]; animation.pause(); animation.currentTime = 110;
    return { height: el.getBoundingClientRect().height, full: el.querySelector('.modal-card').getBoundingClientRect().height };
  });
  assert.ok(opening.height > 0 && opening.height < opening.full, 'The dropdown reveals its contents progressively');
  await editor.evaluate(el => el.getAnimations()[0].play());
  await page.waitForFunction(() => document.activeElement.id === 'connection-name');
  const moduleSwitch = '#connection-modal [data-kind="password"] .module-agent-toggle';
  await dragSwitch(moduleSwitch, 0, 1);
  assert.equal(await page.locator(moduleSwitch).getAttribute('aria-pressed'), 'true');
  assert.equal(await editor.locator('[data-kind="password"] [data-module-visibility]').innerText(), 'AGENT VISIBLE');
  assert.equal(await page.evaluate(() => currentModules.find(module => module.kind === 'password').agentVisible), true);
  await dragSwitch(moduleSwitch, 1, 0);
  assert.equal(await page.locator(moduleSwitch).getAttribute('aria-pressed'), 'false');
  assert.equal(await editor.locator('[data-kind="password"] [data-module-visibility]').innerText(), 'AGENT HIDDEN');
  await page.locator(moduleSwitch).press('Space');
  assert.equal(await editor.locator('[data-kind="password"] [data-module-visibility]').innerText(), 'AGENT VISIBLE');
  await page.locator(moduleSwitch).press('Enter');
  assert.equal(await editor.locator('[data-kind="password"] [data-module-visibility]').innerText(), 'AGENT HIDDEN');
  stateMeasurements.push(await auditControlStates(page));
  geometryMeasurements.push(await auditControlGeometry(page));
  assert.deepEqual(await editor.locator('.modal-card, .module-row, .module-actions, .module-add-wrap, .login-websites').evaluateAll(els => els.map(el => getComputedStyle(el).boxShadow)), Array(await editor.locator('.modal-card, .module-row, .module-actions, .module-add-wrap, .login-websites').count()).fill('none'), 'Expanded groups do not nest recessed trays');
  await toggle('alpha').click();
  await page.waitForFunction(() => document.querySelector('#connection-modal').classList.contains('is-folding'));
  const closing = await editor.evaluate(el => { const animation = el.getAnimations()[0]; animation.pause(); animation.currentTime = 110; return el.getBoundingClientRect().height; });
  assert.ok(closing > 0 && closing < opening.full, 'Collapsing animates before detaching the editor');
  await editor.evaluate(el => el.getAnimations()[0].play()); await editor.waitFor({ state: 'hidden' });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await toggle('alpha').click(); await page.waitForFunction(() => document.activeElement.id === 'connection-name');
  assert.equal(await editor.evaluate(el => el.getAnimations().length), 0);
  await toggle('alpha').click(); await editor.waitFor({ state: 'hidden' });
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await toggle('alpha').click();
  await page.waitForFunction(() => document.activeElement.id === 'connection-name');
  assert.equal(await editor.getAttribute('role'), 'region');
  assert.equal(await editor.getAttribute('aria-modal'), null);
  assert.equal(await editor.evaluate(el => el.parentElement.dataset.itemId), 'alpha');
  assert.equal(await page.locator('.modal-backdrop:not(.inline-editor):not(.hidden)').count(), 0);
  await page.locator('#connection-name').fill('Edited synthetic service');
  await page.locator('#connection-description').fill('Keep this unsaved value');
  await row('alpha').locator('.connection-name').press('ArrowDown');
  await page.waitForFunction(() => !document.querySelector('#connections-list').hasAttribute('aria-busy'));
  assert.deepEqual(await order(), ['beta', 'alpha', 'gamma']);
  assert.equal(await page.locator('#connection-description').inputValue(), 'Keep this unsaved value');
  await row('alpha').locator('.connection-name').press('Home');
  await page.waitForFunction(() => !document.querySelector('#connections-list').hasAttribute('aria-busy'));
  await page.evaluate(async () => { fixture.state.connections[1].name = 'Refreshed server'; await refresh(); });
  assert.equal(await page.locator('#connection-name').inputValue(), 'Edited synthetic service');
  await page.locator('[data-language="zh"]').click();
  assert.equal(await page.locator('#connection-description').inputValue(), 'Keep this unsaved value');
  await fillSearch('no matching service');
  assert.equal(await page.locator('.connection-card').count(), 1, 'Keep the edited row while filtering');
  await fillSearch('');
  await toggle('alpha').click(); await editor.waitFor({ state: 'hidden' });
  assert.equal(await page.evaluate(() => fixture.drafts[0].input.name), 'Edited synthetic service');
  await toggle('alpha').click();
  assert.equal(await page.locator('#connection-name').inputValue(), 'Edited synthetic service');
  await page.locator('#connection-folder').fill('Unsaved folder');
  const itemSwitch = '.switch:has(#connection-enabled)';
  await dragSwitch(itemSwitch, 1, 0);
  assert.equal(await page.locator('#connection-enabled').isChecked(), false);
  await page.locator('#connection-enabled').press('Space');
  assert.equal(await page.locator('#connection-enabled').isChecked(), true);
  await page.locator('#connection-enabled-label').click();
  assert.equal(await page.locator('#connection-enabled').isChecked(), false);
  await toggle('beta').click();
  await page.waitForFunction(() => document.querySelector('#connection-modal').parentElement.dataset.itemId === 'beta' && !document.querySelector('#connection-modal').classList.contains('is-folding'));
  assert.equal(await editor.evaluate(el => el.parentElement.dataset.itemId), 'beta');
  assert.equal(await toggle('alpha').getAttribute('aria-expanded'), 'false');
  assert.equal(await page.evaluate(() => fixture.drafts.find(d => d.input.id === 'alpha').input.folder), 'Unsaved folder');
  assert.equal(await page.evaluate(() => fixture.drafts.find(d => d.input.id === 'alpha').input.enabled), false);
  await toggle('alpha').click();
  await page.waitForFunction(() => document.activeElement.id === 'connection-name');
  assert.equal(await page.locator('#connection-enabled').isChecked(), false, 'Restoring a draft retains the item switch');
  await page.locator('#save-connection-button').focus(); await page.keyboard.press('Tab');
  assert.equal(await page.evaluate(() => document.activeElement.dataset.id), 'beta', `Inline editor must not trap Tab: ${await page.evaluate(() => document.activeElement.outerHTML)}`);
  await page.locator('[data-generate-password]').click();
  await page.waitForFunction(() => document.querySelector('#generator-result').value.length === 24);
  await page.locator('#generator-use').click();
  assert.equal((await page.locator('[data-kind="password"] [data-secret-value]').inputValue()).length, 24);
  await page.locator('#add-module-button').click(); await page.keyboard.press('Escape');
  assert.equal(await page.locator('#module-menu').isHidden(), true);
  assert.equal(await editor.isVisible(), true);
  await page.locator('#add-module-button').click(); await page.locator('[data-add-module="username"]').click();
  assert.equal(await page.locator('[data-kind="username"]').count(), 1);
  for (const theme of ['dark', 'light']) {
    if (await page.locator('html').getAttribute('data-theme') !== theme) await page.locator('#theme-toggle').evaluate(button=>button.click());
    await page.waitForTimeout(180);
    stateMeasurements.push(await auditControlStates(page));
    geometryMeasurements.push(await auditControlGeometry(page));
    await page.locator(moduleSwitch).evaluate(el => { el.disabled = true; });
    try { stateMeasurements.push(await auditControlStates(page)); }
    finally { await page.locator(moduleSwitch).evaluate(el => { el.disabled = false; }); }
    await page.evaluate(() => document.querySelectorAll('.toast').forEach(el => el.remove()));
    await page.locator('#connections-list').evaluate(el => el.scrollTop = 0);
    await page.mouse.move(0, 0); await page.screenshot({ path: path.join(artifacts, `inline-${theme}.png`) });
    assert.equal(await editor.evaluate(el => el.scrollWidth > el.clientWidth), false);
  }
  await page.locator('#save-connection-button').click(); await editor.waitFor({ state: 'hidden' });
  assert.equal(await row('alpha').locator('.connection-name').innerText(), 'Edited synthetic service');
  assert.equal(await page.evaluate(() => fixture.drafts.some(d => d.input.id === 'alpha')), false);
  assert.equal(await page.evaluate(() => fixture.saved[0].secrets.password.length), 24);
  assert.equal(await page.evaluate(() => fixture.saved[0].enabled), false);
  assert.equal(await page.evaluate(() => fixture.saved[0].favorite), true, 'Removing the editor control must preserve existing favorite metadata');
  await toggle('alpha').click(); await page.locator('#connection-name').fill('Draft across navigation');
  await page.evaluate(() => fixture.failDraft = true);
  await toggle('beta').click();
  assert.equal(await editor.evaluate(el => el.parentElement.dataset.itemId), 'alpha', 'A failed draft write must retain the active form');
  assert.equal(await page.locator('#connection-name').inputValue(), 'Draft across navigation');
  await slideFilter('#item-type-switch', 0, 1, false, 0);
  assert.equal(await editor.evaluate(el => el.parentElement.dataset.itemId), 'alpha', 'A failed draft save restores the selected filter and retains the form');
  await page.evaluate(() => fixture.failDraft = false);
  await slideFilter('#item-type-switch', 0, 1);
  await editor.waitFor({ state: 'hidden' });
  assert.deepEqual(await order(), ['beta']);
  assert.equal(await page.evaluate(() => fixture.drafts.find(d => d.input.id === 'alpha').input.name), 'Draft across navigation');
  await slideFilter('#item-type-switch', 1, 0);
  await toggle('alpha').click(); await page.waitForFunction(() => document.activeElement.id === 'connection-name');
  await page.locator('[data-page="settings"]').click(); await editor.waitFor({ state: 'hidden' });
  await page.locator('[data-page="connections"]').click(); await toggle('alpha').click();
  assert.equal(await page.locator('#connection-name').inputValue(), 'Draft across navigation');
  await page.locator('#connection-name').press('Escape'); await editor.waitFor({ state: 'hidden' });
  await toggle('alpha').click();
  await row('alpha').locator('[data-action="delete"]').click(); await page.locator('#confirmation-accept').click();
  await row('alpha').waitFor({ state: 'detached' });
  assert.equal(await editor.count(), 1, 'Removing an expanded row must preserve the editor DOM');
  assert.equal(await editor.isHidden(), true);
  await page.locator('#add-connection-button').click();
  assert.equal(await editor.getAttribute('role'), 'dialog');
  await page.locator('[data-item-template="login"]').click(); await page.locator('[data-generate-password]').click();
  await page.locator('#generator-use').click();
  assert.equal(await page.locator('#generator-modal').isHidden(), true, 'Nested dialogs must stay above the restored new-item editor');
  assert.deepEqual(errors, []);
  await fs.writeFile(path.join(artifacts, 'measurements.json'), JSON.stringify(headerMeasurements, null, 2));
  await fs.writeFile(path.join(artifacts, 'control-geometry.json'), JSON.stringify(geometryMeasurements, null, 2));
  await fs.writeFile(path.join(artifacts, 'control-states.json'), JSON.stringify(stateMeasurements, null, 2));
  console.log(JSON.stringify({ ok: true, checks: ['shared well colours, depths and row heights in both themes', 'neutral inactive icons and glow-only hover', 'draggable theme, sound, language, navigation and add/draft controls', 'settings and module switches commit once and recover from failures', 'PIN drag preserves confirmation and cancellation', 'appearance drag preferences survive reload', 'three-page draggable filter strips with snapping and cancellation', 'animated inline expansion, collapse and reduced motion', 'flat expanded groups without nested trays', 'filter dragging preserves drafts and recovers from write failures', 'pointer and keyboard sorting with filtered slots', 'drag cancellation, edge scrolling and save failure', 'refresh and search preserve edits', 'untrapped keyboard focus', 'generator and module menu', 'save, navigation and deletion', 'new-item dialog restoration'], artifacts }, null, 2));
  }
} finally { await browser.close(); server.close(); }
