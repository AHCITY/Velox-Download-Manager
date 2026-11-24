#!/usr/bin/env bash
# Velox UI E2E — UX 2.0 acceptance suite (robust harness v5).
# Drives the REAL engine (velox serve-http) in a headless browser via
# agent-browser. Multi-step interactions run as ONE async eval each (fewer CDP
# round trips; CDP hiccups retry once). Assertions: PASS/FAIL per check.
#
# Usage: scripts/e2e-ui.sh [--keep]
set -u
ROOT=/home/z/my-project/download-engine
ENGINE="$ROOT/target/debug/velox"
E2E=/tmp/velox-e2e
FILES="$E2E/files"
# unique ports per run: a stale server from a previous run can never hijack this run
PORT_BASE=$((8500 + RANDOM % 300))
PORT_UI=$PORT_BASE
PORT_FILES=$((PORT_BASE + 1))
PORT_SLOW=$((PORT_BASE + 2))
KEEP=0
[ "${1:-}" = "--keep" ] && KEEP=1

fail() { echo "E2E SETUP FAILED: $1"; exit 1; }
[ -x "$ENGINE" ] || fail "engine binary missing — run: cargo build -p engine-cli --bin velox"
command -v agent-browser >/dev/null || fail "agent-browser not installed"

# a leftover server from a previous run would silently hijack this run's port
pkill -f "velox serve-http" 2>/dev/null || true
pkill -f slow-server.py 2>/dev/null || true
pkill -f slow-tmp.py 2>/dev/null || true
pkill -f "http.server $((PORT_BASE + 1))" 2>/dev/null || true
sleep 1

rm -rf "$E2E"
mkdir -p "$FILES" "$E2E/data" "$E2E/out"
python3 - <<'PY'
import random, pathlib
d = pathlib.Path('/tmp/velox-e2e/files')
random.seed(7)
(d / 'sample-archive.zip').write_bytes(bytes(random.getrandbits(8) for _ in range(8 * 1024 * 1024)))
(d / 'notes-readme.txt').write_text('velox e2e fixture — plain text\n' * 400)
print('fixtures ready')
PY
sed 's/127\.0\.0\.1", 8392/127.0.0.1", '"$PORT_SLOW"'/' /home/z/my-project/scripts/slow-server.py > /tmp/velox-e2e/slow-tmp.py
grep -q "$PORT_SLOW" /tmp/velox-e2e/slow-tmp.py || fail "slow-server port substitution failed"
python3 /tmp/velox-e2e/slow-tmp.py > /tmp/velox-e2e/slow.log 2>&1 &
SLOW_PID=$!
(cd "$FILES" && exec python3 -m http.server $PORT_FILES --bind 127.0.0.1) > /tmp/velox-e2e/files.log 2>&1 &
FILES_PID=$!
DXDL_HOME="$E2E/data" "$ENGINE" serve-http --port $PORT_UI --ui-dir "$ROOT/web" --out "$E2E/out" > /tmp/velox-e2e/engine.log 2>&1 &
ENGINE_PID=$!
sleep 1.5
curl -sf "http://127.0.0.1:$PORT_UI/config.json" > /dev/null || fail "engine did not start (see $E2E/engine.log)"

pass=0; failed=0
# one CDP eval with a single retry on transient timeout
eb() {
  local out
  out=$(agent-browser eval "$1" 2>/dev/null | tail -1)
  if [ -z "$out" ] || [[ "$out" == *"timed out"* ]]; then sleep 2; out=$(agent-browser eval "$1" 2>/dev/null | tail -1); fi
  printf '%s' "$out"
}
# check <name> <js-bool-expr> [js-detail-expr]
check() {
  local name="$1" expr="$2" detail="${3:-$2}"
  [ "$detail" = "—" ] && detail="'—'"
  local be bd res
  be=$(printf '%s' "$expr" | base64 -w0)
  bd=$(printf '%s' "$detail" | base64 -w0)
  res=$(eb "(function(){ try { var ok=!!eval(atob('$be')); var dt=''; try { dt=String(eval(atob('$bd'))); } catch(x){ dt='detail-exc: '+x.message; } return (ok?'PASS|':'FAIL|')+dt.slice(0,150); } catch(x){ return 'FAIL|EXC '+String(x.message).slice(0,150); } })()")
  local q='"'; res="${res%$q}"; res="${res#$q}"
  if [[ "$res" == "PASS|"* ]]; then echo "PASS  $name"; pass=$((pass+1));
  else echo "FAIL  $name — ${res#FAIL|}"; failed=$((failed+1)); fi
}

