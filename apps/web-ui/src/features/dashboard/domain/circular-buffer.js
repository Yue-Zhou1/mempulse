function normalizeCapacity(capacity) {
  if (!Number.isFinite(capacity)) {
    return 0;
  }
  return Math.max(0, Math.floor(capacity));
}

export function createCircularBuffer(initialCapacity = 0) {
  let capacity = normalizeCapacity(initialCapacity);
  let storage = new Array(capacity);
  let head = 0;
  let length = 0;

  function clear() {
    storage = new Array(capacity);
    head = 0;
    length = 0;
  }

  function push(value) {
    if (capacity === 0) {
      return;
    }
    if (length < capacity) {
      storage[(head + length) % capacity] = value;
      length += 1;
      return;
    }
    storage[head] = value;
    head = (head + 1) % capacity;
  }

  function appendMany(values) {
    for (const value of values ?? []) {
      push(value);
    }
  }

  function toArray() {
    if (length === 0) {
      return [];
    }
    const result = new Array(length);
    for (let index = 0; index < length; index += 1) {
      result[index] = storage[(head + index) % capacity];
    }
    return result;
  }

  function setCapacity(nextCapacity) {
    const normalizedCapacity = normalizeCapacity(nextCapacity);
    if (normalizedCapacity === capacity) {
      return;
    }
    const preserved = toArray();
    capacity = normalizedCapacity;
    storage = new Array(capacity);
    head = 0;
    length = 0;
    if (capacity === 0) {
      return;
    }
    const startIndex = Math.max(0, preserved.length - capacity);
    for (let index = startIndex; index < preserved.length; index += 1) {
      push(preserved[index]);
    }
  }

  return {
    appendMany,
    capacity: () => capacity,
    clear,
    push,
    setCapacity,
    size: () => length,
    toArray,
  };
}
