/* Homebridge UI - the street: one lot per Homebridge, then the containers
 * that are not managed yet (grey houses with "Take over"), then an empty plot
 * for "Add Homebridge". Lots are keyed by name and patched in place on every refresh, so
 * polling never steals focus or closes a menu. */
(function () {
	'use strict';

	const { h, icon, t } = HB;

	const lots = new Map();      // name -> lot controller
	const unmanaged = new Map(); // name -> lot controller
	let root = null;
	let streetEl = null;
	let introEl = null;
	let plotLot = null;
	let firstPaint = true;

	// ---------- words ----------

	/* { kind: on|off|trouble|busy, text, extra } in the viewer's language. */
	function statusOf(inst, job) {
		const st = inst.state || {};
		if (inst.job) {
			const kind = (job && job.kind) || 'apply';
			const p = job && job.status === 'running' ? HB.jobs.overallPercent(job) : null;
			return { kind: 'busy', text: t(HB.JOB_STATUS[kind] || HB.JOB_STATUS.apply, { percent: p !== null ? HB.number(p / 100, { style: 'percent' }) : '' }).trim() };
		}
		let s;
		switch (st.status) {
			case 'running': {
				const since = HB.parseTime(st.started_at);
				s = { kind: 'on', text: since !== null && Date.now() - since < 60000 ? t('Just started') : t('Running for {duration}', { duration: HB.since(st.started_at) }) };
				break;
			}
			case 'stopped':
				s = { kind: 'off', text: t('Stopped') };
				break;
			case 'paused':
				s = { kind: 'off', text: t('Paused') };
				break;
			case 'restarting':
				s = { kind: 'trouble', text: st.restart_count ? t('Keeps restarting (restarts: {count})', { count: st.restart_count }) : t('Restarting') };
				break;
			case 'dead':
				s = { kind: 'trouble', text: t('Stopped unexpectedly') };
				break;
			case 'missing':
				s = { kind: 'trouble', text: st.foreign ? t('A container with this name is not managed here') : t('Container not found') };
				break;
			default:
				s = { kind: 'off', text: st.status || t('Unknown') };
		}
		if (inst.pending_changes && st.status !== 'missing') {
			s.extra = st.in_container_station ? t('Still in Container Station') : t('Settings not applied');
		}
		return s;
	}
	HB.statusOf = statusOf;

	/* The status line's words: the status, and the note after it when there is one. */
	function paintStatusWords(el, st) {
		HB.clear(el);
		const text = h('span', { class: 'status-text', text: st.text });
		if (st.extra) {
			HB.append(el, HB.tn('{status}, {note}', { status: text, note: h('span', { class: 'status-extra', text: st.extra }) }));
		} else {
			el.appendChild(text);
		}
	}
	HB.paintStatusWords = paintStatusWords;

	function houseLabel(inst, st) {
		const status = st.extra ? t('{status}, {note}', { status: st.text, note: st.extra }) : st.text;
		const up = inst.update || {};
		const update = up.legacy_image ? t('The image is no longer maintained') : (up.available ? t('An update is available') : null);
		return update
			? t('{name}, {status}, {update}. Open the details', { name: HB.displayName(inst), status, update })
			: t('{name}, {status}. Open the details', { name: HB.displayName(inst), status });
	}

	// ---------- lots ----------

	function managedLot(name) {
		const house = HB.house.create({ size: 'md' });
		const nameBtn = h('button', { type: 'button', class: 'lot-name' });
		const domain = h('p', { class: 'lot-domain', hidden: true });
		// the door plate is too small to read in the list layout: the address is repeated as text there
		const addr = h('p', { class: 'lot-addr', hidden: true });
		const statusDot = h('span', { class: 'status-dot', 'aria-hidden': 'true' });
		const statusWords = h('span', { class: 'status-words' });
		const status = h('p', { class: 'lot-status' }, [statusDot, statusWords]);
		const primarySlot = h('span', { class: 'lot-primary' });
		const more = h('button', { type: 'button', class: 'icon-btn icon-btn-sm lot-more', 'aria-label': t('More actions') }, icon('more'));
		const actions = h('div', { class: 'lot-actions' }, [primarySlot, more]);
		const yard = h('div', { class: 'yard' }, house.el);
		const el = h('article', { class: 'lot', role: 'listitem', 'data-name': name }, [yard, h('div', { class: 'lot-info' }, [nameBtn, domain, addr, status, actions])]);
		const ctl = { el, house, inst: null, job: null, unwatch: null, primaryKey: null };

		yard.addEventListener('click', () => HB.drawer.open(ctl.inst.spec.name));
		nameBtn.addEventListener('click', () => HB.drawer.open(ctl.inst.spec.name));
		HB.menuButton(more, () => HB.actions.menuItems(ctl.inst), { label: t('More actions') });

		ctl.update = function (inst) {
			ctl.inst = inst;
			// follow the job this instance is running
			if (inst.job && (!ctl.job || ctl.job.id !== inst.job)) {
				if (ctl.unwatch) {
					ctl.unwatch();
				}
				ctl.job = { id: inst.job, data: null };
				ctl.unwatch = HB.jobs.watch(inst.job, (e) => {
					if (ctl.job && ctl.job.id === e.id) {
						ctl.job.data = e.job;
						paint();
					}
				});
			} else if (!inst.job && ctl.job) {
				if (ctl.unwatch) {
					ctl.unwatch();
				}
				ctl.unwatch = null;
				ctl.job = null;
			}
			paint();
		};

		function paint() {
			const inst = ctl.inst;
			const job = ctl.job ? ctl.job.data : null;
			const model = HB.house.modelFor(inst, job, () => HB.actions.upgrade(inst.spec.name));
			if (job && job.status === 'running') {
				model.lights = HB.jobs.lightsDuring(job, inst);
			}
			house.update(model);
			el.dataset.color = inst.spec.color;
			nameBtn.textContent = HB.displayName(inst);
			nameBtn.title = inst.spec.name;
			const st = statusOf(inst, job);
			el.dataset.state = st.kind;
			nameBtn.setAttribute('aria-label', houseLabel(inst, st));
			const dom = inst.spec.domain || '';
			domain.hidden = !dom;
			domain.textContent = HB.middleTruncate(dom, 26);
			domain.title = dom;
			const ip = (inst.state && inst.state.ip) || (inst.spec.network.mode === 'static' ? inst.spec.network.ip : '') || '';
			addr.hidden = !ip;
			addr.textContent = ip;
			const words = st.text + '|' + (st.extra || '');
			if (statusWords.dataset.words !== words) {
				statusWords.dataset.words = words;
				paintStatusWords(statusWords, st);
			}
			status.dataset.kind = st.kind;
			paintPrimary(inst, st);
		}

		function paintPrimary(inst, st) {
			const s = inst.state.status;
			const url = inst.homebridge && inst.homebridge.ui_url;
			let key;
			if (inst.job) {
				key = 'job';
			} else if (s === 'running') {
				key = url ? 'open:' + url : 'open-none';
			} else if (s === 'missing') {
				key = inst.state.foreign ? 'takeover' : 'create';
			} else if (s === 'restarting' || s === 'dead') {
				key = 'logs';
			} else {
				key = 'start';
			}
			if (key === ctl.primaryKey) {
				return;
			}
			const hadFocus = primarySlot.contains(document.activeElement);
			ctl.primaryKey = key;
			HB.clear(primarySlot);
			let b;
			const name = inst.spec.name;
			if (key === 'job') {
				b = HB.button(t('Show progress'), { size: 'sm', onClick: () => HB.jobs.openDialog(ctl.inst.job, { instance: name, title: HB.jobTitle((ctl.job && ctl.job.data && ctl.job.data.kind) || 'apply', HB.displayName(ctl.inst)) }) });
			} else if (key.startsWith('open:')) {
				b = HB.button(t('Open'), { kind: 'primary', size: 'sm', icon: 'open', href: url, title: url });
				b.setAttribute('aria-label', t('Open the Homebridge web page of “{name}”', { name: HB.displayName(inst) }));
			} else if (key === 'open-none') {
				b = HB.button(t('Open'), { kind: 'primary', size: 'sm', icon: 'open', disabled: true, title: t('Its address is not known yet') });
			} else if (key === 'create' || key === 'takeover') {
				b = HB.button(key === 'takeover' ? t('Take over container') : t('Create container'), { kind: 'primary', size: 'sm', onClick: () => HB.actions.apply(name) });
			} else if (key === 'logs') {
				b = HB.button(t('View logs'), { size: 'sm', icon: 'logs', onClick: () => HB.drawer.open(name, 'logs') });
			} else {
				b = HB.button(t('Start'), { kind: 'primary', size: 'sm', icon: 'play', onClick: (e) => HB.actions.power(name, 'start', e.currentTarget) });
			}
			primarySlot.appendChild(b);
			if (hadFocus) {
				b.focus();
			}
		}

		ctl.destroy = function () {
			if (ctl.unwatch) {
				ctl.unwatch();
			}
			el.remove();
		};
		return ctl;
	}

	function unmanagedLot(c) {
		const house = HB.house.create({ size: 'md' });
		const take = HB.button(t('Take over'), { size: 'sm', icon: 'home', onClick: () => HB.wizard.openImport(ctl.c) });
		const nameEl = h('p', { class: 'lot-name is-static' });
		const status = h('p', { class: 'lot-status', 'data-kind': 'foreign' }, [h('span', { class: 'status-dot', 'aria-hidden': 'true' }), h('span', { class: 'status-words' }, h('span', { class: 'status-text' }))]);
		const el = h('article', { class: 'lot is-unmanaged', role: 'listitem', 'data-name': c.name }, [
			h('div', { class: 'yard' }, house.el),
			h('div', { class: 'lot-info' }, [nameEl, status, h('div', { class: 'lot-actions' }, take)]),
		]);
		const ctl = { el, c };
		ctl.update = function (c2) {
			ctl.c = c2;
			house.update({ kind: 'unmanaged', color: 'slate', emoji: '🏠', lights: c2.status === 'running' ? 'on' : 'off', plate: { text: c2.ip || '—', muted: true } });
			nameEl.textContent = c2.name;
			status.querySelector('.status-text').textContent = c2.status === 'running' ? t('Not managed yet, running') : t('Not managed yet, stopped');
			take.setAttribute('aria-label', t('Take over the container {name}', { name: c2.name }));
		};
		ctl.update(c);
		ctl.destroy = () => el.remove();
		return ctl;
	}

	function plot() {
		const sign = HB.s('svg', { class: 'plot-svg', viewBox: '0 0 200 170', 'aria-hidden': 'true', focusable: 'false' }, [
			HB.s('path', { class: 'plot-outline', d: 'M28 169.5V84.2H20.6a2.6 2.6 0 0 1-1.6-4.7L93.6 21.6a10.4 10.4 0 0 1 12.8 0L181 79.5a2.6 2.6 0 0 1-1.6 4.7H172v85.3' }),
			HB.s('path', { class: 'plot-post', d: 'M100 169.5v-38' }),
			HB.s('rect', { class: 'plot-sign', x: 70, y: 98, width: 60, height: 36, rx: 7 }),
			HB.s('path', { class: 'plot-plus', d: 'M100 106v20M90 116h20' }),
		]);
		const btn = h('button', { type: 'button', class: 'plot' }, [h('span', { class: 'yard' }, sign), h('span', { class: 'plot-label' }, [h('span', { class: 'plot-title', text: t('Add Homebridge') }), h('span', { class: 'plot-text', text: t('Start a new one from scratch') })])]);
		btn.addEventListener('click', () => HB.wizard.openCreate());
		return { el: h('div', { class: 'lot lot-plot', role: 'listitem' }, btn) };
	}

	// ---------- page ----------

	function skeleton() {
		const ghosts = [];
		for (let i = 0; i < 4; i++) {
			ghosts.push(h('div', { class: 'lot is-ghost', 'aria-hidden': 'true' }, [
				h('div', { class: 'yard' }, h('div', { class: 'ghost-house' })),
				h('div', { class: 'lot-info' }, [h('span', { class: 'ghost-line w60' }), h('span', { class: 'ghost-line w40' }), h('span', { class: 'ghost-line w30 tall' })]),
			]));
		}
		return h('div', { class: 'street-wrap' }, [
			h('div', { class: 'street-head' }, h('span', { class: 'ghost-line w30' })),
			h('div', { class: 'street', 'aria-busy': 'true', 'aria-label': t('Loading…') }, ghosts),
		]);
	}

	function mount(container) {
		root = container;
		HB.clear(root);
		root.appendChild(skeleton());
		lots.clear();
		unmanaged.clear();
		streetEl = null;
		firstPaint = true;
	}

	function intro() {
		const n = HB.instances.length;
		const found = HB.discovered.length;
		if (n === 0) {
			return h('section', { class: 'street-head is-intro' }, [
				h('h1', { class: 'intro-title', text: t('Manage your Homebridge on the NAS') }),
				h('p', { class: 'intro-text', text: t('Every Homebridge gets its own address on the local network and its own folders, independent of the others. Upgrades take one step, and if one goes wrong the previous version comes back automatically.') }),
				found
					? h('p', { class: 'intro-text' }, HB.tn('Homebridge containers already on this NAS: {count}. Take them over to manage them here; the names, addresses and HomeKit pairing stay the same. Or add a brand-new one.', { count: h('strong', { text: String(found) }) }))
					: h('p', { class: 'intro-text', text: t('Press “Add Homebridge” to create the first one.') }),
			]);
		}
		const running = HB.instances.filter((i) => i.state.status === 'running').length;
		const trouble = HB.instances.filter((i) => ['restarting', 'dead', 'missing'].includes(i.state.status)).length;
		const text = trouble
			? t('{total} in all, {running} running, {trouble} needing attention', { total: n, running, trouble })
			: t('{total} in all, {running} running', { total: n, running });
		const add = HB.button(t('Add'), { icon: 'plus', size: 'sm', onClick: () => HB.wizard.openCreate() });
		return h('section', { class: 'street-head' }, [h('h1', { class: 'street-title', text: t('Your Homebridge') }), h('p', { class: 'street-summary', text: text }), h('span', { class: 'grow' }), add]);
	}

	function render() {
		if (!root) {
			return;
		}
		if (!streetEl) {
			HB.clear(root);
			introEl = intro();
			streetEl = h('div', { class: 'street', role: 'list', 'aria-label': t('Homebridge list') });
			plotLot = plot();
			root.appendChild(h('div', { class: 'street-wrap' }, [introEl, streetEl]));
		} else {
			const fresh = intro();
			if (!fresh.isEqualNode(introEl)) {
				introEl.replaceWith(fresh);
				introEl = fresh;
			}
		}

		const want = new Set(HB.instances.map((i) => i.spec.name));
		for (const [name, ctl] of lots) {
			if (!want.has(name)) {
				ctl.destroy();
				lots.delete(name);
			}
		}
		const managedNames = want;
		const wantU = new Set(HB.discovered.filter((c) => !managedNames.has(c.name)).map((c) => c.name));
		for (const [name, ctl] of unmanaged) {
			if (!wantU.has(name)) {
				ctl.destroy();
				unmanaged.delete(name);
			}
		}

		const order = [];
		HB.instances.forEach((inst, i) => {
			let ctl = lots.get(inst.spec.name);
			if (!ctl) {
				ctl = managedLot(inst.spec.name);
				lots.set(inst.spec.name, ctl);
				if (firstPaint && inst.state.status === 'running' && !HB.reducedMotion()) {
					// the windows of running houses switch on one after another, once
					const delay = 250 + i * 170;
					const el = ctl.house.el;
					el.classList.add('power-on');
					el.style.setProperty('--delay', delay + 'ms');
					setTimeout(() => el.classList.remove('power-on'), delay + 1000);
				}
			}
			ctl.update(inst);
			order.push(ctl.el);
		});
		for (const c of HB.discovered) {
			if (managedNames.has(c.name)) {
				continue;
			}
			let ctl = unmanaged.get(c.name);
			if (!ctl) {
				ctl = unmanagedLot(c);
				unmanaged.set(c.name, ctl);
			} else {
				ctl.update(c);
			}
			order.push(ctl.el);
		}
		order.push(plotLot.el);
		// re-append only when the order changed, so focus stays where it is
		const current = Array.from(streetEl.children);
		const same = current.length === order.length && current.every((el, i) => el === order[i]);
		if (!same) {
			for (const el of order) {
				streetEl.appendChild(el);
			}
		}
		streetEl.classList.toggle('is-first-run', HB.instances.length === 0);
		firstPaint = false;
	}

	/* After a language switch: build every lot again from the data, without the skeleton or the lights-on moment. */
	function relang() {
		if (!root || !streetEl) {
			return;
		}
		for (const ctl of lots.values()) {
			ctl.destroy();
		}
		for (const ctl of unmanaged.values()) {
			ctl.destroy();
		}
		lots.clear();
		unmanaged.clear();
		streetEl = null;
		firstPaint = false;
		render();
	}

	HB.street = { mount, render, relang, statusOf };
})();
