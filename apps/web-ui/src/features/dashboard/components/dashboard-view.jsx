import { NewsMasthead } from './news-masthead.jsx';
import { OpportunitiesScreen } from './opportunities-screen.jsx';
import { RadarScreen } from './radar-screen.jsx';
import { ReplayScreen } from './replay-screen.jsx';
import { TransactionDialog } from './transaction-dialog.jsx';

export function DashboardView({ model, actions }) {
  const { activeScreen } = model;

  return (
    <div className="news-body min-h-screen text-zinc-900">
      <div className="newspaper-shell flex h-screen w-screen max-w-none flex-col overflow-hidden">
        <NewsMasthead model={model} actions={actions} />

        {activeScreen === 'radar' ? <RadarScreen model={model} actions={actions} /> : null}
        {activeScreen === 'opps' ? <OpportunitiesScreen model={model} actions={actions} /> : null}
        {activeScreen === 'replay' ? <ReplayScreen model={model} actions={actions} /> : null}
      </div>

      <TransactionDialog model={model} actions={actions} />
    </div>
  );
}
