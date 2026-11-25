const TOTP_IMPORT_TIMEOUT_MS = 60_000;
let totpImport;

function clearTotpImport(record = totpImport, invalidate = true) {
  if (!record || totpImport !== record) return;
  totpImport = null;
  clearTimeout(record.timer);
  if (record.draftId) cancelTotpDraft(record.draftId);
  if (invalidate) record.invalidate();
}

function cancelTotpDraft(draftId) {
  void bridgeCall("password.totp.cancel", { draftId }).catch(() => {});
}

function ensureTotpImport(record) {
  if (record !== totpImport || record.session !== totpSession || Date.now() >= record.expiresAt) {
    throw new Error("扫码预览已失效，请重新扫码 / Scan expired; scan again");
  }
}

async function currentTotpPage() {
  const { tab, origin } = await activePage();
  const url = new URL(origin);
  if (url.protocol !== "https:" && !["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)) {
    throw new Error("请在 HTTPS 网页上扫码 / Scan on an HTTPS page");
  }
  const [result] = await chrome.scripting.executeScript({
    target: { tabId: tab.id, frameIds: [0] },
    func: () => location.href
  });
  if (typeof tab.windowId !== "number" || !result?.documentId || result.result !== tab.url) {
    throw new Error("无法确认当前网页 / Could not identify the current page");
  }
  return { tabId: tab.id, windowId: tab.windowId, documentId: result.documentId, url: result.result, origin };
}

async function verifyTotpPage(record) {
  ensureTotpImport(record);
  const page = await currentTotpPage();
  ensureTotpImport(record);
  if (page.tabId !== record.page.tabId || page.windowId !== record.page.windowId
      || page.documentId !== record.page.documentId || page.url !== record.page.url) {
    throw new Error("网页已变化，请重新扫码 / The page changed; scan again");
  }
}

async function decodeTotpScreenshot(screenshot) {
  const prefix = "data:image/png;base64,";
  if (!screenshot.startsWith(prefix) || screenshot.length > 32 * 1024 * 1024) {
    throw new Error("无法处理当前网页截图 / Could not process the page capture");
  }
  const binary = atob(screenshot.slice(prefix.length));
  const bytes = Uint8Array.from(binary, c => c.charCodeAt(0));
  const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
  try {
    const scale = Math.min(1, 2048 / Math.max(bitmap.width, bitmap.height));
    const canvas = new OffscreenCanvas(Math.round(bitmap.width * scale), Math.round(bitmap.height * scale));
    const context = canvas.getContext("2d", { willReadFrequently: true });
    context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
    const uri = OpenKeyletQRDecoder.decode(context.getImageData(0, 0, canvas.width, canvas.height));
    if (!uri?.startsWith("otpauth://totp/") || uri.length > 4096) {
      throw new Error("未找到 TOTP 二维码，请放大二维码并完整显示 / No TOTP QR code found; zoom in and show the entire code");
    }
    return uri;
  } finally {
    bitmap.close();
  }
}

async function scanTotpPage(record, message) {
  const page = await currentTotpPage();
  ensureTotpImport(record);
  if (page.origin !== message.origin) throw new Error("网页已变化，请重新扫码 / The page changed; scan again");
  record.page = page;
  const status = await bridgeCall("status");
  if (status.locked) throw new Error("请先解锁 OpenKeylet / Unlock OpenKeylet first");
  const entries = await bridgeCall("password.list", { origin: page.url });
  const login = entries.find(item => item.id === message.id);
  if (!login) throw new Error("此网页没有匹配的登录项目 / No matching login on this page");
  await verifyTotpPage(record);
  const screenshot = await chrome.tabs.captureVisibleTab(page.windowId, { format: "png" }).catch(error => {
    if (error.message?.includes("MAX_CAPTURE_VISIBLE_TAB_CALLS_PER_SECOND")) {
      throw new Error("扫码过于频繁，请稍后再试 / Scan rate limit reached; wait a moment and try again");
    }
    throw new Error("无法扫描此网页。请从浏览器工具栏打开 OpenKeylet / Open OpenKeylet from the browser toolbar to scan this page");
  });
  await verifyTotpPage(record);
  const uri = await decodeTotpScreenshot(screenshot);
  await verifyTotpPage(record);
  const draft = await bridgeCall("password.totp.prepare", { origin: page.url, id: login.id, uri });
  try {
    ensureTotpImport(record);
  } catch (error) {
    cancelTotpDraft(draft.draftId);
    throw error;
  }
  record.draftId = draft.draftId;
  await verifyTotpPage(record);
  return { issuer: draft.issuer, account: draft.account, replaces: draft.replaces, destination: draft.destination, username: draft.username };
}

chrome.runtime.onConnect.addListener(port => {
  if (port.name !== "totp-import-ui" || !trustedUi(port.sender, "popup.html")) return;
  let record;
  let disconnected = false;
  const reply = message => { if (!disconnected) port.postMessage(message); };
  port.onDisconnect.addListener(() => {
    disconnected = true;
    if (record) clearTotpImport(record, false);
  });
  port.onMessage.addListener(async message => {
    const action = message?.action;
    try {
      if (action === "scan" && !record) {
        clearTotpImport();
        record = { session: totpSession, expiresAt: Date.now() + TOTP_IMPORT_TIMEOUT_MS, invalidate: () => port.disconnect() };
        totpImport = record;
        record.timer = setTimeout(() => clearTotpImport(record), TOTP_IMPORT_TIMEOUT_MS);
        reply({ action, result: await scanTotpPage(record, message) });
      } else if (action === "confirm" && record?.draftId && !record.committing) {
        record.committing = true;
        await verifyTotpPage(record);
        await bridgeCall("password.totp.commit", { origin: record.page.url, draftId: record.draftId });
        record.draftId = null;
        clearTotpImport(record, false);
        reply({ action, result: true });
      }
    } catch (error) {
      if (record) clearTotpImport(record, false);
      reply({ action, error: error.message });
    }
  });
});

chrome.tabs.onActivated.addListener(({ tabId, windowId }) => {
  const page = totpImport?.page;
  if (page?.windowId === windowId && page.tabId !== tabId) clearTotpImport();
});
chrome.tabs.onUpdated.addListener((tabId, change) => {
  if (totpImport?.page?.tabId === tabId && (change.url || change.status === "loading" || change.discarded)) clearTotpImport();
});
chrome.tabs.onRemoved.addListener(tabId => { if (totpImport?.page?.tabId === tabId) clearTotpImport(); });
chrome.tabs.onReplaced.addListener((_, tabId) => { if (totpImport?.page?.tabId === tabId) clearTotpImport(); });
chrome.windows.onFocusChanged.addListener(windowId => {
  if (totpImport?.page && windowId !== chrome.windows.WINDOW_ID_NONE && windowId !== totpImport.page.windowId) clearTotpImport();
});
