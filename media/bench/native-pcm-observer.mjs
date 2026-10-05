import { RING, dimensions, drainCallbacks } from './native-pcm-ring.mjs';

self.onmessage = ({ data: options }) => {
  const { sab, groups, capacity, token, maxRows = 200000, pauseMs = .25 } = options;
  const shape = dimensions(groups, capacity), cells = new BigInt64Array(sab);
  if (cells.length !== shape.cells) throw Error('callback ring size differs');
  if (!Number.isInteger(maxRows) || maxRows < 100 || maxRows > 2000000 || !Number.isFinite(pauseMs) || pauseMs < 0 || pauseMs > 1000 || typeof token !== 'string' || token.length < 32) throw Error('invalid observer policy');
  const rows = Array.from({ length: groups }, () => []), cursors = Array(groups).fill(0);
  const clocks = [], failures = [];
  let missing = 0;
  const stamp = () => BigInt(Math.round(performance.now() * 1e6));
  const drain = () => {
    for (let group = 0; group < groups; group++) {
      const result = drainCallbacks(cells, group, capacity, cursors[group], stamp);
      rows[group].push(...result.rows); missing += result.missing; cursors[group] = result.cursor;
      if (rows[group].length > maxRows) throw Error('bounded callback evidence limit exceeded');
    }
  };
  let minimumStepMs = Infinity, maximumStepMs = 0, last = performance.now();
  for (let index = 0; index < 100000; index++) {
    const next = performance.now(), step = next - last;
    if (step < 0) throw Error('worker clock regressed');
    if (step > 0) { minimumStepMs = Math.min(minimumStepMs, step); maximumStepMs = Math.max(maximumStepMs, step); }
    last = next;
  }
  self.postMessage({ type: 'ready', timeOrigin: performance.timeOrigin, crossOriginIsolated, precision: { minimumStepMs, maximumStepMs, samples: 100000 } });
  try {
    while (!Atomics.load(cells, RING.stop)) {
      Atomics.store(cells, RING.heartbeat, stamp());
      drain();
      const xhr = new XMLHttpRequest();
      xhr.open('GET', '/clock', false); xhr.timeout = 2000;
      xhr.setRequestHeader('authorization', 'Bearer ' + token);
      const p0 = performance.now(); xhr.send(); const p1 = performance.now();
      if (xhr.status !== 200) throw Error('native clock RPC failed');
      const clock = JSON.parse(xhr.responseText);
      if (clock.clock !== 'CLOCK_MONOTONIC' || !/^\d+$/.test(clock.monoNs)) throw Error('wrong native clock response');
      clocks.push({ p0, p1, monoNs: clock.monoNs });
      if (clocks.length > maxRows) throw Error('bounded clock evidence limit exceeded');
      Atomics.store(cells, RING.nativeLower, BigInt(clock.monoNs));
      if (pauseMs) Atomics.wait(new Int32Array(sab), 7, 0, pauseMs);
    }
    drain();
  } catch (error) { failures.push(String(error)); }
  finally { self.postMessage({ type: 'complete', rows, clocks, missing, failures, timeOrigin: performance.timeOrigin }); }
};
