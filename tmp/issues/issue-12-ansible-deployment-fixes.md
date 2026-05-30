# Ansible Deployment Fixes: Override File Missing, Stale Barriers, Wrong Metric Names, Missing Error Handling, Hard-Coded Values

## Summary

The Ansible deployment pipeline for Kora's remote devnet has several bugs that cause silent failures during deploy, restart, and diagnostic operations. The six highest-impact issues are:

1. **`override.yml` does not exist** -- the original issue described an override file that is silently ignored, but the file does not exist anywhere in the repository. The compose override mechanism must either be created or all references to it removed.
2. **Barrier files persist across deploys** -- the startup barrier volume (`startup_barrier`) is never cleaned by Ansible, causing stale `.ready` files to bypass the barrier synchronization on redeploy.
3. **Diagnostic playbooks reference wrong Prometheus metric names** -- five metric names across `diagnose.yml` and `query-metrics.yml` do not match the actual metrics emitted by validators, producing empty or `NaN` results.
4. **`query-metrics.yml` has no error handling** -- the default multi-query mode lacks `ignore_errors` and safe Jinja defaults, so a single failed Prometheus query crashes the entire playbook with a template error.
5. **Hard-coded node counts and developer-specific paths** -- node indices `0 1 2 3` are hard-coded in loops across 4 playbooks and 1 role instead of deriving from `num_validators`. The `collect-logs.yml` fetch destination is hard-coded to `/Users/will/dev/nunchi/daeji/tmp/logs/`.
6. **Ansible must be run from the `ansible/` directory** -- `ansible.cfg` uses a relative `roles_path = roles` that breaks when invoked from any other directory.

Cross-references:
- **Issue 08** (`tmp/issues/issue-08-observability-stack-fixes.md`) covers the same wrong metric names (Sub-issue 4) plus recording rule duplication, alert threshold tuning, and dashboard metric fixes. The metric name corrections in this issue are a subset of Issue 08's complete metric name mapping table.

---

## Environment

- **Project:** Kora -- a minimal EVM execution client built on the Commonware SDK using Simplex BFT consensus
- **Deployment target:** Single Hetzner server (65.21.232.29) running 4 validators + 1 secondary node in Docker containers, managed by Ansible
- **Relevant files (absolute paths from repo root):**

| File | Purpose |
|------|---------|
| `ansible/ansible.cfg` | Ansible configuration with relative `roles_path` |
| `ansible/inventory/group_vars/devnet.yml` | All deployment variables (`num_validators`, `compose_file`, etc.) |
| `ansible/inventory/hosts.yml` | Inventory (gitignored, contains server IP) |
| `ansible/playbooks/deploy.yml` | Deploy playbook (sync -> build -> devnet) |
| `ansible/playbooks/diagnose.yml` | Diagnostic snapshot playbook (wrong metric names) |
| `ansible/playbooks/query-metrics.yml` | Ad-hoc Prometheus query playbook (wrong metrics, no error handling) |
| `ansible/playbooks/collect-logs.yml` | Log collector (hard-coded fetch path) |
| `ansible/playbooks/chaos-node-failure.yml` | Chaos test (hard-coded node indices) |
| `ansible/playbooks/chaos-rolling-restart.yml` | Rolling restart chaos test (hard-coded node loop) |
| `ansible/roles/devnet/tasks/main.yml` | Core deploy role (missing barrier cleanup, hard-coded volumes) |
| `ansible/roles/chaos/tasks/restart-one-node.yml` | Per-node restart task for rolling chaos |
| `docker/compose/devnet.yaml` | Docker Compose file (no override file exists) |
| `docker/scripts/devnet-run.sh` | Local orchestration script (has correct barrier cleanup) |
| `docker/scripts/entrypoint.sh` | Container entrypoint with barrier mechanism |
| `docker/config/recording-rules.yml` | Prometheus recording rules (authoritative metric names) |
| `docker/config/alerts.yml` | Prometheus alert rules (authoritative metric names) |

---

## Bug 1: `override.yml` Does Not Exist

### Problem

The original issue described a Docker Compose override file (`docker/compose/override.yml`) being silently ignored. After auditing the repository, **this file does not exist anywhere**:

