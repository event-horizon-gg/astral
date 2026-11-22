import React from 'react';
import {
  ArrowDownToLine,
  Search,
  Check,
  Package,
  Globe,
  X,
  HardDrive,
  AlertCircle,
  LoaderCircle,
  MoreHorizontal,
} from 'lucide-react';
import AddonIcon from '../../components/AddonIcon.jsx';
import ProviderMark from '../../components/ProviderMark.jsx';
export default function LibraryPage({
  s,
  busy,
  loadError,
  queueResult,
  setQueueResult,
  updateAll,
  filter,
  setFilter,
  updates,
  search,
  query,
  setQuery,
  filtered,
  selected,
  setSelected,
  progress,
  update,
  call,
}) {
  return !s.loaded ? (
    <div className="empty" role="status">
      {loadError ? (
        <>
          <h3>Couldn’t load addons</h3>
          <button className="button secondary" onClick={() => window.location.reload()}>
            Retry
          </button>
        </>
      ) : (
        <>
          <LoaderCircle className="spin" size={22} />
          <p>Loading addons…</p>
        </>
      )}
    </div>
  ) : (
    <>
      {queueResult && (
        <section className="update-result" aria-label="Update results" role="status">
          <div className="section-heading">
            <strong>
              {queueResult.updated.length} updated
              {queueResult.current.length > 0
                ? ` · ${queueResult.current.length} already current`
                : ''}
              {queueResult.failed
                ? ` · 1 failed · ${queueResult.pending.length} not attempted`
                : ''}
            </strong>
            <button className="text-button" onClick={() => setQueueResult(null)}>
              Dismiss
            </button>
          </div>
          {queueResult.updated.length > 0 && (
            <p>{queueResult.updated.map((a) => a.name).join(', ')}</p>
          )}
          {queueResult.failed && (
            <>
              <p className="update-failure">
                <strong>{queueResult.failed.name}:</strong> {queueResult.failed.message}
              </p>
              {queueResult.pending.length > 0 && (
                <p>Not attempted: {queueResult.pending.map((a) => a.name).join(', ')}</p>
              )}
              <button className="text-button" disabled={busy || s.running} onClick={updateAll}>
                Retry remaining updates
              </button>
            </>
          )}
        </section>
      )}
      <div className="list-controls">
        <div className="tabs">
          {['All addons', 'Updates', 'Manual'].map((t) => (
            <button key={t} className={filter === t ? 'selected' : ''} onClick={() => setFilter(t)}>
              {t}
              {t === 'Updates' && updates > 0 && <span>{updates}</span>}
            </button>
          ))}
        </div>
        <label className="search-box">
          <Search size={15} />
          <input
            ref={search}
            aria-label="Search addons"
            placeholder="Find an addon…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          {query && (
            <button aria-label="Clear search" onClick={() => setQuery('')}>
              <X size={13} />
            </button>
          )}
        </label>
      </div>
      <section className="addon-list" aria-label="Installed addons">
        <div className="table-head">
          <span>ADDON</span>
          <span>SOURCE</span>
          <span>VERSION</span>
          <span>STATUS</span>
          <span />
        </div>
        {filtered.length ? (
          filtered.map((a) => (
            <div key={a.id} className={'addon-row ' + (selected?.id === a.id ? 'chosen' : '')}>
              <button className="addon-main" onClick={() => setSelected(a)}>
                <AddonIcon addon={a} />
                <span className="addon-name">
                  <strong>{a.name}</strong>
                </span>
              </button>
              <span className="source-cell">
                {a.repo ? (
                  <ProviderMark name="GitHub" small />
                ) : a.local || a.bundled ? (
                  <HardDrive size={12} />
                ) : (
                  <Globe size={12} />
                )}{' '}
                <span title={a.repo || undefined}>
                  {a.local || a.bundled ? 'Astral' : a.source}
                </span>
              </span>
              <span className="version" title={a.version}>
                {a.id === 'rio' ? (a.version.match(/v\d{12}/) || [a.version])[0] : a.version}
              </span>
              {(a.repo || a.bundled) && (
                <span className={'status-label ' + a.status}>
                  {a.release?.error ? (
                    <>
                      <AlertCircle size={12} />
                      Needs attention
                    </>
                  ) : progress.id === a.id && progress.kind === 'update' ? (
                    <>
                      <LoaderCircle size={12} className="spin" />
                      {progress.percent}%
                    </>
                  ) : a.status === 'update' ? (
                    <>
                      <span className="small-dot amber" />
                      Update ready
                    </>
                  ) : a.status === 'current' ? (
                    <>
                      <Check size={12} />
                      Up to date
                    </>
                  ) : a.status === 'local' ? (
                    'Development'
                  ) : a.status === 'manual' ? (
                    'Manual source'
                  ) : (
                    'Not checked'
                  )}
                </span>
              )}
              <button
                className="row-action"
                disabled={busy || (s.running && a.status === 'update')}
                aria-label={a.status === 'update' ? 'Update ' + a.name : 'Details for ' + a.name}
                onClick={() => (a.status === 'update' ? update(a) : setSelected(a))}
              >
                {a.status === 'update' ? (
                  <ArrowDownToLine size={16} />
                ) : (
                  <MoreHorizontal size={17} />
                )}
              </button>
            </div>
          ))
        ) : (
          <div className="empty">
            <Package size={28} />
            <h3>{s.addons.length ? 'No addons match that search' : 'Choose your WoW folder'}</h3>
            {s.addons.length > 0 && (
              <button
                className="button secondary"
                onClick={() => {
                  setQuery('');
                  setFilter('All addons');
                }}
              >
                Clear filters
              </button>
            )}
            {!s.addons.length && (
              <button className="button secondary" onClick={() => call('chooseRoot')}>
                Find World of Warcraft
              </button>
            )}
          </div>
        )}
      </section>
    </>
  );
}
