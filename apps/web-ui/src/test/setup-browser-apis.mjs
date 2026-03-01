const globalObject = globalThis;

if (!globalObject.window) {
  globalObject.window = globalObject;
}

if (!globalObject.requestAnimationFrame) {
  globalObject.requestAnimationFrame = (callback) => {
    const timeoutId = globalObject.setTimeout(() => {
      callback(Date.now());
    }, 16);
    return timeoutId;
  };
}

if (!globalObject.cancelAnimationFrame) {
  globalObject.cancelAnimationFrame = (handle) => {
    globalObject.clearTimeout(handle);
  };
}

if (!globalObject.ResizeObserver) {
  globalObject.ResizeObserver = class ResizeObserver {
    observe() {}

    unobserve() {}

    disconnect() {}
  };
}

if (!globalObject.Worker) {
  class MockWorker {
    static instances = [];

    constructor(url, options) {
      this.url = url;
      this.options = options;
      this.onmessage = null;
      this.onmessageerror = null;
      this.onerror = null;
      this.#listeners = new Map();
      this.#terminated = false;
      this.messages = [];
      MockWorker.instances.push(this);
    }

    #listeners;

    #terminated;

    postMessage(message, transferListOrOptions) {
      if (this.#terminated) {
        return;
      }
      this.messages.push({
        message,
        transferListOrOptions,
      });
    }

    addEventListener(type, callback) {
      if (!this.#listeners.has(type)) {
        this.#listeners.set(type, new Set());
      }
      this.#listeners.get(type).add(callback);
    }

    removeEventListener(type, callback) {
      const listeners = this.#listeners.get(type);
      if (!listeners) {
        return;
      }
      listeners.delete(callback);
    }

    terminate() {
      this.#terminated = true;
    }

    emitMessage(data) {
      const event = { data, target: this };
      if (typeof this.onmessage === 'function') {
        this.onmessage(event);
      }
      this.#emitToListeners('message', event);
    }

    emitError(error) {
      const event = { error, target: this };
      if (typeof this.onerror === 'function') {
        this.onerror(event);
      }
      this.#emitToListeners('error', event);
    }

    emitMessageError(error) {
      const event = { data: error, target: this };
      if (typeof this.onmessageerror === 'function') {
        this.onmessageerror(event);
      }
      this.#emitToListeners('messageerror', event);
    }

    #emitToListeners(type, event) {
      const listeners = this.#listeners.get(type);
      if (!listeners) {
        return;
      }
      for (const callback of listeners) {
        callback(event);
      }
    }
  }

  globalObject.Worker = MockWorker;
}

if (!globalObject.EventSource) {
  class MockEventSource {
    static instances = [];

    constructor(url, options) {
      this.url = String(url ?? '');
      this.options = options;
      this.onopen = null;
      this.onerror = null;
      this.readyState = 0;
      this.closed = false;
      this.#listeners = new Map();
      MockEventSource.instances.push(this);
    }

    #listeners;

    addEventListener(type, callback) {
      if (!this.#listeners.has(type)) {
        this.#listeners.set(type, new Set());
      }
      this.#listeners.get(type).add(callback);
    }

    removeEventListener(type, callback) {
      const listeners = this.#listeners.get(type);
      if (!listeners) {
        return;
      }
      listeners.delete(callback);
    }

    close() {
      this.closed = true;
      this.readyState = 2;
    }

    emitOpen() {
      if (this.closed) {
        return;
      }
      this.readyState = 1;
      const event = { type: 'open', target: this };
      if (typeof this.onopen === 'function') {
        this.onopen(event);
      }
      this.#emitToListeners('open', event);
    }

    emitEvent(type, data) {
      if (this.closed) {
        return;
      }
      const event = { type, data, target: this };
      this.#emitToListeners(type, event);
    }

    emitError(errorLike = {}) {
      const event = {
        ...(typeof errorLike === 'object' && errorLike ? errorLike : { message: String(errorLike ?? '') }),
        target: this,
      };
      if (typeof this.onerror === 'function') {
        this.onerror(event);
      }
      this.#emitToListeners('error', event);
    }

    #emitToListeners(type, event) {
      const listeners = this.#listeners.get(type);
      if (!listeners) {
        return;
      }
      for (const callback of listeners) {
        callback(event);
      }
    }
  }

  globalObject.EventSource = MockEventSource;
}
