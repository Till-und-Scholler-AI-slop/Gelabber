import test from 'node:test';
import assert from 'node:assert/strict';
import { currentNativeMicrophoneBindings } from '../native-peer-current.mjs';
const members = [{ index: 1, user: 'actual-user-1' }, { index: 2, user: 'actual-user-2' }];
const received = () => ({ '51': { ssrc: 51, track_id: 'actual-track-1', stream_ids: ['actual-user-1:a'] },
  '52': { ssrc: 52, track_id: 'actual-track-2', stream_ids: ['actual-user-2:a'] } });
test('current binding uses actual joined owner MSID and receive SSRC', () => {
  const rows = currentNativeMicrophoneBindings(received(), members);
  assert.deepEqual(rows.map(row => [row.source_name, row.ssrc]), [['peer-1/mic', 51], ['peer-2/mic', 52]]);
});
test('unjoined owner, screen audio, missing/duplicate source and forged track graph fail', () => {
  for (const change of [value => { value['51'].stream_ids = ['unjoined:a']; }, value => { value['51'].stream_ids = ['actual-user-1:s']; },
    value => { delete value['52']; }, value => { value['52'].stream_ids = ['actual-user-1:a']; }, value => { value['52'].ssrc = 51; },
    value => { value['52'].track_id = ''; }, value => { value['52'].stream_ids.push('other:a'); }]) {
    const value = received(); change(value); assert.throws(() => currentNativeMicrophoneBindings(value, members));
  }
  assert.throws(() => currentNativeMicrophoneBindings(received(), [members[0], { ...members[1], index: 1 }]));
});
