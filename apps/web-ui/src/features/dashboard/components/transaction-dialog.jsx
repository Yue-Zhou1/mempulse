export function TransactionDialog({ model, actions }) {
  const {
    dialogHash,
    dialogLoading,
    dialogTransaction,
    dialogMainnet,
    detailSeenRelative,
    detailSender,
    detailSeenAt,
    dialogFeature,
    dialogDetail,
    dialogError,
  } = model;

  const {
    closeDialog,
  } = actions;

  if (!dialogHash) {
    return null;
  }

  return (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-sm"
          onClick={closeDialog}
        >
          <div
            className="news-dialog-enter relative max-h-[92vh] min-h-[32rem] w-full max-w-5xl overflow-auto border-2 border-zinc-900 bg-[#f7f1e6] p-6 text-[18px] shadow-2xl [scrollbar-gutter:stable]"
            aria-busy={dialogLoading}
            onClick={(event) => event.stopPropagation()}
          >
            {dialogLoading ? (
              <div className="absolute inset-0 z-20 flex items-center justify-center bg-[#f7f1e6]/55 backdrop-blur-[2px]">
                <div className="news-mono border border-zinc-900 bg-[#fffdf7] px-4 py-3 text-[16px] uppercase tracking-[0.08em] text-zinc-700 shadow-sm">
                  Loading on-demand transaction detail...
                </div>
              </div>
            ) : null}

            <div className="news-detail-masthead">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="news-kicker">
                    Transaction Detail
                  </div>
                  <div className="news-mono mt-1 break-all text-[18px] leading-relaxed text-zinc-900">{dialogHash}</div>
                </div>
                <button
                  type="button"
                  onClick={closeDialog}
                  aria-label="Close transaction detail"
                  className="news-icon-button"
                >
                  <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden="true">
                    <path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="square" />
                  </svg>
                </button>
              </div>
              <div className="news-mono mt-2 flex flex-wrap items-center justify-between gap-2 border-t border-zinc-900 pt-2 text-[16px] uppercase tracking-[0.1em] text-zinc-700">
                <div className="flex flex-wrap items-center gap-3">
                  <span>Source Edition · {dialogTransaction?.source_id ?? '-'}</span>
                  <span>Mainnet · {dialogMainnet}</span>
                </div>
                <span>{detailSeenRelative}</span>
              </div>
            </div>

            <div className="mt-4 grid gap-4 lg:grid-cols-[minmax(0,70%)_minmax(0,30%)]">
              <section className="news-detail-section">
                <div className="news-kicker border-b border-zinc-900 pb-1">
                  Ledger Entry
                </div>
                <dl className="news-detail-kv mt-3">
                  <dt>Sender</dt>
                  <dd
                    className="news-mono break-all text-[18px]"
                    title={detailSender}
                  >
                    {detailSender}
                  </dd>
                  <dt>Tx Type</dt>
                  <dd>{dialogTransaction?.tx_type ?? '-'}</dd>
                  <dt>Seen At</dt>
                  <dd>{detailSeenAt}</dd>
                  <dt>Mainnet</dt>
                  <dd>{dialogMainnet}</dd>
                  <dt>Method</dt>
                  <dd className="news-mono text-[18px]">{dialogFeature?.method_selector ?? '-'}</dd>
                </dl>
              </section>
              <section className="news-detail-section">
                <div className="news-kicker border-b border-zinc-900 pb-1">
                  Wire Details
                </div>
                <dl className="news-detail-kv mt-3">
                  <dt>Nonce</dt>
                  <dd>{dialogTransaction?.nonce ?? dialogDetail?.nonce ?? '-'}</dd>
                  <dt>Peer</dt>
                  <dd>{dialogDetail?.peer ?? '-'}</dd>
                  <dt>Seen Count</dt>
                  <dd>{dialogDetail?.seen_count ?? '-'}</dd>
                  <dt>Payload</dt>
                  <dd>{dialogDetail?.raw_tx_len ?? '-'} bytes</dd>
                </dl>
              </section>
            </div>

            <section className="news-detail-section mt-4">
              <div className="news-kicker border-b border-zinc-900 pb-1">
                Feature Engine
              </div>
              <div className="mt-3 grid grid-cols-2 gap-3 text-[18px]">
                <div>
                  <div className="news-kicker">Protocol</div>
                  <div>{dialogFeature?.protocol ?? '-'}</div>
                </div>
                <div>
                  <div className="news-kicker">Category</div>
                  <div>{dialogFeature?.category ?? '-'}</div>
                </div>
                <div>
                  <div className="news-kicker">MEV Score</div>
                  <div>{dialogFeature?.mev_score ?? '-'}</div>
                </div>
                <div>
                  <div className="news-kicker">Urgency Score</div>
                  <div>{dialogFeature?.urgency_score ?? '-'}</div>
                </div>
                <div className="col-span-2">
                  <div className="news-kicker">Method Selector</div>
                  <div className="news-mono text-[18px]">{dialogFeature?.method_selector ?? '-'}</div>
                </div>
              </div>
            </section>

            <div className="mt-4 min-h-[2.5rem]">
              {dialogError ? (
                <div className="news-mono border border-rose-900 bg-rose-100/70 px-3 py-2 text-[16px] uppercase tracking-[0.08em] text-rose-800">
                  {dialogError}
                </div>
              ) : null}
            </div>
          </div>
        </div>
  );
}
