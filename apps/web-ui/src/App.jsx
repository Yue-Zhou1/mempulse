import { DashboardView, useDashboardController } from './features/dashboard/index.js';

export default function App() {
  const { model, actions } = useDashboardController();
  return <DashboardView model={model} actions={actions} />;
}
