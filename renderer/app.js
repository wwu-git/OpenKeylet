const { invoke } = window.__TAURI__.core;
const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
const HAN_TEXT = /[\u3400-\u9fff]/;

const isMacOS = navigator.platform.startsWith('Mac') || navigator.userAgent.includes('Macintosh');
document.documentElement.classList.toggle('platform-macos', isMacOS);

{
  const titleDragRegion = document.querySelector('.title-drag-region');
  const isTitleControl = (target) => Boolean(target.closest?.('button, input, select, textarea, a, [role="button"]'));
  titleDragRegion.addEventListener('mousedown', (event) => {
    if (event.button === 0 && !isTitleControl(event.target)) event.preventDefault();
  });
  titleDragRegion.addEventListener('selectstart', (event) => event.preventDefault());
  titleDragRegion.addEventListener('dragstart', (event) => event.preventDefault());
}

// Keep the first paint deterministic. The persisted setting from the vault is
// applied by refresh(); browser storage can survive an app reinstall and must
// not override the English default while the native state is loading.
let language = 'en';

function localized(zh, en) {
  return language === 'en' ? en : zh;
}

const staticCopy = [
  ['.nav', 'aria-label', '主导航', 'Main navigation'],
  ['#connections-list', 'aria-label', '项目列表', 'Vault items'],
  ['.activity-panel', 'aria-label', '操作记录', 'Activity log'],
  ['.settings-layout', 'aria-label', '设置', 'Settings'],
  ['#title-search-input', 'placeholder', '搜索项目…', 'Search items…'],
  ['#page-activity .page-search-input', 'placeholder', '搜索记录…', 'Search activity…'],
  ['#page-settings .page-search-input', 'placeholder', '搜索设置…', 'Search settings…'],
  ['#desktop-shortcut-enabled', 'aria-label', '创建桌面快捷方式', 'Create desktop shortcut'],
  ['#launch-at-login-enabled', 'aria-label', '开机自启动', 'Launch at login'],
  ['#pin-enabled', 'aria-label', '启用 PIN 锁', 'Enable PIN lock'],
  ['#browser-enabled', 'aria-label', '启用 Browser Bridge', 'Enable Browser Bridge'],
  ['.window-control[data-window-action="minimize"]', 'aria-label', '最小化', 'Minimize'],
  ['.window-control[data-window-action="close"]', 'aria-label', '关闭', 'Close'],
  ['.nav-item[data-page="connections"] > span', 'text', '项目', 'ITEMS'],
  ['.nav-item[data-page="activity"] > span', 'text', '记录', 'LOG'],
  ['.nav-item[data-page="settings"] > span', 'text', '设置', 'SYSTEM'],
  ['#page-connections .page-header h1', 'text', '秘密项目', 'VAULT ITEMS'],
  ['#title-search-input', 'aria-label', '搜索秘密项目', 'Search vault items'],
  ['#add-connection-button', 'text', '＋ 添加', '+ ADD'],
  ['#drafts-button', 'text', '草稿', 'DRAFT'],
  ['.creation-control', 'aria-label', '添加 / 草稿切换', 'Add / draft selector'],
  ['#generator-open', 'text', '生成器', 'GENERATE'],
  ['#trash-open', 'text', '回收站', 'TRASH'],
  ['#delete-draft-button', 'aria-label', '删除草稿', 'Delete draft'],
  ['#page-activity .page-search-input', 'aria-label', '搜索操作记录', 'Search activity'],
  ['#page-settings .page-search-input', 'aria-label', '搜索系统设置', 'Search settings'],
  ['#header-export-backup-button', 'text', '一键打包', 'EXPORT'],
  ['#header-import-backup-button', 'text', '一键导入包', 'IMPORT'],
  ['[data-module="SYSTEM / 01"] .settings-heading h2', 'text', '系统入口', 'SYSTEM ACCESS'],
  ['[data-module="SYSTEM / 01"] .settings-heading p', 'text', '控制 OpenKeylet 的桌面快捷方式与开机启动。', 'Control the OpenKeylet desktop shortcut and launch at login.'],
  ['#desktop-shortcut-option strong', 'text', '创建快捷方式到桌面', 'CREATE DESKTOP SHORTCUT'],
  ['#desktop-shortcut-option small', 'text', '在当前用户桌面创建 OpenKeylet 快捷方式', 'Create a OpenKeylet shortcut on the current user’s desktop.'],
  ['#launch-at-login-option strong', 'text', '开机自启动', 'LAUNCH AT LOGIN'],
  ['#launch-at-login-option small', 'text', '登录系统后自动启动 OpenKeylet，并保持单实例', 'Start OpenKeylet after sign-in while preserving single-instance behavior.'],
  ['[data-module="MCP / 02"] .settings-heading h2', 'text', 'Agent 接入', 'AGENT SETUP'],
  ['[data-module="MCP / 02"] .settings-heading p', 'text', '注册本地 stdio MCP；支持的 Agent 同时安装“认证任务优先使用 OpenKeylet”规则。', 'Register the local stdio MCP and install the OpenKeylet-first authentication rule where supported.'],
  ['[data-module="APP / 03"] .settings-heading h2', 'text', '外观、窗口与锁定', 'APPEARANCE, WINDOW AND LOCK'],
  ['[data-module="APP / 03"] .settings-heading p', 'text', '调整界面字号、窗口行为和本地 PIN 锁。', 'Adjust text size, window behavior and the local PIN lock.'],
  ['#text-size-label', 'text', '界面字号', 'Text size'],
  ['#text-size option[value="standard"]', 'text', '标准', 'Standard'],
  ['#text-size option[value="large"]', 'text', '大号', 'Large'],
  ['#item-layout-label', 'text', '项目显示方式', 'Item layout'],
  ['#item-layout-help', 'text', '仅保存在当前设备', 'Saved on this device'],
  ['#item-layout option[value="list"]', 'text', '列表', 'List'],
  ['#item-layout option[value="grid"]', 'text', '宫格', 'Grid'],
  ['label[for="close-behavior"] strong', 'text', '关闭按钮', 'CLOSE BUTTON'],
  ['label[for="close-behavior"] small', 'text', '最小化可保留托盘菜单', 'Minimizing keeps tray controls available.'],
  ['#close-behavior option[value="tray"]', 'text', '最小化到托盘', 'MINIMIZE TO TRAY'],
  ['#close-behavior option[value="exit"]', 'text', '退出 OpenKeylet', 'QUIT OpenKeylet'],
  ['.pin-option strong', 'text', 'PIN 锁', 'PIN LOCK'],
  ['.pin-option small', 'text', '关闭后移除当前 PIN；再次开启时设置新的 PIN', 'Turning this off removes the current PIN. Set a new PIN when enabling it again.'],
  ['.agent-restart-notice span', 'text', '完成后重启 Agent。', 'Restart the agent after setup.'],
  ['#agent-client-list .agent-scan-placeholder', 'text', '正在扫描本机 Agent', 'SCANNING LOCAL AGENTS'],
  ['.manual-mcp-config summary', 'text', '其他 Agent / 手动配置', 'OTHER AGENT / MANUAL SETUP'],
  ['.manual-config-steps > div:nth-child(1) > span', 'text', '打开目标 Agent 的 MCP 配置', 'Open the target agent MCP configuration'],
  ['.manual-config-steps > div:nth-child(2) > span', 'text', '按配置格式复制 JSON 或 TOML', 'Copy JSON or TOML to match its config format'],
  ['.manual-config-steps > div:nth-child(3) > span', 'text', '粘贴保存并重新启动 Agent', 'Paste, save, and restart the agent'],
  ['[data-module="WEB / 04"] .settings-heading h2', 'text', '浏览器接入', 'BROWSER BRIDGE'],
  ['[data-module="WEB / 04"] .settings-heading p', 'text', '通过已配对扩展填写当前浏览器控件。', 'Fill the focused browser field through the paired extension.'],
  ['label[for="browser-port"]', 'text', '本地端口', 'LOCAL PORT'],
  ['#save-browser-settings-button', 'text', '保存', 'SAVE'],
  ['#browser-install-heading', 'text', '安装插件 · 只需两步', 'INSTALL IN TWO STEPS'],
  ['#browser-step-open-title', 'text', '打开扩展页', 'Open the extensions page'],
  ['#browser-step-open-description', 'text', '打开浏览器并复制扩展页地址，粘贴到地址栏后开启“开发者模式”。', 'Open a Chromium-based browser and copy the extensions page address. Paste it into the address bar, then turn on Developer mode.'],
  ['#browser-open-help', 'text', '地址已复制。请粘贴到浏览器地址栏并按回车，然后开启“开发者模式”。', 'Address copied. Paste it into the browser address bar and press Enter, then turn on Developer mode.'],
  ['#browser-step-drag-title', 'text', '拖入插件', 'Drag in the extension'],
  ['#browser-step-drag-description', 'text', '按住下方标识，拖到扩展页的空白区域，松开即可安装。', 'Hold the icon below, drag it onto an empty area of the extensions page, and release to install.'],
  ['#browser-install-complete', 'text', '安装后会自动连接，请保持 OpenKeylet 运行。', 'Once installed, the extension connects automatically. Keep OpenKeylet running.'],
  ['#browser-fallback-title', 'text', '拖拽无法安装？', "Can't install by dragging?"],
  ['#browser-fallback-description', 'text', '打开下方扩展目录，然后在扩展页点击“加载已解压的扩展程序”，选择该目录。', 'Open the extension folder below. On the extensions page, click Load unpacked and select that folder.'],
  ['#drag-extension-label', 'text', '按住此处，拖入扩展页', 'Hold and drag to the extensions page'],
  ['#drag-extension-button', 'aria-label', '拖动插件到 Chrome 扩展页；也可按回车打开扩展目录', 'Drag the extension to Chrome Extensions, or press Enter to open its folder'],
  ['#quick-pairing-button', 'text', '打开浏览器并复制地址', 'OPEN BROWSER & COPY ADDRESS'],
  ['#open-extension-button', 'text', '扩展目录', 'EXTENSION FOLDER'],
  ['#reset-pairing-button', 'text', '重置', 'RESET'],
  ['[data-module="SAFE / 05"] .settings-heading h2', 'text', '本地数据', 'LOCAL DATA'],
  ['#migration-open', 'text', '迁入密码库', 'IMPORT VAULT'],
  ['#migration-format option[value="ssh"]', 'text', 'SSH 配置 · OpenSSH', 'SSH config · OpenSSH'],
  ['#migration-format option[value="env"]', 'text', '环境变量 · .env', 'Environment variables · .env'],
  ['[data-module="SAFE / 05"] .settings-heading p', 'text', '用独立备份密码加密导出全部项目与 Passkey，也可恢复旧版备份。', 'Export all items and passkeys encrypted with a separate backup password, or restore an existing backup.'],
  ['#open-data-button', 'text', '打开数据目录', 'OPEN DATA FOLDER'],
  ['#owner-lock-description', 'text', 'PIN 保护明文查看、浏览器密码库访问和 Passkey 验证；模块值默认对 Agent 隐藏。', 'The PIN protects plaintext viewing, browser vault access and passkey verification. Module values are hidden from agents by default.'],
  ['#owner-pin', 'label', '六位数字 PIN', 'SIX-DIGIT PIN'],
  ['#owner-pin-confirm', 'label', '再次输入 PIN', 'CONFIRM PIN'],
  ['.lock-note span', 'text', '秘密由本机随机主密钥加密；PIN 用于本地解锁与验证。', 'Secrets are encrypted by the local random master key. The PIN is used for local unlocking and verification.'],
  ['#owner-pin-cancel', 'text', '取消', 'CANCEL'],
  ['[data-close-modal].icon-button', 'aria-label', '关闭编辑器', 'Close editor'],
  ['#connection-name', 'label', '名称', 'NAME'],
  ['#connection-type', 'label', '类型', 'TYPE'],
  ['#connection-folder', 'label', '文件夹', 'FOLDER'],
  ['#connection-description', 'label-html', '公开用途 <em>可选</em>', 'PUBLIC PURPOSE <em>OPTIONAL</em>'],
  ['.template-copy strong', 'text', '从一个组合开始', 'START WITH A PRESET'],
  ['.template-copy small', 'text', '模板只添加模块，不会限制之后的修改。', 'Templates only add modules; you can change anything afterward.'],
  ['.module-editor-heading > div > strong', 'text', '项目模块', 'ITEM MODULES'],
  ['.module-editor-heading > div > small', 'text', '拖动把手排序 · 开关控制 Agent 可见性', 'DRAG HANDLE TO REORDER · SWITCH CONTROLS AGENT VISIBILITY'],
  ['#add-module-button', 'text', '＋ 添加模块', '+ ADD MODULE'],
  ['#module-menu [data-add-module="username"]', 'text', '账号', 'USERNAME'],
  ['#module-menu [data-add-module="password"]', 'text', '密码', 'PASSWORD'],
  ['#module-menu [data-add-module="apiCredential"]', 'text', 'API 凭据', 'API CREDENTIAL'],
  ['#module-menu [data-add-module="privateKey"]', 'text', '私钥', 'PRIVATE KEY'],
  ['#module-menu [data-add-module="passphrase"]', 'text', '私钥口令', 'KEY PASSPHRASE'],
  ['#module-menu [data-add-module="totp"]', 'text', 'TOTP', 'TOTP'],
  ['#module-menu [data-add-module="customSecret"]', 'text', '自定义字段', 'CUSTOM FIELD'],
  ['#module-menu [data-add-module="host"]', 'text', '主机 / IP', 'HOST / IP'],
  ['#module-menu [data-add-module="port"]', 'text', '端口', 'PORT'],
  ['#module-menu [data-add-module="url"]', 'text', '服务 URL', 'SERVICE URL'],
  ['[data-item-template="login"] > span', 'html', '登录<small>账号 + 密码</small>', 'LOGIN<small>USERNAME + PASSWORD</small>'],
  ['[data-item-template="card"] > span', 'html', '信用卡<small>卡号 + 有效期 + 安全码</small>', 'CARD<small>NUMBER + EXPIRY + CODE</small>'],
  ['[data-item-template="identity"] > span', 'html', '身份资料<small>姓名 + 联系方式 + 地址</small>', 'IDENTITY<small>NAME + CONTACT + ADDRESS</small>'],
  ['[data-item-template="secureNote"] > span', 'html', '安全笔记<small>加密保存长文本</small>', 'SECURE NOTE<small>ENCRYPTED TEXT</small>'],
  ['[data-item-template="ssh"] > span', 'html', 'SSH<small>主机 + 端口 + 账号 + 密码</small>', 'SSH<small>HOST + PORT + USER + PASSWORD</small>'],
  ['[data-item-template="api"] > span', 'html', 'API<small>API 凭据</small>', 'API<small>API CREDENTIAL</small>'],
  ['[data-item-template="aws"] > span', 'html', 'AWS API<small>SigV4 本地签名，操作由 Agent 决定</small>', 'AWS API<small>Local SigV4 signing; the agent chooses operations</small>'],
  ['[data-item-template="oci"] > span', 'html', 'OCI API<small>本地签名代执行，接口和操作由 Agent 决定</small>', 'OCI API<small>Local signing; the agent chooses API operations</small>'],
  ['#api-service-title', 'text', '选择服务', 'Choose a service'],
  ['#totp-qr-title', 'text', '导入 2FA 二维码', 'Import 2FA QR image'],
  ['#totp-qr-choose', 'text', '选取图片', 'Choose image'],
  ['#totp-qr-paste-label', 'text', '在此粘贴剪贴板图片', 'Paste a clipboard image here'],
  ['#totp-qr-issuer-label', 'text', '发行方', 'Issuer'],
  ['#totp-qr-account-label', 'text', '账户', 'Account'],
  ['#totp-qr-cancel', 'text', '取消', 'Cancel'],
  ['#totp-qr-confirm', 'text', '使用此密钥', 'Use setup key'],
  ['#api-service-modal [data-close-service-picker]', 'aria-label', '关闭', 'Close'],
  ['#api-service-list', 'aria-label', '服务目录', 'Services'],
  ['#api-service-search-label', 'text', '搜索服务', 'Search services'],
  ['#api-service-category-label', 'text', '分类', 'Category'],
  ['#api-service-target-label', 'text', '目标地址', 'Target address'],
  ['#api-service-current-label', 'text', '当前地址', 'Current address'],
  ['#api-service-preview-label', 'text', '填入地址预览', 'Address preview'],
  ['#api-service-refresh-target', 'text', '重新选择目标地址', 'Refresh target addresses'],
  ['#api-service-cancel', 'text', '取消', 'Cancel'],
  ['#api-service-add', 'text', '新增地址', 'Add as another address'],
  ['#http-auth-label', 'text', 'HTTP 认证', 'HTTP authentication'],
  ['#oci-bindings-summary', 'text', 'OCI 签名模块', 'OCI signing modules'],
  ['#oci-bindings-note', 'text', '选择用于签名的模块。删除或清空必需模块后，项目可以保存，调用时会提示配置不完整。', 'Choose the modules used for signing. Missing modules can be saved; requests report incomplete configuration.'],
  ['[data-item-template="blank"] > span', 'html', '空白<small>从零添加模块</small>', 'BLANK<small>START WITH NO MODULES</small>'],
  ['#connection-enabled-label', 'text', '启用此项目', 'ENABLE ITEM'],
  ['#connection-enabled-help', 'text', '禁用后 Agent 无法使用', 'Agents cannot use a disabled item.'],
  ['.secret-hint', 'text', '当前项目的明文只在已解锁 GUI 中显示', 'Plaintext is visible only in the unlocked GUI.'],
  ['#connection-form .modal-footer [data-close-modal]', 'text', '关闭', 'CLOSE'],
  ['#save-connection-button', 'text', '保存', 'SAVE'],
  ['.login-websites summary', 'text', '网站匹配', 'WEBSITE MATCHING'],
  ['.login-websites > p', 'text', '设置多个网站及匹配规则', 'Configure websites and matching rules.'],
  ['#login-uri-add', 'text', '＋ 添加网站', '+ ADD WEBSITE'],
];

function applyLanguage() {
  document.documentElement.lang = language === 'en' ? 'en' : 'zh-CN';
  window.OpenKeyletAppearance.setLanguage(localized);
  $$('.language-switch button').forEach((button) => button.classList.toggle('active', button.dataset.language === language));
  for (const [selector, target, zh, en] of staticCopy) {
    const element = $(selector);
    if (!element) continue;
    const copy = localized(zh, en);
    if (target === 'text') element.textContent = copy;
    else if (target === 'html') element.innerHTML = copy;
    else if (target === 'aria-label' || target === 'placeholder') element.setAttribute(target, copy);
    else if (target === 'label' || target === 'label-html') {
      const label = element.closest('label')?.querySelector(':scope > span') || element.closest('.field')?.querySelector(':scope > span');
      if (label) target === 'label-html' ? label.innerHTML = copy : label.textContent = copy;
    }
  }
  if (isMacOS) {
    $('[data-module="SYSTEM / 01"] .settings-heading p').textContent = localized(
      '控制 OpenKeylet 的登录时启动。',
      'Control whether OpenKeylet launches at login.',
    );
  }
  syncItemTypeFilter();
  syncActivityFilterControl();
  syncSettingsFilterControl();
  syncTitleSearch();
  renderHttpAuthentication();
  if (apiServicePicker) renderApiServicePicker();
}

const ociKeyErrorMessages = {
  OCI_KEY_EMPTY: ['请输入完整的 RSA 私钥。', 'Enter the complete RSA private key.'],
  OCI_KEY_FORMAT: ['请导入完整的 RSA PEM 私钥，包含首尾标记。暂不支持 OpenSSH 格式。', 'Import a complete RSA PEM private key, including its header and footer. OpenSSH format is not supported.'],
  OCI_KEY_ENCRYPTED: ['暂不支持加密私钥，请导入未加密的 RSA PEM 私钥。', 'Encrypted private keys are not supported. Import an unencrypted RSA PEM.'],
  OCI_KEY_BASE64: ['PEM 正文不是有效的 Base64，请完整复制原始密钥。', 'The PEM body is not valid Base64. Copy the complete original key.'],
  OCI_KEY_ALGORITHM: ['OCI 签名需要使用受支持算法的 RSA 私钥。', 'OCI signing requires an RSA key with a supported algorithm.'],
  OCI_KEY_TOO_SMALL: ['RSA 密钥参数低于支持的最低要求，请使用至少 2048 位的密钥。', 'RSA key parameters are below the supported minimum. Use a key of at least 2048 bits.'],
  OCI_KEY_UNSUPPORTED: ['RSA 密钥参数或版本不受支持，请重新下载 OCI API 签名密钥。', 'The RSA key parameters or version are not supported. Download the OCI API signing key again.'],
  OCI_KEY_INVALID: ['私钥未通过 RSA 校验，请重新下载原始 OCI API 签名密钥。', 'The private key could not pass RSA validation. Download the original OCI API signing key again.'],
  OCI_KEY_TOO_LARGE: ['签名私钥超过 64 KiB 大小限制。', 'The signing key exceeds the 64 KiB limit.'],
};

function publicMessage(value, fallback = localized('操作失败', 'Operation failed')) {
  const raw = String(value || '').replace(/^Error:\s*/, '');
  const ociCheck = raw.match(/^OCI_CHECK_(OK|FAILED): HTTP (\d{3})(?:; request ID: (.*))?$/);
  if (ociCheck) {
    const result = ociCheck[1] === 'OK'
      ? localized(`OCI 租户根区间检查成功，HTTP ${ociCheck[2]}`, `OCI tenancy root compartment check successful, HTTP ${ociCheck[2]}`)
      : localized(`OCI 租户根区间检查失败，HTTP ${ociCheck[2]}。请检查签名配置和根区间权限；根区间拒绝不一定表示私钥无效。`, `OCI tenancy root compartment check failed, HTTP ${ociCheck[2]}. Check signing configuration and root compartment permissions; root access denial does not necessarily mean the private key is invalid.`);
    return ociCheck[3] ? `${result}${localized('。请求 ID：', '. Request ID: ')}${ociCheck[3]}` : result;
  }
  const code = raw.split(':', 1)[0];
  const ociMessage = Object.hasOwn(ociKeyErrorMessages, code) ? ociKeyErrorMessages[code] : null;
  if (ociMessage) return localized(...ociMessage);
  if (language !== 'en' || !HAN_TEXT.test(raw)) return raw || fallback;
  return fallback;
}

let settingsWriteQueue = Promise.resolve();

function updateSettings(patch) {
  const operation = settingsWriteQueue.then(() => invoke('update_settings', { patch }));
  settingsWriteQueue = operation.catch(() => {});
  return operation;
}

let editorJumpSelection = "";
let editorSshRouteMode = "direct";

const api = {
  state: () => invoke('get_state'),
  ownerStatus: () => invoke('owner_status'),
  ownerTouch: () => invoke('owner_touch'),
  ownerSetPin: (pin) => invoke('owner_set_pin', { pin }),
  ownerDisablePin: () => invoke('owner_disable_pin'),
  ownerUnlock: (pin) => invoke('owner_unlock', { pin }),
  ownerLock: () => invoke('owner_lock'),
  ownerSecrets: (id) => invoke('owner_secret_view', { id }),
  drafts: () => invoke('owner_editor_drafts'),
  saveDraft: (draftId, input) => invoke('save_editor_draft', { draftId, input }),
  deleteDraft: (id) => invoke('delete_editor_draft', { id }),
  copyOwnerValue: (value, sensitive = true) => invoke('copy_owner_value', { value, sensitive }),
  save: (input) => {
    const pending = currentModules.find(module => module.pendingKind === 'oci');
    return invoke('save_connection', { input, expectedUpdatedAt: editorExistingItem?.updatedAt || null, pendingOciKeyId: pending?.pending || null, pendingOciKeyModuleId: pending?.id || null });
  },
  setEnabled: (id, enabled) => invoke('set_connection_enabled', { id, enabled }),
  reorder: (ids, expectedOrder) => invoke('reorder_connections', { ids, expectedOrder }),
  remove: (id, expectedUpdatedAt) => invoke('delete_connection', { id, expectedUpdatedAt }),
  trash: () => invoke('owner_trash'),
  restore: (id, deletedAt) => invoke('owner_restore_item', { id, deletedAt }),
  purge: (id, deletedAt) => invoke('owner_purge_item', { id, deletedAt }),
  history: (id) => invoke('owner_password_history', { id }),
  clearHistory: (id, expectedUpdatedAt) => invoke('owner_clear_password_history', { id, expectedUpdatedAt }),
  test: (id) => invoke('test_connection', { id }),
  sshTrust: () => invoke('ssh_host_trust'),
  probeSshHost: (id) => invoke('probe_ssh_host', { id }),
  setSshTrust: (id, host, port, fingerprint) => invoke('set_ssh_host_trust', { id, host, port, fingerprint }),
  settings: updateSettings,
  systemIntegration: () => invoke('system_integration_status'),
  setDesktopShortcut: (enabled) => invoke('set_desktop_shortcut', { enabled }),
  setLaunchAtLogin: (enabled) => invoke('set_launch_at_login', { enabled }),
  clear: () => invoke('clear_activities'),
  copyConfig: (format) => invoke('copy_mcp_config', { format }),
  agents: () => invoke('agent_mcp_status'),
  registerAgents: (clientIds) => invoke('agent_mcp_register', { clientIds }),
  repairAgent: (clientId) => invoke('agent_mcp_repair', { clientId }),
  removeAgent: (clientId) => invoke('agent_mcp_remove', { clientId }),
  stageKey: (source, passphrase) => invoke('stage_private_key', { source, passphrase }),
  discardKey: (id) => invoke('discard_private_key', { id }),
  copyPublicKey: (pendingId, itemId) => invoke('copy_ssh_public_key', { pendingId, itemId }),
  quickPair: (port) => invoke('quick_pair_browser', { port }),
  openBrowser: () => invoke('open_chromium_browser'),
  resetPair: () => invoke('reset_browser_pairing'),
  extensionFolder: () => invoke('open_browser_extension_folder'),
  exportBackup: (password) => invoke('export_backup', { password }),
  prepareBackupRestore: (id, password) => invoke('prepare_backup_restore', { id, password }),
  commitBackupRestore: (id, selection) => invoke('commit_backup_restore', { id, selection }),
  cancelBackupRestore: (id) => invoke('cancel_backup_restore', { id }),
  prepareImport: (format) => invoke('prepare_import', { format }),
  commitImport: (id) => invoke('commit_import', { id }),
  cancelImport: () => invoke('cancel_import'),
  dataFolder: () => invoke('open_data_folder'),
  window: (action) => invoke('window_action', { action }),
};

let state;
let systemIntegrationState = { desktopShortcut: false, launchAtLogin: false };
let activePage = 'connections';
let currentActivityFilter = 'all';
const pageSearch = { connections: '', activity: '', settings: '' };
const itemTypeFilterOrder = ['all', 'ssh', 'login', 'api', 'passkey', 'card', 'identity', 'secureNote', 'custom'];
let currentItemTypeFilter = 'all';
let currentFolderFilter = 'all';
let currentItemStatusFilter = 'all';
const settingsFilterOrder = ['all', 'SYSTEM', 'MCP', 'APP', 'WEB', 'CLOUD', 'SAFE'];
let currentSettingsFilter = 'all';
let titleSearchOpen = false;
let creationMode = 'add';
let currentModules = [];
let apiServiceCatalog;
let apiServicePicker = null;
let totpQrImport = null;
let itemEditorGeneration = 0;
let editorDrafts = [];
let currentDraftId = '';
let removedSecretFields = new Set();
let editorHttpAuthType = 'auto';
let editorOciBindings = null;
let editorAwsBindings = null;
let moduleKeyInput = null;
let editorDerivedOciUrl = "";
let ociRegionOptions = [];
let ociRegionLoading;
let moduleKeyStaging = null;
let editorExistingItem = null;
let editorSourceInput = null;
let editorOpenedFromDraft = false;
let editorInitialSnapshot = '';
let editorReturnFocus = null;
let inlineEditorItemId = null;
let editorOpening = false;
let editorClosing = null;
let editorFoldAnimation = null;
let switchPointerDrag = null;
let switchSelectionPending = false;
let suppressSwitchClick = null;
let itemPointerDrag = null;
let itemOrderSaving = false;
let suppressItemClick = false;
// Keep the editor below its nested dialogs in the original stacking order.
const editorHome = document.createComment('Item editor home');
$('#connection-modal').before(editorHome);
let agentClients = [];
let agentRestartRequired = false;
let renderedConnectionsSignature = '';
const ACTIVITY_PAGE_SIZE = 50;
let activityVisibleCount = ACTIVITY_PAGE_SIZE;
let activityMatchCount = 0;
let activityLoadPending = false;
const expandedActivityErrors = new Set();
let ownerLockState = { pinConfigured: false, unlocked: false, expiresInSeconds: 0 };
let ownerEpoch = 0;
let ownerTouchAt = -Infinity;
let ownerTouchPending = false;
let ownerLockPending = false;
let ownerPinSetupRequested = false;
let confirmationResolver = null;
let confirmationPreviousFocus = null;
let confirmationInertElements = [];
const githubBackup = window.OpenKeyletGithubBackup.create({
  invoke, localized, toast, itemTypeLabel,
  isUnlocked: () => ownerLockState.unlocked,
  confirmAction: confirmDestructiveAction,
  onRestore: () => refresh(),
});

