import { resolveReactDevtoolsHookEnabled } from './runtime-flags.js';

function disableReactDevtoolsHook() {
  if (typeof window === 'undefined') {
    return;
  }
  if (resolveReactDevtoolsHookEnabled(import.meta.env)) {
    return;
  }

  const noop = () => {};
  const disabledHook = {
    isDisabled: true,
    supportsFiber: true,
    inject: () => -1,
    onCommitFiberRoot: noop,
    onCommitFiberUnmount: noop,
    onPostCommitFiberRoot: noop,
    sub: () => noop,
  };

  window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = disabledHook;
}

disableReactDevtoolsHook();
