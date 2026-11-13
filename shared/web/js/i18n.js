/* Homebridge UI - languages. Every string the user can see or hear is an
 * English sentence used as the key into a catalog (shared/web/i18n/<lang>.json
 * for the UI, <lang>.backend.json for the backend's messages); a missing
 * translation falls back to the English sentence.
 *
 *   HB.t('Stopped {name}', { name: '一樓' })      -> string
 *   HB.tn('Pairing code {pin}', { pin: node })    -> [text, node, text] for HB.append
 *   HB.N_('Running')                               -> 'Running' (marks a key kept in a table)
 *   HB.msg({ key, args })                          -> a backend Msg in the viewer's language
 *   HB.logLines(text, startedAt)                   -> job log lines, each { time, text, raw }
 *
 * Dates, times, durations and lists use Intl with the active locale. */
(function () {
	'use strict';

	const SUPPORTED = ['zh-TW', 'ja'];
	/* Each language is named in its own words and never translated. */
	const NAMES = { 'zh-TW': '繁體中文', ja: '日本語' };
	const HTML_LANG = { 'zh-TW': 'zh-Hant-TW', ja: 'ja' };
	const LOCALE = { 'zh-TW': 'zh-TW', ja: 'ja-JP' };
	const STORE = 'hb-lang';
	const PLACEHOLDER = /\{([A-Za-z_][A-Za-z0-9_]*)\}/g;

	let lang = 'zh-TW';
	let dict = Object.create(null);

	function lookup(key) {
		return Object.prototype.hasOwnProperty.call(dict, key) && typeof dict[key] === 'string' && dict[key] ? dict[key] : key;
	}

	/* One pass over the template, so a value that itself contains {braces} is never expanded again. */
	function t(key, args) {
		const text = lookup(key);
		if (!args) {
			return text;
		}
		return text.replace(PLACEHOLDER, (m, k) => (args[k] === undefined || args[k] === null ? m : String(args[k])));
	}

	/* Like t(), but values may be DOM nodes (a <strong>, a monospace span…). Returns parts for HB.append. */
	function tn(key, args) {
		const text = lookup(key);
		const out = [];
		let last = 0;
		text.replace(PLACEHOLDER, (m, k, idx) => {
			out.push(text.slice(last, idx));
			const v = args ? args[k] : undefined;
			out.push(v === undefined || v === null ? m : (v instanceof Node ? v : String(v)));
			last = idx + m.length;
			return m;
		});
		out.push(text.slice(last));
		return out.filter((p) => p !== '');
	}

	HB.t = t;
	HB.tn = tn;
	HB.N_ = (key) => key;

	/* A backend Msg ({ key, args }) in the viewer's language. Plain strings pass through
	 * (fields the contract does not list as Msg may still be text). */
	HB.msg = function (m) {
		if (m === null || m === undefined || m === '') {
			return '';
		}
		if (typeof m === 'string') {
			return m;
		}
		if (typeof m === 'object' && typeof m.key === 'string') {
			return t(m.key, m.args || null);
		}
		return String(m);
	};

	/* The text of an API error: its Msg when there is one, else the plain message. */
	HB.errorText = function (err) {
		if (!err) {
			return '';
		}
		if (err.msg) {
			return HB.msg(err.msg);
		}
		return err.message || String(err);
	};

	// ---------- formatting ----------

	const fmtCache = new Map();
	function fmt(kind, opts) {
		const k = lang + '|' + kind + '|' + JSON.stringify(opts || {});
		let f = fmtCache.get(k);
		if (!f) {
			const loc = LOCALE[lang];
			f = kind === 'date' ? new Intl.DateTimeFormat(loc, opts)
				: kind === 'rel' ? new Intl.RelativeTimeFormat(loc, opts)
					: kind === 'list' ? new Intl.ListFormat(loc, opts)
						: new Intl.NumberFormat(loc, opts);
			fmtCache.set(k, f);
		}
		return f;
	}

	HB.locale = () => LOCALE[lang];

	HB.date = function (iso) {
		const ms = HB.parseTime(iso);
		return ms === null ? '' : fmt('date', { year: 'numeric', month: 'numeric', day: 'numeric' }).format(ms);
	};

	HB.dateTime = function (iso) {
		const ms = HB.parseTime(iso);
		return ms === null ? '' : fmt('date', { year: 'numeric', month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }).format(ms);
	};

	/* Month, day and time, for log lines. */
	HB.shortTime = function (ms) {
		return fmt('date', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23' }).format(ms);
	};

	HB.clockTime = function (ms) {
		return fmt('date', { hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23' }).format(ms);
	};

	/* "3 分鐘前", "昨天", "3 日前" */
	HB.ago = function (iso) {
		const ms = HB.parseTime(iso);
		if (ms === null) {
			return '';
		}
		const s = Math.max(0, (Date.now() - ms) / 1000);
		if (s < 45) {
			return t('just now');
		}
		const rel = fmt('rel', { numeric: 'auto' });
		if (s < 3600) {
			return rel.format(-Math.max(1, Math.round(s / 60)), 'minute');
		}
		if (s < 86400) {
			return rel.format(-Math.round(s / 3600), 'hour');
		}
		const d = Math.floor(s / 86400);
		if (d < 30) {
			return rel.format(-d, 'day');
		}
		return HB.date(iso);
	};

	/* How long since: "5 分鐘", "3 小時", "2 週", "4 か月" */
	HB.since = function (iso) {
		const ms = HB.parseTime(iso);
		if (ms === null) {
			return '';
		}
		const s = Math.max(0, (Date.now() - ms) / 1000);
		const unit = (n, u) => fmt('num', { style: 'unit', unit: u, unitDisplay: 'long' }).format(n);
		if (s < 60) {
			return t('less than a minute');
		}
		if (s < 3600) {
			return unit(Math.floor(s / 60), 'minute');
		}
		if (s < 86400) {
			return unit(Math.floor(s / 3600), 'hour');
		}
		const d = Math.floor(s / 86400);
		if (d < 14) {
			return unit(d, 'day');
		}
		if (d < 60) {
			return unit(Math.floor(d / 7), 'week');
		}
		if (d < 730) {
			return unit(Math.floor(d / 30), 'month');
		}
		return unit(Math.floor(d / 365), 'year');
	};

	HB.number = (n, opts) => fmt('num', opts || {}).format(n);

	/* 「一樓」、「院子」 and 「車庫」 — a list of already translated items. */
	HB.list = (items) => fmt('list', { style: 'long', type: 'conjunction' }).format(items);

	// ---------- job logs ----------

	/*
	 * A job log is text, one entry per line: "HH:MM:SS" (UTC), a tab, then a Msg
	 * as JSON or "|" and raw text. The day is not in the line: it is taken from
	 * the job's start (a later time of day than the start means the same day,
	 * an earlier one the next day). Lines in any other shape are shown as they are.
	 * Returns [{ time: ms|null, text, raw: bool }]; only complete lines are parsed.
	 */
	HB.logLines = function (text, startedAt) {
		const out = [];
		if (!text) {
			return out;
		}
		const base = HB.parseTime(startedAt);
		const lines = text.split('\n');
		// the last piece is either empty (the text ended with a newline) or a line still being written
		lines.pop();
		for (const line of lines) {
			const m = /^(\d{2}):(\d{2}):(\d{2})\t(.*)$/.exec(line);
			if (!m) {
				out.push({ time: null, text: line, raw: true });
				continue;
			}
			let time = null;
			if (base !== null) {
				const day = new Date(base);
				time = Date.UTC(day.getUTCFullYear(), day.getUTCMonth(), day.getUTCDate(), Number(m[1]), Number(m[2]), Number(m[3]));
				if (time < base - 3600000) {
					time += 86400000;
				}
			}
			const rest = m[4];
			if (rest.charAt(0) === '|') {
				out.push({ time, text: rest.slice(1), raw: true });
				continue;
			}
			let msg = null;
			try {
				msg = JSON.parse(rest);
			} catch (e) {
				msg = null;
			}
			if (msg && typeof msg === 'object' && typeof msg.key === 'string') {
				out.push({ time, text: HB.msg(msg), raw: false });
			} else {
				out.push({ time, text: rest, raw: true });
			}
		}
		return out;
	};

	// ---------- choosing and loading a language ----------

	function cookie(name) {
		for (const part of (document.cookie || '').split(/;\s*/)) {
			const i = part.indexOf('=');
			if (i > 0 && part.slice(0, i) === name) {
				try {
					return decodeURIComponent(part.slice(i + 1));
				} catch (e) {
					return part.slice(i + 1);
				}
			}
		}
		return '';
	}

	function remembered() {
		try {
			const v = window.localStorage.getItem(STORE);
			return SUPPORTED.includes(v) ? v : null;
		} catch (e) {
			return null;
		}
	}

	/* The remembered choice; else QTS's language (the nas_lang cookie its desktop
	 * writes); else the browser's languages; else Traditional Chinese. */
	function detect() {
		const mine = remembered();
		if (mine) {
			return { lang: mine, from: 'choice' };
		}
		const qts = cookie('nas_lang').toUpperCase();
		if (qts === 'TCH' || qts === 'SCH') {
			return { lang: 'zh-TW', from: 'qts' };
		}
		if (qts === 'JPN') {
			return { lang: 'ja', from: 'qts' };
		}
		const prefs = (navigator.languages && navigator.languages.length) ? navigator.languages : [navigator.language || ''];
		for (const l of prefs) {
			const x = String(l).toLowerCase();
			if (x.startsWith('ja')) {
				return { lang: 'ja', from: 'browser' };
			}
			if (x.startsWith('zh')) {
				return { lang: 'zh-TW', from: 'browser' };
			}
		}
		return { lang: 'zh-TW', from: 'default' };
	}

	async function catalog(url) {
		try {
			const res = await fetch(url, { credentials: 'same-origin', cache: 'no-cache' });
			if (!res.ok) {
				return {};
			}
			const data = await res.json();
			return data && typeof data === 'object' && !Array.isArray(data) ? data : {};
		} catch (e) {
			return {};
		}
	}

	/* The page's own static text (index.html): data-t, data-t-aria-label, data-t-title. */
	function applyStatic() {
		document.documentElement.lang = HTML_LANG[lang];
		document.title = t('Homebridge');
		for (const el of document.querySelectorAll('[data-t]')) {
			el.textContent = t(el.dataset.t);
		}
		for (const el of document.querySelectorAll('[data-t-aria-label]')) {
			el.setAttribute('aria-label', t(el.dataset.tAriaLabel));
		}
		for (const el of document.querySelectorAll('[data-t-title]')) {
			el.setAttribute('title', t(el.dataset.tTitle));
		}
	}

	let source = 'default';

	async function use(l, opts) {
		opts = opts || {};
		if (!SUPPORTED.includes(l)) {
			l = 'zh-TW';
		}
		const [backend, ui] = await Promise.all([catalog('i18n/' + l + '.backend.json'), catalog('i18n/' + l + '.json')]);
		// a key both catalogs define is shown with the UI's wording: there it is a short label for its place
		dict = Object.assign(Object.create(null), backend, ui);
		lang = l;
		fmtCache.clear();
		if (opts.remember) {
			source = 'choice';
			try {
				window.localStorage.setItem(STORE, l);
			} catch (e) {
				/* applied for this visit only */
			}
		}
		applyStatic();
		if (opts.announce) {
			document.dispatchEvent(new CustomEvent('hb:lang', { detail: { lang: l } }));
		}
	}

	HB.i18n = {
		supported: SUPPORTED,
		names: NAMES,
		lang: () => lang,
		source: () => source,
		init() {
			const d = detect();
			source = d.from;
			return use(d.lang);
		},
		/* Switch now and remember the choice; listeners of hb:lang repaint. */
		choose(l) {
			return use(l, { remember: true, announce: true });
		},
		applyStatic,
	};
})();
