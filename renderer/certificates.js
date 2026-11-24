function createCertificatePanel({ invoke, getState, localized: l, escapeHtml: e, toast, cleanError, refresh }) {
  const panel = document.querySelector('#certificate-panel');
  let capabilities = { enabled: false, grants: [], authorities: [] };
  let busy = false;
  const q = selector => panel.querySelector(selector);
  const label = (id, text, control) => `<label class="field"><span>${text}</span>${control.replace('id="x"', `id="${id}"`)}</label>`;
  const select = (id, text) => label(id, text, '<select class="input" id="x"></select>');
  const field = (id, text, attributes = '') => label(id, text, `<input class="input" id="x" ${attributes}>`);
  const area = (id, text, attributes = '') => label(id, text, `<textarea class="input textarea" id="x" rows="5" maxlength="1048576" autocomplete="off" spellcheck="false" ${attributes}></textarea>`);
  const button = (id, text) => `<button type="button" class="small-button" id="${id}">${text}</button>`;
  panel.innerHTML = `<div class="settings-heading"><div><h2>${l('证书与签发权限', 'Certificates and signing permissions')}</h2><p>${l('使用已有 CA。签发权限独立于保险库写入权限，默认关闭。', 'Use an existing CA. Signing permissions are separate from vault writes and disabled by default.')}</p></div></div>
    <div class="desktop-option"><span>${l('允许 Agent 签发证书', 'Allow Agent certificate issuance')}</span><label class="switch"><input type="checkbox" id="certificate-enabled" aria-label="${l('允许 Agent 签发证书', 'Allow Agent certificate issuance')}"><span></span></label></div>
    <p id="certificate-status" role="status"></p><div id="certificate-grants" class="agent-client-list"></div>
    <details><summary>${l('管理已有 CA 和授权', 'Manage existing CAs and grants')}</summary>
      <p>${l('以下操作立即生效。注册 CA 后，Agent 只能通过签发工具使用私钥，不能通过通用凭据工具使用。撤权不会吊销已签发证书。', 'Changes apply immediately. Registered CA secrets are restricted to issuance tools. Revoking a grant does not revoke issued certificates.')}</p>
      ${select('certificate-item', l('项目', 'Project'))}<div class="field-grid two">${select('certificate-ca-kind', l('CA 类型', 'CA kind'))}${select('certificate-private-key', l('私钥模块', 'Private key module'))}${select('certificate-ca-module', l('TLS CA 证书模块', 'TLS CA certificate module'))}</div>
      <div class="button-row">${button('certificate-register', l('注册 CA', 'Register CA'))}${button('certificate-unregister', l('解除 CA 用途限制', 'Unregister CA'))}</div>
      <div class="field-grid two">${select('certificate-grant-kind', l('允许签发的用途', 'Allowed certificate purpose'))}${field('certificate-max-ttl', l('最长有效秒数', 'Maximum lifetime in seconds'), 'type="number" min="1" max="31622400" value="3600"')}</div>${button('certificate-allow', l('替换为此用途授权', 'Replace grant with this purpose'))}
    </details>
    <details><summary>${l('校验和签发', 'Check and issue')}</summary>
      <p>${l('这里只提交公钥、CSR 和证书。私钥从所选项目读取，不会显示。签发结果不会自动部署。', 'Supply public keys, CSRs and certificates only. Private keys stay in the selected project. Results are not deployed automatically.')}</p>
      ${select('certificate-operation-item', l('私钥或 CA 项目', 'Private key or CA project'))}${select('certificate-kind', l('证书用途', 'Certificate purpose'))}
      ${area('certificate-payload', l('待签公钥 / CSR，或待验证证书', 'Public key / CSR to sign, or certificate to verify'))}
      ${field('certificate-names', l('身份名称，逗号分隔', 'Identities, comma separated'))}
      <div class="field-grid two">${field('certificate-key-id', 'SSH key ID')}${field('certificate-ttl', l('有效秒数', 'Lifetime in seconds'), 'type="number" min="1" max="31622400" value="3600"')}</div>
      ${field('certificate-request-id', l('签发请求 UUID，重试保留同一值', 'Issuance request UUID, preserve when retrying'))}
      ${area('certificate-authority-input', l('验证所信任的 CA 公钥 / CA 证书', 'Trusted CA public key / CA certificate'))}
      ${area('certificate-intermediates', l('TLS 中间证书链，可选', 'TLS intermediate chain, optional'))}
      <div class="button-row">${button('certificate-match', l('检查密钥匹配', 'Check key match'))}${button('certificate-verify', l('验证证书', 'Verify certificate'))}${button('certificate-issue', l('签发证书', 'Issue certificate'))}${button('certificate-new-request', l('新请求 ID', 'New request ID'))}</div>
      ${area('certificate-result', l('结果', 'Result'), 'readonly')}
    </details>`;
  const kinds = [['sshUser', l('SSH 用户', 'SSH user')], ['sshHost', l('SSH 主机', 'SSH host')], ['tlsServer', l('TLS 服务端', 'TLS server')], ['tlsClient', l('TLS 客户端', 'TLS client')]];
  for (const selector of ['#certificate-kind', '#certificate-grant-kind']) q(selector).replaceChildren(...kinds.map(([value, text]) => new Option(text, value)));
  q('#certificate-ca-kind').replaceChildren(new Option('SSH', 'ssh'), new Option('TLS', 'tls'));
  q('#certificate-request-id').value = crypto.randomUUID();
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
    const entries = getState().connections.map(item => [item.id, item.name]);
    options(q('#certificate-item'), entries, l('选择已保存项目', 'Select a saved project'));
    options(q('#certificate-operation-item'), entries, l('选择已保存项目', 'Select a saved project'));
    modules();
  }
  async function load() {
    capabilities = await invoke('certificate_capabilities');
    q('#certificate-enabled').checked = Boolean(capabilities.enabled);
    q('#certificate-status').textContent = l('Agent 签发', 'Agent signing') + (capabilities.enabled ? l('已开启', ' enabled') : l('已关闭', ' disabled'));
    q('#certificate-grants').innerHTML = (capabilities.grants || []).map(grant => {
      const item = getState().connections.find(item => item.id === grant.itemId);
      const effective = (capabilities.authorities || []).some(a => a.itemId === grant.itemId && a.effective);
      return `<div class="agent-client-row"><span class="agent-client-copy">${e(item?.name || grant.itemId)} · ${e(grant.kinds.join(', '))} · ${grant.maxTtlSeconds}s · ${effective ? l('生效', 'effective') : l('未生效', 'inactive')}</span><button type="button" class="small-button" data-remove-signing="${e(grant.itemId)}">${l('撤权', 'Revoke')}</button></div>`;
    }).join('');
  }
  async function run(operation) {
    if (busy) return;
    busy = true;
    const controls = [...panel.querySelectorAll('button, input, select, textarea')];
    controls.forEach(control => { control.disabled = true; });
    try { await operation(); } catch (error) { toast(cleanError(error), 'error'); }
    finally { controls.forEach(control => { control.disabled = false; }); busy = false; }
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
    void run(async () => {
      const id = target.id;
      if (['certificate-match', 'certificate-verify', 'certificate-issue'].includes(id)) q('#certificate-result').value = '';
      if (target.dataset.removeSigning) await invoke('certificate_permissions', { id: target.dataset.removeSigning });
      else if (id === 'certificate-register') {
        await invoke('set_certificate_authority', { id: selected('#certificate-item'), binding: { kind: q('#certificate-ca-kind').value, privateKey: selected('#certificate-private-key'), caCertificate: q('#certificate-ca-kind').value === 'tls' ? selected('#certificate-ca-module') : null } });
        await refresh();
      } else if (id === 'certificate-unregister') {
        await invoke('set_certificate_authority', { id: selected('#certificate-item'), binding: null });
        await refresh();
      } else if (id === 'certificate-allow') await invoke('certificate_permissions', { id: selected('#certificate-item'), kinds: [q('#certificate-grant-kind').value], maxTtlSeconds: Number(q('#certificate-max-ttl').value) });
      else if (id === 'certificate-new-request') q('#certificate-request-id').value = crypto.randomUUID();
      else if (id === 'certificate-match') q('#certificate-result').value = JSON.stringify(await invoke('certificate_key_check', { id: selected('#certificate-operation-item'), publicOrCertificate: q('#certificate-payload').value }), null, 2);
      else if (id === 'certificate-verify') q('#certificate-result').value = JSON.stringify(await invoke('certificate_verify', { request: { certificate: q('#certificate-payload').value, authorities: [q('#certificate-authority-input').value], intermediates: pemCertificates(q('#certificate-intermediates').value), identity: q('#certificate-names').value.trim(), kind: q('#certificate-kind').value } }), null, 2);
      else if (id === 'certificate-issue') {
        const kind = q('#certificate-kind').value;
        const names = q('#certificate-names').value.split(',').map(value => value.trim()).filter(Boolean);
        const request = kind.startsWith('ssh') ? { kind, publicKey: q('#certificate-payload').value, principals: names, keyId: q('#certificate-key-id').value, ...(kind === 'sshUser' ? { extensions: [] } : {}) } : { kind, csr: q('#certificate-payload').value, names };
        q('#certificate-result').value = JSON.stringify(await invoke('certificate_issue', { id: selected('#certificate-operation-item'), requestId: q('#certificate-request-id').value, request, ttlSeconds: Number(q('#certificate-ttl').value) }), null, 2);
      }
      await load();
    });
  });
  q('#certificate-item').addEventListener('change', modules);
  q('#certificate-enabled').addEventListener('change', () => { const enabled = q('#certificate-enabled').checked; void run(async () => { try { await invoke('certificate_permissions', { enabled }); } finally { await load(); } }); });
  panel.addEventListener('toggle', () => { if (!busy) void run(load); }, true);
  render();
  return { render, load };
}
