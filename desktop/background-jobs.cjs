function create({
  configuration,
  updates,
  gameRunning,
  hasSession,
  state,
  changed,
  syncReady,
  daily,
  busy,
  failed,
  timers = globalThis,
  now = Date.now,
}) {
  const scheduled = [],
    active = new Set();
  let stopped = false,
    wasRunning;
  async function run(name, operation) {
    if (stopped || active.has(name)) return;
    active.add(name);
    try {
      await operation();
    } catch (error) {
      failed(name, error);
    } finally {
      active.delete(name);
    }
  }
  function schedule(name, operation, delay, repeat = false) {
    if (stopped) return;
    const setter = repeat ? 'setInterval' : 'setTimeout';
    const handle = timers[setter](() => run(name, operation), delay);
    handle.unref?.();
    scheduled.push({ handle, repeat });
  }
  async function start() {
    const checkApp = async () => {
      if (
        configuration().appUpdateChecks !== false &&
        !updates.active &&
        now() - (updates.status.checkedAt || 0) > 86400000
      )
        await updates.check();
    };
    schedule('application-update', checkApp, 10000);
    schedule('application-update', checkApp, 3600000, true);
    await run('session-startup', async () => {
      if ((await hasSession()) && !(await gameRunning()))
        schedule('session-startup', () => syncReady('Checking your connected setup.'), 750);
    });
    await run('game-state', async () => {
      wasRunning = await gameRunning();
    });
    schedule(
      'game-state',
      async () => {
        const running = await gameRunning();
        if (stopped) return;
        if (wasRunning !== running) changed();
        if (wasRunning && !running) {
          const current = await state();
          if (current.signedIn) await syncReady('WoW closed. Your settings are ready to sync.');
        }
        wasRunning = running;
      },
      15000,
      true,
    );
    schedule(
      'sync',
      async () => {
        if (
          !busy() &&
          !configuration().syncProfilePending &&
          (await hasSession()) &&
          !(await gameRunning())
        )
          await syncReady('Checking your connected setup.');
      },
      300000,
      true,
    );
    const checkDaily = async () => {
      if (!busy()) await daily();
    };
    schedule('daily', checkDaily, 60000, true);
    schedule('daily', checkDaily, 15000);
  }
  function stop() {
    stopped = true;
    for (const { handle, repeat } of scheduled)
      timers[repeat ? 'clearInterval' : 'clearTimeout'](handle);
    scheduled.length = 0;
  }
  return { start, stop };
}
module.exports = { create };
