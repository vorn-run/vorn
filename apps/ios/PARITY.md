# Parity with the previous phone app

Every screen and feature of the React Native phone app, and where it stands in this app. Screens come from `crates/phone` through vornui's Native backend; Swift maps the tree (see README).

## Pairing and computers

- [x] Scan the QR code the desktop shows (`scan`)
- [x] Pair by hand: host and code (`pair`, `manual`), host pre-filled from `VORN_HOST`
- [x] Launch screen while the link comes up
- [x] Several computers: add and switch in Settings
- [x] Forget this computer, which returns to pairing when it was the last

## Tabs

- [x] Sessions, Tasks, Workflows; a sidebar and detail on regular width (iPad, split view)
- [x] The Workflows tab counts the gates waiting on you

## Sessions

- [x] List with each agent's own mark and status, last output and age
- [x] Scope sheet: all, a project or a branch, with projects' coloured marks
- [x] View options: group (status, waiting first; project; worktree; none), order (recent, created), status filter, fields shown; kept across launches
- [x] Launcher: project, agent (or shell), main checkout or new worktree, prompt, a tip
- [x] Session: live drawn terminal with one-finger scroll, momentum and rubber-banding; wheel events to programs asking for mouse reporting
- [x] Composer and the keyboard key bar (Esc, Tab, Ctrl, arrows, and the rest)
- [x] Terminal text size, kept across launches
- [x] Permission requests: allow or deny from the session and its row
- [x] Rename and end a session

## Tasks

- [x] Board by status with counts; scope by project; view options (group by project, order newest, oldest or manual, archived, source, fields)
- [x] Swipe a row to move it along, archive or delete
- [x] Task: Markdown description, status, project with its mark, agent, source, dates; move, archive, delete from its menu
- [x] New task, starting from the desktop's description template
- [x] Edit title and description: a Markdown key bar (heading, bold, italic, code, list, task, link, quote), Write/Preview, hide keyboard

## Workflows

- [x] Workflows with their coloured icons, runs, waiting gates; schedule switch
- [x] Start a run, with its inputs form
- [x] Run: steps with status, each step's log (copy), stop a running run
- [x] Gates: approve, reject, ask for changes with a comment, edit the text before approving

## Shared

- [x] Every icon is vornui's or the agent's own mark, shipped at build time; an extension's marks come as SVG (test `every_icon_a_screen_uses_ships_with_the_app`)
- [x] Hardware keyboard in the terminal (arrows, Escape); pointer hover
- [x] Settings pages

## Security

- [x] vornd serves other machines over TLS with its own long-lived self-signed certificate, kept in the data dir (`tls-cert.der`, `tls-key.der`, key 0600). Plain connections are accepted from this machine only.
- [x] The pairing QR (`&fp=`) and the pair-by-hand panel carry the certificate's SHA-256 fingerprint, on the Rust and macOS desktops.
- [x] The phone pins it for pairing and for its sockets: no system trust and no prompts. New pairings need the fingerprint.
- [x] The LAN listener opens only while sharing is on in Settings.
- [x] Settings › Devices shows each device's name, last seen time and address, with Revoke; revoking closes its open sockets at once (close code 4002).
- [x] The phone's Settings shows the computer and certificate it trusts, with Forget this computer. A computer paired before certificates is marked "Not secured. Pair again".
- [x] Tests: a wrong fingerprint is refused (`trusts_a_client_only_with_the_pinned_certificate`, `a_socket_connects_only_to_the_certificate_it_pinned`); a revoked token is refused at once on open sockets (`a_revoked_device_is_closed_out_at_once`); with sharing off nothing listens on the LAN (`nothing_listens_on_the_lan_while_sharing_is_off`).
- New dependencies: rustls (TLS, ring provider only), rcgen (makes the certificate), tokio-rustls (TLS on vornd's and the phone's async sockets).

## Left out, and why

- Push notifications: the previous app has none; vornd has no push sender yet.
- Reading why an agent stalled: not in the previous app either.
- Permission prompts with several choices: answered as allow or deny, as before.
- iPhone landscape: portrait, as before; iPad rotates.
- Connector icons on task rows: the source shows as text.
- Project icons inside menu pickers: SwiftUI menus show text only.
- Drag to reorder tasks: neither app has it; manual order is kept.
- Session scope is project or branch, where the previous app had project or worktree.
- Crash reporting to a development sink: a development aid, not a feature.

## Performance

| Budget | Target | Measured | Method |
| --- | --- | --- | --- |
| Screen open | < 100 ms | Mac, 10k tasks and 1k sessions: sessions 2.0 ms (18.5 before paging), tasks 3.7 ms, task 0.13 ms, workflows 0.05 ms, run 0.13 ms | `screens_build_within_budget_at_ten_thousand_tasks`; on the device, launch with `-vornBench 1` |
| Status update | < 16 ms | Only screens whose revision changed are rebuilt and re-sent | Revision per screen; `vp_revision` gate in `Core.swift` |
| List scroll | 120 Hz | Lazy `List` with stable node ids, 100-row pages | Instruments, Animation Hitches, on iPhone 15 Pro Max |
| Keystroke to glyph | < 30 ms over Wi-Fi | Pending on the device | Instruments signpost from key to drawn frame |
| Idle CPU | ~0 | The terminal draws only when its output or scroll changes | Xcode CPU gauge, idle session |
