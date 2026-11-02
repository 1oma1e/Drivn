// Navigation, selection and settings.
import { t, setLang, errText } from '../lib/i18n.js';
import { view } from '../model/selectors.js';
import { officialDialog } from './tools.js';
import { saveSettings } from '../model/settings.js';

export function appController({ store, backend, notify, applyMotion }) {
  const set = (patch) => store.set(patch);
  const setSettings = (patch) => {
    const settings = { ...store.get().settings, ...patch };
    saveSettings(settings);
    set({ settings });
    applyMotion(settings.motion);
  };

  return {
    nav(d) {
      const s = store.get();
      if (s.scan.phase === 'read' || s.scan.phase === 'search') return;
      set({ screen: ['drivers', 'apps', 'games'].includes(d.to) && !s.result ? 'scan' : d.to, sel: null });
    },
    tab(d) { set({ tab: d.to }); },
    query(value) { set({ query: value }); },
    open(d) {
      set({ sel: d.id, techOpen: false });
      // The installer link of an application, asked to winget once (read only).
      const id = d.id.startsWith('app:') ? d.id.slice(4) : null;
      if (id && store.get().appUrls?.[d.id] === undefined) {
        set((s) => ({ appUrls: { ...s.appUrls, [d.id]: null } }));
        backend.appSource(id)
          .then((url) => set((s) => ({ appUrls: { ...s.appUrls, [d.id]: url ?? t('winget n’a pas donné de lien', 'winget gave no link') } })))
          .catch((e) => set((s) => ({ appUrls: { ...s.appUrls, [d.id]: t(`illisible : ${errText(e)}`, `unreadable: ${errText(e)}`) } })));
      }
    },
    'close-sheet'() { set({ sel: null }); },
    tech() { set((s) => ({ techOpen: !s.techOpen })); },
    pick(d) {
      const s = store.get();
      if (s.pass) return;
      const x = s.result.items.find((it) => it.dev.id === d.id);
      set({ picked: { ...s.picked, [d.id]: !view(s).isPicked(x) } });
    },
    'pick-all'() {
      const s = store.get();
      const v = view(s);
      // R-SRC-06 (ADR-0034): everything, without a confirmation.
      const all = !v.pickable.every(v.isPicked);
      set({ picked: { ...s.picked, ...Object.fromEntries(v.pickable.map((x) => [x.dev.id, all])) } });
    },
    'toggle-pass'(d) {
      const s = store.get();
      const id = Number(d.id);
      const open = s.open[id] ?? s.history[0]?.id === id;
      set({ open: { ...s.open, [id]: !open } });
    },
    motion(d) { setSettings({ motion: d.to }); },
    absent() { setSettings({ showAbsent: !store.get().settings.showAbsent }); },
    all() { setSettings({ showAll: !store.get().settings.showAll }); },
    // The texts built from the last scan are worded again in the new language.
    lang(d) {
      setLang(d.to);
      setSettings({ lang: d.to });
      const again = backend.relabel();
      if (again) set((s) => ({ result: { ...s.result, wu: again.wu, items: again.items, sources: again.sources }, info: again.info }));
    },

    'notice-close'() { set({ notice: null }); },
    async 'dialog-close'() {
      // ADR-0030, ADR-0033, ADR-0023: NVIDIA, Intel or Java ticked with the rest: their installer comes next.
      // Closing the recap cancels the whole selection.
      const s = store.get();
      const next = ['end', 'package', 'tool'].includes(s.dialog?.kind) && s.after?.length
        ? s.result?.items.find((x) => x.dev.id === s.after[0]) : null;
      if (!next) return set({ dialog: null, after: [] });
      set({ dialog: null, after: s.after.slice(1) });
      set({ dialog: await officialDialog(backend, next) });
    },
    // R-RBT-01: a normal restart of Windows, on this click only.
    'reboot-now'() {
      set({ reboot: [], dialog: null });
      backend.rebootNow().catch((e) => notify(t(`Redémarrage pas lancé : ${errText(e)}`, `Restart not started: ${errText(e)}`)));
    },
    // Drivn does not restore the PC itself: it says where Windows does it.
    'system-restore'() {
      notify(t('Restauration du système : Paramètres → Système → Protection du système → Restauration du système.',
        'System Restore: Settings → System → System Protection → System Restore.'));
    },
    'open-protection'() {
      backend.openProtection().catch((e) => notify(t(`Impossible d’ouvrir Protection du système : ${errText(e)}`, `Could not open System Protection: ${errText(e)}`)));
    },
  };
}
