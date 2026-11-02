// Tiny view toolkit: escaped HTML templates + an in-place DOM patcher.
// Patching (instead of innerHTML) keeps existing nodes, so running CSS animations,
// focus and scroll survive a re-render and the browser only repaints what changed.

const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };

class Raw {
  constructor(s) { this.s = s; }
  toString() { return this.s; }
}

/** Marks a string as trusted HTML (icons, attributes built in code). */
export const raw = (s) => new Raw(s);

const show = (v) =>
  v == null || v === false ? '' :
  v instanceof Raw ? v.s :
  Array.isArray(v) ? v.map(show).join('') :
  String(v).replace(/[&<>"']/g, (c) => ESC[c]);

/** Tagged template: every interpolated value is escaped unless it is Raw (or an array of Raw). */
export function html(strings, ...vals) {
  let out = strings[0];
  for (let i = 0; i < vals.length; i++) out += show(vals[i]) + strings[i + 1];
  return new Raw(out);
}

const tpl = document.createElement('template');

/** Makes `el`'s children match `view` while touching as few nodes as possible. */
export function patch(el, view) {
  tpl.innerHTML = String(view);
  morphChildren(el, tpl.content);
}

const keyOf = (n) => (n.nodeType === 1 ? n.getAttribute('data-key') : null);
const same = (a, b) => a.nodeType === b.nodeType && a.nodeName === b.nodeName && keyOf(a) === keyOf(b);

function morphChildren(from, to) {
  let a = from.firstChild;
  let b = to.firstChild;
  while (b) {
    const next = b.nextSibling;
    if (a && same(a, b)) {
      morph(a, b);
      a = a.nextSibling;
    } else {
      // A keyed node may just have moved: reuse it rather than rebuild it.
      const key = keyOf(b);
      let match = null;
      if (key) for (let c = a; c && !match; c = c.nextSibling) if (keyOf(c) === key && same(c, b)) match = c;
      if (match) {
        from.insertBefore(match, a);
        morph(match, b);
      } else {
        from.insertBefore(b, a);
      }
    }
    b = next;
  }
  while (a) {
    const next = a.nextSibling;
    from.removeChild(a);
    a = next;
  }
}

function morph(a, b) {
  if (a.nodeType !== 1) {
    if (a.nodeValue !== b.nodeValue) a.nodeValue = b.nodeValue;
    return;
  }
  for (const { name } of [...a.attributes]) if (!b.hasAttribute(name)) a.removeAttribute(name);
  for (const { name, value } of b.attributes) if (a.getAttribute(name) !== value) a.setAttribute(name, value);
  // A typed value lives in the property, not the attribute: only overwrite it when the user is elsewhere.
  if (a.nodeName === 'INPUT' && a !== document.activeElement && a.value !== b.value) a.value = b.value;
  morphChildren(a, b);
}
