// Synthetic renderer fixture only: no GitHub account or real vault is accessed.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import http from "node:http";
import {once} from "node:events";
import {chromium} from "playwright";
const renderer=path.resolve(import.meta.dirname,"../renderer");
const artifacts=await fs.mkdtemp(path.join(os.tmpdir(),"openkeylet-github-ui-"));
const server=http.createServer(async(req,res)=>{
  const file=path.resolve(renderer,"."+(req.url==="/"?"/index.html":req.url.split("?")[0]));
  if(!file.startsWith(renderer+path.sep)){res.writeHead(403);res.end();return;}
  try{res.setHeader("Content-Type",file.endsWith(".js")?"text/javascript":file.endsWith(".css")?"text/css":file.endsWith(".png")?"image/png":"text/html");res.end(await fs.readFile(file));}
  catch{res.writeHead(404);res.end();}
});
server.listen(0,"127.0.0.1");await once(server,"listening");
const browser=await chromium.launch({headless:true,executablePath:process.env.OPENKEYLET_TEST_CHROMIUM || undefined});
try{
  const page=await browser.newPage({viewport:{width:460,height:690}});
  const errors=[];page.on("pageerror",error=>errors.push(error.message));
  await page.addInitScript(()=>{
    const state={settings:{language:"zh",closeBehavior:"tray",browserEnabled:true,browserPort:39272},connections:[],activities:[],mcp:{status:"ready"},browserBridge:{status:"listening"},security:{encrypted:true}};
    window.fixture={status:{connected:false},connects:[],uploads:0,restores:0,restoreAttempts:[],previews:0,revision:"a",failUpload:false,failConnect:false,unlocked:true};
    window.__TAURI__={event:{listen:async()=>()=>{}},core:{invoke:async(command,args={})=>{
      if(command==="get_state")return structuredClone(state);
      if(command==="owner_status")return {pinConfigured:!fixture.unlocked,unlocked:fixture.unlocked};
      if(command==="owner_editor_drafts"||command==="agent_mcp_status")return [];
      if(command==="github_backup_status")return structuredClone(fixture.status);
      if(command==="github_backup_account")return {login:"fixture-user"};
      if(command==="github_backup_discover"){
        (fixture.discoveryCalls ??= []).push(structuredClone(args.input));
        if(fixture.deferDiscovery)await new Promise(resolve=>{fixture.finishDiscovery=resolve;});
        return fixture.discoveryResult ?? {repositories:[
          {repository:"fixture-user/archive-vault",branch:"backup"},
          {repository:"fixture-user/openkeylet-vault",branch:"main"},
        ],partial:true};
      }
      if(command==="github_backup_generate_key")return "okv1_"+"A".repeat(43);
      if(command==="github_backup_connect"){
        if(fixture.failConnect)throw Error("GITHUB_PRIVATE_REQUIRED");
        fixture.connects.push(structuredClone(args.input));
        fixture.status={connected:true,repository:args.input.repository,branch:"main",hasRemoteBackup:args.input.mode==="existing",needsRestore:args.input.mode==="existing"};
        return structuredClone(fixture.status);
      }
      if(command==="github_backup_upload"){
        if(fixture.failUpload)throw Error("GITHUB_NETWORK");
        fixture.uploads++;fixture.status.hasRemoteBackup=true;fixture.status.lastBackupAt="2026-10-20T00:00:00Z";return structuredClone(fixture.status);
      }
      if(command==="github_backup_preview_restore"){
        fixture.previews++;
        const revision=fixture.revision;
        return {
          snapshot:{repositoryId:"7",branch:"main",sha:revision.repeat(40)},
          createdAt:"2026-10-20T00:00:00Z",
          items:{active:[{name:revision==="a"?"<b>Shared login</b>":"Updated login",itemType:"login",folder:"Work",nameConflict:revision==="a"}],
            trash:[{name:"Old login",itemType:"constructor",folder:"",nameConflict:false}]},
        };
      }
      if(command==="github_backup_restore"){
        fixture.restoreAttempts.push(structuredClone(args.snapshot));
        if(args.snapshot?.sha!==fixture.revision.repeat(40))throw Error("GITHUB_RESTORE_PREVIEW_STALE");
        fixture.restores++;fixture.status.needsRestore=false;fixture.status.lastRestoreAt="2026-10-20T00:00:00Z";
        return {status:structuredClone(fixture.status),summary:{added:2,merged:1}};
      }
      if(command==="github_backup_disconnect"){fixture.status={connected:false};return structuredClone(fixture.status);}
      return {};
    }}};
  });
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  await page.locator('[data-page="settings"]').click();
  await page.locator("#github-connect").click();
  await page.locator("#github-check-account").click();
  await page.waitForFunction(()=>document.querySelector("#github-account-status").textContent.includes("fixture-user"));
  assert.equal(await page.evaluate(()=>fixture.discoveryCalls?.length ?? 0),0);
  assert.ok(await page.locator("#github-discover").isHidden());
  assert.equal(await page.locator("#github-repository").inputValue(),"fixture-user/openkeylet-vault");
  assert.equal(await page.locator("#github-token").isVisible(),false);
  await page.screenshot({path:path.join(artifacts,"account.png")});
  await page.locator("#github-create-repository").check();
  await page.locator("#github-generate-key").click();
  await page.waitForFunction(()=>document.querySelector("#github-recovery-key").value.startsWith("okv1_"));
  await page.locator("#github-submit").click();
  assert.equal(await page.evaluate(()=>fixture.connects.length),0,"Saving the key must be acknowledged");
  await page.locator("#github-key-saved").check();
  await page.screenshot({path:path.join(artifacts,"setup.png")});
  assert.ok(await page.evaluate(()=>document.querySelector(".github-modal-card").getBoundingClientRect().bottom<=innerHeight));
  await page.locator("#github-submit").click();
  await page.waitForFunction(()=>fixture.uploads===1);
  assert.equal(await page.evaluate(()=>fixture.connects[0].authSource),"cli");
  assert.equal(await page.evaluate(()=>fixture.connects[0].createRepository),true);
  assert.equal(await page.locator("#github-recovery-key").inputValue(),"");
  await page.screenshot({path:path.join(artifacts,"uploaded.png")});
  await page.locator("#github-reconfigure").click();
  assert.equal(await page.locator("#github-mode").inputValue(),"existing");
  assert.equal(await page.locator("#github-first-upload-field").isVisible(),false);
  await page.locator("#github-auth-source").selectOption("token");
  await page.locator("#github-token").fill("synthetic-pat");
  await page.locator("#github-recovery-key").fill("okv1_"+"A".repeat(43));
  await page.locator("#github-discover").click();
  await page.locator("#github-discovery-results").waitFor({state:"visible"});
  assert.deepEqual(await page.evaluate(()=>fixture.discoveryCalls[0]),{authSource:"token",token:"synthetic-pat"});
  assert.match(await page.locator("#github-discovery-status").textContent(),/未覆盖全部/);
  await page.screenshot({path:path.join(artifacts,"discovery.png")});
  await page.locator("#github-discovery-results").selectOption("fixture-user/archive-vault");
  assert.equal(await page.locator("#github-repository").inputValue(),"fixture-user/archive-vault");
  assert.equal(await page.locator("#github-recovery-key").inputValue(),"okv1_"+"A".repeat(43));
  assert.deepEqual(await page.evaluate(()=>[fixture.connects.length,fixture.uploads,fixture.restores]),[1,1,0]);
  await page.locator("#github-auth-source").selectOption("cli");
  assert.ok(await page.locator("#github-discovery-results").isHidden());
  await page.evaluate(()=>fixture.discoveryResult={repositories:[],partial:false});
  await page.locator("#github-discover").click();
  await page.waitForFunction(()=>document.querySelector("#github-discovery-status").textContent.includes("未找到"));
  assert.ok(await page.locator("#github-discovery-results").isHidden());
  assert.ok(await page.locator("#github-repository").isEnabled());
  await page.evaluate(()=>delete fixture.discoveryResult);
  await page.locator("#github-auth-source").selectOption("token");
  await page.locator("#github-token").fill("synthetic-pat");
  await page.locator("#github-key-saved").check();
  await page.locator("#github-submit").click();
  await page.locator("#github-modal").waitFor({state:"hidden"});
  assert.equal(await page.evaluate(()=>fixture.uploads),1,"Existing backups must not be uploaded over");
  assert.equal(await page.locator("#github-upload").isDisabled(),true);
  assert.equal(await page.locator("#github-token").inputValue(),"");
  await page.locator("#github-restore").click();
  const preview=page.locator("#confirmation-details");
  await preview.waitFor({state:"visible"});
  assert.match(await page.locator("#confirmation-message").textContent(),/2026.*共 2 项/);
  assert.match(await preview.textContent(),/<b>Shared login<\/b>.*Work.*本机已有同名项目.*废纸篓.*Old login/);
  assert.equal(await preview.locator("b").count(),0);
  assert.ok(await page.locator("#confirmation-modal .confirm-icon").isHidden());
  await page.screenshot({path:path.join(artifacts,"restore-preview.png")});
  await page.locator("#confirmation-cancel").click();
  assert.equal(await preview.textContent(),"");
  assert.equal(await page.evaluate(()=>fixture.restoreAttempts.length),0);
  assert.ok(await page.locator("#github-upload").isDisabled());
  assert.ok(await page.locator("#github-restore").evaluate(button=>button===document.activeElement));

  await page.locator("#github-restore").click();
  await preview.waitFor({state:"visible"});
  await page.evaluate(()=>fixture.revision="b");
  await page.locator("#confirmation-accept").click();
  await page.waitForFunction(()=>document.querySelector("#github-error").textContent.includes("重新预览"));
  assert.equal(await page.evaluate(()=>fixture.restores),0);
  assert.ok(await page.locator("#github-upload").isDisabled());
  assert.equal(await preview.textContent(),"");

  await page.locator("#github-restore").click();
  await preview.waitFor({state:"visible"});
  assert.match(await preview.textContent(),/Updated login/);
  await page.locator("#confirmation-accept").click();
  await page.waitForFunction(()=>fixture.restores===1);
  await page.waitForFunction(()=>!document.querySelector("#github-upload").disabled);
  assert.deepEqual(await page.evaluate(()=>fixture.restoreAttempts),[
    {repositoryId:"7",branch:"main",sha:"a".repeat(40)},
    {repositoryId:"7",branch:"main",sha:"b".repeat(40)},
  ]);
  assert.equal(await page.evaluate(()=>fixture.previews),3);
  await page.screenshot({path:path.join(artifacts,"restored.png")});
  await page.locator("#github-disconnect").click();await page.locator("#confirmation-accept").click();
  await page.locator("#github-connect").waitFor({state:"visible"});
  await page.locator("#github-connect").click();
  await page.locator("#github-repository").fill("fixture-user/another-vault");
  await page.locator("#github-generate-key").click();
  await page.waitForFunction(()=>document.querySelector("#github-recovery-key").value.startsWith("okv1_"));
  await page.locator("#github-key-saved").check();
  await page.evaluate(()=>fixture.failConnect=true);
  await page.locator("#github-submit").click();
  await page.waitForFunction(()=>document.querySelector("#github-form-error").textContent.includes("私有"));
  assert.equal(await page.evaluate(()=>fixture.connects.length),2);
  await page.evaluate(()=>{fixture.failConnect=false;fixture.failUpload=true;});
  await page.locator("#github-submit").click();
  await page.locator("#github-modal").waitFor({state:"hidden"});
  await page.locator("#github-error").waitFor({state:"visible"});
  assert.equal(await page.evaluate(()=>fixture.status.connected),true,"Upload failure must retain the connection");
  await page.evaluate(()=>fixture.failUpload=false);
  await page.locator("#github-upload").click();await page.locator("#confirmation-accept").click();
  await page.waitForFunction(()=>fixture.uploads===2);
  await page.locator("#github-reconfigure").click();
  await page.locator("#github-auth-source").selectOption("token");
  await page.locator("#github-mode").selectOption("existing");
  await page.locator("#github-token").fill("synthetic-private-value");
  await page.evaluate(()=>fixture.deferDiscovery=true);
  await page.locator("#github-discover").click();
  await page.waitForFunction(()=>typeof fixture.finishDiscovery==="function");
  await page.locator("#github-modal-close").click();
  await page.locator("#github-modal").waitFor({state:"hidden"});
  await page.evaluate(()=>{fixture.finishDiscovery();delete fixture.finishDiscovery;});
  await page.locator("#github-reconfigure").click();
  assert.equal(await page.locator("#github-discovery-results option").count(),0);
  await page.locator("#github-auth-source").selectOption("token");
  await page.locator("#github-mode").selectOption("existing");
  await page.locator("#github-token").fill("synthetic-private-value");
  await page.locator("#github-discover").click();
  await page.waitForFunction(()=>typeof fixture.finishDiscovery==="function");
  await page.evaluate(async()=>{fixture.unlocked=false;await refreshOwnerLock();});
  await page.evaluate(()=>fixture.finishDiscovery());
  await page.waitForFunction(()=>!document.querySelector("#github-discover").disabled ||
    document.querySelector("#github-backup-card").getAttribute("aria-busy")==="false");
  assert.ok(await page.locator("#github-discovery-results").isHidden());
  assert.equal(await page.locator("#github-discovery-results option").count(),0);
  assert.equal(await page.locator("#github-token").inputValue(),"");
  assert.equal(await page.locator("#github-modal").isVisible(),false);
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  assert.deepEqual(errors,[]);
  console.log(JSON.stringify({ok:true,checks:["account check","CLI authorization selection","explicit repository creation","recovery key acknowledgement","first encrypted upload","existing backup restore gate","token cleared","restore preview metadata","cancel restore without confirming","stale remote snapshot rejected","re-preview and merge restore","public repository rejection","upload retry without reconnection","lock clears secrets","460px layout"],artifacts},null,2));
}finally{await browser.close();server.close();}