```
$ find . -name '*override*' -type f
# No results
```

The only file in `docker/compose/` is `devnet.yaml`. There is no `override.yml`, `docker-compose.override.yml`, or any variant.

All compose invocations in Ansible use explicit `-f` flags, which bypasses Docker Compose's auto-merge behavior:

In `ansible/inventory/group_vars/devnet.yml` (line 3):
```yaml
compose_file: "{{ remote_project_dir }}/docker/compose/devnet.yaml"
```

Every Ansible task and role invokes:
```
docker compose -f {{ compose_file }} ...
```

The local `docker/scripts/devnet-run.sh` similarly uses:
```bash
docker compose -f compose/devnet.yaml ...
```

### Impact

There is no mechanism for per-environment resource limit overrides, environment variable tweaks, or customizations without editing `devnet.yaml` directly. Operators who create an override file expecting it to be merged (following standard Docker Compose conventions) will be surprised when their changes are not applied.

### Fix: Choose One Approach

**Option A: Create the override mechanism (recommended if overrides are needed)**

1. Create a template override file at `docker/compose/override.yml.example`:
```yaml
# Optional per-environment overrides for devnet.yaml
# Copy to override.yml and customize. Applied via compose_override_file variable.
services:
  validator-node0:
    deploy:
      resources:
        limits:
          memory: 8G
```

2. In `ansible/inventory/group_vars/devnet.yml`, add a `compose_flags` variable that consolidates all `-f` flags:
```yaml
# Base compose flags. To apply overrides, append: -f {{ remote_project_dir }}/docker/compose/override.yml
compose_flags: "-f {{ compose_file }}"
```

3. Replace every `docker compose -f {{ compose_file }}` across all roles and playbooks with `docker compose {{ compose_flags }}`. Affected files:
   - `ansible/roles/devnet/tasks/main.yml` (lines 5, 24, 33, 46, 52, 54, 69, 90, 99, 110)
   - `ansible/roles/observe/tasks/main.yml` (line 5)
   - `ansible/roles/reset/tasks/main.yml` (lines 4-7)

**Option B: Remove all references to overrides (simpler)**

If overrides are not needed, simply document that `devnet.yaml` is the single source of truth and edit it directly. No code changes required -- just remove this bug from tracking.

---

## Bug 2: Barrier Files Not Cleaned Between Deploys

### Problem

The entrypoint script (`docker/scripts/entrypoint.sh`, lines 22-48) implements a startup barrier. Each validator writes a marker file (e.g., `/barrier/node0.ready`) to the shared `startup_barrier` Docker volume, then waits until all 4 markers exist before starting consensus.

This volume persists across container restarts. Ansible's `devnet` role clears runtime state from data volumes but **never clears the barrier volume**.

In `ansible/roles/devnet/tasks/main.yml` (lines 74-85), the "Clear runtime state from data volumes" task cleans `/data/runtime`:
```yaml
- name: Clear runtime state from data volumes
  ansible.builtin.shell: |
    for volume in \
      {{ compose_project_name }}_data_node0 \
      {{ compose_project_name }}_data_node1 \
      {{ compose_project_name }}_data_node2 \
      {{ compose_project_name }}_data_node3 \
      {{ compose_project_name }}_data_secondary0; do
      docker volume inspect "$volume" >/dev/null 2>&1 || continue
      docker run --rm -v "${volume}:/data" alpine rm -rf /data/runtime 2>/dev/null || true
    done
  changed_when: true
```

There is no corresponding task for the `startup_barrier` volume.

The local `docker/scripts/devnet-run.sh` **does** include the correct barrier cleanup (lines 175-180):
```bash
clear_startup_barrier() {
    local volume="kora-devnet_startup_barrier"
    docker volume inspect "$volume" >/dev/null 2>&1 || return 0
    docker run --rm -v "${volume}:/barrier" alpine \
        sh -c 'rm -f /barrier/*.ready' >/dev/null 2>&1 || true
}
```

And calls it at line 319 before starting validators:
```bash
clear_startup_barrier
```

### Impact

