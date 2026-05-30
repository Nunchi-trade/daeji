# Grafana Admin Password Hardcoded as `admin` in Version-Controlled Defaults

**Category:** Security -- Deployment/Infrastructure
**Severity:** High

## Summary

The Grafana admin password is hardcoded as `admin` in two version-controlled configuration files: the Ansible group vars and the Docker Compose devnet config. No secret injection mechanism (Ansible Vault, external secrets manager, or generated password) is configured. Combined with issue 181 (firewall `trusted_ips` defaults to `0.0.0.0/0`), this gives any internet user admin access to the Grafana monitoring dashboard on provisioned servers.

## Problem

Kora's devnet infrastructure deploys Grafana as part of an optional observability stack (activated via the `observability` Docker Compose profile). The admin password for Grafana is set in two places, both defaulting to the string `admin`:

1. **Ansible group vars** (`ansible/inventory/group_vars/devnet.yml`, line 34): Defines `grafana_admin_password: admin` with a `CHANGEME` comment. This value is used by Ansible roles when configuring the deployment.

2. **Docker Compose** (`docker/compose/devnet.yaml`, line 450): The Grafana container's environment sets `GF_SECURITY_ADMIN_PASSWORD=${GF_SECURITY_ADMIN_PASSWORD:-admin}`, which uses the `admin` fallback when no environment variable override is provided. Additionally, anonymous access is enabled with Viewer role at line 451 (`GF_AUTH_ANONYMOUS_ENABLED=true`).

The password is committed to version control in plaintext. There is no Ansible Vault encryption, no `.env` file mechanism that loads secrets at deploy time, and no runtime password generation.

A Grafana admin can:
1. Read all operational metrics (block heights, nullification rates, memory usage, peer topology) via Prometheus data sources.
2. Modify dashboards and alerting rules to suppress warnings during an attack.
3. Add new data sources and execute arbitrary PromQL or LogQL queries against Prometheus and Loki.
4. In some Grafana versions/plugins, access the container file system or execute commands.

**File:** `ansible/inventory/group_vars/devnet.yml`, line 34
**File:** `docker/compose/devnet.yaml`, line 450

## Code Reference

```yaml
# ansible/inventory/group_vars/devnet.yml:34
grafana_admin_password: admin  # CHANGEME: override in host_vars or vault for production
```

```yaml
# docker/compose/devnet.yaml:448-454
    environment:
      - GF_SECURITY_ADMIN_USER=admin
      - GF_SECURITY_ADMIN_PASSWORD=${GF_SECURITY_ADMIN_PASSWORD:-admin}
      - GF_AUTH_ANONYMOUS_ENABLED=true
      - GF_AUTH_ANONYMOUS_ORG_ROLE=Viewer
      # read_only rootfs prevents writing to /var/log/grafana; use console only
      - GF_LOG_MODE=console
```

## Impact

1. **Publicly accessible monitoring with known credentials.** When combined with issue 181 (firewall defaults to `0.0.0.0/0`), the Grafana instance at port 3000 is reachable from the internet with admin/admin credentials.
2. **Full visibility into operational state.** An attacker with Grafana admin access can query all validator metrics (finalized heights, nullification rates, peer counts, memory usage, block build times) to identify the optimal time and method for an attack.
3. **Alert suppression.** An attacker can modify or delete alert rules (the 31 rules defined in `docker/config/alerts.yml`) to prevent operators from receiving notifications about an ongoing attack.
4. **Credential in version control.** The plaintext password is visible to anyone with repository access. Since this is a known default, automated scanners and bots routinely test for `admin/admin` on Grafana instances.
5. **Anonymous access enabled.** Even without logging in, anyone can view dashboards via the anonymous Viewer role (line 451), though they cannot modify them without admin credentials.

## Root Cause

The default password was set for developer convenience during initial devnet setup. No secret management workflow (Ansible Vault, environment variable injection from a secrets manager, or runtime password generation) was established. The `CHANGEME` comment provides no enforcement mechanism.

## Suggested Fix

1. **Remove the default password from version control** and require it to be provided at deploy time:

   ```yaml
   # BEFORE (ansible/inventory/group_vars/devnet.yml:34)
   grafana_admin_password: admin  # CHANGEME: override in host_vars or vault for production

   # AFTER
   # grafana_admin_password must be set in host_vars or via ansible-vault
   # grafana_admin_password: "{{ vault_grafana_admin_password }}"
   ```

2. **Use Ansible Vault** to encrypt the password:

   ```bash
   ansible-vault encrypt_string 'your-strong-password' --name 'grafana_admin_password'
   ```

3. **Add a validation pre-task** in the deploy playbook:

   ```yaml
   - name: Validate Grafana password is not default
     ansible.builtin.fail:
       msg: "grafana_admin_password is still 'admin'. Set a strong password in host_vars or vault."
     when: grafana_admin_password == 'admin'
   ```

4. **Or generate a random password at provisioning time:**

   ```yaml
   - name: Generate Grafana admin password
     ansible.builtin.set_fact:
       grafana_admin_password: "{{ lookup('password', '/dev/null length=24 chars=ascii_letters,digits') }}"
     when: grafana_admin_password == 'admin'

   - name: Display generated Grafana password
     ansible.builtin.debug:
       msg: "Generated Grafana admin password: {{ grafana_admin_password }}"
     when: grafana_admin_password != 'admin'
   ```

5. **Disable anonymous access** unless explicitly needed:

   ```yaml
   # BEFORE
   - GF_AUTH_ANONYMOUS_ENABLED=true

   # AFTER
   - GF_AUTH_ANONYMOUS_ENABLED=false
   ```

## Files to Modify

- `ansible/inventory/group_vars/devnet.yml` -- remove or vault-encrypt `grafana_admin_password`
- `docker/compose/devnet.yaml` -- remove `admin` fallback from `GF_SECURITY_ADMIN_PASSWORD`, consider disabling anonymous access
- `ansible/playbooks/deploy.yml` or `ansible/playbooks/provision.yml` -- add validation pre-task

## Related Issues

- [181 - Firewall trusted_ips defaults to 0.0.0.0/0](/Users/will/dev/nunchi/daeji/tmp/kora/issues/181-firewall-trusted-ips-open-to-internet.md) -- without firewall restriction, Grafana is reachable from the internet; combined with this issue, provides unauthenticated admin access
- [187 - No alerting destination configured](/Users/will/dev/nunchi/daeji/tmp/kora/issues/187-alerting-no-destination-configured.md) -- even if an attacker does not suppress alerts, they are not delivered anywhere

## Labels

`security`, `config`, `docker`, `reliability`
