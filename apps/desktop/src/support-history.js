// The native report is an allow-listed projection, never the raw Activity
// console. Validate its exact shape before displaying, copying or saving.
const ACTIONS = Object.freeze({
  create_world:'Create world', import_world:'Import world', join_request:'Join request', leave_request:'Leave request',
  create_invite:'Create invitation', revoke_invite:'Revoke invitation', verify_replica:'Verify replica',
  replica_preference:'Replica preference', hosting_preference:'Hosting preference', configure_runtime:'Configure runtime',
  install_runtime:'Install runtime', repair_runtime:'Repair runtime', verify_runtime:'Verify runtime', play:'Play',
  stop_world:'Stop world', wake_request:'Wake request', transfer_request:'Transfer request', transfer_step:'Transfer step',
  recovery_copy:'Recover separate copy', export_checkpoint:'Export checkpoint', recover_checkpoint:'Recover checkpoint',
  export_conflict:'Export conflict', mod_acquire:'Acquire required mods', mod_add:'Add mod', mod_remove:'Remove mod',
  publication_preference:'Publication preference', start_networking:'Start networking', stop_networking:'Stop networking',
});
const OUTCOMES=Object.freeze({returned_ok:'Native returned OK',returned_error:'Native returned error',interrupted:'Interrupted'});
const PATHS=new Set(['direct','relay','direct_and_relay','limited','unknown','unavailable']);
const exact=(value,keys)=>value!==null&&typeof value==='object'&&!Array.isArray(value)
  &&Object.keys(value).length===keys.length&&keys.every(key=>Object.hasOwn(value,key));
