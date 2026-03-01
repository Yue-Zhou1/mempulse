// Deprecated: websocket worker transport removed in favor of dashboard SSE events-v1.

self.onmessage = (event) => {
  const message = event?.data;
  if (message?.type === 'stream:stop') {
    self.postMessage({
      type: 'stream:close',
      code: 1000,
      reason: 'stream worker deprecated',
    });
    return;
  }

  if (message?.type === 'stream:init') {
    self.postMessage({
      type: 'stream:error',
      message: 'Dashboard WebSocket worker transport has been removed. Use SSE events-v1.',
    });
  }
};