On redeploy via Ansible:
1. Validators are stopped
2. Runtime state is cleared
3. Validators are started
4. The stale `.ready` files from the previous run are still present in the barrier volume
5. The barrier immediately passes before all validators have started
6. The bootstrap node advances heights alone, causing height drift

### Fix

Add a new task to `ansible/roles/devnet/tasks/main.yml` immediately after the "Clear runtime state from data volumes" task (after line 85). Insert before the "Start validators and secondary" task:

```yaml
- name: Clear startup barrier files
  ansible.builtin.shell: |
    volume="{{ compose_project_name }}_startup_barrier"
    docker volume inspect "$volume" >/dev/null 2>&1 || exit 0
    docker run --rm -v "${volume}:/barrier" alpine sh -c 'rm -f /barrier/*.ready'
  changed_when: true
```

The complete task sequence in the devnet role should be:
1. Stop existing validators (line 2)
2. Check if DKG shares exist (line 10)
3. Run DKG if needed (lines 22-73)
4. Clear runtime state from data volumes (line 74)
5. **Clear startup barrier files** (NEW -- insert here)
6. Start validators and secondary (line 87)
7. Wait for health checks (line 97)

---

## Bug 3: Diagnostic Playbooks Reference Wrong Prometheus Metric Names

Five metric names across two playbooks do not match the actual metrics emitted by the Commonware SDK. The correct names are confirmed by the authoritative sources: `docker/config/recording-rules.yml` and `docker/config/alerts.yml`.

**Note:** Issue 08 (`tmp/issues/issue-08-observability-stack-fixes.md`, Sub-issue 4) documents the same metric name errors plus additional ones in the Grafana dashboards and diagnostic cookbook. The fixes below are the Ansible-specific subset.

### Complete metric name mapping (Ansible files only)

| Wrong Name (in playbook) | Correct Name (from recording-rules.yml / alerts.yml) | Affected File(s) |
|---|---|---|
| `current_view` | `engine_voter_state_current_view` | `diagnose.yml` line 43, `query-metrics.yml` line 22 |
| `simplex_voter_nullifications_total` | `engine_voter_state_nullifications_total` | `query-metrics.yml` line 24 |
| `process_resident_memory_bytes` | `runtime_process_rss` | `diagnose.yml` line 75, `query-metrics.yml` line 28 |

Sources confirming correct names:
- `engine_voter_state_current_view`: used in `docker/config/recording-rules.yml` line 47 (`kora:views_per_sec`), line 55 (`kora:consensus_efficiency`), line 59 (`kora:skip_rate`), and `docker/config/alerts.yml` line 29 (`VoterCrash`), line 62 (`HighSkipRate`)
- `engine_voter_state_nullifications_total`: used in `docker/config/recording-rules.yml` line 67 (`kora:nullification_rate`) and `docker/config/alerts.yml` line 51 (`HighNullificationRate`)
- `runtime_process_rss`: used in `docker/config/alerts.yml` line 96 (`HighMemoryUsage`) and line 191 (`MemoryLeakSuspected`)

### Bug 3a: Wrong `current_view` in `diagnose.yml` (line 43)

```yaml
# Before (ansible/playbooks/diagnose.yml, line 43):
body: "query=1 - avg(rate(finalized_height[1m])) / avg(rate(current_view[1m]))"

# After:
body: "query=1 - avg(rate(finalized_height[1m])) / clamp_min(avg(rate(engine_voter_state_current_view[1m])), 0.001)"
```

Note: Added `clamp_min` to avoid division by zero, matching the pattern used in `docker/config/recording-rules.yml` line 59.

### Bug 3b: Wrong `process_resident_memory_bytes` in `diagnose.yml` (line 75)

```yaml
# Before (ansible/playbooks/diagnose.yml, line 75):
body: "query=process_resident_memory_bytes"

# After:
body: "query=runtime_process_rss"
```

Also update the report template at line 144 which references "Memory (RSS)" -- the label is fine but the division math assumes bytes. `runtime_process_rss` is already in bytes, so the `/ 1048576` conversion to MB is still correct.

### Bug 3c: Wrong `current_view` in `query-metrics.yml` (line 22)

