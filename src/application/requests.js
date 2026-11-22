export function createRequestScope() {
  let revision = 0;
  return {
    invalidate() { revision++; },
    async run(load, accept = () => {}) {
      const token = ++revision;
      try {
        const value = await load();
        if (token !== revision) return undefined;
        accept(value);
        return value;
      } catch (error) {
        if (token !== revision) return undefined;
        throw error;
      }
    },
  };
}
