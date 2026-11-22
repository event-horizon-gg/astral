const commands = require('../shared/commands.json');
function register({ ipcMain, window, exclusive, clearError, failed }) {
  return (name, operation) => {
    const command = commands[name];
    if (!command) throw Error('Undeclared Astral command: ' + name);
    ipcMain.handle('astral:' + name, async (event, ...args) => {
      const contents = window()?.webContents;
      if (!contents || event.sender !== contents || event.senderFrame !== contents.mainFrame)
        throw Error('Untrusted caller.');
      try {
        clearError();
        return await (command.mutates ? exclusive(() => operation(...args)) : operation(...args));
      } catch (error) {
        failed(error);
        throw Error(error.message);
      }
    });
  };
}
module.exports = { register };
