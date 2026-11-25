import test from 'node:test';
import assert from 'node:assert/strict';
import {normalizeHistory,normalizeSupportPreview,validSavedFilename} from '../src/support-history.js';

const build={version:'0.5.1',commit:'a'.repeat(40),tree:'modified'};
const event={unix_ms:1000,action:'create_invite',outcome:'returned_ok',build};
const history={enabled:true,retained_count:1,older_events_dropped:false,recording_may_be_incomplete:false,events:[event]};
const report={schema:'swarmcraft-redacted-support-v1',generated_unix_ms:2000,desktop_build:build,last_reported_node_path:'relay',scope:'selected_world',history};
const preview={token:'b'.repeat(64),report};
test('exact preview shape can be reviewed and copied without token or world identity',()=>{
  const normalized=normalizeSupportPreview(preview,true);
  assert.deepEqual(normalized,preview);
  assert.ok(!JSON.stringify(normalized.report).includes(preview.token));
  assert.throws(()=>normalizeSupportPreview(preview,false));
});
test('reject native payload additions at every nesting boundary, not regex masking',()=>{
  for(const change of [
    {...preview,raw:'scinvite:deadbeef'},
    {...preview,report:{...report,address:'10.0.0.2'}},
    {...preview,report:{...report,desktop_build:{...build,path:'/home/person'}}},
    {...preview,report:{...report,history:{...history,world_id:'scworld:private'}}},
    {...preview,report:{...report,history:{...history,events:[{...event,error:'password'}]}}},
    {...preview,report:{...report,last_reported_node_path:'192.168.1.2'}},
    {...preview,report:{...report,history:{...history,events:[{...event,action:'scinvite:deadbeef'}]}}},
  ])assert.throws(()=>normalizeSupportPreview(change,true));
});
test('bounds, exact counts, known outcomes and build provenance are required',()=>{
  for(const raw of [
    {...history,retained_count:2},{...history,events:Array(257).fill(event),retained_count:257},
    {...history,events:[{...event,unix_ms:Infinity}]},{...history,events:[{...event,outcome:'success'}]},
    {...history,events:[{...event,build:{...build,commit:null,tree:'clean'}}]},
    {...history,events:[{...event,build:{...build,version:'private-user'}}]},
  ])assert.throws(()=>normalizeHistory(raw));
  assert.deepEqual(normalizeHistory({...history,events:[],retained_count:0,enabled:false}).events,[]);
});
test('save receipt contains only a Rust-generated basename',()=>{
  assert.ok(validSavedFilename('swarmcraft-support-1795626954103-123456abcdef.json'));
  assert.ok(!validSavedFilename('/home/person/swarmcraft-support-1-123456abcdef.json'));
  assert.ok(!validSavedFilename('scinvite:deadbeef'));
});