```yaml
# Before (ansible/playbooks/query-metrics.yml, line 22):
- name: "Skip rate"
  query: "1 - rate(finalized_height[1m]) / rate(current_view[1m])"

# After:
- name: "Skip rate"
  query: "1 - rate(finalized_height[1m]) / clamp_min(rate(engine_voter_state_current_view[1m]), 0.001)"
```

### Bug 3d: Wrong `simplex_voter_nullifications_total` in `query-metrics.yml` (line 24)

```yaml
# Before (ansible/playbooks/query-metrics.yml, line 24):
- name: "Nullifications/sec"
  query: "rate(simplex_voter_nullifications_total[1m])"

# After:
- name: "Nullifications/sec"
  query: "rate(engine_voter_state_nullifications_total[1m])"
```

### Bug 3e: Wrong `process_resident_memory_bytes` in `query-metrics.yml` (line 28)

```yaml
# Before (ansible/playbooks/query-metrics.yml, line 28):
- name: "Memory (MB)"
  query: "process_resident_memory_bytes / 1048576"

# After:
- name: "Memory (MB)"
  query: "runtime_process_rss / 1048576"
```

### Impact

- `diagnose.yml`: The skip rate always shows `NaN` or 0% because `current_view` returns no data. The memory section shows empty because `process_resident_memory_bytes` does not exist.
- `query-metrics.yml`: 3 of 7 default queries (Skip rate, Nullifications/sec, Memory) return empty results. Because the default multi-query block lacks `ignore_errors`, this cascades into Bug 4 below.

---

## Bug 4: Missing Error Handling in `query-metrics.yml`

### Problem

The `query-metrics.yml` playbook has two query modes: custom (single query) and default (multi-query). The custom query mode works fine, but the default multi-query mode (lines 57-78) has no error handling:

**4a: No `ignore_errors` on the query loop**

In `ansible/playbooks/query-metrics.yml` (lines 60-68):
```yaml
- name: Execute queries
  ansible.builtin.uri:
    url: "{{ prom }}/api/v1/query"
    method: POST
    body_format: form-urlencoded
    body: "query={{ item.query }}"
    return_content: true
  loop: "{{ default_queries }}"
  register: query_results
```

If Prometheus is down or returns a non-200 status, this task fails and the playbook aborts. The custom query mode (lines 39-54) has no `ignore_errors` either, but since it is a single query, a failure is informative. For the default batch mode, one failing query should not prevent the others from being displayed.

**4b: Unsafe Jinja template access on empty results**

In `ansible/playbooks/query-metrics.yml` (lines 70-78):
```yaml
- name: "=== METRICS SNAPSHOT ==="
  ansible.builtin.debug:
    msg: |
      {% for result in query_results.results %}
      -- {{ result.item.name }} --
      {% for r in result.json.data.result %}
        {{ r.metric.instance | default('aggregate') }}: {{ (r.value[1] | float) | round(2) }}
      {% endfor %}
      {% endfor %}
```

If any query returns an error response (no `.json.data.result` key), the Jinja template crashes with an `UndefinedError`. This is the most likely failure mode because Bugs 3c-3e cause empty results for metrics that do not exist.

**4c: No timeout on Prometheus queries**

The `uri` module defaults to 30 seconds, but if Prometheus is overloaded, queries can hang. There is no explicit timeout set.

### Fix

Replace the default multi-query block in `ansible/playbooks/query-metrics.yml` (lines 57-78) with:

```yaml
    # Default multi-query mode
    - name: Run default diagnostic queries
      when: promql is not defined
      block:
        - name: Execute queries
          ansible.builtin.uri:
            url: "{{ prom }}/api/v1/query"
            method: POST
            body_format: form-urlencoded
            body: "query={{ item.query }}"
            return_content: true
            timeout: 10
          loop: "{{ default_queries }}"
          register: query_results
          ignore_errors: true

        - name: "=== METRICS SNAPSHOT ==="
          ansible.builtin.debug:
            msg: |
              {% for result in query_results.results %}
              -- {{ result.item.name }} --
              {% if result.failed | default(false) %}
                ERROR: query failed ({{ result.msg | default('unknown') }})
              {% elif result.json is defined and result.json.data is defined %}
              {% for r in result.json.data.result | default([]) %}
                {{ r.metric.instance | default('aggregate') }}: {{ (r.value[1] | float) | round(2) }}
              {% endfor %}
              {% if result.json.data.result | default([]) | length == 0 %}
                (no data)
              {% endif %}
              {% else %}
                (unexpected response format)
              {% endif %}
              {% endfor %}
```

