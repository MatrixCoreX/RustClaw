"""Probe a sandboxed frame without requiring an unavailable WebView2 callback."""
SCRIPT = """
const done = arguments[arguments.length - 1];
const probe = {
  subframe: window !== window.top,
  webview2: typeof window.chrome?.webview?.postMessage === 'function',
  read: 'unavailable', write: 'unavailable'
};
window.__desktopIframeProbe = probe;
if (typeof window.__TAURI_INTERNALS__?.invoke !== 'function') { done(probe); return; }
probe.read = 'pending'; probe.write = 'pending';
let finished = false;
const finish = () => { if (!finished) { finished = true; done({...probe}); } };
const attempt = (key, command, args) => {
  try {
    return window.__TAURI_INTERNALS__.invoke(command, args)
      .then(() => { probe[key] = 'resolved'; }, () => { probe[key] = 'rejected'; });
  } catch (_) { probe[key] = 'rejected'; return Promise.resolve(); }
};
Promise.all([
  attempt('read', 'profiles', {}),
  attempt('write', 'add_profile', {
    alias: 'Forbidden iframe profile',
    connection: {kind: 'local', origin: 'http://127.0.0.1:8788'}
  })
]).then(finish);
setTimeout(finish, 1500);
"""


def validate(probe, windows):
    assert probe['subframe'], probe
    for operation in ['read', 'write']:
        result = probe[operation]
        if result == 'pending':
            # Wry 0.55 registers only the top-level WebMessageReceived event.
            # WebView2 subframes need their own handler to deliver the message.
            # The caller MUST also compare the real main-window profile state.
            assert windows and probe['webview2'], probe
        else:
            assert result in ['unavailable', 'rejected'], probe
