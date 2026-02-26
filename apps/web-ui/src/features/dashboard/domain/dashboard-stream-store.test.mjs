import test from 'node:test';
import assert from 'node:assert/strict';
import {
  __resetDashboardStreamStoreForTests,
  clearTransactionDetailCache,
  getTransactionDetailByHash,
  setTransactionDetailByHash,
  useDashboardStreamStore,
} from './dashboard-stream-store.js';

test.beforeEach(() => {
  __resetDashboardStreamStoreForTests();
  clearTransactionDetailCache();
});

test('stream store merges multiple queued batches into one frame commit', () => {
  const rafCallbacks = [];
  const requestFrame = (callback) => {
    rafCallbacks.push(callback);
    return rafCallbacks.length;
  };

  useDashboardStreamStore.getState().enqueueTickerCommit(
    {
      recentTxRows: [{ hash: '0x01' }],
      transactionRows: [{ hash: '0x01' }],
    },
    { requestFrame },
  );
  useDashboardStreamStore.getState().enqueueTickerCommit(
    {
      recentTxRows: [{ hash: '0x02' }],
      transactionRows: [{ hash: '0x02' }],
    },
    { requestFrame },
  );

  assert.equal(rafCallbacks.length, 1);
  assert.deepEqual(useDashboardStreamStore.getState().transactionRows, []);

  rafCallbacks[0](0);
  assert.deepEqual(
    useDashboardStreamStore.getState().transactionRows.map((row) => row.hash),
    ['0x02'],
  );
});

test('store selector subscriptions stay isolated by field', () => {
  let transactionRowsUpdates = 0;
  let selectionUpdates = 0;
  const unsubscribeTransactionRows = useDashboardStreamStore.subscribe(
    (state) => state.transactionRows,
    () => {
      transactionRowsUpdates += 1;
    },
  );
  const unsubscribeSelection = useDashboardStreamStore.subscribe(
    (state) => state.activeSelectedTxId,
    () => {
      selectionUpdates += 1;
    },
  );

  useDashboardStreamStore.getState().setActiveSelectedTxId('0xabc');
  assert.equal(selectionUpdates, 1);
  assert.equal(transactionRowsUpdates, 0);

  unsubscribeTransactionRows();
  unsubscribeSelection();
});

test('store selection falls back when selected hash is evicted', () => {
  useDashboardStreamStore.getState().commitTickerRows({
    recentTxRows: [{ hash: '0x1' }, { hash: '0x2' }],
    transactionRows: [{ hash: '0x1' }, { hash: '0x2' }],
  });
  useDashboardStreamStore.getState().setActiveSelectedTxId('0x2');

  useDashboardStreamStore.getState().commitTickerRows({
    recentTxRows: [{ hash: '0x3' }],
    transactionRows: [{ hash: '0x3' }],
  });

  assert.equal(useDashboardStreamStore.getState().activeSelectedTxId, '0x3');
});

test('activeSelectedTxId resolves detail from non-reactive cache', () => {
  setTransactionDetailByHash('0xabc', { nonce: 7 }, 10);
  useDashboardStreamStore.getState().setActiveSelectedTxId('0xabc');

  const detail = getTransactionDetailByHash(
    useDashboardStreamStore.getState().activeSelectedTxId,
  );
  assert.deepEqual(detail, { nonce: 7 });
  assert.equal(
    Object.prototype.hasOwnProperty.call(useDashboardStreamStore.getState(), 'transactionDetailsByHash'),
    false,
  );
});