const bool=value=>typeof value==='boolean';
const time=value=>Number.isSafeInteger(value)&&value>0&&value<=8.64e15;
const id=value=>typeof value==='string'&&/^scworld:[0-9a-f]{64}$/.test(value);
const hex=value=>typeof value==='string'&&/^[0-9a-f]{64}$/.test(value);
const fail=()=>{throw Error('Invalid redacted support receipt');};
function build(raw){
  if(!exact(raw,['version','commit','tree'])||
    !(raw.version===null||typeof raw.version==='string'&&/^\d{1,6}\.\d{1,6}\.\d{1,6}$/.test(raw.version))||
    !(raw.commit===null||typeof raw.commit==='string'&&/^[a-f0-9]{40}$/.test(raw.commit))||
    !['clean','modified','unknown'].includes(raw.tree)||(raw.tree==='clean'&&!raw.commit))fail();
  return {version:raw.version,commit:raw.commit,tree:raw.tree};
}
export function normalizeHistory(raw){
  if(!exact(raw,['enabled','retained_count','older_events_dropped','recording_may_be_incomplete','events'])||
    !bool(raw.enabled)||!bool(raw.older_events_dropped)||!bool(raw.recording_may_be_incomplete)||
    !Array.isArray(raw.events)||raw.events.length>256||raw.retained_count!==raw.events.length)fail();
  const events=raw.events.map(e=>{
    if(!exact(e,['unix_ms','action','outcome','build'])||!time(e.unix_ms)||
      !Object.hasOwn(ACTIONS,e.action)||!Object.hasOwn(OUTCOMES,e.outcome))fail();
    return {unix_ms:e.unix_ms,action:e.action,outcome:e.outcome,build:build(e.build)};
  });
  return {enabled:raw.enabled,retained_count:raw.retained_count,older_events_dropped:raw.older_events_dropped,
    recording_may_be_incomplete:raw.recording_may_be_incomplete,events};
}
export function normalizeSupportPreview(raw,selected){
  if(!exact(raw,['token','report'])||!hex(raw.token))fail();
  const r=raw.report;
  if(!exact(r,['schema','generated_unix_ms','desktop_build','last_reported_node_path','scope','history'])||
    r.schema!=='swarmcraft-redacted-support-v1'||!time(r.generated_unix_ms)||
    !PATHS.has(r.last_reported_node_path)||r.scope!==(selected?'selected_world':'all_worlds'))fail();
  return {token:raw.token,report:{schema:r.schema,generated_unix_ms:r.generated_unix_ms,desktop_build:build(r.desktop_build),
    last_reported_node_path:r.last_reported_node_path,scope:r.scope,history:normalizeHistory(r.history)}};
}
export function validSavedFilename(value){
  return typeof value==='string'&&/^swarmcraft-support-\d{1,16}-[a-f0-9]{12}\.json$/.test(value);
}
const $=id=>document.getElementById(id);
const node=(tag,value,className)=>{
  const e=document.createElement(tag);e.textContent=value;if(className)e.className=className;return e;
};
export function installSupportHistory(invoke,getWorld){
  if(!$('supportHistoryTitle')||typeof invoke!=='function')return;
  const scope=$('supportScope'), status=$('supportHistoryStatus'), recording=$('supportRecording'),
    refresh=$('supportRefresh'), clear=$('supportClear'), prompt=$('supportClearPrompt'),
    clearConfirm=$('supportClearConfirm'), clearCancel=$('supportClearCancel'),
    events=$('supportEvents'), empty=$('supportEmpty'), previewButton=$('supportPreview'),
    exportStatus=$('supportExportStatus'), panel=$('supportPreviewPanel'), output=$('supportPreviewText'),
    save=$('supportSave'), copy=$('supportCopy'), discard=$('supportDiscard');
  let busy=false,history=null,preview=null,revision=0,selected=null,pendingRead=false,expiry;
  const visible=()=>!$('activityView').hidden;
  const scopedWorld=()=>scope.value==='selected'&&id(selected)?selected:null;
  function notice(text,tone='neutral'){status.textContent=text;status.dataset.tone=tone;}
  function exportNotice(text,tone='neutral'){exportStatus.textContent=text;exportStatus.hidden=!text;exportStatus.dataset.tone=tone;}
  function invalidate(){clearTimeout(expiry);preview=null;output.value='';panel.hidden=true;exportNotice('');}
  function controls(){
    recording.disabled=busy||!history;refresh.disabled=busy;clear.disabled=busy||!history;
    clearConfirm.disabled=busy;clearCancel.disabled=busy;scope.disabled=busy;
    previewButton.disabled=busy||!history;save.disabled=busy||!preview;copy.disabled=busy||!preview;discard.disabled=busy;
    previewButton.textContent=preview?'Regenerate preview':'Preview redacted report';
    previewButton.classList.toggle('button-primary',!preview);
    previewButton.classList.toggle('button-secondary',!!preview);
    recording.textContent=history?.enabled?'Pause recording':'Enable history';
    $('supportHistoryTitle').closest('section').setAttribute('aria-busy',String(busy));
  }
  function render(){
    events.replaceChildren();
    empty.hidden=!!history?.events.length;
    empty.textContent=!history?'History unavailable. Refresh to check again.':history.enabled?
      'No recorded actions in this scope yet.':'Recording is off. No saved actions in this scope.';
    if(history){
      for(const event of [...history.events].reverse()){
        const li=node('li','','support-event');
        const text=node('div','','support-event-copy');
        text.append(node('strong',ACTIONS[event.action]),node('span',OUTCOMES[event.outcome],'meta-text'));
        const stamp=node('time',new Date(event.unix_ms).toLocaleString(),'meta-text');
        stamp.dateTime=new Date(event.unix_ms).toISOString();
        li.append(text,stamp);events.append(li);
      }
      const count=history.retained_count;
      notice((history.enabled?'Recording on.':'Recording off.')+' '+count+' saved action(s) in this scope.'
        +(history.older_events_dropped?' Older entries were removed at the retention limit.':'')
        +(history.recording_may_be_incomplete?' Some actions may not have been recorded.':''),
        history.recording_may_be_incomplete?'warning':'neutral');
    }
    controls();
  }
  async function operation(command,args,handle,{historyOperation=false}={}){
    if(busy)return;
    busy=true;controls();
    const request=revision;
    try{
      const raw=await invoke(command,args);
      if(request===revision&&visible())handle(raw);
    }catch{
      if(request===revision&&visible()){
        if(historyOperation){history=null;render();notice('History could not be confirmed. Refresh before changing it.','warning');}
        else {invalidate();exportNotice('Report operation could not be confirmed. Generate a new preview.','warning');}
      }
    }finally{
      busy=false;controls();
      if(pendingRead&&visible()){pendingRead=false;void read();}
    }
  }
  function read(){
    if(!visible())return;
    if(busy){pendingRead=true;return;}
    notice('Reading local history…');
    return operation('support_history',{world:scopedWorld()},raw=>{history=normalizeHistory(raw);render();},{historyOperation:true});
  }
  function changed(){
    revision++;invalidate();prompt.hidden=true;history=null;render();void read();
  }
  function sync(){
    const value=getWorld();
    const previous=selected;
    selected=id(value)?value:null;
    scope.options[1].disabled=!selected;
    if(!selected&&scope.value==='selected')scope.value='all';
    if(!visible()){
      revision++;invalidate();prompt.hidden=true;pendingRead=false;history=null;controls();return;
    }
    if(previous!==selected&&scope.value==='selected')changed();
    else if(!history)void read();
  }
  document.addEventListener('swarmcraftcontextchanged',sync);
  scope.addEventListener('change',changed);
  refresh.addEventListener('click',()=>{invalidate();void read();});
  recording.addEventListener('click',()=>{
    if(!history||busy)return;
    const enabled=!history.enabled;
    invalidate();prompt.hidden=true;
    void operation('support_recording',{world:scopedWorld(),enabled,confirm:true},raw=>{
      history=normalizeHistory(raw);if(history.enabled!==enabled)fail();render();
    },{historyOperation:true});
  });
  clear.addEventListener('click',()=>{if(busy)return;prompt.hidden=false;clearConfirm.focus();});
  clearCancel.addEventListener('click',()=>{prompt.hidden=true;clear.focus();});
  clearConfirm.addEventListener('click',()=>{
    if(busy)return;
    invalidate();prompt.hidden=true;
    void operation('support_clear_history',{world:scopedWorld(),confirm:true},raw=>{
      history=normalizeHistory(raw);if(history.retained_count!==0)fail();render();refresh.focus();
    },{historyOperation:true});
  });
  previewButton.addEventListener('click',()=>{
    if(!history||busy)return;
    invalidate();const world=scopedWorld();
    exportNotice('Preparing a local-only redacted report…');
    void operation('support_preview',{world,confirm:true},raw=>{
      preview=normalizeSupportPreview(raw,!!world);
      output.value=JSON.stringify(preview.report,null,2)+'\n';panel.hidden=false;
      exportNotice('Review the exact JSON below. No data has been sent. Preview expires in five minutes.');
      expiry=setTimeout(()=>{invalidate();exportNotice('Preview expired. Generate a new report.','warning');controls();},300_000);
      controls();output.focus();output.setSelectionRange(0,0);output.scrollTop=0;
      panel.scrollIntoView({block:'nearest'});
    });
  });
  copy.addEventListener('click',async()=>{
    if(!preview||busy)return;
    const current=preview, request=revision;
    busy=true;controls();
    try{await navigator.clipboard.writeText(JSON.stringify(current.report,null,2)+'\n');
      if(request===revision&&preview===current&&visible())exportNotice('Redacted JSON copied. Nothing was sent.');}
    catch{if(request===revision&&preview===current&&visible()){output.focus();output.select();exportNotice('Clipboard unavailable. Copy the selected JSON manually.','warning');}}
    finally{busy=false;controls();if(pendingRead&&visible()){pendingRead=false;void read();}}
  });
  save.addEventListener('click',()=>{
    if(!preview||busy)return;
    const token=preview.token;
    exportNotice('Choose a local folder for the reviewed report…');
    void operation('support_save',{token,confirm:true},raw=>{
      invalidate();
      if(raw===null)exportNotice('Save cancelled. Nothing was written.');
      else if(validSavedFilename(raw))exportNotice('Saved '+raw+' in the chosen folder. Nothing was sent.');
      else fail();
      previewButton.focus();
    });
  });
  discard.addEventListener('click',()=>{invalidate();controls();previewButton.focus();});
  sync();
}
