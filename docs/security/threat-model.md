# Vorn threat model

This document describes what Vorn protects, who it protects it from, and where each trust boundary
is enforced in the code. It ends with a ranked list of findings to fix before 0.8.0.

It was written from the code at `9f9b12cb`. Paths are relative to `packages/core/crates/` unless
they start with `packages/`, `apps/`, `.github/` or `vendor/`. Line numbers are for that commit and
will drift; the function names cited next to them will not. The VT parser is the vendored library
pinned in `vendor/libghostty-vt-sys/build.rs:7`. Its image loader was read at that pin
(`src/terminal/kitty/graphics_image.zig`).

## Assets

| Asset                                                                                                        | Where it lives                                                                                                                                                                                    |
| ------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Code**: the user's repositories, plus anything Vorn runs on their behalf                                   | Project folders, worktrees, `.vorn/workflows/*` in repos                                                                                                                                          |
| **Keys and tokens**: the desktop local token, device tokens, SSH keys, agent hook tokens                     | `~/.vorn/local-token` (`vornd/src/serve/files.rs:70-82`), device-token hashes (`reach/src/token.rs:62-102`), the vault (`vault/src/lib.rs`), agent settings files (`hooks/src/settings.rs:30-60`) |
| **Sessions and terminal contents**: scrollback, keystrokes, agent transcripts                                | sessiond (`sessiond/`), vornd's session registry, clients over `/ws`                                                                                                                              |
| **The user's files**: everything their account can read or write                                             | Reached through `files.*`, `file:*` RPCs, kitty images, Browser `file:` pages, pack inspect                                                                                                       |
| **Workflow and connector credentials**: API keys and OAuth tokens for connectors, secrets that workflows use | The vault (keychain, or `credentials.json` as a fallback), connector child env (`connectors/src/mcp.rs:25-69`)                                                                                    |
| **Execution**: the ability to run a command as the user                                                      | PTYs, `script:execute`, `process.run`, SSH, agents, updates                                                                                                                                       |

## Actors