This adds:
- `ignore_errors: true` so one failed query does not abort the batch
- `timeout: 10` to prevent hangs
- Safe Jinja access with `| default()` filters and `result.failed` checks
- "(no data)" output for queries that return empty results instead of silently showing nothing

### Impact

Without this fix, running `query-metrics.yml` when Prometheus is down or when any metric name is wrong causes the entire playbook to crash. With Bug 3's wrong metric names, this means the playbook crashes every time it runs with default queries.

---

## Bug 5: Hard-Coded Node Counts and Developer-Specific Paths

### Problem

Multiple playbooks and roles hard-code `0 1 2 3` for node iteration instead of deriving from the `num_validators` variable defined in `ansible/inventory/group_vars/devnet.yml` (line 12: `num_validators: 4`). One playbook also has a developer-specific local path.

### 5a: Hard-coded node loops

| File | Line(s) | Hard-coded value | Should use |
|------|---------|-----------------|------------|
| `ansible/roles/devnet/tasks/main.yml` | 12, 76-81 | `for i in 0 1 2 3` | `for i in $(seq 0 $(({{ num_validators }} - 1)))` |
| `ansible/playbooks/diagnose.yml` | 89 | `for n in 0 1 2 3` | `for n in $(seq 0 $(({{ num_validators }} - 1)))` |
| `ansible/playbooks/collect-logs.yml` | 28, 39 | `loop: [0, 1, 2, 3]` and `for n in 0 1 2 3` | `loop: "{{ range(num_validators) \| list }}"` |
| `ansible/playbooks/chaos-rolling-restart.yml` | 39 | `loop: [0, 1, 2, 3]` | `loop: "{{ range(num_validators) \| list }}"` |

Also, the devnet role's "Clear runtime state" task (lines 76-81) hard-codes all 5 volume names. If `num_validators` changes, these would be wrong.

The chaos playbooks (`chaos-node-failure.yml` line 9, `chaos-rolling-restart.yml`) are less critical since they accept `target_node` as a variable, but the rolling restart's `loop: [0, 1, 2, 3]` should still be parameterized.

### 5b: Developer-specific fetch path

In `ansible/playbooks/collect-logs.yml` (line 53):
```yaml
- name: Fetch archive to local machine
  ansible.builtin.fetch:
    src: "{{ output_dir }}.tar.gz"
    dest: "/Users/will/dev/nunchi/daeji/tmp/logs/"
    flat: true
```

The path `/Users/will/dev/nunchi/daeji/tmp/logs/` is specific to a single developer's machine. This should use a variable or a relative path.

### Fix

**5a fix:** Define a helper list in `ansible/inventory/group_vars/devnet.yml`:
```yaml
# Derived from num_validators -- list of validator indices [0, 1, 2, 3]
validator_indices: "{{ range(num_validators) | list }}"
```

Then update loops in all affected files. For example, in `ansible/playbooks/collect-logs.yml` (line 28):
```yaml
# Before:
loop: [0, 1, 2, 3]

# After:
loop: "{{ range(num_validators) | list }}"
```

For shell loops in `ansible/roles/devnet/tasks/main.yml` and `ansible/playbooks/diagnose.yml`:
```yaml
# Before:
for i in 0 1 2 3; do

# After:
for i in $(seq 0 $(({{ num_validators }} - 1))); do
```

For the volume list in `ansible/roles/devnet/tasks/main.yml` (lines 76-81):
```yaml
# Before:
for volume in \
  {{ compose_project_name }}_data_node0 \
  {{ compose_project_name }}_data_node1 \
  {{ compose_project_name }}_data_node2 \
  {{ compose_project_name }}_data_node3 \
  {{ compose_project_name }}_data_secondary0; do

# After:
for i in $(seq 0 $(({{ num_validators }} - 1))); do
  volume="{{ compose_project_name }}_data_node${i}"
  docker volume inspect "$volume" >/dev/null 2>&1 || continue
  docker run --rm -v "${volume}:/data" alpine rm -rf /data/runtime 2>/dev/null || true
done
# Secondary separately:
volume="{{ compose_project_name }}_data_secondary0"
docker volume inspect "$volume" >/dev/null 2>&1 || true
docker run --rm -v "${volume}:/data" alpine rm -rf /data/runtime 2>/dev/null || true
```

