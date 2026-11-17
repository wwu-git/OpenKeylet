// Isolated renderer fixture: never loads the desktop app or the user's vault.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import http from "node:http";
import { once } from "node:events";
import { chromium } from "playwright";
const renderer = path.resolve(import.meta.dirname,"../renderer");
const artifacts = await fs.mkdtemp(path.join(os.tmpdir(),"openkeylet-write-ui-"));
const server = http.createServer(async(req,res) => {
  const file = path.resolve(renderer,"." + decodeURIComponent(req.url.split("?")[0] === "/" ? "/index.html" : req.url.split("?")[0]));
  if (!file.startsWith(renderer + path.sep)) { res.writeHead(403); res.end(); return; }
  try {
    res.setHeader("Content-Type", file.endsWith(".js") ? "text/javascript" : file.endsWith(".css") ? "text/css" : file.endsWith(".svg") ? "image/svg+xml" : file.endsWith(".png") ? "image/png" : "text/html");
    res.end(await fs.readFile(file));
  } catch { res.writeHead(404); res.end(); }
});
server.listen(0,"127.0.0.1"); await once(server,"listening");
const browser = await chromium.launch({headless:true, channel: process.env.OPENKEYLET_TEST_CHROME || undefined});
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
  await page.locator('[data-page="settings"]').click();
  assert.equal(await page.locator('#agent-write-enabled').count(),1);
  assert.equal(await page.locator('#agent-write-enabled').isChecked(),false);
  await page.locator('#agent-write-folder').fill('Agent 管理');
  await page.locator('#agent-write-add').click();
  await page.waitForFunction(()=>fixture.state.settings.agentWriteFolders?.includes('Agent 管理'));
  await page.locator('label:has(#agent-write-enabled)').click();
  await page.waitForFunction(()=>fixture.state.settings.agentWriteEnabled===true);
  await page.locator('label:has(#agent-write-enabled)').click();
  await page.waitForFunction(()=>fixture.state.settings.agentWriteEnabled===false);
  assert.deepEqual(await page.evaluate(()=>fixture.state.settings.agentWriteFolders),['Agent 管理']);
  await page.locator('label:has(#agent-write-enabled)').click();
  await page.locator('[data-remove-write-folder]').click();
  await page.waitForFunction(()=>fixture.state.settings.agentWriteFolders.length===0);
  assert.match(await page.locator('#agent-write-status').textContent(),/尚未添加/);
  // Reject duplicate/empty entries before submitting settings.
  await page.locator('#agent-write-folder').fill('   ');
  await page.locator('#agent-write-add').click();
  assert.deepEqual(await page.evaluate(()=>fixture.state.settings.agentWriteFolders),[]);
  await page.locator('#agent-write-folder').fill('开发');
  await page.locator('#agent-write-add').click();
  await page.waitForFunction(()=>fixture.state.settings.agentWriteFolders.length===1);
  await page.locator('#agent-write-folder').fill(' 开发 ');
  await page.locator('#agent-write-add').click();
  assert.deepEqual(await page.evaluate(()=>fixture.state.settings.agentWriteFolders),['开发']);
  await page.waitForFunction(()=>document.querySelectorAll('#toast-region .toast').length===0);
  await page.locator('#agent-write-panel').screenshot({path:path.join(artifacts,'write-settings.png')});
  assert.deepEqual(errors,[]);
  console.log('Agent write settings UI passed; screenshot:',path.join(artifacts,'write-settings.png'));
} finally {await browser.close();server.close();}
