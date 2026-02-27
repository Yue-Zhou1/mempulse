import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { MAINNET_FILTER_OPTIONS } from '../domain/mainnet-filter.js';
import {
  formatRelativeTime,
  opportunityCandidateTone,
  opportunityRowKey,
  resolveMainnetLabel,
  shortHex,
} from '../lib/dashboard-helpers.js';
import { buildVirtualizedOpportunityWindow } from '../lib/opportunity-virtualized-model.js';
import { NewspaperFilterSelect } from './newspaper-filter-select.jsx';
import { cn } from '../../../shared/lib/utils.js';

const OPPORTUNITY_ROW_HEIGHT_PX = 96;
const OPPORTUNITY_OVERSCAN_ROWS = 4;

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

  const listViewportRef = useRef(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(480);

  const onListScroll = useCallback((event) => {
    const nextScrollTop = Number(event.currentTarget?.scrollTop ?? 0);
    setScrollTop((current) => (current === nextScrollTop ? current : nextScrollTop));
  }, []);

  useEffect(() => {
    const node = listViewportRef.current;
    if (!node) {
      return undefined;
    }

    const updateViewportHeight = () => {
      const nextHeight = Math.max(
        OPPORTUNITY_ROW_HEIGHT_PX,
        Math.floor(node.clientHeight || 0),
      );
      setViewportHeight((current) => (current === nextHeight ? current : nextHeight));
    };

    updateViewportHeight();
    let resizeObserver = null;
    if (typeof ResizeObserver === 'function') {
      resizeObserver = new ResizeObserver(() => {
        updateViewportHeight();
      });
      resizeObserver.observe(node);
    } else if (typeof window?.addEventListener === 'function') {
      window.addEventListener('resize', updateViewportHeight);
    }

    return () => {
      if (resizeObserver) {
        resizeObserver.disconnect();
      } else if (typeof window?.removeEventListener === 'function') {
        window.removeEventListener('resize', updateViewportHeight);
      }
    };
  }, []);

  const virtualizedWindow = useMemo(
    () => buildVirtualizedOpportunityWindow(filteredOpportunityRows, {
      scrollTop,
      viewportHeight,
      rowHeightPx: OPPORTUNITY_ROW_HEIGHT_PX,
      overscanRows: OPPORTUNITY_OVERSCAN_ROWS,
    }),
    [filteredOpportunityRows, scrollTop, viewportHeight],
  );

  return (
          <div className="grid min-h-0 flex-1 grid-cols-1 bg-[#f7f1e6] lg:grid-cols-[420px_minmax(500px,1fr)]">
            <section className="min-h-0 border-b border-zinc-900 p-4 lg:border-b-0 lg:border-r flex flex-col">
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
              <div
                ref={listViewportRef}
                onScroll={onListScroll}
                className="min-h-0 flex-1 overflow-auto"
              >
                <ul
                  onClick={onOpportunityListClick}
                  className="relative"
                  style={{ height: `${virtualizedWindow.totalHeight}px` }}
                >
                  {virtualizedWindow.visibleRows.map((opportunity, index) => {
                    const absoluteIndex = virtualizedWindow.startIndex + index;
                    const rowTop = absoluteIndex * OPPORTUNITY_ROW_HEIGHT_PX;
                    const rowKey = opportunityRowKey(opportunity);
                    const isActive = selectedOpportunityKey === rowKey;
                    const tone = opportunityCandidateTone(opportunity, isActive);
                    const mainnet = resolveMainnetLabel(opportunity.chain_id, opportunity.source_id);
                    return (
                      <li
                        key={rowKey}
                        data-opportunity-key={rowKey}
                        className="absolute left-0 right-0 list-none pb-2"
                        style={{
                          top: `${rowTop}px`,
                          height: `${OPPORTUNITY_ROW_HEIGHT_PX}px`,
                        }}
                      >
                        <div
                          className={cn(
                            'h-full w-full cursor-pointer border p-3 text-left transition-colors',
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
                        </div>
                      </li>
                    );
                  })}
                </ul>
              </div>
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
