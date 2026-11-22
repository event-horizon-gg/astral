import React from 'react';
export default function SetupComparison({
  rows = [],
  localLabel = 'This computer',
  remoteLabel = 'Cloud',
  showFiles = true,
}) {
  if (!rows.length) return null;
  return (
    <div className="setup-comparison" role="table" aria-label="Setup changes">
      <div className="comparison-head" role="row">
        <span role="columnheader">Settings</span>
        <span role="columnheader">{localLabel}</span>
        <span role="columnheader">{remoteLabel}</span>
      </div>
      {rows.map((row) => (
        <div key={row.id} className={'comparison-item' + (row.changed ? ' differs' : '')}>
          <div className="comparison-values" role="row">
            <strong role="rowheader">{row.label}</strong>
            <span role="cell">{row.local}</span>
            <span role="cell">
              {row.remote}
              {row.changed && row.local === row.remote && <small>Changed</small>}
            </span>
          </div>
          {row.details && row.changed && (
            <details>
              <summary>{row.details}</summary>
              <ul>
                {row.choices?.map((choice) => (
                  <li key={choice.scope + ':' + choice.folder}>
                    {choice.folder} · {choice.scope || 'Account'}: {choice.local} →{' '}
                    {choice.remote}
                  </li>
                ))}
              </ul>
            </details>
          )}
          {showFiles && !!row.files?.length && (
            <details>
              <summary>
                {row.files.length} changed {row.files.length === 1 ? 'file' : 'files'}
              </summary>
              <ul>
                {row.files.map((file) => (
                  <li key={file.path}>
                    <span>{file.path}</span>
                    {!file.local ? ' · New file' : ''}
                  </li>
                ))}
              </ul>
            </details>
          )}
        </div>
      ))}
    </div>
  );
}
