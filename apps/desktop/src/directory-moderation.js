import { normalizeWorldId } from './discovery-diagnostics.js';

export const REPORT_REASONS = Object.freeze({
  spam: 'Spam', impersonation: 'Impersonation', malicious_content: 'Malicious content',
  harassment: 'Harassment', misleading_metadata: 'Misleading metadata', other: 'Other',
});
const HASH = /^[a-f0-9]{64}$/;
const byId = id => document.getElementById(id);
const canonicalId = id => typeof id === 'string' && normalizeWorldId(id) === id && !!id;
const validHash = value => typeof value === 'string' && HASH.test(value);
const stamp = value => Number.isSafeInteger(value) && value > 0;
const text = (tag, value, className) => {
  const node = document.createElement(tag);
  node.textContent = value;
  if (className) node.className = className;
  return node;
};

export function normalizeDirectoryPolicy(raw) {
  if (!raw || raw.version !== 1 || !Array.isArray(raw.hidden_worlds) || raw.hidden_worlds.length > 1024
      || raw.hidden_worlds.some(id => !canonicalId(id)) || new Set(raw.hidden_worlds).size !== raw.hidden_worlds.length
      || !Array.isArray(raw.reports) || raw.reports.length > 128) throw new Error('Local directory records could not be verified.');
  const seen = new Set();
  const reports = raw.reports.map(report => {
    if (!report || !validHash(report.report_id) || seen.has(report.report_id) || !canonicalId(report.world_id)
        || !validHash(report.config_hash) || !Object.hasOwn(REPORT_REASONS, report.reason)
        || !stamp(report.verified_unix_ms) || !stamp(report.recorded_unix_ms)
        || report.recorded_unix_ms < report.verified_unix_ms) throw new Error('Local report is invalid.');
    seen.add(report.report_id);
    // Deliberately project only minimal, known fields, never raw native data.
    return {report_id: report.report_id, world_id: report.world_id, config_hash: report.config_hash,
      reason: report.reason, verified_unix_ms: report.verified_unix_ms, recorded_unix_ms: report.recorded_unix_ms};
  });
  return {hidden: new Set(raw.hidden_worlds), reports};
}

export function reportPayload(report) {
  const verified = normalizeDirectoryPolicy({version:1,hidden_worlds:[],reports:[report]}).reports[0];
  return JSON.stringify({schema:'swarmcraft-local-listing-report-v1', allegation:true, ...verified}, null, 2);
}