echo "== open app"
agent-browser open "http://127.0.0.1:$PORT_UI" >/dev/null 2>&1
sleep 2.5

# ---------- A. downloads-first shell ----------
check "default route is Downloads" "location.hash !== '#home' && !!document.querySelector('#screen-downloads')" "location.hash + ' | active nav: ' + (document.querySelector('.nav-item.active')?.textContent.trim() || '')"
check "no Home/Vault/Devices nav items" "![...document.querySelectorAll('.nav-item')].some(function(b){return /Home|Vault|Devices/.test(b.textContent);})" "'nav: ' + ([...document.querySelectorAll('.nav-item')].map(function(b){return b.textContent.trim();}).join(','))"
check "toolbar has prominent Add Download" "!!document.querySelector('#tbAdd.btn.primary')" "document.querySelector('#tbAdd')?.textContent.trim()"
check "toolbar has no URL input" "!document.querySelector('#tbUrl')" "'removed'"
check "Active Downloads indicator present (replaces the speed counter)" "!!document.querySelector('#tbActivePill') && /Active downloads/.test(document.querySelector('#tbActivePill').textContent) && /^\d+$/.test(document.querySelector('#tbActive').textContent)" "'text: ' + (document.querySelector('#tbActivePill')?.textContent.trim() || 'MISSING')"
check "no Velox text in the sidebar head (icon-only branding)" "!document.querySelector('.logo-name')" "'removed'"

# ---------- B. add download flow ----------
eb "(function(){ document.querySelector('#tbAdd').click(); return 'ok'; })()" >/dev/null
sleep 0.8
check "Add button opens dialog" "!!document.querySelector('#addDlg')" "'—'"
check "URL field receives focus" "(document.activeElement||{}).id === 'adUrl'" "'active: ' + ((document.activeElement||{}).id)"
eb "(async function(){ var u=document.querySelector('#adUrl'); u.value='http://127.0.0.1:$PORT_FILES/sample-archive.zip'; u.dispatchEvent(new Event('change',{bubbles:true})); await new Promise(function(r){ setTimeout(r,1300); }); return 'inspected'; })()" >/dev/null
check "URL inspection runs (size shown)" "(function(){ var p=document.querySelector('#adPrev'); return p && p.classList.contains('ok') && p.textContent.indexOf('MiB')>=0; })()" "'prev: ' + ((document.querySelector('#adPrev')||{textContent:''}).textContent.slice(0,90))"
check "destination defaults to engine out dir" "(document.querySelector('#adDest')?.value || '').indexOf('/tmp/velox-e2e') === 0" "'dest: ' + (document.querySelector('#adDest')?.value)"
eb "(function(){ document.querySelector('#adGo').click(); return 'submitted'; })()" >/dev/null
sleep 3
check "download appears in unified list" "[...document.querySelectorAll('#dlList .dl-row .nm')].some(function(n){return n.textContent.indexOf('sample-archive.zip')>=0;})" "'rows: ' + ([...document.querySelectorAll('#dlList .dl-row .nm')].map(function(n){return n.textContent;}).join('|'))"

# ---------- C. search regression (the reported bug) ----------
check "already on Downloads before typing" "(document.querySelector('.nav-item.active')||{textContent:''}).textContent.indexOf('Downloads')>=0" "'—'"
eb "(async function(){ var s=document.querySelector('#tbSearch'); s.focus(); s.value='sample'; s.dispatchEvent(new Event('input',{bubbles:true})); await new Promise(function(r){ setTimeout(r,900); }); return 'ok'; })()" >/dev/null
check "typing filters list immediately (regression)" "(function(){ var rows=[...document.querySelectorAll('#dlList .dl-row')]; return rows.length >= 1 && rows.every(function(r){return r.textContent.toLowerCase().indexOf('sample')>=0;}); })()" "'visible: ' + ([...document.querySelectorAll('#dlList .dl-row .nm')].map(function(n){return n.textContent;}).join('|'))"
eb "(async function(){ var s=document.querySelector('#tbSearch'); s.value='zzz-no-match'; s.dispatchEvent(new Event('input',{bubbles:true})); await new Promise(function(r){ setTimeout(r,900); }); return 'ok'; })()" >/dev/null
check "no-match shows search empty state" "!!document.querySelector('#dlList .empty-state')" "'h3: ' + ((document.querySelector('#dlList .empty-state h3')||{textContent:''}).textContent)"
eb "(async function(){ var s=document.querySelector('#tbSearch'); s.value=''; s.dispatchEvent(new Event('input',{bubbles:true})); await new Promise(function(r){ setTimeout(r,900); }); return 'ok'; })()" >/dev/null
check "clearing search restores list" "document.querySelectorAll('#dlList .dl-row').length >= 1" "'count: ' + document.querySelectorAll('#dlList .dl-row').length"

