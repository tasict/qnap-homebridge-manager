/* Homebridge UI - the detail drawer for one Homebridge: Overview / Settings /
 * Logs / Backup. Settings are split into what takes effect at once (name, sign,
 * colour, domain) and what needs "Apply and recreate" (everything the container is
 * created from). Opened through the hash #/h/<name>/<tab>. */
(function () {
	'use strict';

	const { h, icon, t } = HB;
	const TABS = [['overview', HB.N_('Overview')], ['settings', HB.N_('Settings')], ['logs', HB.N_('Logs')], ['backup', HB.N_('Backup')]];
	const REBUILD_KEYS = ['image', 'hostname', 'network', 'workdir', 'mounts', 'env', 'restart'];
	const INSTANT_KEYS = ['display_name', 'emoji', 'color', 'domain'];

	let cur = null;
	const domainCache = new Map();

	function hashFor(name, tab) {
		return '#/h/' + encodeURIComponent(name) + '/' + (tab || 'overview');
	}

	function open(name, tab) {
		const target = hashFor(name, tab);
		if (location.hash === target) {
			openFromRoute(name, tab || 'overview');
		} else {
			location.hash = target;
		}
	}

	function openFromRoute(name, tab) {
		const inst = HB.findInstance(name);
		if (!inst) {
			if (cur) {
				close(true);
			}
			history.replaceState(null, '', '#/');
			HB.toast(t('“{name}” was not found. It may have been removed.', { name }), 'warn');
			return;
		}
		if (cur && cur.name === name) {
			cur.tabs.select(tab);
			return;
		}
		if (cur) {
			close(true, true);
		}
		build(inst, tab);
	}

	function closeFromRoute() {
		if (cur) {
			close(true, true);
		}
	}

	/* force: skip the unsaved-changes question; keepHash: called from the router */
	function close(force, keepHash) {
		if (!cur) {
			return;
		}
		if (!force && cur.dirty && cur.dirty()) {
			HB.confirm({ title: t('Discard the unsaved changes?'), body: t('Some settings have changes that are not saved. Closing loses them.'), cancel: t('Keep editing'), confirm: t('Discard changes'), danger: true }).then((ok) => {
				if (ok) {
					close(true, keepHash);
				}
			});
			return;
		}
		const c = cur;
		cur = null;
		c.cleanup();
		if (c.el.open) {
			c.el.close();
		}
		c.el.remove();
		if (!keepHash && /^#\/h\//.test(location.hash)) {
			history.replaceState(null, '', '#/');
		}
		if (c.returnFocus && document.contains(c.returnFocus)) {
			c.returnFocus.focus({ preventScroll: true });
		} else {
			const lot = document.querySelector('.lot[data-name="' + CSS.escape(c.name) + '"] .lot-name');
			if (lot) {
				lot.focus({ preventScroll: true });
			}
		}
	}

	// ---------- build ----------

	function build(inst, tab) {
		const name = inst.spec.name;
		const house = HB.house.create({ size: 'md' });
		const title = h('h2', { class: 'drawer-title', id: HB.uid('dtitle') });
		const sub = h('p', { class: 'drawer-sub', text: name });
		const status = h('p', { class: 'drawer-status' }, [h('span', { class: 'status-dot', 'aria-hidden': 'true' }), h('span', { class: 'status-words' })]);
		const closeBtn = h('button', { type: 'button', class: 'icon-btn drawer-close', 'aria-label': t('Close') }, icon('x'));
		const actions = h('div', { class: 'drawer-actions' });
		const applyBar = h('div', { class: 'apply-bar', hidden: true, role: 'region', 'aria-label': t('Settings not applied') });
		const el = h('dialog', { class: 'drawer', 'aria-labelledby': title.id });

		const c = cur = {
			name, el, house, title, status, actions, applyBar, inst,
			returnFocus: document.activeElement,
			timers: [],
			actionKey: null,
			pinShown: false,
			settings: null,
			cleanup() {
				for (const timer of c.timers) {
					clearInterval(timer);
				}
				c.timers = [];
				if (c.logTimer) {
					clearInterval(c.logTimer);
				}
				if (c.statsTimer) {
					clearInterval(c.statsTimer);
				}
				if (c.jobWatch) {
					c.jobWatch();
				}
				document.removeEventListener('hb:data', onData);
			},
			dirty: () => !!(c.settings && c.settings.dirty()),
		};

		c.tabs = HB.tabs(TABS.map(([key, label]) => ({
			key, label: t(label),
			badge: key === 'settings' ? h('span', { class: 'tab-dot', hidden: true, 'aria-label': t('Some settings are not applied yet') }) : null,
			render: (panel) => renderTab(key, panel),
		})), {
			label: t('Details'),
			onSelect: (key, prev, user) => {
				if (user && location.hash !== hashFor(name, key)) {
					history.replaceState(null, '', hashFor(name, key));
				}
				if (key === 'overview') {
					startStats();
				} else if (c.statsTimer) {
					clearInterval(c.statsTimer);
					c.statsTimer = null;
				}
				if (key !== 'logs' && c.logTimer) {
					clearInterval(c.logTimer);
					c.logTimer = null;
				} else if (key === 'logs' && c.logResume) {
					c.logResume();
				}
			},
		});

		HB.append(el, [
			h('header', { class: 'drawer-head' }, [h('div', { class: 'drawer-house' }, house.el), h('div', { class: 'drawer-titles' }, [title, sub, status]), closeBtn]),
			actions,
			c.tabs.el,
			applyBar,
		]);

		closeBtn.addEventListener('click', () => close(false));
		el.addEventListener('cancel', (e) => {
			e.preventDefault();
			if (!HB.topDialog()) {
				close(false);
			}
		});
		el.addEventListener('close', () => {
			if (cur === c) {
				// closed by the browser (repeated Esc): reopen if there are unsaved edits
				if (c.dirty()) {
					el.showModal();
				} else {
					close(true);
				}
			}
		});
		el.addEventListener('click', (e) => {
			// a click on the backdrop (outside the panel) closes the drawer
			if (e.target === el) {
				const r = el.getBoundingClientRect();
				if (e.clientX < r.left) {
					close(false);
				}
			}
		});

		document.body.appendChild(el);
		el.showModal();
		paintHead();
		c.tabs.select(tab || 'overview');
		closeBtn.focus();
		document.addEventListener('hb:data', onData);
	}

	function onData() {
		if (!cur) {
			return;
		}
		const fresh = HB.findInstance(cur.name);
		if (!fresh) {
			close(true);
			return;
		}
		const prev = cur.inst;
		cur.inst = fresh;
		paintHead();
		const tab = cur.tabs.current;
		const panel = cur.tabs.panels[tab];
		const changed = JSON.stringify(prev) !== JSON.stringify(fresh);
		if (!changed) {
			return;
		}
		const sel = window.getSelection();
		const selecting = sel && !sel.isCollapsed && panel.contains(sel.anchorNode);
		if (tab === 'overview' && !panel.contains(document.activeElement) && !selecting) {
			renderOverview(panel);
		} else if (tab === 'backup') {
			renderBackup(panel);
		} else if (tab === 'settings' && cur.settings && !cur.settings.dirty() && prev.updated_at !== fresh.updated_at) {
			renderSettings(panel);
		}
	}

	// ---------- head ----------

	function paintHead() {
		const c = cur;
		const inst = c.inst;
		const jobEntry = inst.job ? HB.jobs.entry(inst.job) : null;
		const job = jobEntry ? jobEntry.job : null;
		const model = HB.house.modelFor(inst, job, () => HB.actions.upgrade(inst.spec.name));
		if (job && job.status === 'running') {
			model.lights = HB.jobs.lightsDuring(job, inst);
		}
		c.house.update(model);
		c.title.textContent = HB.displayName(inst);
		const st = HB.statusOf(inst, job);
		c.status.dataset.kind = st.kind;
		HB.paintStatusWords(c.status.querySelector('.status-words'), st);
		c.tabs.buttons.settings.querySelector('.tab-dot').hidden = !inst.pending_changes;
		paintActions();
		paintApplyBar();
		if (inst.job && !c.jobWatch) {
			// watch() calls back at once when the job is already known (the progress dialog loaded it):
			// mark the watch as taken first, or that call would come back here and subscribe again, without end
			c.jobWatch = () => {};
			c.jobWatch = HB.jobs.watch(inst.job, () => {
				if (cur === c) {
					paintHead();
				}
			});
		} else if (!inst.job && c.jobWatch) {
			c.jobWatch();
			c.jobWatch = null;
		}
	}

	function paintActions() {
		const c = cur;
		const inst = c.inst;
		const s = inst.state.status;
		const url = inst.homebridge && inst.homebridge.ui_url;
		const up = inst.update || {};
		const key = [s, url, !!inst.job, up.available, up.legacy_image, !!inst.state.foreign].join('|');
		if (key === c.actionKey) {
			return;
		}
		c.actionKey = key;
		const hadFocus = c.actions.contains(document.activeElement);
		HB.clear(c.actions);
		const name = inst.spec.name;
		const list = [];
		if (inst.job) {
			list.push(HB.button(t('Show progress'), { kind: 'primary', size: 'sm', icon: 'refresh', onClick: () => HB.jobs.openDialog(inst.job, { instance: name }) }));
		} else {
			if (s === 'running') {
				list.push(url ? HB.button(t('Open Homebridge'), { kind: 'primary', size: 'sm', icon: 'open', href: url, title: url }) : HB.button(t('Open Homebridge'), { kind: 'primary', size: 'sm', icon: 'open', disabled: true, title: t('Its address is not known yet') }));
				list.push(HB.button(t('Restart'), { size: 'sm', icon: 'restart', onClick: (e) => HB.actions.power(name, 'restart', e.currentTarget) }));
				list.push(HB.button(t('Stop'), { size: 'sm', icon: 'stop', onClick: (e) => HB.actions.power(name, 'stop', e.currentTarget) }));
			} else if (s === 'missing') {
				list.push(HB.button(inst.state.foreign ? t('Take over container') : t('Create container'), { kind: 'primary', size: 'sm', icon: 'refresh', onClick: () => HB.actions.apply(name) }));
			} else if (s === 'restarting') {
				list.push(HB.button(t('Stop'), { size: 'sm', icon: 'stop', onClick: (e) => HB.actions.power(name, 'stop', e.currentTarget) }));
				list.push(HB.button(t('Restart'), { size: 'sm', icon: 'restart', onClick: (e) => HB.actions.power(name, 'restart', e.currentTarget) }));
			} else {
				list.push(HB.button(t('Start'), { kind: 'primary', size: 'sm', icon: 'play', onClick: (e) => HB.actions.power(name, 'start', e.currentTarget) }));
			}
			if (up.legacy_image) {
				list.push(HB.button(t('Switch image and upgrade'), { size: 'sm', icon: 'upgrade', onClick: () => HB.actions.upgrade(name) }));
			} else if (up.available) {
				list.push(HB.button(t('Upgrade'), { size: 'sm', icon: 'upgrade', onClick: () => HB.actions.upgrade(name) }));
			}
		}
		HB.append(c.actions, list);
		if (hadFocus && c.actions.firstChild) {
			c.actions.firstChild.focus();
		}
	}

	function paintApplyBar() {
		const c = cur;
		const inst = c.inst;
		const show = inst.pending_changes && !inst.job;
		c.applyBar.hidden = !show;
		if (!show) {
			return;
		}
		const missing = inst.state.status === 'missing';
		const moving = !missing && !!inst.state.in_container_station;
		const key = missing ? (inst.state.foreign ? 'foreign' : 'missing') : (moving ? 'move' : 'pending');
		if (c.applyBar.dataset.key === key) {
			return;
		}
		c.applyBar.dataset.key = key;
		HB.clear(c.applyBar);
		HB.append(c.applyBar, [
			icon(missing ? 'alert' : 'pencil', { size: 20 }),
			h('div', { class: 'apply-text' }, missing
				? (inst.state.foreign
					? [h('strong', { text: t('A container with this name is not managed here') }), h('span', { text: t('The saved settings are used only after you take it over; that container is then kept as the backup.') })]
					: [h('strong', { text: t('This Homebridge has no container right now') }), h('span', { text: t('Create a new one from the saved settings; the data in the work folder is used as it is.') })])
				: moving
					? [h('strong', { text: t('This Homebridge still runs in Container Station') }), h('span', { text: t('Applying moves it out of Container Station’s list, so it cannot be deleted there by mistake. Homebridge restarts for about a minute; data and pairing are not affected, and the old container is kept as the backup.') })]
					: [h('strong', { text: t('Settings saved but not applied yet') }), h('span', { text: t('Applying recreates the container and restarts Homebridge for about a minute. Data and pairing are not affected, and the old container is kept as the backup.') })]),
			HB.button(missing ? (inst.state.foreign ? t('Take over container') : t('Create container')) : t('Apply and recreate'), { kind: 'primary', size: 'sm', onClick: () => HB.actions.apply(inst.spec.name) }),
		]);
	}

	// ---------- tabs ----------

	function renderTab(key, panel) {
		if (key === 'overview') {
			renderOverview(panel);
		} else if (key === 'settings') {
			renderSettings(panel);
		} else if (key === 'logs') {
			renderLogs(panel);
		} else {
			renderBackup(panel);
		}
	}

	function section(title, body, extra) {
		return h('section', { class: 'ov-section' }, [h('h3', { class: 'section-title' }, [h('span', { text: title }), extra || null]), body]);
	}

	function facts(rows) {
		const dl = h('dl', { class: 'facts' });
		for (const r of rows) {
			if (!r) {
				continue;
			}
			dl.appendChild(h('dt', { text: r[0] }));
			dl.appendChild(h('dd', null, r[1]));
		}
		return dl;
	}

	function renderOverview(panel) {
		const c = cur;
		const inst = c.inst;
		const st = inst.state;
		const hb = inst.homebridge || {};
		const spec = inst.spec;
		const out = [];

		// problems first
		if (st.status === 'missing') {
			out.push(HB.callout('warn', st.foreign ? t('A container with this name is not managed here') : t('This Homebridge’s container was not found'),
				h('p', { text: st.foreign
					? t('The name {container} now belongs to a container that is not managed here. If you just went back to the backup from before the takeover, that is the original container, running as before. To manage it here again, press “Take over container”: it is kept as the backup and a new container is created from the saved settings. If you do not want it managed here, stop managing this Homebridge at the bottom of Settings.', { container: spec.name })
					: t('It may have been deleted outside this app. The work folder is still there; press “Create container” below to create it again from the saved settings.') })));
		} else if (st.status === 'restarting' || st.status === 'dead') {
			out.push(HB.callout('error', st.status === 'dead' ? t('The container stopped unexpectedly') : (st.restart_count ? t('It keeps restarting (restarts: {count})', { count: st.restart_count }) : t('It keeps restarting')),
				h('div', null, [
					h('p', { text: t('Usually a plugin is failing. Look at the last lines of the log, or turn off the plugin you added last in the Homebridge web page; you can also go back to the previous version under Backup.') }),
					h('div', { style: { marginTop: '8px' } }, HB.button(t('View logs'), { size: 'sm', icon: 'logs', onClick: () => c.tabs.select('logs', true) })),
				])));
		}

		// the web page
		const web = h('div', { class: 'web-link' });
		if (hb.ui_url) {
			web.appendChild(icon('globe'));
			web.appendChild(h('a', { class: 'web-url', href: hb.ui_url, target: '_blank', rel: 'noopener', text: hb.ui_url }));
			web.appendChild(h('span', { class: 'grow' }));
			if (spec.domain && hb.ui_url_ip && hb.ui_url_ip !== hb.ui_url) {
				web.appendChild(HB.button(t('Open by IP address'), { kind: 'ghost', size: 'sm', href: hb.ui_url_ip, title: hb.ui_url_ip }));
			}
			web.appendChild(HB.button(t('Open'), { kind: 'primary', size: 'sm', icon: 'open', href: hb.ui_url }));
		} else {
			web.appendChild(icon('globe'));
			const why = {
				running: t('Its address is not known yet; it appears once it has started.'),
				missing: st.foreign ? t('The address of the Homebridge web page appears here after you take the container over.') : t('The Homebridge web page can be opened once a container is created.'),
				restarting: t('It keeps restarting, so the web page cannot be opened right now.'),
				dead: t('The container has stopped, so the web page cannot be opened right now.'),
			}[st.status] || t('The Homebridge web page can be opened once it is started.');
			web.appendChild(h('span', { class: 'muted', text: why }));
		}
		const domainLine = HB.checkLine();
		const webBox = h('div', { class: 'form-stack', style: { gap: '8px' } }, [web]);
		if (spec.domain) {
			webBox.appendChild(h('div', { class: 'domain-row' }, HB.tn('Domain {domain}', { domain: h('span', { class: 'mono', text: spec.domain }) })));
			webBox.appendChild(domainLine);
			checkDomain(spec.domain, st.ip || (spec.network.mode === 'static' ? spec.network.ip : null), domainLine);
		}
		out.push(section(t('Homebridge web page'), webBox));

		// HomeKit
		const pinWrap = h('div', { class: 'pin-card' });
		paintPin(pinWrap, hb.pin);
		out.push(section('HomeKit', facts([
			[t('Bridge name'), hb.bridge_name ? h('span', { text: hb.bridge_name }) : h('span', { class: 'muted', text: t('Not readable yet (there is no config.json in the work folder yet)') })],
			[t('Setup code'), hb.pin ? pinWrap : h('span', { class: 'muted', text: t('Not readable yet') })],
			[t('Plugins'), hb.plugins && hb.plugins.length ? h('div', { class: 'chips' }, hb.plugins.map((p) => h('span', { class: 'chip', text: p.replace(/^homebridge-/, ''), title: p }))) : h('span', { class: 'muted', text: t('No plugins installed yet') })],
		])));

		// running state
		if (st.status === 'running') {
			c.meters = {
				cpu: h('span', { class: 'meter-value', text: '—' }),
				mem: h('span', { class: 'meter-value', text: '—' }),
			};
			if (c.lastStats) {
				paintStats(c.lastStats);
			}
			out.push(section(t('Running state'), h('div', { class: 'form-stack', style: { gap: '12px' } }, [
				h('div', { class: 'meters' }, [
					h('div', { class: 'meter' }, [c.meters.cpu, h('span', { class: 'meter-label', text: 'CPU' })]),
					h('div', { class: 'meter' }, [c.meters.mem, h('span', { class: 'meter-label', text: t('Memory') })]),
					h('div', { class: 'meter' }, [h('span', { class: 'meter-value', text: HB.since(st.started_at) }), h('span', { class: 'meter-label', text: t('Up for') })]),
				]),
				facts([
					[t('Started'), h('span', { text: HB.dateTime(st.started_at) })],
					st.restart_count ? [t('Restarts'), h('span', { text: HB.number(st.restart_count) })] : null,
				]),
			])));
			startStats();
		} else if (st.status !== 'missing') {
			c.meters = null;
			out.push(section(t('Running state'), facts([
				[t('State'), h('span', { text: HB.statusOf(inst, null).text })],
				st.finished_at ? [t('Stopped at'), h('span', { text: t('{date} ({ago})', { date: HB.dateTime(st.finished_at), ago: HB.ago(st.finished_at) }) })] : null,
				st.exit_code !== null && st.exit_code !== undefined ? [t('Exit code'), h('span', { text: st.exit_code === 0 ? t('{code} (a normal exit)', { code: st.exit_code }) : String(st.exit_code) })] : null,
			])));
		}

		// network
		const ip = st.ip || (spec.network.mode === 'static' ? spec.network.ip : null);
		const ifc = ((HB.networks && HB.networks.interfaces) || []).find((i) => i.iface === spec.network.iface);
		out.push(section(t('Network'), facts([
			[t('Address'), h('span', null, [h('span', { class: ['inline-plate', st.status === 'running' ? null : 'is-off'] }, [spec.network.mode === 'dhcp' ? h('small', { text: 'DHCP' }) : null, ip || '—']), ' ', h('span', { class: 'muted', text: spec.network.mode === 'dhcp' ? t('Automatic') : t('Fixed address') })])],
			['MAC', h('span', { class: 'mono', text: st.mac || spec.network.mac || '—' })],
			[t('Network interface'), h('span', { text: ifc ? ifc.display : spec.network.iface })],
			[t('Host name'), h('span', { class: 'mono', text: spec.hostname || '—' })],
			st.network_name ? [t('Docker network'), h('span', { class: 'mono', text: st.network_name })] : null,
		])));

		// image and updates
		out.push(section(t('Image'), facts([
			[t('Image'), h('span', { class: 'mono', text: spec.image })],
			[t('Version'), h('span', { text: st.image_version || '—' })],
			[t('Updates'), updateLine(inst)],
		])));

		// folders
		const mounts = spec.mounts || [];
		const mountRow = (host, path, ro) => h('div', { class: 'mount-item' }, [
			...HB.tn('{folder} mounted at {path}', { folder: h('span', { class: 'mount-path', text: host }), path: h('span', { class: 'mount-path', text: path }) }).map((p) => (typeof p === 'string' ? h('span', { class: 'mount-arrow', text: p.trim() }) : p)),
			ro ? h('span', { class: 'chip', text: t('Read-only') }) : null,
		]);
		out.push(section(t('Folders'), h('div', { class: 'mount-list' }, [
			mountRow(spec.workdir, '/homebridge', false),
			...mounts.map((m) => mountRow(m.host, m.container, m.ro)),
		])));

		HB.clear(panel);
		panel.appendChild(h('div', { class: 'panel-stack' }, out));
	}

	function updateLine(inst) {
		const up = inst.update || {};
		const name = inst.spec.name;
		const parts = [];
		if (up.legacy_image) {
			parts.push(h('span', { class: 'chip chip-warn', text: t('No longer maintained') }));
			parts.push(HB.button(t('Switch image and upgrade'), { kind: 'ghost', size: 'sm', onClick: () => HB.actions.upgrade(name) }));
		} else if (up.available) {
			parts.push(h('span', { class: 'chip chip-violet', text: up.reason === 'newer_local' ? t('A newer image is on this NAS') : t('An update is available') }));
			parts.push(HB.button(t('Upgrade'), { kind: 'ghost', size: 'sm', onClick: () => HB.actions.upgrade(name) }));
		} else if (up.checked_at) {
			parts.push(h('span', { class: 'chip chip-ok', text: t('Up to date') }));
		} else {
			parts.push(h('span', { class: 'muted', text: t('Not checked yet') }));
		}
		const lines = [h('div', { class: 'chips', style: { alignItems: 'center' } }, parts)];
		if (up.error) {
			lines.push(h('p', { class: 'check-line is-warn' }, [icon('alert', { size: 16 }), h('span', { text: t('The last check failed: {reason}', { reason: HB.msg(up.error_msg) || HB.msg(up.error) }) })]));
		}
		if (up.checked_at) {
			lines.push(h('p', { class: 'muted', style: { fontSize: '12.5px' }, text: t('Checked {ago}', { ago: HB.ago(up.checked_at) }) }));
		}
		return h('div', { class: 'form-stack', style: { gap: '4px' } }, lines);
	}

	function paintPin(wrap, pin) {
		HB.clear(wrap);
		if (!pin) {
			return;
		}
		const shown = cur.pinShown;
		const code = h('span', { class: ['pin-code', shown ? null : 'is-hidden'], text: shown ? pin : '•••-••-•••', 'aria-label': shown ? t('Setup code {pin}', { pin }) : t('Setup code hidden') });
		const toggle = h('button', { type: 'button', class: 'icon-btn icon-btn-sm', 'aria-label': shown ? t('Hide the setup code') : t('Show the setup code'), title: shown ? t('Hide') : t('Show') }, icon(shown ? 'eyeOff' : 'eye'));
		toggle.addEventListener('click', () => {
			cur.pinShown = !cur.pinShown;
			paintPin(wrap, pin);
			wrap.querySelector('button').focus();
		});
		const copy = h('button', { type: 'button', class: 'icon-btn icon-btn-sm', 'aria-label': t('Copy the setup code'), title: t('Copy') }, icon('copy'));
		copy.addEventListener('click', async () => {
			const ok = await HB.copyText(pin);
			HB.toast(ok ? t('Setup code copied') : t('Could not copy. Type it in yourself.'), ok ? 'ok' : 'warn');
		});
		HB.append(wrap, [icon('home', { size: 20 }), code, toggle, shown ? copy : null]);
	}

	async function checkDomain(domain, ip, line) {
		const key = domain + '|' + (ip || '');
		const cached = domainCache.get(key);
		if (cached && Date.now() - cached.t < 60000) {
			HB.specform.paintDomainResult(line, cached.r);
			return;
		}
		line.set('busy', t('Looking it up in DNS…'));
		try {
			const r = await HB.api.post('/check/domain', { domain, ip: ip || undefined });
			domainCache.set(key, { r, t: Date.now() });
			HB.specform.paintDomainResult(line, r);
		} catch (err) {
			if (!err.handled) {
				line.set('info', t('Cannot look it up in DNS right now ({reason})', { reason: HB.errorText(err) }));
			}
		}
	}

	function startStats() {
		const c = cur;
		if (!c || c.statsTimer || c.inst.state.status !== 'running') {
			if (c && c.inst.state.status === 'running' && c.meters) {
				loadStats();
			}
			return;
		}
		loadStats();
		c.statsTimer = setInterval(() => {
			if (!document.hidden) {
				loadStats();
			}
		}, 5000);
	}

	async function loadStats() {
		const c = cur;
		if (!c || !c.meters) {
			return;
		}
		try {
			const s = await HB.api.get('/instances/' + HB.enc(c.name) + '/stats');
			if (cur !== c || !c.meters) {
				return;
			}
			c.lastStats = s;
			paintStats(s);
		} catch (err) {
			/* stats are a nicety */
		}
	}

	function paintStats(s) {
		const c = cur;
		c.meters.cpu.textContent = s.cpu_percent === null || s.cpu_percent === undefined ? '—' : s.cpu_percent.toFixed(1) + '%';
		c.meters.mem.textContent = s.mem_bytes === null || s.mem_bytes === undefined ? '—' : HB.bytes(s.mem_bytes);
	}

	// ---------- settings ----------

	function renderSettings(panel) {
		const c = cur;
		const inst = c.inst;
		const name = inst.spec.name;
		const server = () => cur.inst.spec;

		// instant group: display name, sign, colour, domain
		const a = {};
		for (const k of INSTANT_KEYS) {
			a[k] = HB.deepCopy(inst.spec[k]) || (k === 'domain' ? '' : inst.spec[k]);
		}
		const aSave = HB.button(t('Save'), { kind: 'primary', size: 'sm', disabled: true });
		const aNote = h('span', { class: 'group-note', text: t('Takes effect as soon as you save; no recreate needed.') });
		const aChanged = () => {
			const dirty = INSTANT_KEYS.some((k) => (a[k] || '') !== (server()[k] || ''));
			aSave.disabled = !dirty;
			aNote.textContent = dirty ? t('Not saved yet. Takes effect as soon as you save; no recreate needed.') : t('Takes effect as soon as you save; no recreate needed.');
		};
		const domainF = HB.specform.domainField(a, aChanged, { ip: () => cur.inst.state.ip || (server().network.mode === 'static' ? server().network.ip : null) });
		const groupA = h('section', { class: 'group is-instant' }, [
			h('div', { class: 'group-head' }, [icon('sparkle', { size: 18 }), h('div', null, [h('h3', { class: 'group-title', text: t('Name, look and domain') }), h('p', { class: 'group-text', text: t('Changes only what you see here and in the browser.') })])]),
			h('div', { class: 'form-stack' }, [
				HB.specform.displayNameField(a, aChanged),
				HB.specform.emojiField(a, aChanged),
				HB.specform.colorField(a, aChanged),
				domainF,
			]),
			h('div', { class: 'group-foot' }, [aNote, aSave]),
		]);
		aSave.addEventListener('click', () => HB.busy(aSave, t('Saving…'), async () => {
			HB.clearFieldErrors(groupA);
			const d = domainF.validate();
			if (d) {
				HB.showFieldErrors(groupA, { domain: d });
				return;
			}
			const spec = HB.deepCopy(server());
			for (const k of INSTANT_KEYS) {
				spec[k] = a[k];
			}
			try {
				const fresh = await HB.api.put('/instances/' + HB.enc(name), { spec, confirm_ip_in_use: false });
				HB.actions.store(fresh);
				HB.toast(t('Saved; it takes effect now'), 'ok');
				aChanged();
			} catch (err) {
				handleSaveError(err, groupA);
			}
		}).then(aChanged));

		// rebuild group: the container's own settings
		const b = HB.deepCopy(inst.spec);
		const bSave = HB.button(t('Save'), { kind: 'primary', size: 'sm', disabled: true });
		const bNote = h('span', { class: 'group-note', text: t('After saving, the new settings are used once you choose “Apply and recreate”.') });
		const pick = (s) => JSON.stringify(REBUILD_KEYS.map((k) => s[k] === undefined ? null : s[k]));
		const bChanged = () => {
			const dirty = pick(b) !== pick(server());
			bSave.disabled = !dirty;
			bNote.textContent = dirty ? t('Not saved yet. After saving, the new settings are used once you choose “Apply and recreate”.') : t('After saving, the new settings are used once you choose “Apply and recreate”.');
		};
		const net = HB.specform.networkFields(b, bChanged, { exclude: () => name, hostnameFollows: false, checkNow: false });
		const folders = HB.specform.folderFields(b, bChanged, { checkNow: false, self: name });
		const imageF = HB.specform.imageField(b, bChanged, { defaultImage: (HB.settings && HB.settings.default_image) || null });
		const groupB = h('section', { class: 'group is-rebuild' }, [
			h('div', { class: 'group-head' }, [icon('refresh', { size: 18 }), h('div', null, [h('h3', { class: 'group-title', text: t('Container settings') }), h('p', { class: 'group-text', text: t('The container is created from these settings, so a change takes effect after a recreate.') })])]),
			h('div', { class: 'form-stack' }, [imageF, net.el, folders.el]),
			h('div', { class: 'group-foot' }, [bNote, bSave]),
		]);
		let confirmIp = false;
		bSave.addEventListener('click', () => HB.busy(bSave, t('Saving…'), async () => {
			HB.clearFieldErrors(groupB);
			if (b.network.mode === 'static' && b.network.ip && b.network.ip !== server().network.ip) {
				await net.ipCheck();
			}
			const errs = Object.assign({}, net.validate(), folders.validate());
			const im = imageF.validate();
			if (im) {
				errs.image = im;
			}
			if (Object.keys(errs).length) {
				HB.showFieldErrors(groupB, errs);
				return;
			}
			const spec = HB.deepCopy(server());
			for (const k of REBUILD_KEYS) {
				spec[k] = HB.deepCopy(b[k]);
			}
			if (spec.network.mode === 'dhcp') {
				spec.network.ip = '';
			}
			for (let attempt = 0; attempt < 2; attempt++) {
				try {
					const fresh = await HB.api.put('/instances/' + HB.enc(name), { spec, confirm_ip_in_use: confirmIp });
					HB.actions.store(fresh);
					confirmIp = false;
					if (fresh.pending_changes) {
						HB.toast(t('Saved. The new settings are used once you choose “Apply and recreate”.'), 'ok', { action: { label: t('Apply and recreate'), onClick: () => HB.actions.apply(name) } });
					} else {
						HB.toast(t('Saved'), 'ok');
					}
					// keep the editors in step with what the server now holds
					Object.assign(b, HB.deepCopy(fresh.spec));
					bChanged();
					return;
				} catch (err) {
					const d = err.details || {};
					if (err.code === 'conflict' && d.kind === 'ip_in_use' && d.by === 'lan' && !confirmIp) {
						const ok = await HB.actions.confirmIpInUse(spec.network.ip, d);
						if (!ok) {
							return;
						}
						confirmIp = true;
						continue;
					}
					if (err.code === 'conflict' && d.kind === 'ip_in_use') {
						HB.showFieldErrors(groupB, { 'network.ip': HB.errorText(err) });
						return;
					}
					handleSaveError(err, groupB);
					return;
				}
			}
		}).then(bChanged));

		// danger zone
		const groupC = h('section', { class: 'group danger-zone' }, [
			h('div', { class: 'group-head' }, [icon('trash', { size: 18 }), h('div', null, [h('h3', { class: 'group-title', text: t('Remove this Homebridge') }), h('p', { class: 'group-text', text: t('You can stop managing it (the container keeps running) or remove the container too. Folders are never deleted.') })])]),
			h('div', null, HB.button(t('Remove…'), { kind: 'danger-quiet', size: 'sm', icon: 'trash', onClick: () => HB.actions.remove(name) })),
		]);

		c.settings = {
			dirty: () => !aSave.disabled || !bSave.disabled,
		};
		HB.clear(panel);
		panel.appendChild(h('div', { class: 'panel-stack' }, [groupA, groupB, groupC]));
	}

	function handleSaveError(err, root) {
		HB.formError(root, err);
	}

	// ---------- logs ----------

	function renderLogs(panel) {
		const c = cur;
		const name = c.name;
		if (c.inst.state.status === 'missing') {
			HB.clear(panel);
			panel.appendChild(h('p', { class: 'rows-empty', text: t('This Homebridge has no container right now, so there are no logs.') }));
			return;
		}
		let tail = 200;
		let auto = true;
		const pre = h('pre', { class: 'log log-view', tabindex: '0', 'aria-label': t('Container log'), 'aria-live': 'off', 'data-empty': t('Nothing logged yet.') });
		const note = h('p', { class: 'muted', style: { fontSize: '12.5px' } });
		const tailSel = h('select', { class: 'select', 'aria-label': t('Lines to show') }, [200, 500, 1000, 2000].map((n) => h('option', { value: String(n), text: t('Last {count} lines', { count: n }) })));
		tailSel.addEventListener('change', () => {
			tail = Number(tailSel.value);
			load(true);
		});
		const autoSw = HB.switchControl({ checked: true, label: t('Refresh automatically'), onChange: (v) => {
			auto = v;
			schedule();
		} });
		const copyBtn = HB.button(t('Copy'), { icon: 'copy', size: 'sm', onClick: async () => {
			const ok = await HB.copyText(pre.textContent);
			HB.toast(ok ? t('Log copied') : t('Could not copy. Select the text and copy it yourself.'), ok ? 'ok' : 'warn');
		} });
		const bottomBtn = HB.button(t('Jump to the latest'), { icon: 'chevronDown', size: 'sm', kind: 'ghost', onClick: () => {
			pre.scrollTop = pre.scrollHeight;
		} });
		HB.clear(panel);
		HB.append(panel, [
			h('div', { class: 'log-toolbar' }, [autoSw, tailSel, h('span', { class: 'grow' }), bottomBtn, copyBtn]),
			pre,
			note,
		]);
		let last = '';
		async function load(force) {
			if (cur !== c) {
				return;
			}
			let r;
			try {
				r = await HB.api.get('/instances/' + HB.enc(name) + '/logs', { tail });
			} catch (err) {
				if (!err.handled) {
					note.textContent = t('Cannot read the log: {reason}', { reason: HB.errorText(err) });
				}
				return;
			}
			const text = r.lines.join('\n');
			if (text === last && !force) {
				return;
			}
			const sel = window.getSelection();
			if (!force && sel && !sel.isCollapsed && pre.contains(sel.anchorNode)) {
				return; // do not pull text out from under a selection
			}
			last = text;
			const atBottom = force || pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 30 || !pre.textContent;
			HB.clear(pre);
			const frag = document.createDocumentFragment();
			for (const line of r.lines) {
				const m = /^(\d{4}-\d\d-\d\dT[\d:.]+Z?)\s(.*)$/.exec(line);
				const body = m ? m[2] : line;
				const cls = /error|fail|exception|cannot/i.test(body) ? 'l-err' : (/warn/i.test(body) ? 'l-warn' : null);
				const row = h('span', { class: cls });
				if (m) {
					const at = HB.parseTime(m[1]);
					row.appendChild(h('span', { class: 'l-time', text: (at === null ? m[1] : HB.shortTime(at)) + '  ' }));
				}
				row.appendChild(document.createTextNode(body + '\n'));
				frag.appendChild(row);
			}
			pre.appendChild(frag);
			if (!r.lines.length) {
				pre.textContent = '';
			}
			if (atBottom) {
				pre.scrollTop = pre.scrollHeight;
			}
			note.textContent = r.truncated ? t('Only the last {count} lines are shown. Times are in this computer’s time zone.', { count: tail }) : t('Times are in this computer’s time zone.');
		}
		function schedule() {
			if (c.logTimer) {
				clearInterval(c.logTimer);
				c.logTimer = null;
			}
			if (auto && cur === c && c.tabs.current === 'logs') {
				c.logTimer = setInterval(() => {
					if (!document.hidden) {
						load(false);
					}
				}, 3000);
			}
		}
		c.logResume = () => {
			load(false);
			schedule();
		};
		load(true);
		schedule();
	}

	// ---------- backup ----------

	function renderBackup(panel) {
		const c = cur;
		const inst = c.inst;
		const b = inst.backup;
		const out = [
			HB.callout('info', t('What the backup is'), h('p', { text: t('Every upgrade or Apply and recreate stops the previous container and keeps it, renamed, as the backup. If the new one has a problem, you can go back in one step. The backup does not start with the NAS and does not take up an address.') }), 'shield'),
		];
		if (b) {
			out.push(h('div', { class: 'backup-card' }, [
				h('div', { class: 'backup-icon' }, icon('shield', { size: 22 })),
				h('div', null, [
					h('h3', { class: 'group-title', text: t('The container kept on {date}', { date: HB.dateTime(b.created_at) }) }),
					h('p', { class: 'group-text', text: HB.ago(b.created_at) }),
					h('div', { style: { marginTop: '10px' } }, facts([
						[t('Container name'), h('span', { class: 'mono', text: b.container_name })],
						[t('Image'), h('span', { class: 'mono', text: b.image })],
					])),
					h('div', { class: 'backup-actions' }, [
						HB.button(t('Go back to this backup'), { kind: 'primary', size: 'sm', icon: 'undo', disabled: !!inst.job, onClick: () => HB.actions.rollback(inst.spec.name) }),
						HB.button(t('Delete backup'), { kind: 'danger-quiet', size: 'sm', icon: 'trash', disabled: !!inst.job, onClick: () => HB.actions.deleteBackup(inst.spec.name) }),
					]),
				]),
			]));
		} else {
			out.push(h('p', { class: 'rows-empty', text: t('There is no backup yet. The next upgrade or Apply and recreate keeps one automatically.') }));
		}
		HB.clear(panel);
		panel.appendChild(h('div', { class: 'panel-stack' }, out));
	}

	/* After a language switch: build the open drawer again on the same tab, unless it holds unsaved edits. */
	function relang() {
		if (!cur || cur.dirty()) {
			return;
		}
		const name = cur.name;
		const tab = cur.tabs.current;
		close(true, true);
		const inst = HB.findInstance(name);
		if (inst) {
			build(inst, tab);
		}
	}

	HB.drawer = { open, openFromRoute, closeFromRoute, close, relang, isOpen: () => !!cur };
})();
