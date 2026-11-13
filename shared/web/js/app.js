/* Homebridge UI - start-up, polling, the header (update check, preferences,
 * theme), whole-page states (signed out, not an administrator, Container
 * Station down), the language switch and the hash route for the drawer. */
(function () {
	'use strict';

	const { h, icon, t } = HB;
	const main = document.getElementById('main');
	const tools = document.getElementById('topbarTools');

	const REFRESH_MS = 8000;
	const DISCOVER_MS = 45000;
	const STALE_MS = 10 * 60 * 1000;

	let timers = [];
	let stopped = false;
	let refreshing = null;
	let lastDiscover = 0;
	let lastFatal = null;

	// ---------- data ----------

	async function refresh(withDiscover) {
		if (stopped) {
			return;
		}
		if (refreshing) {
			return refreshing;
		}
		refreshing = (async () => {
			try {
				const wantDiscover = withDiscover || Date.now() - lastDiscover > DISCOVER_MS;
				const [list, disc] = await Promise.all([
					HB.api.get('/instances'),
					wantDiscover ? HB.api.get('/discover').catch((e) => (e.handled ? Promise.reject(e) : null)) : Promise.resolve(undefined),
				]);
				HB.instances = list.instances || [];
				if (disc) {
					HB.discovered = disc.containers || [];
					lastDiscover = Date.now();
				}
				HB.street.render();
				document.dispatchEvent(new CustomEvent('hb:data'));
			} catch (err) {
				if (!err.handled && err.code === 'network') {
					showOffline(true);
				} else if (!err.handled) {
					HB.fail(err);
				}
				return;
			} finally {
				refreshing = null;
			}
			showOffline(false);
		})();
		return refreshing;
	}
	HB.refresh = refresh;

	let offlineToast = null;
	function showOffline(on) {
		if (on && !offlineToast) {
			offlineToast = HB.toast(t('The NAS cannot be reached right now. Trying again automatically.'), 'warn', { timeout: 600000 });
		} else if (!on && offlineToast) {
			offlineToast();
			offlineToast = null;
		}
	}

	function every(ms, fn) {
		const id = setInterval(() => {
			if (!document.hidden && !stopped) {
				fn();
			}
		}, ms);
		timers.push(id);
	}

	document.addEventListener('visibilitychange', () => {
		if (!document.hidden && !stopped && HB.session) {
			refresh();
		}
	});

	document.addEventListener('hb:instances-changed', () => HB.street.render());

	/* A job nobody is watching finished: say how it went. */
	const DONE = {
		create: HB.N_('Created “{name}”'),
		apply: HB.N_('Applied the new settings to “{name}”'),
		upgrade: HB.N_('Upgraded “{name}”'),
		rollback: HB.N_('“{name}” is back on its backup'),
		remove: HB.N_('Removed the container of “{name}”'),
	};
	const UNDONE = {
		create: HB.N_('Creating “{name}” did not work; the container it replaced is running again'),
		apply: HB.N_('Recreating “{name}” did not work; the original container is running again'),
		upgrade: HB.N_('Upgrading “{name}” did not work; the original container is running again'),
	};
	const FAILED = {
		create: HB.N_('Creating “{name}” did not finish'),
		apply: HB.N_('Recreating “{name}” did not finish'),
		upgrade: HB.N_('Upgrading “{name}” did not finish'),
		rollback: HB.N_('Restoring “{name}” did not finish'),
		remove: HB.N_('Removing “{name}” did not finish'),
	};

	document.addEventListener('hb:job-finished', async (e) => {
		const job = e.detail;
		if (job.kind === 'check_updates') {
			return;
		}
		const before = job.instance ? HB.findInstance(job.instance) : null;
		const name = before ? HB.displayName(before) : (job.instance || '');
		await refresh(true);
		if (HB.jobs.isShown(job.id)) {
			return;
		}
		const view = { label: t('Show'), onClick: () => HB.jobs.openDialog(job.id, { instance: job.instance, title: HB.jobTitle(job.kind, name) }) };
		if (job.status === 'succeeded') {
			HB.toast(t(DONE[job.kind] || HB.N_('“{name}”: done'), { name }), 'ok', { action: view });
		} else if (job.status === 'rolled_back') {
			HB.toast(t(UNDONE[job.kind] || HB.N_('“{name}”: it did not work, and the original container is running again'), { name }), 'warn', { action: view, timeout: 12000 });
		} else {
			HB.toast(t(FAILED[job.kind] || HB.N_('“{name}”: the job did not finish'), { name }), 'error', { action: view, timeout: 12000 });
		}
	});

	// ---------- update check ----------

	/* error keeps what the API said (a Msg or text), so a language switch repaints it. */
	const upd = { checkedAt: null, checking: false, error: null, manual: false, unwatch: null };
	const updState = document.getElementById('updateState');
	const checkBtn = document.getElementById('checkUpdates');

	function paintUpdates() {
		HB.clear(updState);
		updState.className = 'update-state';
		updState.removeAttribute('title');
		if (upd.checking) {
			updState.classList.add('is-checking');
			HB.append(updState, [h('span', { class: 'spinner', 'aria-hidden': 'true' }), h('span', { text: t('Checking for updates…') })]);
		} else if (upd.error) {
			const reason = HB.msg(upd.error);
			updState.classList.add('is-error');
			updState.title = upd.checkedAt
				? t('This check did not work: {reason}', { reason }) + '\n' + t('Last successful check: {date}', { date: HB.dateTime(upd.checkedAt) })
				: t('This check did not work: {reason}', { reason });
			HB.append(updState, [
				upd.checkedAt ? h('span', { class: 'update-text update-last', text: t('Checked {ago}', { ago: HB.ago(upd.checkedAt) }) }) : null,
				h('span', { class: 'update-fail' }, [
					icon('alert', { size: 16 }),
					h('span', { class: 'update-text update-fail-short', text: t('This check did not work') }),
					h('span', { class: 'update-text update-fail-long', text: t('This check did not work: {reason}', { reason }) }),
				]),
			]);
		} else if (upd.checkedAt) {
			updState.title = t('Last check: {date}', { date: HB.dateTime(upd.checkedAt) });
			HB.append(updState, h('span', { class: 'update-text', text: t('Checked for updates {ago}', { ago: HB.ago(upd.checkedAt) }) }));
		} else {
			HB.append(updState, h('span', { class: 'update-text', text: t('Not checked for updates yet') }));
		}
		checkBtn.disabled = upd.checking;
		checkBtn.classList.toggle('is-checking', upd.checking);
	}

	function withUpdates() {
		return new Set(HB.instances.filter((i) => i.update && i.update.available).map((i) => i.spec.name));
	}

	async function checkUpdates(manual) {
		if (upd.checking) {
			return;
		}
		const before = withUpdates();
		upd.checking = true;
		upd.error = null;
		upd.manual = manual;
		paintUpdates();
		let jobId;
		try {
			jobId = (await HB.api.post('/updates/check')).job_id;
		} catch (err) {
			upd.checking = false;
			upd.error = err.handled ? null : (err.msg || HB.errorText(err));
			paintUpdates();
			return;
		}
		upd.unwatch = HB.jobs.watch(jobId, async (e) => {
			if (!e.finished) {
				return;
			}
			if (upd.unwatch) {
				upd.unwatch();
				upd.unwatch = null;
			}
			const job = e.job;
			if (job.status !== 'succeeded') {
				upd.error = (job.error && (job.error.msg || job.error.message)) || { key: HB.N_('It did not finish') };
			}
			try {
				const u = await HB.api.get('/updates');
				upd.checkedAt = u.checked_at;
				const failed = (u.images || []).filter((i) => i.error);
				if (!upd.error && failed.length && failed.length === (u.images || []).length) {
					upd.error = failed[0].error_msg || failed[0].error;
				}
			} catch (err) {
				/* keep the last known time */
			}
			upd.checking = false;
			await refresh();
			paintUpdates();
			const after = withUpdates();
			const fresh = HB.instances.filter((i) => after.has(i.spec.name) && !before.has(i.spec.name));
			if (fresh.length === 1) {
				HB.toast(t('An update for “{name}” is here. Click the parcel at its door to upgrade.', { name: HB.displayName(fresh[0]) }), 'info', { action: { label: t('Upgrade'), onClick: () => HB.actions.upgrade(fresh[0].spec.name) } });
			} else if (fresh.length > 1 && fresh.length <= 3) {
				const names = HB.list(fresh.map((i) => t('“{name}”', { name: HB.displayName(i) })));
				HB.toast(t('Updates for {names} are here. Click the parcel at a door to upgrade.', { names }), 'info');
			} else if (fresh.length > 3) {
				HB.toast(t('Updates for {count} Homebridge are here. Click the parcel at a door to upgrade.', { count: fresh.length }), 'info');
			} else if (manual && !upd.error) {
				HB.toast(after.size ? t('No other updates') : t('Everything is up to date'), 'ok');
			}
		});
	}

	async function initUpdates(activeJobs) {
		try {
			const u = await HB.api.get('/updates');
			upd.checkedAt = u.checked_at;
		} catch (err) {
			if (err.handled) {
				return;
			}
		}
		paintUpdates();
		const running = (activeJobs || []).find((j) => j.kind === 'check_updates');
		const last = HB.parseTime(upd.checkedAt);
		if (running) {
			checkResume(running.id);
		} else if (last === null || Date.now() - last > STALE_MS) {
			checkUpdates(false);
		}
	}

	/* Pick up a check that was already running when the page loaded. */
	function checkResume(id) {
		upd.checking = true;
		paintUpdates();
		const unwatch = HB.jobs.watch(id, async (e) => {
			if (!e.finished) {
				return;
			}
			unwatch();
			upd.checking = false;
			try {
				upd.checkedAt = (await HB.api.get('/updates')).checked_at;
			} catch (err) { /* ignore */ }
			await refresh();
			paintUpdates();
		});
	}

	// ---------- preferences ----------

	let prefs = null; // the open preferences dialog: { ctl, state, render }

	async function openSettings() {
		let settings;
		try {
			settings = await HB.api.get('/settings');
		} catch (err) {
			HB.fail(err);
			return;
		}
		const state = { image: settings.default_image || '', mounts: HB.deepCopy(settings.default_mounts || []) };
		const ctl = HB.dialog({
			title: t('Preferences'),
			size: 'md',
			body: [],
			onClose: () => {
				prefs = null;
			},
			actions: [],
		});
		prefs = { ctl, state, render: () => renderPrefs(ctl, state) };
		prefs.render();
		return ctl;
	}

	/* Builds (and, after a language switch, rebuilds) the dialog from state, so unsaved input survives. */
	function renderPrefs(ctl, state) {
		ctl.setTitle(t('Preferences'));
		const langSel = h('select', { class: 'select', lang: null }, HB.i18n.supported.map((code) => h('option', { value: code, lang: code, text: HB.i18n.names[code], selected: code === HB.i18n.lang() })));
		langSel.addEventListener('change', async () => {
			langSel.disabled = true;
			await HB.i18n.choose(langSel.value);
		});
		const source = HB.i18n.source();
		const langHelp = source === 'choice' ? t('Remembered in this browser.')
			: source === 'qts' ? t('Follows the language of QTS until you choose one here.')
				: t('Follows this browser’s language until you choose one here.');
		const image = HB.input({ value: state.image, placeholder: 'homebridge/homebridge:latest' });
		image.addEventListener('input', () => {
			state.image = image.value;
		});
		const mounts = HB.specform.mountsEditor(state.mounts, { pathPrefix: 'default_mounts', addLabel: t('Add a default folder') });
		const s = HB.session || {};
		const form = h('div', { class: 'form-stack' }, [
			HB.field({ label: t('Language'), control: langSel, help: langHelp }),
			HB.field({ label: t('Default image for a new Homebridge'), path: 'default_image', control: image, help: t('Filled in when you add a Homebridge. Usually homebridge/homebridge:latest.') }),
			h('div', { class: 'field', 'data-path': 'default_mounts' }, [
				h('span', { class: 'field-label', text: t('Default folders for a new Homebridge') }),
				h('p', { class: 'field-help', text: t('Every new Homebridge starts with these folders, such as a shared certificates folder.') }),
				mounts.el,
				h('p', { class: 'field-error', hidden: true }),
			]),
			h('dl', { class: 'facts facts-compact' }, [
				h('dt', { text: t('Package version') }), h('dd', { text: s.version || '—' }),
				h('dt', { text: t('Docker') }), h('dd', { text: (s.docker && s.docker.version) || '—' }),
				h('dt', { text: t('Signed in as') }), h('dd', { text: s.user || '—' }),
			]),
		]);
		HB.clear(ctl.body);
		ctl.body.appendChild(form);
		ctl.setActions(['spacer', { label: t('Cancel') }, {
			label: t('Save'), kind: 'primary', onClick: async (c) => {
				HB.clearFieldErrors(form);
				c.setBusy(true);
				try {
					const saved = await HB.api.put('/settings', { default_image: state.image.trim(), default_mounts: mounts.get() });
					HB.settings = saved;
					HB.toast(t('Preferences saved'), 'ok');
					c.setBusy(false);
					return true;
				} catch (err) {
					c.setBusy(false);
					HB.formError(form, err);
					return false;
				}
			},
		}]);
	}

	// ---------- theme ----------

	const themeBtn = document.getElementById('themeToggle');
	const THEMES = { auto: ['auto', HB.N_('Follow the system')], light: ['sun', HB.N_('Light')], dark: ['moon', HB.N_('Dark')] };

	function currentTheme() {
		const v = document.documentElement.getAttribute('data-theme');
		return v === 'light' || v === 'dark' ? v : 'auto';
	}

	function paintTheme() {
		const mode = currentTheme();
		HB.clear(themeBtn);
		themeBtn.appendChild(icon(THEMES[mode][0]));
		themeBtn.setAttribute('aria-label', t('Appearance: {mode}. Click to change it', { mode: t(THEMES[mode][1]) }));
		themeBtn.title = t('Appearance: {mode}', { mode: t(THEMES[mode][1]) });
	}

	themeBtn.addEventListener('click', () => {
		const next = { auto: 'light', light: 'dark', dark: 'auto' }[currentTheme()];
		if (next === 'auto') {
			document.documentElement.removeAttribute('data-theme');
		} else {
			document.documentElement.setAttribute('data-theme', next);
		}
		try {
			if (next === 'auto') {
				window.localStorage.removeItem('hb-theme');
			} else {
				window.localStorage.setItem('hb-theme', next);
			}
		} catch (e) {
			/* not remembered, still applied */
		}
		paintTheme();
		HB.toast(t('Appearance: {mode}', { mode: t(THEMES[next][1]) }), 'info', { timeout: 1600 });
	});

	// ---------- whole-page states ----------

	function fatal(err) {
		lastFatal = err;
		stopped = true;
		for (const id of timers) {
			clearInterval(id);
		}
		timers = [];
		HB.jobs.stopAll();
		HB.closeAllDialogs();
		if (HB.drawer) {
			HB.drawer.close(true);
		}
		tools.hidden = true;
		const inFrame = window.top !== window.self;
		let title;
		let text;
		let extra = null;
		let houseKind = 'managed';
		let showCode = false;
		if (err.status === 401) {
			title = t('Your QTS session has ended');
			text = inFrame
				? t('For safety, Homebridge can be managed only while you are signed in to QTS. Sign in to QTS again, then press “Try again”.')
				: t('For safety, Homebridge can be managed only while you are signed in to QTS. Sign in to QTS first, then come back here and press “Try again”.');
			if (!inFrame) {
				extra = HB.button(t('Sign in to QTS'), { href: '/', external: false, icon: 'key' });
			}
		} else if (err.status === 403) {
			title = t('Administrators only');
			text = t('Homebridge can mount any folder on the NAS, so only accounts in the administrators group may use it. Sign in to QTS with an administrator account.');
		} else if (err.code === 'docker_unavailable') {
			title = t('Container Station is not running');
			text = t('Every Homebridge lives in Container Station. Open Container Station in the App Center, wait until it has started, then press “Try again”.');
			houseKind = 'missing';
		} else if (err.code === 'qts_unavailable') {
			title = t('Cannot confirm your sign-in right now');
			text = t('QTS did not answer the sign-in check. This usually clears up in a few seconds.');
		} else if (err.code === 'network') {
			title = t('Cannot reach the NAS');
			text = t('Make sure this device and the NAS are on the same network, then press “Try again”.');
		} else {
			title = t('This page cannot be shown right now');
			text = HB.errorText(err);
			showCode = true;
		}
		const house = HB.house.create({ size: 'lg' });
		house.update({ kind: houseKind, color: 'slate', emoji: houseKind === 'missing' ? '📦' : '🔒', lights: 'off', plate: null });
		const retry = HB.button(t('Try again'), { kind: 'primary', icon: 'refresh', onClick: () => start() });
		HB.clear(main);
		main.appendChild(h('section', { class: 'state-page', 'aria-labelledby': 'stateTitle' }, [
			h('div', { class: 'state-house' }, house.el),
			h('div', { class: 'state-copy' }, [
				h('h1', { id: 'stateTitle', text: title }),
				h('p', { text: text }),
				showCode ? h('p', { class: 'muted', text: t('Error code: {code}', { code: err.code || err.status }) }) : null,
				h('div', { class: 'state-actions' }, [retry, extra]),
			]),
		]));
		retry.focus();
	}
	HB.onFatal = fatal;

	// ---------- language ----------

	function paintChrome() {
		HB.clear(checkBtn);
		HB.append(checkBtn, [icon('refresh'), h('span', { class: 'label-wide', text: t('Check for updates') })]);
		checkBtn.setAttribute('aria-label', t('Check for updates'));
		const settingsBtn = document.getElementById('openSettings');
		settingsBtn.setAttribute('aria-label', t('Preferences'));
		settingsBtn.title = t('Preferences');
		paintTheme();
		paintUpdates();
	}

	document.addEventListener('hb:lang', () => {
		paintChrome();
		if (stopped && lastFatal) {
			fatal(lastFatal);
			return;
		}
		HB.street.relang();
		HB.drawer.relang();
		if (prefs) {
			prefs.render();
			const sel = prefs.ctl.body.querySelector('select');
			if (sel) {
				sel.focus();
			}
		}
		HB.toast(t('The language is now {language}', { language: HB.i18n.names[HB.i18n.lang()] }), 'ok', { timeout: 2500 });
	});

	// ---------- route: #/h/<name>/<tab> opens the drawer ----------

	function route() {
		const m = location.hash.match(/^#\/h\/([^/]+)(?:\/([a-z]+))?$/);
		if (m && HB.session && !stopped) {
			HB.drawer.openFromRoute(decodeURIComponent(m[1]), m[2] || 'overview');
		} else {
			HB.drawer.closeFromRoute();
		}
	}
	window.addEventListener('hashchange', route);

	// ---------- start ----------

	async function start() {
		stopped = false;
		lastFatal = null;
		for (const id of timers) {
			clearInterval(id);
		}
		timers = [];
		tools.hidden = true;
		HB.street.mount(main);
		let session;
		try {
			session = await HB.api.get('/session');
		} catch (err) {
			if (!err.handled) {
				fatal(err);
			}
			return;
		}
		HB.session = session;
		if (!session.docker || !session.docker.ok) {
			fatal(new HB.ApiError(503, 'docker_unavailable', { key: HB.N_('Container Station is not running') }));
			return;
		}
		let active = [];
		try {
			const [list, disc, settings, jobs, networks] = await Promise.all([
				HB.api.get('/instances'),
				HB.api.get('/discover').catch((e) => (e.handled ? Promise.reject(e) : { containers: [] })),
				HB.api.get('/settings').catch((e) => (e.handled ? Promise.reject(e) : null)),
				HB.api.get('/jobs', { active: 1 }).catch((e) => (e.handled ? Promise.reject(e) : { jobs: [] })),
				HB.api.get('/networks').catch((e) => (e.handled ? Promise.reject(e) : null)),
			]);
			HB.networks = networks;
			HB.instances = list.instances || [];
			HB.discovered = disc.containers || [];
			HB.settings = settings;
			active = jobs.jobs || [];
			lastDiscover = Date.now();
		} catch (err) {
			if (!err.handled) {
				fatal(err);
			}
			return;
		}
		// jobs that were running before a reload: put them back on their houses
		for (const j of active) {
			const inst = j.instance ? HB.findInstance(j.instance) : null;
			if (inst && !inst.job) {
				inst.job = j.id;
			}
		}
		tools.hidden = false;
		HB.street.render();
		paintUpdates();
		initUpdates(active);
		every(REFRESH_MS, () => refresh());
		every(30000, paintUpdates);
		route();
	}

	async function init() {
		await HB.i18n.init();
		checkBtn.addEventListener('click', () => checkUpdates(true));
		const settingsBtn = document.getElementById('openSettings');
		settingsBtn.appendChild(icon('sliders'));
		settingsBtn.addEventListener('click', openSettings);
		paintChrome();
		start();
	}

	init();
})();
