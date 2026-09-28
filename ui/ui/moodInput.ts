import { h, icon, ICONS } from './dom';

export const MAX_MOOD_LENGTH = 200;

export interface MoodInput {
  el: HTMLElement;
  setValue(value: string): void;
  focus(): void;
  setHasApiKey(hasKey: boolean): void;
  setGenerating(generating: boolean): void;
  setError(message: string | null): void;
  /** A playlist is on screen, so the clear button is offered even with an empty input. */
  setClearable(clearable: boolean): void;
}

export function createMoodInput(opts: {
  onGenerate: (mood: string) => void;
  onOpenSettings: () => void;
  /** Clear button clicked. */
  onClear: () => void;
}): MoodInput {
  const input = h('input', {
    class: 'mood-input',
    type: 'text',
    placeholder: "What's the vibe?",
    maxlength: String(MAX_MOOD_LENGTH),
    'aria-label': 'Mood',
    autocomplete: 'off',
    spellcheck: 'false',
  });
  const button = h('button', { class: 'generate-btn', type: 'submit', 'aria-label': 'Generate', title: 'Generate' });
  const clearBtn = h(
    'button',
    { class: 'clear-btn', type: 'button', 'aria-label': 'Clear', title: 'Clear' },
    icon(ICONS.close),
  );
  const form = h('form', { class: 'mood-form' }, input, clearBtn, button);
  const message = h('div', { class: 'mood-message', role: 'status' });
  const el = h('section', { class: 'mood' }, form, message);

  let hasKey = false;
  let generating = false;
  let error: string | null = null;
  let clearable = false;

  const render = () => {
    input.disabled = generating;
    const showClear = !generating && (clearable || input.value.length > 0);
    clearBtn.hidden = !showClear;
    form.classList.toggle('has-clear', showClear);
    button.disabled = generating || !hasKey || input.value.trim().length === 0;
    button.classList.toggle('is-loading', generating);
    button.replaceChildren(generating ? h('span', { class: 'spinner spinner--dark' }) : icon(ICONS.arrow));

    message.replaceChildren();
    message.classList.toggle('is-error', error !== null);
    if (error !== null) {
      message.textContent = error;
    } else if (!hasKey) {
      const link = h('button', { class: 'link-btn', type: 'button' }, 'Settings');
      link.addEventListener('click', opts.onOpenSettings);
      message.append('Add your OpenAI API key in ', link);
    }
  };

  input.addEventListener('input', () => {
    error = null;
    render();
  });
  clearBtn.addEventListener('click', opts.onClear);
  form.addEventListener('submit', (e) => {
    e.preventDefault();
    const mood = input.value.trim();
    if (!mood || generating || !hasKey) return;
    opts.onGenerate(mood);
  });

  render();
  return {
    el,
    setValue(value) {
      input.value = value.slice(0, MAX_MOOD_LENGTH);
      error = null;
      render();
    },
    focus: () => input.focus(),
    setHasApiKey(v) {
      hasKey = v;
      render();
    },
    setGenerating(v) {
      generating = v;
      render();
      if (!v) input.focus();
    },
    setError(msg) {
      error = msg;
      render();
    },
    setClearable(v) {
      clearable = v;
      render();
    },
  };
}
