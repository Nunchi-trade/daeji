# Ansible Runs as Root with No Sudo Boundary or Audit Trail

**Category:** Security -- Deployment/Infrastructure
**Severity:** High

## Summary

All Ansible playbooks connect to the remote devnet server directly as `root` via SSH. There is no dedicated deployment user, no sudo boundary, and no privilege escalation audit trail. A compromised developer workstation gains immediate, unrestricted root access to all managed infrastructure, and accidental execution of destructive playbooks (like `reset.yml`, which wipes all devnet state) has no privilege escalation prompt to prevent mistakes.

## Problem

Kora uses Ansible to manage its devnet infrastructure on a Hetzner server. The Ansible inventory (`ansible/inventory/hosts.yml`, line 7) sets `ansible_user: root`, which means every playbook task runs directly as the root user on the remote server.

The Ansible playbook suite includes both routine and destructive operations:

| Playbook                            | Effect                                                   |
|-------------------------------------|----------------------------------------------------------|
| `ansible/playbooks/deploy.yml`      | Syncs code, builds Docker image, starts devnet           |
| `ansible/playbooks/reset.yml`       | Wipes all containers, volumes, and state                 |
| `ansible/playbooks/provision.yml`   | Installs packages, configures firewall, sets up Docker   |
| `ansible/playbooks/stop.yml`        | Stops all devnet containers                              |

Several playbooks declare `become: true` (e.g., `deploy.yml` at line 5), but this directive is meaningless when `ansible_user` is already root -- privilege escalation from root to root is a no-op. The `become: true` declarations create a false impression that privilege boundaries exist.

There is one devnet host defined in the inventory:

```yaml
# ansible/inventory/hosts.yml
all:
  children:
    devnet:
      hosts:
        hetzner-devnet:
          ansible_host: 65.21.232.29
          ansible_user: root
          ansible_python_interpreter: /usr/bin/python3
```

**File:** `ansible/inventory/hosts.yml`, line 7
**File:** `ansible/playbooks/deploy.yml`, lines 1-13
**File:** `ansible/playbooks/reset.yml`, lines 1-8

## Code Reference

The inventory configuration:

```yaml
# ansible/inventory/hosts.yml:1-9
all:
  children:
    devnet:
      hosts:
        hetzner-devnet:
          ansible_host: 65.21.232.29
          ansible_user: root
          ansible_python_interpreter: /usr/bin/python3
```

The deploy playbook (note the redundant `become: true` when already root):

```yaml
# ansible/playbooks/deploy.yml:1-13
---
# Repeatable deploy: sync code -> build image -> start devnet
- name: Deploy devnet
  hosts: devnet
  become: true
  roles:
    - role: sync
      tags: [sync]
    - role: build
      tags: [build]
    - role: devnet
      tags: [devnet]
```

The destructive reset playbook (no confirmation, no privilege barrier):

```yaml
# ansible/playbooks/reset.yml:1-8
---
# Wipe devnet clean: stop containers, remove volumes, optionally remove image
- name: Reset devnet
  hosts: devnet
  become: true
  roles:
    - reset
```

## Impact

1. **Compromised developer workstation = full root access.** Any attacker who compromises an engineer's laptop (e.g., via SSH key theft, malware, or a supply chain attack on development tools) gains immediate root access to the devnet server. There is no secondary authentication factor, no sudo password, and no MFA.
2. **No audit trail for privilege escalation.** Because all operations run as root, there is no `sudo` log that distinguishes between automated deployment actions and manual administrative actions. All actions appear identically as root operations in system logs.
3. **No accidental execution guardrail.** Running `ansible-playbook ansible/playbooks/reset.yml -i ansible/inventory/hosts.yml` from the wrong terminal or the wrong branch immediately wipes all devnet state (containers, volumes, DKG shares, QMDB data) with no confirmation prompt, no privilege escalation request, and no undo capability.
4. **Blast radius is unlimited.** Root access allows modification of any file on the system, including the SSH configuration itself, the Docker daemon, the firewall rules, and kernel parameters. An attacker can establish persistence that survives a full redeploy.

## Root Cause

The inventory was configured with `ansible_user: root` for simplicity during initial server setup. No dedicated deployment user was created on the remote server, and no privilege separation policy was established. The `become: true` directives in playbooks were likely added as a "best practice" cargo-cult without recognizing they are no-ops when already root.

## Suggested Fix

1. **Create a dedicated deployment user** on the remote server:

   ```bash
   # On the remote server
   useradd -m -s /bin/bash kora-deploy
   mkdir -p /home/kora-deploy/.ssh
   # Copy the authorized SSH public key
   cp /root/.ssh/authorized_keys /home/kora-deploy/.ssh/
   chown -R kora-deploy:kora-deploy /home/kora-deploy/.ssh
   ```

2. **Configure limited sudo permissions** for the deployment user:

   ```
   # /etc/sudoers.d/kora-deploy
   kora-deploy ALL=(root) NOPASSWD: /usr/bin/docker, /usr/bin/docker-compose, /usr/bin/systemctl restart docker
   kora-deploy ALL=(root) NOPASSWD: /usr/sbin/nft -f /etc/nftables.conf
   ```

3. **Update the Ansible inventory**:

   ```yaml
   # BEFORE (ansible/inventory/hosts.yml:7)
   ansible_user: root

   # AFTER
   ansible_user: kora-deploy
   ```

4. **Use `become: true` only where genuinely needed** -- package installation, service management, and firewall configuration.

5. **Add a confirmation prompt for destructive playbooks** by requiring an extra variable:

   ```yaml
   # ansible/playbooks/reset.yml
   - name: Confirm reset
     ansible.builtin.fail:
       msg: "Pass -e confirm_reset=yes to confirm destructive reset"
     when: confirm_reset is not defined or confirm_reset != 'yes'
   ```

## Files to Modify

- `ansible/inventory/hosts.yml` -- change `ansible_user` from `root` to a dedicated deployment user
- `ansible/playbooks/reset.yml` -- add confirmation guard for destructive operations
- Remote server -- create the deployment user with appropriate sudo permissions

## Related Issues

- [181 - Firewall trusted_ips defaults to 0.0.0.0/0](/Users/will/dev/nunchi/daeji/tmp/kora/issues/181-firewall-trusted-ips-open-to-internet.md) -- firewall misconfiguration amplifies the impact of root access: if the server is already exposed, a compromised SSH key provides root
- [192 - Ansible sync rsync --delete removes manual configs](/Users/will/dev/nunchi/daeji/tmp/kora/issues/192-ansible-sync-rsync-delete-removes-manual-configs.md) -- the sync role runs as root and can silently delete operator-placed files

## Labels

`security`, `config`, `docker`, `reliability`
