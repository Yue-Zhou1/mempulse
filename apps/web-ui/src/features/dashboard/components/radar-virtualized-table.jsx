import { memo } from 'react';
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
import { cn } from '../../../shared/lib/utils.js';

const HEADER_CELLS = [
  ['Timestamp', 'news-tx-header-cell w-[170px] px-2 py-2 text-left'],
  ['Tx Hash', 'news-tx-header-cell w-[170px] px-2 py-2 text-left'],
  ['Sender', 'news-tx-header-cell w-[170px] px-2 py-2 text-left'],
  ['Source', 'news-tx-header-cell w-[92px] px-2 py-2 text-left'],
  ['Mainnet', 'news-tx-header-cell w-[92px] px-2 py-2 text-left'],
  ['Type', 'news-tx-header-cell w-[66px] px-2 py-2 text-left'],
  ['Nonce', 'news-tx-header-cell w-[72px] px-2 py-2 text-left'],
  ['Protocol', 'news-tx-header-cell w-[102px] px-2 py-2 text-left'],
  ['Category', 'news-tx-header-cell w-[100px] px-2 py-2 text-left'],
  ['Amt.', 'news-tx-header-cell w-[76px] px-2 py-2 text-right'],
  ['Status', 'news-tx-header-cell w-[104px] px-2 py-2 text-left'],
  ['Risk', 'news-tx-header-cell w-[74px] px-2 py-2 text-left'],
  ['Op.', 'news-tx-header-cell w-[36px] px-2 py-2 text-center'],
];

function EmptyRowCells() {
  return (
    <>
      <td className="news-tx-cell px-2 py-2 align-middle whitespace-nowrap">-</td>
      <td className="news-tx-cell px-2 py-2 align-middle whitespace-nowrap">-</td>
      <td className="news-tx-cell px-2 py-2 align-middle whitespace-nowrap">-</td>
      <td className="news-tx-cell px-2 py-2 align-middle whitespace-nowrap">-</td>
      <td className="news-tx-cell px-2 py-2 align-middle whitespace-nowrap">-</td>
      <td className="news-tx-cell px-2 py-2 align-middle">-</td>
      <td className="news-tx-cell px-2 py-2 align-middle">-</td>
      <td className="news-tx-cell px-2 py-2 align-middle whitespace-nowrap">-</td>
      <td className="news-tx-cell px-2 py-2 align-middle whitespace-nowrap">-</td>
      <td className="news-tx-cell px-2 py-2 text-right align-middle">-</td>
      <td className="news-tx-cell px-2 py-2 align-middle">-</td>
      <td className="news-tx-cell px-2 py-2 align-middle">-</td>
      <td className="news-tx-cell px-2 py-2 text-center align-middle">-</td>
    </>
  );
}

const TickerRowCells = memo(function TickerRowCells({ row, feature, isActive }) {
  const risk = classifyRisk(feature);
  const status = statusForRow(feature);
  const amountValue = feature ? (feature.urgency_score / 10).toFixed(1) : '--.--';
  const protocolValue = feature?.protocol ?? '-';
  const categoryValue = feature?.category ?? '-';
  const senderValue = row?.sender ?? '-';
  const sourceValue = row?.source_id ?? '-';
  const mainnetValue = resolveMainnetLabel(row?.chain_id, row?.source_id);
  const nonceValue = row?.nonce ?? '-';
  const txTypeValue = row?.tx_type ?? '-';

  return (
    <>
      <td className="news-tx-cell px-2 py-2 align-middle whitespace-nowrap">{formatTickerTime(row.seen_unix_ms)}</td>
      <td className="news-tx-cell px-2 py-2 align-middle overflow-hidden text-ellipsis whitespace-nowrap" title={row.hash}>{shortHex(row.hash, 18, 8)}</td>
      <td className="news-tx-cell px-2 py-2 align-middle overflow-hidden text-ellipsis whitespace-nowrap" title={senderValue}>{shortHex(senderValue, 10, 8)}</td>
      <td className="news-tx-cell px-2 py-2 align-middle overflow-hidden text-ellipsis whitespace-nowrap" title={sourceValue}>{sourceValue}</td>
      <td className="news-tx-cell px-2 py-2 align-middle overflow-hidden text-ellipsis whitespace-nowrap" title={mainnetValue}>{mainnetValue}</td>
      <td className="news-tx-cell px-2 py-2 align-middle">{txTypeValue}</td>
      <td className="news-tx-cell px-2 py-2 align-middle">{nonceValue}</td>
      <td className="news-tx-cell px-2 py-2 align-middle overflow-hidden text-ellipsis whitespace-nowrap" title={protocolValue}>{protocolValue}</td>
      <td className="news-tx-cell px-2 py-2 align-middle overflow-hidden text-ellipsis whitespace-nowrap" title={categoryValue}>{categoryValue}</td>
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
    </>
  );
}, (left, right) => (
  left.row === right.row
  && left.feature === right.feature
  && left.isActive === right.isActive
));

export function RadarVirtualizedTable({ rowModels, featureByHash, selectedHash }) {
  return (
    <div className="news-list-scroll h-full border-b-2 border-zinc-900">
      <table className="news-tx-table news-mono min-w-[1360px] w-full table-fixed border-collapse">
        <thead className="news-tx-head text-[13px] font-bold uppercase tracking-[0.1em]">
          <tr>
            {HEADER_CELLS.map(([label, className]) => (
              <th key={label} scope="col" className={className}>
                {label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rowModels.map((rowModel) => {
            const row = rowModel?.row ?? null;
            if (!row) {
              return (
                <tr
                  key={rowModel?.key ?? 'slot-empty'}
                  data-tx-hash=""
                  className="news-tx-row border-b border-dashed border-zinc-900 text-[13px] text-zinc-400"
                >
                  <EmptyRowCells />
                </tr>
              );
            }

            const feature = featureByHash.get(row.hash);
            const status = statusForRow(feature);
            const mainnetValue = resolveMainnetLabel(row.chain_id, row.source_id);
            const mainnetRowClasses = resolveMainnetRowClasses(mainnetValue);
            const isActive = row.hash === selectedHash;

            return (
              <tr
                key={rowModel.key}
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
                <TickerRowCells
                  row={row}
                  feature={feature}
                  isActive={isActive}
                />
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
