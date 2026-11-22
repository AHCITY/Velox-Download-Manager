// Velox popup — renders state from the service worker; submits URLs through
// the same native-messaging path as the context menu (no second pipeline).
const $ = (id) => document.getElementById(id);

function fmtBytes(n) {
  if (n == null) return '—';
  if (n < 1024) return n + ' B';
  const u = ['KiB', 'MiB', 'GiB', 'TiB'];
  let v = n, i = -1;
  do { v /= 1024; i++; } while (v >= 1024 && i < u.length - 1);
  return (v >= 100 ? v.toFixed(0) : v.toFixed(1)) + ' ' + u[i];
}

function fmtSpeed(bps) {
  return bps && bps >= 1 ? fmtBytes(bps) + '/s' : '—';
}

function render(s) {
  const conn = $('conn');
  conn.className = 'conn ' + (s.connected ? 'ok' : 'err');
  $('connText').textContent = s.connected
    ? 'connected' + (s.appVersion ? ' · v' + s.appVersion : '')
    : 'disconnected';
  $('speed').textContent = fmtSpeed(s.aggBps);
  $('active').textContent = String(s.active ?? '—');
  $('ver').textContent = s.connected ? (s.appVersion || '—') : '—';

  const list = $('recent');
  if (s.recent && s.recent.length) {
    list.innerHTML = '';
    for (const r of s.recent) {
      const div = document.createElement('div');
      div.className = 'item';
      const st = String(r.state || '');
      div.innerHTML =
        '<span class="nm" title=""></span><span class="chip ' + st + '">' + st + '</span><span class="st">' +
        (r.total ? fmtBytes(r.received) + ' / ' + fmtBytes(r.total) : fmtBytes(r.received)) + '</span>';
      div.querySelector('.nm').textContent = r.name || ('#' + r.id);
      list.appendChild(div);
    }
  } else {
    list.innerHTML = '<div class="empty">' +
      (s.connected ? 'Nothing yet — downloads appear here.' : 'Velox is not running — start the desktop app.') +
      '</div>';
  }

  $('hint').textContent = s.connected
    ? ''
    : (s.error || 'Start Velox, then click Reconnect.');
}

function send(msg) {
  return new Promise((resolve) => chrome.runtime.sendMessage(msg, (r) => resolve(r || {})));
}

async function refresh() {
  const s = await send({ type: 'get_state' });
  $('intercept').value = (s.prefs && s.prefs.interceptMode) || 'ask';
  $('clip').checked = !!(s.prefs && s.prefs.clipboardEnabled);
  $('permState').textContent = $('clip').checked ? '(granted)' : '';
  render(s);
}

$('add').addEventListener('click', async () => {
  const url = $('url').value.trim();
  const fe = $('fe');
  if (!/^https?:\/\//i.test(url)) {
    fe.hidden = false;
    fe.textContent = 'Enter an http(s) URL.';
    return;
  }
  fe.hidden = true;
  $('add').disabled = true;
  const r = await send({ type: 'add_url', url });
  $('add').disabled = false;
  if (r.ok) {
    $('url').value = '';
    fe.hidden = true;
  } else {
    fe.hidden = false;
    fe.textContent = r.detail || 'Velox is unreachable — start the desktop app.';
  }
  refresh();
});

$('url').addEventListener('keydown', (e) => { if (e.key === 'Enter') $('add').click(); });

$('intercept').addEventListener('change', (e) => {
  send({ type: 'set_pref', key: 'interceptMode', value: e.target.value }).then(refresh);
});

$('clip').addEventListener('change', async (e) => {
  const on = e.target.checked;
  const r = await send({ type: 'set_pref', key: 'clipboardEnabled', value: on });
  if (!r.ok) e.target.checked = false;
  $('permState').textContent = $('clip').checked ? '(granted)' : '';
});

$('reconnect').addEventListener('click', async () => {
  $('hint').textContent = 'reconnecting…';
  await send({ type: 'reconnect' });
  setTimeout(refresh, 800);
});

// optional clipboard suggestion (explicit opt-in; NEVER silent)
$('clip').addEventListener('change', async () => {
  if (!$('clip').checked) return;
  try {
    const text = await navigator.clipboard.readText();
    const t = String(text || '').trim();
    if (/^https?:\/\//i.test(t) && !$('url').value) $('url').value = t;
  } catch (e) { /* permission revoked or no URL — stay silent */ }
});

refresh();
setInterval(refresh, 2000);
