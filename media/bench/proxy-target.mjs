export function proxyTarget(requestUrl, backend) {
  const base = new URL(backend), target = new URL(requestUrl, base);
  if (target.origin !== base.origin) throw new Error('foreign proxy origin rejected');
  return target;
}
