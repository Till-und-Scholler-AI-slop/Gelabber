// Monotone sequence fences prevent torn snapshots and slot-reuse ambiguity.
export const RING = Object.freeze({ header: 8, groupHeader: 8, row: 6, stop: 0, heartbeat: 1, nativeLower: 2 });
export function dimensions(groups, capacity) {
  if (!Number.isInteger(groups) || groups < 1 || groups > 2 || !Number.isInteger(capacity) || capacity < 2 || capacity > 32768) throw Error('invalid callback ring dimensions');
  return { stride: RING.groupHeader + capacity * RING.row, cells: RING.header + groups * (RING.groupHeader + capacity * RING.row) };
}
export function publishCallback(cells, group, capacity, sequence, frame, quantum, lower, flags, inputFrames) {
  const base = RING.header + group * (RING.groupHeader + capacity * RING.row);
  const slot = base + RING.groupHeader + (sequence - 1) % capacity * RING.row;
  Atomics.store(cells, slot, -BigInt(sequence));
  for (const [i, value] of [frame, quantum, lower, flags, inputFrames].entries()) Atomics.store(cells, slot + i + 1, BigInt(value));
  Atomics.store(cells, slot, BigInt(sequence));
  Atomics.store(cells, base, BigInt(sequence));
}
export function drainCallbacks(cells, group, capacity, cursor, stamp) {
  const base = RING.header + group * (RING.groupHeader + capacity * RING.row);
  const latest = Number(Atomics.load(cells, base)), rows = [];
  let missing = 0;
  if (latest < cursor) throw Error('callback sequence regressed');
  if (latest - cursor > capacity) { missing += latest - cursor - capacity; cursor = latest - capacity; }
  for (let sequence = cursor + 1; sequence <= latest; sequence++) {
    const slot = base + RING.groupHeader + (sequence - 1) % capacity * RING.row;
    const before = Atomics.load(cells, slot);
    const values = Array.from({ length: 5 }, (_, index) => Atomics.load(cells, slot + index + 1));
    const upper = stamp(), after = Atomics.load(cells, slot);
    if (before !== BigInt(sequence) || after !== before) { missing++; continue; }
    rows.push({ sequence, firstFrame: Number(values[0]), frames: Number(values[1]), lowerMs: Number(values[2]) / 1e6, upperMs: Number(upper) / 1e6, flags: Number(values[3]), inputFrames: Number(values[4]) });
  }
  return { rows, missing, cursor: latest };
}
