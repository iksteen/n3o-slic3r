// Mock of @tauri-apps/api/core for the browser demo. `invoke` dispatches to the
// canned command registry; unmocked commands log once and resolve to a benign
// value. `Channel` is an inert sink (the demo streams nothing).

import { COMMANDS } from "./commands";

const warned = new Set<string>();

export async function invoke<T = unknown>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const handler = COMMANDS[cmd];
  if (handler) return (await handler(args ?? {})) as T;
  if (!warned.has(cmd)) {
    warned.add(cmd);
    console.warn("[demo] unmocked invoke:", cmd, args ?? {});
  }
  return undefined as T;
}

export class Channel<T = unknown> {
  onmessage: ((message: T) => void) | null = null;
  // Real Channels stream camera frames / upload progress; the demo has neither.
}
