import React from 'react';
import {
  FileInput,
  FileOutput,
  Search,
  RefreshCw,
  ChevronRight,
  ChevronDown,
  Cloud,
  Monitor,
  Settings,
  History,
  Globe,
  FolderOpen,
  LogOut,
  MoreHorizontal,
  Sun,
  Moon,
} from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import wowRetailIcon from '../../assets/games/wow-retail.png';
import wowForeverIcon from '../../assets/games/wow-forever.png';
import { localTime as time } from '../application/format.js';
import { pages } from '../application/navigation.js';
import { navigationIcons } from './navigation-icons.js';
export default function AppHeader({
  page,
  s,
  busy,
  theme,
  setTheme,
  setQuery,
  setSelected,
  call,
  onNavigate,
  onCommand,
  onShare,
  overlayOpen,
}) {
  const [popover, setPopover] = useState(null);
  const accountOpen = popover === 'account',
    toolsOpen = popover === 'tools';
  const accountRef = useRef(null),
    toolsRef = useRef(null);
  const navigate = (next) => {
    setPopover(null);
    onNavigate(next);
  };
  useEffect(() => {
    const dismiss = (event) => {
      if (!accountRef.current?.contains(event.target) && !toolsRef.current?.contains(event.target))
        setPopover(null);
    };
    const escape = (event) => {
      if (event.key === 'Escape') setPopover(null);
    };
    document.addEventListener('pointerdown', dismiss);
    window.addEventListener('keydown', escape);
    return () => {
      document.removeEventListener('pointerdown', dismiss);
      window.removeEventListener('keydown', escape);
    };
  }, []);
  useEffect(() => {
    if (overlayOpen) setPopover(null);
  }, [overlayOpen]);
  return (
    <header className="topbar">
      <button
        className="editorial-brand"
        onClick={() => navigate('Addons')}
        aria-label="Astral library"
      >
        <span className="eclipse-mark" aria-hidden="true" />
        <strong>Astral</strong>
      </button>
      <nav className="primary-nav" aria-label="Main navigation">
        {pages
          .filter((item) => item.primary)
          .map((item) => (
            <button
              key={item.id}
              className={page === item.id ? 'active' : ''}
              aria-current={page === item.id ? 'page' : undefined}
              onClick={() => navigate(item.id)}
            >
              {item.label}
            </button>
          ))}
      </nav>
      <div className="header-actions">
        <div className="game">
          <img
            className="client-game-icon"
            src={s.flavor === 'forever' ? wowForeverIcon : wowRetailIcon}
            alt=""
          />
          <select
            className="client-select"
            aria-label="Game client"
            value={s.flavor || 'retail'}
            disabled={busy}
            onChange={(e) => {
              setQuery('');
              setSelected(null);
              call('client', e.target.value);
            }}
          >
            {(s.clients?.length ? s.clients : [{ id: 'retail', label: 'Retail' }]).map((c) => (
              <option value={c.id} key={c.id}>
                {c.label}
              </option>
            ))}
          </select>
        </div>
        <button
          className="icon-button"
          aria-label="Quick find"
          title="Quick find · Cmd/Ctrl K"
          onClick={onCommand}
        >
          <Search size={18} />
        </button>
        <button
          className="icon-button"
          aria-label={'Switch to ' + (theme === 'dark' ? 'light' : 'dark') + ' theme'}
          title="Switch appearance"
          onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}
        >
          {theme === 'dark' ? <Sun size={18} /> : <Moon size={18} />}
        </button>
        <div className="header-popover-anchor" ref={toolsRef}>
          <button
            className="icon-button"
            aria-label="Tools"
            aria-expanded={toolsOpen}
            onClick={() => {
              setPopover((current) => (current === 'tools' ? null : 'tools'));
            }}
          >
            <MoreHorizontal size={20} />
          </button>
          {toolsOpen && (
            <div className="header-popover tools-popover" aria-label="Tools">
              {pages
                .filter((item) => item.tools)
                .map((item) => {
                  const Icon = navigationIcons[item.icon];
                  return (
                    <button key={item.id} onClick={() => navigate(item.id)}>
                      <Icon size={16} />
                      {item.menuLabel || item.label}
                    </button>
                  );
                })}
              {[
                ['export', 'Export setup', FileOutput, 'setupExportInfo'],
                ['import', 'Import setup', FileInput, 'openSetupImport'],
              ].map(([mode, label, Icon, action]) => (
                <button
                  key={mode}
                  disabled={busy || !s.account}
                  title={!s.account ? 'Select a local WoW account in Settings.' : undefined}
                  onClick={async () => {
                    setPopover(null);
                    onShare(mode, action);
                  }}
                >
                  <Icon size={16} />
                  {label}
                </button>
              ))}
              <button
                onClick={() => {
                  setPopover(null);
                  call('chooseRoot');
                }}
              >
                <FolderOpen size={16} />
                Choose WoW folder
              </button>
            </div>
          )}
        </div>
        <div className="header-popover-anchor" ref={accountRef}>
          <button
            className="profile-trigger"
            aria-label="Account menu"
            aria-expanded={accountOpen}
            onClick={() => {
              setPopover((current) => (current === 'account' ? null : 'account'));
            }}
          >
            <span className="profile-avatar">
              {s.signedIn ? s.accountName?.slice(0, 1) || 'A' : <Cloud size={18} />}
            </span>
            <ChevronDown size={13} />
          </button>
          {accountOpen && (
            <div className="header-popover profile-popover" aria-label="Account">
              <div className="profile-summary">
                <span className="profile-avatar large">
                  {s.signedIn ? s.accountName?.slice(0, 1) || 'A' : <Cloud size={22} />}
                </span>
                <div>
                  <strong>{s.signedIn ? s.accountName : 'Your Astral account'}</strong>
                  <small>
                    <i className={'connection-dot ' + (s.signedIn ? 'connected' : '')} />
                    {s.signedIn ? 'Battle.net connected' : 'Not connected'}
                  </small>
                </div>
              </div>
              <button onClick={() => navigate(pages.find((item) => item.account).id)}>
                <Cloud size={17} />
                {pages.find((item) => item.account).label}
                <ChevronRight size={14} />
              </button>
              {s.signedIn && (
                <div className="profile-sync">
                  <RefreshCw size={16} />
                  <span>
                    Last sync<small>{time(s.lastSync)}</small>
                  </span>
                </div>
              )}
              <div className="profile-device">
                <Monitor size={15} />
                {s.platform === 'win32' ? 'This Windows PC' : 'This Mac'}
                <span>{s.running ? 'WoW running' : 'WoW closed'}</span>
              </div>
              {s.signedIn && (
                <button
                  className="profile-signout"
                  disabled={busy}
                  onClick={() => {
                    setPopover(null);
                    call('disconnect');
                  }}
                >
                  <LogOut size={16} />
                  Sign out
                </button>
              )}
            </div>
          )}
        </div>
      </div>
    </header>
  );
}
