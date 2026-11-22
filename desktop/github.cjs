const headers = Object.freeze({
  'User-Agent': 'Astral-Companion/0.3',
  Accept: 'application/vnd.github+json',
});
function validRepo(repo) {
  return (
    typeof repo === 'string' &&
    /^[A-Za-z0-9][A-Za-z0-9-]*\/[A-Za-z0-9_.-]+$/.test(repo) &&
    !['.', '..'].includes(repo.split('/')[1])
  );
}
async function apiJson(
  url,
  {
    request = fetch,
    timeout = 30000,
    unavailableMessage = (status) => 'GitHub is unavailable (' + status + ').',
    rateMessage = 'GitHub rate limit reached. Try again later.',
  } = {},
) {
  const response = await request(url, { headers, signal: AbortSignal.timeout(timeout) });
  if (!response.ok) {
    const limited = response.status === 403 || response.status === 429;
    const error = Error(
      limited
        ? rateMessage
        : typeof unavailableMessage === 'function'
          ? unavailableMessage(response.status)
          : unavailableMessage,
    );
    error.code = limited ? 'RATE_LIMIT' : 'PROVIDER_ERROR';
    error.status = response.status;
    throw error;
  }
  return response.json();
}
async function latest(repo, options) {
  if (!validRepo(repo)) throw Error('Invalid GitHub project.');
  return apiJson('https://api.github.com/repos/' + repo + '/releases/latest', options);
}
module.exports = { validRepo, headers, apiJson, latest };