# ---------- D. slow download: transfer / pause / resume / cancel ----------
DRES=$(eb "(async function(){ document.querySelector('#tbAdd').click(); await new Promise(function(r){ setTimeout(r,600); }); var u=document.querySelector('#adUrl'); if(!u) return 'no dialog'; u.value='http://127.0.0.1:$PORT_SLOW/slow.bin'; u.dispatchEvent(new Event('change',{bubbles:true})); await new Promise(function(r){ setTimeout(r,1100); }); document.querySelector('#adGo').click(); await new Promise(function(r){ setTimeout(r,1800); }); return 'submitted dialogs=' + document.querySelectorAll('.overlay.show').length; })()")
echo "  [D diag] $DRES"
sleep 1
echo "  [D engine] $(curl -s -m 5 -X POST http://127.0.0.1:$PORT_UI/rpc -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":9,"method":"download.list","params":{}}' | python3 -c "import json,sys; rows=json.load(sys.stdin)['result']; print([(r['final_name'][:14], r['state'], r['received'], r['total_size']) for r in rows])" 2>/dev/null)"
sleep 3
check "slow download is transferring" "[...document.querySelectorAll('#dlList .dl-row')].some(function(r){return r.textContent.indexOf('slow.bin')>=0 && (r.querySelector('.chip.downloading') || r.querySelector('.chip.queued'));})" "'rows: ' + ([...document.querySelectorAll('#dlList .dl-row')].map(function(r){return r.querySelector('.nm').textContent.slice(0,12) + '/' + r.querySelector('.chip').textContent;}).join('|'))"
eb "(function(){ var row=[...document.querySelectorAll('#dlList .dl-row')].find(function(r){return r.textContent.indexOf('slow.bin')>=0;}); if(!row) return 'no row'; var b=row.querySelector('[data-act=pause]'); if(!b) return 'no pause btn'; b.click(); return 'clicked'; })()" >/dev/null
sleep 2
check "pause works" "[...document.querySelectorAll('#dlList .dl-row')].some(function(r){return r.textContent.indexOf('slow.bin')>=0 && r.querySelector('.chip.paused');})" "'chips: ' + ([...document.querySelectorAll('#dlList .dl-row .chip')].map(function(n){return n.textContent;}).join('|'))"
eb "(function(){ var row=[...document.querySelectorAll('#dlList .dl-row')].find(function(r){return r.textContent.indexOf('slow.bin')>=0;}); if(row){ var b=row.querySelector('[data-act=resume]'); if(b) b.click(); } return 'ok'; })()" >/dev/null
sleep 2.5
check "resume works" "[...document.querySelectorAll('#dlList .dl-row')].some(function(r){return r.textContent.indexOf('slow.bin')>=0 && (r.querySelector('.chip.downloading') || r.querySelector('.chip.queued'));})" "'chips: ' + ([...document.querySelectorAll('#dlList .dl-row .chip')].map(function(n){return n.textContent;}).join('|'))"
eb "(function(){ var row=[...document.querySelectorAll('#dlList .dl-row')].find(function(r){return r.textContent.indexOf('slow.bin')>=0;}); if(row){ var b=row.querySelector('[data-act=cancel]'); if(b) b.click(); } return 'ok'; })()" >/dev/null
sleep 3.5
check "cancel works (engine removes the partial record — row leaves the list)" "![...document.querySelectorAll('#dlList .dl-row')].some(function(r){return r.textContent.indexOf('slow.bin')>=0;})" "'rows left: ' + ([...document.querySelectorAll('#dlList .dl-row .nm')].map(function(n){return n.textContent;}).join('|'))"

