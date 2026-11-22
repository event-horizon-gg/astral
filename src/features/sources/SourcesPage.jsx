import React from 'react';
import { Check, ArrowUpRight } from 'lucide-react';
import ProviderMark from '../../components/ProviderMark.jsx';
export default function SourcesPage({ s, call }) {
  return (
    <div className="source-list">
      {s.providers.map((p) => (
        <article className="source-card" key={p.name}>
          <span className="source-brand">
            <ProviderMark name={p.name} />
          </span>
          <div>
            <h3>{p.name}</h3>
            {p.name === 'GitHub' && (
              <small>
                {s.addons.filter((a) => a.repo).length} installed addon groups use GitHub releases
              </small>
            )}
            <button
              className="text-button source-link"
              onClick={() => call('openProvider', p.name)}
            >
              Visit {p.name}
              <ArrowUpRight size={14} />
            </button>
          </div>
          <span className={'provider-status ' + (p.available ? 'ready' : '')}>
            {p.available ? <Check size={13} /> : <span className="small-dot amber" />}
            {p.available ? 'Available' : 'Unavailable'}
          </span>
        </article>
      ))}
    </div>
  );
}
