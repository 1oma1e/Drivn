// Official tools and installers: Java Update (R-SRC-10), the NVIDIA or Intel package opened by hand
// (ADR-0030, ADR-0033), an optional restore point alone first (R-SEC-09), and the update of Drivn.
import { t, errText } from '../lib/i18n.js';
import { isPackage, packageDialog } from '../model/selectors.js';

/** The first dialog of the official installer of row `x`: NVIDIA's or Intel's, or Java Update. */
export async function officialDialog(backend, x) {
  if (isPackage(x)) return packageDialog(x);
  return toolDialog(backend, 'java');
}

async function toolDialog(backend, kind) {
  const check = await backend.toolCheck(kind).catch((e) => ({ run: false, why: String(e), name: kind === 'java' ? 'Java Update' : 'Intel DSA' }));
  return { kind: 'tool', tool: kind, name: check.name, run: check.run, why: check.why && errText(check.why), step: 'ask' };
}

export function toolsController({ store, backend, notify }) {
  const setDialog = (dialog) => store.set({ dialog });
  const current = () => store.get().dialog;
  const now = () => Date.now() / 1000;

  async function launch() {
    const d = current();
    try {
      await backend.toolLaunch(d.tool);
      setDialog(null);
      notify(t(`${d.name} est ouvert. Au prochain scan, Drivn relira la version installée.`, `${d.name} is open. At the next scan, Drivn reads the installed version again.`));
    } catch (e) {
      setDialog({ ...d, step: 'error', why: errText(e) });
    }
  }

  /** A restore point alone (its own UAC), then `next`. */
  async function restoreThen(next) {
    const d = current();
    setDialog({ ...d, step: 'restore', at: now() });
    const r = await backend.restorePointAlone().catch((e) => ({ ok: false, why: errText(e) }));
    if (r.simulation) return setDialog({ ...d, step: 'sim' });
    if (!r.ok) return setDialog({ ...d, step: 'restore_failed', why: r.why });
    notify(t(`Point de restauration créé${r.seq ? ` (n° ${r.seq})` : ''}.`, `Restore point created${r.seq ? ` (#${r.seq})` : ''}.`));
    return next();
  }

  async function runPackage() {
    const d = current();
    setDialog({ ...d, step: 'download', mb: 0, at: now() });
    try {
      await backend.packageInstall(d.id, (p) => {
        const live = current();
        if (live?.kind !== 'package') return;
        if (p.step === 'download') setDialog({ ...live, step: 'download', mb: p.mb });
        else if (p.step === 'check') setDialog({ ...live, step: 'check', at: now() });
      });
      setDialog({ ...current(), step: 'launched' });
    } catch (e) {
      setDialog({ ...current(), step: 'error', why: errText(e) });
    }
  }

  return {
    actions: {
      async 'tool-ask'(el) { setDialog(await toolDialog(backend, el.tool)); store.set({ sel: null }); },
      'tool-run'() { launch(); },
      'tool-restore'() { restoreThen(launch); },
      'tool-page'() { const d = current(); backend.toolPage(d.tool).catch(() => {}); setDialog(null); },
      'update-ask'() { setDialog({ kind: 'update', step: 'ask', version: store.get().update?.version }); },
      async 'update-go'() {
        setDialog({ ...current(), step: 'run', done: 0, total: null, at: now() });
        try {
          await backend.updateInstall((p) => { const live = current(); if (live?.kind === 'update') setDialog({ ...live, done: p.done, total: p.total ?? null }); });
        } catch (e) {
          setDialog({ ...current(), step: 'error', why: errText(e) });
        }
      },
      'package-ask'(el) {
        const x = store.get().result?.items.find((it) => it.dev.id === el.id);
        return x && setDialog(packageDialog(x));
      },
      'package-go'() { runPackage(); },
      'package-restore'() { restoreThen(runPackage); },
    },
  };
}