**5b fix:** In `ansible/playbooks/collect-logs.yml` (line 53), replace the hard-coded path with a variable:
```yaml
# Add to ansible/inventory/group_vars/devnet.yml:
local_log_dir: "{{ playbook_dir }}/../../tmp/logs"

# In collect-logs.yml:
- name: Fetch archive to local machine
  ansible.builtin.fetch:
    src: "{{ output_dir }}.tar.gz"
    dest: "{{ local_log_dir }}/"
    flat: true
```

---

## Bug 6: Ansible Must Be Run from `ansible/` Directory

### Problem

In `ansible/ansible.cfg` (line 3):
```ini
roles_path = roles
```

This is a relative path. Ansible resolves it relative to the current working directory, not relative to `ansible.cfg`. Running from the project root fails:

```bash
# This fails with "role 'sync' not found"
ansible-playbook ansible/playbooks/deploy.yml -i ansible/inventory/hosts.yml
```

You must `cd ansible` first:
```bash
cd ansible
ansible-playbook playbooks/deploy.yml -i inventory/hosts.yml
```

### Fix

Either document this requirement prominently (it is not documented in `ansible.cfg`, `inventory/hosts.yml.example`, or any README), or add a pre-flight check to playbooks. The most portable fix is to add a comment to `ansible.cfg`:

```ini
# NOTE: roles_path is relative -- Ansible must be run from the ansible/ directory.
# Run: cd ansible && ansible-playbook playbooks/deploy.yml -i inventory/hosts.yml
roles_path = roles
```

---

## Additional Issue: Deploy Playbook Does Not Handle Partial Failures

### Problem

The deploy playbook (`ansible/playbooks/deploy.yml`) runs three roles sequentially: `sync` -> `build` -> `devnet`. If the build step fails (compile error, timeout), the play stops but side effects remain:

- Source code has been synced to the remote (new, potentially broken code)
- The Docker image was NOT built
- Validators are still running the old image

The `devnet` role also always stops validators before restarting them (line 2 of `ansible/roles/devnet/tasks/main.yml`), even if nothing changed. This means every deploy causes downtime.

### Recommendation

Add a pre-flight check to the devnet role that verifies the Docker image was actually built before stopping running validators. Insert at the top of `ansible/roles/devnet/tasks/main.yml`:

```yaml
- name: Verify Docker image exists
  ansible.builtin.command:
    cmd: docker image inspect {{ docker_image }}
  register: image_check
  failed_when: image_check.rc != 0

- name: Stop existing validators
  # ... (existing task, line 2)
```

---

## Correct Deployment Procedure

Until these fixes are applied, the correct step-by-step procedure for deploying the devnet:

```bash
# 1. Always run from the ansible/ directory (Bug 6)
cd ansible

# 2. First-time server setup (only needed once)
ansible-playbook playbooks/provision.yml -i inventory/hosts.yml

# 3. Deploy the devnet
ansible-playbook playbooks/deploy.yml -i inventory/hosts.yml

# 4. Manual barrier cleanup (needed on every redeploy until Bug 2 is fixed)
ssh root@65.21.232.29 'docker run --rm -v kora-devnet_startup_barrier:/barrier alpine sh -c "rm -f /barrier/*.ready"'

# 5. Start observability stack
ansible-playbook playbooks/observe.yml -i inventory/hosts.yml

# 6. Verify health (skip rate and memory will show wrong values until Bug 3 is fixed)
ansible-playbook playbooks/diagnose.yml -i inventory/hosts.yml

# Workaround: use direct Prometheus queries with correct metric names
ssh root@65.21.232.29 'curl -s "http://localhost:9090/api/v1/query" \
  --data-urlencode "query=avg(rate(finalized_height[30s]))"'

ssh root@65.21.232.29 'curl -s "http://localhost:9090/api/v1/query" \
  --data-urlencode "query=1 - avg(rate(finalized_height[1m])) / clamp_min(avg(rate(engine_voter_state_current_view[1m])), 0.001)"'

# 7. For a clean redeploy (if validators are stuck):
ansible-playbook playbooks/reset.yml -i inventory/hosts.yml
# WARNING: This destroys all DKG state. A full re-init will be needed.
ansible-playbook playbooks/deploy.yml -i inventory/hosts.yml
ansible-playbook playbooks/observe.yml -i inventory/hosts.yml
```

