import test from 'node:test';
import assert from 'node:assert/strict';
import { discoveryPage, normalizePublicSearch, searchSummary, DISCOVERY_RESULT_CAP, DISCOVERY_PAGE_SIZE } from '../src/discovery-lobby.js';

const now = 100_000;
const listing = (i, overrides={}) => ({
  world_id: 'scworld:' + i.toString(16).padStart(64,'0'), name: 'World '+i,
  visibility: 'public', membership_policy:'invite_only', join_action:'invite_required',
  minecraft_version:'26.1.2', loader_id:'fabric', loader_version:'0.19.3',
  issued_unix_ms:now-1_000, verified_unix_ms:now-500, expires_unix_ms:now+60_000, ...overrides,
});
const report = (results, overrides={}) => ({
  state: results.length ? 'ok' : 'empty', results, truncated:false,
  providers_seen:2, providers_responded:2, rejected_stale:0, rejected_invalid:0, ...overrides,
});

test('public snapshot only admits unique unexpired public invite-only verified cards', () => {
  const raw = report([listing(1), listing(2,{visibility:'private'}), listing(3,{visibility:'unlisted'}),
    listing(4), listing(4), listing(5,{expires_unix_ms:now}), listing(6,{join_action:'open'}),
    listing(7,{verified_unix_ms:undefined}), listing(8,{description:'<script>literal</script>'})]);
  const normalized = normalizePublicSearch(raw, now);
  assert.equal(normalized.state,'partial');
  assert.deepEqual(normalized.cards.map(c=>c.world_id), [listing(1).world_id, listing(8).world_id]);
  assert.equal(normalized.cards[1].description, '<script>literal</script>');
});

test('page bounds and expiration are recomputed over the verified snapshot', () => {
  const result = normalizePublicSearch(report(Array.from({length:DISCOVERY_RESULT_CAP},(_,i)=>
    listing(i, {expires_unix_ms:now+(i<6?1000:60000)}))), now);
  assert.equal(result.cards.length, DISCOVERY_RESULT_CAP);
  const first = discoveryPage(result.cards,-9,now);
  assert.equal(first.cards.length,DISCOVERY_PAGE_SIZE);
  assert.equal(first.index,0);
  const last=discoveryPage(result.cards,999,now);
  assert.equal(last.index,5);
  assert.equal(last.cards.length,2);
  const expired=discoveryPage(result.cards,5,now+1000);
  assert.equal(expired.expired,6);
  assert.equal(expired.index,4);
  assert.equal(expired.total,26);
  assert.equal(discoveryPage(result.cards,2,now+60000).cards.length,0);
});

test('empty, incomplete, cap and unavailable states never claim a complete directory', () => {
  for (const [state, expected] of [
    ['empty','No matching public listings'], ['partial','Discovery incomplete'],
    ['network_unavailable','network unavailable'], ['provider_unavailable','No discovery provider'],
  ]) {
    const result=normalizePublicSearch(report([], {state, providers_responded: state==='empty'?2:0}),now);
    assert.match(searchSummary(result), new RegExp(expected,'i'));
  }
  const capped=normalizePublicSearch(report([listing(1)],{state:'partial',truncated:true}),now);
  assert.match(searchSummary(capped),/bounded directory snapshot.*Search limit reached/);
  assert.equal(normalizePublicSearch({state:'timeout'},now).state,'timeout');
  assert.equal(normalizePublicSearch(report([],{providers_responded:0}),now).state,'invalid');
  assert.equal(normalizePublicSearch(report([listing(1)],{state:'partial',providers_responded:0}),now).state,'invalid');
  assert.equal(normalizePublicSearch(report(Array.from({length:33},(_,i)=>listing(i))),now).state,'invalid');
  assert.equal(normalizePublicSearch(report([listing(1)],{state:'network_unavailable'}),now).state,'invalid');
  assert.ok(!searchSummary(normalizePublicSearch(report([],{state:'partial',detail:'secret bootstrap address'}),now)).includes('secret'));
});
