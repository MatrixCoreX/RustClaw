import { invoke } from '@tauri-apps/api/core';
import { copy } from '../frontend/i18n';

// Blob downloads need the Storage Access Framework on Android WebView.
// Fetch immediately, before the UI revokes its own temporary object URL.
const click = HTMLAnchorElement.prototype.click;
let saving = false;
function download(anchor: HTMLAnchorElement): boolean {
  if (!anchor.hasAttribute('download') || !anchor.href.startsWith(`blob:${location.origin}/`)) return false;
  if (saving) return true;
  saving = true;
  const content = fetch(anchor.href).then(response => response.blob());
  void content.then(async blob => {
    if (blob.size > 16 * 1024 * 1024) throw new Error(copy('导出文件超过 16 MB，请分批导出。', 'The export exceeds 16 MB. Export smaller batches.'));
    await invoke('save_export', { filename: anchor.download || 'export', bytes: [...new Uint8Array(await blob.arrayBuffer())] });
  }).catch(() => window.alert(copy('文件未能保存，请检查可用空间后重试。', 'Could not save the file. Check available space and try again.')))
    .finally(() => { saving = false; });
  return true;
}
HTMLAnchorElement.prototype.click = function () { if (!download(this)) click.call(this); };
document.addEventListener('click', event => {
  const anchor = (event.target as Element)?.closest?.('a[download]');
  if (anchor instanceof HTMLAnchorElement && download(anchor)) event.preventDefault();
}, true);
