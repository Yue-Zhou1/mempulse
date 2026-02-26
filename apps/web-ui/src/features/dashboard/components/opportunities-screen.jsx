import { MAINNET_FILTER_OPTIONS } from '../domain/mainnet-filter.js';
import {
  formatRelativeTime,
  opportunityCandidateTone,
  opportunityRowKey,
  resolveMainnetLabel,
  shortHex,
} from '../lib/dashboard-helpers.js';
import { NewspaperFilterSelect } from './newspaper-filter-select.jsx';
import { cn } from '../../../shared/lib/utils.js';

export function OpportunitiesScreen({ model, actions }) {
  const {
    liveMainnetFilter,
    filteredOpportunityRows,
    selectedOpportunityKey,
    selectedOpportunity,
    selectedOpportunityMainnet,
  } = model;

  const {
    onLiveMainnetFilterChange,
    onOpportunityListClick,
  } = actions;

  return (
          <div className="grid min-h-0 flex-1 grid-cols-1 bg-[#f7f1e6] lg:grid-cols-[420px_minmax(500px,1fr)]">
            <section className="min-h-0 overflow-auto border-b border-zinc-900 p-4 lg:border-b-0 lg:border-r">
              <div className="news-kicker mb-3">
                Opportunity Pipeline
              </div>
              <div className="mb-3 flex items-center gap-2">
                <NewspaperFilterSelect
                  id="opps-mainnet-filter"
                  value={liveMainnetFilter}
                  onChange={onLiveMainnetFilterChange}
                  options={MAINNET_FILTER_OPTIONS}
                />
              </div>
              <ul className="space-y-2" onClick={onOpportunityListClick}>
                {filteredOpportunityRows.map((opportunity) => {
                  const rowKey = opportunityRowKey(opportunity);
                  const isActive = selectedOpportunityKey === rowKey;
                  const tone = opportunityCandidateTone(opportunity, isActive);
                  const mainnet = resolveMainnetLabel(opportunity.chain_id, opportunity.source_id);
                  return (
                    <li
                      key={rowKey}
                      data-opportunity-key={rowKey}
                      className={cn(
                        'w-full cursor-pointer list-none border p-3 text-left transition-colors',
                        tone.container,
                      )}
                    >
                      <div className="flex items-center justify-between gap-2">
                        <div className="news-headline text-sm font-semibold">{opportunity.strategy}</div>
                        <div className={cn('news-mono text-[11px] uppercase tracking-[0.1em]', tone.subtle)}>
                          score {opportunity.score}
                        </div>
                      </div>
                      <div className={cn('news-mono mt-1 text-[11px] uppercase tracking-[0.1em]', tone.subtle)}>
                        {mainnet} · {opportunity.protocol} · {opportunity.category}
                      </div>
                      <div className={cn('news-mono mt-1 text-[11px] uppercase tracking-[0.1em]', tone.subtle)}>
                        tx {shortHex(opportunity.tx_hash, 14, 10)} · {formatRelativeTime(opportunity.detected_unix_ms)}
                      </div>
                    </li>
                  );
                })}
              </ul>
              {filteredOpportunityRows.length === 0 ? (
                <div className="news-dotted-box mt-4 bg-[#fffdf7] p-6 text-sm text-zinc-700">
                  No opportunities available.
                </div>
              ) : null}
            </section>

            <section className="min-h-0 overflow-auto p-4">
              <div className="news-card p-4">
                <div className="news-kicker">
                  Selected Opportunity
                </div>
                {selectedOpportunity ? (
                  <div className="mt-3 space-y-3 text-sm">
                    <div>
                      <div className="news-kicker">Transaction</div>
                      <div className="news-mono text-[13px]">{selectedOpportunity.tx_hash}</div>
                    </div>
                    <div className="grid grid-cols-2 gap-3">
                      <div>
                        <div className="news-kicker">Strategy</div>
                        <div>{selectedOpportunity.strategy}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Score</div>
                        <div>{selectedOpportunity.score}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Protocol</div>
                        <div>{selectedOpportunity.protocol}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Mainnet</div>
                        <div>{selectedOpportunityMainnet}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Category</div>
                        <div>{selectedOpportunity.category}</div>
                      </div>
                    </div>
                    <div className="border border-zinc-900 bg-[#fffdf7] p-3">
                      <div className="news-kicker">
                        Rule Versions
                      </div>
                      <div className="news-mono mt-2 text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                        feature-engine: {selectedOpportunity.feature_engine_version}
                      </div>
                      <div className="news-mono text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                        scorer: {selectedOpportunity.scorer_version}
                      </div>
                      <div className="news-mono text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                        strategy: {selectedOpportunity.strategy_version}
                      </div>
                    </div>
                    <div>
                      <div className="news-kicker">Reasons</div>
                      <ul className="news-mono mt-1 space-y-1 text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                        {selectedOpportunity.reasons?.length
                          ? selectedOpportunity.reasons.map((reason, index) => (
                              <li key={`${reason}-${index}`} className="border border-zinc-900/40 bg-[#fffdf7] px-2 py-1">
                                {reason}
                              </li>
                            ))
                          : [<li key="none" className="border border-zinc-900/40 bg-[#fffdf7] px-2 py-1">No reasons provided.</li>]}
                      </ul>
                    </div>
                  </div>
                ) : (
                  <div className="mt-3 text-sm text-zinc-700">Select an opportunity from the left pane.</div>
                )}
              </div>
            </section>
          </div>
  );
}
