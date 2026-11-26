function createCertificatePanel({ invoke, getState, isUnlocked, localized: l, escapeHtml: e, toast, cleanError, refresh }) {
  const panel = document.querySelector('#certificate-panel');
  let capabilities = { enabled: false, grants: [], authorities: [] };
  let busy = false;
  let epoch = 0;
  const q = selector => panel.querySelector(selector);
  const label = (id, text, control) => `<label class="field"><span id="${id}-label">${text}</span>${control.replace('id="x"', `id="${id}"`)}</label>`;
  const select = (id, text) => label(id, text, '<select class="input" id="x"></select>');
  const field = (id, text, attributes = '') => label(id, text, `<input class="input" id="x" ${attributes}>`);
  const area = (id, text, attributes = '') => label(id, text, `<textarea class="input textarea" id="x" rows="5" maxlength="1048576" autocomplete="off" spellcheck="false" ${attributes}></textarea>`);
  const button = (id, text) => `<button type="button" class="small-button" id="${id}">${text}</button>`;
  panel.innerHTML = `<div class="settings-heading"><div><h2>${l('证书与签发权限', 'Certificates and signing permissions')}</h2><p>${l('使用已有 CA。签发权限独立于保险库写入权限，默认关闭。', 'Use an existing CA. Signing permissions are separate from vault writes and disabled by default.')}</p></div></div>
    <div class="desktop-option"><span>${l('允许 Agent 签发证书', 'Allow Agent certificate issuance')}</span><label class="switch"><input type="checkbox" id="certificate-enabled" aria-label="${l('允许 Agent 签发证书', 'Allow Agent certificate issuance')}"><span></span></label></div>
    <p id="certificate-status" role="status"></p><div id="certificate-grants" class="agent-client-list"></div>
    <details id="certificate-ca-management"><summary>${l('管理已有 CA 和授权', 'Manage existing CAs and grants')}</summary>
      <p>${l('以下操作立即生效。注册 CA 后，Agent 只能通过签发工具使用私钥，不能通过通用凭据工具使用。撤权不会吊销已签发证书。', 'Changes apply immediately. Registered CA secrets are restricted to issuance tools. Revoking a grant does not revoke issued certificates.')}</p>
      ${select('certificate-item', l('项目', 'Project'))}<div class="field-grid two">${select('certificate-ca-kind', l('CA 类型', 'CA kind'))}${select('certificate-private-key', l('私钥模块', 'Private key module'))}${select('certificate-ca-module', l('TLS CA 证书模块', 'TLS CA certificate module'))}</div>
      <div class="button-row">${button('certificate-register', l('注册 CA', 'Register CA'))}${button('certificate-unregister', l('解除 CA 用途限制', 'Unregister CA'))}</div>
      <div class="field-grid two">${select('certificate-grant-kind', l('允许签发的用途', 'Allowed certificate purpose'))}${field('certificate-max-ttl', l('最长有效秒数', 'Maximum lifetime in seconds'), 'type="number" min="1" max="31622400" value="31622400"')}</div>${button('certificate-allow', l('替换为此用途授权', 'Replace grant with this purpose'))}
    </details>
    <details id="certificate-issue-panel"><summary>${l('签发证书', 'Issue certificate')}</summary>
      <div id="certificate-issue-empty">
        <p>${l('还没有注册的 CA。保存签发材料后，请注册 CA。', 'No CA is registered. Register a CA after saving its signing materials.')}</p>
        <div class="button-row">${button('certificate-open-ca-management', l('前往注册 CA', 'Go to CA registration'))}</div>
      </div>
      <div id="certificate-issue-fields">
        ${select('certificate-issue-item', l('已注册的 CA 项目', 'Registered CA project'))}${select('certificate-issue-kind', l('证书用途', 'Certificate purpose'))}
        ${area('certificate-issue-payload', l('待签 SSH 公钥', 'SSH public key to sign'))}
        ${field('certificate-issue-names', l('身份名称，逗号分隔', 'Identities, comma separated'))}
        <div class="field-grid two">${field('certificate-key-id', 'SSH key ID')}${field('certificate-ttl', l('有效秒数', 'Lifetime in seconds'), 'type="number" min="1" max="31622400" value="3600"')}</div>
        ${field('certificate-request-id', l('签发请求 UUID，重试保留同一值', 'Issuance request UUID, preserve when retrying'))}
        <div class="button-row">${button('certificate-issue', l('签发证书', 'Issue certificate'))}${button('certificate-new-request', l('新请求 ID', 'New request ID'))}</div>
        ${area('certificate-issue-result', l('签发结果', 'Issuance result'), 'readonly')}
      </div>
    </details>
    <details id="certificate-verify-panel"><summary>${l('验证证书', 'Verify certificate')}</summary>
      ${select('certificate-verify-kind', l('证书用途', 'Certificate purpose'))}
      ${area('certificate-verify-payload', l('待验证证书', 'Certificate to verify'))}
      ${field('certificate-verify-identity', l('要验证的身份，填一个', 'Identity to verify, enter one'))}
      ${area('certificate-authority-input', l('受信任的 CA 公钥', 'Trusted CA public key'))}
      ${area('certificate-intermediates', l('TLS 中间证书链，可选', 'TLS intermediate chain, optional'))}
      <div class="button-row">${button('certificate-verify', l('验证证书', 'Verify certificate'))}</div>
      ${area('certificate-verify-result', l('验证结果', 'Verification result'), 'readonly')}
    </details>
    <details id="certificate-match-panel"><summary>${l('检查密钥匹配', 'Check key match')}</summary>
      ${select('certificate-match-item', l('私钥所在项目', 'Project containing the private key'))}
      ${area('certificate-match-payload', l('待匹配的公钥或证书', 'Public key or certificate to match'))}
      <div class="button-row">${button('certificate-match', l('检查密钥匹配', 'Check key match'))}</div>
      ${area('certificate-match-result', l('匹配结果', 'Key match result'), 'readonly')}
    </details>`;
  const kinds = [['sshUser', l('SSH 用户', 'SSH user')], ['sshHost', l('SSH 主机', 'SSH host')], ['tlsServer', l('TLS 服务端', 'TLS server')], ['tlsClient', l('TLS 客户端', 'TLS client')]];
  for (const selector of ['#certificate-issue-kind', '#certificate-verify-kind', '#certificate-grant-kind']) q(selector).replaceChildren(...kinds.map(([value, text]) => new Option(text, value)));
  q('#certificate-ca-kind').replaceChildren(new Option('SSH', 'ssh'), new Option('TLS', 'tls'));
  q('#certificate-request-id').value = crypto.randomUUID();
  function renderIssueFields() {
    const ssh = q('#certificate-issue-kind').value.startsWith('ssh');
    q('#certificate-key-id').closest('label').classList.toggle('hidden', !ssh);
    q('#certificate-issue-payload-label').textContent = ssh ? l('待签 SSH 公钥', 'SSH public key to sign') : l('待签 TLS CSR（PEM）', 'TLS CSR to sign (PEM)');
  }
  function renderVerifyFields() {
    const tls = q('#certificate-verify-kind').value.startsWith('tls');
    q('#certificate-intermediates').closest('label').classList.toggle('hidden', !tls);
    q('#certificate-authority-input-label').textContent = tls ? l('受信任的 CA 证书（PEM）', 'Trusted CA certificate (PEM)') : l('受信任的 CA 公钥', 'Trusted CA public key');
  }
  function options(select, entries, empty) {
    const prior = select.value;
    select.replaceChildren(new Option(empty, ''), ...entries.map(([id, name]) => new Option(name, id)));
    if (entries.some(([id]) => id === prior)) select.value = prior;
  }
  function modules() {
    const item = getState().connections.find(item => item.id === q('#certificate-item').value);
    const binding = item?.certificateAuthority;
    options(q('#certificate-private-key'), (item?.modules || []).filter(m => m.kind === 'privateKey').map(m => [m.id, `${m.name || 'privateKey'} · ${m.id.slice(0, 8)}`]), l('选择私钥模块', 'Select private key module'));
    options(q('#certificate-ca-module'), (item?.modules || []).filter(m => m.kind === 'caCertificate').map(m => [m.id, `${m.name || 'caCertificate'} · ${m.id.slice(0, 8)}`]), l('仅 TLS 必填', 'Required for TLS only'));
    if (binding) {
      q('#certificate-ca-kind').value = binding.kind;
      q('#certificate-private-key').value = binding.privateKey;
      q('#certificate-ca-module').value = binding.caCertificate || '';
    }
  }
  function render() {
    const connections = getState().connections;
    const entries = connections.map(item => [item.id, item.name]);
    const issuers = connections.filter(item => item.certificateAuthority).map(item => [item.id, item.name]);
    options(q('#certificate-item'), entries, l('选择已保存项目', 'Select a saved project'));
    options(q('#certificate-issue-item'), issuers, l('选择已注册的 CA', 'Select a registered CA'));
    q('#certificate-issue-empty').classList.toggle('hidden', issuers.length > 0);
    q('#certificate-issue-fields').classList.toggle('hidden', issuers.length === 0);
    options(q('#certificate-match-item'), entries, l('选择已保存项目', 'Select a saved project'));
    modules();
    renderIssueFields();
    renderVerifyFields();
  }
  async function load() {
    if (!isUnlocked()) return;
    const requestEpoch = epoch;
    const next = await invoke('certificate_capabilities');
    if (requestEpoch !== epoch) return;
    capabilities = next;
    q('#certificate-enabled').checked = Boolean(capabilities.enabled);
    q('#certificate-status').textContent = l('Agent 签发', 'Agent signing') + (capabilities.enabled ? l('已开启', ' enabled') : l('已关闭', ' disabled'));
    q('#certificate-grants').innerHTML = (capabilities.grants || []).map(grant => {
      const item = getState().connections.find(item => item.id === grant.itemId);
      const effective = (capabilities.authorities || []).some(a => a.itemId === grant.itemId && a.effective);
      return `<div class="agent-client-row"><span class="agent-client-copy">${e(item?.name || grant.itemId)} · ${e(grant.kinds.join(', '))} · ${grant.maxTtlSeconds}s · ${effective ? l('生效', 'effective') : l('未生效', 'inactive')}</span><button type="button" class="small-button" data-remove-signing="${e(grant.itemId)}">${l('撤权', 'Revoke')}</button></div>`;
    }).join('');
  }
  async function run(operation) {
    if (busy || !isUnlocked()) return;
    const requestEpoch = epoch;
    busy = true;
    const controls = [...panel.querySelectorAll('button, input, select, textarea')];
    const focused = document.activeElement;
    controls.forEach(control => { control.disabled = true; });
    try { await operation(() => requestEpoch === epoch); }
    catch (error) { if (requestEpoch === epoch) toast(cleanError(error), 'error'); }
    finally {
      controls.forEach(control => { control.disabled = false; });
      busy = false;
      if (requestEpoch === epoch && controls.includes(focused) && document.activeElement === document.body) focused.focus({ preventScroll: true });
    }
  }
  function clear() {
    epoch++;
    capabilities = { enabled: false, grants: [], authorities: [] };
    q('#certificate-enabled').checked = false;
    q('#certificate-status').replaceChildren();
    q('#certificate-grants').replaceChildren();
    for (const control of panel.querySelectorAll('#certificate-issue-panel textarea, #certificate-issue-names, #certificate-key-id, #certificate-verify-panel textarea, #certificate-verify-identity, #certificate-match-panel textarea')) control.value = '';
    q('#certificate-request-id').value = crypto.randomUUID();
  }
  function pemCertificates(text) {
    if (!text.trim()) return [];
    const pattern = /-----BEGIN CERTIFICATE-----[\s\S]*?-----END CERTIFICATE-----/g;
    const certificates = text.match(pattern);
    if (!certificates || text.replace(pattern, '').trim()) throw Error(l('请输入完整的 PEM 证书，不要附加其他内容', 'Enter complete PEM certificates without additional material'));
    return certificates;
  }
  const selected = selector => { const id = q(selector).value; if (!id) throw Error(l('请先选择已保存项目', 'Select a saved project')); return id; };
  panel.addEventListener('click', event => {
    const target = event.target.closest('button');
    if (!target) return;
    if (target.id === 'certificate-open-ca-management') {
      if (busy || !isUnlocked()) return;
      q('#certificate-ca-management').open = true;
      q('#certificate-item').focus();
      return;
    }
    void run(async current => {
      const id = target.id;
      const resultField = q(`#${id}-result`);
      const showResult = result => { if (current()) resultField.value = JSON.stringify(result, null, 2); };
      if (resultField) resultField.value = '';
      if (target.dataset.removeSigning) await invoke('certificate_permissions', { id: target.dataset.removeSigning });
      else if (id === 'certificate-register') {
        await invoke('set_certificate_authority', { id: selected('#certificate-item'), binding: { kind: q('#certificate-ca-kind').value, privateKey: selected('#certificate-private-key'), caCertificate: q('#certificate-ca-kind').value === 'tls' ? selected('#certificate-ca-module') : null } });
        await refresh();
      } else if (id === 'certificate-unregister') {
        await invoke('set_certificate_authority', { id: selected('#certificate-item'), binding: null });
        await refresh();
      } else if (id === 'certificate-allow') await invoke('certificate_permissions', { id: selected('#certificate-item'), kinds: [q('#certificate-grant-kind').value], maxTtlSeconds: Number(q('#certificate-max-ttl').value) });
      else if (id === 'certificate-new-request') q('#certificate-request-id').value = crypto.randomUUID();
      else if (id === 'certificate-match') showResult(await invoke('certificate_key_check', { id: selected('#certificate-match-item'), publicOrCertificate: q('#certificate-match-payload').value }));
      else if (id === 'certificate-verify') {
        const kind = q('#certificate-verify-kind').value;
        showResult(await invoke('certificate_verify', { request: {
          kind,
          certificate: q('#certificate-verify-payload').value,
          authorities: [q('#certificate-authority-input').value],
          intermediates: kind.startsWith('tls') ? pemCertificates(q('#certificate-intermediates').value) : [],
          identity: q('#certificate-verify-identity').value.trim(),
        } }));
      } else if (id === 'certificate-issue') {
        const kind = q('#certificate-issue-kind').value;
        const names = q('#certificate-issue-names').value.split(',').map(value => value.trim()).filter(Boolean);
        const payload = q('#certificate-issue-payload').value;
        const request = kind.startsWith('ssh') ? { kind, publicKey: payload, principals: names, keyId: q('#certificate-key-id').value, ...(kind === 'sshUser' ? { extensions: [] } : {}) } : { kind, csr: payload, names };
        showResult(await invoke('certificate_issue', { id: selected('#certificate-issue-item'), requestId: q('#certificate-request-id').value, request, ttlSeconds: Number(q('#certificate-ttl').value) }));
      }
      if (current()) await load();
    });
  });
  q('#certificate-item').addEventListener('change', modules);
  q('#certificate-issue-kind').addEventListener('change', renderIssueFields);
  q('#certificate-verify-kind').addEventListener('change', renderVerifyFields);
  q('#certificate-enabled').addEventListener('change', () => { const enabled = q('#certificate-enabled').checked; void run(async current => { try { await invoke('certificate_permissions', { enabled }); } finally { if (current()) await load(); } }); });
  panel.addEventListener('toggle', () => { if (!busy) void run(load); }, true);
  render();
  return { render, load, clear };
}
