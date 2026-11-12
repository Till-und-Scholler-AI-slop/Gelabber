import { useState } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { settledOffset, usePaging, type PageCount } from "./paging.ts";

describe("settledOffset", () => {
  it("stays on a page that has rows, and on the first page", () => {
    expect(settledOffset(50, { rows: 1, total: 51 })).toBe(50);
    expect(settledOffset(0, { rows: 0, total: 0 })).toBe(0);
    // Rows may still arrive behind a total the refetch has not seen.
    expect(settledOffset(50, { rows: 0, total: 51 })).toBe(50);
  });

  it("steps back from a page an action emptied to the last page with rows", () => {
    expect(settledOffset(50, { rows: 0, total: 50 })).toBe(0);
    expect(settledOffset(100, { rows: 0, total: 100 })).toBe(50);
    expect(settledOffset(150, { rows: 0, total: 51 })).toBe(50);
    expect(settledOffset(50, { rows: 0, total: 0 })).toBe(0);
  });
});

/** Goes to `start`, then settles on what `load` answers for each offset. */
function Harness({
  start,
  load,
}: {
  start: number;
  load: (offset: number) => PageCount;
}) {
  const [offset, setOffset, settle] = usePaging("open");
  const [moved, setMoved] = useState(false);
  if (!moved) {
    setMoved(true);
    setOffset(start);
  } else settle(load(offset));
  return <output>{offset}</output>;
}

describe("usePaging", () => {
  it("leaves a page the last row was taken from for the last page with rows", () => {
    // 51 open reports; the one on page 2 was dismissed.
    const html = renderToStaticMarkup(
      <Harness
        start={50}
        load={(offset) => ({ rows: offset === 0 ? 50 : 0, total: 50 })}
      />,
    );
    expect(html).toBe("<output>0</output>");
  });

  it("starts over when the search or the filter changes", () => {
    function Search() {
      const [key, setKey] = useState("all\n");
      const [offset, setOffset] = usePaging(key);
      const [step, setStep] = useState(0);
      if (step === 0) {
        setStep(1);
        setOffset(50);
      } else if (step === 1 && offset === 50) {
        setStep(2);
        setKey("all\nada");
      }
      return <output>{`${step} ${offset}`}</output>;
    }
    expect(renderToStaticMarkup(<Search />)).toBe("<output>2 0</output>");
  });

  it("keeps a later page that still has rows", () => {
    const html = renderToStaticMarkup(
      <Harness start={50} load={() => ({ rows: 1, total: 51 })} />,
    );
    expect(html).toBe("<output>50</output>");
  });
});
