# Ansible Sync Role Uses `rsync --delete` Which Silently Removes Manually-Placed Config Files

**Category:** Bug -- Deployment/Infrastructure
**Severity:** Medium

## Summary

The Ansible sync role uses `rsync --delete` (via `ansible.posix.synchronize` with `delete: true`) to mirror the local project tree to the remote devnet server. This removes any file on the remote server that does not exist in the local source tree, including custom configuration files placed by operators (e.g., the 10-node compose file, `.env` files, custom Prometheus rules, override compose files). Every `ansible-playbook deploy.yml` run silently deletes these manual additions, forcing operators to re-apply customizations after every deploy.

## Problem

Kora's Ansible deployment pipeline includes a `sync` role that copies the project source tree to the remote server at `/opt/kora`. The sync task (`ansible/roles/sync/tasks/main.yml`, lines 2-12) uses the `ansible.posix.synchronize` module (which wraps `rsync`) with `delete: true`:

```yaml
- name: Sync project to remote server
  ansible.posix.synchronize:
    src: "{{ playbook_dir }}/../../"
    dest: "{{ remote_project_dir }}/"
    delete: true
    rsync_opts:
      - "--exclude=.git"
      - "--exclude=target/"
      - "--exclude=.DS_Store"
      - "--exclude=testnet-artifacts/"
      - "--exclude=ansible/"
```

The `delete: true` flag tells rsync to remove files on the destination that do not exist in the source. The exclude list (`--exclude`) only covers a few known directories (`.git`, `target/`, `.DS_Store`, `testnet-artifacts/`, `ansible/`). It does not cover common manual additions that operators place on the remote server:

| Missing Exclude                    | What Gets Deleted                                           |
|------------------------------------|-------------------------------------------------------------|
| `docker/compose/*.override.yaml`   | Docker Compose override files for custom configurations     |
| `docker/.env`                      | Environment variable files for Docker Compose               |
| `docker/compose/devnet-10node.yaml`| The 10-node compose file (not in version control)           |
| `tmp/`                             | Runtime artifacts, logs, and temporary files                |
| Custom scripts in `docker/scripts/`| Operator-created scripts that are not in the repo           |

The `remote_project_dir` is set to `/opt/kora` in the Ansible group vars (`ansible/inventory/group_vars/devnet.yml`, line 2).

**File:** `ansible/roles/sync/tasks/main.yml`, lines 2-12

## Code Reference

```yaml
# ansible/roles/sync/tasks/main.yml:1-12
---
- name: Sync project to remote server
  ansible.posix.synchronize:
    src: "{{ playbook_dir }}/../../"
    dest: "{{ remote_project_dir }}/"
    delete: true                          # <-- Deletes any file on remote not in local tree
    rsync_opts:
      - "--exclude=.git"
      - "--exclude=target/"
      - "--exclude=.DS_Store"
      - "--exclude=testnet-artifacts/"
      - "--exclude=ansible/"              # <-- Excludes ansible/ but not other manual additions
```

The deploy playbook that triggers the sync:

```yaml
# ansible/playbooks/deploy.yml:1-13
---
# Repeatable deploy: sync code -> build image -> start devnet
- name: Deploy devnet
  hosts: devnet
  become: true
  roles:
    - role: sync                          # <-- First role: syncs with --delete
      tags: [sync]
    - role: build
      tags: [build]
    - role: devnet
      tags: [devnet]
```

The remote project directory:

```yaml
# ansible/inventory/group_vars/devnet.yml:2
remote_project_dir: /opt/kora
```

## Impact

1. **Custom configurations silently deleted on every deploy.** An operator who has placed a custom compose override file (e.g., `docker/compose/devnet.yaml.override` for 10-node configuration), a `.env` file for environment-specific settings, or custom monitoring rules on the remote server will find them deleted after running `ansible-playbook deploy.yml`.
2. **10-node compose file deleted on every sync.** The current production devnet uses a 10-node compose file (`docker/compose/devnet-10node.yaml`) that is not in version control (tracked in issue 093). Every deploy sync deletes this file, requiring the operator to manually recreate it.
3. **No warning before deletion.** The rsync `--delete` operation runs silently as part of the sync role. The operator sees no warning, diff, or confirmation before files are removed. The deletions only become apparent when a subsequent step fails (e.g., `docker compose -f compose/devnet-10node.yaml up` fails because the file no longer exists).
4. **Workflow friction.** Operators must either re-apply manual changes after every deploy or maintain a separate mechanism to preserve custom files, creating operational overhead and error-prone manual steps.

## Root Cause

The `delete: true` flag was set to ensure the remote server exactly mirrors the local source tree, preventing stale files from accumulating. The exclude list was created with the initial set of known non-code directories but was not updated as the deployment workflow evolved to include manual customizations on the remote server.

## Suggested Fix

**Option A: Expand the exclude list** to cover common manual additions:

```yaml
# BEFORE (ansible/roles/sync/tasks/main.yml)
    rsync_opts:
      - "--exclude=.git"
      - "--exclude=target/"
      - "--exclude=.DS_Store"
      - "--exclude=testnet-artifacts/"
      - "--exclude=ansible/"

# AFTER
    rsync_opts:
      - "--exclude=.git"
      - "--exclude=target/"
      - "--exclude=.DS_Store"
      - "--exclude=testnet-artifacts/"
      - "--exclude=ansible/"
      - "--exclude=tmp/"
      - "--exclude=.env"
      - "--exclude=*.override.yaml"
      - "--exclude=devnet-10node.yaml"
```

**Option B: Switch to `delete: false`** and use a separate cleanup task for known stale files:

```yaml
# BEFORE
    delete: true

# AFTER
    delete: false
```

Then add a separate cleanup task that only removes known stale files (e.g., old build artifacts) rather than deleting everything not in the source tree.

**Option C: Use a `.rsync-filter` file** in the project root to centralize exclusion rules:

```
# .rsync-filter (in project root)
- .git
- target/
- .DS_Store
- testnet-artifacts/
- ansible/
- tmp/
- .env
- *.override.yaml
```

Then in the sync task:

```yaml
    rsync_opts:
      - "--filter=: .rsync-filter"
```

## Files to Modify

- `ansible/roles/sync/tasks/main.yml` -- expand excludes or switch to `delete: false`
- Optionally create `.rsync-filter` in the project root for centralized exclusion rules

## Related Issues

- [093 - 10-node compose file not in version control](/Users/will/dev/nunchi/daeji/tmp/kora/issues/093-docker-10node-compose-not-in-vcs.md) -- the 10-node compose file is the most prominent victim of the `--delete` behavior; putting it in version control would also fix this specific case
- [183 - Ansible runs as root](/Users/will/dev/nunchi/daeji/tmp/kora/issues/183-ansible-runs-as-root-no-sudo-boundary.md) -- the sync role runs as root, so the deletion happens with full privileges and no safeguards

## Labels

`bug`, `docker`, `config`, `reliability`
