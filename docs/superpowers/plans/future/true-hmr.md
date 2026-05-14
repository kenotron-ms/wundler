# Future Plan: True HMR (Hot Module Replacement)

**Status:** Deferred — currently implemented as live reload (`location.reload()`)

## What We Have Now

`hmr-client.js` connects to `/__wundler__/hmr` via EventSource. On any file change event, it calls `location.reload()`. Full page tear-down, all React state resets, every module re-fetched.

This is live reload, not HMR.

## What True HMR Requires

### 1. react-refresh SWC transform

Every React component file needs the react-refresh transform pass during `transform_on_demand()`:

```rust
// In swc_util.rs — add after JSX transform:
use swc_core::ecma::transforms::react::refresh;

program = program.apply(refresh(Default::default(), None, cm.clone(), None));
```

This injects `__register` and `__accept` calls into components so react-refresh can swap implementations in-place without losing state.

### 2. react-refresh runtime in DEV_INDEX_HTML

```html
<script src="https://esm.sh/react-refresh@0.14.0/runtime"></script>
<script>
  window.$RefreshRuntime$ = ReactRefreshRuntime;
  ReactRefreshRuntime.injectIntoGlobalHook(window);
  window.$RefreshReg$ = () => {};
  window.$RefreshSig$ = () => (type) => type;
</script>
```

### 3. Module-level hot swap in hmr-client.js (not location.reload)

```js
es.addEventListener('change', async (ev) => {
  const path = ev.data;              // e.g. "src/pages/Home.tsx"
  const url = `/${path}?t=${Date.now()}`;  // cache-bust
  try {
    await import(url);               // re-fetch + re-execute the module
    if (window.__wundler_refresh__) {
      window.__wundler_refresh__();  // calls ReactRefreshRuntime.performReactRefresh()
    }
  } catch (e) {
    console.warn('[wundler] HMR failed, falling back to reload', e);
    location.reload();
  }
});
```

### 4. Dependency graph propagation

When `utils/format.ts` changes, any module that imports it also needs to re-execute (it may have bound to the old export values). Wundler already has the full module graph — serialize a lightweight `importers` map into the dev HTML so the HMR client can walk up the dependency chain.

```html
<script>
window.__wundler_graph__ = {
  "src/utils/format.ts": ["src/pages/Home.tsx", "src/utils/analytics.ts"],
  "src/pages/Home.tsx":  ["src/App.tsx"],
  // ...
};
</script>
```

The client iterates `importers[changedPath]` recursively and re-imports the full affected subtree before calling `performReactRefresh()`.

## Acceptance Criteria

- [ ] Edit a React component's JSX → only that component re-renders, `useState` in siblings is preserved
- [ ] Edit a utility module → components that import it re-render, state preserved in unaffected components
- [ ] Edit a module with a non-recoverable error → falls back to `location.reload()` gracefully
- [ ] `[wundler] HMR` console messages show path, not a full page reload marker

## Estimated Effort

3–4 tasks in a plan. The `react-refresh` SWC transform and runtime injection are the bulk of it. Dependency graph propagation is optional for a first pass (fall back to full subtree reload if no `accept` handler).
