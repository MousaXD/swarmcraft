import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeDirectoryPolicy, reportPayload, normalizeListingReceipt } from '../src/directory-moderation.js';
import { normalizePublicSearch, searchSummary } from '../src/discovery-lobby.js';
import { normalizeDiscoveryCard } from '../src/discovery-diagnostics.js';

const world='scworld:'+'a'.repeat(64), config='b'.repeat(64), id='c'.repeat(64);
const report={report_id:id,world_id:world,config_hash:config,reason:'spam',verified_unix_ms:100,recorded_unix_ms:101};
const policy=(overrides={})=>({version:1,hidden_worlds:[world],reports:[report],...overrides});

test('policy receipt validates exact IDs and categories and rejects duplicate or invalid records',()=>{
  const parsed=normalizeDirectoryPolicy(policy());
  assert.ok(parsed.hidden.has(world));
  assert.deepEqual(parsed.reports,[report]);
  for(const raw of [
    policy({version:2}),policy({hidden_worlds:['scinvite:0000']}),policy({hidden_worlds:[world,world]}),
    policy({reports:[report,report]}),policy({reports:[{...report,reason:'arbitrary allegation'}]}),
    policy({reports:[{...report,recorded_unix_ms:99}]}),
    policy({reports:[{...report,config_hash:'short'}]}),
  ]) assert.throws(()=>normalizeDirectoryPolicy(raw));
});

test('export only minimal explicit allegation fields, never private or unexpected native values',()=>{
  const payload=JSON.parse(reportPayload({...report,token:'scinvite:abcd',private_address:'10.0.0.1'}));
  assert.deepEqual(payload,{schema:'swarmcraft-local-listing-report-v1',allegation:true,...report});
  assert.throws(()=>reportPayload({...report,world_id:'scinvite:aa'}));
});

test('locally verified signed visibility and local pause receipt cannot imply changed membership or config',()=>{
  const raw={world_id:world,signed_visibility:'public',locally_paused:true,signed_visibility_changed:false,world_membership_changed:false};
  assert.deepEqual(normalizeListingReceipt(raw,world),{paused:true,visibility:'public'});
  for(const change of [{world_id:'scworld:'+'d'.repeat(64)},{signed_visibility:'unknown'},{signed_visibility_changed:true},{world_membership_changed:true}]) {
    assert.equal(normalizeListingReceipt({...raw,...change},world),null);
  }
});

test('all hidden listings are not described as a directory with no matches',()=>{
  const raw={state:'ok',results:[],providers_seen:1,providers_responded:1,truncated:false,hidden_results:2};
  const normalized=normalizePublicSearch(raw,1000);
  assert.equal(normalized.state,'ok');
  assert.match(searchSummary(normalized),/2 verified listing\(s\) hidden on this device/);
  assert.doesNotMatch(searchSummary(normalized),/No matching public listings/);
  for(const change of [{hidden_results:33},{hidden_results:-1},{state:'empty'},{providers_responded:0}]) {
    assert.equal(normalizePublicSearch({...raw,...change}).state,'invalid');
  }
});

test('signed language tags project separately, invalid hashes do not permit reporting',()=>{
  const now=100000;
  const card={world_id:world,name:'Village',config_hash:config,visibility:'public',membership_policy:'invite_only',join_action:'invite_required',
    minecraft_version:'26.1.2',loader_id:'fabric',loader_version:'0.19.3',issued_unix_ms:now-1000,
    verified_unix_ms:now,expires_unix_ms:now+60000,approximate_region:'middle-east',
    tags:['lang:ar','lang:ar','lang:en','survival','lang:not-a-language']};
  const normalized=normalizeDiscoveryCard(card,now);
  assert.deepEqual(normalized.languages,['ar','en']);
  assert.equal(normalized.config_hash,config);
  assert.ok(!normalized.tags.includes('lang:ar'));
  assert.equal(normalizeDiscoveryCard({...card,config_hash:'short'},now).config_hash,null);
});
