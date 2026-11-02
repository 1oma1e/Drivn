// One state object, immutable updates, at most one render per frame.
// requestAnimationFrame also means no rendering at all while the window is hidden.

export function createStore(state, render) {
  let queued = false;
  return {
    get: () => state,
    /** Shallow-merges `patch` (an object, or a function of the state that returns one). */
    set(patch) {
      state = { ...state, ...(typeof patch === 'function' ? patch(state) : patch) };
      if (queued) return;
      queued = true;
      requestAnimationFrame(() => {
        queued = false;
        render(state);
      });
    },
  };
}

/** Caches the last result of `fn` while its arguments stay the same objects. */
export function memo(fn) {
  let last = null;
  let out;
  return (...args) => {
    if (last && args.length === last.length && args.every((x, i) => x === last[i])) return out;
    last = args;
    return (out = fn(...args));
  };
}
