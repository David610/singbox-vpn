# fin1 host changes since the rebuild (each with its rollback)

| When (UTC) | Change | Why | Rollback |
|---|---|---|---|
| 2026-10-22 | `git clone` to `/root/singbox-vpn`; repo-local `core.sshCommand` (deploy key `/root/.ssh/singbox_staging_deploy`, pinned GitHub host key `/root/.ssh/known_hosts_github_singbox`), repo-local `user.name`/`user.email` | durable pushes to `recovery/2026-10-22` | `git -C /root/singbox-vpn config --unset core.sshCommand`; delete the deploy key in GitHub settings and the two files |
| 2026-10-22 | `apt-get install age` (1.0.0-1ubuntu0.1, Ubuntu archive) | encrypted state backup and its test | `apt-get remove age` |
