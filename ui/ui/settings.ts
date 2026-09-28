import { api, errorMessage, events, type Settings, type ToolInfo, type ToolsStatus } from '../api';
import { formatBytes, h, icon, ICONS } from './dom';

export interface SettingsPanel {
  el: HTMLElement;
  open(notice?: string): void;
  close(): void;
  isOpen(): boolean;
}

export function createSettings(opts: {
  /** Called after anything that changes `get_settings` / tools state. */
  onChanged: () => void;
}): SettingsPanel {
  // --- API key
  const notice = h('p', { class: 'settings-notice', hidden: true });
  const keyInput = h('input', {
    class: 'pill-input',
    type: 'password',
    placeholder: 'sk-…',
    autocomplete: 'off',
    spellcheck: 'false',
    'aria-label': 'OpenAI API key',
    id: 'api-key',
  });
  const reveal = h('button', { class: 'reveal-btn', type: 'button', 'aria-label': 'Show key' }, icon(ICONS.eye));
  const keyStatus = h('p', { class: 'field-status', role: 'status' });
  const removeKey = h('button', { class: 'text-btn', type: 'button', hidden: true }, 'Remove key');

  // --- Model
  const modelInput = h('input', {
    class: 'pill-input',
    type: 'text',
    autocomplete: 'off',
    spellcheck: 'false',
    'aria-label': 'Model',
    id: 'model',
  });

  // --- Tools
  const ytDlpLine = h('span', { class: 'kv-value' });
  const ffmpegLine = h('span', { class: 'kv-value' });
  const toolsStatus = h('p', { class: 'field-status', role: 'status' });
  const installBtn = h('button', { class: 'outline-pill', type: 'button' }, 'Install / Update');

  // --- Storage
  const dataDir = h('span', { class: 'kv-value' });
  const cacheSize = h('span', { class: 'kv-value' });
  const clearBtn = h('button', { class: 'outline-pill', type: 'button' }, 'Clear cache');

  const saveBtn = h('button', { class: 'primary-pill', type: 'submit' }, 'Save');
  const closeBtn = h('button', { class: 'dark-pill', type: 'button' }, 'Close');

  const dialog = h(
    'form',
    { class: 'dialog', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'settings-title' },
    h('h2', { class: 'dialog-title', id: 'settings-title' }, 'Settings'),
    notice,
    h(
      'section',
      { class: 'field' },
      h('label', { class: 'field-label', for: 'api-key' }, 'OpenAI API key'),
      h('div', { class: 'input-with-btn' }, keyInput, reveal),
      h('div', { class: 'field-row' }, keyStatus, removeKey),
    ),
    h(
      'section',
      { class: 'field' },
      h('label', { class: 'field-label', for: 'model' }, 'Model'),
      modelInput,
    ),
    h(
      'section',
      { class: 'field' },
      h('span', { class: 'field-label' }, 'Tools'),
      h('div', { class: 'kv' }, h('span', {}, 'yt-dlp'), ytDlpLine),
      h('div', { class: 'kv' }, h('span', {}, 'ffmpeg'), ffmpegLine),
      h('div', { class: 'field-row' }, toolsStatus, installBtn),
    ),
    h(
      'section',
      { class: 'field' },
      h('span', { class: 'field-label' }, 'Storage'),
      h('div', { class: 'kv' }, h('span', {}, 'Location'), dataDir),
      h('div', { class: 'kv' }, h('span', {}, 'Cache'), cacheSize),
      h('div', { class: 'field-row field-row--end' }, clearBtn),
    ),
    h('div', { class: 'dialog-actions' }, closeBtn, saveBtn),
  );
  const el = h('div', { class: 'overlay', hidden: true }, dialog);

  let settings: Settings | null = null;
  let busy = false;

  const setStatus = (node: HTMLElement, text: string, isError = false) => {
    node.textContent = text;
    node.classList.toggle('is-error', isError);
  };

  const toolLabel = (t: ToolInfo) =>
    t.version ? `${t.version}${t.managed ? '' : ' (system)'}` : 'missing';

  const renderTools = (tools: ToolsStatus) => {
    ytDlpLine.textContent = toolLabel(tools.ytDlp);
    ffmpegLine.textContent = toolLabel(tools.ffmpeg);
    ytDlpLine.classList.toggle('is-error', !tools.ytDlp.version);
    ffmpegLine.classList.toggle('is-error', !tools.ffmpeg.version);
  };

  const refresh = async () => {
    settings = await api.getSettings();
    modelInput.value = settings.model;
    removeKey.hidden = !settings.hasApiKey;
    if (!keyStatus.classList.contains('is-error')) {
      setStatus(keyStatus, settings.hasApiKey ? 'Key saved' : '');
    }
    dataDir.textContent = settings.dataDir;
    dataDir.title = settings.dataDir;
    cacheSize.textContent = `${formatBytes(settings.cacheSizeBytes)} of ${formatBytes(settings.cacheMaxBytes)}`;
    renderTools(await api.getToolsStatus());
  };

  const setBusy = (v: boolean) => {
    busy = v;
    for (const b of [saveBtn, removeKey, installBtn, clearBtn]) b.disabled = v;
  };

  reveal.addEventListener('click', () => {
    const show = keyInput.type === 'password';
    keyInput.type = show ? 'text' : 'password';
    reveal.setAttribute('aria-label', show ? 'Hide key' : 'Show key');
    reveal.replaceChildren(icon(show ? ICONS.eyeOff : ICONS.eye));
  });

  dialog.addEventListener('submit', async (e) => {
    e.preventDefault();
    if (busy) return;
    setBusy(true);
    try {
      const model = modelInput.value.trim();
      if (settings && model !== settings.model) await api.setModel(model);
      const key = keyInput.value.trim();
      if (key) {
        setStatus(keyStatus, 'Checking key…');
        await api.saveApiKey(key);
        keyInput.value = '';
        setStatus(keyStatus, 'Key saved');
        opts.onChanged();
        panel.close();
        return;
      }
      opts.onChanged();
      if (settings?.hasApiKey) panel.close();
      else setStatus(keyStatus, 'Paste your OpenAI API key first.', true);
    } catch (err) {
      setStatus(keyStatus, errorMessage(err), true);
    } finally {
      setBusy(false);
      void refresh();
    }
  });

  removeKey.addEventListener('click', async () => {
    setBusy(true);
    try {
      await api.removeApiKey();
      setStatus(keyStatus, 'Key removed');
      opts.onChanged();
    } catch (err) {
      setStatus(keyStatus, errorMessage(err), true);
    } finally {
      setBusy(false);
      void refresh();
    }
  });

  installBtn.addEventListener('click', async () => {
    setBusy(true);
    setStatus(toolsStatus, 'Working…');
    try {
      await api.installTools();
      setStatus(toolsStatus, 'Tools are ready');
      opts.onChanged();
    } catch (err) {
      setStatus(toolsStatus, errorMessage(err), true);
    } finally {
      setBusy(false);
      void refresh();
    }
  });
  void events.onToolsProgress((p) => {
    if (busy && !el.hidden) setStatus(toolsStatus, `${p.message} ${p.progress < 100 ? `${Math.floor(p.progress)}%` : ''}`);
  });

  clearBtn.addEventListener('click', async () => {
    setBusy(true);
    try {
      const { freedBytes } = await api.clearCache();
      cacheSize.textContent = `Freed ${formatBytes(freedBytes)}`;
    } finally {
      setBusy(false);
      setTimeout(() => void refresh(), 1200);
    }
  });

  closeBtn.addEventListener('click', () => panel.close());
  el.addEventListener('mousedown', (e) => {
    if (e.target === el) panel.close();
  });
  el.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') panel.close();
  });

  const panel: SettingsPanel = {
    el,
    open(message) {
      notice.hidden = !message;
      notice.textContent = message ?? '';
      setStatus(keyStatus, '');
      setStatus(toolsStatus, '');
      el.hidden = false;
      void refresh().then(() => keyInput.focus());
    },
    close() {
      el.hidden = true;
      keyInput.value = '';
    },
    isOpen: () => !el.hidden,
  };
  return panel;
}
