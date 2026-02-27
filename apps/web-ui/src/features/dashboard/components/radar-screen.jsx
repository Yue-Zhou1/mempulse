import { MAINNET_FILTER_ALL, MAINNET_FILTER_OPTIONS } from '../domain/mainnet-filter.js';
import {
  classifyRisk,
  formatTickerTime,
  resolveMainnetLabel,
  resolveMainnetRowClasses,
  riskBadgeClass,
  shortHex,
  statusBadgeClass,
  statusForRow,
} from '../lib/dashboard-helpers.js';
import { rowsFromVirtualizedModels } from '../lib/radar-virtualized-model.js';
import { NewspaperFilterSelect } from './newspaper-filter-select.jsx';
import { RadarVirtualizedTable } from './radar-virtualized-table.jsx';
import { RollingInt, RollingPercent } from './rolling-number.jsx';
import { cn } from '../../../shared/lib/utils.js';

export function RadarScreen({ model, actions }) {
  const {
    showTickerFilters,
    liveMainnetFilter,
    query,
    hasError,
    statusMessage,
    virtualizedTickerEnabled,
    virtualizedTickerRows,
    featureByHash,
    selectedHash,
    filteredTransactions,
    transactionPageStart,
    transactionPageEnd,
    pagedTransactions,
    normalizedTransactionPage,
    transactionPageCount,
    paginationPages,
    selectedTransaction,
    selectedFeature,
    selectedRecent,
    selectedTransactionMainnet,
    totalSignalVolume,
    successRate,
    totalTxCount,
    highRiskCount,
    featureTrendPath,
    lowRiskCount,
    mediumRiskCount,
    topMixRows,
    mixTotal,
  } = model;

  const {
    onTickerToolbarClick,
    onSearchChange,
    onLiveMainnetFilterChange,
    onTickerListClick,
    onTransactionPaginationClick,
  } = actions;
  const nonVirtualizedTickerRows = rowsFromVirtualizedModels(virtualizedTickerRows);

  return (
          <div className="grid min-h-0 flex-1 grid-cols-1 gap-6 overflow-hidden bg-[#f7f1e6] p-4 lg:grid-cols-[minmax(0,1fr)_minmax(360px,420px)] xl:grid-cols-[minmax(0,1fr)_460px]">
            <main className="flex min-h-0 flex-col overflow-hidden lg:border-r-2 lg:border-zinc-900 lg:pr-6">
              <div className="mb-4 text-center">
                <div className="news-mono mt-1 flex items-center justify-center gap-2 text-[11px] uppercase tracking-[0.12em] text-zinc-700">
                  <span className="inline-block size-2 animate-pulse rounded-full bg-zinc-900" />
                  <span>Live Wire Service • Updates Continuously</span>
                </div>
              </div>

              <div className="mb-1 flex flex-wrap items-center justify-between gap-2 border-b-4 border-zinc-900 pb-2">
                <div className="news-section-title text-xl font-bold uppercase">Latest Ticker</div>
                <ul className="flex gap-2" onClick={onTickerToolbarClick}>
                  <li
                    data-ticker-action="filter"
                    className={cn(
                      'news-tab news-mono cursor-pointer list-none px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] transition-colors',
                      showTickerFilters || liveMainnetFilter !== MAINNET_FILTER_ALL ? 'news-tab-active' : '',
                    )}
                  >
                    Filter
                  </li>
                  <li
                    data-ticker-action="follow"
                    className="news-tab news-mono cursor-pointer list-none px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] transition-colors"
                  >
                    Follow
                  </li>
                </ul>
              </div>

              <div className="mb-2 flex items-center gap-2">
                <input
                  type="search"
                  value={query}
                  onChange={onSearchChange}
                  placeholder="Search hash, sender, source, mainnet, protocol, category"
                  className="news-mono w-full border border-zinc-900 bg-[#fffdf7] px-3 py-2 text-sm outline-none transition-colors focus:bg-white"
                />
              </div>
              {showTickerFilters ? (
                <div className="mb-2 flex flex-wrap items-center gap-2 border border-zinc-900 bg-[#fffdf7] px-3 py-2">
                  <NewspaperFilterSelect
                    id="live-mainnet-filter"
                    value={liveMainnetFilter}
                    onChange={onLiveMainnetFilterChange}
                    options={MAINNET_FILTER_OPTIONS}
                  />
                </div>
              ) : null}

              <div
                className={cn(
                  'news-mono mb-3 min-h-[1rem] overflow-hidden text-ellipsis whitespace-nowrap text-[10px] uppercase tracking-[0.12em]',
                  hasError ? 'text-rose-700' : 'text-zinc-700',
                )}
              >
                {statusMessage}
              </div>

              <div className="news-list-shell min-h-0 flex flex-1 flex-col overflow-hidden">
                <div onClick={onTickerListClick} className="min-h-0 h-full">
                  {virtualizedTickerEnabled ? (
                    <RadarVirtualizedTable
                      rowModels={virtualizedTickerRows}
                      featureByHash={featureByHash}
                      selectedHash={selectedHash}
                    />
                  ) : (
                    <div className="news-list-scroll h-full border-b-2 border-zinc-900">
                      <table className="news-tx-table news-mono min-w-[1360px] w-full table-fixed border-collapse">
                        <thead className="news-tx-head text-[13px] font-bold uppercase tracking-[0.1em]">
                          <tr>
                            <th scope="col" className="news-tx-header-cell w-[170px] px-2 py-2 text-left">Timestamp</th>
                            <th scope="col" className="news-tx-header-cell w-[170px] px-2 py-2 text-left">Tx Hash</th>
                            <th scope="col" className="news-tx-header-cell w-[170px] px-2 py-2 text-left">Sender</th>
                            <th scope="col" className="news-tx-header-cell w-[92px] px-2 py-2 text-left">Source</th>
                            <th scope="col" className="news-tx-header-cell w-[92px] px-2 py-2 text-left">Mainnet</th>
                            <th scope="col" className="news-tx-header-cell w-[66px] px-2 py-2 text-left">Type</th>
                            <th scope="col" className="news-tx-header-cell w-[72px] px-2 py-2 text-left">Nonce</th>
                            <th scope="col" className="news-tx-header-cell w-[102px] px-2 py-2 text-left">Protocol</th>
                            <th scope="col" className="news-tx-header-cell w-[100px] px-2 py-2 text-left">Category</th>
                            <th scope="col" className="news-tx-header-cell w-[76px] px-2 py-2 text-right">Amt.</th>
                            <th scope="col" className="news-tx-header-cell w-[104px] px-2 py-2 text-left">Status</th>
                            <th scope="col" className="news-tx-header-cell w-[74px] px-2 py-2 text-left">Risk</th>
                            <th scope="col" className="news-tx-header-cell w-[36px] px-2 py-2 text-center">Op.</th>
                          </tr>
                        </thead>
                        <tbody>
                          {nonVirtualizedTickerRows.map((row) => {
                            const feature = featureByHash.get(row.hash);
                            const status = statusForRow(feature);
                            const risk = classifyRisk(feature);
                            const mainnetValue = resolveMainnetLabel(row.chain_id, row.source_id);
                            const mainnetRowClasses = resolveMainnetRowClasses(mainnetValue);
                            const isActive = row.hash === selectedHash;
                            const amountValue = feature ? (feature.urgency_score / 10).toFixed(1) : '--.--';
                            return (
                              <tr
                                key={row.hash}
                                data-tx-hash={row.hash}
                                className={cn(
                                  'news-tx-row cursor-pointer border-b border-dashed border-zinc-900 text-[13px]',
                                  isActive
                                    ? 'bg-zinc-900 text-[#f7f1e6]'
                                    : status === 'Flagged'
                                      ? 'bg-zinc-900 text-[#f7f1e6]'
                                      : `${mainnetRowClasses} text-zinc-800`,
                                )}
                              >
                                <td className="news-tx-cell px-2 py-2 align-middle whitespace-nowrap">{formatTickerTime(row.seen_unix_ms)}</td>
                                <td className="news-tx-cell px-2 py-2 align-middle overflow-hidden text-ellipsis whitespace-nowrap" title={row.hash}>{shortHex(row.hash, 18, 8)}</td>
                                <td className="news-tx-cell px-2 py-2 align-middle overflow-hidden text-ellipsis whitespace-nowrap" title={row.sender ?? '-'}>{shortHex(row.sender ?? '-', 10, 8)}</td>
                                <td className="news-tx-cell px-2 py-2 align-middle overflow-hidden text-ellipsis whitespace-nowrap">{row.source_id ?? '-'}</td>
                                <td className="news-tx-cell px-2 py-2 align-middle overflow-hidden text-ellipsis whitespace-nowrap">{mainnetValue}</td>
                                <td className="news-tx-cell px-2 py-2 align-middle">{row.tx_type ?? '-'}</td>
                                <td className="news-tx-cell px-2 py-2 align-middle">{row.nonce ?? '-'}</td>
                                <td className="news-tx-cell px-2 py-2 align-middle overflow-hidden text-ellipsis whitespace-nowrap">{feature?.protocol ?? '-'}</td>
                                <td className="news-tx-cell px-2 py-2 align-middle overflow-hidden text-ellipsis whitespace-nowrap">{feature?.category ?? '-'}</td>
                                <td className="news-tx-cell px-2 py-2 text-right align-middle">{amountValue}</td>
                                <td className="news-tx-cell px-2 py-2 align-middle">
                                  <span
                                    className={cn(
                                      'border px-1 text-[12px] font-bold uppercase tracking-[0.08em]',
                                      statusBadgeClass(status, isActive || status === 'Flagged'),
                                    )}
                                  >
                                    {status}
                                  </span>
                                </td>
                                <td className="news-tx-cell px-2 py-2 align-middle">
                                  <span
                                    className={cn(
                                      'border px-1 text-[12px] font-bold uppercase tracking-[0.08em]',
                                      riskBadgeClass(risk.label, isActive || status === 'Flagged'),
                                    )}
                                  >
                                    {risk.label}
                                  </span>
                                </td>
                                <td className={cn('news-tx-cell px-2 py-2 text-center align-middle font-bold', isActive ? 'text-[#f7f1e6]' : 'text-zinc-500')}>
                                  •••
                                </td>
                              </tr>
                            );
                          })}
                        </tbody>
                      </table>
                    </div>
                  )}
                </div>
              </div>

              {filteredTransactions.length > 0 ? (
                <div className="mt-3 border-t-2 border-zinc-900 pt-2">
                  <div className="news-mono mb-2 text-center text-[10px] uppercase tracking-[0.14em] text-zinc-700">
                    Showing {transactionPageStart + 1}-{transactionPageEnd} of {pagedTransactions.length} rows · page {normalizedTransactionPage}/{transactionPageCount}
                  </div>
                  <ul className="flex items-center justify-center gap-1.5" onClick={onTransactionPaginationClick}>
                    <li
                      data-page-action="prev"
                      data-disabled={normalizedTransactionPage <= 1 ? 'true' : 'false'}
                      className={cn(
                        'news-tab news-mono list-none px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] transition-colors',
                        normalizedTransactionPage <= 1 ? 'news-tab-disabled' : 'cursor-pointer',
                      )}
                    >
                      Prev
                    </li>
                    {paginationPages.map((page) => (
                      <li
                        key={page}
                        data-page={page}
                        className={cn(
                          'news-tab news-mono cursor-pointer list-none px-2.5 py-1 text-[10px] font-bold uppercase tracking-[0.12em] transition-colors',
                          page === normalizedTransactionPage ? 'news-tab-active' : '',
                        )}
                      >
                        {page}
                      </li>
                    ))}
                    <li
                      data-page-action="next"
                      data-disabled={normalizedTransactionPage >= transactionPageCount ? 'true' : 'false'}
                      className={cn(
                        'news-tab news-mono list-none px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] transition-colors',
                        normalizedTransactionPage >= transactionPageCount ? 'news-tab-disabled' : 'cursor-pointer',
                      )}
                    >
                      Next
                    </li>
                  </ul>
                </div>
              ) : null}

              {filteredTransactions.length === 0 ? (
                <div className="news-dotted-box mt-4 bg-[#fffdf7] p-8 text-center text-sm text-zinc-700">
                  No rows match your search.
                </div>
              ) : null}
            </main>

            <aside className="grid min-h-0 min-w-0 grid-rows-[30fr_45fr_25fr] gap-3 overflow-hidden pr-0.5">
              <div className="news-dotted-box relative flex h-full min-h-0 flex-col overflow-hidden bg-transparent p-4">
                <div className="absolute -left-1 -top-1 h-3 w-3 border-l-2 border-t-2 border-zinc-900" />
                <div className="absolute -right-1 -top-1 h-3 w-3 border-r-2 border-t-2 border-zinc-900" />
                <div className="absolute -bottom-1 -left-1 h-3 w-3 border-b-2 border-l-2 border-zinc-900" />
                <div className="absolute -bottom-1 -right-1 h-3 w-3 border-b-2 border-r-2 border-zinc-900" />
                <h3 className="news-headline border-b-2 border-zinc-900 pb-2 text-center text-xl font-black uppercase tracking-tight xl:text-2xl">
                  Market Statistics
                </h3>

                <div className="mt-3 flex min-h-0 flex-1 flex-col justify-between gap-3 overflow-y-auto pr-1 [scrollbar-gutter:stable]">
                  <div className="text-center">
                    <p className="news-kicker mb-1">Total Signal Volume</p>
                    <p className="news-headline text-3xl font-bold">
                      <RollingInt value={totalSignalVolume} durationMs={650} />
                    </p>
                    <p className="news-mono mt-1 inline-block border-t border-zinc-900 px-2 pt-1 text-[11px] uppercase tracking-[0.12em]">
                      <RollingPercent value={successRate} durationMs={460} suffix="% stable stream" />
                    </p>
                  </div>

                  <div className="news-double-divider" />

                  <div className="grid grid-cols-2 gap-2">
                    <div className="text-center">
                      <p className="news-kicker mb-1">Tx Count</p>
                      <p className="news-headline text-2xl font-bold">
                        <RollingInt value={totalTxCount} durationMs={500} />
                      </p>
                      <p className="news-mono mt-1 text-[10px] uppercase tracking-[0.12em]">
                        tx stream volume
                      </p>
                    </div>
                    <div className="border-l border-dashed border-zinc-900 pl-2 text-center">
                      <p className="news-kicker mb-1">Success Rate</p>
                      <p className="news-headline text-2xl font-bold">
                        <RollingPercent value={successRate} durationMs={460} />
                      </p>
                      <p className="news-mono mt-1 text-[10px] uppercase tracking-[0.12em]">
                        {highRiskCount} high risk rows
                      </p>
                    </div>
                  </div>
                </div>
              </div>

              <div className="news-dotted-box min-h-0 flex h-full flex-col overflow-y-auto bg-transparent p-3 [scrollbar-gutter:stable]">
                <h3 className="news-headline mb-2 border-b border-zinc-900 pb-1 text-lg font-bold uppercase xl:text-xl">
                  Feature Engine Report
                </h3>

                <div className="relative mb-2 flex min-h-0 flex-[0_0_48%] flex-col border border-zinc-900 bg-[#fffdf7] p-2">
                  <div className="news-mono absolute right-0 top-0 bg-zinc-900 px-1 text-[10px] uppercase tracking-[0.12em] text-[#f7f1e6]">
                    Fig. 1A
                  </div>
                  <div className="w-full min-h-0 flex-1 pt-2">
                    <svg viewBox="0 0 100 40" className="h-full w-full overflow-visible" preserveAspectRatio="none">
                      <line x1="0" y1="10" x2="100" y2="10" stroke="#c4c4c4" strokeWidth="0.5" strokeDasharray="2 1" />
                      <line x1="0" y1="20" x2="100" y2="20" stroke="#c4c4c4" strokeWidth="0.5" strokeDasharray="2 1" />
                      <line x1="0" y1="30" x2="100" y2="30" stroke="#c4c4c4" strokeWidth="0.5" strokeDasharray="2 1" />
                      <path d={featureTrendPath} fill="none" stroke="#111827" strokeWidth="1.6" vectorEffect="non-scaling-stroke" />
                    </svg>
                  </div>
                  <p className="news-mono mt-2 text-center text-[10px] uppercase tracking-[0.1em] text-zinc-700">
                    Fig 1. Feature mix pressure trend.
                  </p>
                </div>

                <ul className="space-y-1 text-[13px]">
                  <li className="flex items-center justify-between gap-2 border-b border-dashed border-zinc-500 pb-1">
                    <span className="inline-flex min-w-0 items-center gap-2">
                      <span
                        className={cn(
                          'news-mono border px-1 text-[10px] font-bold uppercase tracking-[0.1em]',
                          riskBadgeClass('Low'),
                        )}
                      >
                        Low
                      </span>
                      <span className="text-[13px] leading-tight">Risk (velocity normal)</span>
                    </span>
                    <span
                      className={cn(
                        'news-mono border px-1 text-[11px] font-bold uppercase tracking-[0.1em]',
                        riskBadgeClass('Low'),
                      )}
                    >
                      {lowRiskCount}
                    </span>
                  </li>
                  <li className="flex items-center justify-between gap-2 border-b border-dashed border-zinc-500 pb-1">
                    <span className="inline-flex min-w-0 items-center gap-2">
                      <span
                        className={cn(
                          'news-mono border px-1 text-[10px] font-bold uppercase tracking-[0.1em]',
                          riskBadgeClass('Medium'),
                        )}
                      >
                        Medium
                      </span>
                      <span className="text-[13px] leading-tight">Risk (pattern drift)</span>
                    </span>
                    <span
                      className={cn(
                        'news-mono border px-1 text-[11px] font-bold uppercase tracking-[0.1em]',
                        riskBadgeClass('Medium'),
                      )}
                    >
                      {mediumRiskCount}
                    </span>
                  </li>
                  <li className="flex items-center justify-between gap-2 pb-1 font-bold">
                    <span className="inline-flex min-w-0 items-center gap-2">
                      <span
                        className={cn(
                          'news-mono border px-1 text-[10px] font-bold uppercase tracking-[0.1em]',
                          riskBadgeClass('High'),
                        )}
                      >
                        High
                      </span>
                      <span className="text-[13px] leading-tight">Risk (opportunity spike)</span>
                    </span>
                    <span
                      className={cn(
                        'news-mono border px-1 text-[11px] font-bold uppercase tracking-[0.1em]',
                        riskBadgeClass('High'),
                      )}
                    >
                      {highRiskCount}
                    </span>
                  </li>
                </ul>

                <div className="news-mono mt-2 border-t border-zinc-900 pt-1 text-[9px] uppercase tracking-[0.1em] text-zinc-700">
                  {topMixRows.map((row) => {
                    const pct = mixTotal ? Math.round((row.count / mixTotal) * 100) : 0;
                    return (
                      <div key={`${row.protocol}-${row.category}`} className="flex justify-between py-0.5">
                        <span>{row.protocol} / {row.category}</span>
                        <span>{pct}%</span>
                      </div>
                    );
                  })}
                </div>
              </div>

              <div className="border-2 border-zinc-900 bg-transparent p-3 h-full min-h-0 flex flex-col overflow-y-auto [scrollbar-gutter:stable]">
                <h4 className="news-headline mb-1 text-center text-base font-bold uppercase">
                  Selected Brief
                </h4>
                <div className="news-mono text-xs uppercase tracking-[0.1em]">
                  <div className="grid grid-cols-2 gap-x-4 gap-y-1.5">
                    <div className="col-span-2">
                      <span className="block text-zinc-500">Tx</span>
                      <span className="block truncate">{selectedTransaction ? shortHex(selectedTransaction.hash, 18, 8) : '-'}</span>
                    </div>
                    <div>
                      <span className="block text-zinc-500">Protocol</span>
                      <span className="block">{selectedFeature?.protocol ?? '-'}</span>
                    </div>
                    <div>
                      <span className="block text-zinc-500">Category</span>
                      <span className="block">{selectedFeature?.category ?? '-'}</span>
                    </div>
                    <div>
                      <span className="block text-zinc-500">Mainnet</span>
                      <span className="block">{selectedTransactionMainnet}</span>
                    </div>
                    <div>
                      <span className="block text-zinc-500">Source</span>
                      <span className="block">{selectedRecent?.source_id ?? selectedTransaction?.source_id ?? '-'}</span>
                    </div>
                  </div>
                  <div className="mt-1.5 grid grid-cols-2 gap-1.5 border-t border-zinc-900 pt-1.5">
                    <div>
                      <span className="block text-zinc-500">Mev</span>
                      <span className="text-base font-bold">{selectedFeature?.mev_score ?? '-'}</span>
                    </div>
                    <div>
                      <span className="block text-zinc-500">Urgency</span>
                      <span className="text-base font-bold">{selectedFeature?.urgency_score ?? '-'}</span>
                    </div>
                  </div>
                  <div className="mt-1.5 border-t border-zinc-900 pt-1.5 text-[10px]">
                    {selectedRecent
                      ? `Source ${selectedRecent.source_id} observed type ${selectedRecent.tx_type}.`
                      : 'Select a row to inspect metadata.'}
                  </div>
                </div>
              </div>
            </aside>
          </div>
  );
}
