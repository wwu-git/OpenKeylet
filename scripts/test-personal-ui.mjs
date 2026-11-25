// Isolated renderer fixture: never loads the desktop app or the user's vault.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import http from "node:http";
import { once } from "node:events";
import { chromium } from "playwright";
const renderer = path.resolve(import.meta.dirname,"../renderer");
const artifacts = await fs.mkdtemp(path.join(os.tmpdir(),"openkeylet-personal-ui-"));
const server = http.createServer(async(req,res) => {
  const file = path.resolve(renderer,"." + decodeURIComponent(req.url.split("?")[0] === "/" ? "/index.html" : req.url.split("?")[0]));
  if (!file.startsWith(renderer + path.sep)) { res.writeHead(403); res.end(); return; }
  try {
    res.setHeader("Content-Type", file.endsWith(".js") ? "text/javascript" : file.endsWith(".css") ? "text/css" : file.endsWith(".svg") ? "image/svg+xml" : file.endsWith(".png") ? "image/png" : "text/html");
    res.end(await fs.readFile(file));
  } catch { res.writeHead(404); res.end(); }
});
server.listen(0,"127.0.0.1"); await once(server,"listening");
const browser = await chromium.launch({headless:true,channel:process.env.OPENKEYLET_TEST_CHROME || undefined});
try {
  const page = await browser.newPage({viewport:{width:690,height:1000}});
  const errors = []; page.on("pageerror",e=>errors.push(e.message));
  await page.addInitScript(() => {
    const state = {settings:{language:"zh",closeBehavior:"tray",browserEnabled:true,browserPort:39272},connections:[],activities:[],mcp:{status:"ready",endpoint:"fixture",stdioCommand:"fixture"},browserBridge:{status:"listening",enabled:true,connected:true,paired:true,endpoint:"fixture"},security:{encrypted:true,storage:"fixture"}};
    window.fixture = {saved:[],drafts:[],exports:[],copies:[],pairings:[],browserOpens:0,imports:[],trash:[],history:[{password:"synthetic-previous-password",changedAt:"2026-01-01T00:00:00Z"}],state};
    window.__TAURI__ = {event:{listen:async()=>()=>{}},core:{invoke:async(command,args={}) => {
      if(command==="get_state") return structuredClone(state);
      if(command==="owner_status") return {pinConfigured:false,unlocked:true};
      if(command==="owner_editor_drafts") return structuredClone(fixture.drafts);
      if(command==="agent_mcp_status") return [];
      if(command==="save_editor_draft") {
        const draft={id:args.draftId||'fixture-draft',input:structuredClone(args.input)};
        fixture.drafts=[draft,...fixture.drafts.filter(d=>d.id!==draft.id)];return draft;
      }
      if(command==="delete_editor_draft") {fixture.drafts=fixture.drafts.filter(d=>d.id!==args.id);return;}
      if(command==="github_backup_status") return {connected:false};
      if(command==="system_integration_status") return {desktopShortcut:false,launchAtLogin:false};
      if(command==="update_settings") {Object.assign(state.settings,args.patch);return state.settings;}
      if(command==="save_connection") {
        if (fixture.failSave) throw Error("Synthetic save failure");
        fixture.saved.push(structuredClone(args.input));
        const item = {...args.input,id:args.input.id || String(fixture.saved.length),updatedAt:new Date().toISOString(),capabilities:[],passkeys:[],secret:{fields:[]},host:args.input.modules.find(m=>m.kind==='host')?.value||'',port:Number(args.input.modules.find(m=>m.kind==='port')?.value||0),baseUrl:args.input.modules.find(m=>m.kind==='url')?.value||'',modules:args.input.modules.map(m=>({...m,secret:!['host','url','port'].includes(m.kind),configured:true,value:['host','url','port'].includes(m.kind)?m.value:""}))};
        const index = state.connections.findIndex(existing=>existing.id===item.id);
        if (index<0) state.connections.push(item); else state.connections[index]=item;
        return item;
      }
      if(command==="generate_password") return "Aa1!".repeat(32).slice(0,args.options.length);
      if(command==="copy_owner_value") {if(fixture.failCopy)throw Error("Copy failed");fixture.copies.push(args.value);return;}
      if(command==="quick_pair_browser") {fixture.pairings.push(args.port);if(fixture.failPairing)throw Error("Pairing failed");state.settings.browserEnabled=true;state.settings.browserPort=args.port;return;}
      if(command==="open_chromium_browser") {fixture.browserOpens++;if(fixture.failBrowser)throw Error("Browser failed");return;}
      if(command==="export_backup") {fixture.exports.push(args.password);return "fixture.mvault";}
      if(command==="prepare_backup_restore") return {id:args.id,preview:{active:[
        {index:0,name:"Synthetic jump",itemType:"ssh"},
        {index:1,name:"Synthetic target",itemType:"ssh",jumpIndex:0},
        {index:2,name:"Synthetic note",itemType:"secureNote",nameConflict:true},
      ],trash:[{index:0,name:"Synthetic deleted item",itemType:"login"}]}};
      if(command==="commit_backup_restore") { (fixture.restores ??= []).push(args.selection); return {added:2,merged:0}; }
      if(command==="cancel_backup_restore") { (fixture.restoreCancels ??= []).push(args.id); return; }
      if(command==="prepare_import") return {id:"fixture-import",preview:{total:4,types:{login:1,card:1,identity:1,secureNote:1},passkeys:1,skippedDeleted:1,warnings:["defaultDomainMatch","passwordHistory"]}};
      if(command==="prepare_curl_import") {
        if(args.command.includes("--data")) throw Error("CURL_IMPORT_UNSUPPORTED");
        const authentication=args.command.includes("Bearer")?"bearer":"basic";
        const id=`curl-${authentication}`;
        fixture.pendingCurl=id;
        return {id,preview:{target:"https://api.example.test",authentication}};
      }
      if(command==="cancel_import") {delete fixture.pendingCurl;return;}
      if(command==="commit_import") {fixture.imports.push(args.id);return {added:4,merged:0};}
      if(command==="prepare_env_import") return {id:args.id,empty:1,multiline:2,rows:[
        {index:0,name:"API_KEY",conflict:false},
        {index:1,name:"KEEP_EXISTING",conflict:true},
        {index:2,name:"OTHER_KEY",conflict:false},
      ]};
      if(command==="cancel_env_import") { (fixture.envCancels ??= []).push(args.id); return; }
      if(command==="commit_env_import") { (fixture.envImports ??= []).push(args.selection); return {added:1,skipped:0,undoId:"env-undo-fixture"}; }
      if(command==="undo_env_import") { fixture.envUndo = args.id; return {removed:1,skipped:0}; }
      if(command==="prepare_ssh_import") return {id:args.id,rows:[
        {index:0,name:"test-jump",host:"jump.example.test",port:22,user:"alice",route:"direct",conflict:false},
        {index:1,name:"test-target",host:"target.example.test",port:2200,user:"bob",route:"jump",jump:"test-jump",jumpIndex:0,conflict:false},
        {index:2,name:"test-missing",host:"missing.example.test",port:22,user:"",route:"unresolved",jump:"unknown",conflict:false},
        {index:3,name:"existing",host:"old.example.test",port:22,user:"",route:"direct",conflict:true},
      ]};
      if(command==="cancel_ssh_import") return;
      if(command==="commit_ssh_import") { fixture.sshImport=args.selection; return {added:2,skipped:0,undoId:"ssh-undo-fixture"}; }
      if(command==="undo_ssh_import") { fixture.sshUndo=args.id; return {removed:2,skipped:0}; }
      if(command==="owner_secret_view") return {fields:[]};
      if(command==="delete_connection") {
        const item=state.connections.find(i=>i.id===args.id);
        if(!item||item.updatedAt!==args.expectedUpdatedAt)throw Error("Stale deletion");
        state.connections=state.connections.filter(i=>i.id!==args.id);fixture.trash.push({item,deletedAt:new Date().toISOString()});return;
      }
      if(command==="owner_trash")return structuredClone(fixture.trash);
      if(command==="owner_restore_item"||command==="owner_purge_item"){
        const entry=fixture.trash.find(e=>e.item.id===args.id&&e.deletedAt===args.deletedAt);
        if(!entry)throw Error("Stale trash token");
        if(command==="owner_restore_item")state.connections.push(entry.item);
        fixture.trash=fixture.trash.filter(e=>e!==entry);return;
      }
      if(command==="owner_password_history")return {updatedAt:state.connections.find(i=>i.id===args.id).updatedAt,entries:structuredClone(fixture.history)};
      if(command==="owner_clear_password_history"){
        const item=state.connections.find(i=>i.id===args.id);if(item.updatedAt!==args.expectedUpdatedAt)throw Error("Stale history");
        item.passwordHistoryCount=0;fixture.history=[];return;
      }
      return {};
    }}};
  });
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  await page.locator("#connections-list .empty-state").waitFor();
  await page.locator('[data-page="settings"]').click();
  const manualConfig = page.locator('.manual-mcp-config');
  await manualConfig.locator('summary').click();
  await page.locator('.manual-config-steps b .lettering-ink').first().waitFor();
  const manualCopy = {
    zh: ['打开目标 Agent 的 MCP 配置', '按配置格式复制 JSON 或 TOML', '粘贴保存并重新启动 Agent'],
    en: ['Open the target agent MCP configuration', 'Copy JSON or TOML to match its config format', 'Paste, save, and restart the agent'],
  };
  for (const width of [690, 460]) {
    await page.setViewportSize({width,height:1000});
    const size = width === 460 ? 'large' : 'standard';
    await page.evaluate(size => {document.documentElement.dataset.textSize=size;}, size);
    for (const theme of ['dark','light']) {
      if (await page.locator('html').getAttribute('data-theme') !== theme) await page.locator('#theme-toggle').click();
      for (const locale of ['en','zh']) {
        await page.locator(`[data-language="${locale}"]`).click();
        await page.locator('.manual-config-steps b .lettering-ink').first().waitFor();
        await manualConfig.screenshot({path:path.join(artifacts,`manual-config-${width}-${size}-${locale}-${theme}.png`)});
        assert.deepEqual(await page.locator('.manual-config-steps > div > b').allTextContents(), ['1','2','3'], 'Manual setup step numbers survive lettering and language switches');
        assert.deepEqual(await page.locator('.manual-config-steps > div > span').allTextContents(), manualCopy[locale]);
        const rows = await page.locator('.manual-config-steps > div').evaluateAll(rows=>rows.map(row=>{
          const bounds = el => {const r=el.getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom};};
          return {row:bounds(row),number:bounds(row.querySelector(':scope > b')),text:bounds(row.querySelector(':scope > span'))};
        }));
        for (const [index,{row,number,text}] of rows.entries()) {
          assert.ok(number.right<=text.left && number.bottom<=row.bottom+1 && text.bottom<=row.bottom+1, JSON.stringify(rows[index]));
          if (index) assert.ok(row.top>=rows[index-1].row.bottom-1, JSON.stringify(rows));
        }
        assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
      }
    }
  }
  await page.evaluate(()=>{document.documentElement.dataset.textSize='standard';});
  await page.setViewportSize({width:690,height:1000});
  await manualConfig.locator('summary').click();
  await page.locator('[data-page="connections"]').click();
  const projectSearch = page.locator('#title-search-input');
  const fillItemSearch = async value => {
    if (!await projectSearch.isVisible()) await page.locator('#title-search-toggle').click();
    await projectSearch.fill(value);
  };
  assert.equal(await page.locator('#page-connections input[type="search"]').count(), 0);
  assert.equal(await page.locator('[data-page-search="connections"]').count(), 1);
  await page.evaluate(async () => {
    const item = { name: 'GitHub', description: '生产', itemType: 'api', enabled: true, folder: 'agent',
      capabilities: ['http'], baseUrl: 'https://api.example.invalid', modules: [{kind: 'url', value: 'https://api.example.invalid'}] };
    fixture.state.connections = [
      { ...item, id: 'exact' },
      { ...item, id: 'nested', folder: 'agent/测试' },
      { ...item, id: 'disabled', enabled: false },
      { ...item, id: 'ssh', itemType: 'ssh' },
      { ...item, id: 'unfiled', folder: '' },
    ];
    await refresh();
  });
  const visible = () => page.locator('.connection-card').evaluateAll(rows => rows.map(row => row.dataset.itemId));
  const filterPanel = page.locator('#connection-filter-panel');
  const openFilters = () => page.keyboard.press('Control+f');
  assert.equal(await page.locator('#connection-filter-toggle, .connection-search-tools').count(), 0);
  assert.equal(await filterPanel.isHidden(), true);
  await openFilters();
  assert.equal(await page.locator('#connection-clear-all').isHidden(), true, 'Clear all stays hidden without active search or filters');
  await page.locator('#title-search-toggle').click();
  const folder = page.locator('#connection-folder-filter');
  const status = page.locator('#connection-status-filter');
  await page.locator('[data-item-type-filter="api"]').click();
  await openFilters();
  await folder.selectOption({label: 'agent'});
  await status.selectOption('enabled');
  assert.equal(await filterPanel.isVisible(), true, 'Selecting filters keeps the shared search panel open');
  await page.locator('#title-search-toggle').click();
  assert.equal(await filterPanel.isHidden(), true);
  assert.match(await page.locator('#title-search-toggle').getAttribute('aria-label'), /筛选中/);
  assert.deepEqual(await visible(), ['exact'], 'Collapsing search preserves folder, status and type filters');
  await fillItemSearch('生产 github');
  assert.deepEqual(await visible(), ['exact']);
  assert.match(await page.locator('#connection-result-count').textContent(), /1\s*\/\s*5/);
  await page.keyboard.press('Control+f');
  assert.equal(await projectSearch.evaluate(el => document.activeElement === el), true);
  assert.deepEqual(await projectSearch.evaluate(el => [el.selectionStart,el.selectionEnd]), [0,'生产 github'.length]);
  await projectSearch.press('Escape');
  assert.equal(await projectSearch.inputValue(), '');
  assert.equal(await filterPanel.isVisible(), true, 'First Escape clears keywords without closing the filter panel');
  assert.deepEqual(await visible(), ['exact'], 'Clearing keywords preserves the active filters');
  await page.locator('[data-clear-item-filter="status"]').click();
  assert.equal(await projectSearch.isVisible(), true);
  assert.equal(await projectSearch.evaluate(el => document.activeElement === el), true);
  assert.deepEqual(await visible(), ['exact', 'disabled']);
  await openFilters();
  await status.selectOption('enabled');
  await openFilters();
  await page.locator('[data-item-id="exact"] .item-disclosure').click();
  await page.locator('#connection-description').fill('未保存的编辑');
  await fillItemSearch('不存在');
  await openFilters();
  await folder.selectOption({label: '未分组'});
  await status.selectOption('disabled');
  await page.locator('#title-search-toggle').click();
  await page.locator('[data-item-type-filter="ssh"]').click();
  assert.equal(await page.locator('#connection-description').inputValue(), '未保存的编辑');
  assert.deepEqual(await visible(), ['exact']);
  assert.match(await page.locator('#connection-result-count').textContent(), /0\s*\/\s*5/);
  await page.locator('[data-item-id="exact"] .item-disclosure').click();
  await page.locator('#connection-modal').waitFor({state: 'hidden'});
  await page.waitForFunction(() => fixture.drafts.some(d => d.input.description === '未保存的编辑'));
  await openFilters();
  await page.locator('#connection-clear-all').click();
  assert.equal(await page.locator('#connection-clear-all').isHidden(), true, 'Clear all hides after resetting every search condition');
  assert.equal(await projectSearch.inputValue(), '');
  assert.equal((await visible()).length, 5);
  await fillItemSearch('github');
  for (const [pageName, query] of [['activity', '独立记录'], ['settings', '安全']]) {
    if (await projectSearch.isVisible()) await page.locator('#title-search-toggle').click();
    await page.locator(`[data-page="${pageName}"]`).click();
    await page.keyboard.press('Control+f');
    assert.equal(await filterPanel.isHidden(), true, 'Project filters stay hidden on other pages');
    assert.equal(await projectSearch.inputValue(), '');
    await projectSearch.fill(query);
  }
  await page.locator('[data-page="connections"]').click();
  await page.keyboard.press('Meta+f');
  assert.equal(await projectSearch.inputValue(), 'github');
  assert.equal(await projectSearch.evaluate(el => document.activeElement === el), true);
  for (const [pageName, query] of [['activity', '独立记录'], ['settings', '安全']]) {
    if (await projectSearch.isVisible()) await page.locator('#title-search-toggle').click();
    await page.locator(`[data-page="${pageName}"]`).click();
    await page.keyboard.press('Control+f');
    assert.equal(await projectSearch.inputValue(), query);
    await projectSearch.press('Escape');
  }
  await page.locator('[data-page="connections"]').click();
  await fillItemSearch('github');
  await folder.selectOption({label:'agent'});
  await status.selectOption('enabled');
  for (const width of [690, 460]) {
    await page.setViewportSize({width,height:1000});
    for (const theme of ['dark', 'light']) {
      if (await page.locator('html').getAttribute('data-theme') !== theme) await page.locator('#theme-toggle').click();
      await page.keyboard.press('Control+f');
      assert.equal(await page.locator('[data-page-search="connections"]').count(), 1);
      assert.equal(await projectSearch.isVisible(), true);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      assert.equal(await filterPanel.isVisible(), true);
      const panelBounds = await filterPanel.evaluate(el => { const rect=el.getBoundingClientRect(); return {left:rect.left,right:rect.right,bottom:rect.bottom,top:rect.top,width:el.clientWidth,scroll:el.scrollWidth}; });
      assert.ok(panelBounds.left>=0 && panelBounds.right<=width && panelBounds.bottom<=1000 && panelBounds.scroll<=panelBounds.width+1, JSON.stringify(panelBounds));
      assert.ok(panelBounds.top >= await projectSearch.evaluate(el=>el.getBoundingClientRect().bottom));
      await page.screenshot({path:path.join(artifacts, `single-search-${width}-${theme}.png`)});
    }
  }
  await page.locator('#connection-clear-all').click();
  await projectSearch.press('Escape');
  await page.setViewportSize({width:690,height:1000});
  await page.evaluate(async () => {
    fixture.state.connections = [{id:'addresses',name:'Multiple addresses',itemType:'custom',enabled:true,folder:'',host:'a.example',port:22,baseUrl:'https://a.example/',updatedAt:'original',modules:[
      {kind:'host',value:'a.example',alternateValues:['b.example'],agentVisible:true},
      {kind:'url',value:'https://a.example/',alternateValues:['https://b.example/'],agentVisible:true},
      {kind:'port',value:'22',agentVisible:true}
    ]}];
    await refresh();
  });
  const hostModule = page.locator('#module-list [data-kind="host"]');
  const urlModule = page.locator('#module-list [data-kind="url"]');
  const addressValues = module => module.locator('[data-module-value], [data-module-alternate-value]').evaluateAll(inputs=>inputs.map(input=>input.value));
  await page.locator('[data-item-id="addresses"] .item-disclosure').click();
  await hostModule.locator('[data-add-address]').click();
  await hostModule.locator('[data-module-alternate-value]').last().fill('c.example');
  await hostModule.locator('[data-default-address="1"]').click();
  assert.deepEqual(await addressValues(hostModule), ['b.example','a.example','c.example']);
  assert.equal(await page.locator('#ssh-trust-probe').isDisabled(), true, 'Changing the draft default disables trust actions for the saved host');
  await hostModule.locator('[data-remove-address="0"]').click();
  assert.deepEqual(await addressValues(hostModule), ['a.example','c.example']);
  await hostModule.locator('[data-default-address="1"]').click();
  await urlModule.locator('[data-add-address]').click();
  await urlModule.locator('[data-module-alternate-value]').last().fill('https://c.example/');
  await urlModule.locator('[data-default-address="1"]').click();
  assert.deepEqual(await addressValues(urlModule), ['https://b.example/','https://a.example/','https://c.example/']);
  await urlModule.locator('[data-remove-address="0"]').click();
  assert.deepEqual(await addressValues(urlModule), ['https://a.example/','https://c.example/']);
  await urlModule.locator('[data-default-address="1"]').click();
  await hostModule.locator('[data-module-agent-visible]').click();
  await hostModule.locator('[data-module-drag-handle]').press('ArrowDown');
  assert.deepEqual(await addressValues(hostModule), ['c.example','a.example']);
  assert.equal(await hostModule.locator('[data-module-agent-visible]').getAttribute('aria-pressed'), 'false');
  await page.locator('[data-language="en"]').click();
  assert.deepEqual(await addressValues(urlModule), ['https://c.example/','https://a.example/']);
  await page.locator('[data-language="zh"]').click();
  for (const width of [690,460]) {
    await page.setViewportSize({width,height:1000});
    for (const theme of ['dark','light']) {
      if (await page.locator('html').getAttribute('data-theme') !== theme) await page.locator('#theme-toggle').click();
      await urlModule.scrollIntoViewIfNeeded();
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
      assert.equal(await page.locator('.module-address-line').evaluateAll(rows=>rows.every(row=>row.getBoundingClientRect().right<=innerWidth && row.scrollWidth<=row.clientWidth+1)),true);
      await page.screenshot({path:path.join(artifacts,`multiple-addresses-${width}-${theme}.png`)});
      await page.locator('#module-list').screenshot({path:path.join(artifacts,`multiple-address-modules-${width}-${theme}.png`)});
    }
  }
  await page.setViewportSize({width:460,height:1000});
  await page.locator('[data-language="en"]').click();
  await page.evaluate(()=>{document.documentElement.dataset.textSize='large';});
  for (const theme of ['dark','light']) {
    if (await page.locator('html').getAttribute('data-theme') !== theme) await page.locator('#theme-toggle').click();
    await urlModule.scrollIntoViewIfNeeded();
    assert.equal(await page.locator('.module-addresses .small-button').evaluateAll(buttons=>buttons.every(button=>button.scrollWidth<=button.clientWidth+1 && button.scrollHeight<=button.clientHeight+1)),true);
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
    await page.screenshot({path:path.join(artifacts,`multiple-addresses-460-large-en-${theme}.png`)});
    await page.locator('#module-list').screenshot({path:path.join(artifacts,`multiple-address-modules-460-large-en-${theme}.png`)});
  }
  await page.evaluate(()=>{document.documentElement.dataset.textSize='standard';});
  await page.locator('[data-language="zh"]').click();
  await page.setViewportSize({width:690,height:1000});
  for (const invalid of ['', 'https://c.example/', 'file:///tmp/test']) {
    await urlModule.locator('[data-module-alternate-value]').fill(invalid);
    await page.locator('#save-connection-button').click();
    assert.equal(await page.locator('#connection-modal').isVisible(),true);
    assert.equal(await page.evaluate(()=>fixture.saved.length),0);
    assert.equal(await page.evaluate(()=>fixture.state.connections[0].updatedAt),'original');
  }
  await urlModule.locator('[data-module-alternate-value]').fill('https://a.example/');
  await page.locator('[data-item-id="addresses"] .item-disclosure').click();
  await page.waitForFunction(()=>fixture.drafts.some(d=>d.input.id==='addresses'));
  await page.locator('#connection-modal').waitFor({state:'hidden'});
  assert.deepEqual(await page.evaluate(()=>fixture.state.connections[0].modules[0].alternateValues),['b.example']);
  await page.locator('#drafts-button').click();
  assert.deepEqual(await addressValues(hostModule),['c.example','a.example']);
  assert.deepEqual(await addressValues(urlModule),['https://c.example/','https://a.example/']);
  assert.equal(await hostModule.locator('[data-module-agent-visible]').getAttribute('aria-pressed'),'false');
  await page.evaluate(()=>{fixture.failSave=true;});
  await page.locator('#save-connection-button').click();
  assert.equal(await page.evaluate(()=>fixture.state.connections[0].updatedAt),'original');
  assert.equal(await page.locator('#connection-modal').isVisible(),true);
  await page.evaluate(()=>{fixture.failSave=false;});
  await page.locator('#save-connection-button').click();
  await page.locator('#connection-modal').waitFor({state:'hidden'});
  await page.locator('[data-item-id="addresses"] .item-disclosure').click();
  assert.deepEqual(await addressValues(hostModule),['c.example','a.example']);
  assert.deepEqual(await addressValues(urlModule),['https://c.example/','https://a.example/']);
  await urlModule.locator('[data-module-alternate-value]').fill('https://only-other-changed.example/');
  await page.locator('#save-connection-button').click();
  await page.locator('#connection-modal').waitFor({state:'hidden'});
  await fillItemSearch('only-other-changed.example');
  assert.deepEqual(await visible(),['addresses']);
  await projectSearch.press('Escape');
  await projectSearch.press('Escape');
  await page.locator('[data-item-id="addresses"] .item-disclosure').click();
  assert.deepEqual(await addressValues(urlModule),['https://c.example/','https://only-other-changed.example/']);
  await hostModule.locator('[data-remove-address="0"]').click();
  await hostModule.locator('[data-remove-address="0"]').click();
  assert.deepEqual(await addressValues(hostModule),[''], 'Removing the final address leaves one editable empty default');
  await page.locator('[data-item-id="addresses"] .item-disclosure').click();
  await page.locator('#connection-modal').waitFor({state:'hidden'});
  await page.evaluate(()=>{fixture.saved=[];});
  await page.evaluate(async () => {
    fixture.state.connections = [];
    fixture.drafts = [];
    await refresh();
    await refreshDrafts();
  });
  assert.equal(await page.locator('#drafts-button').isDisabled(),true);
  // A closed unsaved editor remains recoverable through the shared selector.
  await page.locator('#add-connection-button').click();
  await page.waitForFunction(()=>document.activeElement.id==='connection-name');
  await page.locator('#save-connection-button').focus();
  await page.keyboard.press('Tab');
  assert.equal(await page.evaluate(()=>document.activeElement.matches('#connection-modal .modal-header [data-close-modal]')),true);
  await page.keyboard.press('Shift+Tab');
  assert.equal(await page.evaluate(()=>document.activeElement.id),'save-connection-button');
  await page.locator('[data-item-template="login"]').click();
  await page.locator('#connection-name').fill('Synthetic draft');
  await page.locator('[data-kind="password"] [data-secret-value]').fill('synthetic-draft-password');
  await page.keyboard.press('Escape');
  await page.locator('#connection-modal').waitFor({state:'hidden'});
  assert.equal(await page.evaluate(()=>document.activeElement.id),'add-connection-button');
  assert.equal(await page.locator('#drafts-button').isEnabled(),true);
  await page.locator('#drafts-button').click();
  assert.equal(await page.locator('#connection-name').inputValue(),'Synthetic draft');
  assert.equal(await page.locator('[data-kind="password"] [data-secret-value]').inputValue(),'synthetic-draft-password');
  assert.equal(await page.locator('.creation-control').getAttribute('data-mode'),'draft');
  await page.locator('#connection-modal .modal-header [data-close-modal]').click();
  await page.locator('#connection-modal').waitFor({state:'hidden'});
  await page.locator('#add-connection-button').click();
  assert.equal(await page.locator('#connection-name').inputValue(),'');
  assert.equal(await page.locator('.creation-control').getAttribute('data-mode'),'add');
  await page.locator('#connection-modal .modal-header [data-close-modal]').click();
  await page.locator('#connection-modal').waitFor({state:'hidden'});
  assert.equal(await page.evaluate(()=>fixture.drafts.length),1,'Adding must preserve the saved draft');
  await page.locator("#add-connection-button").click();
  await page.locator('[data-item-template="card"]').click();
  await page.locator("#connection-name").fill("测试信用卡");
  await page.locator("#connection-folder").fill("钱包");
  assert.equal(await page.locator("#connection-enabled").isChecked(),true);
  await page.locator('.switch:has(#connection-enabled)').click();
  await page.locator('[data-kind="cardNumber"] [data-secret-value]').fill("4111111111111111");
  await page.locator('[data-kind="cardSecurityCode"] [data-secret-value]').fill("987");
  await page.screenshot({path:path.join(artifacts,"card-editor.png")});
  await page.locator("#save-connection-button").click();
  await page.locator("#connection-modal").waitFor({state:"hidden"});
  const saved = await page.evaluate(()=>fixture.saved[0]);
  assert.equal(saved.itemType,"card");assert.equal(saved.folder,"钱包");assert.equal(saved.favorite,false);assert.equal(saved.enabled,false);
  assert.equal(saved.secrets.namedSecrets.cardNumber,"4111111111111111");
  assert.ok(saved.modules.every(m=>m.value==="" && m.agentVisible===false));
  await fillItemSearch("不存在的项目");
  assert.equal(await page.locator(".connection-card").count(),0);
  await fillItemSearch("钱包");
  assert.equal(await page.locator(".connection-card").count(),1);
  await page.screenshot({path:path.join(artifacts,"vault-organization.png")});
  await fillItemSearch("");
  await page.locator("#add-connection-button").click();
  await page.locator('[data-item-template="secureNote"]').click();
  await page.locator("#connection-name").fill("测试笔记");
  await page.locator('[data-kind="secureNote"] textarea').fill("第一行\n第二行");
  await page.locator("#save-connection-button").click();
  await page.locator("#connection-modal").waitFor({state:"hidden"});
  assert.equal(await page.evaluate(()=>fixture.saved[1].secrets.namedSecrets.secureNote),"第一行\n第二行");
  await page.locator("#add-connection-button").click();
  await page.locator('[data-item-template="login"]').click();
  await page.locator("[data-generate-password]").click();
  await page.locator("#generator-result").waitFor({state:"visible"});
  await page.waitForFunction(()=>document.querySelector("#generator-result").value.length===24);
  await page.locator("#generator-use").click();
  assert.equal((await page.locator('[data-kind="password"] [data-secret-value]').inputValue()).length,24);
  await page.locator(".login-websites summary").click();
  await page.locator("#login-uri-add").click();
  await page.locator("[data-login-uri]").fill("https://example.com/login");
  await page.locator("[data-login-match]").selectOption("exact");
  assert.deepEqual(await page.evaluate(()=>serializeItem(false).loginUris),[{uri:"https://example.com/login",matchType:"exact"}]);
  await page.evaluate(()=>closeEditor());
  await page.locator('[data-page="settings"]').click();
  await page.locator('#browser-port').fill('39273');
  const pairingCases = [
    {copy:false,pairing:false,browser:false,errors:[]},
    {copy:false,pairing:true,browser:false,errors:['Pairing failed']},
    {copy:false,pairing:false,browser:true,errors:['Browser failed']},
    {copy:true,pairing:false,browser:false,errors:['Copy failed']},
    {copy:true,pairing:true,browser:true,errors:['Copy failed','Pairing failed','Browser failed']},
    {copy:false,pairing:false,browser:false,errors:[]},
  ];
  for (const [index, sample] of pairingCases.entries()) {
    await page.evaluate(sample=>{
      fixture.failCopy=sample.copy;fixture.failPairing=sample.pairing;fixture.failBrowser=sample.browser;
      document.querySelectorAll('.toast').forEach(toast=>toast.remove());
    },sample);
    const previousCopies=await page.evaluate(()=>fixture.copies.length);
    await page.locator('#quick-pairing-button').click();
    await page.waitForFunction(()=>!document.querySelector('#quick-pairing-button').disabled);
    assert.equal(await page.evaluate(()=>fixture.browserOpens),index+1);
    assert.deepEqual(await page.evaluate(()=>fixture.pairings),Array(index+1).fill(39273));
    assert.equal(await page.evaluate(()=>fixture.copies.length),previousCopies+Number(!sample.copy));
    if(!sample.copy)assert.equal(await page.evaluate(()=>fixture.copies.at(-1)),'chrome://extensions/');
    assert.equal(await page.locator('#browser-open-help').isVisible(),!sample.copy);
    assert.deepEqual(await page.locator('#toast-region .toast.error').allTextContents(),sample.errors);
    const messages=await page.locator('#toast-region .toast').allTextContents();
    if(sample.errors.length)assert.ok(!messages.some(message=>/配对已准备|Pairing is ready/.test(message)));
    else assert.ok(messages.some(message=>/配对已准备|Pairing is ready/.test(message)));
  }
  await page.evaluate(()=>document.querySelectorAll('.toast').forEach(toast=>toast.remove()));
  await page.locator("#header-export-backup-button").click();
  await page.locator("#backup-password").fill("synthetic-test-password");
  await page.locator("#backup-password-confirm").fill("synthetic-test-password");
  await page.locator('#backup-password-form button[type="submit"]').click();
  await page.waitForFunction(()=>fixture.exports.length===1);
  assert.equal(await page.evaluate(()=>fixture.exports[0]),"synthetic-test-password");
  assert.equal(await page.locator("#backup-password").inputValue(),"");
  await page.setViewportSize({width:460,height:690});
  async function openRestorePreview() {
    await page.locator("#header-import-backup-button").click();
    await page.locator('#backup-password-form button[type="submit"]').click();
    await page.locator("#backup-restore-modal").waitFor();
  }
  await openRestorePreview();
  const restore = page.locator("#backup-restore-modal");
  const confirmRestore = restore.locator("#backup-restore-confirm");
  assert.equal(await confirmRestore.isDisabled(), true);
  await restore.getByRole("checkbox", { name: /Synthetic target/ }).check();
  assert.equal(await confirmRestore.isDisabled(), true);
  assert.match(await restore.locator('[role="status"]').textContent(), /Synthetic jump/);
  await restore.getByRole("checkbox", { name: /^Synthetic jump/ }).check();
  assert.equal(await confirmRestore.isEnabled(), true);
  await page.screenshot({ path: path.join(artifacts, "backup-restore-preview.png") });
  await restore.locator("#backup-restore-cancel").click();
  await restore.waitFor({ state: "hidden" });
  assert.equal(await page.evaluate(() => fixture.restores?.length ?? 0), 0);
  assert.equal(await page.evaluate(() => fixture.restoreCancels.length), 1);
  await openRestorePreview();
  await restore.getByRole("checkbox", { name: /Synthetic note/ }).check();
  await restore.getByRole("checkbox", { name: /Synthetic deleted item/ }).check();
  await confirmRestore.click();
  await restore.waitFor({ state: "hidden" });
  assert.deepEqual(await page.evaluate(() => fixture.restores), [{ active: [2], trash: [0] }]);
  await page.locator("#migration-open").click();
  await page.locator("#migration-select").click();
  await page.locator("#migration-confirm").waitFor({state:"visible"});
  assert.equal(await page.evaluate(()=>fixture.imports.length),0,"Preview must not import");
  assert.match(await page.locator("#migration-preview").innerText(),/4/);
  await page.screenshot({path:path.join(artifacts,"migration-preview.png")});
  assert.ok(await page.evaluate(()=>document.querySelector("#migration-modal .modal-card").getBoundingClientRect().bottom<=innerHeight));
  await page.locator("#migration-close").click();
  assert.equal(await page.evaluate(()=>fixture.imports.length),0,"Cancel must not import");
  await page.locator("#migration-open").click();
  assert.equal(await page.locator("#migration-confirm").isVisible(),false,"Old preview must be cleared");
  await page.locator("#migration-format").selectOption("chrome");
  await page.locator("#migration-select").click();
  await page.locator("#migration-confirm").click();
  await page.locator("#migration-modal").waitFor({state:"hidden"});
  assert.deepEqual(await page.evaluate(()=>fixture.imports),["fixture-import"]);
  await page.locator("#migration-open").click();
  await page.locator("#migration-format").selectOption("env");
  await page.locator("#migration-select").click();
  const envPreview = page.locator("#migration-preview");
  const conflict = envPreview.getByRole("checkbox",{name:/KEEP_EXISTING/});
  await conflict.waitFor();
  assert.ok(await conflict.isDisabled());
  assert.equal(await conflict.isChecked(),false);
  assert.match(await page.locator("#migration-batch-status").textContent(), /1/);
  assert.match(await page.locator("#migration-batch-status").textContent(), /2/);
  assert.equal(await page.evaluate(()=>fixture.envImports?.length ?? 0),0);
  await page.locator("#migration-close").click();
  assert.equal(await page.evaluate(()=>fixture.envCancels.length),1);
  await page.locator("#migration-open").click();
  await page.locator("#migration-select").click();
  await envPreview.getByRole("checkbox",{name:/OTHER_KEY/}).uncheck();
  await page.locator("#migration-confirm").click();
  await page.locator("#migration-batch-undo").click();
  await page.locator("#migration-batch-undo").waitFor({state:"hidden"});
  assert.deepEqual(await page.evaluate(()=>fixture.envImports),[[0]]);
  assert.equal(await page.evaluate(()=>fixture.envUndo),"env-undo-fixture");
  await page.locator("#migration-close").click();
  await page.locator("#migration-open").click();
  await page.locator("#migration-format").selectOption("ssh");
  await page.locator("#migration-select").click();
  const sshTarget = envPreview.locator("label").filter({hasText:"test-target"});
  await sshTarget.waitFor();
  assert.match(await sshTarget.innerText(), /bob@target\.example\.test · 2200/);
  assert.match(await sshTarget.innerText(), /通过跳板 test-jump/);
  await envPreview.locator("label").filter({hasText:"test-jump"}).filter({hasText:"jump.example.test"}).getByRole("checkbox").uncheck();
  assert.match(await sshTarget.innerText(), /连接受阻/);
  assert.ok(await envPreview.getByRole("checkbox",{name:/existing/}).isDisabled());
  assert.equal(await page.evaluate(()=>fixture.sshImport),undefined);
  await page.locator("#migration-confirm").click();
  await page.locator("#migration-batch-undo").click();
  await page.locator("#migration-batch-undo").waitFor({state:"hidden"});
  assert.deepEqual(await page.evaluate(()=>fixture.sshImport),[1,2]);
  assert.equal(await page.evaluate(()=>fixture.sshUndo),"ssh-undo-fixture");
  await page.locator("#migration-close").click();
  const externalImportRequests=[];
  page.on("request",request=>{
    if(!request.url().startsWith(`http://127.0.0.1:${server.address().port}/`))externalImportRequests.push(request.url());
  });
  await page.locator("#migration-open").click();
  await page.locator("#migration-format").selectOption("curl");
  const curlInput=page.locator("#migration-curl");
  const beforeCurl=await page.evaluate(()=>fixture.imports.length);
  assert.ok(await page.locator("#migration-select").isDisabled());
  await curlInput.fill("curl 'https://api.example.test/v1' -H 'Authorization: Bearer synthetic-curl-token'");
  await page.locator("#migration-select").click();
  await envPreview.waitFor();
  assert.equal(await curlInput.inputValue(),"");
  assert.match(await envPreview.innerText(),/https:\/\/api\.example\.test\s+Bearer/);
  assert.doesNotMatch(await envPreview.innerText(),/synthetic-curl-token/);
  await page.screenshot({path:path.join(artifacts,"curl-import-preview.png")});
  assert.ok(await curlInput.isHidden());
  await page.locator("#migration-select").click();
  await page.waitForFunction(()=>!fixture.pendingCurl);
  assert.ok(await envPreview.isHidden());
  await curlInput.fill("curl 'https://api.example.test/v1' -H 'Authorization: Bearer synthetic-curl-token'");
  await page.locator("#migration-select").click();
  await envPreview.waitFor();
  await page.locator("#migration-close").click();
  await page.waitForFunction(()=>!fixture.pendingCurl);
  assert.equal(await page.evaluate(()=>fixture.imports.length),beforeCurl);

  await page.locator("#migration-open").click();
  await curlInput.fill("curl 'https://api.example.test/v1' --data 'synthetic-body'");
  await page.locator("#migration-select").click();
  await page.locator("#migration-error").waitFor();
  assert.doesNotMatch(await page.locator("#migration-error").innerText(),/synthetic-body/);
  assert.equal(await curlInput.inputValue(),"");
  await curlInput.fill("curl -u 'synthetic-user:synthetic-password' https://api.example.test/v1");
  await page.locator("#migration-select").click();
  await envPreview.waitFor();
  assert.match(await envPreview.innerText(),/Basic/);
  assert.doesNotMatch(await envPreview.innerText(),/synthetic-user|synthetic-password/);
  await page.locator("#migration-confirm").click();
  await page.locator("#migration-modal").waitFor({state:"hidden"});
  assert.deepEqual(await page.evaluate(start=>fixture.imports.slice(start),beforeCurl),["curl-basic"]);
  assert.deepEqual(externalImportRequests,[]);
  await page.locator('[data-page="connections"]').click();
  await page.setViewportSize({width:460,height:690});
  await fillItemSearch("");
  await page.evaluate(async()=>{
    fixture.state.connections[0].passwordHistoryCount=1;await refresh();
  });
  await page.locator('[data-action="history"]').click();
  const previous=page.locator("#vault-management-list input");
  assert.equal(await previous.inputValue(),"synthetic-previous-password");
  assert.equal(await previous.getAttribute("type"),"password");
  await page.screenshot({path:path.join(artifacts,"password-history.png")});
  await page.locator("#history-clear").click();
  await page.locator("#confirmation-cancel").click();
  assert.equal(await page.evaluate(()=>fixture.history.length),1);
  await page.locator("#history-clear").click();await page.locator("#confirmation-accept").click();
  await page.waitForFunction(()=>fixture.history.length===0);
  await page.locator("#vault-management-close").click();
  assert.equal(await previous.count(),0);
  await page.locator('[data-action="delete"]').first().click();await page.locator("#confirmation-accept").click();
  await page.waitForFunction(()=>fixture.trash.length===1);
  await page.locator('[data-page="settings"]').click();
  await page.locator("#trash-open").click();
  await page.screenshot({path:path.join(artifacts,"trash.png")});
  await page.locator('[data-trash-action="restore"]').click();
  await page.waitForFunction(()=>fixture.trash.length===0);
  await page.locator("#vault-management-close").click();
  await page.locator('[data-page="connections"]').click();
  await page.locator('[data-action="delete"]').first().click();await page.locator("#confirmation-accept").click();
  await page.waitForFunction(()=>fixture.trash.length===1);
  await page.locator('[data-page="settings"]').click();
  await page.locator("#trash-open").click();
  await page.locator('[data-trash-action="purge"]').click();await page.locator("#confirmation-cancel").click();
  assert.equal(await page.evaluate(()=>fixture.trash.length),1);
  await page.locator('[data-trash-action="purge"]').click();await page.locator("#confirmation-accept").click();
  await page.waitForFunction(()=>fixture.trash.length===0);
  await page.locator("#vault-management-close").click();
  await page.locator('[data-page="connections"]').click();
  await page.setViewportSize({width:460,height:520});
  await page.locator('#toast-region .toast').last().waitFor({state:'hidden'});
  await openFilters();
  assert.ok(await page.locator('#connection-filter-panel').evaluate(el => {
    const rect = el.getBoundingClientRect();
    return rect.left >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight;
  }));
  assert.equal(await projectSearch.isVisible(), true);
  await page.screenshot({path:path.join(artifacts,'narrow-filter-panel.png')});
  await projectSearch.press('Escape');
  assert.equal(await page.locator('#connection-filter-panel').isHidden(), true);
  await page.keyboard.press('Control+f');
  assert.ok(await projectSearch.evaluate(el => el.getBoundingClientRect().width >= 100));
  await page.screenshot({path:path.join(artifacts,"narrow-vault.png")});
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  await page.locator("#add-connection-button").click();
  await page.locator('[data-item-template="card"]').click();
  await page.screenshot({path:path.join(artifacts,"narrow-card-editor.png")});
  assert.ok(await page.evaluate(()=>document.querySelector("#connection-modal .modal-card").getBoundingClientRect().bottom<=innerHeight));
  assert.deepEqual(errors,[]);
  console.log(JSON.stringify({ok:true,checks:["combined add/draft selector","draft preservation and recovery","card editor","encrypted field payload","one header search and filter panel with Ctrl/Cmd+F and independent page queries","multiple host and URL addresses, default switching, removal, drafts, failed save and address search","combined keyword, type, exact folder and enabled status filters","unsaved edits survive filtering and are saved as drafts","460px filter controls","multiline secure note","generator inserts password","browser address copy, pairing, launch failures and recovery","backup password flow","multiple website rules","migration preview/cancel/commit","offline cURL preview, cancel, rejection and confirmation","password history","trash restore and purge from settings","narrow layout"],artifacts},null,2));
} finally {await browser.close();server.close();}
