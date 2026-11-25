import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeContactSettings, normalizePresence, contactInput } from '../src/friends.js';

const own='scpeer:'+'a'.repeat(64), peer='scpeer:'+'b'.repeat(64), key='c'.repeat(64);
const contact = (overrides={})=>({peer_id:peer,public_key_hex:key,label:'Friend',share_presence:false,presence:'unknown',shared_worlds:[],...overrides});
const settings = (contacts=[contact()],overrides={})=>({own_peer_id:own,own_public_key_hex:'d'.repeat(64),presence_enabled:false,contacts,...overrides});

test('contact records are not inferred consent, live presence or world access',()=>{
  const value=normalizeContactSettings(settings());
  assert.equal(value.presence_enabled,false);
  assert.equal(value.contacts[0].share_presence,false);
  assert.deepEqual(value.contacts[0].shared_worlds,[]);
  assert.throws(()=>normalizeContactSettings(settings([contact({share_presence:undefined})])));
  assert.throws(()=>normalizeContactSettings(settings([contact({presence:'online'})])));
  assert.throws(()=>normalizeContactSettings(settings([contact(),contact()])));
  assert.throws(()=>normalizeContactSettings(settings([contact({peer_id:own})])));
  assert.throws(()=>normalizeContactSettings(settings([],{presence_enabled:undefined})));
});

test('only exact unexpired native observation has positive presence, never a game route',()=>{
  const now=100000;
  const good={peer_id:peer,state:'online',observed_unix_ms:now-1000,expires_unix_ms:now+29000};
  const value=normalizePresence(good,peer,now);
  assert.match(value.message,/Online observed at.*not a live game route/);
  assert.equal(value.expires,now+29000);
  for(const change of [{peer_id:own},{expires_unix_ms:now},{expires_unix_ms:now+30001},{observed_unix_ms:now+1},{observed_unix_ms:null}]) {
    assert.match(normalizePresence({...good,...change},peer,now).message,/Not confirmed/);
  }
});

test('silent, hidden, refused, stale and network failure never assert offline or leak details',()=>{
  for(const state of ['offline','not_observed','stale','invalid','network_unavailable','timeout']) {
    const observation=normalizePresence({peer_id:peer,state,detail:'private address and token'},peer);
    assert.match(observation.message,/Not confirmed/);
    assert.equal(observation.expires,null);
    assert.ok(!observation.message.includes('private address'));
    assert.ok(!observation.message.includes('Offline'));
  }
});

test('contact input rejects short IDs, invite tokens, empty labels and UTF-8 byte overflow',()=>{
  assert.deepEqual(contactInput(peer.toUpperCase().replace('SCPEER:','scpeer:'),key.toUpperCase(),' Friend '),{peer,publicKey:key,label:'Friend'});
  assert.throws(()=>contactInput('scinvite:0000',key,'Friend'));
  assert.throws(()=>contactInput(peer,'bad','Friend'));
  assert.throws(()=>contactInput(peer,key,''));
  assert.throws(()=>contactInput(peer,key,'🙂'.repeat(25)));
});
