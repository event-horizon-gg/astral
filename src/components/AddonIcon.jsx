import React from 'react';
import { Package } from 'lucide-react';
import astralMark from '../../assets/icon.png';
export default function AddonIcon({ addon, large = false }) {
  const artwork = addon.icon || (addon.local || addon.bundled ? astralMark : null);
  return (
    <span
      className={
        'addon-icon ' + (large ? 'large ' : '') + (artwork ? 'original-art ' : '') + addon.color
      }
      aria-hidden="true"
    >
      {artwork ? (
        <img src={artwork} alt="" aria-hidden="true" decoding="async" />
      ) : (
        <Package size={large ? 28 : 19} />
      )}
    </span>
  );
}