function escapeHtml(value = '') {
  return String(value).replace(/[&<>"]/g, (character) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[character]);
}

function cleanError(error) {
  return publicMessage(error?.message || error, localized('未知错误', 'Unknown error'));
}

function normalizeSearch(value = '') {
  return String(value).trim().toLocaleLowerCase('zh-CN');
}

function toast(message, type = '') {
  const item = document.createElement('div');
  item.className = `toast ${type}`;
  item.textContent = message;
  $('#toast-region').append(item);
  setTimeout(() => item.remove(), 3200);
}

function finishConfirmation(accepted, restoreFocus = true) {
  if (!confirmationResolver) return;
  const resolve = confirmationResolver;
  confirmationResolver = null;
  $('#confirmation-modal').classList.add('hidden');
  $('#confirmation-details').replaceChildren();
  $('#confirmation-details').classList.add('hidden');
  confirmationInertElements.forEach((element) => { element.inert = false; });
  confirmationInertElements = [];
  if (restoreFocus) {
    let previousFocus = confirmationPreviousFocus;
    if (!previousFocus?.isConnected && previousFocus?.dataset.action && previousFocus?.dataset.id) {
      previousFocus = $$('[data-action][data-id]').find((button) => button.dataset.action === confirmationPreviousFocus.dataset.action && button.dataset.id === confirmationPreviousFocus.dataset.id);
    }
    if (previousFocus?.isConnected && !previousFocus.closest('.hidden')) previousFocus.focus();
    else if ($('#connection-modal').classList.contains('hidden')) $('#add-connection-button').focus();
  }
  confirmationPreviousFocus = null;
  resolve(accepted);
}

function confirmDestructiveAction({ title, message, details, destructive = true, returnFocus = document.activeElement, acceptLabel = localized('删除', 'DELETE'), tag = localized('确认删除', 'CONFIRM DELETION') }) {
  if (confirmationResolver) return Promise.resolve(false);
  finishModulePointerDrag(false);
  $('#confirmation-tag').textContent = tag;
  $('#confirmation-tag').classList.toggle('hidden', !tag);
  $('#confirmation-modal .confirm-icon').classList.toggle('hidden', !destructive);
  $('#confirmation-title').textContent = title;
  $('#confirmation-message').textContent = message;
  const detailsPanel = $('#confirmation-details');
  detailsPanel.replaceChildren(...(details ? [details] : []));
  detailsPanel.classList.toggle('hidden', !details);
  $('#confirmation-cancel').textContent = localized('取消', 'CANCEL');
  const accept = $('#confirmation-accept');
  accept.textContent = acceptLabel;
  accept.classList.toggle('danger', destructive);
  accept.classList.toggle('primary', !destructive);
  confirmationPreviousFocus = returnFocus;
  confirmationInertElements = [...document.body.children].filter((element) => element !== $('#confirmation-modal') && !element.inert && !['SCRIPT', 'STYLE'].includes(element.tagName));
  confirmationInertElements.forEach((element) => { element.inert = true; });
  $('#confirmation-modal').classList.remove('hidden');
  requestAnimationFrame(() => { if (confirmationResolver) $('#confirmation-cancel').focus(); });
  return new Promise((resolve) => { confirmationResolver = resolve; });
}

function clearOwnerPlaintext() {
  void finishSwitchDrag(false);
  finishItemPointerDrag(false);
  githubBackup.clearSecrets();
  finishConfirmation(false, false);
  closeEditor();
  currentModules.forEach((module) => { module.secretValue = ''; });
  $$('[data-secret-value]').forEach((input) => { input.value = ''; });
  currentModules = [];
  editorExistingItem = null;
  $('#connection-form').reset();
  $('#module-list').replaceChildren();
  $('#passkey-list').replaceChildren();
  $('#login-uri-list').replaceChildren();
  $('#vault-folder-options').replaceChildren();
  $('#ssh-jump-item').replaceChildren();
  $('#ssh-route-path').replaceChildren();
  $('#ssh-trust-status').replaceChildren();
}

function pinInputs(id) {
  return $$(`[data-pin-group="${id}"] .pin-cell-input`);
}

function readPin(id) {
  return pinInputs(id).map((input) => input.value).join('');
}

function clearPin(id) {
  pinInputs(id).forEach((input) => { input.value = ''; });
}

function focusPin(id) {
  const inputs = pinInputs(id);
  (inputs.find((input) => !input.value) || inputs[inputs.length - 1])?.focus();
}

function renderOwnerLock() {
  const configured = ownerLockState.pinConfigured;
  const unlocked = ownerLockState.unlocked;
  const setupRequested = ownerPinSetupRequested && !configured;
  const locked = configured && !unlocked;
  const layerVisible = setupRequested || locked;
  $('#owner-lock-layer').classList.toggle('hidden', !layerVisible);
  $('#owner-lock-button').classList.toggle('hidden', !configured || !unlocked);
  $('#owner-pin-confirm-field').classList.toggle('hidden', configured);
  $('#owner-pin-cancel').classList.toggle('hidden', !setupRequested);
  pinInputs('owner-pin-confirm').forEach((input) => { input.required = !configured; });
  $('#owner-lock-mode').textContent = configured ? 'OWNER VERIFY' : 'SET LOCAL PIN';
  $('#owner-lock-title').textContent = configured
    ? localized('输入六位 PIN', 'ENTER SIX-DIGIT PIN')
    : localized('设置六位 PIN', 'SET SIX-DIGIT PIN');
  $('#owner-lock-code').textContent = configured ? 'LOCK' : 'INIT';
  $('#owner-unlock-action').textContent = configured ? 'UNLOCK' : 'SET PIN';
  if (locked) {
    importUndoIds.clear();
    if (!$('#vault-management-modal').classList.contains('hidden')) closeVaultManagement();
    if (!$('#migration-modal').classList.contains('hidden')) closeMigration();
    closeGenerator();
    finishBackupPassword(null);
    closeBackupRestore();
    clearOwnerPlaintext();
    editorDrafts = [];
    renderDrafts();
  }
  $('.app-shell').inert = layerVisible;
  clearPin('owner-pin');
  clearPin('owner-pin-confirm');
  if (layerVisible) setTimeout(() => focusPin('owner-pin'), 0);
}

function applyOwnerLockState(next) {
  const changed = next.pinConfigured !== ownerLockState.pinConfigured || next.unlocked !== ownerLockState.unlocked;
  ownerLockState = next;
  if (!changed) return;
  ownerEpoch++;
  ownerTouchAt = -Infinity;
  const locked = next.pinConfigured && !next.unlocked;
  if (locked) {
    if (state) state = { ...state, connections: [], activities: [] };
    certificatePanel?.clear();
    $('#toast-region').replaceChildren();
  }
  renderOwnerLock();
  if (locked && state) render();
}

async function refreshOwnerLock(showError = true) {
  const epoch = ownerEpoch;
  try {
    const next = await api.ownerStatus();
    if (epoch === ownerEpoch && !ownerLockPending) applyOwnerLockState(next);
  } catch (error) {
    if (showError) toast(cleanError(error), 'error');
  }
}

async function lockOwner() {
  if (!ownerLockState.pinConfigured) return;
  ownerLockPending = true;
  applyOwnerLockState({ ...ownerLockState, unlocked: false, expiresInSeconds: 0 });
  const button = $('#owner-unlock-action');
  button.disabled = true;
  try { await api.ownerLock(); }
  catch (error) { toast(cleanError(error), 'error'); }
  finally { ownerLockPending = false; button.disabled = false; }
}

async function reportOwnerActivity(event) {
  if (!event.isTrusted || document.hidden || !ownerLockState.pinConfigured || !ownerLockState.unlocked || ownerTouchPending) return;
  const now = performance.now();
  if (now - ownerTouchAt < 1000) return;
  ownerTouchAt = now;
  ownerTouchPending = true;
  const epoch = ownerEpoch;
  try {
    const next = await api.ownerTouch();
    if (epoch === ownerEpoch) applyOwnerLockState(next);
  } catch (_) {
    if (epoch === ownerEpoch) await refreshOwnerLock(false);
  } finally {
    ownerTouchPending = false;
  }
}

async function refresh(showError = true) {
  const epoch = ownerEpoch;
  try {
    const next = await api.state();
    if (epoch !== ownerEpoch) return;
    state = ownerLockState.pinConfigured && !ownerLockState.unlocked
      ? { ...next, connections: [], activities: [] }
      : next;
    try {
      systemIntegrationState = await api.systemIntegration();
    } catch (_) {
      systemIntegrationState = { desktopShortcut: false, launchAtLogin: false };
    }
    if (epoch !== ownerEpoch) return;
    language = state.settings.language === 'en' ? 'en' : 'zh';
    await githubBackup.refresh();
    if (epoch !== ownerEpoch) return;
    applyLanguage();
    render();
  } catch (error) {
    if (showError) toast(cleanError(error), 'error');
  }
}

function render() {
  renderMetrics();
  renderConnections();
  renderDrafts();
  renderActivity();
  renderSettings();
}

const segmentNames = ['a', 'b', 'c', 'd', 'e', 'f', 'g'];
const segmentGlyphs = {
  0: 'abcdef', 1: 'bc', 2: 'abdeg', 3: 'abcdg', 4: 'bcfg',
  5: 'acdfg', 6: 'acdefg', 7: 'abc', 8: 'abcdefg', 9: 'abcdfg',
  s: 'acdfg', e: 'adefg', t: 'defg', r: 'eg', d: 'bcdeg', y: 'bcdfg', '-': 'g',
};

function sevenSegmentGlyph(character) {
  const lit = segmentGlyphs[String(character).toLowerCase()] || '';
  const segments = segmentNames.map((segment) => `<i class="segment segment-${segment}" data-lit="${lit.includes(segment)}"></i>`).join('');
  // Reuse the exact lamp contours; only the separate emission layer is blurred.
  return `<span class="seven-digit" aria-hidden="true">${segments}<span class="seven-emission" aria-hidden="true">${segments}</span></span>`;
}

function sevenSegmentNumber(value, length = 4) {
  const exact = Math.max(0, Math.trunc(Number(value) || 0));
  const maximum = (10 ** length) - 1;
  const overflow = exact > maximum;
  const digits = String(Math.min(exact, maximum)).padStart(length, '0');
  return `<span class="seven-number ${overflow ? 'overflow' : ''}" role="img" aria-label="${exact}">${[...digits].map(sevenSegmentGlyph).join('')}${overflow ? '<b class="seven-overflow" aria-hidden="true">+</b>' : ''}</span>`;
}

function activityCode(action = '') {
  const value = String(action).toUpperCase();
  if (/BROWSER|浏览器|\bWEB\b/.test(value)) return 'WEB';
  if (/TERMINAL|终端|PTY/.test(value)) return 'TERM';
  if (/SSH/.test(value)) return 'SSH';
  if (/API|HTTP|\bGET\b|\bPOST\b|\bPUT\b|\bPATCH\b|\bDELETE\b/.test(value)) return 'API';
  if (/FILL|填写|填入/.test(value)) return 'FILL';
  if (/TEST|测试|CHECK/.test(value)) return 'CHECK';
  return 'CALL';
}

function renderMetrics() {
  const total = state.connections.length;
  const enabled = state.connections.filter((item) => item.enabled).length;
  const browserOn = ['listening', 'delegated'].includes(state.browserBridge.status);
  const mcpReady = state.mcp.status === 'ready';
  const now = Date.now();
  const since24Hours = now - 86_400_000;
  const activities24h = state.activities.filter((activity) => {
    const timestamp = new Date(activity.time).getTime();
    return Number.isFinite(timestamp) && timestamp >= since24Hours && timestamp <= now;
  });
  const errors24h = activities24h.filter((activity) => activity.status === 'error').length;
  const registeredAgents = agentClients.filter((client) => client.state === 'registered').length;
  const brokenAgents = agentClients.filter((client) => ['stale', 'conflict', 'error'].includes(client.state)).length;
  const models = {
    connections: {
      value: total, mode: localized('密码库', 'VAULT'), unit: localized('项目', 'ITEMS'),
      label: localized('密码库项目总数', 'Total vault items'),
      secondary: { value: enabled, mode: localized('已启用', 'ENABLED'), unit: localized('项目', 'ITEMS'), label: localized('已启用项目数，包含所有类型', 'Enabled items across all item types') },
    },
    activity: {
      value: activities24h.length, mode: localized('操作', 'CALLS'), unit: '24H',
      label: localized('本地日志中最近 24 小时的操作数', 'Operations in the local log from the last 24 hours'),
      secondary: { value: errors24h, mode: localized('失败', 'ERRORS'), unit: '24H', label: localized('本地日志中最近 24 小时的失败数', 'Errors in the local log from the last 24 hours') },
    },
    settings: {
      value: registeredAgents, mode: localized('已接入', 'LINKED'), unit: 'AGENTS',
      label: localized('已注册 OpenKeylet MCP 的 Agent 客户端数', 'Agent clients registered with OpenKeylet MCP'),
      secondary: { value: brokenAgents, mode: localized('待修复', 'REPAIR'), unit: 'AGENTS', label: localized('需要修复接入配置的 Agent 客户端数', 'Agent clients whose connection configuration needs repair') },
    },
  };
  const model = models[activePage] || models.connections;
  const secondary = model.secondary;
  // These lamps describe current services, independently of historical operation errors.
  const services = [
    {
      name: 'MCP', value: mcpReady ? 'READY' : state.mcp.status === 'error' ? 'ERROR' : 'SETUP',
      tone: mcpReady ? 'on' : state.mcp.status === 'error' ? 'fault' : 'warn',
      label: mcpReady ? localized('本地 MCP 就绪', 'Local MCP is ready') : state.mcp.status === 'error' ? localized('本地 MCP 异常', 'Local MCP has an error') : localized('本地 MCP 需要配置', 'Local MCP needs setup'),
    },
    {
      name: 'WEB', value: state.browserBridge.status === 'error' ? 'ERROR' : !browserOn ? 'OFF' : !state.browserBridge.paired ? 'PAIR' : state.browserBridge.connected ? 'ON' : 'WAIT',
      tone: state.browserBridge.status === 'error' ? 'fault' : browserOn ? state.browserBridge.paired && state.browserBridge.connected ? 'on' : 'warn' : '',
      label: state.browserBridge.status === 'error' ? localized('浏览器桥接异常', 'Browser Bridge has an error') : !browserOn ? localized('浏览器桥接已关闭', 'Browser Bridge is off') : !state.browserBridge.paired ? localized('浏览器扩展等待配对', 'Browser extension is waiting to pair') : state.browserBridge.connected ? localized('浏览器扩展已连接', 'Browser extension is connected') : localized('等待浏览器扩展连接', 'Waiting for the browser extension to connect'),
    },
    {
      name: 'PIN', value: ownerLockState.pinConfigured ? 'ON' : 'OFF', tone: ownerLockState.pinConfigured ? 'on' : '',
      label: ownerLockState.pinConfigured ? localized('已设置本地 PIN 保护', 'Local PIN protection is configured') : localized('尚未设置本地 PIN', 'Local PIN protection is not configured'),
    },
  ];
  $('#metrics').innerHTML = `
    <div class="lcd-screen">
      <div class="lcd-primary" role="group" aria-label="${escapeHtml(model.label)}" title="${escapeHtml(`${model.label}: ${model.value}`)}">
        ${sevenSegmentNumber(model.value)}<div class="lcd-caption" aria-hidden="true"><span>${model.mode}</span><strong>${model.unit}</strong></div>
      </div>
      <div class="lcd-secondary" role="group" aria-label="${escapeHtml(secondary.label)}" title="${escapeHtml(`${secondary.label}: ${secondary.value}`)}">
        ${sevenSegmentNumber(secondary.value, 3)}<div class="lcd-caption" aria-hidden="true"><span>${secondary.mode}</span><strong>${secondary.unit}</strong></div>
      </div>
      <div class="lcd-system" role="group" aria-label="${localized('当前服务状态', 'Current service status')}">
        ${services.map((service) => `<div class="lcd-service" role="group" aria-label="${escapeHtml(service.label)}" title="${escapeHtml(service.label)}"><span aria-hidden="true">${service.name}</span><strong class="lcd-status ${service.tone}" aria-hidden="true">${service.value}</strong></div>`).join('')}
      </div>
    </div>`;
}

function itemDetail(item) {
  if (['card', 'identity', 'secureNote'].includes(item.itemType)) {
    return [itemTypeLabel(item.itemType), item.folder, localized('字段已加密', 'ENCRYPTED FIELDS')].filter(Boolean).join(' · ');
  }
  if (item.passkeys?.length && !itemCapabilities(item).length) return `PASSKEY · ${item.passkeys.map(k => k.rpId).join(' · ')}`;
  const capabilities = itemCapabilities(item);
  const targets = [];
  if (capabilities.includes('ssh') && item.host) targets.push(`${item.host}:${item.port || 22}`);
  if (capabilities.includes('http') && item.baseUrl) targets.push(item.baseUrl);
  if (targets.length) return targets.join(' · ');
  const count = (item.modules || []).filter((module) => module.secret).length || item.secret?.fields?.length || 0;
  return capabilities.includes('fill')
    ? `${count} ENCRYPTED FIELD${count === 1 ? '' : 'S'}`
    : localized('草稿 · 尚无可用动作', 'DRAFT · NO AVAILABLE ACTION');
}

function itemCapabilities(item) {
  return Array.isArray(item.capabilities) ? item.capabilities : [];
}

function itemDisplayKind(item) {
  if (item.itemType && item.itemType !== 'custom' && ITEM_TYPES[item.itemType]) return item.itemType;
  const capabilities = itemCapabilities(item);
  if (capabilities.includes('ssh')) return 'ssh';
  if (capabilities.includes('http')) return 'api';
  if (item.passkeys?.length) return 'passkey';
  if (item.modules?.some(module => module.kind === 'apiCredential')) return 'api';
  if (item.modules?.some(module => module.kind === 'password')) return 'login';
  return 'custom';
}

function itemFilterLabel(kind) {
  if (kind === 'all') return localized('全部类型', 'All types');
  if (kind === 'login') return localized('密码', 'Password');
  return itemTypeLabel(kind);
}

function itemDisplayType(item) { return itemFilterLabel(itemDisplayKind(item)); }

function itemTypeIcon(kind) {
  const paths = {
    all: '<rect x="4" y="4" width="6" height="6" rx="1"/><rect x="14" y="4" width="6" height="6" rx="1"/><rect x="4" y="14" width="6" height="6" rx="1"/><rect x="14" y="14" width="6" height="6" rx="1"/>',
    ssh: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3m6 0h4"/>',
    login: '<circle cx="8" cy="8" r="4.5"/><path d="m11.3 11.3 9.2 9.2m-4-4 2-2m-5-1 2-2"/>',
    api: '<path d="m7 7-5 5 5 5m10-10 5 5-5 5M14 4l-4 16"/>',
    passkey: '<circle cx="8" cy="7" r="3"/><path d="M3 20v-2a5 5 0 0 1 10 0v2m5-5v6m0-3h3"/><circle cx="18" cy="12" r="3"/>',
    card: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M3 10h18M7 15h3"/>',
    identity: '<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="8" cy="10" r="2"/><path d="M5 17v-1a3 3 0 0 1 6 0v1m3-8h4m-4 4h4m-4 4h2"/>',
    secureNote: '<path d="M5 3h10l4 4v14H5Zm9 0v5h5M8 12h8m-8 4h6"/>',
    custom: '<path d="m12 3 8 4v10l-8 4-8-4V7Zm-8 4 8 4 8-4m-8 4v10"/>',
    error: '<circle cx="12" cy="12" r="9"/><path d="m9 9 6 6m0-6-6 6"/>',
    success: '<circle cx="12" cy="12" r="9"/><path d="m7 12 3 3 7-7"/>',
    SYSTEM: '<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M12 17v4m-5 0h10"/>',
    MCP: '<path d="M9 3v4m6-4v4M6 7h12v4a6 6 0 0 1-12 0Zm6 10v4"/>',
    APP: '<path d="M12 3v3m0 12v3M3 12h3m12 0h3M5.6 5.6l2.1 2.1m8.6 8.6 2.1 2.1m0-12.8-2.1 2.1m-8.6 8.6-2.1 2.1"/><circle cx="12" cy="12" r="6"/><circle cx="12" cy="12" r="2"/>',
    WEB: '<circle cx="12" cy="12" r="9"/><ellipse cx="12" cy="12" rx="4" ry="9"/><path d="M3 12h18"/>',
    CLOUD: '<path d="M7 18a5 5 0 1 1 0-10 6 6 0 0 1 11-1 5.5 5.5 0 0 1 0 11Z"/>',
    SAFE: '<path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6Z"/><path d="m8 12 3 3 5-6"/>',
  };
  return `<svg class="item-type-icon" viewBox="0 0 24 24" aria-hidden="true">${paths[kind] || paths.custom}</svg>`;
}

function syncFilterStrip(selector, order, selected, attribute, label, showLabels = false) {
  const control = $(selector);
  if (!control.children.length) {
    control.innerHTML = order.map(kind => `<button type="button" ${attribute}="${kind}">${itemTypeIcon(kind)}${showLabels ? '<span data-filter-label></span>' : ''}</button>`).join('');
  }
  control.style.setProperty('--type-count', order.length);
  control.style.setProperty('--selected-type', order.indexOf(selected));
  $$('button', control).forEach(button => {
    const kind = button.getAttribute(attribute);
    button.setAttribute('aria-pressed', String(kind === selected));
    button.tabIndex = kind === selected ? 0 : -1;
    button.title = label(kind);
    button.setAttribute('aria-label', button.title);
    if (showLabels) $('[data-filter-label]', button).textContent = button.title;
  });
}

function syncItemTypeFilter() {
  $('#item-type-switch').setAttribute('aria-label', localized('项目类型筛选', 'Item type filter'));
  syncFilterStrip('#item-type-switch', itemTypeFilterOrder, currentItemTypeFilter, 'data-item-type-filter', itemFilterLabel);
}

function syncSettingsFilterControl() {
  const labels = { all: ['全部设置', 'All settings'], SYSTEM: ['系统入口', 'System access'], MCP: ['Agent 接入', 'Agent setup'], APP: ['窗口与锁定', 'Window & lock'], WEB: ['浏览器', 'Browser'], CLOUD: ['云端备份', 'Cloud backup'], SAFE: ['本地数据', 'Local data'] };
  $('#settings-filter-switch').setAttribute('aria-label', localized('设置分组筛选', 'Settings section filter'));
  syncFilterStrip('#settings-filter-switch', settingsFilterOrder, currentSettingsFilter, 'data-settings-filter', kind => localized(...labels[kind]));
}

async function selectFilterButton(button) {
  if (button.dataset.itemTypeFilter) await setItemTypeFilter(button.dataset.itemTypeFilter);
  if (button.dataset.activityFilter) {
    currentActivityFilter = button.dataset.activityFilter;
    activityVisibleCount = ACTIVITY_PAGE_SIZE;
    $('.activity-panel').scrollTop = 0;
    renderActivity();
  }
  if (button.dataset.settingsFilter) {
    currentSettingsFilter = button.dataset.settingsFilter;
    syncSettingsFilterControl();
    applySettingsSearch();
    $('.settings-layout').scrollTop = 0;
  }
}

async function setItemTypeFilter(kind) {
  if (!itemTypeFilterOrder.includes(kind) || kind === currentItemTypeFilter) return;
  currentItemTypeFilter = kind;
  syncItemTypeFilter();
  renderConnections();
}

async function selectPage(page) {
  if (inlineEditorItemId && page !== 'connections' && !await closeEditorWithDraft()) return;
  activePage = page;
  syncTitleSearch();
  $('.nav').dataset.active = page;
  $$('.nav-item').forEach(item => {
    item.classList.toggle('active', item.dataset.page === page);
    if (item.dataset.page === page) item.setAttribute('aria-current', 'page');
    else item.removeAttribute('aria-current');
  });
  $$('.page').forEach(item => item.classList.toggle('active', item.id === `page-${page}`));
  renderMetrics();
  if (page === 'settings') scanAgents();
}

async function selectCreationMode(mode) {
  if (mode === 'draft' && !editorDrafts[0]) return;
  if (!await openEditor(null, mode === 'draft' ? editorDrafts[0] : null)) return;
  creationMode = mode;
  renderDrafts();
}

function moduleAgentVisibilityText(visible) {
  return visible
    ? { label: localized('Agent 可查看此值', 'Agent can see this value'), status: localized('Agent 可见', 'AGENT VISIBLE') }
    : { label: localized('不向 Agent 显示此值', 'Hidden from agent'), status: localized('Agent 隐藏', 'AGENT HIDDEN') };
}

function setModuleAgentVisibility(button, visible) {
  button.setAttribute('aria-pressed', String(visible));
  const { label, status } = moduleAgentVisibilityText(visible);
  button.setAttribute('aria-label', label);
  button.title = label;
  button.closest('.module-row').querySelector('[data-module-visibility]').textContent = status;
  syncModuleDraft();
}

// Every travelling cap uses the same drag lifecycle and its existing action.
function switchControlFor(target) {
  const strip = target.closest?.('.filter-strip, .title-switch, .language-switch, .nav, .creation-options, .switch, .module-agent-toggle');
  if (!strip || strip.closest('[inert]') || strip.matches(':disabled, [aria-disabled="true"]')) return null;
  if (strip.matches('.title-switch')) {
    return { strip, count: 2, read: () => Number(strip.getAttribute('aria-checked') === 'true'),
      preview: $$('.title-switch-option', strip), focus: () => strip,
      commit: index => window.OpenKeyletAppearance[strip.id === 'theme-toggle' ? 'setTheme' : 'setSound'](Boolean(index)) };
  }
  if (strip.matches('.switch')) {
    const input = $('input', strip);
    if (input.disabled) return null;
    return { strip, count: 2, read: () => Number(input.checked), focus: () => input,
      disabled: () => input.disabled,
      commit: async index => {
        input.checked = Boolean(index);
        if (input.id === 'desktop-shortcut-enabled') await setSystemIntegration('desktop', input.checked);
        else if (input.id === 'launch-at-login-enabled') await setSystemIntegration('startup', input.checked);
        else if (input.id === 'browser-enabled') await saveBrowserSettings();
        else if (input.id === 'pin-enabled') await savePinSettings();
      } };
  }
  if (strip.matches('.module-agent-toggle')) {
    return { strip, count: 2, read: () => Number(strip.getAttribute('aria-pressed') === 'true'), focus: () => strip,
      commit: index => setModuleAgentVisibility(strip, Boolean(index)) };
  }
  const buttons = $$('button', strip);
  const read = () => buttons.findIndex(button => strip.matches('.nav, .language-switch') ? button.classList.contains('active') : button.getAttribute('aria-pressed') === 'true');
  return { strip, count: buttons.length, buttons, preview: buttons, read,
    disabled: index => buttons[index]?.disabled,
    focus: () => buttons[read()],
    commit: async index => {
      const button = buttons[index];
      if (strip.matches('.filter-strip')) await selectFilterButton(button);
      else if (strip.matches('.language-switch')) await setLanguage(button.dataset.language);
      else if (strip.matches('.nav')) await selectPage(button.dataset.page);
      else await selectCreationMode(button.id === 'drafts-button' ? 'draft' : 'add');
    } };
}

function moveSwitchThumb(event) {
  const drag = switchPointerDrag;
  if (!drag || event.pointerId !== drag.pointerId) return;
  const dx = event.clientX - drag.startX;
  if (!drag.started && Math.abs(dx) < 4) return;
  if (!drag.started && event.pointerType === 'touch' && Math.abs(event.clientY - drag.startY) > Math.abs(dx)) return;
  event.preventDefault();
  if (!drag.started) {
    drag.started = true;
    drag.strip.classList.add('is-dragging');
    drag.strip.classList.add('has-drag-preview');
    drag.strip.setPointerCapture(event.pointerId);
  }
  const position = Math.max(0, Math.min(drag.count - 1, drag.origin + dx / drag.cellWidth));
  drag.index = Math.round(position);
  drag.strip.style.setProperty('--drag-type', position);
  drag.preview?.forEach((button, index) => button.classList.toggle('is-preview', index === drag.index && !drag.disabled?.(index)));
}

async function finishSwitchDrag(commit) {
  const drag = switchPointerDrag;
  if (!drag) return;
  switchPointerDrag = null;
  if (drag.strip.hasPointerCapture(drag.pointerId)) drag.strip.releasePointerCapture(drag.pointerId);
  drag.strip.classList.remove('is-dragging');
  drag.preview?.forEach(button => button.classList.remove('is-preview'));
  if (drag.started) {
    // Escape may precede pointer release by any duration. Keep ownership of
    // that gesture's click until it arrives or a new pointer gesture starts.
    suppressSwitchClick = drag.strip;
  }
  if (commit && drag.started && drag.index !== drag.read() && !drag.disabled?.(drag.index) && drag.strip.isConnected) {
    switchSelectionPending = true;
    drag.strip.setAttribute('aria-busy', 'true');
    drag.strip.style.setProperty('--drag-type', drag.index);
    try {
      if (titleSearchOpen) setTitleSearchOpen(false, false);
      await drag.commit(drag.index);
      if (drag.read() === drag.index && drag.strip.id !== 'sound-toggle') window.OpenKeyletAppearance.feedback();
    } catch (error) { toast(cleanError(error), 'error'); }
    finally {
      switchSelectionPending = false;
      drag.strip.removeAttribute('aria-busy');
      drag.strip.style.removeProperty('--drag-type');
      drag.strip.classList.remove('has-drag-preview');
      // Opening a drawer or confirmation retains its own focus management.
      if (!confirmationResolver && (drag.strip.contains(document.activeElement) || document.activeElement === document.body)) drag.focus()?.focus({ preventScroll: true });
    }
  } else {
    drag.strip.style.removeProperty('--drag-type');
    drag.strip.classList.remove('has-drag-preview');
  }
}

document.addEventListener('pointerdown', event => {
  suppressSwitchClick = null;
  suppressItemClick = false;
  if (event.button !== 0 || !event.isPrimary || switchPointerDrag || switchSelectionPending || event.ctrlKey || event.metaKey) return;
  const control = switchControlFor(event.target);
  if (!control) return;
  const { strip } = control;
  const button = event.target.closest('button');
  if (button?.disabled) return;
  const style = getComputedStyle(strip);
  const width = strip.getBoundingClientRect().width - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
  const origin = control.buttons?.includes(button) ? control.buttons.indexOf(button) : control.read();
  const cellWidth = parseFloat(style.getPropertyValue('--switch-travel')) || width / control.count;
  switchPointerDrag = { ...control, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, origin, index: origin, cellWidth, started: false };
});
document.addEventListener('pointermove', moveSwitchThumb, { passive: false });
document.addEventListener('pointerup', event => {
  if (switchPointerDrag?.pointerId === event.pointerId) { moveSwitchThumb(event); void finishSwitchDrag(true); }
});
document.addEventListener('pointercancel', event => { if (switchPointerDrag?.pointerId === event.pointerId) void finishSwitchDrag(false); });
document.addEventListener('lostpointercapture', event => { if (switchPointerDrag?.pointerId === event.pointerId) void finishSwitchDrag(false); });
window.addEventListener('blur', () => { void finishSwitchDrag(false); });
document.addEventListener('click', event => {
  const strip = suppressSwitchClick;
  suppressSwitchClick = null;
  if (!strip || event.detail === 0 || !strip.contains(event.target)) return;
  event.preventDefault(); event.stopImmediatePropagation();
}, true);

function syncTitleSearch() {
  const open = titleSearchOpen;
  const labels = { connections: ['搜索和筛选项目', 'Search and filter items'], activity: ['搜索记录', 'Search activity'], settings: ['搜索设置', 'Search settings'] };
  const label = localized(...labels[activePage]);
  const control = $('#title-search');
  control.classList.toggle('is-open', open);
  const hasFilters = Boolean(pageSearch[activePage]) || (activePage === 'connections' && [currentItemTypeFilter, currentFolderFilter, currentItemStatusFilter].some(value => value !== 'all'));
  control.classList.toggle('has-query', hasFilters);
  $('#connection-filter-panel').hidden = !open || activePage !== 'connections';
  $('.title-drag-region').classList.toggle('search-open', open);
  const input = $('#title-search-input');
  input.hidden = !open;
  input.dataset.pageSearch = activePage;
  input.placeholder = activePage === 'connections' ? localized('搜索项目名、主机或公开用途…', 'Search names, hosts, or descriptions…') : `${label}…`;
  input.setAttribute('aria-label', label);
  if (input.value !== pageSearch[activePage]) input.value = pageSearch[activePage];
  const button = $('#title-search-toggle');
  button.setAttribute('aria-expanded', String(open));
  button.title = open ? localized('收起搜索', 'Collapse search') : label;
  if (hasFilters && !open) button.title += localized(' · 筛选中', ' · Filter active');
  button.setAttribute('aria-label', button.title);
}

function setTitleSearchOpen(open, focus = true) {
  titleSearchOpen = open;
  syncTitleSearch();
  if (focus) (open ? $('#title-search-input') : $('#title-search-toggle')).focus({ preventScroll: true });
}

function itemPermission(item) {
  const capabilities = itemCapabilities(item);
  if (capabilities.includes('http')) return 'HTTP';
  if (capabilities.includes('ssh')) return 'SSH';
  return capabilities.includes('fill') ? 'FOCUS INPUT' : 'NOT EXPOSED';
}

const activityFilterDefinitions = {
  all: { key: 'A', label: 'ALL' },
  error: { key: 'B', label: 'ERROR' },
  success: { key: 'C', label: 'PASS' },
};
const activityFilterOrder = Object.keys(activityFilterDefinitions);

function syncActivityFilterControl() {
  const labels = { all: ['全部', 'ALL'], error: ['错误', 'ERROR'], success: ['成功', 'PASS'] };
  $('#activity-filter-switch').setAttribute('aria-label', localized('记录结果筛选', 'Activity result filter'));
  syncFilterStrip('#activity-filter-switch', activityFilterOrder, currentActivityFilter, 'data-activity-filter', kind => localized(...labels[kind]), true);
}

function syncConnectionSearchControls(matchCount, editingOutsideFilters) {
  $('#connection-folder-filter-label').textContent = localized('文件夹', 'Folder');
  $('#connection-status-filter-label').textContent = localized('启用状态', 'Status');

  const folders = [...new Set(state.connections.map(item => item.folder).filter(Boolean))];
  // Keep a selected folder available when its last project is moved or removed.
  if (currentFolderFilter.startsWith('folder:')) {
    const selected = currentFolderFilter.slice('folder:'.length);
    if (!folders.includes(selected)) folders.push(selected);
  }
  const folderOptions = [
    ['all', localized('全部文件夹', 'All folders')],
    ['unfiled', localized('未分组', 'Unfiled')],
    ...folders.sort((a, b) => a.localeCompare(b, language)).map(name => [`folder:${name}`, name]),
  ];
  const statusOptions = [
    ['all', localized('全部状态', 'All statuses')],
    ['enabled', localized('已启用', 'Enabled')],
    ['disabled', localized('已禁用', 'Disabled')],
  ];
  for (const [selector, options, selected] of [
    ['#connection-folder-filter', folderOptions, currentFolderFilter],
    ['#connection-status-filter', statusOptions, currentItemStatusFilter],
  ]) {
    const select = $(selector);
    const markup = options.map(([value, label]) => `<option value="${escapeHtml(value)}">${escapeHtml(label)}</option>`).join('');
    if (select.innerHTML !== markup) select.innerHTML = markup;
    select.value = selected;
  }

  const chips = [];
  if (currentItemTypeFilter !== 'all') chips.push(['type', itemFilterLabel(currentItemTypeFilter)]);
  if (currentFolderFilter !== 'all') chips.push(['folder', folderOptions.find(([value]) => value === currentFolderFilter)[1]]);
  if (currentItemStatusFilter !== 'all') chips.push(['status', statusOptions.find(([value]) => value === currentItemStatusFilter)[1]]);
  const chipList = $('#connection-filter-chips');
  const chipMarkup = chips.map(([kind, label]) => `<button type="button" class="button quiet" data-clear-item-filter="${kind}" aria-label="${escapeHtml(localized(`移除筛选：${label}`, `Remove filter: ${label}`))}">${escapeHtml(label)} <span aria-hidden="true">×</span></button>`).join('');
  if (chipList.innerHTML !== chipMarkup) chipList.innerHTML = chipMarkup;
  $('#connection-result-count').textContent = localized(
    `${matchCount} / ${state.connections.length} 个项目` + (editingOutsideFilters ? ' · 保留正在编辑的项目' : ''),
    `${matchCount} / ${state.connections.length} projects` + (editingOutsideFilters ? ' · Open editor kept visible' : ''),
  );
  const clearAll = $('#connection-clear-all');
  clearAll.textContent = localized('清除全部', 'Clear all');
  clearAll.hidden = !chips.length && !pageSearch.connections;
  syncTitleSearch();
}

function matchesConnectionFilters(item, words) {
  if (currentItemTypeFilter !== 'all' && itemDisplayKind(item) !== currentItemTypeFilter) return false;
  const folder = item.folder ? `folder:${item.folder}` : 'unfiled';
  if (currentFolderFilter !== 'all' && currentFolderFilter !== folder) return false;
  if (currentItemStatusFilter !== 'all' && Boolean(item.enabled) !== (currentItemStatusFilter === 'enabled')) return false;
  if (!words.length) return true;
  const fields = (item.secret?.fields || []).map(field => field.name);
  const addresses = [...(item.loginUris || []).map(rule => rule.uri), ...(item.modules || []).filter(module => module.kind === 'host' || module.kind === 'url').flatMap(module => [module.value || '', ...(module.alternateValues || [])])];
  const searchable = normalizeSearch([
    item.name, item.description, item.folder, item.host, item.baseUrl, ...addresses,
    itemTypeLabel(item.itemType), itemDisplayType(item), ...itemCapabilities(item), itemDetail(item), itemPermission(item), ...fields,
  ].join(' '));
  return words.every(word => searchable.includes(word));
}

function renderConnections() {
  if (itemPointerDrag || itemOrderSaving) return;
  const words = normalizeSearch(pageSearch.connections).split(/\s+/).filter(Boolean);
  const signature = JSON.stringify([state.connections, pageSearch.connections, language, inlineEditorItemId, currentItemTypeFilter, currentFolderFilter, currentItemStatusFilter]);
  if (signature === renderedConnectionsSignature) return;
  renderedConnectionsSignature = signature;
  const matched = new Set(state.connections.filter(item => matchesConnectionFilters(item, words)).map(item => item.id));
  const items = state.connections.filter(item => matched.has(item.id) || item.id === inlineEditorItemId);
  syncConnectionSearchControls(matched.size, items.length > matched.size);
  const container = $('#connections-list');
  const editor = $('#connection-modal');
  const focusedEditorControl = editor.contains(document.activeElement) ? document.activeElement : null;
  // Preserve the live form (and unsaved values) when language or list data refreshes.
  if (container.contains(editor)) editorHome.after(editor);
  if (inlineEditorItemId && !state.connections.some(item => item.id === inlineEditorItemId)) closeEditor();
  if (!items.length) {
    const hasItems = state.connections.length > 0;
    container.innerHTML = `<div class="empty-state"><div><div class="empty-code">LOCAL VAULT</div><h3>${hasItems ? localized('没有匹配项目', 'NO MATCHING ITEMS') : localized('还没有秘密项目', 'NO VAULT ITEMS')}</h3><p>${hasItems ? localized('当前筛选没有结果。', 'No items match the current filters.') : localized('保存密码、Passkey、信用卡、身份资料和安全笔记，在需要时使用。', 'Keep passwords, passkeys, cards, identities and secure notes together.')}</p><button class="button primary" data-action="${hasItems ? 'clear-connection-filters' : 'add'}">${hasItems ? localized('清除筛选', 'CLEAR FILTERS') : localized('添加项目', 'ADD ITEM')}</button></div></div>`;
    return;
  }
  container.innerHTML = items.map((item) => {
    const type = itemDisplayType(item);
    const canTest = item.enabled && item.canTest;
    const checkTitle = item.httpAuthType === 'oci-signature'
      ? localized('只读检查租户根区间的实例列表，最多请求 1 项，不显示实例内容。', 'Read-only ListInstances check of the tenancy root compartment, limit 1; instance data is not displayed.')
      : item.httpAuthType === 'aws-sigv4'
        ? localized('仅检查本机 AWS 签名配置，不发送请求，不验证云端凭据或 IAM 权限。', 'Check local AWS signing configuration only. No request is sent; cloud credentials and IAM permissions are not verified.')
      : localized('检查连接', 'Check connection');
    const agentActions = itemCapabilities(item).length > 0;
    const reorderLabel = localized(`${item.name}：单击展开或收起；拖动排序，或按上/下方向键`, `${item.name}: click to expand or collapse; drag to reorder, or use the Up/Down arrow keys`);
    const toggleLabel = item.enabled
      ? localized('停用此项目；Agent 将无法使用其中的秘密', 'Disable this item; agents will no longer be able to use its secrets')
      : localized('启用此项目；Agent 将可以使用其中的秘密', 'Enable this item so agents can use its secrets');
    return `<article class="connection-card ${itemCapabilities(item).includes('ssh') ? 'ssh' : itemCapabilities(item).includes('http') ? 'http' : 'fill'} ${item.enabled ? '' : 'disabled'}" data-item-id="${escapeHtml(item.id)}">
      <div class="item-disclosure-well"><button class="item-disclosure" type="button" data-action="edit" data-id="${item.id}" aria-expanded="false" aria-controls="connection-modal" aria-label="${escapeHtml(localized(`展开编辑 ${item.name}`, `Expand editor for ${item.name}`))}" title="${localized('展开编辑', 'Expand editor')}"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg></button></div>
      <span class="connection-type" role="img" data-item-type="${itemDisplayKind(item)}" aria-label="${escapeHtml(type)}" title="${escapeHtml(type)}">${itemTypeIcon(itemDisplayKind(item))}</span>
      <button class="connection-name" type="button" draggable="false" data-action="edit" data-id="${escapeHtml(item.id)}" data-item-reorder="${escapeHtml(item.id)}" aria-expanded="false" aria-controls="connection-modal" aria-label="${escapeHtml(reorderLabel)}" aria-keyshortcuts="Enter Space ArrowUp ArrowDown Home End" title="${escapeHtml(reorderLabel)}"><span class="lettering-ink">${escapeHtml(item.name)}</span></button>
      <div class="card-actions" role="group" aria-label="${escapeHtml(item.name)}">
        <button class="module-state item-toggle" type="button" data-action="toggle-enabled" data-id="${item.id}" aria-pressed="${item.enabled}" title="${toggleLabel}" aria-label="${toggleLabel}"><i class="status-dot ${item.enabled ? '' : 'off'}" aria-hidden="true"></i></button>
        ${agentActions ? `<button class="small-button" type="button" data-action="test" data-id="${item.id}" title="${escapeHtml(checkTitle)}" ${canTest ? '' : 'disabled'}>CHECK</button><button class="small-button" type="button" data-action="copy-name" data-id="${item.id}" title="${localized('复制 OpenKeylet MCP 使用提示', 'Copy OpenKeylet MCP use prompt')}" aria-label="${localized('复制 OpenKeylet MCP 使用提示', 'Copy OpenKeylet MCP use prompt')}">USE</button>` : ''}${item.passwordHistoryCount ? `<button class="small-button" type="button" data-action="history" data-id="${item.id}">HISTORY</button>` : ''}<button class="small-button delete" type="button" data-action="delete" data-id="${item.id}">DEL</button>
      </div>
    </article>`;
  }).join('');
  if (inlineEditorItemId) {
    placeItemEditor();
    if (focusedEditorControl?.isConnected) focusedEditorControl.focus({ preventScroll: true });
  }
}

function draftSecretValues(secrets = {}) {
  const named = secrets.namedSecrets || {};
  return {
    ...named,
    password: secrets.password || named.password || '',
    passphrase: secrets.passphrase || named.passphrase || '',
    privateKey: secrets.privateKey || '',
    apiCredential: named.apiCredential || secrets.token || secrets.apiKey || '',
  };
}

function renderDrafts() {
  const button = $('#drafts-button');
  const hasDraft = Boolean(editorDrafts[0]);
  if (!hasDraft) creationMode = 'add';
  $('.creation-control').dataset.mode = creationMode;
  $('#add-connection-button').setAttribute('aria-pressed', String(creationMode === 'add'));
  button.setAttribute('aria-pressed', String(creationMode === 'draft'));
  const deleteButton = $('#delete-draft-button');
  button.disabled = !hasDraft;
  button.textContent = localized('草稿', 'DRAFT');
  button.title = hasDraft ? localized('继续最近的草稿', 'Continue the most recent draft') : localized('暂无草稿', 'No draft');
  deleteButton.classList.toggle('hidden', !hasDraft || creationMode !== 'draft');
  deleteButton.title = localized('删除草稿', 'Delete draft');
}

async function refreshDrafts(showError = false) {
  if (!ownerLockState.unlocked) {
    editorDrafts = [];
    renderDrafts();
    return;
  }
  const epoch = ownerEpoch;
  try {
    const drafts = await api.drafts();
    if (epoch !== ownerEpoch) return;
    editorDrafts = drafts;
    renderDrafts();
  } catch (error) {
    if (showError) toast(cleanError(error), 'error');
  }
}

function formatTime(value) {
  const diff = Date.now() - new Date(value).getTime();
  if (diff < 60_000) return localized('刚刚', 'JUST NOW');
  if (diff < 3_600_000) return localized(`${Math.floor(diff / 60_000)} 分钟前`, `${Math.floor(diff / 60_000)} MIN AGO`);
  if (diff < 86_400_000) return localized(`${Math.floor(diff / 3_600_000)} 小时前`, `${Math.floor(diff / 3_600_000)} HR AGO`);
  return new Date(value).toLocaleString(language === 'en' ? 'en-US' : 'zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
}

function activityAction(action = '') {
  const raw = String(action);
  if (language !== 'en' || !HAN_TEXT.test(raw)) return raw;
  if (/测试连接/.test(raw)) return 'Test connection';
  if (/重置.*SSH.*主机信任/.test(raw)) return 'Reset SSH host trust';
  if (/导入备份/.test(raw)) return 'Import portable backup';
  if (/填写|填入/.test(raw)) return `Fill secret field (${activityCode(raw)})`;
  return 'OpenKeylet operation';
}

function activitySource(source = '') {
  if (language === 'en' && HAN_TEXT.test(String(source))) return 'APP';
  return source;
}

function renderActivity() {
  syncActivityFilterControl();
  const container = $('#activity-list');
  const query = normalizeSearch(pageSearch.activity);
  const activities = state.activities.map((activity, sourceIndex) => ({ activity, sourceIndex })).filter(({ activity }) => {
    if (currentActivityFilter !== 'all' && activity.status !== currentActivityFilter) return false;
    return !query || normalizeSearch([activity.action, activity.connectionName, activity.source, activity.status, activity.error].join(' ')).includes(query);
  });
  activityMatchCount = activities.length;
  if (!activities.length) {
    const hasActivity = state.activities.length > 0;
    container.innerHTML = `<div class="empty-state"><div><h3>${hasActivity ? localized('没有匹配记录', 'NO MATCHING ACTIVITY') : localized('暂无操作记录', 'NO ACTIVITY YET')}</h3><p>${hasActivity ? localized('当前搜索或结果筛选没有记录。', 'No activity matches the current search and result filter.') : localized('Agent 使用 OpenKeylet 后，这里只显示不含秘密的摘要。', 'After an agent uses OpenKeylet, only secret-free summaries appear here.')}</p>${hasActivity ? `<button class="button primary" data-action="clear-activity-filters">${localized('重置搜索与筛选', 'RESET SEARCH & FILTER')}</button>` : ''}</div></div>`;
    return;
  }
  container.innerHTML = activities.slice(0, activityVisibleCount).map(({ activity, sourceIndex }) => {
    const errorExpanded = expandedActivityErrors.has(activity.id);
    return `<article class="activity-item"><div class="log-index">${String(state.activities.length - sourceIndex).padStart(4, '0')}</div><div class="activity-status ${activity.status === 'error' ? 'error' : ''}">${activity.status === 'error' ? '×' : '✓'}</div><div><div class="activity-title">${escapeHtml(activityAction(activity.action))} · ${escapeHtml(activity.connectionName)}</div><div class="activity-meta">${escapeHtml(activitySource(activity.source))} · ${activity.durationMs} ms</div>${activity.error ? `<button type="button" class="activity-error ${errorExpanded ? 'expanded' : ''}" data-expand-error="${activity.id}" aria-expanded="${errorExpanded}">${escapeHtml(publicMessage(activity.error, localized('操作失败', 'Operation failed. Details are available in the local log.')))}</button>` : ''}</div><time class="activity-time">${formatTime(activity.time)}</time></article>`;
  }).join('');
}

function queueMoreActivities() {
  if (activityLoadPending || activityVisibleCount >= activityMatchCount) return;
  activityLoadPending = true;
  setTimeout(() => {
    activityVisibleCount = Math.min(activityVisibleCount + ACTIVITY_PAGE_SIZE, activityMatchCount);
    renderActivity();
    activityLoadPending = false;
  }, 120);
}

function applySettingsSearch() {
  const query = normalizeSearch(pageSearch.settings);
  let visible = 0;
  for (const card of $$('.settings-card')) {
    const matchesSection = currentSettingsFilter === 'all' || card.dataset.module?.startsWith(`${currentSettingsFilter} /`);
    const matches = matchesSection && (!query || normalizeSearch(`${card.dataset.module || ''} ${card.textContent}`).includes(query));
    card.classList.toggle('search-hidden', !matches);
    if (matches) visible += 1;
  }
  $('#settings-search-empty').classList.toggle('hidden', visible > 0);
}

let certificatePanel;
function renderSettings() {
  certificatePanel ??= createCertificatePanel({ invoke, getState: () => state, isUnlocked: () => ownerLockState.unlocked, localized, escapeHtml, toast, cleanError, refresh: () => refresh() });
  certificatePanel.render();
  if (ownerLockState.unlocked) void certificatePanel.load().catch(() => {});
  renderAgentWriteSettings();
  githubBackup.render();
  $('#mcp-endpoint').textContent = `${state.mcp.stdioCommand} mcp stdio`;
  const mcpFault = state.mcp.status === 'error';
  const mcpStrip = $('#mcp-status-strip');
  mcpStrip.className = `status-strip ${mcpFault ? 'error' : ''}`;
  mcpStrip.innerHTML = `<span class="status-dot"></span><span class="status-key">${mcpFault ? 'FAULT' : 'READY'}</span><span>${mcpFault ? escapeHtml(publicMessage(state.mcp.error)) : localized('Agent 使用时自动启动 stdio，无需常驻服务。', 'stdio starts on demand. No background service.')}</span>`;
  $('#agent-restart-notice').classList.toggle('hidden', !agentRestartRequired);
  $('#desktop-shortcut-enabled').checked = Boolean(systemIntegrationState.desktopShortcut);
  $('#launch-at-login-enabled').checked = Boolean(systemIntegrationState.launchAtLogin);
  $('#pin-enabled').checked = ownerLockState.pinConfigured || ownerPinSetupRequested;
  $('#close-behavior').value = state.settings.closeBehavior === 'exit' ? 'exit' : 'tray';
  $('#browser-enabled').checked = state.settings.browserEnabled;
  $('#browser-port').value = state.settings.browserPort;
  const bridge = state.browserBridge;
  const online = ['listening', 'delegated'].includes(bridge.status);
  const strip = $('#browser-status-strip');
  strip.className = `status-strip ${bridge.status === 'error' ? 'error' : online ? '' : 'offline'}`;
  strip.innerHTML = `<span class="status-dot"></span><span class="status-key">${bridge.status === 'error' ? 'FAULT' : online ? (bridge.connected ? 'CONNECTED' : bridge.paired ? 'WAITING' : 'READY') : 'OFF'}</span><span>${bridge.status === 'error' ? escapeHtml(publicMessage(bridge.error)) : online ? escapeHtml(bridge.endpoint) : localized('Browser Bridge 已关闭', 'Browser Bridge is off')}</span>`;
  $('#reset-pairing-button').disabled = !bridge.paired;
  renderAgents();
  applySettingsSearch();
}

const agentLabels = { notDetected: 'NOT FOUND', available: 'READY', registered: 'CONNECTED', stale: 'NEEDS REPAIR', conflict: 'CONFLICT', error: 'ERROR' };
function agentDetail(client) {
  if (client.state === 'error') {
    if (client.message === 'Configuration detected, but no callable CLI was found') {
      return localized(
        `检测到配置，但未找到可调用的 ${client.displayName} 命令。请安装命令行工具或检查安装路径。`,
        `Configuration found, but the ${client.displayName} CLI is unavailable. Install the CLI or check its installation path.`,
      );
    }
    return publicMessage(client.message, localized('检测失败', 'Detection failed'));
  }
  const details = language === 'en'
    ? { notDetected: 'Not installed', available: 'Ready to connect', registered: 'Connected', stale: 'OpenKeylet setup needs repair', conflict: 'Config conflict', error: 'Detection failed' }
    : { notDetected: '未安装或未发现', available: '可连接', registered: '已连接', stale: 'OpenKeylet 连接或全局规则需要修复', conflict: '配置冲突', error: '检测失败' };
  return details[client.state] || publicMessage(client.message, localized('状态已更新', 'Status updated'));
}
function renderAgents() {
  const container = $('#agent-client-list');
  container.innerHTML = agentClients.length ? agentClients.map((client) => `<div class="agent-client-row ${client.canRegister ? '' : 'is-disabled'}"><span class="agent-client-copy"><strong>${escapeHtml(client.displayName)}</strong><span>${escapeHtml(agentDetail(client))}</span></span><div class="agent-row-controls"><span class="agent-state ${escapeHtml(client.state)}">${escapeHtml(agentLabels[client.state] || client.state)}</span><div class="agent-row-actions">${client.canRegister ? `<button type="button" class="agent-row-action primary" data-agent-action="connect" data-client-id="${escapeHtml(client.clientId)}">CONNECT</button>` : ''}${client.canRepair ? `<button type="button" class="agent-row-action primary" data-agent-action="repair" data-client-id="${escapeHtml(client.clientId)}">REPAIR</button>` : ''}${client.canRemove ? `<button type="button" class="agent-row-action" data-agent-action="remove" data-client-id="${escapeHtml(client.clientId)}">REMOVE</button>` : ''}</div></div></div>`).join('') : `<div class="agent-scan-placeholder">${localized('没有检测到支持的 Agent', 'NO SUPPORTED AGENT DETECTED')}</div>`;
  applySettingsSearch();
}

async function scanAgents(showToast = false) {
  try {
    agentClients = await api.agents();
    renderAgents();
    if (state) renderMetrics();
    if (showToast) toast(localized('Agent 扫描完成', 'Agent scan complete'));
  } catch (error) { toast(cleanError(error), 'error'); }
}

const MODULE_DEFS = {
  cardholderName: { code: 'CHN', zh: '持卡人姓名', en: 'CARDHOLDER', secret: true },
  cardNumber: { code: 'PAN', zh: '卡号', en: 'CARD NUMBER', secret: true },
  cardBrand: { code: 'BRD', zh: '卡组织', en: 'BRAND', secret: true },
  cardExpMonth: { code: 'MM', zh: '到期月份', en: 'EXPIRY MONTH', secret: true },
  cardExpYear: { code: 'YYYY', zh: '到期年份', en: 'EXPIRY YEAR', secret: true },
  cardSecurityCode: { code: 'CVC', zh: '安全码', en: 'SECURITY CODE', secret: true },
  identityTitle: { code: 'TTL', zh: '称谓', en: 'TITLE', secret: true },
  firstName: { code: 'FN', zh: '名字', en: 'FIRST NAME', secret: true },
  middleName: { code: 'MN', zh: '中间名', en: 'MIDDLE NAME', secret: true },
  lastName: { code: 'LN', zh: '姓氏', en: 'LAST NAME', secret: true },
  company: { code: 'ORG', zh: '公司', en: 'COMPANY', secret: true },
  email: { code: 'MAIL', zh: '电子邮箱', en: 'EMAIL', secret: true },
  phone: { code: 'TEL', zh: '电话号码', en: 'PHONE', secret: true },
  address1: { code: 'AD1', zh: '地址第一行', en: 'ADDRESS LINE 1', secret: true },
  address2: { code: 'AD2', zh: '地址第二行', en: 'ADDRESS LINE 2', secret: true },
  address3: { code: 'AD3', zh: '地址第三行', en: 'ADDRESS LINE 3', secret: true },
  city: { code: 'CTY', zh: '城市', en: 'CITY', secret: true },
  state: { code: 'REG', zh: '省 / 州', en: 'STATE / REGION', secret: true },
  postalCode: { code: 'ZIP', zh: '邮政编码', en: 'POSTAL CODE', secret: true },
  country: { code: 'CTR', zh: '国家 / 地区', en: 'COUNTRY / REGION', secret: true },
  passportNumber: { code: 'PPT', zh: '护照号码', en: 'PASSPORT', secret: true },
  licenseNumber: { code: 'DL', zh: '驾照号码', en: 'DRIVER LICENSE', secret: true },
  nationalId: { code: 'ID', zh: '身份证号码', en: 'NATIONAL ID', secret: true },
  secureNote: { code: 'NOTE', zh: '安全笔记', en: 'SECURE NOTE', secret: true, multiline: true },
  username: { code: 'USR', zh: '账号', en: 'USERNAME', secret: true },
  password: { code: 'PWD', zh: '密码', en: 'PASSWORD', secret: true },
  apiCredential: { code: 'API', zh: 'API 凭据', en: 'API CREDENTIAL', secret: true },
  privateKey: { code: 'KEY', zh: '私钥', en: 'PRIVATE KEY', secret: true },
  passphrase: { code: 'PPH', zh: '私钥口令', en: 'KEY PASSPHRASE', secret: true },
  totp: { code: 'OTP', zh: 'TOTP', en: 'TOTP', secret: true },
  customSecret: { code: 'SEC', zh: '自定义字段', en: 'CUSTOM FIELD', secret: true },
  host: { code: 'HST', zh: '主机 / IP', en: 'HOST / IP', secret: false },
  port: { code: 'PRT', zh: '端口', en: 'PORT', secret: false },
  url: { code: 'URL', zh: '服务 URL', en: 'SERVICE URL', secret: false },
  publicKey: { code: 'PUB', zh: '公钥', en: 'PUBLIC KEY', secret: false, multiline: true },
  sshCertificate: { code: 'SSH', zh: 'SSH 证书', en: 'SSH CERTIFICATE', secret: false, multiline: true },
  certificate: { code: 'CRT', zh: 'X.509 证书', en: 'X.509 CERTIFICATE', secret: false, multiline: true },
  csr: { code: 'CSR', zh: '证书签名请求', en: 'CERTIFICATE REQUEST', secret: false, multiline: true },
  caCertificate: { code: 'CA', zh: 'CA 证书', en: 'CA CERTIFICATE', secret: false, multiline: true },
};
const PERSONAL_FIELDS = Object.keys(MODULE_DEFS).slice(0, Object.keys(MODULE_DEFS).indexOf('username'));
const ITEM_TYPES = { login: ['登录', 'Login'], card: ['信用卡', 'Card'], identity: ['身份资料', 'Identity'], secureNote: ['安全笔记', 'Secure note'], ssh: ['SSH', 'SSH'], api: ['API', 'API'], custom: ['自定义', 'Custom'], passkey: ['Passkey', 'Passkey'] };
function itemTypeLabel(kind) { const labels = Object.hasOwn(ITEM_TYPES, kind) ? ITEM_TYPES[kind] : ITEM_TYPES.custom; return localized(...labels); }
function typeOptions(emptyLabel, selected, includePasskey = false) {
  return `${emptyLabel ? `<option value="">${emptyLabel}</option>` : ''}${Object.keys(ITEM_TYPES).filter(k => includePasskey || k !== 'passkey').map(k => `<option value="${k}" ${k === selected ? 'selected' : ''}>${itemTypeLabel(k)}</option>`).join('')}`;
}
for (const kind of [...PERSONAL_FIELDS, 'publicKey', 'sshCertificate', 'certificate', 'csr', 'caCertificate']) {
  const button = document.createElement('button'); button.type = 'button'; button.dataset.addModule = kind;
  button.textContent = moduleLabel(kind); $('#module-menu').append(button);
  staticCopy.push([`#module-menu [data-add-module="${kind}"]`, 'text', MODULE_DEFS[kind].zh, MODULE_DEFS[kind].en]);
}

function moduleHasPlaintextReveal(kind) {
  return Boolean(MODULE_DEFS[kind]?.secret);
}

function defaultModuleAgentVisible(kind) {
  return !moduleHasPlaintextReveal(kind);
}

function moduleSecretName(module) {
  return module.kind === 'customSecret' ? String(module.name || '').trim() : MODULE_DEFS[module.kind]?.secret ? module.kind : '';
}

function moduleLabel(kind) {
  const definition = MODULE_DEFS[kind] || MODULE_DEFS.customSecret;
  return language === 'en' ? definition.en : definition.zh;
}

function syncModuleDraft() {
  const rows = $$('.module-row', $('#module-list'));
  if (!rows.length) return;
  currentModules = rows.map((row, index) => {
    row.dataset.moduleIndex = String(index);
    return {
      id: row.dataset.moduleId || undefined,
      kind: row.dataset.kind,
      name: $('[data-module-name]', row)?.value ?? row.dataset.name ?? '',
      value: $('[data-module-value]', row)?.value ?? '',
      alternateValues: $$('[data-module-alternate-value]', row).map(input => input.value),
      secretValue: $('[data-secret-value]', row)?.value ?? '',
      configured: row.dataset.configured === 'true',
      existing: row.dataset.existing === 'true',
      privateKeyName: row.dataset.privateKeyName || '',
      pending: row.dataset.pending || '',
      pendingKind: row.dataset.pendingKind || '',
      fingerprint: row.dataset.keyFingerprint || '',
      agentVisible: $('[data-module-agent-visible]', row)?.getAttribute('aria-pressed') === 'true',
    };
  });
}

function renderModules() {
  renderSshRoute();
  renderHttpAuthentication();
  finishModulePointerDrag(false);
  const container = $('#module-list');
  if (!currentModules.length) {
    container.innerHTML = `<div class="module-empty"><b>EMPTY / DRAFT</b><span>${localized('选择模板或添加任意模块', 'Choose a template or add any module')}</span></div>`;
    syncModuleDraft();
    syncModuleKeyControls();
    return;
  }
  container.innerHTML = currentModules.map((module, index) => {
    const definition = MODULE_DEFS[module.kind] || MODULE_DEFS.customSecret;
    const agentVisible = typeof module.agentVisible === 'boolean' ? module.agentVisible : defaultModuleAgentVisible(module.kind);
    const { label: agentVisibilityLabel, status: agentVisibilityStatus } = moduleAgentVisibilityText(agentVisible);
    const secretName = moduleSecretName(module);
    const nameControl = module.kind === 'customSecret'
      ? `<input class="input module-name-input" data-module-name value="${escapeHtml(module.name || '')}" placeholder="${localized('字段名称', 'FIELD NAME')}" />`
      : `<strong>${escapeHtml(moduleLabel(module.kind))}</strong>`;
    const secretCaption = secretName && !PERSONAL_FIELDS.includes(module.kind) && module.kind !== 'customSecret' ? `<small>${escapeHtml(secretName)}</small>` : '';
    const customName = `${nameControl}<div class="module-metadata">${secretCaption}<span class="module-visibility-state" data-module-visibility aria-hidden="true">${agentVisibilityStatus}</span></div>`;
    let control = '';
    let actions = '';
    const ociKey = module.pendingKind === 'oci' || moduleKeyInput?.moduleId === module.id || (editorHttpAuthType === 'oci-signature' && editorOciBindings?.privateKey === module.id);
    if (module.kind === 'privateKey' || ociKey) {
      const keyState = module.pending ? (module.pendingKind === 'oci' ? localized('RSA 私钥待保存', 'RSA key pending save') : module.privateKeyName) : module.configured ? (module.privateKeyName || 'IMPORTED KEY') : localized('尚未导入', 'NOT IMPORTED');
      const sources = [
        ...(ociKey ? [] : [['generate', localized('生成新密钥', 'Generate new key')]]),
        ['file', localized('导入密钥文件', 'Import key file')],
        ['clipboard', localized('从剪贴板粘贴密钥', 'Paste key from clipboard')],
        ...(ociKey ? [['input', localized('手动输入密钥', 'Enter key manually')]] : []),
      ];
      const keyTitle = module.fingerprint ? `${keyState} · ${module.fingerprint}` : keyState;
      control = `<div class="module-key-control"><span title="${escapeHtml(keyTitle)}">${escapeHtml(keyTitle)}</span><details class="key-source-menu"><summary class="copy-secret-button">${localized('添加私钥', 'Add private key')}</summary><div class="key-source-options">${sources.map(([source, label]) => `<button type="button" data-stage-key="${source}">${label}</button>`).join('')}</div></details></div>`;
      if (module.pending) control += `<button class="small-button" type="button" data-cancel-pending-key>${localized('取消替换', 'Cancel replacement')}</button>`;
      if (!ociKey && (module.pending || (module.configured && editorExistingItem?.id))) {
        control += `<button class="small-button" type="button" data-copy-public-key>${localized('复制公钥', 'Copy public key')}</button>`;
      }
      if (module.secretValue && !module.pending) {
        control += `<div class="secret-input-control module-secret-control multiline module-key-secret"><textarea class="input textarea secret-masked" data-secret-value readonly>${escapeHtml(module.secretValue)}</textarea></div>`;
      } else {
        control += `<input type="hidden" data-secret-value value="${escapeHtml(module.secretValue || '')}" />`;
      }
      if (moduleKeyInput?.moduleId === module.id) control += `<div class="module-key-input"><label class="field"><span>${localized('RSA 私钥 PEM', 'RSA private key PEM')}</span><textarea class="input textarea" data-key-input rows="6" maxlength="65536" autocomplete="off" autocapitalize="off" spellcheck="false">${escapeHtml(moduleKeyInput.text)}</textarea></label><p>${localized('未加密的 RSA PEM，至少 2048 位。确认后暂存，保存项目后生效。', 'Unencrypted RSA PEM, at least 2048 bits. Confirm to stage, then save the item to apply.')}</p><div class="button-row"><button type="button" class="small-button" data-confirm-key-input>${localized('确认导入', 'Import key')}</button><button type="button" class="small-button" data-cancel-key-input>${localized('取消输入', 'Cancel input')}</button></div></div>`;
      const keyActionsDisabled = !module.secretValue || Boolean(module.pending);
      actions = `${secretActionButton('reveal', true, keyActionsDisabled)}${secretActionButton('copy', true, keyActionsDisabled)}`;
    } else if (definition.secret) {
      const placeholder = module.kind === 'totp' ? localized('Base32 设置密钥', 'BASE32 SETUP KEY') : localized('输入秘密值', 'ENTER SECRET VALUE');
      control = definition.multiline
        ? `<div class="secret-input-control module-secret-control multiline"><textarea class="input textarea secret-masked" data-secret-value rows="6" maxlength="65536" autocomplete="off" placeholder="${placeholder}">${escapeHtml(module.secretValue || '')}</textarea></div>`
        : `<div class="secret-input-control module-secret-control"><input class="input" data-secret-value type="password" autocomplete="off" value="${escapeHtml(module.secretValue || '')}" placeholder="${placeholder}" /></div>`;
      actions = `${secretActionButton('reveal')}${secretActionButton('copy')}`;
      if (module.kind === 'password') control += `<button class="small-button" type="button" data-generate-password>${localized('生成密码', 'GENERATE')}</button>`;
      if (module.kind === 'totp') control += `<button class="small-button" type="button" data-import-totp-qr>${localized('导入二维码图片', 'Import QR image')}</button>`;
    } else if (module.kind === 'host' || module.kind === 'url') {
      const addresses = [module.value || '', ...(module.alternateValues || [])];
      const placeholder = module.kind === 'url' ? 'https://api.example.com/v1/' : 'host.example.com';
      control = `<div class="module-addresses">${addresses.map((address, addressIndex) => {
        const addressLabel = localized(`${moduleLabel(module.kind)} 地址 ${addressIndex + 1}`, `${moduleLabel(module.kind)} address ${addressIndex + 1}`);
        const defaultLabel = localized('设为默认', 'Make default');
        const removeAddressLabel = localized(`删除${addressLabel}`, `Remove ${addressLabel}`);
        return `<div class="module-address-line"><input class="input" ${addressIndex === 0 ? 'data-module-value' : 'data-module-alternate-value'} type="text" value="${escapeHtml(address)}" placeholder="${placeholder}" aria-label="${escapeHtml(addressLabel)}" /><button type="button" class="small-button module-address-default" data-default-address="${addressIndex}" aria-label="${escapeHtml(`${defaultLabel} ${addressLabel}`)}" aria-pressed="${addressIndex === 0}" ${addressIndex === 0 ? 'disabled' : ''}>${addressIndex === 0 ? localized('默认', 'Default') : defaultLabel}</button><button type="button" class="small-button module-address-remove" data-remove-address="${addressIndex}" aria-label="${escapeHtml(removeAddressLabel)}">×</button></div>`;
      }).join('')}<div class="module-address-buttons"><button type="button" class="small-button module-address-add" data-add-address>${localized('添加地址', 'Add address')}</button>${module.kind === 'url' ? `<button type="button" class="small-button" data-select-api-service>${localized('选择服务', 'Choose service')}</button>` : ''}</div></div>`;
      actions = `<span class="module-action-spacer" aria-hidden="true"></span>${secretActionButton('copy', false)}`;
    } else if (definition.multiline) {
      control = `<textarea class="input textarea" data-module-value rows="5" maxlength="1048576" spellcheck="false" autocomplete="off">${escapeHtml(module.value || '')}</textarea>`;
      actions = `<span class="module-action-spacer" aria-hidden="true"></span>${secretActionButton('copy', false)}`;
    } else {
      const inputType = module.kind === 'port' ? 'number' : 'text';
      const placeholder = '22';
      control = `<input class="input" data-module-value type="${inputType}" ${module.kind === 'port' ? 'min="1" max="65535"' : ''} value="${escapeHtml(module.value || '')}" placeholder="${placeholder}" />`;
      actions = `<span class="module-action-spacer" aria-hidden="true"></span>${secretActionButton('copy', false)}`;
    }
    if (editorHttpAuthType === 'oci-signature' && editorOciBindings?.region === module.id) control += `<select class="input oci-region-helper" data-fill-oci-region aria-label="${localized('选择区域填入字段', 'Choose a region to fill the field')}">${ociRegionChoices()}</select>`;
    const removeLabel = localized('删除模块', 'Remove module');
    const reorderLabel = localized('拖动调整顺序，或聚焦后按上/下方向键', 'Drag to reorder, or focus and use the Up/Down arrow keys');
    return `<div class="module-row" data-module-index="${index}" data-module-id="${escapeHtml(module.id || '')}" data-kind="${escapeHtml(module.kind)}" data-name="${escapeHtml(module.name || '')}" data-existing="${Boolean(module.existing)}" data-configured="${Boolean(module.configured)}" data-private-key-name="${escapeHtml(module.privateKeyName || '')}" data-pending="${escapeHtml(module.pending || '')}" data-pending-kind="${escapeHtml(module.pendingKind || '')}" data-key-fingerprint="${escapeHtml(module.fingerprint || '')}"><button type="button" class="module-drag-handle" data-module-drag-handle aria-label="${reorderLabel}" title="${reorderLabel}" aria-keyshortcuts="ArrowUp ArrowDown"><svg viewBox="0 0 12 20" aria-hidden="true"><path d="M4 5h.01M8 5h.01M4 10h.01M8 10h.01M4 15h.01M8 15h.01"/></svg></button><button type="button" class="module-agent-toggle" data-module-agent-visible aria-pressed="${agentVisible}" aria-label="${agentVisibilityLabel}" title="${agentVisibilityLabel}"><i aria-hidden="true"></i></button><div class="module-identity">${customName}</div><div class="module-control">${control}</div><div class="module-actions ${actions ? 'full' : 'single'}">${actions}<button type="button" class="remove-module module-action-icon" data-remove-module aria-label="${removeLabel}" title="${removeLabel}"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3 5.5h14M7.5 5.5V3.25h5v2.25M5.25 5.5l.75 11h8l.75-11M8.25 8.5v5M11.75 8.5v5"/></svg></button></div></div>`;
  }).join('');
  syncModuleDraft();
  syncModuleKeyControls();
  if (editorHttpAuthType === 'oci-signature' && !ociRegionOptions.length && !ociRegionLoading) {
    ociRegionLoading = invoke('oci_regions').then(regions => { ociRegionOptions = regions; $$('[data-fill-oci-region]').forEach(select => { select.innerHTML = ociRegionChoices(); }); }).catch(error => toast(cleanError(error), 'error')).finally(() => { ociRegionLoading = null; });
  }
}

const API_SERVICE_CATEGORIES = [
  ['', '全部服务', 'All services'], ['ai', 'AI 服务', 'AI services'],
  ['developer', '开发工具', 'Developer tools'], ['cloud', '云服务', 'Cloud services'], ['payments', '支付', 'Payments'],
];

function totpQrTarget(importer) {
  if (!ownerLockState.unlocked || importer.generation !== itemEditorGeneration) return null;
  return $$('.module-row[data-kind="totp"]', $('#module-list')).find(row => row.dataset.moduleId === importer.moduleId);
}

function resetTotpQrPreview() {
  $('#totp-qr-status').textContent = '';
  $('#totp-qr-issuer').textContent = '';
  $('#totp-qr-account').textContent = '';
  $('#totp-qr-preview').hidden = true;
  $('#totp-qr-confirm').disabled = true;
}

function closeTotpQrImport(restoreFocus = true) {
  const importer = totpQrImport;
  if (!importer) return;
  totpQrImport = null;
  $('#totp-qr-dialog').close();
  $('#totp-qr-image').value = '';
  $('#totp-qr-paste').value = '';
  resetTotpQrPreview();
  if (restoreFocus) totpQrTarget(importer)?.querySelector('[data-import-totp-qr]')?.focus({ preventScroll: true });
}

async function importTotpQrImage(file) {
  if (!file || !totpQrImport) return;
  const importer = { ...totpQrImport, uri: null };
  totpQrImport = importer;
  resetTotpQrPreview();
  const status = $('#totp-qr-status');
  status.textContent = localized('正在识别图片…', 'Reading image…');
  let uri;
  try {
    uri = await decodeTotpQrImage(file);
  } catch {
    uri = null;
  }
  if (totpQrImport !== importer) return;
  if (!uri) {
    status.textContent = localized('未找到二维码。请选择清晰的图片，大小不超过 10 MB。', 'No QR code found. Choose a clear image up to 10 MB.');
    return;
  }
  try {
    const { issuer, account } = await invoke('preview_totp_uri', { uri });
    if (totpQrImport !== importer) return;
    const row = totpQrTarget(importer);
    if (!row) { closeTotpQrImport(false); return; }
    importer.uri = uri;
    $('#totp-qr-issuer').textContent = issuer || localized('未提供', 'Not provided');
    $('#totp-qr-account').textContent = account || localized('未提供', 'Not provided');
    $('#totp-qr-preview').hidden = false;
    $('#totp-qr-confirm').disabled = false;
    status.textContent = $('[data-secret-value]', row).value
      ? localized('保存项目后，此密钥将替换原有密钥。', 'Saving the item will replace its existing setup key.')
      : localized('确认后保存项目即可使用验证码。', 'Confirm, then save the item to use the code.');
  } catch {
    if (totpQrImport === importer) {
      status.textContent = localized('此二维码不包含受支持的 TOTP 设置密钥。', 'This QR code does not contain a supported TOTP setup key.');
    }
  }
}

$('#totp-qr-choose').addEventListener('click', () => $('#totp-qr-image').click());
$('#totp-qr-image').addEventListener('change', ({ target }) => {
  const file = target.files[0];
  target.value = '';
  void importTotpQrImage(file);
});
$('#totp-qr-dialog').addEventListener('paste', event => {
  if (!totpQrImport) return;
  event.preventDefault();
  const clipboard = event.clipboardData;
  const file = [...(clipboard?.files || [])].find(file => file.type.startsWith('image/'))
    || [...(clipboard?.items || [])].find(item => item.type.startsWith('image/'))?.getAsFile();
  if (file) void importTotpQrImage(file);
  else {
    resetTotpQrPreview();
    totpQrImport = { ...totpQrImport, uri: null };
    $('#totp-qr-status').textContent = localized('剪贴板里没有图片。', 'The clipboard contains no image.');
  }
});
$('#totp-qr-paste').addEventListener('beforeinput', event => event.preventDefault());
$('#totp-qr-cancel').addEventListener('click', () => closeTotpQrImport());
$('#totp-qr-dialog').addEventListener('cancel', event => {
  event.preventDefault();
  closeTotpQrImport();
});
$('#totp-qr-confirm').addEventListener('click', () => {
  const importer = totpQrImport;
  if (!importer?.uri) return;
  const row = totpQrTarget(importer);
  if (!row) { closeTotpQrImport(false); return; }
  $('[data-secret-value]', row).value = importer.uri;
  syncModuleDraft();
  renderModules();
  closeTotpQrImport();
});

function moduleAddresses(module) {
  return [module.value || '', ...(module.alternateValues || [])];
}

function closeApiServicePicker(restoreFocus = true) {
  const picker = apiServicePicker;
  apiServicePicker = null;
  $('#api-service-modal').classList.add('hidden');
  if (restoreFocus && picker?.generation === itemEditorGeneration) {
    const button = $$('[data-select-api-service]', $('#module-list')).find(button => button.closest('.module-row').dataset.moduleId === picker.moduleId);
    button?.focus({ preventScroll: true });
  }
}

async function openApiServicePicker(button) {
  syncModuleDraft();
  const module = currentModules.find(module => module.id === button.closest('.module-row').dataset.moduleId && module.kind === 'url');
  if (!module) return;
  const picker = { moduleId: module.id, generation: itemEditorGeneration, addresses: moduleAddresses(module), target: 0, entry: null, parameters: {}, query: '', category: '', catalog: null };
  apiServicePicker = picker;
  $('#api-service-search').value = '';
  $('#api-service-modal').classList.remove('hidden');
  renderApiServicePicker();
  $('#api-service-search').focus();
  try {
    apiServiceCatalog ||= invoke('api_service_catalog').catch(error => { apiServiceCatalog = null; throw error; });
    const catalog = await apiServiceCatalog;
    if (apiServicePicker !== picker || picker.generation !== itemEditorGeneration) return;
    picker.catalog = catalog;
    renderApiServicePicker();
  } catch (error) {
    if (apiServicePicker === picker) $('#api-service-list').textContent = publicMessage(error, localized('无法读取服务目录，请关闭后重试。', 'Unable to load services. Close and try again.'));
  }
}

function renderApiServicePicker() {
  const picker = apiServicePicker;
  if (!picker) return;
  $('#api-service-category').innerHTML = API_SERVICE_CATEGORIES.map(([id, zh, en]) => `<option value="${id}" ${picker.category === id ? 'selected' : ''}>${localized(zh, en)}</option>`).join('');
  const query = picker.query.trim().toLocaleLowerCase();
  const entries = (picker.catalog || []).filter(entry => (!picker.category || entry.category === picker.category) && `${entry.name} ${entry.urlTemplate} ${entry.requirements}`.toLocaleLowerCase().includes(query));
  $('#api-service-list').innerHTML = picker.catalog
    ? entries.map(entry => `<button class="button quiet api-service-option" type="button" data-api-service="${escapeHtml(entry.id)}" aria-pressed="${picker.entry?.id === entry.id}"><strong>${escapeHtml(entry.name)}</strong><small>${escapeHtml(entry.urlTemplate)}</small></button>`).join('') || `<p>${localized('没有匹配的服务。', 'No matching services.')}</p>`
    : `<p>${localized('正在读取本机服务目录…', 'Loading local services…')}</p>`;
  const entry = picker.entry;
  $('#api-service-selection').classList.toggle('hidden', !entry);
  if (entry) {
    const hint = entry.authMode === 'external'
      ? localized('仅填写地址。OpenKeylet 尚未内置该服务的请求签名。', 'Address only. Native request signing is not implemented for this service.')
      : entry.authMode === 'aws-sigv4' && editorHttpAuthType !== 'aws-sigv4'
        ? localized('此服务需要 AWS SigV4。请选择 AWS API 模板或绑定 AWS 签名字段；填写地址不会配置凭据。', 'This service needs AWS SigV4. Choose the AWS API template or bind signing fields; filling the address does not configure credentials.')
      : entry.authMode === 'oci-signature' && editorHttpAuthType !== 'oci-signature'
        ? localized('此服务需要 OCI 签名认证。填写地址不会配置认证或私钥。', 'This service needs OCI signing. Filling the address does not configure authentication or keys.')
        : localized('仅填写服务地址，不更改凭据、认证或可见性。', 'Only fills the address. Credentials, authentication and visibility stay unchanged.');
    $('#api-service-requirements').textContent = `${hint} ${entry.requirements}`;
    $('#api-service-parameters').innerHTML = entry.parameters.map(name => `<label class="field"><span>${escapeHtml(name === 'region' ? localized('区域', 'Region') : name === 'workspace-id' ? localized('工作区 ID', 'Workspace ID') : name)}</span><input class="input" data-api-service-parameter="${escapeHtml(name)}" value="${escapeHtml(picker.parameters[name] || '')}" maxlength="120" autocomplete="off" /></label>`).join('');
    $('#api-service-target').innerHTML = picker.addresses.map((address, index) => `<option value="${index}" ${picker.target === index ? 'selected' : ''}>${index === 0 ? localized('默认地址', 'Default address') : localized(`地址 ${index + 1}`, `Address ${index + 1}`)} · ${escapeHtml(address || localized('空地址', 'Empty address'))}</option>`).join('');
  }
  renderApiServicePreview();
}

function apiServicePreview() {
  const picker = apiServicePicker;
  if (!picker?.entry) return { error: '' };
  let candidate = picker.entry.urlTemplate;
  for (const name of picker.entry.parameters) {
    const parameter = (picker.parameters[name] || '').trim();
    if (!parameter) return { error: localized('请先填写完整的地址参数。', 'Complete the address parameters first.') };
    if (!/^[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?$/.test(parameter)) return { error: localized('地址参数须为 1 至 63 位字母、数字或连字符，首尾不能是连字符。', 'Use 1 to 63 letters, numbers or hyphens, starting and ending with a letter or number.') };
    candidate = candidate.split(`<${name}>`).join(parameter);
  }
  let url;
  try {
    url = new URL(candidate);
    if (/[<>]/.test(candidate) || !['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.hash) throw Error();
  } catch { return { error: localized('服务地址不完整或无效。', 'The service address is incomplete or invalid.') }; }
  if (editorHttpAuthType === 'oci-signature') {
    const region = currentModules.find(module => module.id === editorOciBindings?.region)?.secretValue?.trim();
    if (!region) return { error: localized('请先填写当前绑定的 OCI 区域模块。', 'Fill the currently bound OCI region module first.') };
    const suffix = [`.${region}.oraclecloud.com`, `.${region}.oci.oraclecloud.com`].find(suffix => url.hostname.endsWith(suffix));
    const prefix = suffix ? url.hostname.slice(0, -suffix.length) : '';
    if (url.protocol !== 'https:' || (url.port && url.port !== '443') || !suffix || !prefix.split('.').every(label => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(label))) {
      return { error: localized('地址必须是当前绑定区域内的 OCI HTTPS 服务，填写辅助不会修改区域。', 'The address must be an OCI HTTPS endpoint in the bound region. This helper does not change the region.') };
    }
  }
  return { url: url.href, error: '' };
}

function renderApiServicePreview() {
  const picker = apiServicePicker;
  const preview = apiServicePreview();
  $('#api-service-current').value = picker?.addresses[picker.target] || '';
  $('#api-service-preview').value = preview.url || '';
  $('#api-service-error').textContent = preview.error;
  $('#api-service-add').disabled = !preview.url;
  $('#api-service-replace').disabled = !preview.url;
  $('#api-service-replace').textContent = picker?.addresses[picker.target]?.trim() ? localized('替换当前地址', 'Replace current address') : localized('填入地址', 'Fill address');
}

function applyApiServiceAddress(add) {
  const picker = apiServicePicker;
  if (!picker || picker.generation !== itemEditorGeneration) return closeApiServicePicker(false);
  syncModuleDraft();
  const module = currentModules.find(module => module.id === picker.moduleId && module.kind === 'url');
  if (!module || JSON.stringify(moduleAddresses(module)) !== JSON.stringify(picker.addresses)) {
    $('#api-service-error').textContent = localized('目标地址已变化，请重新选择目标地址后再应用。', 'Target addresses changed. Refresh the target addresses before applying.');
    return;
  }
  const preview = apiServicePreview();
  if (!preview.url) return renderApiServicePreview();
  const canonical = value => { const trimmed = value.trim(); try { return new URL(trimmed.includes('://') ? trimmed : `https://${trimmed}`).href; } catch { return trimmed; } };
  if (picker.addresses.some((address, index) => canonical(address) === preview.url && (add || index !== picker.target))) {
    $('#api-service-error').textContent = localized('该地址已经存在，没有新增或更改默认地址。', 'This address already exists. No address or default was changed.');
    return;
  }
  const addresses = [...picker.addresses];
  if (add && addresses.some(address => address.trim())) addresses.push(preview.url);
  else addresses[picker.target] = preview.url;
  [module.value, ...module.alternateValues] = addresses;
  closeApiServicePicker(false);
  renderModules();
  const row = $$('.module-row', $('#module-list')).find(row => row.dataset.moduleId === module.id);
  const index = add && addresses.length > picker.addresses.length ? addresses.length - 1 : picker.target;
  $$('[data-module-value], [data-module-alternate-value]', row)[index]?.focus();
}

function ociRegionChoices() {
  return `<option value="">${localized('选择区域填入字段', 'Choose a region to fill the field')}</option>${ociRegionOptions.map(region => `<option value="${escapeHtml(region)}">${escapeHtml(region)}</option>`).join('')}`;
}

function secretActionButton(kind, secret = true, disabled = false) {
  const disabledAttribute = disabled ? ' disabled' : '';
  if (kind === 'reveal') {
    const label = localized('显示明文', 'Show secret');
    return `<button type="button" class="reveal-secret-button secret-icon-button module-action-icon" data-toggle-module-secret aria-pressed="false" aria-label="${label}" title="${label}"${disabledAttribute}><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3 10s2.4-4.75 7-4.75S17 10 17 10s-2.4 4.75-7 4.75S3 10 3 10Z"/><circle cx="10" cy="10" r="2.5"/><path class="secret-icon-slash" d="M3.5 3.5l13 13"/></svg></button>`;
  }
  const label = secret ? localized('复制隐私值', 'Copy private value') : localized('复制值', 'Copy value');
  const attribute = secret ? 'data-copy-module-secret' : 'data-copy-module-value';
  return `<button type="button" class="copy-secret-button secret-icon-button module-action-icon" ${attribute} aria-label="${label}" title="${label}"${disabledAttribute}><svg viewBox="0 0 20 20" aria-hidden="true"><rect x="7" y="7" width="10" height="10"/><path d="M14 7V3H3v11h4"/></svg></button>`;
}

function positionModuleMenu() {
  const menu = $('#module-menu');
  if (menu.classList.contains('hidden')) return;
  const buttonRect = $('#add-module-button').getBoundingClientRect();
  const gutter = 8;
  const menuWidth = Math.min(276, window.innerWidth - gutter * 2);
  menu.style.width = `${menuWidth}px`;
  menu.style.left = `${Math.round(Math.min(Math.max(gutter, buttonRect.right - menuWidth), window.innerWidth - menuWidth - gutter))}px`;
  menu.style.right = 'auto';
  menu.style.maxHeight = `${window.innerHeight - gutter * 2}px`;
  const menuHeight = Math.min(menu.scrollHeight, window.innerHeight - gutter * 2);
  const below = buttonRect.bottom + 4;
  const top = below + menuHeight <= window.innerHeight - gutter
    ? below
    : Math.max(gutter, buttonRect.top - menuHeight - 4);
  menu.style.top = `${Math.round(top)}px`;
}

function setModuleMenu(open) {
  const menu = $('#module-menu');
  const home = $('#add-module-button').closest('.module-add-wrap');
  if (open) {
    document.body.appendChild(menu);
    menu.classList.remove('hidden');
    positionModuleMenu();
  } else {
    menu.classList.add('hidden');
    home.appendChild(menu);
    menu.removeAttribute('style');
  }
  $('#add-module-button').setAttribute('aria-expanded', String(Boolean(open)));
}

function addModule(kind, options = {}) {
  syncModuleDraft();
  if (kind !== 'customSecret' && currentModules.some((module) => module.kind === kind)) {
    setModuleMenu(false);
    return toast(localized('这个模块已经存在', 'That module already exists'));
  }
  currentModules.push({
    id: crypto.randomUUID(),
    kind,
    name: kind === 'customSecret' ? '' : kind,
    value: kind === 'port' ? '22' : '',
    alternateValues: [],
    secretValue: '',
    configured: false,
    existing: false,
    privateKeyName: '',
    pending: '',
    agentVisible: defaultModuleAgentVisible(kind),
    ...options,
  });
  setModuleMenu(false);
  $('#template-picker').classList.add('hidden');
  renderModules();
  const row = $$('.module-row').at(-1);
  ($('[data-module-name]', row) || $('[data-secret-value]', row) || $('[data-module-value]', row))?.focus();
}

function sshJumpCandidate(item) {
  const configured = kind => item.modules?.some(module => module.kind === kind && (module.configured || module.value));
  const mode = item.sshAuthType || 'password';
  return item.id !== editorExistingItem?.id && item.enabled && !item.sshJumpItemId && !item.sshJumpUnresolved
    && item.host && item.port > 0 && configured('username')
    && (mode === 'certificate' ? configured('privateKey') && configured('sshCertificate') : configured(mode));
}

function selectedSshJumpId() {
  return editorSshRouteMode === 'jump' ? editorJumpSelection.replace(/^unresolved:/, '') || null : null;
}

function sshTrustReady() {
  if (!editorExistingItem) return false;
  if (editorSshRouteMode === 'jump' && !state.connections.some(item => item.id === editorJumpSelection && sshJumpCandidate(item))) return false;
  const host = currentModules.find(module => module.kind === 'host')?.value?.trim() || '';
  const port = Number(currentModules.find(module => module.kind === 'port')?.value || 22);
  return host === editorExistingItem.host && port === editorExistingItem.port
    && selectedSshJumpId() === (editorExistingItem.sshJumpItemId || null)
    && !editorJumpSelection.startsWith('unresolved:') && !editorExistingItem.sshJumpUnresolved;
}

function renderSshRoute() {
  const show = value('#connection-type') === 'ssh' || currentModules.some(module => module.kind === 'host') || editorSshRouteMode === 'jump';
  $('#ssh-route-config').classList.toggle('hidden', !show);
  value('#ssh-auth-type', editorSourceInput?.sshAuthType || '');
  $('#ssh-auth-label').textContent = localized('认证方式', 'Authentication');
  $('#ssh-trust-kind-label').textContent = localized('信任类型', 'Trust kind');
  $('#ssh-route-legend').textContent = localized('SSH 连接', 'SSH connection');
  $('#ssh-route-mode-label').textContent = localized('连接方式', 'Connection method');
  $('#ssh-route-mode option[value="direct"]').textContent = localized('直接连接', 'Direct');
  $('#ssh-route-mode option[value="jump"]').textContent = localized('通过跳板项目', 'Through a jump project');
  value('#ssh-route-mode', editorSshRouteMode);
  $('#ssh-jump-label').textContent = localized('跳板项目', 'Jump project');
  $('#ssh-jump-field').classList.toggle('hidden', editorSshRouteMode !== 'jump');
  const select = $('#ssh-jump-item');
  select.replaceChildren(new Option(localized('请选择已有项目', 'Select an existing project'), ''));
  const candidates = state.connections.filter(sshJumpCandidate);
  for (const item of candidates) select.add(new Option(`${item.name} · ${item.host}:${item.port}`, item.id));
  if (editorJumpSelection && !candidates.some(item => item.id === editorJumpSelection)) select.add(new Option(localized('跳板不可用，请重新选择', 'Jump unavailable. Select again.'), editorJumpSelection));
  select.value = editorJumpSelection;
  const jump = candidates.find(item => item.id === editorJumpSelection);
  $('#ssh-route-path').textContent = editorSshRouteMode === 'jump'
    ? localized(`本机 → ${jump?.name || '跳板不可用'} → ${value('#connection-name') || '目标项目'}`, `Local → ${jump?.name || 'Jump unavailable'} → ${value('#connection-name') || 'Target'}`)
    : localized('从本机直接连接目标。', 'Connect directly from this machine.');
  $('#ssh-trust-summary').textContent = localized('本机主机身份信任', 'Local host identity');
  $('#ssh-trust-note').textContent = sshTrustReady()
    ? localized(`以下操作立即应用到已保存的 ${editorExistingItem.host}:${editorExistingItem.port}，不随项目取消撤销，也不随备份迁移。请先从可信来源核对指纹。`, `These actions immediately affect saved ${editorExistingItem.host}:${editorExistingItem.port}. Closing the editor does not undo them. Trust is device-local. Verify the fingerprint against a trusted source.`)
    : localized('请先保存项目的主机、端口和跳板配置，再管理主机信任。', 'Save the host, port, and jump configuration before managing trust.');
  $('#ssh-trust-fingerprint-label').textContent = localized('SHA256 主机指纹', 'SHA256 host fingerprint');
  $('#ssh-trust-verified-label').textContent = localized('我已通过可信来源核对该指纹。', 'I verified this fingerprint against a trusted source.');
  $('#ssh-trust-probe').textContent = localized('读取候选指纹', 'Read candidate fingerprint');
  $('#ssh-trust-save').textContent = localized('确认并保存本机信任', 'Confirm and save local trust');
  $('#ssh-trust-remove').textContent = localized('移除本机信任', 'Remove local trust');
  for (const id of ['ssh-trust-probe','ssh-trust-save','ssh-trust-remove']) $(`#${id}`).disabled = !sshTrustReady();
}

async function refreshSshTrust() {
  if (!sshTrustReady()) return;
  const id = editorExistingItem.id;
  const entries = await api.sshTrust();
  if (editorExistingItem?.id !== id) return;
  const entry = entries.find(entry => entry.itemId === id && entry.host === editorExistingItem.host.trim().replace(/^\[|\]$/g, '').toLowerCase() && entry.port === editorExistingItem.port);
  if (entry) value('#ssh-trust-kind', entry.kind || 'hostKey');
  $('#ssh-trust-status').textContent = entry ? localized(`已信任 ${entry.fingerprint}`, `Trusted ${entry.fingerprint}`) : localized('尚未信任。连接会在认证前拒绝。', 'Not trusted. Connections stop before authentication.');
}

$('#ssh-auth-type').addEventListener('change', () => { editorSourceInput = { ...editorSourceInput, sshAuthType: value('#ssh-auth-type') }; });
$('#ssh-trust-kind').addEventListener('change', () => { $('#ssh-trust-verified').checked = false; value('#ssh-trust-fingerprint', ''); });
$('#ssh-route-mode').addEventListener('change', () => { editorSshRouteMode = value('#ssh-route-mode'); renderSshRoute(); });
$('#ssh-jump-item').addEventListener('change', () => { editorJumpSelection = value('#ssh-jump-item'); renderSshRoute(); });
$('#connection-form').addEventListener('input', event => {
  if (event.target.closest('#module-list') || event.target.id === 'connection-name') { syncModuleDraft(); renderSshRoute(); }
});
$('#connection-type').addEventListener('change', renderSshRoute);
$('#ssh-trust-details').addEventListener('toggle', () => { if ($('#ssh-trust-details').open) void refreshSshTrust().catch(error => toast(cleanError(error), 'error')); });
$('#ssh-trust-probe').addEventListener('click', async () => {
  try {
    if (!sshTrustReady()) return;
    const savedItem = editorExistingItem;
    const result = await api.probeSshHost(savedItem.id);
    if (editorExistingItem !== savedItem || !sshTrustReady()) return;
    value('#ssh-trust-fingerprint', value('#ssh-trust-kind') === 'certificateAuthority' ? result.caFingerprint || '' : result.fingerprint);
    $('#ssh-trust-verified').checked = false;
    $('#ssh-trust-status').textContent = localized('候选指纹尚未验证，读取操作不会自动信任。', 'Candidate fingerprint is unverified. Reading it does not establish trust.');
  } catch (error) { toast(cleanError(error), 'error'); }
});
$('#ssh-trust-fingerprint').addEventListener('input', () => { $('#ssh-trust-verified').checked = false; });
$('#ssh-trust-save').addEventListener('click', async () => {
  try {
    if (!sshTrustReady()) return;
    if (!$('#ssh-trust-verified').checked) throw new Error(localized('请先从可信来源核对指纹并勾选确认。', 'Verify the fingerprint against a trusted source and check the confirmation.'));
    await invoke(value('#ssh-trust-kind') === 'certificateAuthority' ? 'set_ssh_host_ca_trust' : 'set_ssh_host_trust', { id: editorExistingItem.id, host: editorExistingItem.host, port: editorExistingItem.port, fingerprint: value('#ssh-trust-fingerprint').trim() });
    $('#ssh-trust-verified').checked = false;
    await refreshSshTrust();
    toast(localized('已保存本机信任。项目修改仍需点击保存。', 'Local trust saved. Item changes still require Save.'));
  } catch (error) { toast(cleanError(error), 'error'); }
});
$('#ssh-trust-remove').addEventListener('click', async () => {
  try {
    if (!sshTrustReady()) return;
    await api.setSshTrust(editorExistingItem.id, editorExistingItem.host, editorExistingItem.port, null);
    await refreshSshTrust();
  } catch (error) { toast(cleanError(error), 'error'); }
});

const OCI_BINDING_FIELDS = [
  ['region', '区域', 'Region'],
  ['serviceUrl', '服务 URL', 'Service URL'],
  ['tenancyOcid', '租户 OCID', 'Tenancy OCID'],
  ['userOcid', '用户 OCID', 'User OCID'],
  ['fingerprint', '公钥指纹', 'Public key fingerprint'],
  ['privateKey', 'RSA 签名私钥', 'RSA signing private key'],
];

const AWS_BINDING_FIELDS = [
  ['region', '签名区域', 'Signing region'],
  ['serviceUrl', '服务 URL', 'Service URL'],
  ['accessKeyId', 'Access Key ID', 'Access Key ID'],
  ['secretAccessKey', 'Secret Access Key', 'Secret Access Key'],
  ['sessionToken', 'Session Token，可选', 'Session Token, optional'],
];

function renderAuthBindingFields(fields, bindings, attribute) {
  return fields.map(([role, zh, en]) => {
    const bound = bindings?.[role] || '';
    const missing = bound && !currentModules.some(module => module.id === bound);
    const candidates = currentModules.filter(module => role === 'serviceUrl' ? module.kind === 'url' : moduleHasPlaintextReveal(module.kind));
    return `<label class="field"><span>${localized(zh, en)}</span><select class="input" ${attribute}="${role}"><option value="">${localized('未绑定', 'Not bound')}</option>${missing ? `<option value="${escapeHtml(bound)}" selected>${localized('模块已删除', 'Module removed')}</option>` : ''}${candidates.map(module => `<option value="${escapeHtml(module.id)}" ${module.id === bound ? 'selected' : ''}>${escapeHtml(module.kind === 'customSecret' ? module.name || localized('未命名字段', 'Unnamed field') : moduleLabel(module.kind))}</option>`).join('')}</select></label>`;
  }).join('');
}

function renderHttpAuthentication() {
  const select = $('#http-auth-type');
  if (!select) return;
  const selected = editorHttpAuthType || 'auto';
  const existingModes = [...new Set([selected, editorSourceInput?.httpAuthType])].filter(mode => mode && !['auto', 'oci-signature', 'aws-sigv4'].includes(mode));
  select.innerHTML = `<option value="auto">${localized('自动', 'Automatic')}</option><option value="oci-signature">OCI API</option><option value="aws-sigv4">AWS SigV4</option>${existingModes.map(mode => `<option value="${escapeHtml(mode)}">${escapeHtml(mode)}</option>`).join('')}`;
  select.value = selected;
  $('#oci-bindings').classList.toggle('hidden', selected !== 'oci-signature');
  $('#oci-binding-fields').innerHTML = renderAuthBindingFields(OCI_BINDING_FIELDS, editorOciBindings, 'data-oci-binding');
  $('#aws-bindings').classList.toggle('hidden', selected !== 'aws-sigv4');
  $('#aws-bindings-summary').textContent = localized('AWS 签名模块', 'AWS signing modules');
  $('#aws-bindings-note').textContent = localized('区域须与服务 URL 一致。绑定已有字段，凭据在本机签名；临时凭据还需 Session Token。不自动获取或续期凭据。', 'Region must match the service URL. Bind ordinary fields for local signing; temporary credentials require a Session Token. Credentials are not acquired or refreshed automatically.');
  $('#aws-binding-fields').innerHTML = renderAuthBindingFields(AWS_BINDING_FIELDS, editorAwsBindings, 'data-aws-binding');
}

function syncModuleKeyControls() {
  $('#save-connection-button').disabled = Boolean(moduleKeyStaging || moduleKeyInput);
  $$('.module-key-input button, .module-key-input textarea, [data-stage-key]').forEach(control => { control.disabled = Boolean(moduleKeyStaging); });
}

function discardModuleKey(module) {
  return module.pendingKind === 'oci' ? invoke('discard_oci_key', { id: module.pending }) : api.discardKey(module.pending);
}

function updateOciModuleUrl() {
  const region = currentModules.find(module => module.id === editorOciBindings?.region)?.secretValue.trim();
  const urlModule = currentModules.find(module => module.id === editorOciBindings?.serviceUrl);
  if (!urlModule || (urlModule.value && urlModule.value !== editorDerivedOciUrl)) return;
  editorDerivedOciUrl = region ? `https://iaas.${region}.oraclecloud.com/20160918/` : '';
  urlModule.value = editorDerivedOciUrl;
  const input = $(`[data-module-id="${urlModule.id}"] [data-module-value]`, $('#module-list'));
  if (input) input.value = urlModule.value;
}

async function stageModuleKey(row, source, text = null) {
  if (moduleKeyStaging) return;
  syncModuleDraft();
  const moduleId = row.dataset.moduleId;
  const oci = source === 'input' || currentModules.find(module => module.id === moduleId)?.pendingKind === 'oci' || (editorHttpAuthType === 'oci-signature' && editorOciBindings?.privateKey === moduleId);
  if (oci && currentModules.some(module => module.pendingKind === 'oci' && module.id !== moduleId)) return toast(localized('请先保存或取消其他模块中待保存的 RSA 私钥。', 'Save or cancel the pending RSA key in the other module first.'), 'error');
  const token = { moduleId };
  moduleKeyStaging = token;
  syncModuleKeyControls();
  try {
    const preview = oci
      ? await invoke('stage_oci_key', { source, text })
      : await api.stageKey(source, currentModules.find(module => module.kind === 'passphrase')?.secretValue || null);
    if (!preview) return;
    const pending = oci ? preview : preview.id;
    if (token.cancelled || moduleKeyStaging !== token || !currentModules.some(module => module.id === moduleId) || $('#connection-modal').classList.contains('hidden')) {
      await discardModuleKey({ pending, pendingKind: oci ? 'oci' : '' });
      return;
    }
    syncModuleDraft();
    const module = currentModules.find(module => module.id === moduleId);
    if (!module) return;
    if (module.pending) await discardModuleKey(module);
    Object.assign(module, { pending, pendingKind: oci ? 'oci' : '', configured: true, privateKeyName: oci ? localized('RSA 私钥待保存', 'RSA key pending save') : preview.name, fingerprint: oci ? '' : preview.fingerprint });
    moduleKeyInput = null;
    renderModules();
  } catch (error) { toast(cleanError(error), 'error'); }
  finally {
    if (moduleKeyStaging === token) moduleKeyStaging = null;
    syncModuleKeyControls();
  }
}

$('#module-list').addEventListener('keydown', event => {
  if (event.key !== 'Escape' || !event.target.closest('.module-key-input')) return;
  event.preventDefault();
  event.stopPropagation();
  if (!moduleKeyStaging) {
    moduleKeyInput = null;
    syncModuleDraft();
    renderModules();
  }
});

function applyTemplate(template) {
  editorHttpAuthType = template === 'oci' ? 'oci-signature' : 'auto';
  editorOciBindings = null;
  editorAwsBindings = null;
  editorDerivedOciUrl = '';
  if (template === 'aws') {
    editorHttpAuthType = 'aws-sigv4';
    const fields = [['serviceUrl', 'url'], ['region', 'aws_region'], ['accessKeyId', 'aws_access_key_id'], ['secretAccessKey', 'aws_secret_access_key'], ['sessionToken', 'aws_session_token']];
    currentModules = fields.map(([role, name]) => ({ id: crypto.randomUUID(), kind: role === 'serviceUrl' ? 'url' : 'customSecret', name, value: role === 'serviceUrl' ? 'https://ec2.us-east-1.amazonaws.com/' : '', alternateValues: [], secretValue: role === 'region' ? 'us-east-1' : '', configured: false, existing: false, privateKeyName: '', pending: '', agentVisible: ['region', 'serviceUrl'].includes(role) }));
    editorAwsBindings = Object.fromEntries(fields.map(([role], index) => [role, currentModules[index].id]));
    value('#connection-type', 'api');
    $('#template-picker').classList.add('hidden');
    renderModules();
    return;
  }
  if (template === 'oci') {
    const fields = [['region', 'region'], ['serviceUrl', 'url'], ['tenancyOcid', 'tenancy_ocid'], ['userOcid', 'user_ocid'], ['fingerprint', 'fingerprint'], ['privateKey', 'privateKey']];
    currentModules = fields.map(([role, name]) => ({ id: crypto.randomUUID(), kind: role === 'serviceUrl' ? 'url' : role === 'privateKey' ? 'privateKey' : 'customSecret', name, value: '', alternateValues: [], secretValue: '', configured: false, existing: false, privateKeyName: '', pending: '', agentVisible: role === 'serviceUrl' }));
    editorOciBindings = Object.fromEntries(fields.map(([role], index) => [role, currentModules[index].id]));
    value('#connection-type', 'api');
    $('#template-picker').classList.add('hidden');
    renderModules();
    return;
  }
  const presets = {
    login: ['url', 'username', 'password'],
    card: ['cardholderName', 'cardNumber', 'cardBrand', 'cardExpMonth', 'cardExpYear', 'cardSecurityCode'],
    identity: ['firstName', 'lastName', 'email', 'phone', 'address1', 'address2', 'city', 'state', 'postalCode', 'country'],
    secureNote: ['secureNote'],
    ssh: ['host', 'port', 'username', 'password'],
    api: ['url', 'apiCredential'],
    sshCertificate: ['host', 'port', 'username', 'privateKey', 'sshCertificate'],
    tlsCertificate: ['privateKey', 'publicKey', 'certificate', 'csr', 'caCertificate'],
    sshAuthority: ['privateKey', 'publicKey'],
    tlsAuthority: ['privateKey', 'caCertificate'],
    blank: [],
  };
  value('#connection-type', template === 'sshCertificate' ? 'ssh' : ['blank', 'tlsCertificate', 'sshAuthority', 'tlsAuthority'].includes(template) ? 'custom' : template);
  if (template === 'sshCertificate') editorSourceInput = { ...editorSourceInput, sshAuthType: 'certificate' };
  currentModules = (presets[template] || []).map((kind) => ({ id: crypto.randomUUID(), kind, name: kind, value: kind === 'port' ? '22' : '', alternateValues: [], secretValue: '', configured: false, existing: false, privateKeyName: '', pending: '', agentVisible: defaultModuleAgentVisible(kind) }));
  $('#template-picker').classList.add('hidden');
  renderModules();
}

function value(selector, next) {
  const element = $(selector);
  if (arguments.length > 1) element.value = next ?? '';
  return element.value;
}

function updateEditorTitle() {
  $('#modal-title').textContent = editorOpenedFromDraft
    ? (editorExistingItem ? localized('继续编辑', 'CONTINUE EDIT') : localized('继续草稿', 'CONTINUE DRAFT'))
    : (editorExistingItem ? localized('编辑项目', 'EDIT ITEM') : localized('添加项目', 'ADD ITEM'));
}

function placeItemEditor() {
  const editor = $('#connection-modal');
  const card = $$('.connection-card').find(card => card.dataset.itemId === inlineEditorItemId);
  if (card) {
    card.appendChild(editor);
    card.classList.add('is-editing');
    const trigger = $('.item-disclosure', card);
    trigger.setAttribute('aria-expanded', 'true');
    trigger.setAttribute('aria-label', localized(`收起编辑 ${editorExistingItem.name}`, `Collapse editor for ${editorExistingItem.name}`));
    trigger.title = localized('收起编辑', 'Collapse editor');
    $('.connection-name', card).setAttribute('aria-expanded', 'true');
    editorReturnFocus = editorReturnFocus?.matches('.connection-name') ? $('.connection-name', card) : trigger;
  } else editorHome.after(editor);
  editor.classList.toggle('inline-editor', Boolean(card));
  editor.setAttribute('role', card ? 'region' : 'dialog');
  if (card) editor.removeAttribute('aria-modal');
  else editor.setAttribute('aria-modal', 'true');
}

async function animateInlineEditor(open) {
  const editor = $('#connection-modal');
  if (!inlineEditorItemId || editor.classList.contains('hidden') || matchMedia('(prefers-reduced-motion: reduce)').matches) return true;
  const height = editor.getBoundingClientRect().height;
  editorFoldAnimation?.cancel();
  editor.classList.add('is-folding');
  // Animate the real content height: modules and multiline fields need no cap.
  const animation = editor.animate([
    { height: `${open ? 0 : height}px`, opacity: open ? 0 : 1 },
    { height: `${open ? height : 0}px`, opacity: open ? 1 : 0 },
  ], { duration: 220, easing: 'cubic-bezier(.2,.8,.2,1)', fill: 'both' });
  editorFoldAnimation = animation;
  let completed = true;
  try { await animation.finished; } catch (_) { completed = false; }
  if (editorFoldAnimation === animation) {
    animation.cancel();
    editorFoldAnimation = null;
    editor.classList.remove('is-folding');
  }
  return completed;
}

async function openEditor(item = null, draft = null) {
  if (editorOpening) return false;
  if (!ownerLockState.unlocked) return false;
  const epoch = ownerEpoch;
  editorOpening = true;
  try {
    const launcher = document.activeElement;
    const collapseCurrent = inlineEditorItemId && inlineEditorItemId === item?.id && !draft;
    if (!$('#connection-modal').classList.contains('hidden') && !await closeEditorWithDraft()) return false;
    if (collapseCurrent) return true;
    if (!draft && item) draft = editorDrafts.find((candidate) => candidate.input?.id === item.id) || null;
    if (!item && draft?.input?.id) item = state.connections.find((candidate) => candidate.id === draft.input.id) || null;
    let ownerValues = {};
    if (draft) {
      ownerValues = draftSecretValues(draft.input?.secrets);
    } else if (item) {
      try {
        const view = await api.ownerSecrets(item.id);
        ownerValues = Object.fromEntries(view.fields.map((field) => [field.name, field.value]));
      } catch (error) {
        toast(cleanError(error), 'error');
        await refreshOwnerLock(false);
        return false;
      }
    }
    if (epoch !== ownerEpoch) return false;
    const source = draft?.input || item || null;
    removedSecretFields = new Set();
    itemEditorGeneration++;
    editorExistingItem = item;
    editorSourceInput = source;
    editorHttpAuthType = source?.httpAuthType || 'auto';
    editorOciBindings = source?.ociBindings ? structuredClone(source.ociBindings) : null;
    editorAwsBindings = source?.awsBindings ? structuredClone(source.awsBindings) : null;
    moduleKeyInput = null;
    editorDerivedOciUrl = '';
    editorJumpSelection = source?.sshJumpUnresolved ? `unresolved:${source?.sshJumpItemId || ''}` : source?.sshJumpItemId || '';
    editorSshRouteMode = source?.sshJumpItemId || source?.sshJumpUnresolved ? 'jump' : 'direct';
    $('#ssh-trust-details').open = false;
    $('#ssh-trust-fingerprint').value = '';
    $('#ssh-trust-verified').checked = false;
    $('#ssh-trust-status').textContent = '';
    editorOpenedFromDraft = Boolean(draft);
    currentDraftId = draft?.id || '';
    value('#connection-id', item?.id);
    value('#connection-name', source?.name);
    value('#connection-description', source?.description);
    $('#connection-type').innerHTML = typeOptions('', source?.itemType || 'custom');
    value('#connection-folder', source?.folder);
    renderLoginUris(source?.loginUris || []);
    $('#vault-folder-options').innerHTML = [...new Set(state.connections.map(i => i.folder).filter(Boolean))].map(f => `<option value="${escapeHtml(f)}"></option>`).join('');
    $('#connection-enabled').checked = source?.enabled ?? true;
    updateEditorTitle();
    $('#template-picker').classList.toggle('hidden', Boolean(item || draft?.input?.modules?.length));
    currentModules = (source?.modules || []).map((module) => {
      const secretName = module.kind === 'customSecret' ? module.name : module.kind;
      const privateKeyName = draft?.input?.secrets?.privateKeyName || item?.privateKeyName || '';
      return { ...module, id: module.id || crypto.randomUUID(), alternateValues: [...(module.alternateValues || [])], secretValue: ownerValues[secretName] || '', configured: Boolean(module.configured || ownerValues[secretName]), existing: Boolean(item), privateKeyName: module.kind === 'privateKey' ? privateKeyName : '', pending: '', agentVisible: typeof module.agentVisible === 'boolean' ? module.agentVisible : defaultModuleAgentVisible(module.kind) };
    });
    const regionModule = currentModules.find(module => module.id === editorOciBindings?.region);
    const urlModule = currentModules.find(module => module.id === editorOciBindings?.serviceUrl);
    const defaultUrl = regionModule?.secretValue ? `https://iaas.${regionModule.secretValue}.oraclecloud.com/20160918/` : '';
    editorDerivedOciUrl = urlModule?.value === defaultUrl ? defaultUrl : '';
    renderModules();
    editorReturnFocus = launcher;
    inlineEditorItemId = item?.id || null;
    if (inlineEditorItemId) renderConnections();
    placeItemEditor();
    $('#connection-modal').classList.remove('hidden');
    $('#passkey-list').innerHTML = (item?.passkeys || []).map(key => `<div class="passkey-summary"><strong>PASSKEY · ${escapeHtml(key.rpId)}</strong><span>${escapeHtml(key.displayName || key.userName)}</span><small>${localized('私钥加密保存在本地；在浏览器中使用。移入回收站会停用 Passkey，恢复后可继续使用。', 'Private key encrypted locally. Moving this item to trash disables its passkeys; restore it to use them again.')}</small></div>`).join('');
    $('.form-scroll', $('#connection-modal')).scrollTop = 0;
    editorInitialSnapshot = JSON.stringify(serializeItem(false));
    void animateInlineEditor(true).then(completed => {
      if (!completed) return;
      requestAnimationFrame(() => {
        if (!$('#connection-modal').classList.contains('hidden') && editorExistingItem === item) {
          if ($('#connection-modal').contains(document.activeElement)) return;
          if (inlineEditorItemId) $('#connection-name').scrollIntoView({ block: 'nearest' });
          $('#connection-name').focus({ preventScroll: true });
        }
      });
    });
    return true;
  } finally { editorOpening = false; }
}

function closeEditor() {
  itemEditorGeneration++;
  closeTotpQrImport(false);
  closeApiServicePicker(false);
  moduleKeyInput = null;
  if (moduleKeyStaging) moduleKeyStaging.cancelled = true;
  for (const module of currentModules) {
    if (module.pending) void discardModuleKey(module).catch(() => {});
  }
  syncModuleKeyControls();
  editorFoldAnimation?.cancel();
  editorFoldAnimation = null;
  $('#connection-modal').classList.remove('is-folding');
  finishModulePointerDrag(false);
  $('#connection-modal').classList.add('hidden');
  setModuleMenu(false);
  const card = $$('.connection-card').find(card => card.dataset.itemId === inlineEditorItemId);
  if (card) {
    card.classList.remove('is-editing');
    const trigger = $('.item-disclosure', card);
    trigger.setAttribute('aria-expanded', 'false');
    trigger.setAttribute('aria-label', localized(`展开编辑 ${editorExistingItem.name}`, `Expand editor for ${editorExistingItem.name}`));
    trigger.title = localized('展开编辑', 'Expand editor');
    $('.connection-name', card).setAttribute('aria-expanded', 'false');
    if (!editorReturnFocus?.isConnected || !card.contains(editorReturnFocus)) editorReturnFocus = trigger;
  }
  inlineEditorItemId = null;
  placeItemEditor();
  editorSourceInput = null;
  editorOpenedFromDraft = false;
  editorInitialSnapshot = '';
  if (editorReturnFocus?.isConnected && !editorReturnFocus.closest('.hidden')) editorReturnFocus.focus({ preventScroll: true });
  editorReturnFocus = null;
}

function collectModules(validate = true) {
  syncModuleDraft();
  const modules = [];
  const secrets = { namedSecrets: {} };
  const secretNames = new Set();
  for (const module of currentModules) {
    const name = moduleSecretName(module);
    if (module.kind === 'customSecret') {
      if (validate && !name) throw new Error(localized('请输入字段名称', 'Enter a field name'));
      const normalizedName = name.toLocaleLowerCase().replace(/[\s_-]+/g, '');
      if (validate && [...Object.keys(MODULE_DEFS).map(k => k.toLowerCase()), 'token', 'apikey'].includes(normalizedName)) throw new Error(localized(`字段名与内置模块冲突：${name}`, `Field name conflicts with a built-in module: ${name}`));
    }
    if (validate && (module.kind === 'host' || module.kind === 'url')) {
      const addresses = [String(module.value || '').trim(), ...(module.alternateValues || []).map(address => String(address).trim())];
      const seen = new Set();
      for (const address of addresses) {
        if (!address && addresses.length === 1) continue;
        if (!address) throw new Error(localized('请填写地址，或删除空白地址行', 'Enter an address or remove the empty address row'));
        let normalized;
        if (module.kind === 'host') {
          if (/\s/.test(address)) throw new Error(localized('主机地址不能包含空白字符', 'Host addresses cannot contain whitespace'));
          normalized = address.replace(/^\[|\]$/g, '').toLowerCase();
        } else {
          let url;
          try { url = new URL(address.includes('://') ? address : `https://${address}`); }
          catch { throw new Error(localized('请输入有效的服务 URL', 'Enter a valid service URL')); }
          if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.username || url.password) throw new Error(localized('服务 URL 必须使用 HTTP/HTTPS，且不能包含账号密码', 'Service URLs must use HTTP/HTTPS without embedded credentials'));
          normalized = url.href;
        }
        if (seen.has(normalized)) throw new Error(localized('地址不能重复', 'Addresses must be unique'));
        seen.add(normalized);
      }
    }
    const uniqueName = name.toLocaleLowerCase().replace(/[\s_-]+/g, '');
    if (validate && name && secretNames.has(uniqueName)) throw new Error(localized(`字段名重复：${name}`, `Duplicate field name: ${name}`));
    if (name) secretNames.add(uniqueName);
    modules.push({ id: module.id, kind: module.kind, name: module.kind === 'customSecret' ? name : '', value: MODULE_DEFS[module.kind]?.secret ? '' : String(module.value || '').trim(), alternateValues: (module.alternateValues || []).map(address => String(address).trim()), agentVisible: Boolean(module.agentVisible) });
    const fieldValue = module.secretValue || '';
    if (name === 'password') secrets.password = fieldValue;
    else if (name === 'passphrase') secrets.passphrase = fieldValue;
    else if (name === 'privateKey') secrets.privateKey = fieldValue;
    else if (name) secrets.namedSecrets[name] = fieldValue;
  }
  return { modules, secrets, privateKeyImportPath: currentModules.find((module) => module.kind === 'privateKey' && module.pending && module.pendingKind !== 'oci')?.pending || '' };
}

function serializeItem(validate = true) {
  const name = value('#connection-name').trim();
  if (validate && !name) throw new Error(localized('请输入项目名称', 'Enter an item name'));
  const id = value('#connection-id') || null;
  if (validate && state.connections.some((item) => item.id !== id && item.name.trim().toLowerCase() === name.toLowerCase())) {
    throw new Error(localized('项目名称已存在，请使用其他名称', 'An item with this name already exists'));
  }
  syncModuleDraft();
  if (validate && editorHttpAuthType !== 'oci-signature' && !currentModules.length) throw new Error(localized('请至少添加一个模块', 'Add at least one module'));
  const collected = collectModules(validate);
  return {
    id,
    itemType: value('#connection-type'),
    folder: value('#connection-folder').trim(),
    favorite: Boolean(editorSourceInput?.favorite),
    loginUris: $$('#login-uri-list .login-uri-row').map(row => ({uri: $('[data-login-uri]', row).value.trim(), matchType: $('[data-login-match]', row).value})).filter(rule => rule.uri),
    modules: collected.modules,
    name,
    description: value('#connection-description'),
    enabled: $('#connection-enabled').checked,
    sshJumpItemId: editorSshRouteMode === 'jump' ? (editorJumpSelection.replace(/^unresolved:/, '') || null) : null,
    sshJumpUnresolved: editorSshRouteMode === 'jump' && (!editorJumpSelection || editorJumpSelection.startsWith('unresolved:')),
    sshAuthType: editorSourceInput?.sshAuthType || '',
    httpAuthType: editorHttpAuthType,
    ociBindings: editorHttpAuthType === 'oci-signature' ? editorOciBindings || {} : null,
    awsBindings: editorHttpAuthType === 'aws-sigv4' ? editorAwsBindings || {} : null,
    authHeader: editorSourceInput?.authHeader || '',
    authLocation: editorSourceInput?.authLocation || '',
    authPrefix: editorSourceInput?.authPrefix || '',
    apiAuthHeaders: editorSourceInput?.apiAuthHeaders || [],
    testPath: editorSourceInput?.testPath || '',
    privateKeyImportPath: collected.privateKeyImportPath,
    // A newly entered replacement must not be deleted by the earlier removal.
    removeSecretNames: [...new Set([...removedSecretFields, ...currentModules.filter(m => PERSONAL_FIELDS.includes(m.kind) && m.configured && !m.secretValue).map(moduleSecretName)])].filter((name) => !currentModules.some((module) =>
      moduleSecretName(module) === name && (module.secretValue || module.pending))),
    secrets: collected.secrets,
  };
}

function editorHasDraftContent() {
  syncModuleDraft();
  return Boolean(value('#connection-name').trim() || value('#connection-description').trim() || currentModules.length);
}

function closeEditorWithDraft() {
  if (editorClosing) return editorClosing;
  editorClosing = saveDraftAndCloseEditor().finally(() => { editorClosing = null; });
  return editorClosing;
}

async function saveDraftAndCloseEditor() {
  try {
    const input = serializeItem(false);
    input.privateKeyImportPath = '';
    const changed = JSON.stringify(input) !== editorInitialSnapshot;
    if (!editorHasDraftContent() && !editorExistingItem) {
      if (currentDraftId) await api.deleteDraft(currentDraftId);
    } else if (changed || editorOpenedFromDraft || !editorExistingItem) {
      const saved = await api.saveDraft(currentDraftId || null, input);
      currentDraftId = saved.id;
      toast(editorExistingItem
        ? localized('未保存修改已保留，可随时继续', 'Unsaved changes kept for later')
        : localized('草稿已加密保存', 'Draft saved and encrypted'));
    }
    await refreshDrafts();
    const trigger = $(`.connection-card[data-item-id="${inlineEditorItemId}"] .item-disclosure`);
    trigger?.setAttribute('aria-expanded', 'false');
    await animateInlineEditor(false);
    closeEditor();
    return true;
  } catch (error) {
    toast(cleanError(error), 'error');
    return false;
  }
}

let agentWriteBusy = false;
function renderAgentWriteSettings() {
  const folders = state.settings.agentWriteFolders || [];
  const enabled = Boolean(state.settings.agentWriteEnabled);
  $('#agent-write-title').textContent = localized('保险库写入权限', 'Vault write permissions');
  $('#agent-write-description').textContent = localized('允许修改指定文件夹内的项目内容，包括连接目标和凭据。', 'Allow changes to item content, targets and credentials in authorized folders.');
  $('#agent-write-label').textContent = localized('允许 Agent 新增和修改项目', 'Allow Agent to create and edit items');
  $('#agent-write-enabled').checked = enabled;
  $('#agent-write-enabled').disabled = agentWriteBusy;
  $('#agent-write-enabled').setAttribute('aria-label', $('#agent-write-label').textContent);
  $('#agent-write-status').textContent = !enabled ? localized('当前禁止写入', 'Writing is disabled') : !folders.length ? localized('尚未添加可写入的文件夹', 'No writable folders added') : localized('仅允许写入以下文件夹', 'Writing is limited to these folders');
  $('#agent-write-folder').placeholder = localized('选择已有文件夹或输入新名称', 'Select a folder or enter a new name');
  $('#agent-write-folder').setAttribute('aria-label', localized('可写入的文件夹', 'Writable folder'));
  $('#agent-write-folder').disabled = agentWriteBusy;
  $('#agent-write-add').textContent = localized('＋ 添加文件夹', '+ Add folder');
  $('#agent-write-add').disabled = agentWriteBusy;
  $('#agent-write-folders').innerHTML = folders.map((folder, index) => `<div class="agent-client-row"><span class="agent-client-copy">${escapeHtml(folder)}</span><button type="button" class="button quiet" data-remove-write-folder="${index}" ${agentWriteBusy ? 'disabled' : ''}>${localized('移除', 'Remove')}</button></div>`).join('');
  $('#agent-write-folder-options').innerHTML = [...new Set(state.connections.map(item => item.folder).filter(Boolean))].map(folder => `<option value="${escapeHtml(folder)}"></option>`).join('');
  $('#agent-write-boundary').textContent = localized('对连接本机的所有 Agent 生效，不限制客户端手动操作。移动时原、目标文件夹都必须可写。Agent 不能删除项目或修改字段可见性，写入结果不返回秘密。OCI 签名配置仍需在客户端编辑。', 'Applies to all Agents on this device, not manual edits. Moves require both folders. Agents cannot delete items or change field visibility. Write results contain no secrets. Edit OCI signing settings in the desktop client.');
}
async function saveAgentWriteSettings(patch) {
  if (agentWriteBusy) return;
  agentWriteBusy = true;
  renderAgentWriteSettings();
  try {
    state.settings = await api.settings(patch);
    $('#agent-write-folder').value = '';
    toast(localized('Agent 写入权限已保存', 'Agent write permissions saved'));
  } catch (error) {
    toast(cleanError(error), 'error');
    await refresh(false);
  } finally {
    agentWriteBusy = false;
    renderAgentWriteSettings();
  }
}
function addAgentWriteFolder() {
  const folder = $('#agent-write-folder').value.trim();
  const folders = state.settings.agentWriteFolders || [];
  if (!folder || folders.includes(folder)) {
    toast(localized('请输入非空且不重复的文件夹名称', 'Enter a nonempty, unique folder name'), 'error');
    return;
  }
  return saveAgentWriteSettings({ agentWriteFolders: [...folders, folder] });
}

async function saveBrowserSettings() {
  try {
    state.settings = await api.settings({ browserEnabled: $('#browser-enabled').checked, browserPort: Number($('#browser-port').value) });
    toast(localized('Browser Bridge 设置已保存', 'Browser Bridge settings saved'));
    await refresh();
  } catch (error) {
    toast(cleanError(error), 'error');
    await refresh(false);
  }
}

async function saveDesktopSettings() {
  try {
    state.settings = await api.settings({
      closeBehavior: $('#close-behavior').value,
    });
    renderSettings();
    toast(localized('窗口设置已保存', 'Window settings saved'));
  } catch (error) {
    toast(cleanError(error), 'error');
    await refresh(false);
  }
}

async function setSystemIntegration(kind, enabled) {
  const input = kind === 'desktop' ? $('#desktop-shortcut-enabled') : $('#launch-at-login-enabled');
  input.disabled = true;
  try {
    systemIntegrationState = kind === 'desktop'
      ? await api.setDesktopShortcut(enabled)
      : await api.setLaunchAtLogin(enabled);
    renderSettings();
    toast(kind === 'desktop'
      ? (enabled ? localized('桌面快捷方式已创建', 'Desktop shortcut created') : localized('桌面快捷方式已移除', 'Desktop shortcut removed'))
      : (enabled ? localized('开机自启动已开启', 'Launch at login enabled') : localized('开机自启动已关闭', 'Launch at login disabled')));
  } catch (error) {
    input.checked = kind === 'desktop'
      ? Boolean(systemIntegrationState.desktopShortcut)
      : Boolean(systemIntegrationState.launchAtLogin);
    toast(cleanError(error), 'error');
  } finally {
    input.disabled = false;
  }
}

async function savePinSettings() {
  const toggle = $('#pin-enabled');
  if (toggle.checked) {
    ownerPinSetupRequested = true;
    renderOwnerLock();
    return;
  }
  if (!await confirmDestructiveAction({
    title: localized('关闭 PIN 锁？', 'Turn off PIN lock?'),
    message: localized('当前 PIN 将被移除；再次开启时需要设置新 PIN。', 'The current PIN will be removed. You will need to set a new PIN when enabling the lock again.'),
    acceptLabel: localized('关闭并移除', 'TURN OFF'),
    tag: localized('确认移除 PIN', 'CONFIRM PIN REMOVAL'),
  })) {
    toggle.checked = true;
    return;
  }
  toggle.disabled = true;
  try {
    ownerPinSetupRequested = false;
    applyOwnerLockState(await api.ownerDisablePin());
    renderMetrics();
    renderSettings();
    toast(localized('PIN 锁已关闭', 'PIN lock disabled'));
  } catch (error) {
    toast(cleanError(error), 'error');
    await refreshOwnerLock(false);
    renderSettings();
  } finally {
    toggle.disabled = false;
  }
}

async function setLanguage(next) {
  const selected = next === 'en' ? 'en' : 'zh';
  if (selected === language) return;
  language = selected;
  applyLanguage();
  if (state) {
    state.settings.language = language;
    renderOwnerLock();
    render();
    syncModuleDraft();
    if (!$('#connection-modal').classList.contains('hidden')) renderModules();
    if (!$('#connection-modal').classList.contains('hidden')) updateEditorTitle();
    try {
      state.settings = await api.settings({ language });
    } catch (error) {
      toast(cleanError(error), 'error');
    }
  }
}

function toastImportSummary(summary) {
  toast(localized(`导入完成：新增 ${summary.added}，合并重复 ${summary.merged}`, `Import complete: ${summary.added} added, ${summary.merged} duplicates merged`));
}

async function exportBackup(button) {
  button.disabled = true;
  try {
    const password = await askBackupPassword(true);
    if (password === null) return;
    const path = await api.exportBackup(password);
    if (path) toast(localized('加密备份已导出；恢复时需要备份密码', 'Encrypted backup exported; keep its password for restoration'));
  } catch (error) {
    toast(cleanError(error), 'error');
  } finally {
    button.disabled = false;
  }
}

let backupRestore = null;

function closeBackupRestore() {
  const current = backupRestore;
  backupRestore = null;
  $('#backup-restore-modal').classList.add('hidden');
  $('#backup-restore-items').replaceChildren();
  $('#backup-restore-status').textContent = '';
  $('#backup-restore-error').textContent = '';
  $('#backup-restore-error').classList.add('hidden');
  if (current) {
    void api.cancelBackupRestore(current.id).catch(() => {});
    if (ownerLockState.unlocked) current.button.focus();
  }
}

function backupRestoreSelection() {
  const selected = { active: [], trash: [] };
  for (const box of $$('#backup-restore-items input:checked')) {
    selected[box.dataset.group].push(Number(box.value));
  }
  return selected;
}

function updateBackupRestoreSelection() {
  const selected = backupRestoreSelection();
  const { active, trash } = backupRestore.preview;
  const activeIds = new Set(selected.active);
  const missing = [...selected.active.map(index => active[index]), ...selected.trash.map(index => trash[index])]
    .find(row => row.jumpIndex != null && !activeIds.has(row.jumpIndex));
  const count = selected.active.length + selected.trash.length;
  $('#backup-restore-status').textContent = missing
    ? localized(`「${missing.name}」需要同时选择跳板「${active[missing.jumpIndex].name}」。`, `Select jump host “${active[missing.jumpIndex].name}” for “${missing.name}”.`)
    : localized(`已选择 ${count} 项`, `${count} selected`);
  $('#backup-restore-confirm').disabled = Boolean(missing) || !count;
}

function showBackupRestore(preview) {
  $('#backup-restore-title').textContent = localized('预览备份恢复', 'Preview backup restore');
  $('#backup-restore-help').textContent = localized(
    '仅恢复勾选的条目。内容不同的同名活动项目会以新名称添加。',
    'Only checked entries will be restored. Active items with the same name and different content are added under a new name.');
  for (const [key, label] of Object.entries({
    all: localized('全选', 'Select all'),
    none: localized('取消全选', 'Clear selection'),
    cancel: localized('取消', 'Cancel'),
    confirm: localized('恢复所选', 'Restore selected'),
  })) $(`#backup-restore-${key}`).textContent = label;
  const list = $('#backup-restore-items');
  for (const [group, rows] of Object.entries({ active: preview.active, trash: preview.trash })) {
    if (!rows.length) continue;
    const fieldset = document.createElement('fieldset');
    const legend = document.createElement('legend');
    legend.textContent = group === 'active' ? localized('项目', 'Items') : localized('回收站', 'Trash');
    fieldset.append(legend);
    for (const row of rows) {
      const label = document.createElement('label');
      label.className = 'backup-restore-row';
      const box = document.createElement('input');
      box.type = 'checkbox';
      box.value = String(row.index);
      box.dataset.group = group;
      const details = document.createElement('span');
      details.textContent = row.name;
      const metadata = document.createElement('small');
      metadata.textContent = [itemTypeLabel(row.itemType), row.folder,
        row.nameConflict && localized('同名项目，可能重命名', 'Name conflict, may be renamed'),
        row.jumpUnresolved && localized('跳板引用未解析', 'Unresolved jump host'),
        row.jumpIndex != null && localized(`跳板：${preview.active[row.jumpIndex].name}`, `Jump host: ${preview.active[row.jumpIndex].name}`),
      ].filter(Boolean).join(' · ');
      details.append(metadata);
      label.append(box, details);
      fieldset.append(label);
    }
    list.append(fieldset);
  }
  updateBackupRestoreSelection();
  $('#backup-restore-modal').classList.remove('hidden');
  $('#backup-restore-close').focus();
}

async function importBackup(button) {
  if (backupRestore) return;
  button.disabled = true;
  try {
    const password = await askBackupPassword(false);
    if (password === null) return;
    const current = { id: crypto.randomUUID(), button };
    backupRestore = current;
    const result = await api.prepareBackupRestore(current.id, password);
    if (backupRestore !== current) return;
    if (!result) { closeBackupRestore(); return; }
    current.preview = result.preview;
    showBackupRestore(result.preview);
  } catch (error) {
    closeBackupRestore();
    toast(cleanError(error), 'error');
  } finally {
    button.disabled = false;
  }
}

for (const key of ['close', 'cancel']) $(`#backup-restore-${key}`).onclick = closeBackupRestore;
for (const key of ['all', 'none']) $(`#backup-restore-${key}`).onclick = () => {
  for (const box of $$('#backup-restore-items input')) box.checked = key === 'all';
  updateBackupRestoreSelection();
};
$('#backup-restore-items').onchange = updateBackupRestoreSelection;
$('#backup-restore-confirm').onclick = async () => {
  const current = backupRestore;
  if (!current?.preview || current.committing) return;
  current.committing = true;
  const selection = backupRestoreSelection();
  const modal = $('#backup-restore-modal');
  const controls = $$('button:not(:disabled), input:not(:disabled)', modal);
  controls.forEach(control => { control.disabled = true; });
  $('#backup-restore-error').classList.add('hidden');
  try {
    const result = await api.commitBackupRestore(current.id, selection);
    if (backupRestore !== current) return;
    closeBackupRestore();
    toastImportSummary(result);
    await refresh();
  } catch (error) {
    if (backupRestore === current) {
      $('#backup-restore-error').textContent = cleanError(error);
      $('#backup-restore-error').classList.remove('hidden');
    }
  } finally {
    controls.forEach(control => { control.disabled = false; });
    current.committing = false;
  }
};
$('#backup-restore-modal').addEventListener('keydown', event => {
  if (event.key === 'Escape' && !backupRestore?.committing) {
    event.preventDefault();
    event.stopPropagation();
    closeBackupRestore();
  }
  if (event.key !== 'Tab') return;
  const controls = $$('button:not(:disabled), input:not(:disabled)', event.currentTarget);
  const first = controls[0], last = controls.at(-1);
  if (!controls.length || (event.shiftKey ? document.activeElement === first : document.activeElement === last)) {
    event.preventDefault();
    (event.shiftKey ? last : first)?.focus();
  }
});

let vaultManagement = null, vaultManagementSequence = 0;
function closeVaultManagement() {
  vaultManagementSequence++; vaultManagement = null;
  $('#vault-management-modal').classList.add('hidden');
  $('#vault-management-list').replaceChildren();
  $('#history-clear').classList.add('hidden');
}
async function openVaultManagement(mode, id = null) {
  await refreshOwnerLock();
  if (!ownerLockState.unlocked) return;
  const sequence = ++vaultManagementSequence;
  vaultManagement = { mode, id };
  $('#vault-management-title').textContent = mode === 'trash' ? localized('回收站', 'TRASH') : localized('密码历史', 'PASSWORD HISTORY');
  $('#vault-management-help').textContent = mode === 'trash'
    ? localized('已删除项目会保留到你选择永久删除。恢复时如有同名项目，会自动改名。', 'Deleted items remain until you permanently delete them. Restored items are renamed if necessary.')
    : localized('保留最近 20 条旧密码；密码历史不会提供给 Agent 或自动填写。', 'The latest 20 previous passwords are kept. History is unavailable to agents and autofill.');
  $('#vault-management-list').textContent = localized('正在读取…', 'Loading…');
  $('#history-clear').classList.add('hidden');
  $('#vault-management-modal').classList.remove('hidden');
  try {
    const data = mode === 'trash' ? await api.trash() : await api.history(id);
    if (sequence !== vaultManagementSequence) return;
    const list = $('#vault-management-list'); list.replaceChildren();
    const entries = mode === 'trash' ? data : data.entries;
    if (!entries.length) { list.textContent = mode === 'trash' ? localized('回收站为空', 'Trash is empty') : localized('暂无旧密码', 'No previous passwords'); return; }
    if (mode === 'history') { vaultManagement.updatedAt = data.updatedAt; $('#history-clear').classList.remove('hidden'); }
    for (const entry of entries) {
      const row = document.createElement('section'); row.className = 'vault-management-row';
      const title = document.createElement('strong');
      const date = new Date(mode === 'trash' ? entry.deletedAt : entry.changedAt).toLocaleString(language === 'zh' ? 'zh-CN' : 'en');
      title.textContent = mode === 'trash' ? `${entry.item.name} · ${itemTypeLabel(entry.item.itemType)}` : date;
      const actions = document.createElement('div'); actions.className = 'button-row';
      row.append(title);
      if (mode === 'trash') {
        const subtitle = document.createElement('small'); subtitle.textContent = date; row.append(subtitle);
        for (const action of ['restore','purge']) {
          const button = document.createElement('button'); button.type='button'; button.className = action === 'purge' ? 'button danger-soft' : 'button';
          button.textContent = action === 'restore' ? localized('恢复', 'RESTORE') : localized('永久删除', 'DELETE FOREVER');
          button.dataset.trashAction = action;
          button.onclick = async () => {
            if (action === 'purge' && !await confirmDestructiveAction({title:localized('永久删除此项目？','Permanently delete this item?'),message:localized('本机回收站中的秘密和 Passkey 将被移除，无法撤销。已有备份不会被改动。','The item and its passkeys will be removed from this vault permanently. Existing backups are unchanged.')})) return;
            button.disabled=true;
            try { await api[action](entry.item.id,entry.deletedAt); await refresh(); if (sequence === vaultManagementSequence) await openVaultManagement('trash'); }
            catch(error) { toast(cleanError(error),'error'); button.disabled=false; }
          };
          actions.append(button);
        }
      } else {
        const input=document.createElement('input'); input.className='input'; input.type='password'; input.readOnly=true; input.autocomplete='off'; input.value=entry.password; input.setAttribute('aria-label',localized('旧密码','Previous password'));
        const reveal=document.createElement('button'); reveal.type='button'; reveal.className='button'; reveal.textContent=localized('显示','SHOW');
        reveal.onclick=()=>{ input.type=input.type==='password'?'text':'password'; reveal.textContent=input.type==='password'?localized('显示','SHOW'):localized('隐藏','HIDE'); };
        const copy=document.createElement('button'); copy.type='button'; copy.className='button'; copy.textContent=localized('复制','COPY');
        copy.onclick=async()=>{ try { await api.copyOwnerValue(input.value); toast(localized('已复制','Copied')); } catch(error){toast(cleanError(error),'error');} };
        row.append(input); actions.append(reveal,copy);
      }
      row.append(actions); list.append(row);
    }
  } catch(error) { if (sequence === vaultManagementSequence) $('#vault-management-list').textContent=cleanError(error); }
}
$('#trash-open').onclick = () => openVaultManagement('trash');
$('#vault-management-close').onclick = closeVaultManagement;
$('#history-clear').onclick = async () => {
  const snapshot = vaultManagement;
  if (snapshot?.mode !== 'history' || !await confirmDestructiveAction({title:localized('清除密码历史？','Clear password history?'),message:localized('只清除旧密码，当前密码保持不变。此操作无法撤销。','Previous passwords will be removed permanently. The current password is preserved.')})) return;
  try { await api.clearHistory(snapshot.id,snapshot.updatedAt); await refresh(); if (vaultManagement === snapshot) await openVaultManagement('history',snapshot.id); }
  catch(error){toast(cleanError(error),'error');}
};

function renderLoginUris(rules) {
  const choices = [['origin','同源','Same origin'],['domain','主域名','Base domain'],['host','主机与端口','Host and port'],['exact','完整网址','Exact URL'],['prefix','网址前缀','URL prefix'],['regex','正则表达式','Regular expression'],['never','不匹配','Never']];
  $('#login-uri-list').innerHTML = rules.map(rule => `<div class="login-uri-row"><input class="input" data-login-uri aria-label="${localized('网站地址','Website address')}" value="${escapeHtml(rule.uri)}" maxlength="8192"><select class="input" data-login-match aria-label="${localized('匹配规则','Matching rule')}">${choices.map(([key,zh,en]) => `<option value="${key}" ${rule.matchType===key?'selected':''}>${localized(zh,en)}</option>`).join('')}</select><button class="small-button" data-remove-uri type="button" aria-label="${localized('移除网站','Remove website')}">×</button></div>`).join('');
}
$('#login-uri-add').onclick = () => renderLoginUris([...$$('#login-uri-list .login-uri-row').map(row => ({uri:$('[data-login-uri]',row).value,matchType:$('[data-login-match]',row).value})),{uri:'',matchType:'origin'}]);
$('#login-uri-list').onclick = event => event.target.closest('[data-remove-uri]')?.closest('.login-uri-row').remove();

let migrationId = null;
let migrationSequence = 0;
let migrationCancellation = Promise.resolve();
let batchImport = null;
const importUndoIds = new Map();
const selectableImportFormats = new Set(['env', 'ssh']);
let migrationBusy = false;
const migrationWarnings = {
  nonWebUris: ['非网页地址已保留，设为“不匹配”。','Non-web addresses are preserved with matching disabled.'],
  defaultDomainMatch: ['未指定规则的网址使用主域名匹配；保存的协议和端口必须一致。可在项目内调整。','Unspecified rules use base-domain matching, with matching scheme and port. Edit rules in the item.'],
  unsupportedTotp: ['部分验证码密钥格式尚不支持生成；原始密钥已保留。','Some TOTP formats cannot generate codes yet; their original keys are preserved.'],
  passkeyCounter: ['包含旧版计数型 Passkey。导入后请只在 OpenKeylet 中使用这些 Passkey，避免另一副本的计数超前。','Includes legacy counter-based passkeys. Use these only in OpenKeylet after migration to avoid diverging counters.'],
  linkedFields: ['关联字段已作为加密自定义字段保留，暂不自动关联填写。','Linked fields are preserved as encrypted custom fields; automatic linking is unavailable.'],
  passwordHistory: ['最近 20 条旧密码会进入密码历史，更早的记录保留在加密自定义字段中。','The latest 20 previous passwords appear in history; older records are archived in an encrypted custom field.'],
  reprompt: ['原逐项主密码验证设置不生效；本地 PIN 锁仍适用。','Source per-item master-password reprompt settings are not applied; the local PIN lock still applies.'],
  attachments: ['JSON 不含附件文件内容；请从原密码库单独保留附件。','JSON does not contain attachment files. Keep them separately from the source vault.'],
  organization: ['组织项目将成为本地个人项目，不保留共享权限。','Organization items become local personal items without sharing permissions.'],
};
function migrationHelp() {
  const format = $('#migration-format').value;
  const help = {
    bitwarden: localized('在 Bitwarden 中导出未加密 JSON，然后选择文件预览。原文件不变；导入的秘密会加密存入本地库。', 'Export unencrypted JSON from Bitwarden, then choose the file to preview. Imported secrets are encrypted locally.'),
    chrome: localized('选择 Chrome 或 Edge 导出的密码 CSV。支持带引号、逗号和多行备注的文件。', 'Choose a password CSV exported by Chrome or Edge. Quoted fields, commas and multiline notes are supported.'),
    ssh: localized('选取一个 OpenSSH 配置文件。预览显式主机及路由。需同时选中可用的直连跳板，导入后再配置认证和主机指纹。', 'Choose one OpenSSH config file to preview explicit hosts and routes. Select the direct jump host as well, then configure credentials and host fingerprints after import.'),
    env: localized('只预览变量名。每个选中的变量会在 Env 文件夹中建立一个加密项目。同名项目会跳过。macOS 可按 ⌘⇧. 显示隐藏文件。', 'Preview variable names. Each selected variable becomes an encrypted item in the Env folder. Items with existing names are skipped. On macOS, press ⌘⇧. to show hidden files.'),
    curl: localized('粘贴一条使用 Bearer 或 Basic 认证的 HTTPS cURL 命令。确认后保存服务根地址，凭据会在本机加密保存。', 'Paste one HTTPS cURL command with Bearer or Basic authentication. Confirm to save the service origin and encrypted credentials.'),
  };
  $('#migration-help').textContent = help[format];
  $('#migration-curl-field span').textContent = localized('cURL 命令', 'cURL command');
  updateImportSelection();
  $('#migration-preview').setAttribute('role', selectableImportFormats.has(format) ? 'group' : 'status');
  $('#migration-preview').setAttribute('aria-label', localized('导入预览', 'Import preview'));
  $('#migration-batch-undo').textContent = localized('撤销本批新增', 'Undo added items');
  $('#migration-batch-undo').title = localized('本次解锁期间可撤销最近一次导入。', 'Undo the latest import during this unlocked session.');
  $('#migration-batch-undo').classList.toggle('hidden', !importUndoIds.has(format));
}
function resetMigration() {
  migrationSequence++; migrationId = null;
  $('#migration-curl').value = '';
  if (batchImport) invoke(`cancel_${batchImport.format}_import`, { id: batchImport.id }).catch(() => {});
  batchImport = null;
  $('#migration-preview').replaceChildren(); $('#migration-preview').classList.add('hidden');
  $('#migration-confirm').classList.add('hidden');
  $('#migration-error').classList.add('hidden');
  $('#migration-batch-status').classList.add('hidden');
}
function setMigrationBusy(busy) {
  migrationBusy = busy;
  $('#migration-preview').inert = busy;
  for (const key of ['curl', 'format', 'close', 'batch-undo']) $(`#migration-${key}`).disabled = busy;
  updateImportSelection();
}
function updateImportSelection() {
  const format = $('#migration-format').value;
  const curlPreview = format === 'curl' && migrationId !== null;
  const selectButton = $('#migration-select');
  selectButton.textContent = format !== 'curl' ? localized('选择文件', 'CHOOSE FILE') : curlPreview
    ? localized('重新粘贴', 'PASTE AGAIN') : localized('预览命令', 'PREVIEW COMMAND');
  selectButton.disabled = migrationBusy || (format === 'curl' && !curlPreview && !$('#migration-curl').value.trim());
  $('#migration-curl-field').classList.toggle('hidden', format !== 'curl' || curlPreview);
  const selected = $$('#migration-preview input:checked');
  $('#migration-confirm').disabled = migrationBusy || (selectableImportFormats.has(format) && !selected.length);
  if (batchImport?.format !== 'ssh') return;
  const selectedIndices = new Set(selected.map(box => Number(box.value)));
  for (const route of $$('[data-import-ssh-route]')) {
    const row = batchImport.rows[Number(route.dataset.importSshRoute)];
    route.textContent = sshImportRouteLabel(row, selectedIndices);
  }
}
function sshImportRouteLabel(row, selectedIndices) {
  if (row.route === 'direct') return localized('直连', 'Direct');
  if (row.route === 'jump' && selectedIndices.has(row.jumpIndex)) {
    return localized(`通过跳板 ${row.jump}`, `Via jump host ${row.jump}`);
  }
  return row.jump
    ? localized(`跳板 ${row.jump} 未配置，连接受阻`, `Jump host ${row.jump} unresolved. Connection blocked.`)
    : localized('路由待配置，连接受阻', 'Route unresolved. Connection blocked.');
}
function renderBatchImport({ rows, empty, multiline }) {
  const preview = $('#migration-preview');
  for (const row of rows) {
    const label = document.createElement('label');
    label.className = 'backup-restore-row';
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.value = row.index;
    box.disabled = row.conflict;
    box.checked = !box.disabled;
    const name = document.createElement('span');
    name.textContent = row.name;
    if (batchImport.format === 'ssh') {
      const endpoint = document.createElement('small');
      endpoint.textContent = `${row.user ? `${row.user}@` : ''}${row.host} · ${row.port}`;
      const route = document.createElement('small');
      route.dataset.importSshRoute = row.index;
      name.append(endpoint, route);
    }
    if (row.conflict) {
      const status = document.createElement('small');
      status.textContent = localized('名称重复，跳过', 'Duplicate name, skipped');
      name.append(status);
    }
    label.append(box, name);
    preview.append(label);
  }
  preview.classList.remove('hidden');
  $('#migration-confirm').classList.toggle('hidden', !rows.some(row => !row.conflict));
  updateImportSelection();
  const notices = [];
  if (empty) notices.push(localized(`跳过 ${empty} 个空值变量。`, `Skipped ${empty} empty variables.`));
  if (multiline) notices.push(localized(`跳过 ${multiline} 个多行值变量。`, `Skipped ${multiline} variables with multiline values.`));
  if (notices.length) showBatchImportStatus(notices.join(' '));
}
function showBatchImportStatus(message) {
  const status = $('#migration-batch-status');
  status.textContent = message;
  status.classList.remove('hidden');
}
function cancelMigrationImport() {
  migrationCancellation = migrationCancellation.then(() => api.cancelImport()).catch(() => {});
}
function closeMigration() {
  resetMigration(); $('#migration-modal').classList.add('hidden');
  cancelMigrationImport();
}
$('#migration-open').onclick = async () => {
  await refreshOwnerLock();
  if (!ownerLockState.unlocked) return;
  resetMigration(); migrationHelp(); $('#migration-modal').classList.remove('hidden');
};
$('#migration-close').onclick = closeMigration;
$('#migration-format').onchange = () => { resetMigration(); cancelMigrationImport(); migrationHelp(); };
$('#migration-curl').oninput = () => {
  $('#migration-error').classList.add('hidden');
  updateImportSelection();
};
$('#migration-preview').onchange = updateImportSelection;
$('#migration-select').onclick = async () => {
  const format = $('#migration-format').value;
  if (format === 'curl' && migrationId) {
    resetMigration();
    cancelMigrationImport();
    updateImportSelection();
    $('#migration-curl').focus();
    return;
  }
  let curlInput = $('#migration-curl').value;
  resetMigration(); const sequence = migrationSequence;
  setMigrationBusy(true);
  try {
    await migrationCancellation;
    if (sequence !== migrationSequence) return;
    if (format === 'curl') {
      const response = invoke('prepare_curl_import', { command: curlInput });
      curlInput = '';
      const { id, preview } = await response;
      if (sequence !== migrationSequence) return;
      migrationId = id;
      const target = document.createElement('p');
      target.textContent = preview.target;
      const authentication = document.createElement('p');
      authentication.textContent = preview.authentication === 'basic'
        ? localized('Basic 认证', 'Basic authentication') : localized('Bearer 认证', 'Bearer authentication');
      $('#migration-preview').replaceChildren(target, authentication);
      $('#migration-preview').classList.remove('hidden');
      $('#migration-confirm').classList.remove('hidden');
      return;
    }
    if (selectableImportFormats.has(format)) {
      const id = crypto.randomUUID();
      batchImport = { id, format, rows: {} };
      const result = await invoke(`prepare_${format}_import`, { id });
      if (!result || sequence !== migrationSequence) return;
      batchImport.rows = Object.fromEntries(result.rows.map(row => [row.index, row]));
      renderBatchImport(result);
      return;
    }
    const result = await api.prepareImport(format);
    if (!result || sequence !== migrationSequence) return;
    migrationId = result.id;
    const p = result.preview;
    const lines = [localized(`可导入 ${p.total} 项 · Passkey ${p.passkeys} 个`, `${p.total} items · ${p.passkeys} passkeys`),
      ...Object.entries(p.types).map(([type,count]) => `${itemTypeLabel(type)}: ${count}`),
      ...(p.skippedDeleted ? [localized(`跳过回收站项目 ${p.skippedDeleted} 项`, `Skipped ${p.skippedDeleted} deleted items`)] : []),
      ...p.warnings.map(key => migrationWarnings[key] ? localized(...migrationWarnings[key]) : key)];
    const preview = $('#migration-preview'); preview.replaceChildren(...lines.map(line => {const p = document.createElement('p'); p.textContent=line; return p;}));
    preview.classList.remove('hidden'); $('#migration-confirm').classList.toggle('hidden', p.total === 0);
  } catch(error) {
    const code = String(error?.message || error);
    if (sequence === migrationSequence) {
      $('#migration-error').textContent = format !== 'curl' ? cleanError(error) : code === 'CURL_IMPORT_UNSUPPORTED'
        ? localized('请使用单个 HTTPS 地址和 Bearer 或 Basic 认证。请去掉查询参数和不支持的选项。', 'Use one HTTPS URL with Bearer or Basic authentication. Remove URL query parameters and unsupported options.')
        : localized('无法预览命令。请检查格式后重新粘贴。', 'Could not preview the command. Check the format and paste it again.');
      $('#migration-error').classList.remove('hidden');
    }
  } finally {
    curlInput = '';
    setMigrationBusy(false);
    if (format === 'curl' && migrationId && sequence === migrationSequence) $('#migration-confirm').focus();
  }
};
$('#migration-confirm').onclick = async () => {
  if (migrationBusy || (!migrationId && !batchImport)) return;
  const sequence = migrationSequence;
  setMigrationBusy(true);
  try {
    if (batchImport) {
      const { id, format } = batchImport;
      const selection = $$('#migration-preview input:checked').map(box => Number(box.value));
      const previewConflicts = $$('#migration-preview input:disabled').length;
      const result = await invoke(`commit_${format}_import`, { id, selection });
      if (sequence === migrationSequence) {
        batchImport = null;
        if (result.undoId) importUndoIds.set(format, result.undoId);
        resetMigration();
        migrationHelp();
        const skipped = result.skipped + previewConflicts;
        showBatchImportStatus(localized(`已导入 ${result.added} 项，跳过 ${skipped} 个同名项。`, `Imported ${result.added} items. Skipped ${skipped} name conflicts.`));
      }
      await refresh();
      return;
    }
    const result = await api.commitImport(migrationId); closeMigration(); toastImportSummary(result); await refresh();
  }
  catch(error) { if (sequence === migrationSequence) { $('#migration-error').textContent=cleanError(error); $('#migration-error').classList.remove('hidden'); } }
  finally { setMigrationBusy(false); }
};
$('#migration-batch-undo').onclick = async () => {
  const format = $('#migration-format').value;
  const id = importUndoIds.get(format);
  if (!id || migrationBusy) return;
  const sequence = migrationSequence;
  setMigrationBusy(true);
  try {
    const result = await invoke(`undo_${format}_import`, { id });
    importUndoIds.delete(format);
    if (sequence === migrationSequence) {
      resetMigration();
      migrationHelp();
      showBatchImportStatus(localized(`已移入回收站 ${result.removed} 项，保留 ${result.skipped} 项。`, `Moved ${result.removed} items to Trash. Kept ${result.skipped} items.`));
    }
    await refresh();
  } catch (error) {
    if (sequence === migrationSequence) { $('#migration-error').textContent=cleanError(error); $('#migration-error').classList.remove('hidden'); }
  } finally { setMigrationBusy(false); }
};

document.addEventListener('click', async (event) => {
  if (event.target.closest('#title-search-toggle')) setTitleSearchOpen(!titleSearchOpen);
  else if (titleSearchOpen && !event.target.closest('#title-search, [data-clear-item-filter], [data-action="clear-connection-filters"]')) setTitleSearchOpen(false, false);
  const filterButton = event.target.closest('.filter-strip button');
  if (filterButton && !switchSelectionPending) await selectFilterButton(filterButton);
  const languageButton = event.target.closest('[data-language]');
  if (languageButton) await setLanguage(languageButton.dataset.language);
  const nav = event.target.closest('[data-page]');
  if (nav) await selectPage(nav.dataset.page);
  const windowButton = event.target.closest('[data-window-action]');
  if (windowButton) {
    if (windowButton.dataset.windowAction === 'close' && !$('#connection-modal').classList.contains('hidden')) await closeEditorWithDraft();
    await api.window(windowButton.dataset.windowAction);
    if (windowButton.dataset.windowAction === 'minimize') await refreshOwnerLock(false);
  }
  if (event.target.closest('#owner-lock-button')) {
    if (!$('#connection-modal').classList.contains('hidden')) await closeEditorWithDraft();
    await lockOwner();
  }
  const deleteDraftButton = event.target.closest('#delete-draft-button');
  if (deleteDraftButton && editorDrafts[0]) {
    const draftId = editorDrafts[0].id;
    if (!await confirmDestructiveAction({
      title: localized('删除当前草稿？', 'Delete this draft?'),
      message: localized('草稿中的未保存内容将永久丢失。', 'Unsaved content in this draft will be permanently lost.'),
    })) return;
    try {
      await api.deleteDraft(draftId);
      editorDrafts = editorDrafts.filter((draft) => draft.id !== draftId);
      renderDrafts();
      $('#add-connection-button').focus();
      toast(localized('草稿已删除', 'Draft deleted'));
    } catch (error) { toast(cleanError(error), 'error'); }
    return;
  }
  const draftsButton = event.target.closest('#drafts-button');
  if (draftsButton && editorDrafts[0]) await selectCreationMode('draft');
  const moduleButton = event.target.closest('#add-module-button');
  if (moduleButton) setModuleMenu(moduleButton.getAttribute('aria-expanded') !== 'true');
  else if (!event.target.closest('#module-menu')) setModuleMenu(false);
  const addModuleButton = event.target.closest('[data-add-module]');
  if (addModuleButton) addModule(addModuleButton.dataset.addModule);
  const templateButton = event.target.closest('[data-item-template]');
  if (templateButton) applyTemplate(templateButton.dataset.itemTemplate);
  if (event.target.closest('#add-connection-button') || event.target.closest('[data-action="add"]')) {
    await selectCreationMode('add');
  }
  if (event.target === $('#connection-modal') && !inlineEditorItemId) await closeEditorWithDraft();
  if (event.target.closest('[data-close-modal]')) await closeEditorWithDraft();
  const copyInput = event.target.closest('[data-copy-input]');
  const copyModule = event.target.closest('[data-copy-module-secret], [data-copy-module-value]');
  if (copyInput || copyModule) {
    const input = copyInput ? $(`#${copyInput.dataset.copyInput}`) : $('[data-secret-value], [data-module-value]', copyModule.closest('.module-row'));
    try { await api.copyOwnerValue(input.value); toast(localized('已复制', 'Copied')); } catch (error) { toast(cleanError(error), 'error'); }
  }
  const totpQrButton = event.target.closest('[data-import-totp-qr]');
  if (totpQrButton && ownerLockState.unlocked && !totpQrImport) {
    totpQrImport = { moduleId: totpQrButton.closest('.module-row').dataset.moduleId, generation: itemEditorGeneration, uri: null };
    $('#totp-qr-dialog').showModal();
    $('#totp-qr-paste').focus();
    return;
  }
  const serviceButton = event.target.closest('[data-select-api-service]');
  if (serviceButton) { await openApiServicePicker(serviceButton); return; }
  if (event.target.closest('[data-close-service-picker]') || event.target === $('#api-service-modal')) { closeApiServicePicker(); return; }
  const serviceOption = event.target.closest('[data-api-service]');
  if (serviceOption && apiServicePicker) {
    apiServicePicker.entry = apiServicePicker.catalog.find(entry => entry.id === serviceOption.dataset.apiService);
    apiServicePicker.parameters = {};
    if (apiServicePicker.entry.parameters.includes('region') && editorHttpAuthType === 'oci-signature') apiServicePicker.parameters.region = currentModules.find(module => module.id === editorOciBindings?.region)?.secretValue || '';
    renderApiServicePicker();
    ($('[data-api-service-parameter]', $('#api-service-modal')) || $('#api-service-target')).focus();
  }
  if (event.target.closest('#api-service-replace')) applyApiServiceAddress(false);
  if (event.target.closest('#api-service-add')) applyApiServiceAddress(true);
  if (event.target.closest('#api-service-refresh-target') && apiServicePicker) {
    syncModuleDraft();
    const module = currentModules.find(module => module.id === apiServicePicker.moduleId && module.kind === 'url');
    if (!module) { closeApiServicePicker(); return; }
    apiServicePicker.addresses = moduleAddresses(module);
    apiServicePicker.target = Math.min(apiServicePicker.target, apiServicePicker.addresses.length - 1);
    renderApiServicePicker();
  }
  const addressAction = event.target.closest('[data-add-address], [data-default-address], [data-remove-address]');
  if (addressAction) {
    syncModuleDraft();
    const moduleIndex = Number(addressAction.closest('.module-row').dataset.moduleIndex);
    const module = currentModules[moduleIndex];
    const addresses = [module.value || '', ...(module.alternateValues || [])];
    let focusedIndex = 0;
    if (addressAction.hasAttribute('data-add-address')) {
      addresses.push('');
      focusedIndex = addresses.length - 1;
    } else if (addressAction.hasAttribute('data-default-address')) {
      const index = Number(addressAction.dataset.defaultAddress);
      [addresses[0], addresses[index]] = [addresses[index], addresses[0]];
    } else {
      const index = Number(addressAction.dataset.removeAddress);
      addresses.splice(index, 1);
      if (!addresses.length) addresses.push('');
      focusedIndex = Math.min(index, addresses.length - 1);
    }
    [module.value, ...module.alternateValues] = addresses;
    renderModules();
    const row = $$('.module-row', $('#module-list'))[moduleIndex];
    $$('[data-module-value], [data-module-alternate-value]', row)[focusedIndex]?.focus();
  }
  const toggleModuleSecret = event.target.closest('[data-toggle-module-secret]');
  if (toggleModuleSecret) {
    const field = $('[data-secret-value]', toggleModuleSecret.closest('.module-row'));
    const revealed = toggleModuleSecret.getAttribute('aria-pressed') !== 'true';
    if (field.tagName === 'INPUT') field.type = revealed ? 'text' : 'password';
    else field.classList.toggle('secret-masked', !revealed);
    toggleModuleSecret.setAttribute('aria-pressed', String(revealed));
    const label = revealed ? localized('隐藏明文', 'Hide secret') : localized('显示明文', 'Show secret');
    toggleModuleSecret.setAttribute('aria-label', label);
    toggleModuleSecret.title = label;
  }
  const toggleAgentVisible = event.target.closest('[data-module-agent-visible]');
  if (toggleAgentVisible) setModuleAgentVisibility(toggleAgentVisible, toggleAgentVisible.getAttribute('aria-pressed') !== 'true');
  const cancelPendingKey = event.target.closest('[data-cancel-pending-key]');
  if (cancelPendingKey && !moduleKeyStaging) {
    syncModuleDraft();
    const module = currentModules.find(module => module.id === cancelPendingKey.closest('.module-row').dataset.moduleId);
    if (module?.pending) {
      await discardModuleKey(module);
      Object.assign(module, { pending: '', pendingKind: '', configured: Boolean(module.secretValue), privateKeyName: editorExistingItem?.privateKeyName || '', fingerprint: '' });
      renderModules();
    }
  }
  const cancelKeyInput = event.target.closest('[data-cancel-key-input]');
  if (cancelKeyInput && !moduleKeyStaging) {
    moduleKeyInput = null;
    syncModuleDraft();
    renderModules();
  }
  const confirmKeyInput = event.target.closest('[data-confirm-key-input]');
  if (confirmKeyInput && moduleKeyInput) await stageModuleKey(confirmKeyInput.closest('.module-row'), 'input', moduleKeyInput.text);
  const stageKey = event.target.closest('[data-stage-key]');
  if (stageKey && !moduleKeyStaging) {
    const row = stageKey.closest('.module-row');
    stageKey.closest('details').open = false;
    syncModuleDraft();
    if (stageKey.dataset.stageKey === 'input') {
      moduleKeyInput = { moduleId: row.dataset.moduleId, text: '' };
      renderModules();
      $('[data-key-input]', $('#module-list')).focus();
    } else await stageModuleKey(row, stageKey.dataset.stageKey);
  }
  const copyPublicKey = event.target.closest('[data-copy-public-key]');
  if (copyPublicKey) {
    try {
      await api.copyPublicKey(copyPublicKey.closest('.module-row').dataset.pending || null, editorExistingItem?.id || null);
      toast(localized('已复制公钥', 'Public key copied'));
    } catch (error) { toast(cleanError(error), 'error'); }
  }
  const removeModuleButton = event.target.closest('[data-remove-module]');
  if (removeModuleButton) {
    syncModuleDraft();
    const row = removeModuleButton.closest('.module-row');
    const index = Number(row.dataset.moduleIndex);
    const removed = currentModules[index];
    if (!removed) return;
    const moduleName = removed.kind === 'customSecret' && removed.name.trim() ? removed.name.trim() : moduleLabel(removed.kind);
    if (!await confirmDestructiveAction({
      title: localized(`删除“${moduleName}”模块？`, `Delete the "${moduleName}" module?`),
      message: removed?.existing
        ? localized('保存项目后，该模块及其加密值将永久删除。', 'After you save the item, this module and its encrypted value will be permanently deleted.')
        : localized('该模块中尚未保存的内容将丢失。', 'Unsaved content in this module will be lost.'),
    })) return;
    if (!row.isConnected || $('#connection-modal').classList.contains('hidden')) return;
    if (removed?.existing && moduleSecretName(removed)) removedSecretFields.add(moduleSecretName(removed));
    if (removed.pending) void discardModuleKey(removed).catch(() => {});
    if (moduleKeyInput?.moduleId === removed.id) moduleKeyInput = null;
    currentModules.splice(index, 1);
    renderModules();
    const remainingRows = $$('.module-row', $('#module-list'));
    ($('[data-remove-module]', remainingRows[Math.min(index, remainingRows.length - 1)] || $('#module-list')) || $('#add-module-button')).focus();
  }
  const action = event.target.closest('[data-action]');
  if (action?.dataset.action === 'clear-connection-filters') {
    pageSearch.connections = '';
    currentItemTypeFilter = 'all';
    currentFolderFilter = 'all';
    currentItemStatusFilter = 'all';
    syncItemTypeFilter();
    syncTitleSearch();
    renderConnections();
    setTitleSearchOpen(true);
  }
  if (action?.dataset.action === 'clear-activity-filters') {
    currentActivityFilter = 'all';
    pageSearch.activity = '';
    $('[data-page-search="activity"]').value = '';
    activityVisibleCount = ACTIVITY_PAGE_SIZE;
    renderActivity();
  }
  if (action?.dataset.id) {
    const item = state.connections.find((candidate) => candidate.id === action.dataset.id);
    if (action.dataset.action === 'toggle-enabled') {
      try {
        await api.setEnabled(item.id, !item.enabled);
        toast(item.enabled ? localized('项目已停用', 'Item disabled') : localized('项目已启用', 'Item enabled'));
        await refresh();
      } catch (error) { toast(cleanError(error), 'error'); }
    }
    if (action.dataset.action === 'edit') await openEditor(item);
    if (action.dataset.action === 'history') await openVaultManagement('history', item.id);
    if (action.dataset.action === 'copy-name') {
      try {
        await api.copyOwnerValue(`use ${item.name} in OpenKeylet MCP`, false);
        toast(localized('OpenKeylet MCP 使用提示已复制', 'OpenKeylet MCP use prompt copied'));
      } catch (error) { toast(cleanError(error), 'error'); }
    }
    if (action.dataset.action === 'test') {
      try { toast(publicMessage(await api.test(item.id), localized('连接测试完成', 'Connection test complete'))); await refresh(false); } catch (error) { toast(cleanError(error), 'error'); }
    }
    if (action.dataset.action === 'delete' && await confirmDestructiveAction({
      title: localized(`将“${item.name}”移入回收站？`, `Move "${item.name}" to trash?`),
      message: localized('项目将停止用于填写、Passkey 登录和 Agent 操作。你可以在回收站恢复它。', 'The item will stop being available for autofill, passkeys and agents. You can restore it from Trash.'),
      acceptLabel: localized('移入回收站', 'MOVE TO TRASH'),
    })) {
      try { await api.remove(item.id, item.updatedAt); toast(localized('已移入回收站', 'Moved to trash')); await refresh(); $('#add-connection-button').focus(); } catch (error) { toast(cleanError(error), 'error'); }
    }
  }
  const errorDetail = event.target.closest('[data-expand-error]');
  if (errorDetail) {
    const expanded = errorDetail.getAttribute('aria-expanded') === 'true';
    if (expanded) expandedActivityErrors.delete(errorDetail.dataset.expandError);
    else expandedActivityErrors.add(errorDetail.dataset.expandError);
    errorDetail.setAttribute('aria-expanded', String(!expanded));
    errorDetail.classList.toggle('expanded', !expanded);
  }
  const copy = event.target.closest('[data-copy-format]');
  if (copy) {
    try { await api.copyConfig(copy.dataset.copyFormat); toast(localized('stdio 配置已复制', 'stdio configuration copied')); } catch (error) { toast(cleanError(error), 'error'); }
  }
  const agentAction = event.target.closest('[data-agent-action]');
  if (agentAction) {
    const kind = agentAction.dataset.agentAction;
    if (kind === 'remove') {
      const client = agentClients.find((candidate) => candidate.clientId === agentAction.dataset.clientId);
      if (!await confirmDestructiveAction({
        title: localized(`移除 ${client?.displayName || 'Agent'} 的 OpenKeylet 接入？`, `Remove OpenKeylet from ${client?.displayName || 'this agent'}?`),
        message: localized('将删除该 Agent 的 OpenKeylet MCP 配置；之后可以重新连接。', 'This removes the OpenKeylet MCP configuration from the agent. You can connect it again later.'),
        acceptLabel: localized('移除', 'REMOVE'),
        tag: localized('确认移除接入', 'CONFIRM INTEGRATION REMOVAL'),
      })) return;
    }
    try {
      const result = kind === 'connect'
        ? (await api.registerAgents([agentAction.dataset.clientId]))[0]
        : kind === 'repair'
          ? await api.repairAgent(agentAction.dataset.clientId)
          : await api.removeAgent(agentAction.dataset.clientId);
      if (result?.ok) {
        agentRestartRequired = true;
        $('#agent-restart-notice').classList.remove('hidden');
      }
      toast(result?.ok ? localized('已更新 · 新开 Agent 会话后生效', 'Updated · Open a new agent session to apply') : publicMessage(result?.message), result?.ok ? '' : 'error');
      await scanAgents();
    } catch (error) { toast(cleanError(error), 'error'); }
  }
});

$('#connection-folder-filter').addEventListener('change', event => {
  currentFolderFilter = event.target.value;
  renderConnections();
});
$('#connection-status-filter').addEventListener('change', event => {
  currentItemStatusFilter = event.target.value;
  renderConnections();
});
$('#connection-filter-chips').addEventListener('click', event => {
  const kind = event.target.closest('[data-clear-item-filter]')?.dataset.clearItemFilter;
  if (!kind) return;
  if (kind === 'type') { currentItemTypeFilter = 'all'; syncItemTypeFilter(); }
  if (kind === 'folder') currentFolderFilter = 'all';
  if (kind === 'status') currentItemStatusFilter = 'all';
  renderConnections();
  setTitleSearchOpen(true);
});
document.addEventListener('input', (event) => {
  if (event.target.id === 'api-service-search' && apiServicePicker) { apiServicePicker.query = event.target.value; renderApiServicePicker(); return; }
  if (event.target.matches('[data-api-service-parameter]') && apiServicePicker) { apiServicePicker.parameters[event.target.dataset.apiServiceParameter] = event.target.value; renderApiServicePreview(); return; }
  if (event.target.matches('.pin-cell-input')) {
    event.target.value = event.target.value.replace(/\D/g, '').slice(-1);
    if (event.target.value) {
      const inputs = $$('.pin-cell-input', event.target.closest('.pin-control'));
      inputs[inputs.indexOf(event.target) + 1]?.focus();
    }
  }
  if (event.target.matches('[data-key-input]') && moduleKeyInput) moduleKeyInput.text = event.target.value;
  if (event.target.closest('.module-row')) {
    syncModuleDraft();
    if (event.target.closest('.module-row').dataset.moduleId === editorOciBindings?.region && event.target.matches('[data-secret-value]')) updateOciModuleUrl();
    if (event.target.matches('[data-module-name]')) renderHttpAuthentication();
  }
  const page = event.target.dataset.pageSearch;
  if (!page) return;
  pageSearch[page] = event.target.value;
  syncTitleSearch();
  if (page === 'connections') renderConnections();
  if (page === 'activity') {
    activityVisibleCount = ACTIVITY_PAGE_SIZE;
    renderActivity();
  }
  if (page === 'settings') applySettingsSearch();
});

$('#api-service-category').addEventListener('change', event => {
  if (!apiServicePicker) return;
  apiServicePicker.category = event.target.value;
  renderApiServicePicker();
  $('#api-service-category').focus();
});
$('#api-service-target').addEventListener('change', event => {
  if (!apiServicePicker) return;
  apiServicePicker.target = Number(event.target.value);
  renderApiServicePreview();
});

const itemList = $('#connections-list');

function visibleItemIds() { return $$('.connection-card', itemList).map(row => row.dataset.itemId); }

async function saveItemOrder(visibleOrder, expectedOrder, focusedId) {
  if (itemOrderSaving) return;
  const visible = new Set(visibleOrder);
  let index = 0;
  // A filtered list rearranges only its own slots, leaving hidden items in place.
  const ids = expectedOrder.map(id => visible.has(id) ? visibleOrder[index++] : id);
  if (ids.every((id, position) => id === expectedOrder[position])) { renderConnections(); return; }
  itemOrderSaving = true;
  itemList.setAttribute('aria-busy', 'true');
  let saved = false;
  try { await api.reorder(ids, expectedOrder); saved = true; }
  catch (error) { toast(cleanError(error), 'error'); }
  finally {
    itemOrderSaving = false;
    renderedConnectionsSignature = '';
    await refresh(false);
    itemList.removeAttribute('aria-busy');
    const handle = $$('[data-item-reorder]', itemList).find(button => button.dataset.itemReorder === focusedId);
    handle?.focus({ preventScroll: true });
    if (saved) {
      handle?.scrollIntoView({ block: 'nearest' });
      const name = state.connections.find(item => item.id === focusedId)?.name || '';
      $('#item-reorder-status').textContent = localized(`${name}：第 ${ids.indexOf(focusedId) + 1} 项，共 ${ids.length} 项`, `${name}: position ${ids.indexOf(focusedId) + 1} of ${ids.length}`);
    }
  }
}

function clearItemDropSeam() {
  $$('.item-drop-before, .item-drop-after', itemList).forEach(row => row.classList.remove('item-drop-before', 'item-drop-after', 'item-drop-side'));
}

function updateItemDropSeam() {
  const drag = itemPointerDrag;
  if (!drag?.started) return;
  clearItemDropSeam();
  const bounds = itemList.getBoundingClientRect();
  drag.valid = drag.x >= bounds.left && drag.x <= bounds.right && drag.y >= bounds.top && drag.y <= bounds.bottom;
  drag.preview.style.opacity = drag.valid ? '1' : '.5';
  drag.target = null;
  if (!drag.valid) return;
  const others = $$('.connection-card', itemList).filter(row => row !== drag.row);
  if (!others.length) return;
  if (document.documentElement.dataset.itemLayout === 'grid') {
    const singleColumn = getComputedStyle(itemList).gridTemplateColumns.trim().split(/\s+/).length === 1;
    const targets = others.map(row => {
      const rect = row.getBoundingClientRect();
      const dx = Math.max(rect.left - drag.x, 0, drag.x - rect.right);
      const dy = Math.max(rect.top - drag.y, 0, drag.y - rect.bottom);
      const after = singleColumn || row.classList.contains('is-editing')
        ? drag.y > rect.top + rect.height / 2
        : drag.y < rect.top ? false : drag.y > rect.bottom ? true : drag.x > rect.left + rect.width / 2;
      return { row, distance: dx * dx + dy * dy, after };
    });
    drag.target = targets.reduce((nearest, target) => target.distance < nearest.distance ? target : nearest);
    drag.target.row.classList.add(drag.target.after ? 'item-drop-after' : 'item-drop-before');
    drag.target.row.classList.toggle('item-drop-side', !singleColumn && !drag.target.row.classList.contains('is-editing'));
    return;
  }
  const seams = others.map(row => ({ row, after: false, y: row.getBoundingClientRect().top }));
  seams.push({ row: others.at(-1), after: true, y: others.at(-1).getBoundingClientRect().bottom });
  drag.target = seams.reduce((nearest, seam) => Math.abs(drag.y - seam.y) < Math.abs(drag.y - nearest.y) ? seam : nearest);
  drag.target.row.classList.add(drag.target.after ? 'item-drop-after' : 'item-drop-before');
}

function itemDragFrame() {
  const drag = itemPointerDrag;
  if (!drag?.started) return;
  const bounds = itemList.getBoundingClientRect();
  if (drag.x >= bounds.left && drag.x <= bounds.right && drag.y >= bounds.top && drag.y <= bounds.bottom) {
    if (drag.y < bounds.top + 32) itemList.scrollTop -= Math.ceil((bounds.top + 32 - drag.y) / 3);
    else if (drag.y > bounds.bottom - 32) itemList.scrollTop += Math.ceil((drag.y - bounds.bottom + 32) / 3);
  }
  updateItemDropSeam();
  const x = Math.max(8, Math.min(drag.x + 14, innerWidth - drag.preview.offsetWidth - 8));
  const y = Math.max(8, Math.min(drag.y + 14, innerHeight - drag.preview.offsetHeight - 8));
  drag.preview.style.transform = `translate3d(${x}px, ${y}px, 0)`;
  drag.frame = requestAnimationFrame(itemDragFrame);
}

function finishItemPointerDrag(commit) {
  const drag = itemPointerDrag;
  if (!drag) return;
  itemPointerDrag = null;
  cancelAnimationFrame(drag.frame);
  try { if (itemList.hasPointerCapture(drag.pointerId)) itemList.releasePointerCapture(drag.pointerId); } catch (_) { /* Already released. */ }
  drag.row.classList.remove('is-item-dragging');
  itemList.classList.remove('is-reordering');
  document.body.classList.remove('item-pointer-dragging');
  drag.preview?.remove();
  clearItemDropSeam();
  if (drag.started) {
    suppressItemClick = true;
  }
  if (commit && drag.started && drag.valid && drag.target) {
    const ids = drag.visibleOrder.filter(id => id !== drag.id);
    const position = ids.indexOf(drag.target.row.dataset.itemId) + (drag.target.after ? 1 : 0);
    ids.splice(position, 0, drag.id);
    void saveItemOrder(ids, drag.expectedOrder, drag.id);
  } else if (drag.started) renderConnections();
}

itemList.addEventListener('dragstart', event => {
  if (event.target.closest?.('[data-item-reorder]')) event.preventDefault();
});
itemList.addEventListener('pointerdown', event => {
  const handle = event.target.closest?.('[data-item-reorder]');
  if (!handle || event.button !== 0 || !event.isPrimary || event.ctrlKey || event.metaKey || event.shiftKey || itemPointerDrag || itemOrderSaving || confirmationResolver || !ownerLockState.unlocked) return;
  event.preventDefault();
  handle.focus({ preventScroll: true });
  itemPointerDrag = { id: handle.dataset.itemReorder, item: state.connections.find(item => item.id === handle.dataset.itemReorder), row: handle.closest('.connection-card'), pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, x: event.clientX, y: event.clientY, started: false, visibleOrder: visibleItemIds(), expectedOrder: state.connections.map(item => item.id) };
});
document.addEventListener('pointermove', event => {
  const drag = itemPointerDrag;
  if (!drag || event.pointerId !== drag.pointerId) return;
  drag.x = event.clientX; drag.y = event.clientY;
  if (!drag.started && Math.hypot(drag.x - drag.startX, drag.y - drag.startY) < 6) return;
  event.preventDefault();
  if (!drag.started) {
    drag.started = true;
    drag.row.classList.add('is-item-dragging');
    itemList.classList.add('is-reordering');
    document.body.classList.add('item-pointer-dragging');
    const item = drag.item;
    drag.preview = document.createElement('div');
    drag.preview.className = 'item-drag-preview';
    drag.preview.setAttribute('aria-hidden', 'true');
    drag.preview.innerHTML = `${itemTypeIcon(itemDisplayKind(item))}<span>${escapeHtml(item.name)}</span>`;
    document.body.appendChild(drag.preview);
    try { itemList.setPointerCapture(event.pointerId); } catch (_) { /* Document tracking remains active. */ }
    itemDragFrame();
  }
  updateItemDropSeam();
}, { passive: false });
document.addEventListener('pointerup', event => {
  if (itemPointerDrag?.pointerId !== event.pointerId) return;
  if (itemPointerDrag.started) { event.preventDefault(); updateItemDropSeam(); }
  finishItemPointerDrag(true);
});
document.addEventListener('pointercancel', event => { if (itemPointerDrag?.pointerId === event.pointerId) finishItemPointerDrag(false); });
itemList.addEventListener('lostpointercapture', event => { if (itemPointerDrag?.pointerId === event.pointerId) finishItemPointerDrag(false); });
window.addEventListener('blur', () => finishItemPointerDrag(false));
document.addEventListener('click', event => {
  if (!suppressItemClick || event.detail === 0) return;
  suppressItemClick = false;
  event.preventDefault(); event.stopImmediatePropagation();
}, true);
itemList.addEventListener('keydown', event => {
  const handle = event.target.closest?.('[data-item-reorder]');
  if (!handle || itemPointerDrag || itemOrderSaving || confirmationResolver || !ownerLockState.unlocked || event.altKey || event.ctrlKey || event.metaKey || !['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
  event.preventDefault(); event.stopPropagation();
  const ids = visibleItemIds(), id = handle.dataset.itemReorder, from = ids.indexOf(id);
  const to = event.key === 'Home' ? 0 : event.key === 'End' ? ids.length - 1 : Math.max(0, Math.min(ids.length - 1, from + (event.key === 'ArrowUp' ? -1 : 1)));
  ids.splice(from, 1); ids.splice(to, 0, id);
  void saveItemOrder(ids, state.connections.map(item => item.id), id);
});

let draggedModuleRow = null;
let moduleDropTarget = null;
let moduleDropAfter = false;
let moduleDragPreview = null;
let modulePointerDrag = null;
let suppressModuleClick = false;
const moduleList = $('#module-list');
function clearModuleDropSeam() {
  $$('.module-row.drop-before, .module-row.drop-after', moduleList).forEach((row) => row.classList.remove('drop-before', 'drop-after'));
  moduleDropTarget = null;
  moduleDropAfter = false;
}

function beginModulePointerDrag(event) {
  draggedModuleRow = modulePointerDrag.row;
  draggedModuleRow.classList.add('is-dragging');
  moduleList.classList.add('is-sorting');
  document.body.classList.add('module-pointer-dragging');
  const definition = MODULE_DEFS[draggedModuleRow.dataset.kind] || MODULE_DEFS.customSecret;
  moduleDragPreview = document.createElement('div');
  moduleDragPreview.className = 'module-drag-preview';
  moduleDragPreview.innerHTML = `<b>${escapeHtml(definition.code)}</b><span>${escapeHtml(moduleLabel(draggedModuleRow.dataset.kind))}</span>`;
  document.body.appendChild(moduleDragPreview);
  modulePointerDrag.started = true;
  try { draggedModuleRow.setPointerCapture(event.pointerId); } catch (_) { /* document tracking remains active */ }
}

function updateModuleDropSeam(clientY) {
  clearModuleDropSeam();
  const rows = $$('.module-row', moduleList);
  if (!rows.length) return;
  const seams = rows.map((row) => ({ row, after: false, y: row.getBoundingClientRect().top }));
  const lastRow = rows.at(-1);
  seams.push({ row: lastRow, after: true, y: lastRow.getBoundingClientRect().bottom });
  const closest = seams.reduce((best, seam) => Math.abs(clientY - seam.y) < Math.abs(clientY - best.y) ? seam : best);
  moduleDropTarget = closest.row;
  moduleDropAfter = closest.after;
  moduleDropTarget.classList.add(moduleDropAfter ? 'drop-after' : 'drop-before');
}

function positionModuleDragPreview(clientX, clientY) {
  if (moduleDragPreview) moduleDragPreview.style.transform = `translate3d(${Math.round(clientX + 14)}px, ${Math.round(clientY + 14)}px, 0)`;
}

function autoScrollModuleEditor(clientY) {
  const scroller = inlineEditorItemId ? $('#connections-list') : $('.form-scroll', $('#connection-modal'));
  if (!scroller) return;
  const bounds = scroller.getBoundingClientRect();
  if (clientY < bounds.top + 34) scroller.scrollTop -= 12;
  else if (clientY > bounds.bottom - 34) scroller.scrollTop += 12;
}

function finishModulePointerDrag(commit) {
  if (!modulePointerDrag) return;
  const { row, pointerId, started } = modulePointerDrag;
  if (started && commit && moduleDropTarget) {
    moduleList.insertBefore(row, moduleDropAfter ? moduleDropTarget.nextElementSibling : moduleDropTarget);
  }
  try { if (row.hasPointerCapture(pointerId)) row.releasePointerCapture(pointerId); } catch (_) { /* capture may already be released */ }
  row.classList.remove('is-dragging');
  moduleList.classList.remove('is-sorting');
  document.body.classList.remove('module-pointer-dragging');
  moduleDragPreview?.remove();
  moduleDragPreview = null;
  draggedModuleRow = null;
  modulePointerDrag = null;
  clearModuleDropSeam();
  if (started && commit) {
    syncModuleDraft();
    $('[data-module-drag-handle]', row)?.focus({ preventScroll: true });
    announceModuleOrder(row);
  }
}

function announceModuleOrder(row) {
  const rows = $$('.module-row', moduleList);
  const name = $('[data-module-name]', row)?.value.trim() || moduleLabel(row.dataset.kind);
  const position = rows.indexOf(row) + 1;
  $('#module-reorder-status').textContent = localized(`${name}：第 ${position} 项，共 ${rows.length} 项`, `${name}: position ${position} of ${rows.length}`);
}

moduleList.addEventListener('dragstart', (event) => {
  if (event.target.closest?.('[data-module-drag-handle]')) event.preventDefault();
});
moduleList.addEventListener('pointerdown', (event) => {
  if (event.button !== 0 || !event.isPrimary || confirmationResolver) return;
  const handle = event.target.closest?.('[data-module-drag-handle]');
  const row = handle?.closest('.module-row');
  if (!row || modulePointerDrag) return;
  event.preventDefault();
  handle.focus({ preventScroll: true });
  modulePointerDrag = { row, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, started: false };
});
moduleList.addEventListener('keydown', (event) => {
  const handle = event.target.closest?.('[data-module-drag-handle]');
  if (!handle || confirmationResolver || modulePointerDrag || !['ArrowUp', 'ArrowDown'].includes(event.key)) return;
  event.preventDefault();
  const row = handle.closest('.module-row');
  const sibling = event.key === 'ArrowUp' ? row.previousElementSibling : row.nextElementSibling;
  if (!sibling) return;
  moduleList.insertBefore(row, event.key === 'ArrowUp' ? sibling : sibling.nextElementSibling);
  syncModuleDraft();
  handle.focus({ preventScroll: true });
  row.scrollIntoView({ block: 'nearest' });
  announceModuleOrder(row);
});
document.addEventListener('pointermove', (event) => {
  if (!modulePointerDrag || event.pointerId !== modulePointerDrag.pointerId) return;
  if (!modulePointerDrag.started && Math.hypot(event.clientX - modulePointerDrag.startX, event.clientY - modulePointerDrag.startY) < 6) return;
  if (!modulePointerDrag.started) beginModulePointerDrag(event);
  event.preventDefault();
  autoScrollModuleEditor(event.clientY);
  updateModuleDropSeam(event.clientY);
  positionModuleDragPreview(event.clientX, event.clientY);
}, { passive: false });
document.addEventListener('pointerup', (event) => {
  if (!modulePointerDrag || event.pointerId !== modulePointerDrag.pointerId) return;
  if (modulePointerDrag.started) {
    event.preventDefault();
    suppressModuleClick = true;
    setTimeout(() => { suppressModuleClick = false; }, 0);
  }
  finishModulePointerDrag(true);
});
document.addEventListener('pointercancel', (event) => {
  if (modulePointerDrag?.pointerId === event.pointerId) finishModulePointerDrag(false);
});
document.addEventListener('click', (event) => {
  if (!suppressModuleClick) return;
  suppressModuleClick = false;
  if (event.target.closest?.('.module-row')) {
    event.preventDefault();
    event.stopImmediatePropagation();
  }
}, true);
window.addEventListener('blur', () => {
  if (modulePointerDrag) finishModulePointerDrag(false);
});

document.addEventListener('paste', (event) => {
  if (!event.target.matches('.pin-cell-input')) return;
  const digits = event.clipboardData?.getData('text').replace(/\D/g, '').slice(0, 6) || '';
  if (!digits) return;
  event.preventDefault();
  const inputs = $$('.pin-cell-input', event.target.closest('.pin-control'));
  inputs.forEach((input, index) => { input.value = digits[index] || ''; });
  (inputs[Math.min(digits.length, inputs.length) - 1] || inputs[0])?.focus();
});

document.addEventListener('keydown', (event) => {
  if (totpQrImport) return;
  if (apiServicePicker) {
    if (event.key === 'Escape') { event.preventDefault(); closeApiServicePicker(); return; }
    if (event.key === 'Tab') {
      const controls = $$('button:not(:disabled), input, select', $('#api-service-modal')).filter(control => control.getClientRects().length && !control.closest('.hidden'));
      const first = controls[0], last = controls.at(-1);
      if (event.shiftKey && (document.activeElement === first || !controls.includes(document.activeElement)) || !event.shiftKey && (document.activeElement === last || !controls.includes(document.activeElement))) {
        event.preventDefault(); (event.shiftKey ? last : first)?.focus();
      }
    }
    return;
  }
  if (event.key === 'Escape' && switchPointerDrag) {
    event.preventDefault(); void finishSwitchDrag(false); return;
  }
  if (event.key === 'Escape' && itemPointerDrag) {
    event.preventDefault(); finishItemPointerDrag(false); return;
  }
  if (confirmationResolver) {
    if (event.key === 'Escape') {
      event.preventDefault();
      finishConfirmation(false);
      return;
    }
    if (event.key === 'Tab') {
      const focusable = $$('#confirmation-modal button, #confirmation-details:not(.hidden)');
      const index = focusable.indexOf(document.activeElement);
      event.preventDefault();
      focusable[(index + (event.shiftKey ? -1 : 1) + focusable.length) % focusable.length].focus();
    }
    return;
  }
  if (event.key === 'Escape' && modulePointerDrag) {
    event.preventDefault();
    finishModulePointerDrag(false);
    return;
  }
  if (event.key === 'Escape' && vaultManagement) {
    event.preventDefault();
    closeVaultManagement();
    return;
  }
  // Like Bricktone's WorkspaceDialog, keep keyboard navigation in the editor.
  // Nested dialogs keep their own keyboard handling and draft saving stays intact.
  const editor = $('#connection-modal');
  const editorIsTopmost = !editor.classList.contains('hidden') &&
    !$$('.modal-backdrop:not(.hidden)').some(modal => modal !== editor);
  const editorOwnsKey = !inlineEditorItemId || event.key === 'Escape' && (editor.contains(event.target) || $('#module-menu').contains(event.target));
  if (editorIsTopmost && editorOwnsKey && ['Tab', 'Escape'].includes(event.key) && !event.defaultPrevented) {
    const menu = $('#module-menu');
    if (event.key === 'Escape') {
      event.preventDefault();
      if (!menu.classList.contains('hidden')) { setModuleMenu(false); $('#add-module-button').focus(); }
      else void closeEditorWithDraft();
      return;
    }
    const selector = 'button:not(:disabled), input:not(:disabled):not([type="hidden"]), select:not(:disabled), textarea:not(:disabled), summary, [tabindex="0"]';
    const controls = [...$$(selector, editor), ...(!editor.contains(menu) && !menu.classList.contains('hidden') ? $$(selector, menu) : [])]
      .filter(control => control.getClientRects().length && !control.closest('[inert], .hidden'));
    const first = controls[0], last = controls.at(-1);
    if (first && (event.shiftKey && (document.activeElement === first || !controls.includes(document.activeElement)) ||
      !event.shiftKey && (document.activeElement === last || !controls.includes(document.activeElement)))) {
      event.preventDefault();
      (event.shiftKey ? last : first).focus();
    }
  }
  if (event.target.closest('.creation-options') && ['ArrowLeft', 'ArrowRight'].includes(event.key)) {
    event.preventDefault();
    const button = $(event.key === 'ArrowLeft' ? '#add-connection-button' : '#drafts-button');
    if (!button.disabled) { button.focus(); button.click(); }
    return;
  }
  if (event.target.matches('.pin-cell-input')) {
    const inputs = $$('.pin-cell-input', event.target.closest('.pin-control'));
    const index = inputs.indexOf(event.target);
    if (event.key === 'Backspace' && !event.target.value && index > 0) {
      event.preventDefault();
      inputs[index - 1].value = '';
      inputs[index - 1].focus();
      return;
    }
    if (event.key === 'ArrowLeft' && index > 0) { event.preventDefault(); inputs[index - 1].focus(); return; }
    if (event.key === 'ArrowRight' && index < inputs.length - 1) { event.preventDefault(); inputs[index + 1].focus(); return; }
  }
  const filterButton = event.target.closest?.('.filter-strip button');
  if (filterButton && !switchSelectionPending && ['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
    event.preventDefault();
    const strip = filterButton.closest('.filter-strip');
    const buttons = $$('button', strip);
    const index = buttons.findIndex(button => button.getAttribute('aria-pressed') === 'true');
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowLeft' ? -1 : 1) + buttons.length) % buttons.length;
    void selectFilterButton(buttons[next]).then(() => $('[aria-pressed="true"]', strip).focus());
    return;
  }
  const pageSearchAvailable = (inlineEditorItemId || $('#connection-modal').classList.contains('hidden')) && $('#owner-lock-layer').classList.contains('hidden') && !githubBackup.isOpen();
  if (pageSearchAvailable && (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f' && !event.altKey) {
    event.preventDefault();
    setTitleSearchOpen(true);
    const search = $(`[data-page-search="${activePage}"]`);
    search?.focus();
    search?.select();
    return;
  }
  if (event.key === 'Escape') {
    const search = event.target.closest?.('#title-search') ? $('#title-search-input') : null;
    if (search?.value) {
      const page = search.dataset.pageSearch;
      search.value = '';
      pageSearch[page] = '';
      if (page === 'connections') { syncTitleSearch(); renderConnections(); }
      if (page === 'activity') { activityVisibleCount = ACTIVITY_PAGE_SIZE; renderActivity(); }
      if (page === 'settings') applySettingsSearch();
      event.preventDefault();
      return;
    }
    if (event.target.closest?.('#title-search')) { event.preventDefault(); setTitleSearchOpen(false); }
  }
});

$('#module-list').addEventListener('change', event => {
  if (!event.target.matches('[data-fill-oci-region]') || !event.target.value) return;
  const row = event.target.closest('.module-row');
  const field = $('[data-secret-value]', row);
  if (field) field.value = event.target.value;
  event.target.value = '';
  syncModuleDraft();
  updateOciModuleUrl();
});

$('#http-auth-type').addEventListener('change', () => {
  syncModuleDraft();
  editorHttpAuthType = value('#http-auth-type');
  if (editorHttpAuthType === 'oci-signature') editorOciBindings ||= {};
  if (editorHttpAuthType === 'aws-sigv4') editorAwsBindings ||= {};
  renderModules();
});
$('#aws-binding-fields').addEventListener('change', event => {
  const role = event.target.dataset.awsBinding;
  if (!role) return;
  syncModuleDraft();
  editorAwsBindings ||= {};
  editorAwsBindings[role] = event.target.value || null;
  renderModules();
});
$('#oci-binding-fields').addEventListener('change', event => {
  const role = event.target.dataset.ociBinding;
  if (!role) return;
  syncModuleDraft();
  editorOciBindings ||= {};
  editorOciBindings[role] = event.target.value || null;
  renderModules();
});

$('#connection-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  if (moduleKeyStaging || moduleKeyInput) {
    toast(localized('请先确认导入或取消密钥输入', 'Import the key or cancel key input before saving'), 'error');
    return;
  }
  const button = $('#save-connection-button');
  button.disabled = true;
  try {
    const input = serializeItem();
    await api.save(input);
    if (currentDraftId) await api.deleteDraft(currentDraftId);
    currentDraftId = '';
    await animateInlineEditor(false);
    closeEditor();
    toast(localized('项目已加密保存', 'Item saved and encrypted'));
    await refresh();
    await refreshDrafts();
  } catch (error) { toast(cleanError(error), 'error'); } finally { button.disabled = false; }
});
$('#owner-lock-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  if (ownerLockPending) return;
  const pin = readPin('owner-pin');
  if (!/^\d{6}$/.test(pin)) return toast(localized('PIN 必须是六位数字', 'PIN must contain six digits'), 'error');
  if (!ownerLockState.pinConfigured && pin !== readPin('owner-pin-confirm')) return toast(localized('两次 PIN 不一致', 'PIN entries do not match'), 'error');
  const button = $('#owner-unlock-action');
  const cancelButton = $('#owner-pin-cancel');
  button.disabled = true;
  cancelButton.disabled = true;
  try {
    const settingPin = !ownerLockState.pinConfigured;
    const epoch = ownerEpoch;
    const next = settingPin ? await api.ownerSetPin(pin) : await api.ownerUnlock(pin);
    if (epoch !== ownerEpoch) return;
    ownerPinSetupRequested = false;
    applyOwnerLockState(next);
    if (settingPin) toast(localized('PIN 锁已开启', 'PIN lock enabled'));
    await refresh();
    await refreshDrafts(true);
  } catch (error) {
    const message = String(error?.message || error);
    const cooldown = message.match(/(?:wait|retry in)\s+(\d+)\s*(?:s\b|seconds?\b)/i);
    toast(cooldown
      ? localized(`PIN 尝试次数过多，请在 ${cooldown[1]} 秒后重试`, `Too many PIN attempts. Try again in ${cooldown[1]} seconds.`)
      : message === 'Incorrect PIN' ? localized('PIN 不正确', message) : cleanError(error), 'error');
    clearPin('owner-pin');
    focusPin('owner-pin');
  } finally {
    button.disabled = false;
    cancelButton.disabled = false;
  }
});
$('#owner-pin-cancel').addEventListener('click', () => {
  ownerPinSetupRequested = false;
  renderOwnerLock();
  renderSettings();
});
$('#confirmation-modal').addEventListener('click', (event) => {
  if (event.target === $('#confirmation-modal') || event.target.closest('#confirmation-cancel')) finishConfirmation(false);
  if (event.target.closest('#confirmation-accept')) finishConfirmation(true);
});
$('#clear-activity-button').addEventListener('click', async () => {
  if (!await confirmDestructiveAction({
    title: localized('清空本地操作记录？', 'Clear the local activity log?'),
    message: localized('全部本地操作记录将永久删除。', 'All local activity entries will be permanently deleted.'),
    acceptLabel: localized('清空', 'CLEAR'),
    tag: localized('确认清空记录', 'CONFIRM LOG DELETION'),
  })) return;
  try {
    await api.clear();
    currentActivityFilter = 'all';
    pageSearch.activity = '';
    expandedActivityErrors.clear();
    $('[data-page-search="activity"]').value = '';
    await refresh();
  } catch (error) { toast(cleanError(error), 'error'); }
});
$('#save-browser-settings-button').addEventListener('click', saveBrowserSettings);
$('#browser-enabled').addEventListener('change', saveBrowserSettings);
if (!isMacOS) $('#desktop-shortcut-enabled').addEventListener('change', (event) => setSystemIntegration('desktop', event.currentTarget.checked));
$('#launch-at-login-enabled').addEventListener('change', (event) => setSystemIntegration('startup', event.currentTarget.checked));
$('#pin-enabled').addEventListener('change', savePinSettings);
$('#close-behavior').addEventListener('change', saveDesktopSettings);
$('#quick-pairing-button').addEventListener('click', async event => {
  const button = event.currentTarget;
  button.disabled = true;
  try {
    $('#browser-open-help').hidden = true;
    const errors = [];
    try {
      await api.copyOwnerValue('chrome://extensions/', false);
      $('#browser-open-help').hidden = false;
    } catch (error) { errors.push(error); }
    try {
      await api.quickPair(Number($('#browser-port').value));
      $('#browser-enabled').checked = true;
    } catch (error) { errors.push(error); }
    try { await api.openBrowser(); } catch (error) { errors.push(error); }
    if (errors.length) errors.forEach(error => toast(cleanError(error), 'error'));
    else toast(localized('地址已复制，配对已准备。', 'Address copied. Pairing is ready.'));
    await refresh(false);
  } catch (error) {
    toast(cleanError(error), 'error');
  } finally {
    button.disabled = false;
  }
});
$('#reset-pairing-button').addEventListener('click', async () => {
  if (!await confirmDestructiveAction({
    title: localized('重置浏览器配对？', 'Reset browser pairing?'),
    message: localized('所有已配对扩展会立即失效，需要重新配对才能继续使用。', 'Every paired extension will be revoked immediately and must be paired again.'),
    acceptLabel: localized('重置', 'RESET'),
    tag: localized('确认重置配对', 'CONFIRM PAIRING RESET'),
  })) return;
  try { await api.resetPair(); toast(localized('配对已重置', 'Pairing reset')); await refresh(); } catch (error) { toast(cleanError(error), 'error'); }
});
$('#open-extension-button').addEventListener('click', () => api.extensionFolder().catch((error) => toast(cleanError(error), 'error')));
$('#open-data-button').addEventListener('click', () => api.dataFolder().catch((error) => toast(cleanError(error), 'error')));
$('#rescan-agents').addEventListener('click', () => scanAgents(true));
$('#header-export-backup-button').addEventListener('click', (event) => exportBackup(event.currentTarget));
$('#header-import-backup-button').addEventListener('click', (event) => importBackup(event.currentTarget));

let generatorTarget = null, generatorSequence = 0;
let backupPasswordResolve = null, backupPasswordExport = false;
function finishBackupPassword(result) {
  $('#backup-password-form').reset(); $('#backup-password-modal').classList.add('hidden');
  const resolve = backupPasswordResolve; backupPasswordResolve = null; resolve?.(result);
}
function askBackupPassword(exporting) {
  finishBackupPassword(null); backupPasswordExport = exporting;
  $('#backup-password-title').textContent = exporting ? localized('导出加密备份', 'EXPORT ENCRYPTED BACKUP') : localized('恢复备份', 'RESTORE BACKUP');
  $('#backup-password-help').textContent = exporting
    ? localized('设置至少 12 个字符的备份密码。恢复时需要此密码，请妥善保管。', 'Use a backup password of at least 12 characters. Keep it for restoration.')
    : localized('输入创建备份时设置的密码；旧版自动解锁备份可留空。', 'Enter the backup password. Leave blank for older automatically unlocked backups.');
  $('#backup-password-confirm-field').classList.toggle('hidden', !exporting);
  $('#backup-password').required = exporting;
  $('#backup-password-confirm').required = exporting;
  $('#backup-password-modal').classList.remove('hidden'); $('#backup-password').focus();
  return new Promise(resolve => { backupPasswordResolve = resolve; });
}
$('#backup-password-close').onclick = () => finishBackupPassword(null);
$('#backup-password-form').onsubmit = e => {
  e.preventDefault();
  const password = $('#backup-password').value;
  if (backupPasswordExport && ([...password].length < 12 || password !== $('#backup-password-confirm').value))
    return toast(localized('密码须至少 12 个字符，且两次输入一致', 'Use at least 12 characters and matching passwords'), 'error');
  finishBackupPassword(password);
};
function closeGenerator() {
  generatorSequence++;
  $('#generator-modal').classList.add('hidden'); $('#generator-result').value = ''; generatorTarget = null;
}
async function generatePassword() {
  const sequence = ++generatorSequence;
  try {
    const options = { length: Number($('#generator-length').value) };
    for (const key of ['uppercase', 'lowercase', 'digits', 'symbols']) options[key] = $(`#generator-${key}`).checked;
    const result = await invoke('generate_password', { options });
    if (sequence === generatorSequence && !$('#generator-modal').classList.contains('hidden')) $('#generator-result').value = result;
  } catch (error) { toast(cleanError(error), 'error'); }
}
function openGenerator(target = null) {
  generatorTarget = target; $('#generator-use').classList.toggle('hidden', !target);
  $('#generator-modal').classList.remove('hidden'); generatePassword();
}
$('#generator-open').onclick = () => openGenerator();
$('#generator-close').onclick = closeGenerator;
$('#generator-regenerate').onclick = generatePassword;
$('#generator-copy').onclick = async () => { try { if ($('#generator-result').value) { await api.copyOwnerValue($('#generator-result').value); toast(localized('已复制', 'Copied')); } } catch(error) { toast(cleanError(error), 'error'); } };
$('#generator-use').onclick = () => {
  if (generatorTarget?.isConnected && $('#generator-result').value) { generatorTarget.value = $('#generator-result').value; generatorTarget.dispatchEvent(new Event('input', { bubbles:true })); syncModuleDraft(); }
  closeGenerator();
};
document.addEventListener('click', e => {
  const button = e.target.closest('[data-generate-password]');
  if (button) openGenerator($('[data-secret-value]', button.closest('.module-row')));
});

$('#page-activity .activity-panel').addEventListener('scroll', (event) => {
  const panel = event.currentTarget;
  if (panel.scrollHeight - panel.scrollTop - panel.clientHeight < 160) queueMoreActivities();
}, { passive: true });

window.__TAURI__.event.listen('state-changed', async () => { await refresh(false); await refreshDrafts(); });
window.__TAURI__.event.listen('pin-setup-requested', async () => {
  await refreshOwnerLock(false);
  if (ownerLockState.pinConfigured) return;
  ownerPinSetupRequested = true;
  renderOwnerLock();
  if (state) renderSettings();
});
window.__TAURI__.event.listen('owner-locked', () => {
  if (ownerLockState.pinConfigured) applyOwnerLockState({ ...ownerLockState, unlocked: false, expiresInSeconds: 0 });
});
window.addEventListener('focus', async () => { await refreshOwnerLock(false); await refresh(false); await refreshDrafts(); });
for (const event of ['pointerdown', 'keydown', 'wheel']) {
  window.addEventListener(event, reportOwnerActivity, { capture: true, passive: true });
}
window.addEventListener('resize', positionModuleMenu);
$('.form-scroll', $('#connection-modal')).addEventListener('scroll', positionModuleMenu, { passive: true });
$('#connections-list').addEventListener('scroll', positionModuleMenu, { passive: true });
setInterval(() => { if ($('#page-activity').classList.contains('active')) refresh(false); }, 2000);
setInterval(() => { if (!document.hidden && $('#connection-modal').classList.contains('hidden') && $('#page-connections').classList.contains('active')) refresh(false); }, 2000);
let extensionDragStart = null, extensionDragBusy = false;
$('#drag-extension-button').addEventListener('pointerdown', event => { if(event.button === 0) extensionDragStart = {x:event.clientX,y:event.clientY}; });
window.addEventListener('pointerup', () => { extensionDragStart = null; });
window.addEventListener('pointercancel', () => { extensionDragStart = null; });
$('#drag-extension-button').addEventListener('pointermove', async event => {
  if(event.buttons !== 1 || !extensionDragStart || extensionDragBusy || Math.hypot(event.clientX-extensionDragStart.x,event.clientY-extensionDragStart.y)<5)return;
  extensionDragStart=null; extensionDragBusy=true;
  try { await invoke('drag_browser_extension'); } catch(error) { toast(cleanError(error), 'error'); } finally { extensionDragBusy=false; }
});
$('#drag-extension-button').addEventListener('dragstart', event => event.preventDefault());
$('#drag-extension-button').addEventListener('keydown', event => { if(event.key==='Enter' || event.key===' ') { event.preventDefault(); api.extensionFolder().catch(error=>toast(cleanError(error),'error')); } });

async function bootstrap() {
  applyLanguage();
  initLettering();
  await refreshOwnerLock();
  await refresh();
  await refreshDrafts();
  scanAgents();
}
bootstrap();

$('#agent-write-enabled').addEventListener('change', () => saveAgentWriteSettings({agentWriteEnabled: $('#agent-write-enabled').checked}));
$('#agent-write-add').addEventListener('click', addAgentWriteFolder);
$('#agent-write-folder').addEventListener('keydown', event => { if (event.key === 'Enter') { event.preventDefault(); addAgentWriteFolder(); } });
$('#agent-write-folders').addEventListener('click', event => {
  const button = event.target.closest('[data-remove-write-folder]');
  if (button) saveAgentWriteSettings({agentWriteFolders: (state.settings.agentWriteFolders || []).filter((_, index) => index !== Number(button.dataset.removeWriteFolder))});
});