---

## Files to Change (Complete List)

| File | Change | Bug |
|------|--------|-----|
| `ansible/roles/devnet/tasks/main.yml` | Add barrier cleanup task after line 85 | Bug 2 |
| `ansible/roles/devnet/tasks/main.yml` | Add Docker image pre-flight check at top | Partial failure |
| `ansible/roles/devnet/tasks/main.yml` | Parameterize volume loop with `num_validators` | Bug 5a |
| `ansible/playbooks/diagnose.yml` line 43 | `current_view` -> `engine_voter_state_current_view` (add `clamp_min`) | Bug 3a |
| `ansible/playbooks/diagnose.yml` line 75 | `process_resident_memory_bytes` -> `runtime_process_rss` | Bug 3b |
| `ansible/playbooks/query-metrics.yml` line 22 | `current_view` -> `engine_voter_state_current_view` (add `clamp_min`) | Bug 3c |
| `ansible/playbooks/query-metrics.yml` line 24 | `simplex_voter_nullifications_total` -> `engine_voter_state_nullifications_total` | Bug 3d |
| `ansible/playbooks/query-metrics.yml` line 28 | `process_resident_memory_bytes / 1048576` -> `runtime_process_rss / 1048576` | Bug 3e |
| `ansible/playbooks/query-metrics.yml` lines 60-78 | Add `ignore_errors`, `timeout`, safe Jinja defaults | Bug 4 |
| `ansible/playbooks/collect-logs.yml` line 28 | Replace `loop: [0, 1, 2, 3]` with `range(num_validators)` | Bug 5a |
| `ansible/playbooks/collect-logs.yml` line 39 | Replace `for n in 0 1 2 3` with `seq 0 $((num_validators - 1))` | Bug 5a |
| `ansible/playbooks/collect-logs.yml` line 53 | Replace `/Users/will/...` with `{{ playbook_dir }}/../../tmp/logs` | Bug 5b |
| `ansible/playbooks/diagnose.yml` line 89 | Replace `for n in 0 1 2 3` with `seq 0 $((num_validators - 1))` | Bug 5a |
| `ansible/playbooks/chaos-rolling-restart.yml` line 39 | Replace `loop: [0, 1, 2, 3]` with `range(num_validators)` | Bug 5a |
| `ansible/ansible.cfg` | Add comment about relative roles_path | Bug 6 |
| `ansible/inventory/group_vars/devnet.yml` | Add `compose_flags` variable (if override mechanism desired) | Bug 1 |
| `ansible/inventory/group_vars/devnet.yml` | Add `local_log_dir` variable | Bug 5b |

---

## Cross-References

- **Issue 08** (`tmp/issues/issue-08-observability-stack-fixes.md`): Covers the same metric name corrections (Sub-issue 4) plus additional fixes for recording rules (duplicate names), alert thresholds (fire on healthy networks), and dashboard panels (wrong metric prefix). If Issue 08 is implemented first, Bugs 3a-3e in this issue will already be fixed.
- **Issue 08 Sub-issue 2**: Documents that `kora:p2p:channel_sent:rate1m` recording rules overwrite each other due to duplicate names. Not directly related to Ansible, but the playbook queries that use these recording rules will return partial data.
- **Issue 08 Sub-issue 3**: Documents that `HeightDrift`, `HighNullificationRate`, `HighTimeoutRate`, and `BroadcastFailures` alerts fire on healthy networks. The `diagnose.yml` playbook's "Firing Alerts" section will always show noise until those thresholds are tuned.

---

## Labels

`bug`, `ansible`, `infrastructure`, `devnet`
