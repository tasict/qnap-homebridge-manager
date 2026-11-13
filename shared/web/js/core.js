/* Homebridge UI - core: the HB namespace, DOM and SVG builders, icons, the
 * API client and small formatting helpers. Everything that comes from the
 * API reaches the page through textContent or attributes, never as markup. */
(function () {
	'use strict';

	const HB = window.HB = {
		/* Marks an English key kept in a table; translated where it is shown (see i18n.js). */
		N_: (key) => key,
		session: null,
		settings: null,
		networks: null,
		instances: [],
		discovered: [],
	};

	// ---------- DOM ----------

	const PROPS = new Set(['value', 'checked', 'disabled', 'selected', 'hidden', 'indeterminate', 'open', 'readOnly', 'required', 'multiple']);

	function setAttrs(el, attrs) {
		for (const k of Object.keys(attrs)) {
			const v = attrs[k];
			if (v === null || v === undefined || v === false) {
				continue;
			}
			if (k === 'class') {
				el.setAttribute('class', Array.isArray(v) ? v.filter(Boolean).join(' ') : v);
			} else if (k === 'text') {
				el.textContent = String(v);
			} else if (k === 'style' && typeof v === 'object') {
				for (const s of Object.keys(v)) {
					if (s.startsWith('--')) {
						el.style.setProperty(s, v[s]);
					} else {
						el.style[s] = v[s];
					}
				}
			} else if (k === 'dataset') {
				for (const d of Object.keys(v)) {
					el.dataset[d] = v[d];
				}
			} else if (k.startsWith('on') && typeof v === 'function') {
				el.addEventListener(k.slice(2), v);
			} else if (PROPS.has(k)) {
				el[k] = v;
			} else {
				el.setAttribute(k, v === true ? '' : String(v));
			}
		}
	}

	function append(el, children) {
		if (children === null || children === undefined || children === false) {
			return el;
		}
		if (!Array.isArray(children)) {
			children = [children];
		}
		for (const c of children) {
			if (c === null || c === undefined || c === false) {
				continue;
			}
			if (Array.isArray(c)) {
				append(el, c);
			} else if (c instanceof Node) {
				el.appendChild(c);
			} else {
				el.appendChild(document.createTextNode(String(c)));
			}
		}
		return el;
	}

	/* h('div', { class: 'x', onclick: fn }, [children]) */
	function h(tag, attrs, children) {
		const el = document.createElement(tag);
		if (attrs) {
			setAttrs(el, attrs);
		}
		return append(el, children);
	}

	const SVG_NS = 'http://www.w3.org/2000/svg';

	function s(tag, attrs, children) {
		const el = document.createElementNS(SVG_NS, tag);
		if (attrs) {
			for (const k of Object.keys(attrs)) {
				const v = attrs[k];
				if (v === null || v === undefined || v === false) {
					continue;
				}
				if (k === 'text') {
					el.textContent = String(v);
				} else if (k === 'class') {
					el.setAttribute('class', Array.isArray(v) ? v.filter(Boolean).join(' ') : v);
				} else {
					el.setAttribute(k, String(v));
				}
			}
		}
		return append(el, children);
	}

	function clear(el) {
		while (el.firstChild) {
			el.removeChild(el.firstChild);
		}
		return el;
	}

	HB.h = h;
	HB.s = s;
	HB.append = append;
	HB.clear = clear;
	HB.$ = (sel, root) => (root || document).querySelector(sel);
	HB.$$ = (sel, root) => Array.from((root || document).querySelectorAll(sel));

	let uid = 0;
	HB.uid = (prefix) => (prefix || 'hb') + '-' + (++uid);

	// ---------- Icons (24 px grid, drawn with the current text colour) ----------

	const ICONS = {
		plus: [['p', 'M12 5v14M5 12h14']],
		power: [['p', 'M12 3.5v8'], ['p', 'M6.6 6.8a7.5 7.5 0 1 0 10.8 0']],
		play: [['p', 'M8 5.8v12.4a.8.8 0 0 0 1.2.7l10-6.2a.8.8 0 0 0 0-1.4l-10-6.2a.8.8 0 0 0-1.2.7z', true]],
		stop: [['r', 6.5, 6.5, 11, 11, 2, true]],
		restart: [['p', 'M19.5 12a7.5 7.5 0 1 1-2.2-5.3'], ['p', 'M19.5 4.5v4h-4']],
		refresh: [['p', 'M4.5 11a7.5 7.5 0 0 1 13-4.2l2 2'], ['p', 'M19.5 4.5v4.3h-4.3'], ['p', 'M19.5 13a7.5 7.5 0 0 1-13 4.2l-2-2'], ['p', 'M4.5 19.5v-4.3h4.3']],
		open: [['p', 'M13.5 4.5h6v6'], ['p', 'M19.5 4.5l-8.5 8.5'], ['p', 'M18 14v4.5a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 4 18.5v-11A1.5 1.5 0 0 1 5.5 6H10']],
		more: [['c', 5.5, 12, 1.6, true], ['c', 12, 12, 1.6, true], ['c', 18.5, 12, 1.6, true]],
		sliders: [['p', 'M4 7h9M17.5 7H20M4 17h3M11.5 17H20'], ['c', 15.2, 7, 2.2], ['c', 9.2, 17, 2.2]],
		sun: [['c', 12, 12, 4], ['p', 'M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4']],
		moon: [['p', 'M19.5 14.3A7.8 7.8 0 0 1 9.7 4.5a7.8 7.8 0 1 0 9.8 9.8z']],
		auto: [['c', 12, 12, 8], ['p', 'M12 4a8 8 0 0 1 0 16z', true]],
		folder: [['p', 'M3.5 7.5a2 2 0 0 1 2-2h3.8l2 2h7.2a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z']],
		folderPlus: [['p', 'M3.5 7.5a2 2 0 0 1 2-2h3.8l2 2h7.2a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z'], ['p', 'M12 10.5v5M9.5 13h5']],
		check: [['p', 'M5 12.5l4.5 4.5L19 7.5']],
		x: [['p', 'M6.5 6.5l11 11M17.5 6.5l-11 11']],
		alert: [['p', 'M10.3 4.6a2 2 0 0 1 3.4 0l7.1 12.4a2 2 0 0 1-1.7 3H4.9a2 2 0 0 1-1.7-3z'], ['p', 'M12 9.5v4M12 16.8v.01']],
		info: [['c', 12, 12, 8.5], ['p', 'M12 11v5M12 7.9v.01']],
		copy: [['r', 8.5, 8.5, 11, 11, 2], ['p', 'M15.5 8.5V6.5a2 2 0 0 0-2-2h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2']],
		eye: [['p', 'M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z'], ['c', 12, 12, 2.8]],
		eyeOff: [['p', 'M4 4l16 16'], ['p', 'M10 5.7a9.7 9.7 0 0 1 2-.2c6 0 9.5 6.5 9.5 6.5a16 16 0 0 1-2.6 3.4M6.6 6.9C4 8.6 2.5 12 2.5 12S6 18.5 12 18.5a9 9 0 0 0 4.1-1']],
		chevronRight: [['p', 'M9.5 6l6 6-6 6']],
		chevronLeft: [['p', 'M14.5 6l-6 6 6 6']],
		chevronDown: [['p', 'M6 9.5l6 6 6-6']],
		trash: [['p', 'M4.5 7h15M9.5 7V4.5h5V7M6.5 7l.9 12.2a1.5 1.5 0 0 0 1.5 1.3h6.2a1.5 1.5 0 0 0 1.5-1.3L17.5 7']],
		upgrade: [['c', 12, 12, 8.5], ['p', 'M12 16V8.5M8.8 11.5L12 8.3l3.2 3.2']],
		parcel: [['p', 'M3.5 8L12 4l8.5 4v8.5L12 20.5l-8.5-4z'], ['p', 'M3.5 8l8.5 4 8.5-4M12 12v8.5M7.8 6l8.4 4']],
		undo: [['p', 'M9 14.5l-5-5 5-5'], ['p', 'M4 9.5h10.5a5.5 5.5 0 0 1 0 11H11']],
		logs: [['p', 'M5 6.5h14M5 10.5h14M5 14.5h9M5 18.5h11']],
		globe: [['c', 12, 12, 8.5], ['p', 'M3.5 12h17M12 3.5c2.4 2.4 3.5 5.3 3.5 8.5s-1.1 6.1-3.5 8.5c-2.4-2.4-3.5-5.3-3.5-8.5s1.1-6.1 3.5-8.5z']],
		pencil: [['p', 'M4.5 19.5h4l10-10a2.8 2.8 0 0 0-4-4l-10 10z'], ['p', 'M13 7l4 4']],
		shield: [['p', 'M12 3.5l7.5 3v5.7c0 4.2-3.1 7.6-7.5 8.3-4.4-.7-7.5-4.1-7.5-8.3V6.5z']],
		home: [['p', 'M4 11l8-6.5 8 6.5'], ['p', 'M6.5 9.5v10h11v-10']],
		download: [['p', 'M12 4.5v10.5M7.5 10.5L12 15l4.5-4.5M5 19.5h14']],
		key: [['c', 8, 15, 3.5], ['p', 'M10.5 12.5L19 4M16 7l2.5 2.5M14 9l2 2']],
		sparkle: [['p', 'M12 3.5l1.8 5.2 5.2 1.8-5.2 1.8L12 17.5l-1.8-5.2L5 10.5l5.2-1.8z', true]],
	};

	HB.icon = function (name, opts) {
		opts = opts || {};
		const def = ICONS[name] || ICONS.info;
		const el = s('svg', { viewBox: '0 0 24 24', class: ['icon', opts.class], 'aria-hidden': 'true', focusable: 'false', width: opts.size || 18, height: opts.size || 18 });
		for (const part of def) {
			let node;
			const filled = part[part.length - 1] === true;
			if (part[0] === 'p') {
				node = s('path', { d: part[1] });
			} else if (part[0] === 'c') {
				node = s('circle', { cx: part[1], cy: part[2], r: part[3] });
			} else {
				node = s('rect', { x: part[1], y: part[2], width: part[3], height: part[4], rx: part[5] });
			}
			if (filled) {
				node.setAttribute('class', 'fill');
			}
			el.appendChild(node);
		}
		return el;
	};

	// ---------- API ----------

	/* msg: the Msg to show ({ key, args }); fields: path -> Msg; message: plain English. */
	class ApiError extends Error {
		constructor(status, code, msg, fields, details, message) {
			super(message || (msg && msg.key) || code);
			this.status = status;
			this.code = code;
			this.msg = msg || null;
			this.fields = fields || null;
			this.details = details || null;
			this.handled = false;
		}
	}
	HB.ApiError = ApiError;

	/* Errors that replace the whole page (session ended, not an administrator,
	 * Container Station down). app.js installs the handler. */
	HB.onFatal = null;

	function isFatal(err) {
		return err.status === 401
			|| (err.status === 403 && err.code === 'admin_required')
			|| (err.status === 503 && (err.code === 'docker_unavailable' || err.code === 'qts_unavailable'));
	}

	async function request(method, path, body, query) {
		let url = 'api' + path;
		if (query) {
			const q = new URLSearchParams();
			for (const k of Object.keys(query)) {
				if (query[k] !== undefined && query[k] !== null) {
					q.set(k, String(query[k]));
				}
			}
			const qs = q.toString();
			if (qs) {
				url += '?' + qs;
			}
		}
		const init = { method: method === 'GET' ? 'GET' : 'POST', credentials: 'same-origin', headers: { Accept: 'application/json' }, cache: 'no-store' };
		if (method !== 'GET') {
			init.headers['X-HB-Request'] = '1';
			if (method === 'PUT' || method === 'DELETE') {
				init.headers['X-HB-Method'] = method;
			}
			if (body !== undefined) {
				init.headers['Content-Type'] = 'application/json';
				init.body = JSON.stringify(body);
			}
		}
		let res;
		try {
			res = await fetch(url, init);
		} catch (e) {
			throw new ApiError(0, 'network', { key: HB.N_('Cannot reach the NAS. Check the network connection and try again.') });
		}
		let env = null;
		try {
			env = await res.json();
		} catch (e) {
			env = null;
		}
		if (env && env.ok === true) {
			return env.data;
		}
		let err;
		if (env && env.error) {
			const e = env.error;
			const msg = e.msg || (e.message ? { key: e.message } : { key: HB.N_('Something went wrong ({code})'), args: { code: e.code || res.status } });
			err = new ApiError(res.status, e.code, msg, e.fields, e.details, e.message);
		} else if (res.status === 401) {
			err = new ApiError(401, 'auth_required', { key: HB.N_('Your QTS session has ended') });
		} else {
			err = new ApiError(res.status, 'http', { key: HB.N_('The NAS did not answer properly (HTTP {status}). Try again in a moment.'), args: { status: res.status } });
		}
		if (isFatal(err) && HB.onFatal) {
			err.handled = true;
			HB.onFatal(err);
		}
		throw err;
	}

	HB.api = {
		get: (path, query) => request('GET', path, undefined, query),
		post: (path, body) => request('POST', path, body === undefined ? {} : body),
		put: (path, body) => request('PUT', path, body),
		del: (path, query) => request('DELETE', path + (query ? '?' + new URLSearchParams(query).toString() : ''), undefined),
	};

	HB.enc = encodeURIComponent;

	// ---------- Formatting ----------

	HB.parseTime = function (iso) {
		if (!iso) {
			return null;
		}
		const t = Date.parse(iso);
		return isNaN(t) ? null : t;
	};

	HB.bytes = function (n) {
		if (n === null || n === undefined) {
			return '';
		}
		if (n < 1024 * 1024) {
			return Math.max(1, Math.round(n / 1024)) + ' KB';
		}
		if (n < 1024 * 1024 * 1024) {
			return Math.round(n / 1048576) + ' MB';
		}
		return (n / 1073741824).toFixed(1) + ' GB';
	};

	/* "1f.example.com" stays; long names lose their middle. */
	HB.middleTruncate = function (text, max) {
		if (!text || text.length <= max) {
			return text || '';
		}
		const keep = max - 1;
		const head = Math.ceil(keep / 2);
		return text.slice(0, head) + '…' + text.slice(text.length - (keep - head));
	};

	HB.deepCopy = (v) => (v === undefined ? undefined : JSON.parse(JSON.stringify(v)));

	HB.debounce = function (fn, ms) {
		let t = null;
		const d = function () {
			const args = arguments;
			clearTimeout(t);
			t = setTimeout(() => fn.apply(null, args), ms);
		};
		d.cancel = () => clearTimeout(t);
		return d;
	};

	HB.firstGrapheme = function (text) {
		text = (text || '').trim();
		if (!text) {
			return '';
		}
		if (window.Intl && Intl.Segmenter) {
			const it = new Intl.Segmenter('zh-Hant', { granularity: 'grapheme' }).segment(text)[Symbol.iterator]();
			const first = it.next();
			return first.done ? '' : first.value.segment;
		}
		return Array.from(text)[0];
	};

	HB.byteLength = (text) => new TextEncoder().encode(text || '').length;

	/* Copy text; navigator.clipboard needs a secure context, which QTS over
	 * plain HTTP is not, so fall back to a hidden textarea. */
	HB.copyText = async function (text) {
		if (navigator.clipboard && window.isSecureContext) {
			try {
				await navigator.clipboard.writeText(text);
				return true;
			} catch (e) {
				/* fall through */
			}
		}
		const ta = h('textarea', { class: 'offscreen', readonly: true, 'aria-hidden': 'true' });
		ta.value = text;
		document.body.appendChild(ta);
		ta.select();
		let ok = false;
		try {
			ok = document.execCommand('copy');
		} catch (e) {
			ok = false;
		}
		ta.remove();
		return ok;
	};

	HB.reducedMotion = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches;

	// ---------- Domain words ----------

	/* Keys are translated where they are shown (HB.t), so a language switch follows. */
	HB.COLORS = [
		['violet', HB.N_('Violet')], ['orange', HB.N_('Orange')], ['teal', HB.N_('Teal')], ['pink', HB.N_('Pink')],
		['blue', HB.N_('Blue')], ['green', HB.N_('Green')], ['amber', HB.N_('Amber')], ['slate', HB.N_('Slate')],
	];

	HB.EMOJI = ['🏠', '🏡', '🏘️', '🏢', '🛋️', '🛏️', '🍳', '🛁', '🚗', '🌿', '🪴', '☀️', '🌙', '💡', '🔌', '🌡️', '📺', '🎮', '🐱', '🐶', '🧪', '🧸', '📚', '🔒'];

	/* "Upgrading 42%": the status line while a job runs on a house. */
	HB.JOB_STATUS = {
		create: HB.N_('Creating {percent}'), apply: HB.N_('Recreating {percent}'), upgrade: HB.N_('Upgrading {percent}'),
		rollback: HB.N_('Restoring {percent}'), remove: HB.N_('Removing {percent}'), check_updates: HB.N_('Checking {percent}'),
	};
	/* Job dialog titles: "Upgrade 「一樓」". */
	HB.JOB_TITLE = {
		create: HB.N_('Create “{name}”'), apply: HB.N_('Apply and recreate “{name}”'), upgrade: HB.N_('Upgrade “{name}”'),
		rollback: HB.N_('Restore “{name}” from the backup'), remove: HB.N_('Remove “{name}”'), check_updates: HB.N_('Check for updates'),
	};
	HB.jobTitle = (kind, name) => HB.t(HB.JOB_TITLE[kind] || HB.N_('Progress'), { name });

	HB.displayName = (inst) => (inst.spec.display_name || inst.spec.name);

	HB.findInstance = (name) => HB.instances.find((i) => i.spec.name === name) || null;
})();
