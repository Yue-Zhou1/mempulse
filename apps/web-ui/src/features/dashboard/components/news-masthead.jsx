import { cn } from '../../../shared/lib/utils.js';

export function NewsMasthead({ model, actions }) {
  const {
    editionDate,
    activeScreen,
    statusMessage,
    chainStatusBadges,
  } = model;

  const {
    onMastheadNavClick,
  } = actions;

  const tabs = [
    { id: 'radar', label: 'Front Page' },
    { id: 'opps', label: 'Opportunity Desk' },
  ];

  return (
        <header className="news-masthead px-5 pb-3 pt-4">
          <div className="news-mono mb-3 flex items-center justify-between border-b border-zinc-900 pb-2 text-[11px] uppercase tracking-[0.15em]">
            <span>Vol. 03 · Prototype Desk</span>
            <span>{editionDate}</span>
            <span>Global Edition</span>
          </div>
          <div className="relative mb-4 flex items-center justify-center border-b border-zinc-900 pb-3">
            <h1 className="news-headline text-center text-4xl font-extrabold uppercase leading-none tracking-tight md:text-6xl">
              Mempulse
            </h1>
            <div className="news-mono absolute right-0 top-1/2 hidden -translate-y-1/2 border border-zinc-900 bg-[#f7f1e6] px-2 py-1 text-[10px] uppercase tracking-[0.12em] lg:block">
              Live Wire
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2 border-y-2 border-zinc-900 py-2">
            <ul className="flex flex-wrap items-center gap-2" onClick={onMastheadNavClick}>
              {tabs.map((tab) => (
                <li
                  key={tab.id}
                  data-screen-id={tab.id}
                  className={cn(
                    'news-tab news-mono cursor-pointer list-none px-3 py-1.5 text-[11px] font-bold uppercase tracking-[0.14em] transition-colors',
                    activeScreen === tab.id ? 'news-tab-active' : '',
                  )}
                >
                  {tab.label}
                </li>
              ))}
            </ul>
            <div className="ml-auto flex min-w-[20rem] flex-1 flex-col items-end gap-1">
              <div className="news-mono w-full text-right text-[11px] uppercase tracking-[0.12em] text-zinc-700 lg:max-w-[36rem] lg:truncate">
                {statusMessage}
              </div>
              {chainStatusBadges.length ? (
                <div className="news-chain-status-strip">
                  {chainStatusBadges.map((badge) => (
                    <div
                      key={badge.key}
                      className={cn(
                        'news-chain-status-chip news-mono border px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-[0.08em]',
                        badge.tone,
                      )}
                      title={badge.title}
                    >
                      {badge.chainLabel} {badge.stateLabel} · {badge.silentToken} · ep {badge.endpointToken} · r{badge.rotations}
                    </div>
                  ))}
                </div>
              ) : null}
            </div>
          </div>
        </header>
  );
}
