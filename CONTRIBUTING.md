# Contributing to Vorn

Vorn takes contributions as issues. Bug reports, feature requests and ideas are all welcome.

We do not accept pull requests from outside the team for now. The core changes daily, so outside patches go stale before we can review them. An issue that explains the problem well helps us more than a patch.

## Open an issue

Pick a template on the [new issue page](https://github.com/vorn-run/vorn/issues/new/choose):

- **Bug report** for something that is broken.
- **Feature request** for a new idea or a change to how something works.

Search the existing issues first. If one already covers your case, add your details there.

## Write a good bug report

A bug we can reproduce is a bug we can fix. Include:

- **Vorn version.** Find it in Settings › Updates.
- **OS and version.** For example, macOS 15.3 or Ubuntu 24.04.
- **Steps to reproduce.** Start from a fresh launch and list each step.
- **What you expected** to happen.
- **What happened instead.** Add screenshots if the problem is visual.
- **Logs.** Attach the lines around the time of the problem from:
  - the daemon log: `~/.vorn/server.log`
  - the app log:
    - macOS: `~/Library/Logs/vorn/main.log`
    - Linux: `~/.config/vorn/logs/main.log`
    - Windows: `%USERPROFILE%\AppData\Roaming\vorn\logs\main.log`

Check logs for tokens, paths or anything else private before you post them.

## Request a feature

Describe the problem before the solution. Tell us what you are trying to do, what gets in the way, and how you work around it today. Then describe the change you have in mind and any alternatives you considered.

## Security issues

Do not report security problems in a public issue. Use [private vulnerability reporting](https://github.com/vorn-run/vorn/security/advisories/new) instead.
