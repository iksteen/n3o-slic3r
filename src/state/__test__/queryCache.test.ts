// Query-cache spike — proves the behaviors that justify the pattern:
//   - two consumers of a key share ONE fetch (the dedup win),
//   - one invalidation → one refetch,
//   - invalidations arriving mid-flight coalesce to a single trailing refetch,
//   - a failed refetch keeps stale data (non-fatal); a failed INITIAL load is
//     fatal.
//
// The router is mocked out so the cache logic is tested in isolation —
// invalidation is driven directly via `invalidateQuery`, the same call the
// router makes on a Tauri event.

import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../eventRouter", () => ({ onEvents: () => () => {} }));

import {
  defineQuery,
  invalidateQuery,
  peekQueryForTests,
  primeQueryForTests,
  resetQueryCacheForTests,
  selectMemo,
  type QueryState,
} from "../queryCache";

afterEach(() => {
  resetQueryCacheForTests();
  vi.restoreAllMocks();
});

/** Let queued microtasks + the trailing-requeue chain settle. */
const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (v: T) => void;
} {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

describe("queryCache", () => {
  it("shares one fetch across consumers and refetches once per invalidation", async () => {
    const fetch = vi.fn().mockResolvedValue({ n: 1 });
    const q = defineQuery({ key: "q1", fetch, invalidateOn: ["e"] });

    // Two components mount the same key.
    primeQueryForTests(q);
    primeQueryForTests(q);
    await flush();

    expect(fetch).toHaveBeenCalledTimes(1); // shared — not one per consumer
    expect(peekQueryForTests("q1")).toMatchObject({
      data: { n: 1 },
      loading: false,
      error: null,
    });

    invalidateQuery("q1");
    await flush();
    expect(fetch).toHaveBeenCalledTimes(2); // one event → one refetch
  });

  it("coalesces mid-flight invalidations into a single trailing refetch", async () => {
    const first = deferred<{ n: number }>();
    const second = deferred<{ n: number }>();
    const fetch = vi
      .fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const q = defineQuery({ key: "q2", fetch, invalidateOn: ["e"] });

    primeQueryForTests(q); // fetch #1 in flight
    expect(fetch).toHaveBeenCalledTimes(1);

    // Two invalidations land while #1 is still running.
    invalidateQuery("q2");
    invalidateQuery("q2");
    expect(fetch).toHaveBeenCalledTimes(1); // nothing new while in flight

    first.resolve({ n: 1 });
    await flush();
    expect(fetch).toHaveBeenCalledTimes(2); // exactly one trailing refetch, not two

    second.resolve({ n: 2 });
    await flush();
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(peekQueryForTests("q2")).toMatchObject({ data: { n: 2 } });
  });

  it("keeps stale data and stays non-fatal when a refetch fails", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const fetch = vi
      .fn()
      .mockResolvedValueOnce({ n: 1 })
      .mockRejectedValueOnce(new Error("boom"));
    const q = defineQuery({ key: "q3", fetch, invalidateOn: ["e"] });

    primeQueryForTests(q);
    await flush();
    expect(peekQueryForTests("q3")).toMatchObject({ data: { n: 1 }, error: null });

    invalidateQuery("q3");
    await flush();
    const s = peekQueryForTests<{ n: number }>("q3");
    expect(s?.data).toEqual({ n: 1 }); // stale value retained
    expect(s?.error).toBeNull(); // refetch failure is non-fatal
  });

  it("surfaces an initial-load failure as a fatal error", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const fetch = vi.fn().mockRejectedValue(new Error("nope"));
    const q = defineQuery({ key: "q4", fetch, invalidateOn: ["e"] });

    primeQueryForTests(q);
    await flush();
    const s = peekQueryForTests("q4");
    expect(s?.error).toContain("nope");
    expect(s?.data).toBeNull();
  });
});

describe("selectMemo", () => {
  const st = (n: number): QueryState<{ n: number }> => ({
    data: { n },
    loading: false,
    error: null,
  });
  // A selector returning a fresh object each call (the realistic case — a
  // projection), so reference stability has to come from the memo, not the
  // selector.
  const select = (s: QueryState<{ n: number }>): { v: number } => ({
    v: s.data?.n ?? -1,
  });
  const eq = (a: { v: number }, b: { v: number }): boolean => a.v === b.v;

  it("returns the prior memo verbatim when the raw state ref is unchanged", () => {
    const input = st(1);
    const first = selectMemo(null, input, select, eq);
    const second = selectMemo(first, input, select, eq);
    expect(second).toBe(first); // no recompute, same ref
  });

  it("keeps the prior output ref when a new raw state projects equal", () => {
    const first = selectMemo(null, st(1), select, eq);
    // Different raw state object, but the projection (v) is identical.
    const second = selectMemo(first, st(1), select, eq);
    expect(second.output).toBe(first.output); // stable → React skips render
    expect(second.input).not.toBe(first.input); // input advanced
  });

  it("produces a fresh output when the projection actually changes", () => {
    const first = selectMemo(null, st(1), select, eq);
    const second = selectMemo(first, st(2), select, eq);
    expect(second.output).not.toBe(first.output);
    expect(second.output).toEqual({ v: 2 });
  });
});
