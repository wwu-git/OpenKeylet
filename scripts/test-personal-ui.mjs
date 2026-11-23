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
    window.fixture = {saved:[],drafts:[],exports:[],copies:[],imports:[],trash:[],history:[{password:"synthetic-previous-password",changedAt:"2026-01-01T00:00:00Z"}],state};
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
        fixture.saved.push(structuredClone(args.input));
        const item = {...args.input,id:String(fixture.saved.length),updatedAt:new Date().toISOString(),capabilities:[],passkeys:[],secret:{fields:[]},modules:args.input.modules.map(m=>({...m,secret:true,configured:true,value:""}))};
        state.connections.push(item);return item;
      }
      if(command==="generate_password") return "Aa1!".repeat(32).slice(0,args.options.length);
      if(command==="copy_owner_value") {fixture.copies.push(args.value);return;}
      if(command==="export_backup") {fixture.exports.push(args.password);return "fixture.mvault";}
      if(command==="import_backup") return {added:0,merged:0};
      if(command==="prepare_import") return {id:"fixture-import",preview:{total:4,types:{login:1,card:1,identity:1,secureNote:1},passkeys:1,skippedDeleted:1,warnings:["defaultDomainMatch","passwordHistory"]}};
      if(command==="commit_import") {fixture.imports.push(args.id);return {added:4,merged:0};}
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
  const filters = page.locator('#connection-filter-toggle');
  const folder = page.locator('#connection-folder-filter');
  const status = page.locator('#connection-status-filter');
  await page.locator('[data-item-type-filter="api"]').click();
  await filters.click();
  await folder.selectOption({label: 'agent'});
  await status.selectOption('enabled');
  await filters.click();
  await fillItemSearch('生产 github');
  assert.deepEqual(await visible(), ['exact']);
  assert.match(await page.locator('#connection-result-count').textContent(), /1\s*\/\s*5/);
  await page.keyboard.press('Control+f');
  assert.equal(await projectSearch.evaluate(el => document.activeElement === el), true);
  assert.deepEqual(await projectSearch.evaluate(el => [el.selectionStart,el.selectionEnd]), [0,'生产 github'.length]);
  await projectSearch.press('Escape');
  assert.equal(await projectSearch.inputValue(), '');
  assert.deepEqual(await visible(), ['exact'], 'Clearing keywords preserves the active filters');
  await page.locator('[data-clear-item-filter="status"]').click();
  assert.equal(await projectSearch.isVisible(), true);
  assert.equal(await projectSearch.evaluate(el => document.activeElement === el), true);
  assert.deepEqual(await visible(), ['exact', 'disabled']);
  await filters.click();
  await status.selectOption('enabled');
  await filters.click();
  await page.locator('[data-item-id="exact"] .item-disclosure').click();
  await page.locator('#connection-description').fill('未保存的编辑');
  await fillItemSearch('不存在');
  await filters.click();
  await folder.selectOption({label: '未分组'});
  await status.selectOption('disabled');
  await page.locator('[data-item-type-filter="ssh"]').click();
  assert.equal(await page.locator('#connection-description').inputValue(), '未保存的编辑');
  assert.deepEqual(await visible(), ['exact']);
  assert.match(await page.locator('#connection-result-count').textContent(), /0\s*\/\s*5/);
  await page.locator('[data-item-id="exact"] .item-disclosure').click();
  await page.locator('#connection-modal').waitFor({state: 'hidden'});
  await page.waitForFunction(() => fixture.drafts.some(d => d.input.description === '未保存的编辑'));
  await page.locator('#connection-clear-all').click();
  assert.equal(await projectSearch.inputValue(), '');
  assert.equal((await visible()).length, 5);
  await fillItemSearch('github');
  for (const [pageName, query] of [['activity', '独立记录'], ['settings', '安全']]) {
    await page.locator(`[data-page="${pageName}"]`).click();
    await page.keyboard.press('Control+f');
    assert.equal(await projectSearch.inputValue(), '');
    await projectSearch.fill(query);
  }
  await page.locator('[data-page="connections"]').click();
  await page.keyboard.press('Meta+f');
  assert.equal(await projectSearch.inputValue(), 'github');
  assert.equal(await projectSearch.evaluate(el => document.activeElement === el), true);
  for (const [pageName, query] of [['activity', '独立记录'], ['settings', '安全']]) {
    await page.locator(`[data-page="${pageName}"]`).click();
    await page.keyboard.press('Control+f');
    assert.equal(await projectSearch.inputValue(), query);
    await projectSearch.press('Escape');
  }
  await page.locator('[data-page="connections"]').click();
  await fillItemSearch('github');
  for (const width of [690, 460]) {
    await page.setViewportSize({width,height:1000});
    for (const theme of ['dark', 'light']) {
      if (await page.locator('html').getAttribute('data-theme') !== theme) await page.locator('#theme-toggle').click();
      await page.keyboard.press('Control+f');
      assert.equal(await page.locator('[data-page-search="connections"]').count(), 1);
      assert.equal(await projectSearch.isVisible(), true);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      const toolbar = await page.locator('.connection-search-tools').evaluate(el => ({width:el.clientWidth,scroll:el.scrollWidth}));
      assert.ok(toolbar.scroll <= toolbar.width + 1, JSON.stringify(toolbar));
      await page.screenshot({path:path.join(artifacts, `single-search-${width}-${theme}.png`)});
    }
  }
  await projectSearch.press('Escape');
  await page.setViewportSize({width:690,height:1000});
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
  await page.locator("#header-export-backup-button").click();
  await page.locator("#backup-password").fill("synthetic-test-password");
  await page.locator("#backup-password-confirm").fill("synthetic-test-password");
  await page.locator('#backup-password-form button[type="submit"]').click();
  await page.waitForFunction(()=>fixture.exports.length===1);
  assert.equal(await page.evaluate(()=>fixture.exports[0]),"synthetic-test-password");
  assert.equal(await page.locator("#backup-password").inputValue(),"");
  await page.setViewportSize({width:460,height:690});
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
  await filters.click();
  assert.ok(await page.locator('#connection-filter-panel').evaluate(el => {
    const rect = el.getBoundingClientRect();
    return rect.left >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight;
  }));
  assert.equal(await projectSearch.isHidden(), true);
  await page.screenshot({path:path.join(artifacts,'narrow-filter-panel.png')});
  await filters.press('Escape');
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
  console.log(JSON.stringify({ok:true,checks:["combined add/draft selector","draft preservation and recovery","card editor","encrypted field payload","one header search with Ctrl/Cmd+F and independent page queries","combined keyword, type, exact folder and enabled status filters","unsaved edits survive filtering and are saved as drafts","460px filter controls","multiline secure note","generator inserts password","backup password flow","multiple website rules","migration preview/cancel/commit","password history","trash restore and purge from settings","narrow layout"],artifacts},null,2));
} finally {await browser.close();server.close();}
