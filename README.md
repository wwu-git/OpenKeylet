<p align="center">
  <img src=".github/assets/openkeylet-hero.svg" alt="OpenKeylet" width="100%" />
</p>

<h1 align="center">OpenKeylet</h1>

<p align="center"><strong>A local password vault and credential manager for AI agents.</strong></p>

<p align="center">
  <a href="../../releases/latest"><strong>Download</strong></a> ·
  <a href="README.zh-CN.md">简体中文</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="SECURITY.md">Security</a>
</p>

OpenKeylet stores passwords, passkeys, SSH keys, API credentials, and one-time code seeds in an encrypted vault on your device. You can use the vault from the desktop app or Chromium extension. An AI agent can also use the credentials through local `stdio` MCP. For example, it can connect to a server, call an API, or fill a login while hidden values stay out of normal MCP responses.

OpenKeylet is free and open source. It runs on Windows, macOS, and Linux without an OpenKeylet account.

## Interface gallery

![OpenKeylet vault showing example login, SSH, API, card, identity, and note items](.github/assets/readme-vault.png)

<table>
  <tr>
    <td width="50%" valign="top">
      <img src=".github/assets/readme-agent-setup.png" alt="Agent connection settings with example connection states" /><br />
      <strong>Agent setup.</strong> Connect or repair local MCP registrations from Settings.
    </td>
    <td width="50%" valign="top">
      <img src=".github/assets/readme-ssh-trust.png" alt="SSH editor displaying an unverified candidate host fingerprint before saving" /><br />
      <strong>SSH host identity.</strong> Read and verify a candidate fingerprint before saving the project.
    </td>
  </tr>
</table>

The screenshots show the current interface populated with synthetic data.

## Quick start

1. Download and open the [build for your platform](../../releases/latest).
2. Choose **Add** and save a login, SSH project, or API credential. Give the project a unique name.
3. Open **Settings → Agent connection**, select **Connect** for your client, and start a new agent session. Use **Repair** if the registered executable moved.
4. Click **Use** on the project row to copy a reference for your agent, or name the project directly in your request.

Supported setup targets include Antigravity, Claude Code, Codex, Cursor, DeepSeek Harness, OpenClaw, and OpenCode. Other clients can register a local server manually. The MCP process starts on demand.

| Task | Example request |
| --- | --- |
| Check a server | `Use "Staging Gateway" in OpenKeylet MCP and report disk usage.` |
| Read an authenticated API | `Use "Release API" in OpenKeylet MCP to list recent commits for owner/repo.` |
| Fill a login | `Open the demo console and use "Atlas Console" in OpenKeylet MCP when it asks for credentials.` |

## What you can do

### Manage passwords, passkeys, and 2FA

Organize logins, cards, identities, secure notes, and custom fields with folders, favorites, and search. Generate passwords, review encrypted password history, and restore items from Trash.

The Chromium extension shares the same vault through the local Browser Bridge. It can fill website logins, cards, and identities, and lets you review a captured login before saving. Website rules scope fills from the extension popup. For supported passkeys, you can register or sign in after confirming the request. The private key stays in the vault.

Import a TOTP setup QR code offline from an image or the clipboard. The extension can also scan a QR code on the page, preview it, and save it to a matching login after confirmation. Fill the current code from that login.

### Connect to servers and run commands

SSH supports passwords, private keys, and user certificates, with a direct connection or a single saved jump project. Each hop uses its own credentials. Agents can run commands with stdin, read the exit code and output, and transfer files and folders with SFTP. If the connection drops without a confirmed command result, OpenKeylet can report that the remote outcome is unknown.

In a new or edited SSH project, open **Local host identity** and read the candidate fingerprint before saving. Verify it against a trusted source, confirm it, then save the project and its local trust together. For an existing project whose route is unchanged, you can update its local trust immediately. [SSH jump hosts](docs/ssh-jump.md) · [Certificate workflows](docs/certificates.md)

### Call APIs and use secrets in local workflows

Use saved credentials in HTTP requests, local commands, and managed terminal sessions. References such as `{{openkeylet:module name}}` are resolved locally. For supported tasks, MCP tools can show the selected project, suggest the next callable request, and explain missing parameters and pagination.

Native workflows cover GitHub commits, OCI compartments and running instances, AWS EC2 running instances, Cloudflare zones and DNS, DigitalOcean Droplets, and Hetzner servers. General HTTP requests cover other APIs. Certificate signing and vault writes require separate owner permissions. [API provider support](docs/api-provider-integration.md) · [MCP vault write permissions](docs/mcp-vault-write-permissions.md)

### Migrate and back up your vault

| Source | What to use |
| --- | --- |
| Other vaults and browsers | Bitwarden JSON, Chrome or Edge CSV, KeePass 2.x XML, and 1Password 1PUX or CSV |
| Developer configuration | `.env`, OpenSSH configuration, cURL commands, and AWS CLI credential profiles |
| Another OpenKeylet device | Password-protected `.mvault` export, or an encrypted snapshot in your own private GitHub repository |

Import previews let you check the contents before writing to the vault. When importing a `.env` file, OpenKeylet can generate a template with project-bound references for the variables saved in that batch. The template contains placeholders in place of the original values.

GitHub backup uses manual encrypted snapshots and a separate recovery key. You can preview a snapshot and confirm a merge restore. If the snapshot changes after preview, refresh the preview before restoring.

## Credential controls

| Control | What it does |
| --- | --- |
| Agent visibility | Hidden modules can be used locally by an enabled project. Making a module visible lets MCP return its value. |
| Item enable switch | Disable a project to stop agents from using it. |
| Local PIN | Locks desktop and extension vault access. Enabled MCP actions can continue while the GUI is locked. |
| SSH host identity | Existing trust records are checked. Jump routes require trusted endpoints. The setting that allows a direct connection without a trust record is off by default. |

The vault encrypts secret fields with XChaCha20-Poly1305. OpenKeylet runs with the permissions of your OS user. An enabled project can perform the actions you give it, including commands on a remote server. Review the [security model](SECURITY.md) before using it with sensitive systems.

## Downloads

| Platform | Package |
| --- | --- |
| Windows x64 | Portable ZIP |
| macOS arm64 | ZIP containing the macOS app |
| Linux x64 desktop | Tar archive containing an AppImage. Desktop input supports X11. |
| Linux x64 headless | Tar archive containing a CLI and MCP executable without a WebView dependency. |

<details>
<summary>Manual MCP configuration, browser setup, and upgrading</summary>

Run one of these commands to print your local MCP configuration:

```text
openkeylet config stdio-json
openkeylet config stdio-toml
```

Configure your client to run `openkeylet mcp stdio` with the absolute path to the installed executable. For Linux without a desktop, see the [headless CLI guide](docs/headless-admin-cli.md).

To use the browser extension, start **Browser Bridge** in Settings and follow the prompts to install and pair the bundled extension. Keep the desktop app and bridge running while using it.

OpenKeylet was previously called KRU. It continues to use the existing local vault and supports legacy `{{kru:module}}` references. After updating from KRU, use **Agent connection → Repair** and start a new agent session.

</details>

## Build from source

Install Rust, Node.js 22+, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). Run `npm ci` to install JavaScript dependencies, `npm test` to run the Rust suite, and `npm run dev` to start the desktop app. See the [release workflow](.github/workflows/release.yml) for platform builds and packaging.

[MIT License](LICENSE)
