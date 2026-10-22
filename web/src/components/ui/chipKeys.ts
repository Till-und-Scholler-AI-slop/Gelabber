/**
 * Where an arrow, Home or End key moves in a row of chips, skipping disabled
 * ones and wrapping around; `null` for any other key.
 */
export function nextChipIndex(
  key: string,
  current: number,
  disabled: readonly boolean[],
): number | null {
  const count = disabled.length;
  const enabled = (index: number) => !disabled[index];
  const step = (from: number, by: 1 | -1) => {
    for (let i = 1; i <= count; i++) {
      const index = (from + by * i + count * count) % count;
      if (enabled(index)) return index;
    }
    return null;
  };
  switch (key) {
    case "ArrowRight":
    case "ArrowDown":
      return step(current, 1);
    case "ArrowLeft":
    case "ArrowUp":
      return step(current, -1);
    case "Home":
      return step(count - 1, 1);
    case "End":
      return step(0, -1);
    default:
      return null;
  }
}
