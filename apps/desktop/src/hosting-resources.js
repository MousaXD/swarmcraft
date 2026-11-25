// Shipping frontend adapter for the same Rust resource-consent API used by
// the candidate component. A saved preference is never authority or a route.
const byId = id => document.getElementById('hostingContribution' + id);
const valid = (number, min, max) => Number.isSafeInteger(number) && number >= min && number <= max;

export function resourceStatus(raw) {
  const cpu = raw.process_cpu;
  const memory = raw.process_memory;
  const requested = raw.selected_cpu_millicores == null
    ? (raw.consented ? 'legacy heap-only' : '0')
    : raw.selected_cpu_millicores / 1000 + ' CPU equivalents';
  const capacity = raw.cpu_capacity_millicores == null ? 'unknown' : raw.cpu_capacity_millicores / 1000 + ' CPU equivalents (not reserved)';
  let enforcement = 'No explicit active CPU cap observed.';
  if (cpu?.mode === 'linux_cgroup_active') {
    enforcement = 'Active admission: ' + cpu.admitted_heap_mib + ' MiB Java heap, ' + cpu.limit_millicores / 1000 +
      ' CPU equivalents; kernel CPU time ' + Math.floor(cpu.usage_usec / 1000000) + ' s.';
  } else if (cpu?.mode === 'linux_cgroup_reserved') {
    enforcement = 'Prepared admission: ' + cpu.admitted_heap_mib + ' MiB Java heap, ' + cpu.limit_millicores / 1000 + ' CPU equivalents; no active JVM verified.';
  } else if (cpu?.mode === 'linux_cgroup_configured') {
    enforcement = 'CPU quota configured for a future launch; no active CPU admission observed.';
  } else if (cpu?.mode === 'unavailable') {
    enforcement = 'Requested CPU cap unavailable; hosting blocked.';
  }
  let charges = 'Heap-only: Whole-JVM RSS is not hard-capped.';
  if (memory?.mode === 'linux_cgroup_active') charges = 'Linux cgroup charge cap: ' + memory.limit_mib + ' MiB; current charges (including descendants): ' + Math.ceil(memory.current_bytes / 1048576) + ' MiB.';
  if (memory?.mode === 'linux_cgroup_configured') charges = 'Linux cgroup charge cap configured: ' + memory.limit_mib + ' MiB; not yet verified active.';
  if (memory?.mode === 'unavailable') charges = 'Memory enforcement unavailable; hosting is blocked.';
  return {
    cpu: 'Requested CPU: ' + requested + '. Measured capacity: ' + capacity + '. ' + enforcement,
    memory: 'Requested Java heap: ' + raw.selected_heap_mib + ' MiB · Available memory: ' +
      (raw.available_mib == null ? 'unknown' : raw.available_mib + ' MiB') + '. ' + charges,
  };
}