export function installDirectoryModeration(invoke, onChange) {
  const details = byId('directoryModeration');
  const draft = byId('directoryReportDraft');
  const status = byId('directoryPolicyStatus');
  const refresh = byId('directoryRefresh');
  const hiddenList = byId('directoryHidden');
  const reportsList = byId('directoryReports');
  const form = byId('directoryReportForm');
  const reason = byId('directoryReportReason');
  const save = byId('directoryReportSave');
  const cancel = byId('directoryReportCancel');
  const wrap = byId('directoryExportWrap');
  const output = byId('directoryExport');
  let policy = null;
  let appliedHidden = new Set();
  let busy = false;
  let reportCard = null;
  let exportId = null;
  let generation = 0;

  function notice(value, tone='neutral') {
    status.textContent = value; status.dataset.tone = tone;
  }
  function controls() {
    refresh.disabled = busy; save.disabled = busy; cancel.disabled = busy;
    for (const button of details.querySelectorAll('ul button')) button.disabled = busy;
  }
  function button(label, handler, action) {
    const node=text('button',label,'button button-secondary button-small');
    node.type='button'; node.dataset.action=action;
    node.addEventListener('click',handler);
    return node;
  }
  function clearExport() { exportId=null; output.value=''; wrap.hidden=true; }
  function closeDraft() {
    reportCard=null; draft.hidden=true; reason.value='';
  }
  function render() {
    const focused=document.activeElement;
    const hiddenRow=focused?.closest?.('#directoryHidden li[data-world-id]');
    const reportRow=focused?.closest?.('#directoryReports li[data-report-id]');
    const action=focused?.dataset.action;
    hiddenList.replaceChildren(); reportsList.replaceChildren();
    if (!policy) {
      hiddenList.append(text('li','Not verified. Refresh local records.','field-help'));
      reportsList.append(text('li','Not verified. Refresh local records.','field-help'));
    } else {
      if (!policy.hidden.size) hiddenList.append(text('li','No hidden listings.','field-help'));
      for (const world of policy.hidden) {
        const li=text('li','','directory-record');
        li.dataset.worldId=world;
        li.append(text('span',world,'machine-line'),
          button('Restore',()=>change('directory_hide',{world,enabled:false}, next=>!next.hidden.has(world),'Listing restored to this device’s browse eligibility. Search again.','restore'),'restore'));
        hiddenList.append(li);
      }
      if (!policy.reports.length) reportsList.append(text('li','No saved reports.','field-help'));
      for (const report of [...policy.reports].reverse()) {
        const li=text('li','','directory-record');
        li.dataset.reportId=report.report_id;
        const content=text('div','','directory-record-copy');
        content.append(text('strong', REPORT_REASONS[report.reason] + ' · local allegation'),
          text('span',report.world_id,'machine-line'));
        const actions=text('div','','compact-actions');
        actions.append(button('View report',()=>{exportId=report.report_id;output.value=reportPayload(report);wrap.hidden=false;output.focus();},'view'),
          button('Delete record',()=>change('directory_dismiss_report',{report:report.report_id},
            next=>!next.reports.some(r=>r.report_id===report.report_id),'Local report deleted. No listing or membership changed.','delete'),'delete'));
        li.append(content,actions);reportsList.append(li);
      }
      if (exportId && !policy.reports.some(r=>r.report_id===exportId)) clearExport();
    }
    controls();
    if (hiddenRow || reportRow) {
      const rows=hiddenRow?hiddenList.children:reportsList.children;
      const replacement=[...rows].find(row=>hiddenRow?row.dataset.worldId===hiddenRow.dataset.worldId:row.dataset.reportId===reportRow.dataset.reportId);
      ([...(replacement?.querySelectorAll('button')||[])].find(btn=>btn.dataset.action===action&&!btn.disabled)||refresh).focus();
    }
  }
  async function read() {
    if (busy) return;
    busy=true;controls();onChange();notice('Reading local directory records…');
    try {
      policy=normalizeDirectoryPolicy(await invoke('directory_policy',{}));
      appliedHidden=policy.hidden;
      notice(policy.hidden.size+' hidden listing(s) · '+policy.reports.length+' local report(s). Nothing sent.');
    } catch {
      policy=null;clearExport();notice('Directory policy could not be verified. Refresh before changing it.','warning');
    } finally { busy=false;render();onChange(); }
  }
  async function change(command,args,confirm,message,action) {
    if (busy) return false;
    details.open=true;
    busy=true;controls();onChange();notice(action==='report'?'Rechecking the signed public listing before saving…':'Updating local directory records…');
    const requestGeneration=generation;
    let success=false;
    try {
      // Mutating native commands are awaited to completion. A timeout must
      // not unlock another mutation while the original process still runs.
      const next=normalizeDirectoryPolicy(await invoke(command,args));
      if (!confirm(next)) throw new Error('Change not confirmed');
      policy=next;appliedHidden=next.hidden;success=true;
      notice(message);
    } catch {
      policy=null;clearExport();notice('Change could not be confirmed. Refresh local records; for a report, search again for the exact public listing.','warning');
    } finally {
      busy=false;render();onChange();
    }
    if (requestGeneration!==generation) return false;
    return success;
  }
  form.addEventListener('submit',async event=>{
    event.preventDefault();
    const card=reportCard;
    if (!card || busy || !Object.hasOwn(REPORT_REASONS,reason.value)) { reason.focus(); return; }
    if (card.expires_unix_ms<=Date.now()) {
      closeDraft();details.open=true;notice('Listing expired. Search again before reporting.','warning');return;
    }
    const category=reason.value;
    const ok=await change('directory_report',{world:card.world_id,configHash:card.config_hash,reason:category},
      next=>next.reports.some(r=>r.world_id===card.world_id && r.config_hash===card.config_hash && r.reason===category),
      'Report saved locally. View or copy it below to share manually; nothing was sent.','report');
    if (ok) { closeDraft(); status.scrollIntoView({block:'nearest'});refresh.focus(); }
  });
  cancel.addEventListener('click',()=>{closeDraft();byId('publicWorldSearch').focus();});
  refresh.addEventListener('click',read);
  details.addEventListener('toggle',()=>{if(details.open&&!policy&&!busy)void read();});
  byId('directoryCopyReport').addEventListener('click',async()=>{
    if (!output.value) return;
    try {await navigator.clipboard.writeText(output.value);notice('Minimal report copied. Nothing was sent.');}
    catch {output.focus();output.select();notice('Clipboard unavailable. Copy the selected report manually.','warning');}
  });
  document.addEventListener('swarmcraftcontextchanged',()=>{
    if (byId('discoverPanel').hidden) {generation++;closeDraft();clearExport();}
    else if (details.open && !busy) void read();
  });
  return {
    isHidden: id => appliedHidden.has(id),
    isBusy: () => busy,
    hide: card => {
      if (busy || card.expires_unix_ms<=Date.now()) return;
      void change('directory_hide',{world:card.world_id,enabled:true},next=>next.hidden.has(card.world_id),
        'Listing hidden on this device only. Exact lookup and world membership are unchanged.','hide');
    },
    report: card => {
      if (busy || !card.config_hash || card.visibility!=='public' || card.expires_unix_ms<=Date.now()) return;
      reportCard=card;reason.value='';byId('directoryReportTarget').textContent=card.name+' · '+card.world_id;
      draft.hidden=false;reason.focus();draft.scrollIntoView({block:'nearest'});
    },
    resetDraft: () => { if(!busy) closeDraft(); },
  };
}