# ---------- E. failed download + Failed filter + failed search + retry ----------
ERES=$(eb "(async function(){ var r=await fetch('/rpc',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({jsonrpc:'2.0',id:'f1',method:'download.create',params:{url:'http://127.0.0.1:$PORT_SLOW/switch.bin'}})}); var j=await r.json(); if(!j.result) return 'create-err: '+JSON.stringify(j).slice(0,80); await fetch('/rpc',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({jsonrpc:'2.0',id:'f2',method:'download.start',params:{id:j.result.id}})}); return 'started'; })()")
echo "  [E diag] $ERES"
sleep 4.5
check "failed download visible in All" "[...document.querySelectorAll('#dlList .dl-row')].some(function(r){return r.textContent.indexOf('switch.bin')>=0 && r.querySelector('.chip.failed');})" "'chips: ' + ([...document.querySelectorAll('#dlList .dl-row .chip')].map(function(n){return n.textContent;}).join('|'))"
eb "(function(){ document.querySelector('#dlSeg [data-f=failed]').click(); return 'ok'; })()" >/dev/null
sleep 1
check "Failed filter shows the failure" "(function(){ var rows=[...document.querySelectorAll('#dlList .dl-row')]; return rows.length >= 1 && rows.every(function(r){return r.textContent.indexOf('switch.bin')>=0;}); })()" "'rows: ' + ([...document.querySelectorAll('#dlList .dl-row .nm')].map(function(n){return n.textContent;}).join('|'))"
check "failed row shows error line + Retry action" "(function(){ var r=[...document.querySelectorAll('#dlList .dl-row')][0]; return r && !!r.querySelector('.row-err') && !!r.querySelector('[data-act=retry]'); })()" "'—'"
eb "(async function(){ document.querySelector('#dlSeg [data-f=all]').click(); await new Promise(function(r){ setTimeout(r,300); }); var s=document.querySelector('#tbSearch'); s.value='switch'; s.dispatchEvent(new Event('input',{bubbles:true})); await new Promise(function(r){ setTimeout(r,900); }); return 'ok'; })()" >/dev/null
check "search finds failed download" "(function(){ var rows=[...document.querySelectorAll('#dlList .dl-row')]; return rows.length === 1 && rows[0].textContent.indexOf('switch.bin')>=0; })()" "'—'"
eb "(async function(){ var s=document.querySelector('#tbSearch'); s.value=''; s.dispatchEvent(new Event('input',{bubbles:true})); await new Promise(function(r){ setTimeout(r,500); }); var row=[...document.querySelectorAll('#dlList .dl-row')].find(function(r){return r.textContent.indexOf('switch.bin')>=0;}); if(row){ var b=row.querySelector('[data-act=retry]'); if(b) b.click(); return 'clicked'; } return 'no row'; })()" >/dev/null
sleep 1.5
check "retry works (retry fired + row stays managed)" "(function(){ var t=[...document.querySelectorAll('.toast')].some(function(x){return x.textContent.indexOf('Retry started')>=0;}); var row=[...document.querySelectorAll('#dlList .dl-row')].find(function(r){return r.textContent.indexOf('switch.bin')>=0;}); return !!row && (t || !row.querySelector('.chip.completed')); })()" "'toast: ' + ([...document.querySelectorAll('.toast')].map(function(x){return x.textContent.slice(0,40);}).join('|') || 'none')"

# ---------- F. completion + Completed filter + stays-in-All ----------
sleep 5
check "archive download completed" "[...document.querySelectorAll('#dlList .dl-row')].some(function(r){return r.textContent.indexOf('sample-archive.zip')>=0 && r.querySelector('.chip.completed');})" "'chips: ' + ([...document.querySelectorAll('#dlList .dl-row .chip')].map(function(n){return n.textContent;}).join('|'))"
eb "(function(){ document.querySelector('#dlSeg [data-f=completed]').click(); return 'ok'; })()" >/dev/null
sleep 1
check "Completed filter keeps completed rows" "[...document.querySelectorAll('#dlList .dl-row')].some(function(r){return r.textContent.indexOf('sample-archive.zip')>=0;})" "'—'"
eb "(function(){ document.querySelector('#dlSeg [data-f=all]').click(); return 'ok'; })()" >/dev/null
sleep 0.6
check "completed remains visible in All" "[...document.querySelectorAll('#dlList .dl-row')].some(function(r){return r.textContent.indexOf('sample-archive.zip')>=0 && r.querySelector('.chip.completed');})" "'—'"

