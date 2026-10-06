// One source capture object can have several native RID encodings.
export function publisherEncoderProgress(observed, before) {
  const captures = observed.captures;
  if (
    captures.length !== 3 ||
    new Set(captures.map((capture) => capture.capture)).size !== 3 ||
    captures.filter((capture) => capture.kind === "camera").length !== 1 ||
    captures.filter((capture) => capture.kind === "display").length !== 2 ||
    observed.senders.length !== 3
  )
    return false;
  return captures.every(({ capture }) => {
    const senders = observed.senders.filter(
      (sender) => sender.capture === capture,
    );
    if (senders.length !== 1 || !senders[0].current) return false;
    const sender = senders[0];
    const frames = sender.encodings.reduce(
      (total, encoding) => total + (encoding.frames ?? 0),
      0,
    );
    if (!(frames > 0)) return false;
    if (!before) return true;
    const prior = before.senders.find(
      (item) =>
        item.current &&
        item.capture === capture &&
        item.peer === sender.peer &&
        item.sender === sender.sender,
    );
    if (!prior) return false;
    return (
      frames >
      prior.encodings.reduce(
        (total, encoding) => total + (encoding.frames ?? 0),
        0,
      )
    );
  });
}