| Actor                                                                                                                  | Capability assumed                                                                                                                                                                     | Trusted?                                                                          |
| ---------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| **Same-user local code**: the CLI, agents, programs started by the user                                                | Can read `~/.vorn/local-token` (0600, same UID) and so has full vornd authority                                                                                                        | Trusted. The OS account is the boundary; Vorn does not defend against it.         |
| **Malicious extension**: installed from git or a folder, program or Wasm tier                                          | Runs code inside the host's gate. It holds only the permissions it declares in its manifest, and the user accepted those at install time.                                              | Untrusted beyond its declared permissions                                         |
| **Malicious repo or folder** opened in Vorn: cloned, added as a project, or inspected as a pack                        | Controls file contents, names and symlinks, `.vorn/workflows/*`, `.git/config`, and pack manifests                                                                                     | Untrusted                                                                         |
| **Program running inside a terminal emitting escape sequences**: `cat` of a hostile file, an SSH host, a container log | Controls every byte the PTY emits. It gets replies only through PTY write-back.                                                                                                        | Untrusted. Note that it may be remote: SSH output reaches the same parser.        |
| **Network peer** on the LAN when Network Access is on, or a web page in the user's browser                             | Can reach TCP 50091, or loopback through DNS rebinding                                                                                                                                 | Untrusted                                                                         |
| **Paired device**: a phone, or a browser that ran pairing                                                              | Holds a device token with full `/ws` authority                                                                                                                                         | Trusted as the owner. It is a risk only if the device is lost or the token leaks. |
| **Shared workspace member**                                                                                            | There is no such principal in code. A workspace is only a field on records (`store/src/config.rs:44-60`, `store/src/dispatch.rs:292-305`), so every token holder sees every workspace. | Not modelled. Anyone who is given access is the owner.                            |
| **Update or release channel**: GitHub Releases, the feed, the install scripts                                          | Delivers code that runs as the user                                                                                                                                                    | Trusted only after verification (see [Updates](#12-updates-and-release-signing))  |

## Trust boundaries

Each subsection gives what crosses the boundary, where it is enforced, and any gaps. The gaps are
collected in [Findings](#findings).

### 1. vornd network and local surfaces

**What crosses:** JSON-RPC over WebSocket, HTTP requests, pairing codes, bearer tokens, and terminal
bytes to and from clients.

| Surface                 | Enforced at                                                                                       | Notes                                                                                                                                                                                                                                |
| ----------------------- | ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| TCP listener on 50091   | `vornd/src/serve/port.rs:13-51`                                                                   | Binds `0.0.0.0` only when Network Access is on (`:32`). Otherwise it binds `127.0.0.1` (`:34`).                                                                                                                                      |
| Non-loopback routing    | `vornd/src/endpoint.rs:505-507`, `:571-578` (`public_route`)                                      | Remote peers reach only `/ws`, `/grid`, HTTP answers, pairing, `/artifact/` and `/gate-view/`.                                                                                                                                       |
| TLS                     | `vornd/src/serve/tls.rs:29-67`, `endpoint.rs:458-483` (`must_use_tls`)                            | Self-signed certificate with the fingerprint pinned at pairing. The key is written 0600.                                                                                                                                             |
| WebSocket origin        | `endpoint.rs:794+`, `reach/src/origin.rs:68-94`                                                   | A missing `Origin` is allowed (native clients); a foreign one is refused.                                                                                                                                                            |
| Credential to principal | `vornd/src/native/config.rs:40-48` (`Viewer::of_credential`)                                      | Desktop token, device token, or Local. Only `stream_events` checks the kind (`native/mod.rs:1202-1213`). Every other method is open to every viewer, including `file:readContent` and `file:writeContent` (`native/mod.rs:921-955`). |
| Local token             | `vornd/src/serve/files.rs:70-82` (0600, `create_new`), dir 0700 at `:101-108`                     | No ACL is set on Windows. Read by the app (`vornd/src/native/reach.rs:143-148`) and the CLI (`cli/src/rpc.rs:25`).                                                                                                                   |
| `/vornd/sessions`       | `endpoint.rs:527-534`, `:769-786`                                                                 | Loopback only, but no token and no `Host` check, so it is reachable through DNS rebinding.                                                                                                                                           |
| Pairing                 | `reach/src/pairing.rs:15-25`, `:215-260` (`redeem`), `:301+` (`approve`)                          | 8-character base-32 code (40 bits), 5-minute TTL, 10 tries per code, and the operator must approve. There is no per-source rate limit.                                                                                               |
| Device tokens           | `reach/src/token.rs:62-102`, minted at `vornd/src/native/reach.rs:403-429`, revoked at `:345-366` | Stored as SHA-256 hashes and compared in constant time. They never expire and carry no scope.                                                                                                                                        |
| Browser client token    | `vorn-web/src/lib.rs:86-107`                                                                      | Taken from `?token=` or `#token=` and then stripped from the address. The query form can still reach logs and history first.                                                                                                         |
| Work routes             | `vornd/src/native/work/routes.rs:1-200`                                                           | Artifacts and gate views use a `?t=` token in the query. Webhooks are local only, but a loopback relay's `PEER_HEADER` is trusted.                                                                                                   |
| MCP                     | `vornd/src/mcp.rs:71-105`                                                                         | POST `/mcp` needs the local token, refuses any `Origin` and has a 16 MiB limit. Its tools include `write_to_terminal` and keys (`mcp/src/tools/sessions.rs:245-348`) and `execute_workflow` (`mcp/src/tools/mod.rs:204-215`).        |

### 2. sessiond

**What crosses:** PTY bytes, resize and spawn requests, and session lifecycle between vornd and
sessiond.

- Unix: the socket directory is 0700, the socket 0600, and the peer UID is checked
  (`sessiond/src/os.rs:25-55`).
- Windows: a named pipe (`sessiond/src/os.rs:101-194`).
- `vornd:spawn` is only a client RPC and is gated by `allow_spawn` (`vornd/src/terminal.rs:59-76`).

Same-user code can connect, which is in line with the actor model above.

### 3. Terminal parsing (program output to Vorn)

**What crosses:** arbitrary bytes from local or remote programs. Out of them come screen state,
replies written back to the PTY, clipboard writes, link targets, titles, the cwd, notifications,
program status, and image loads.

| Effect                                                | Enforced at                                                                                                               | Behaviour                                                                                                                                                                                                                                                                                                                            |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Replies to the program                                | `screen/src/emulator.rs:209-212` (`on_pty_write` to `Effect::Reply`)                                                      | Every query reply goes back to the PTY, and so to a remote host for SSH output.                                                                                                                                                                                                                                                      |
| Kitty images                                          | `screen/src/images.rs:95-119` (`set_images`), turned on for every terminal at `engine/src/term.rs:83-92`, `:211`          | `set_images(true)` enables the **file** medium (`t=f`), the **temp-file** medium (`t=t`, directory `std::env::temp_dir()`) and **shared memory** (`t=s`). There is no user switch and no local-only restriction. Storage is capped at 10 MB.                                                                                         |
| Kitty `t=f` (in the pinned VT library)                | `graphics_image.zig`                                                                                                      | Opens any path relative to the cwd. It blocks only `/proc`, `/sys` and `/dev` (except `/dev/shm`), requires a regular file and reads up to 400 MB. Pixels from local files are drawn and forwarded to every client, and the error reply tells apart "cannot open" and "opened but not an image", or a short read when `S=` is given. |
| Kitty `t=t`                                           | same                                                                                                                      | Deletes the file afterwards if it is under `/tmp`, `/dev/shm` or `TMPDIR` and its name contains `tty-graphics-protocol`.                                                                                                                                                                                                             |
| Kitty `t=s`                                           | same                                                                                                                      | `shm_open`s any name and then `shm_unlink`s it, so remote output can destroy the user's POSIX shared-memory segments.                                                                                                                                                                                                                |
| OSC 52 clipboard                                      | `emulator.rs:217-239`                                                                                                     | Write only, with no confirmation and no size limit beyond the 1 MiB sequence cap. There is no read.                                                                                                                                                                                                                                  |
| OSC 8 hyperlinks                                      | `vorn-app/src/app/tasks.rs:1323-1342`, `vorn-app/src/act.rs:210-216`                                                      | Opening a link needs a click and is limited to `https:`, `http:` and `mailto:`.                                                                                                                                                                                                                                                      |
| Title, cwd (OSC 7/5522), OSC 133, OSC 9/777, OSC 7501 | `emulator.rs:470-478`, `:480-482`, `:542-590`, `screen/src/lib.rs:141-148`, `emulator.rs:461-468`, `:240-245`, `:637-715` | Parsed into state; OSC 7501 is checked against the protocol's limits.                                                                                                                                                                                                                                                                |
| Unfinished sequences                                  | `emulator.rs:18-20`, `:201-203`                                                                                           | Capped at 1 MiB.                                                                                                                                                                                                                                                                                                                     |

### 4. Extension host

**What crosses:** JSON-RPC calls from extension code into vornd's world, events out to extensions,
and the extension's manifest, id and files into the user's data directory.

**Who is calling.** `extensions/src/gate.rs:22-32` (`Caller`), `:37-53` (`Caller::of`,
`Caller::builtin`). A program authenticates with its per-launch token
(`extensions/src/host.rs:205-237`, checked in `by_token` at `:280-286`). Wasm calls are routed
through `router::answer` (`extensions/src/wasm.rs:552-579`).

**Permissions.**

- `gate.rs:232-292` (`CORE_METHODS`) maps each method to a permission.
- `:296-299` (`enforced`), `:302-307` (`WITHHELD` headers), `:310-335` (`admit_call`).
- The comment on `CORE_METHODS` says shipped extensions "call nothing an installed one could not".
  That means every method once reserved for built-ins is now open to any installed extension that
  declares the permission. Their implementations are in `vornd/src/native/extensions/world.rs`:

| Method                                                              | world.rs                                                   | Reach                                                                                                                                                   |
| ------------------------------------------------------------------- | ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `files.read`, `files.list`, `files.stat`, `files.write`             | 525-566                                                    | Any absolute path, with no project scope. Writing to a shell rc file or a git hook runs code without `process.run`.                                     |
| `terminal.read`, `terminal.send`, `terminal.selection`              | 439-447 to `vornd/src/native/extensions/routes.rs:207-237` | Any live session, not just the extension's own. Sending keys to a shell runs commands.                                                                  |
| `sessions.launch`                                                   | 434-438 (`:258`)                                           | Starts agents or terminals in a project.                                                                                                                |
| `permissions.resolve`                                               | 491-502                                                    | Approves agents' permission prompts.                                                                                                                    |
| `web.call`, `video.call`                                            | 750-790                                                    | Forwards any `web:{op}` or `video:{op}` to the app (`vorn-app/src/app/web.rs:418-516`): page script, reads, network log, and app install on simulators. |
| `remote.saveHosts` and `remote.test`                                | 799-808, `remote_test` at 317-354                          | Stores a host and runs `ssh` with it. See [Remote hosts](#9-remote-and-ssh-hosts).                                                                      |
| `remote.importKey`                                                  | 811-817, `read_key` at 356-374                             | Reads any path into the vault, and the error `"{path}: {e}"` reveals whether the path exists.                                                           |
| `remote.keys`                                                       | 809 to `vornd/src/native/credential.rs:84-86`              | Lists the SSH keys stored in the vault.                                                                                                                 |
| `secrets.get`                                                       | 568, `secret_of` at 121-144                                | Scoped to the owning extension. This is correct.                                                                                                        |
| `workflow.run`, `workflow.seed`, `workflow.unseed`, `workflow.fire` | 639-650, 663-684, 686-711, 593-600                         | Gated by `admit_run` (`gate.rs:95-109`) and `admit_seed` (`gate.rs:111-198`).                                                                           |

**Ownership and reserved ids.**

- `gate.rs:57-66` (`owns`): a built-in owns its namespace, and an installed extension owns any id
  that is not reserved.
- `gate.rs:79-93` (`admit_own`), `:205-216` (`admit_storage`).
- `extensions/src/pack.rs:167-175` (`is_safe_id`).
- `RESERVED_IDS` is `import`, `connector` and `system` (`pack.rs:179`), and `SHIPPED_NAMESPACE` is
  `vorn.` (`:183`).
- `is_reserved` and `check_id` (`pack.rs:186-208`) compare case-sensitively, while `is_safe_id`
  allows uppercase, so `Vorn.x` passes.
- The comment on `is_reserved` refers to a pinned checksum for the shipped set, but no checksum
  pinning exists. Shipped extensions are recognised by id and location only.

**Install, inspect, link and relabel.** These steps cross from untrusted repo files into the
user's data directory.

- `extensions/src/install.rs:134-152` (`usable_git_url`): https, ssh, `git@`, `file://` or an
  absolute path. A leading `-`, `::` and control characters are refused.
- `install.rs:254-302` (`inspect_git_at`): a plain folder with no `.git` goes through
  `copy_folder` (`:852-866`) and `walk` (`:868-886`). **Both follow symlinks**: `fs::copy` reads the
  target, and a link to a directory is listed as a file. A folder with
  `id_rsa -> ~/.ssh/id_rsa` copies the key into staging.
- `install.rs:314-335` (`clone_into`) keeps git from running code from the repo: hooks are pointed
  at a nonexistent directory, `protocol.ext.allow=never`, `--depth 1`, no submodules, and
  `GIT_LFS_SKIP_SMUDGE`.
- `install.rs:680-701` (`keep_only`) refuses symlinks. `:704-711` (`linkable`) canonicalizes the
  folder. `:713-732` (`read_staged_manifest`, `installable_id`) runs `relabel_dir` and then
  `check_id`.
- `install.rs:786-850` (`pack`, `bundle`): tar `append_path_with_name` follows symlinks, so files
  outside the folder are published inside the tarball. `pack` also runs the manifest's
  `checks.command` (`:802-818`).
- `install.rs:738-744` (`write_current`): writes a fixed `.tmp` name with `fs::write`, which follows
  a planted symlink.
- `pack.rs:250-283` (`relabel_dir`): refuses a symlinked manifest and writes with `create_new`
  (test at `:551-575`). A small window remains between `symlink_metadata` and `rename`.
  `relabel_installed` (`:288-310`) runs at every vornd start.
- The build step of a program extension runs after the user confirms the id (`install.rs` tests
  around `:1018-1028`).

**Events out.** `extensions/src/events.rs:38-73` (`CORE_EVENTS`): there are no keystroke or
terminal-output events, but subscribers receive the whole payload.

**Network and sign-in grants.** `extensions/src/netproxy.rs:133-171` allows only hosts the manifest
declares (`gate.rs:337-360`). Sign-in is handled at `extensions/src/signin.rs:75-161`.

### 5. Wasm sandbox

**What crosses:** imported host calls (through `router::answer`, so the same gate as programs) and
memory and CPU.

- `extensions/src/wasm.rs:845-855`: no preopened directories, no environment and no sockets.
- `wasm.rs:57-61`, `:506-539`: memory is capped at 64 MiB and tables at 100k entries.
- `wasm.rs:72-76`, `:88-121`, `:1019-1032`: epoch interruption only, with no fuel metering.
- `extensions/src/lifecycle.rs:71-105`: idle components stop after a minute, with at most 24
  resident.

This is the strongest tier: a Wasm extension can do only what the gate admits.

### 6. Program tier

**What crosses:** a native process that runs as the user, with the gate as its only API.

- `extensions/src/child.rs:247-265`: the environment is cleared and stdio is piped.
- `extensions/src/sandbox.rs:1-55`: the OS sandbox is **opt-in and macOS only**. It is off unless
  `<root>/<id>/sandbox.json` turns it on (used at `extensions/src/router.rs:324`, `:345`, `:571`).

By default a program extension can therefore read and write any file and open any socket, whatever
permissions it declared. For programs the permission list is advisory.

### 7. Connectors and credentials

**What crosses:** secrets from the vault into connector processes and outgoing HTTP, and tool
results back.

- `vault/src/lib.rs:172-185`: the keychain is used first. When it is missing, secrets fall back to
  plaintext `credentials.json` (`vault/src/file.rs:80-96`), which is 0600 on Unix and has no mode
  or ACL elsewhere.
- `connectors/src/mcp.rs:25-69`, `:145-190`: connector MCP children get their secrets in the
  environment, and their stderr is inherited, so secrets that a child prints end up in vornd's log.
- `connectors/src/http.rs:1-5`: the HTTP connector does not follow redirects, so credentials are
  not forwarded to another host.

### 8. Workflows

**What crosses:** workflow definitions from the app, from extensions, and **from repository files**
into the store. Commands and agents come out of them when they run.

- Steps that run commands: `script:execute` (`workflows/src/script.rs:24-40`). Triggers include
  cron, once, and webhooks (`work/src/trigger.rs`). The scheduler honours `enabled`
  (`vornd/src/native/work/mod.rs:1257`).
- Extension seeding: `admit_seed` (`gate.rs:111-198`) refuses a seed whose top-level
  `nodes[].type` runs commands (`runs_commands`, `world.rs:378-389`) unless the caller declares
  `process.run`. Nested nodes are not inspected.
- Contributed workflows: `vornd/src/native/work/files.rs:553-600` (`seed_contributed`, called from
  `work/mod.rs:178-188`) inserts the manifest's `[[contributes.workflows]]` with **no
  `admit_seed` check**. `with_defaults` (`files.rs:603-620`) sets `enabled = true`.
- Repository files: `files.rs:27` (`FOLDER = ".vorn/workflows"`), `:214-223` (`watch_files` polls
  every second), `:225-273` (`look_at_files`), `:276-420` (`read_file`). Every project's
  `.vorn/workflows/*` is upserted into the store **with no trust prompt**, and `enabled` defaults to
  true. Only an id already taken by another workflow is refused, so a file can claim `import:…`,
  `vorn.x:…` or another extension's namespace. `read_file` also writes the normalised text back into
  the repo (`:381`).
- The result: a cloned repo that is added as a project and contains a workflow with
  `triggerType = "cron"`, `* * * * *` and a script step runs its command within a minute, with no
  consent.
- The portability checker (`packages/shared/src/workflow-portability.ts:339`) applies only to
  exports, not to imports.

### 9. Remote and SSH hosts

**What crosses:** host definitions (user, hostname and free-form `ssh_options`) into an `ssh`
command line, and remote output back into the terminal parser.

- `remote/src/lib.rs:31-62` (`args`): `ssh_options.split_whitespace()` is appended verbatim
  (`:57`), so `-oProxyCommand=…` runs a local command. `user@host` is pushed with no `--` (`:59`),
  so a user field starting with `-o` is parsed as an option.
- Host keys: `StrictHostKeyChecking=accept-new` (`:38`), which trusts on first use without asking.
- Multiplexing: the ControlPath is `$TMPDIR` or `/tmp` plus `vorn-ssh-%h-%p` (`:40-47`, `:65-76`).
  On Linux `/tmp` is shared, so another local user can pre-create the socket.
- Saving hosts: `vornd/src/native/config.rs:157-167` (`save_hosts`) stores the array with no
  validation.
- Remote file access: `file::remote::*` (`native/mod.rs:921-955`) goes over the same SSH
  connection.

### 10. Agents and adapters

**What crosses:** agent CLIs started in PTYs, their hook calls back into vornd, and permission
prompts.

- `agents/src/launch/env.rs:1-10`: the launch environment drops the bootstrap token.
  `agents/src/probe.rs:240-262` probes for installed agents.
- `hooks/src/settings.rs:30-60`, `hooks/src/file.rs:40-120`: hooks are written into each agent's
  own settings, with the vornd bearer token embedded. The token ends up in files other tools read
  and may sync.
- `vornd/src/native/hooks.rs:679-694` (`write_private`): uses `create` plus `mode`, so an existing
  file keeps its wider mode.
- Agent permission prompts can be resolved by any extension with `permissions.resolve` (§4) and by
  MCP clients.

### 11. Browser and device panes

**What crosses:** web content into a webview, page IPC back to the app, and app-driven script into
pages.

- `vorn-app/src/web.rs:1-52`, `vorn-app/src/web/views.rs:108-244`: wry webviews. Page IPC goes
  through the injected `web/page.js`.
- `vorn-app/src/app/web.rs:529-541` (`vet`): a `file:` URL loads only inside the session's folder.
  Other schemes pass through.
- `vorn-app/src/app/web.rs:543-575`: the app evaluates script in pages for `web:*` calls, and those
  can come from extensions through `web.call`.
- `vorn-app/src/video.rs:86-199`: simulators through `xcrun simctl`, including installing any
  `.app`.

### 12. Updates and release signing

**What crosses:** code from GitHub Releases into the user's machine.

- `vorn-app/src/update.rs:237-251` (`fetch_feed`): the feed comes from GitHub over HTTPS.
  `VORN_UPDATE_FEED` overrides it (`:41`).
- `update.rs:266-331`: the SHA-256 and size are checked against the feed, so the feed is the only
  root of trust for the hash.
- macOS: `update.rs:413-465` checks `codesign`, the same team id and bundle id, and Gatekeeper
  (`spctl`). If the running app has no team id (ad-hoc build), the team check is skipped (`:433`).
- Windows: `update.rs:468-495` checks Authenticode. **If the running exe is unsigned, any installer
  is accepted** (`:487-489`).
- Linux AppImage: there is no signature check.
- `install.sh` and `install.ps1` download the release with no checksum or signature check.
- CI: `.github/workflows/native-app.yml:164-194` signs the macOS app only when a certificate is
  configured ("signed ad hoc" otherwise). The feed is published last (`:387+`). `release.yml` has
  `contents: write`.
- Extensions: installs from git carry no publisher signature. The user confirms the id and the
  permissions only.

## Findings

Ranked by severity, then by how easily an untrusted actor reaches them. "Boundary" is the section
above. The S column is the follow-up stage that owns the fix:

- **S2**: terminal parsing
- **S3**: extension gate and install
- **S4**: workflows and repo trust
- **S5**: network, pairing and remote
- **S6**: credentials, updates and supply chain

| #   | Sev      | S     | Boundary                            | Where                                                                                         | Finding                                                                                                                                                                                                                                                      | Fix idea                                                                                                                                                                                                        |
| --- | -------- | ----- | ----------------------------------- | --------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Critical | S4    | §8 Workflows (repo to store)        | `vornd/src/native/work/files.rs:276-420`, `:603-620`                                          | A repo's `.vorn/workflows/*` is imported enabled. A cron trigger with a script step runs commands within a minute of adding the project, with no consent.                                                                                                    | Import repo workflows disabled and untrusted until the user approves that file's hash. Refuse command steps from untrusted files.                                                                               |
| 2   | Critical | S2    | §3 Terminal (output to local files) | `screen/src/images.rs:95-106`, `engine/src/term.rs:83-87`                                     | Kitty `t=f`/`t=t`/`t=s` are always on, including for SSH output. A remote host can render any local file the user can read onto the screen and to paired clients, probe existence and size through error replies, delete temp files and unlink shm segments. | Allow only direct (`t=d`) by default. Enable file, temp and shm only for local PTYs and only under an allowlisted temp directory. Make the error replies the same for every failure.                            |
| 3   | Critical | S5    | §9 Remote (host to local exec)      | `remote/src/lib.rs:56-59`, `vornd/src/native/config.rs:157-167`, `world.rs:799-808`           | `ssh_options` are appended verbatim and `user@host` comes with no `--`. A saved host with `-oProxyCommand=…` (from an extension with `remote.saveHosts`) runs a local command on `remote.test`, connect or a file op, bypassing `process.run`.               | Parse `ssh_options` against an allowlist of `-o` keys (no `ProxyCommand`, `LocalCommand` or `KnownHostsCommand`). Reject a user or host that starts with `-`. Pass `--` before the destination.                 |
| 4   | High     | S3    | §4 Gate (extension to world)        | `extensions/src/gate.rs:232-292`, `world.rs:439-447`, `routes.rs:207-237`                     | `terminal.send` and `terminal.read` reach every live session, so an extension without `process.run` can type into a shell and read its output.                                                                                                               | Scope terminal methods to sessions the extension started, or require `process.run` and a per-session grant.                                                                                                     |
| 5   | High     | S3    | §4 Gate                             | `world.rs:525-566`                                                                            | `files.*` take any absolute path. A write to `~/.zshrc` or `.git/hooks` runs code without `process.run`, and reads reach keys and tokens.                                                                                                                    | Scope to the caller's project roots and the extension's own data directory. Deny `~/.vorn`, `~/.ssh` and shell rc files. Treat writes outside a project as `process.run`.                                       |
| 6   | High     | S4    | §8 Workflows (extension to store)   | `vornd/src/native/work/files.rs:553-620`, `work/mod.rs:178-188`                               | `seed_contributed` skips `admit_seed`, so a manifest-contributed workflow with script steps is seeded enabled for an extension without `process.run`.                                                                                                        | Run `admit_seed` (with the manifest's caller) on every contributed workflow and seed them disabled.                                                                                                             |
| 7   | High     | S3    | §4 Gate                             | `world.rs:491-502`, `:434-438`, `:750-790`                                                    | `permissions.resolve` approves agents' tool prompts, `sessions.launch` starts agents, and `web.call`/`video.call` forward any op (page script, simulator installs). Each executes or escalates without `process.run`.                                        | Put `permissions.resolve` and `sessions.launch` behind `process.run`. Allowlist the `web:`/`video:` ops per permission and refuse script evaluation for installed extensions.                                   |
| 8   | High     | S3    | §4 Install (repo to data dir)       | `extensions/src/install.rs:852-886`, `:841-850`                                               | `copy_folder`, `walk` and `bundle` follow symlinks. Inspecting a folder copies outside files (such as SSH keys) into staging, and `pack` publishes them in the tarball.                                                                                      | Use `symlink_metadata` and refuse (or skip) symlinks in `walk`, `copy_folder` and `bundle`, as `keep_only` already does.                                                                                        |
| 9   | High     | S6    | §6 Program tier                     | `extensions/src/sandbox.rs:1-55`, `router.rs:324`                                             | The program-tier sandbox is opt-in and macOS only, so declared permissions do not limit a program extension's file or network access.                                                                                                                        | Turn the sandbox on by default on macOS (deny files outside the extension and project, network unless declared), add Linux (landlock or seccomp), and show "unsandboxed" at install where neither is available. |
| 10  | High     | S3    | §4 `remote.importKey`               | `world.rs:356-374`, `:811-817`                                                                | Reads any path into the vault and echoes `"{path}: {e}"`, which is both an arbitrary read and an existence oracle.                                                                                                                                           | Accept only paths the user picked in a file dialog, or paths under `~/.ssh`. Return one generic error.                                                                                                          |
| 11  | High     | S3/S4 | §4/§8 Reserved ids                  | `extensions/src/pack.rs:186-208`, `vornd/src/native/work/files.rs:276-420`                    | `check_id` is case-sensitive (`Vorn.x` passes), and repo workflow ids are never checked, so a repo file can claim `import:`, `vorn.*` or another extension's namespace.                                                                                      | Compare ids lowercased (or refuse uppercase). Run `check_id` and namespace ownership on repo workflow ids.                                                                                                      |
| 12  | Medium   | S3    | §4 Shipped-set pins                 | `extensions/src/pack.rs:186-189`                                                              | The shipped set is trusted by id and location, while the comment promises a pinned checksum that does not exist.                                                                                                                                             | Record a checksum per shipped extension at build time and verify it before granting built-in status.                                                                                                            |
| 13  | Medium   | S5    | §9 Remote                           | `remote/src/lib.rs:38`, `:40-47`, `:65-76`                                                    | `accept-new` trusts unknown host keys silently, and the ControlPath sits in a shared `/tmp` on Linux.                                                                                                                                                        | Ask before accepting a new host key. Put ControlPath under a 0700 per-user directory in Vorn's data dir.                                                                                                        |
| 14  | Medium   | S5    | §1 Loopback HTTP                    | `vornd/src/endpoint.rs:527-534`, `:769-786`                                                   | `/vornd/sessions` has no token and no `Host` check, so it is readable by a web page through DNS rebinding.                                                                                                                                                   | Require `Host` to be a loopback literal on every plain HTTP route, and require the local token.                                                                                                                 |
| 15  | Medium   | S5    | §1 Pairing and tokens               | `reach/src/pairing.rs:15-25`, `reach/src/token.rs:62-102`, `vornd/src/native/config.rs:40-48` | Device tokens never expire and carry full authority, including `file:*`. Pairing limits tries per code but not per source.                                                                                                                                   | Add token expiry or idle expiry and a last-seen list. Rate-limit pairing per peer address. Consider a read-mostly scope for devices.                                                                            |
| 16  | Medium   | S5    | §1 Tokens in URLs                   | `vorn-web/src/lib.rs:86-107`, `vornd/src/native/work/routes.rs`                               | Device and artifact tokens travel in query strings, where they reach history, logs and referrers.                                                                                                                                                            | Use only `#token=` for the web client. Swap `?t=` for a short-lived, single-use ticket.                                                                                                                         |
| 17  | Medium   | S6    | §12 Updates                         | `vorn-app/src/update.rs:487-489`, `:433`                                                      | An unsigned (or ad-hoc) running app accepts any installer, and Linux has no signature at all. The feed hash is the only check.                                                                                                                               | Sign the feed (an embedded public key), verify it before trusting asset hashes, and never skip the signer check.                                                                                                |
| 18  | Medium   | S6    | §12 Install scripts                 | `install.sh`, `install.ps1`                                                                   | The install scripts verify neither a checksum nor a signature.                                                                                                                                                                                               | Download the signed feed and check SHA-256 before installing.                                                                                                                                                   |
| 19  | Medium   | S6    | §7 Vault                            | `vault/src/lib.rs:172-185`, `vault/src/file.rs:80-96`                                         | Without a keychain, secrets are plaintext JSON. It is 0600 on Unix and has no ACL on Windows.                                                                                                                                                                | Use DPAPI or the Secret Service, otherwise refuse or warn loudly. Set an owner-only ACL on Windows (also for `local-token`, `serve/files.rs:70-108`).                                                           |
| 20  | Medium   | S2    | §3 OSC 52                           | `screen/src/emulator.rs:217-239`                                                              | Any output, including from remote hosts, can overwrite the clipboard silently, up to about 1 MiB.                                                                                                                                                            | Cap the size. Ask (or notify) on writes from programs that are not in the foreground. Offer a per-host setting.                                                                                                 |
| 21  | Medium   | S6    | §7 Connectors                       | `connectors/src/mcp.rs:25-69`, `:145-190`                                                     | Secrets are passed in env and stderr is inherited, so a child that prints them leaks them into vornd's log.                                                                                                                                                  | Capture stderr and redact known secret values before logging.                                                                                                                                                   |
| 22  | Medium   | S6    | §10 Agents                          | `hooks/src/settings.rs:30-60`, `vornd/src/native/hooks.rs:679-694`                            | The vornd bearer token is embedded in agents' settings files, and `write_private` keeps an existing file's mode.                                                                                                                                             | Issue a hook-only token scoped to hook routes. `set_permissions(0o600)` after open.                                                                                                                             |
| 23  | Low      | S4    | §8 `admit_seed`                     | `extensions/src/gate.rs:111-198`                                                              | Only top-level `nodes[].type` is inspected. Nested or sub-workflow nodes are not checked for running commands.                                                                                                                                               | Walk every node, sub-workflow references included.                                                                                                                                                              |
| 24  | Low      | S3    | §4 Install temp writes              | `extensions/src/install.rs:738-744`, `pack.rs:250-283`                                        | `write_current` writes a fixed `.tmp` with `fs::write` (it follows symlinks), and `relabel_dir` has a check-then-rename window.                                                                                                                              | Use `create_new` with a random name and `O_NOFOLLOW`, as `relabel_dir` does.                                                                                                                                    |
| 25  | Low      | S3    | §5 Wasm                             | `extensions/src/wasm.rs:72-121`                                                               | Epoch interruption only, with no fuel, so a tight loop holds a worker until the epoch ticks.                                                                                                                                                                 | Add fuel per call alongside epochs.                                                                                                                                                                             |
| 26  | Low      | S4    | §8 Repo write-back                  | `vornd/src/native/work/files.rs:381`                                                          | Importing a repo workflow rewrites the user's file in their working tree.                                                                                                                                                                                    | Never write back to an untrusted repo file; keep the normalised form in the store only.                                                                                                                         |
| 27  | Info     | n/a   | Actors                              | `store/src/config.rs:44-60`                                                                   | There are no workspace members or ACLs. Any token holder sees every workspace.                                                                                                                                                                               | Document it as single-owner. Add ACLs only together with shared workspaces.                                                                                                                                     |

### Not in scope here

- Same-user local code is trusted (see Actors). Hardening against it (for example, scoping the
  local token per client) is a separate design decision.
- The repo's own `.git/config` (`core.fsmonitor`, `core.sshCommand`) may affect git calls Vorn makes
  on opened repos (`vorn-git/src/exec.rs:8-75`, fallback to git in `vorn-git/src/fast.rs:48-50`,
  `:116-124`). This was not fully traced. S4 should confirm which git calls run on untrusted repos
  and pass `-c core.fsmonitor= -c core.hooksPath=…` the way `clone_into` does.
