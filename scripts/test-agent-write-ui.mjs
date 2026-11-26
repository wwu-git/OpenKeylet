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
    const state = {settings:{language:"zh",closeBehavior:"tray",browserEnabled:true,browserPort:39272},folders:[],connections:[],activities:[],mcp:{status:"ready",endpoint:"fixture",stdioCommand:"fixture"},browserBridge:{status:"listening",enabled:true,connected:true,paired:true,endpoint:"fixture"},security:{encrypted:true,storage:"fixture"}};
    window.fixture = {saved:[],drafts:[],exports:[],copies:[],imports:[],trash:[],settingsWrites:0,history:[{password:"synthetic-previous-password",changedAt:"2026-01-01T00:00:00Z"}],state};
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
      if(command==="update_settings") {
        fixture.settingsWrites++;
        const {expectedAgentWriteFolders,...patch}=args.patch;
        if(patch.agentWriteFolders&&JSON.stringify(expectedAgentWriteFolders)!==JSON.stringify(state.settings.agentWriteFolders||[]))throw Error('Stale folder grant list');
        Object.assign(state.settings,patch);return state.settings;
      }
      if(command==="manage_folder") {
        const {operation,folder,newName}=args;
        if(operation==='create')state.folders.push(folder);
        else {
          state.folders=state.folders.flatMap(name=>name!==folder?[name]:operation==='rename'?[newName]:[]);
          state.settings.agentWriteFolders=(state.settings.agentWriteFolders||[]).flatMap(name=>name!==folder?[name]:operation==='rename'?[newName]:[]);
          for(const item of state.connections)if(item.folder===folder)item.folder=operation==='rename'?newName:'';
        }
      }
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
  const selector = page.locator('#agent-write-folder');
  const addFolder = page.locator('#agent-write-add');
  assert.equal(await selector.evaluate(element=>element.tagName),'SELECT');
  assert.equal(await selector.isDisabled(),true);
  assert.equal(await addFolder.isDisabled(),true);
  await page.evaluate(async()=>{fixture.state.folders=['Agent 管理','开发'];await refresh();});
  assert.equal(await selector.isEnabled(),true);
  assert.equal(await addFolder.isDisabled(),true);
  await selector.selectOption('Agent 管理');
  await addFolder.click();
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
  assert.equal(await addFolder.isDisabled(),true);
  await selector.selectOption('开发');
  await addFolder.click();
  await page.waitForFunction(()=>fixture.state.settings.agentWriteFolders.length===1);
  assert.equal(await selector.locator('option[value="开发"]').count(),0);
  const writes = await page.evaluate(()=>fixture.settingsWrites);
  for(const name of ['不存在','开发']) {
    await selector.evaluate((element,value)=>element.add(new Option(value,value)),name);
    await selector.selectOption(name);
    await addFolder.click();
    assert.equal(await page.evaluate(()=>fixture.settingsWrites),writes);
  }
  assert.deepEqual(await page.evaluate(()=>fixture.state.settings.agentWriteFolders),['开发']);
  await page.locator('[data-page="connections"]').click();
  const sidebar = page.locator('#project-sidebar');
  const folders = page.locator('#project-folders');
  const folderMenu = folders.getByRole('menu');
  const moreFor = name => folders.locator(`[data-project-folder-menu="folder:${name}"]`);
  const folderRow = name => folders.locator(`[data-project-folder="folder:${name}"]`);
  const dialog = page.locator('#project-folder-dialog');
  const addNew = async name => {
    await sidebar.locator('[data-folder-action="create"]').click();
    await dialog.locator('#project-folder-name').fill(name);
    await dialog.locator('[type="submit"]').click();
    await folders.locator(`[data-project-folder="folder:${name}"]`).waitFor();
  };
  await addNew('空文件夹');
  await page.locator('[data-page="settings"]').click();
  await selector.selectOption('空文件夹');
  await addFolder.click();
  await page.waitForFunction(()=>fixture.state.settings.agentWriteFolders.includes('空文件夹'));
  await page.evaluate(async()=>{
    fixture.state.connections.push({id:'folder-member',name:'合成项目',folder:'空文件夹',itemType:'custom',enabled:true,modules:[],capabilities:[],description:'',updatedAt:'fixture'});
    await refresh();
  });
  await page.locator('[data-page="connections"]').click();
  const agentFolder = folderRow('Agent 管理');
  const testFolder = folderRow('空文件夹');
  await agentFolder.click();
  assert.equal(await folderMenu.isHidden(), true);
  assert.equal(await moreFor('Agent 管理').isVisible(), true);
  const beforeMenu = await agentFolder.boundingBox();
  await testFolder.click({button:'right'});
  await folderMenu.waitFor({state:'visible'});
  await folderMenu.getByRole('menuitem', {name:/重命名/}).press('Escape');
  await testFolder.press('Shift+F10');
  await folderMenu.waitFor({state:'visible'});
  assert.equal(await agentFolder.getAttribute('aria-pressed'), 'true');
  assert.deepEqual(await agentFolder.boundingBox(), beforeMenu, 'Opening folder actions leaves the list in place');
  await folderMenu.getByRole('menuitem', {name:/重命名/}).click();
  assert.equal(await dialog.locator('#project-folder-name').inputValue(), '空文件夹');
  assert.deepEqual(await dialog.locator('#project-folder-name').evaluate(el => [el.selectionStart, el.selectionEnd]), [0, 4]);
  await dialog.locator('#project-folder-name').fill('重命名后的空文件夹');
  await dialog.locator('[type="submit"]').click();
  const renamed = folderRow('重命名后的空文件夹');
  await renamed.waitFor();
  assert.equal(await agentFolder.getAttribute('aria-pressed'), 'true', 'Renaming another folder keeps the current filter');
  await renamed.click();
  const more = moreFor('重命名后的空文件夹');
  await more.press('Enter');
  const rename = folderMenu.getByRole('menuitem', {name:/重命名/});
  const remove = folderMenu.getByRole('menuitem', {name:/删除/});
  assert.equal(await rename.evaluate(el => document.activeElement === el), true);
  await rename.press('Escape');
  assert.equal(await folderMenu.isHidden(), true);
  assert.equal(await more.evaluate(el => document.activeElement === el), true);
  await page.waitForFunction(() => !document.querySelector('#toast-region .toast'));
  await page.setViewportSize({width:460,height:520});
  for (const theme of ['dark','light']) {
    if (await page.locator('html').getAttribute('data-theme') !== theme) await page.locator('#theme-toggle').click();
    await more.scrollIntoViewIfNeeded();
    await more.click();
    assert.ok(await folderMenu.evaluate(el => {
      const {left, right, top, bottom} = el.getBoundingClientRect();
      return right > left && bottom > top && left >= 0 && right <= innerWidth && top >= 0 && bottom <= innerHeight;
    }));
    await page.screenshot({path:path.join(artifacts,`folder-menu-${theme}.png`)});
    await folders.locator('#project-folder-heading').click();
    assert.equal(await folderMenu.isHidden(), true);
  }
  await more.press('Enter');
  await rename.press('ArrowDown');
  assert.equal(await remove.evaluate(el => document.activeElement === el), true);
  await remove.press('Enter');
  assert.match(await page.locator('#confirmation-message').textContent(), /1 个项目.*未分组.*写权限.*撤销/);
  await page.locator('#confirmation-cancel').click();
  assert.equal(await renamed.isVisible(),true);
  await more.click();
  await remove.click();
  await page.locator('#confirmation-accept').click();
  await renamed.waitFor({state:'detached'});
  const unfiled = folders.locator('[data-project-folder="unfiled"]');
  assert.equal(await unfiled.getAttribute('aria-pressed'), 'true');
  await unfiled.click({button:'right'});
  assert.equal(await folderMenu.isHidden(), true);
  assert.equal(await page.locator('[data-item-id="folder-member"]').count(),1);
  await page.setViewportSize({width:690,height:1000});
  await addNew('重命名后的空文件夹');
  await page.locator('[data-page="settings"]').click();
  assert.equal(await selector.locator('option[value="重命名后的空文件夹"]').count(),1);
  assert.deepEqual(await page.evaluate(()=>fixture.state.settings.agentWriteFolders),['开发']);
  await page.waitForFunction(()=>document.querySelectorAll('#toast-region .toast').length===0);
  await page.locator('#agent-write-panel').screenshot({path:path.join(artifacts,'write-settings.png')});
  assert.deepEqual(errors,[]);
  console.log('Folder actions and Agent write settings UI passed; screenshot:',path.join(artifacts,'write-settings.png'));
} finally {await browser.close();server.close();}
