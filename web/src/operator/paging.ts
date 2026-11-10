import { useState } from "react";

/** Paging starts over whenever the search or the filter (`key`) changes. */
export function usePaging(key: string) {
  const [paging, setPaging] = useState({ key, offset: 0 });
  const offset = paging.key === key ? paging.offset : 0;
  return [offset, (next: number) => setPaging({ key, offset: next })] as const;
}
