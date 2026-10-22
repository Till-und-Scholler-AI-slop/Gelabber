import { useState } from "react";

/**
 * Whether content now stands where this component showed a skeleton: it
 * fades in (`ui-reveal`) instead of popping over the placeholder. Content
 * that was there from the start, from the cache, does not.
 */
export function useAfterSkeleton(loading: boolean): boolean {
  const [hadSkeleton, setHadSkeleton] = useState(loading);
  if (loading && !hadSkeleton) setHadSkeleton(true);
  return hadSkeleton && !loading;
}
