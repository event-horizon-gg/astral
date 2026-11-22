import React, { useEffect, useRef } from 'react';
import useApplication from './useApplication.js';
import useNavigation from './useNavigation.js';
import useTheme from './useTheme.js';
import AppHeader from '../components/AppHeader.jsx';
import PageHeader from '../components/PageHeader.jsx';
import AppDialogs from '../components/AppDialogs.jsx';
import Notification from '../components/Notification.jsx';
import AddonIcon from '../components/AddonIcon.jsx';
import LibraryPage from '../features/library/LibraryPage.jsx';
import AddonDrawer from '../features/library/AddonDrawer.jsx';
import useLibrary from '../features/library/useLibrary.js';
import DiscoveryPanel from '../features/discovery/DiscoveryPanel.jsx';
import AccountPage from '../features/account/AccountPage.jsx';
import ActivityPage from '../features/activity/ActivityPage.jsx';
import SourcesPage from '../features/sources/SourcesPage.jsx';
import SettingsPage from '../features/settings/SettingsPage.jsx';
import ShareSetup from '../features/sharing/ShareSetup.jsx';
import useSharing from '../features/sharing/useSharing.js';
import { localTime } from './format.js';
import { ProfilesProvider } from '../features/account/SyncProfiles.jsx';
import FeedbackContext from '../components/FeedbackContext.jsx';

export default function App({ api = globalThis.window?.astral }) {
  const application = useApplication(api);
  const {
    s,
    client,
    call,
    requestBusy,
    progress,
    toast,
    notify,
    loadError,
    commandErrors,
    dialog,
    setDialog,
  } = application;
  const navigation = useNavigation(s);
  const {
    page,
    selected,
    setSelected,
    command,
    setCommand,
    query,
    setQuery,
    filter,
    setFilter,
  } = navigation;
  const navigate = (page) => {
    setDialog(null);
    navigation.navigate(page);
  };
  const [theme, setTheme] = useTheme();
  const library = useLibrary({ s, client, call, notify, setSelected });
  const { queueBusy, queueResult, setQueueResult, check, update, importZip, updateAll } =
    library;
  const sharing = useSharing(s, call);
  const search = useRef(null);
  const busy = requestBusy || queueBusy || progress.kind === 'update' || !s.loaded;
  const updates = s.addons.filter((addon) => addon.status === 'update').length;
  const filtered = s.addons
    .filter((addon) =>
      (addon.name + ' ' + addon.category).toLowerCase().includes(query.toLowerCase()),
    )
    .filter(
      (addon) =>
        filter === 'All addons' ||
        (filter === 'Updates' && addon.status === 'update') ||
        (filter === 'Manual' && ['manual', 'local'].includes(addon.status)),
    );
  const selectedBackup =
    selected &&
    s.backups.find(
      (backup) => backup.addonId === selected.id && backup.previousFolders?.length > 0,
    );
  const selectedFailure =
    selected && s.addonActivity?.find((entry) => entry.addonId === selected.id);
  useEffect(() => {
    const keydown = (event) => {
      if ((event.metaKey || event.ctrlKey) && event.key === 'k') {
        event.preventDefault();
        setCommand((value) => !value);
      }
      if (event.key === 'Escape') setSelected(null);
    };
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, []);
  return (
    <FeedbackContext.Provider value={{ toast, onDismiss: application.dismissToast }}>
      <ProfilesProvider
        key={JSON.stringify([s.root, s.account, s.flavor, s.accountName])}
        s={s}
        busy={busy}
        call={call}
        client={client}
      >
        <div
          className={
            'app editorial' + (client.available && s.platform === 'darwin' ? ' native-mac' : '')
          }
        >
          <div className="workspace">
            <AppHeader
              page={page}
              s={s}
              busy={busy}
              theme={theme}
              setTheme={setTheme}
              setQuery={setQuery}
              setSelected={setSelected}
              call={call}
              onNavigate={navigate}
              onCommand={() => setCommand(true)}
              onShare={sharing.open}
              overlayOpen={command || !!dialog || !!sharing.share}
            />
            <main data-page={page}>
              <PageHeader
                page={page}
                s={s}
                busy={busy}
                updates={updates}
                check={check}
                updateAll={updateAll}
                navigate={navigate}
                api={client.available}
                progress={progress}
              />
              {page === 'Addons' && (
                <LibraryPage
                  s={s}
                  busy={busy}
                  loadError={loadError}
                  queueResult={queueResult}
                  setQueueResult={setQueueResult}
                  updateAll={updateAll}
                  filter={filter}
                  setFilter={setFilter}
                  updates={updates}
                  search={search}
                  query={query}
                  setQuery={setQuery}
                  filtered={filtered}
                  selected={selected}
                  setSelected={setSelected}
                  progress={progress}
                  update={update}
                  call={call}
                />
              )}
              {page === 'Discover' && (
                <DiscoveryPanel
                  s={s}
                  busy={busy}
                  call={call}
                  client={client}
                  notify={notify}
                  AddonIcon={AddonIcon}
                  onLibrary={(name) => {
                    setQuery(name);
                    navigate('Addons');
                  }}
                />
              )}
              {page === 'Your setup' && (
                <AccountPage
                  s={s}
                  busy={busy}
                  call={call}
                  client={client}
                  notify={notify}
                  time={localTime}
                  connectionError={commandErrors.connect}
                />
              )}
              {page === 'Sources' && <SourcesPage s={s} call={call} />}
              {page === 'Activity' && (
                <ActivityPage
                  s={s}
                  busy={busy}
                  call={call}
                  notify={notify}
                  setDialog={setDialog}
                  navigate={navigate}
                  update={update}
                />
              )}
              {page === 'Settings' && (
                <SettingsPage s={s} busy={busy} call={call} notify={notify} />
              )}
            </main>
          </div>
          {selected && (
            <AddonDrawer
              selected={selected}
              setSelected={setSelected}
              s={s}
              busy={busy}
              call={call}
              update={update}
              importZip={importZip}
              check={check}
              selectedBackup={selectedBackup}
              selectedFailure={selectedFailure}
              setDialog={setDialog}
            />
          )}
          {sharing.share && (
            <ShareSetup
              share={sharing.share}
              s={s}
              busy={busy}
              call={call}
              notify={notify}
              onClose={sharing.close}
            />
          )}
          <AppDialogs
            dialog={dialog}
            command={command}
            setDialog={setDialog}
            setCommand={setCommand}
            navigate={navigate}
            call={call}
            notify={notify}
            busy={busy}
          />
          <Notification toast={toast} onDismiss={application.dismissToast} />
        </div>
      </ProfilesProvider>
    </FeedbackContext.Provider>
  );
}