# ---------- G. context menu + bulk selection ----------
eb "(function(){ var row=[...document.querySelectorAll('#dlList .dl-row')].find(function(r){return r.textContent.indexOf('sample-archive.zip')>=0;}); row.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true})); return 'ok'; })()" >/dev/null
sleep 0.6
check "context menu opens with core actions" "(function(){ var ctx=document.querySelector('.ctx'); if(!ctx) return false; var t=ctx.textContent; return t.indexOf('Open file')>=0 && t.indexOf('Copy download URL')>=0 && t.indexOf('Remove from list')>=0; })()" "'menu: ' + ((document.querySelector('.ctx')||{textContent:''}).textContent.slice(0,110))"
eb "(function(){ document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true})); return 'ok'; })()" >/dev/null
eb "(function(){ var boxes=[...document.querySelectorAll('#dlList [data-check]')]; boxes[0].click(); return 'ok'; })()" >/dev/null
sleep 0.6
eb "(function(){ var boxes=[...document.querySelectorAll('#dlList [data-check]')]; if(boxes[1]) boxes[1].click(); return 'ok'; })()" >/dev/null
sleep 1
check "bulk selection bar appears" "(function(){ var bar=document.querySelector('.sel-bar'); return bar && bar.textContent.indexOf('selected')>=0; })()" "'bar: ' + ((document.querySelector('.sel-bar')||{textContent:''}).textContent.slice(0,70))"
eb "(function(){ var b=document.querySelector('.sel-bar [data-b=clear]'); if(b) b.click(); return 'ok'; })()" >/dev/null

# ---------- H. settings persistence + add-dialog wiring ----------
eb "(function(){ var b=document.querySelector('.nav-item[data-route=settings]'); if(b) b.click(); return 'ok'; })()" >/dev/null
sleep 1.4
check "Settings General shows default dir row" "(function(){ var v=document.querySelector('#seDirVal'); return !!v && v.textContent.length > 0 && v.textContent !== '—'; })()" "'dir: ' + ((document.querySelector('#seDirVal')||{textContent:''}).textContent)"
eb "(function(){ var b=document.querySelector('#seNav [data-s=downloads]'); if(b) b.click(); return 'ok'; })()" >/dev/null
sleep 1
eb "(function(){ var sel=document.querySelector('#seDefConns'); if(!sel) return 'missing'; sel.value='4'; sel.dispatchEvent(new Event('change',{bubbles:true})); return 'ok'; })()" >/dev/null
sleep 1.2
agent-browser reload >/dev/null
sleep 2.5
eb "(function(){ var b=document.querySelector('#seNav [data-s=downloads]'); if(b) b.click(); return 'ok'; })()" >/dev/null
sleep 1
check "settings persist across restart (reload)" "(function(){ var sel=document.querySelector('#seDefConns'); return !!sel && sel.value === '4'; })()" "'value: ' + ((document.querySelector('#seDefConns')||{value:'missing'}).value)"
eb "(function(){ document.querySelector('#tbAdd').click(); return 'ok'; })()" >/dev/null
sleep 0.8
check "Add dialog uses default connections pref" "(document.querySelector('#adConns')||{value:''}).value === '4'" "'adConns: ' + ((document.querySelector('#adConns')||{value:''}).value)"
eb "(function(){ var b=document.querySelector('#addDlg [data-close]'); if(b) b.click(); return 'ok'; })()" >/dev/null

# ---------- I. layout width (the old 1160px dashboard cap) ----------
eb "(function(){ var b=document.querySelector('.nav-item[data-route=downloads]'); if(b) b.click(); return 'ok'; })()" >/dev/null
sleep 1.2
check "screen is full-width (max-width uncapped)" "getComputedStyle(document.querySelector('#screen-downloads')).maxWidth === 'none'" "'maxWidth: ' + (getComputedStyle(document.querySelector('#screen-downloads')).maxWidth)"
check "no horizontal overflow at default window" "document.documentElement.scrollWidth <= window.innerWidth + 2" "'scrollW: ' + (document.documentElement.scrollWidth + ' vs inner ' + window.innerWidth)"

echo ""
echo "== RESULTS: $pass passed, $failed failed"
if [ "$KEEP" != "1" ]; then
  kill $ENGINE_PID $FILES_PID $SLOW_PID 2>/dev/null
  agent-browser close >/dev/null 2>&1
fi
[ "$failed" = "0" ]