export function normalizeListingReceipt(raw, world) {
  if (!raw || !canonicalId(world) || raw.world_id!==world || typeof raw.locally_paused!=='boolean'
      || !['private','public','unlisted'].includes(raw.signed_visibility)
      || raw.signed_visibility_changed!==false || raw.world_membership_changed!==false) return null;
  return {paused:raw.locally_paused,visibility:raw.signed_visibility};
}

export function installWorldListing(invoke, getWorld) {
  const details=byId('worldListingDetails');
  const state=byId('worldListingState');
  const toggle=byId('worldListingToggle');
  const refresh=byId('worldListingRefresh');
  if (!details || typeof invoke!=='function') return;
  let selected=null, receipt=null, busy=false, generation=0, pending=false;
  const shown=()=>details.open && !byId('worldsView').hidden;
  function render() {
    toggle.disabled=busy||!receipt||receipt.visibility==='private';
    refresh.disabled=busy||!selected;
    toggle.textContent=receipt?.paused?'Resume this device':'Pause this device';
    if (receipt) state.textContent='Locally verified signed visibility: '+receipt.visibility+'. '
      +(receipt.visibility==='private'?'Private worlds cannot be published.':receipt.paused?'This device’s publication is paused.':'This device’s publication is not paused; live listing is not guaranteed.');
  }
  async function read(paused=null) {
    if (!shown() || busy || !canonicalId(selected)) return;
    busy=true;pending=false;const id=selected, revision=generation;
    receipt=null;state.textContent=paused===null?'Checking signed visibility and local preference…':'Updating local publication preference…';render();
    try {
      const result=normalizeListingReceipt(await invoke('world_listing',{world:id,paused}),id);
      if (generation!==revision || selected!==id) return;
      if (!result || (paused!==null && result.paused!==paused)) throw new Error('Local preference not verified');
      receipt=result;
    } catch {
      if (generation===revision) state.textContent='Publication state could not be confirmed. Refresh before changing it.';
    } finally {
      busy=false;render();
      if(pending && shown())void read();
    }
  }
  function sync() {
    const id=getWorld();
    if (id!==selected) {
      selected=id;generation++;receipt=null;pending=true;
      state.textContent=id?'Check local publication state.':'Select a world to check its signed visibility.';render();
    }
    if(shown() && !busy)void read();
  }
  details.addEventListener('toggle',sync);
  document.addEventListener('swarmcraftcontextchanged',sync);
  refresh.addEventListener('click',()=>read());
  toggle.addEventListener('click',()=>{if(receipt&&!toggle.disabled)void read(!receipt.paused);});
}

export function installListingMetadata() {
  const visibility=byId('createVisibility');
  const details=byId('createListingMetadata');
  if(!visibility||!details)return;
  function sync() {
    const privateWorld=visibility.value==='private';
    details.hidden=privateWorld;
    if(privateWorld) {
      details.open=false;
      byId('createListingRegion').value='';
      byId('createListingLanguage').value='';
    }
  }
  visibility.addEventListener('change',sync);
  byId('createForm').addEventListener('reset',()=>queueMicrotask(sync));
  sync();
}