export function installHostingResources(invoke, selectedWorld) {
  const panel = byId('Panel');
  if (!panel) return;
  const form = byId('Form'), heap = byId('Heap'), cpu = byId('Cpu'), confirm = byId('Confirm');
  const save = byId('OptIn'), withdraw = byId('Withdraw'), refresh = byId('Refresh');
  const memory = panel.querySelector('[data-hosting-memory]');
  let generation = 0, world = '', readiness = null, busy = false;
  const current = (id, token) => selectedWorld() === id && world === id && generation === token;
  const message = (element, value) => { element.textContent = value; element.hidden = !value; };
  function controls() {
    const enabled = Boolean(readiness) && !busy;
    heap.disabled = cpu.disabled = confirm.disabled = !enabled;
    refresh.disabled = busy || !world;
    save.disabled = !enabled || !confirm.checked || !valid(heap.valueAsNumber, 512, 65536) || !valid(cpu.valueAsNumber, 250, 64000);
    withdraw.disabled = !enabled || !readiness?.consented;
    save.textContent = busy ? 'Working…' : 'Save explicit opt-in';
    panel.setAttribute('aria-busy', String(busy));
  }
  function render(resetInputs = false) {
    panel.dataset.consentState = busy ? 'checking' : readiness ? (readiness.consented ? 'opted-in' : 'player-only') : 'unknown';
    byId('Status').textContent = busy && !readiness ? 'Checking saved consent and device capacity…'
      : !readiness ? 'Local hosting preference unavailable — no eligibility is assumed.'
      : readiness.checkpoint_state === 'checkpoint_blocked' ? 'Checkpoint not confirmed — recovery required'
      : readiness.checkpoint_state === 'stopping_at_checkpoint' ? 'Stopping at checkpoint'
      : readiness.consented ? 'Hosting resource preference saved' : 'Player only — 0 MiB / 0 CPU';
    const explanation = readiness ? readiness.reason + ' This is a local capacity check, not permission to take authority.' : '';
    byId('Reason').textContent = readiness?.checkpoint_state === 'checkpoint_blocked'
      ? explanation + ' Keep the controller running if Java is active; review Diagnostics. Do not force-stop or reset the world.'
      : explanation;
    const status = readiness ? resourceStatus(readiness) : { cpu: '', memory: '' };
    byId('CpuStatus').textContent = status.cpu;
    memory.textContent = status.memory;
    if (resetInputs && readiness) {
      heap.value = String(readiness.selected_heap_mib || 4096);
      cpu.value = String(readiness.selected_cpu_millicores || 1000);
      confirm.checked = false;
    }
    controls();
  }
  function validate(raw, id) {
    if (!raw || raw.world_id !== id || typeof raw.consented !== 'boolean' ||
        !(raw.selected_heap_mib === 0 || valid(raw.selected_heap_mib, 512, 65536)) ||
        raw.consented !== (raw.selected_heap_mib !== 0) ||
        (raw.selected_cpu_millicores != null && (!raw.consented || !valid(raw.selected_cpu_millicores, 250, 64000))) ||
        typeof raw.reason !== 'string') throw new Error('Invalid native hosting readiness');
    return raw;
  }
  async function read(resetInputs = true) {
    if (!world || busy) return;
    const id = world, token = ++generation;
    busy = true;
    message(byId('Error'), '');
    render();
    try {
      const result = validate(await invoke('hosting_contribution_get', { world: id }), id);
      if (!current(id, token)) return;
      readiness = result;
      render(resetInputs);
    } catch {
      if (!current(id, token)) return;
      readiness = null;
      confirm.checked = false;
      message(byId('Error'), 'Cannot read this device’s hosting contribution. Review Diagnostics and retry.');
    } finally {
      if (current(id, token)) { busy = false; render(); }
    }
  }
  async function change(heapMib, cpuMillicores) {
    if (!world || busy || !readiness) return;
    const id = world, token = ++generation;
    busy = true;
    message(byId('Error'), '');
    message(byId('Notice'), '');
    render();
    try {
      window.__swarmcraftInvalidateTransientSafety?.(id);
      const result = validate(await invoke('hosting_contribution_set', { world: id, heapMib, cpuMillicores }), id);
      if (!current(id, token)) return;
      if (result.selected_heap_mib !== heapMib || (result.selected_cpu_millicores ?? 0) !== cpuMillicores || !result.consent_revision)
        throw new Error('Unconfirmed preference');
      readiness = result;
      confirm.checked = false;
      message(byId('Notice'), heapMib === 0
        ? 'Player-only preference saved. An active host must still finish safe checkpoint, fencing and handoff or shutdown.'
        : 'Local preference saved. Only verified world policy, quorum and fencing can grant host authority.');
    } catch {
      if (!current(id, token)) return;
      readiness = null;
      confirm.checked = false;
      message(byId('Error'), 'Could not confirm resource preference. Refresh status before retrying; do not assume a running host has stopped.');
    } finally {
      if (current(id, token)) { busy = false; render(); }
    }
  }
  form.addEventListener('submit', event => {
    event.preventDefault();
    if (!save.disabled) void change(heap.valueAsNumber, cpu.valueAsNumber);
  });
  withdraw.addEventListener('click', () => { if (!withdraw.disabled) void change(0, 0); });
  refresh.addEventListener('click', () => void read());
  for (const input of [heap, cpu]) input.addEventListener('input', () => { confirm.checked = false; controls(); });
  confirm.addEventListener('change', controls);
  function selection() {
    if (world === selectedWorld()) return;
    generation++;
    world = selectedWorld();
    readiness = null;
    busy = false;
    confirm.checked = false;
    message(byId('Error'), ''); message(byId('Notice'), '');
    render();
    if (world) void read();
  }
  document.addEventListener('swarmcraftcontextchanged', selection);
  window.setInterval(() => {
    if (!busy && readiness && (['stopping_at_checkpoint', 'checkpoint_blocked'].includes(readiness.checkpoint_state)
        || ['linux_cgroup_active','linux_cgroup_reserved'].includes(readiness.process_cpu?.mode))) void read(false);
  }, 1500);
  selection();
  controls();
}
