import { MAINNET_FILTER_OPTIONS } from '../domain/mainnet-filter.js';
import {
  formatRelativeTime,
  formatTime,
  opportunityCandidateTone,
  opportunityRowKey,
  resolveMainnetLabel,
  resolveMainnetRowClasses,
  shortHex,
} from '../lib/dashboard-helpers.js';
import { NewspaperFilterSelect } from './newspaper-filter-select.jsx';
import { cn } from '../../../shared/lib/utils.js';

export function ReplayScreen({ model, actions }) {
  const {
    archiveLoading,
    archiveQuery,
    archiveMainnetFilter,
    filteredArchiveTxRows,
    archiveTxRows,
    filteredArchiveOppRows,
    archiveOppRows,
    archiveError,
    archiveTxPageRows,
    selectedArchiveTxHash,
    normalizedArchiveTxPage,
    archiveTxPages,
    archiveTxPageCount,
    archiveOppPageRows,
    selectedArchiveOppKey,
    normalizedArchiveOppPage,
    archiveOppPages,
    archiveOppPageCount,
    selectedArchiveTx,
    selectedArchiveTxMainnet,
    selectedArchiveOpp,
    selectedArchiveOppMainnet,
  } = model;

  const {
    onArchiveRefreshClick,
    onArchiveQueryChange,
    onArchiveMainnetFilterChange,
    onArchiveTxListClick,
    onArchiveTxPaginationClick,
    onArchiveOppListClick,
    onArchiveOppPaginationClick,
    onInspectArchiveTxClick,
  } = actions;

  return (
          <div className="grid min-h-0 flex-1 grid-cols-1 gap-4 bg-[#f7f1e6] p-4 lg:grid-cols-[minmax(0,1.25fr)_minmax(360px,1fr)]">
            <section className="flex h-full min-h-0 flex-col overflow-hidden border border-zinc-900 bg-[#fffdf7] p-4">
              <div className="flex flex-wrap items-end justify-between gap-3 border-b-2 border-zinc-900 pb-2">
                <div>
                  <div className="news-kicker">Archive Desk</div>
                  <h2 className="news-headline text-2xl font-bold">Historical Records</h2>
                </div>
                <div
                  data-archive-refresh="true"
                  data-disabled={archiveLoading ? 'true' : 'false'}
                  onClick={onArchiveRefreshClick}
                  className={cn(
                    'news-tab news-mono list-none px-3 py-1.5 text-[11px] font-bold uppercase tracking-[0.14em] transition-colors',
                    archiveLoading ? 'news-tab-disabled' : 'cursor-pointer',
                  )}
                >
                  {archiveLoading ? 'Syncing' : 'Refresh'}
                </div>
              </div>

              <div className="mt-3 flex flex-wrap items-center gap-2">
                <input
                  value={archiveQuery}
                  onChange={onArchiveQueryChange}
                  placeholder="Search hash, sender, mainnet, protocol, category, status"
                  className="news-mono min-w-[18rem] flex-1 border border-zinc-900 bg-[#fffdf7] px-3 py-2 text-[12px] uppercase tracking-[0.08em] outline-none focus:bg-white"
                />
                <NewspaperFilterSelect
                  id="archive-mainnet-filter"
                  value={archiveMainnetFilter}
                  onChange={onArchiveMainnetFilterChange}
                  options={MAINNET_FILTER_OPTIONS}
                  compact
                  ariaLabel="Archive mainnet filter"
                />
                <div className="news-mono text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                  tx:{filteredArchiveTxRows.length}/{archiveTxRows.length} · opps:{filteredArchiveOppRows.length}/{archiveOppRows.length}
                </div>
              </div>

              {archiveError ? (
                <div className="news-mono mt-2 border border-rose-900 bg-rose-100/70 px-3 py-2 text-[11px] uppercase tracking-[0.1em] text-rose-800">
                  {archiveError}
                </div>
              ) : null}

              <div className="mt-3 grid min-h-0 flex-1 grid-rows-[minmax(0,1fr)_minmax(0,1fr)] gap-3 overflow-hidden">
                <div className="news-card min-h-0 flex flex-col p-3">
                  <div className="mb-2 flex items-center justify-between border-b border-zinc-900 pb-1">
                    <div className="news-kicker">Historical Transactions</div>
                    <div className="news-mono text-[10px] uppercase tracking-[0.1em] text-zinc-700">
                      {filteredArchiveTxRows.length} rows
                    </div>
                  </div>
                  <div className="news-list-scroll min-h-0 flex-1 space-y-1" onClick={onArchiveTxListClick}>
                    {archiveTxPageRows.map((row) => {
                      const isActive = selectedArchiveTxHash === row.hash;
                      const lifecycle = row.lifecycle_status ?? 'pending';
                      const mainnet = resolveMainnetLabel(row.chain_id, row.source_id);
                      const mainnetRowClasses = resolveMainnetRowClasses(mainnet);
                      return (
                        <div
                          key={row.hash}
                          data-archive-tx-hash={row.hash}
                          className={cn(
                            'w-full cursor-pointer border px-3 py-2 text-left transition-colors',
                            isActive
                              ? 'border-zinc-900 bg-zinc-900 text-[#f7f1e6]'
                              : `border-zinc-900 ${mainnetRowClasses} text-zinc-900`,
                          )}
                        >
                          <div className="news-mono text-[11px] uppercase tracking-[0.1em]">
                            {shortHex(row.hash, 16, 10)}
                          </div>
                          <div className={cn('news-mono mt-1 text-[10px] uppercase tracking-[0.1em]', isActive ? 'text-zinc-300' : 'text-zinc-700')}>
                            {formatTime(row.first_seen_unix_ms)} · {mainnet} · {row.protocol ?? 'unknown'} / {row.category ?? 'pending'}
                          </div>
                          <div className={cn('news-mono mt-1 text-[10px] uppercase tracking-[0.1em]', isActive ? 'text-zinc-300' : 'text-zinc-700')}>
                            {lifecycle} · mev {row.mev_score ?? '-'} · urgency {row.urgency_score ?? '-'}
                          </div>
                        </div>
                      );
                    })}
                    {archiveTxPageRows.length === 0 ? (
                      <div className="news-mono border border-zinc-900 bg-[#fffdf7] px-3 py-4 text-center text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                        No archived transactions found.
                      </div>
                    ) : null}
                  </div>
                  <div className="mt-2 flex items-center justify-between border-t border-zinc-900 pt-2">
                    <ul className="flex items-center gap-1" onClick={onArchiveTxPaginationClick}>
                      <li
                        data-archive-tx-page-action="prev"
                        data-disabled={normalizedArchiveTxPage <= 1 ? 'true' : 'false'}
                        className={cn(
                          'news-tab news-mono list-none px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em]',
                          normalizedArchiveTxPage <= 1 ? 'news-tab-disabled' : 'cursor-pointer',
                        )}
                      >
                        Prev
                      </li>
                      {archiveTxPages.map((page) => (
                        <li
                          key={`archive-tx-page-${page}`}
                          data-archive-tx-page={page}
                          className={cn(
                            'news-tab news-mono cursor-pointer list-none px-2.5 py-1 text-[10px] font-bold uppercase tracking-[0.12em]',
                            page === normalizedArchiveTxPage ? 'news-tab-active' : '',
                          )}
                        >
                          {page}
                        </li>
                      ))}
                      <li
                        data-archive-tx-page-action="next"
                        data-disabled={normalizedArchiveTxPage >= archiveTxPageCount ? 'true' : 'false'}
                        className={cn(
                          'news-tab news-mono list-none px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em]',
                          normalizedArchiveTxPage >= archiveTxPageCount ? 'news-tab-disabled' : 'cursor-pointer',
                        )}
                      >
                      Next
                      </li>
                    </ul>
                  </div>
                </div>

                <div className="news-card min-h-0 flex flex-col p-3">
                  <div className="mb-2 flex items-center justify-between border-b border-zinc-900 pb-1">
                    <div className="news-kicker">Historical Opportunities</div>
                    <div className="news-mono text-[10px] uppercase tracking-[0.1em] text-zinc-700">
                      {filteredArchiveOppRows.length} rows
                    </div>
                  </div>
                  <div className="news-list-scroll min-h-0 flex-1 space-y-1" onClick={onArchiveOppListClick}>
                    {archiveOppPageRows.map((row) => {
                      const rowKey = opportunityRowKey(row);
                      const isActive = selectedArchiveOppKey === rowKey;
                      const tone = opportunityCandidateTone(row, isActive);
                      const mainnet = resolveMainnetLabel(row.chain_id, row.source_id);
                      return (
                        <div
                          key={rowKey}
                          data-archive-opp-key={rowKey}
                          className={cn('w-full cursor-pointer border px-3 py-2 text-left transition-colors', tone.container)}
                        >
                          <div className="flex items-center justify-between gap-2">
                            <div className="news-headline text-sm font-semibold">{row.strategy}</div>
                            <div className={cn('news-mono text-[10px] uppercase tracking-[0.1em]', tone.subtle)}>
                              score {row.score}
                            </div>
                          </div>
                          <div className={cn('news-mono mt-1 text-[10px] uppercase tracking-[0.1em]', tone.subtle)}>
                            {mainnet} · {row.protocol} · {row.category} · {row.status}
                          </div>
                          <div className={cn('news-mono mt-1 text-[10px] uppercase tracking-[0.1em]', tone.subtle)}>
                            tx {shortHex(row.tx_hash, 14, 10)} · {formatRelativeTime(row.detected_unix_ms)}
                          </div>
                        </div>
                      );
                    })}
                    {archiveOppPageRows.length === 0 ? (
                      <div className="news-mono border border-zinc-900 bg-[#fffdf7] px-3 py-4 text-center text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                        No archived opportunities found.
                      </div>
                    ) : null}
                  </div>
                  <div className="mt-2 flex items-center justify-between border-t border-zinc-900 pt-2">
                    <ul className="flex items-center gap-1" onClick={onArchiveOppPaginationClick}>
                      <li
                        data-archive-opp-page-action="prev"
                        data-disabled={normalizedArchiveOppPage <= 1 ? 'true' : 'false'}
                        className={cn(
                          'news-tab news-mono list-none px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em]',
                          normalizedArchiveOppPage <= 1 ? 'news-tab-disabled' : 'cursor-pointer',
                        )}
                      >
                        Prev
                      </li>
                      {archiveOppPages.map((page) => (
                        <li
                          key={`archive-opp-page-${page}`}
                          data-archive-opp-page={page}
                          className={cn(
                            'news-tab news-mono cursor-pointer list-none px-2.5 py-1 text-[10px] font-bold uppercase tracking-[0.12em]',
                            page === normalizedArchiveOppPage ? 'news-tab-active' : '',
                          )}
                        >
                          {page}
                        </li>
                      ))}
                      <li
                        data-archive-opp-page-action="next"
                        data-disabled={normalizedArchiveOppPage >= archiveOppPageCount ? 'true' : 'false'}
                        className={cn(
                          'news-tab news-mono list-none px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em]',
                          normalizedArchiveOppPage >= archiveOppPageCount ? 'news-tab-disabled' : 'cursor-pointer',
                        )}
                      >
                      Next
                      </li>
                    </ul>
                  </div>
                </div>
              </div>
            </section>

            <section className="min-h-0 overflow-auto space-y-3">
              <div className="news-card p-4">
                <div className="news-kicker">Selected Transaction Record</div>
                {selectedArchiveTx ? (
                  <div className="mt-3 space-y-3 text-sm">
                    <div>
                      <div className="news-kicker">Hash</div>
                      <div className="news-mono text-[13px]">{selectedArchiveTx.hash}</div>
                    </div>
                    <div className="grid grid-cols-2 gap-3">
                      <div>
                        <div className="news-kicker">Seen At</div>
                        <div>{formatTime(selectedArchiveTx.first_seen_unix_ms)}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Status</div>
                        <div>{selectedArchiveTx.lifecycle_status ?? 'pending'}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Protocol</div>
                        <div>{selectedArchiveTx.protocol ?? 'unknown'}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Mainnet</div>
                        <div>{selectedArchiveTxMainnet}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Category</div>
                        <div>{selectedArchiveTx.category ?? 'pending'}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Sender</div>
                        <div className="news-mono text-[12px]">{selectedArchiveTx.sender ?? '-'}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Peer</div>
                        <div>{selectedArchiveTx.peer ?? '-'}</div>
                      </div>
                    </div>
                    <div
                      data-inspect-hash={selectedArchiveTx.hash}
                      onClick={onInspectArchiveTxClick}
                      className="news-tab news-mono inline-flex cursor-pointer px-3 py-1.5 text-[11px] font-bold uppercase tracking-[0.12em]"
                    >
                      Inspect Tx Detail
                    </div>
                  </div>
                ) : (
                  <div className="mt-3 text-sm text-zinc-700">Select an archived transaction.</div>
                )}
              </div>

              <div className="news-card p-4">
                <div className="news-kicker">Selected Opportunity Record</div>
                {selectedArchiveOpp ? (
                  <div className="mt-3 space-y-3 text-sm">
                    <div>
                      <div className="news-kicker">Transaction</div>
                      <div className="news-mono text-[13px]">{selectedArchiveOpp.tx_hash}</div>
                    </div>
                    <div className="grid grid-cols-2 gap-3">
                      <div>
                        <div className="news-kicker">Strategy</div>
                        <div>{selectedArchiveOpp.strategy}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Score</div>
                        <div>{selectedArchiveOpp.score}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Protocol</div>
                        <div>{selectedArchiveOpp.protocol}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Mainnet</div>
                        <div>{selectedArchiveOppMainnet}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Category</div>
                        <div>{selectedArchiveOpp.category}</div>
                      </div>
                    </div>
                    <div>
                      <div className="news-kicker">Reasons</div>
                      <ul className="news-mono mt-1 space-y-1 text-[10px] uppercase tracking-[0.1em] text-zinc-700">
                        {(selectedArchiveOpp.reasons?.length
                          ? selectedArchiveOpp.reasons
                          : ['No reasons provided.']).map((reason, index) => (
                          <li key={`${reason}-${index}`} className="border border-zinc-900/40 bg-[#fffdf7] px-2 py-1">
                            {reason}
                          </li>
                        ))}
                      </ul>
                    </div>
                  </div>
                ) : (
                  <div className="mt-3 text-sm text-zinc-700">Select an archived opportunity.</div>
                )}
              </div>
            </section>
          </div>
  );
}
