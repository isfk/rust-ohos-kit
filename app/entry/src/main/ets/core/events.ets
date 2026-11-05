// The one place a core event becomes something a component knows about.
//
// The core pushes events through a single thread-safe channel, and the bridge hands
// them to whichever function listen() was given — so the registration can only
// be owned in one place. That place used to be Index.ets, where a growing
// if (e.kind === …) chain meant every new event edited the page.
//
// Now a subscriber takes the kinds it cares about and the page doesn't move:
//
//   const off = onCoreEvent('todos', (e: Event) => { … });
//   startCoreEvents();   // once, before rustcore.start()

import { Event, rustcore } from './RustCore';

/** What a subscriber to a core event is. */
export type CoreEventHandler = (e: Event) => void;

const handlers: Map<string, Set<CoreEventHandler>> = new Map<string, Set<CoreEventHandler>>();
let wired: boolean = false;

/**
 * Takes one kind of core event, and answers with the unsubscribe: a component
 * that goes away has to call it, or it keeps being written to after its state
 * is gone.
 *
 * Subscribe before rustcore.start(): the core pushes its first list as soon as it
 * has read one, and an event with nobody listening is an event lost.
 */
export function onCoreEvent(kind: string, fn: CoreEventHandler): () => void {
  const existing = handlers.get(kind);
  const set = existing !== undefined ? existing : new Set<CoreEventHandler>();
  if (existing === undefined) {
    handlers.set(kind, set);
  }
  set.add(fn);
  return (): void => {
    set.delete(fn);
  };
}

/**
 * Wires the bridge to the subscriptions above. Safe to call from every page:
 * listen() takes one function, and a second registration would drop the first.
 */
export function startCoreEvents(): void {
  if (wired) {
    return;
  }
  wired = true;
  rustcore.listen((e: Event): void => {
    const set = handlers.get(e.kind);
    if (set === undefined) {
      return;
    }
    // A copy would be wasteful here: the set is small, and forEach tolerates a
    // handler that unsubscribes itself.
    set.forEach((fn: CoreEventHandler): void => {
      fn(e);
    });
  });
}
