/* Homebridge UI - the create wizard (four real steps, so they are numbered)
 * with a live preview of the house, and the same wizard prefilled from
 * GET /discover to take over an existing container. Ends in the job panel:
 * scaffolding goes up, comes down, and the windows switch on. */
(function () {
	'use strict';

	const { h, icon, t } = HB;
	const STEPS = [
		{ key: 'basic', title: HB.N_('Basics') },
		{ key: 'network', title: HB.N_('Network') },
		{ key: 'folders', title: HB.N_('Folders') },
		{ key: 'review', title: HB.N_('Review') },
	];

	async function ensureNetworks() {
		if (!HB.networks) {
			HB.networks = await HB.api.get('/networks');
		}
		if (!HB.settings) {
			try {
				HB.settings = await HB.api.get('/settings');
			} catch (e) {
				if (e.handled) {
					throw e;
				}
			}
		}
	}

	function nextName() {
		const used = new Set(HB.instances.map((i) => i.spec.name.toLowerCase()).concat(HB.discovered.map((c) => c.name.toLowerCase())));
		if (!used.size) {
			return 'homebridge';
		}
		for (let n = HB.instances.length + 1; n < 1000; n++) {
			if (!used.has('homebridge-' + n)) {
				return 'homebridge-' + n;
			}
		}
		return 'homebridge-new';
	}

	/* The folder new work directories go into: wherever the others live. */
	function suggestedParent() {
		const dirs = HB.instances.map((i) => i.spec.workdir).concat(HB.discovered.map((c) => c.spec && c.spec.workdir)).filter(Boolean);
		const count = {};
		for (const d of dirs) {
			const p = d.replace(/\/[^/]+$/, '');
			if (p.split('/').length >= 3) {
				count[p] = (count[p] || 0) + 1;
			}
		}
		const best = Object.keys(count).sort((a, b) => count[b] - count[a])[0];
		return best || '/share/Container';
	}

	function unusedColor() {
		const used = new Set(HB.instances.map((i) => i.spec.color));
		const free = HB.COLORS.map((c) => c[0]).find((c) => !used.has(c));
		return free || HB.COLORS[HB.instances.length % HB.COLORS.length][0];
	}

	async function openCreate() {
		try {
			await ensureNetworks();
		} catch (err) {
			HB.fail(err);
			return;
		}
		const defaults = (HB.session && HB.session.defaults) || {};
		const name = nextName();
		const spec = {
			name,
			display_name: '',
			emoji: HB.EMOJI[HB.instances.length % 8],
			color: unusedColor(),
			image: (HB.settings && HB.settings.default_image) || defaults.image || 'homebridge/homebridge:latest',
			domain: '',
			hostname: '',
			network: { mode: 'static', iface: '', ip: '', mac: '' },
			workdir: suggestedParent() + '/' + name,
			mounts: HB.deepCopy((HB.settings && HB.settings.default_mounts) || []),
			env: HB.deepCopy(defaults.env || { TZ: (HB.session && HB.session.host && HB.session.host.timezone) || 'Asia/Taipei' }),
			restart: defaults.restart || 'always',
		};
		open({ mode: 'create', spec, step: 0 });
	}

	async function openImport(container) {
		try {
			await ensureNetworks();
		} catch (err) {
			HB.fail(err);
			return;
		}
		const spec = HB.deepCopy(container.spec);
		spec.name = container.name;
		if (!spec.display_name || spec.display_name === container.name) {
			spec.display_name = '';
		}
		spec.emoji = spec.emoji || '🏠';
		spec.color = HB.instances.some((i) => i.spec.color === spec.color) ? unusedColor() : (spec.color || unusedColor());
		spec.domain = spec.domain || '';
		spec.mounts = spec.mounts || [];
		spec.env = spec.env || {};
		open({ mode: 'import', spec, step: 0, from: container });
	}

	function open(w) {
		const spec = w.spec;
		const importing = w.mode === 'import';
		let step = w.step;
		let visited = step;
		let busy = false;
		const flags = { replace: importing, confirmIp: false };

		// ---------- preview ----------
		const house = HB.house.create({ size: 'lg' });
		const pName = h('p', { class: 'wiz-preview-name' });
		const pDomain = h('p', { class: 'wiz-preview-domain' });
		const pNote = h('p', { class: 'wiz-preview-note' });
		const preview = h('aside', { class: 'wiz-preview', 'aria-label': t('Preview') }, [house.el, h('div', null, [pName, pDomain, pNote])]);

		function paintPreview() {
			const ip = spec.network.mode === 'static' ? spec.network.ip : null;
			house.update({
				kind: 'draft',
				color: spec.color,
				emoji: spec.emoji,
				lights: importing && w.from.status === 'running' ? 'on' : 'off',
				plate: { text: ip || '—', dhcp: spec.network.mode === 'dhcp' },
			});
			pName.textContent = spec.display_name || spec.name || t('New Homebridge');
			pDomain.textContent = spec.domain || '';
			pDomain.hidden = !spec.domain;
			pNote.textContent = importing ? t('Taking over {name}', { name: w.from.name }) : t('Not created yet');
		}

		function changed(path) {
			paintPreview();
			if (path === 'name' && !importing) {
				if (!touchedWorkdir) {
					const wdv = suggestedParent() + '/' + (spec.name || 'homebridge');
					spec.workdir = wdv;
					folders.workdir.setValue(wdv);
				}
				network.hostnameSync();
			}
			if (path === 'domain') {
				network.hostnameSync();
			}
			if (path === 'workdir') {
				touchedWorkdir = true;
			}
			if (path === 'network.ip' || path === 'network.mode') {
				domain.recheck && debouncedDomain();
			}
		}
		let touchedWorkdir = importing;
		const debouncedDomain = HB.debounce(() => domain.recheck(), 900);

		// ---------- pages ----------
		const basic = h('section', { class: 'wiz-page', 'data-step': '0' });
		const dn = HB.specform.displayNameField(spec, changed);
		const nameF = HB.specform.nameField(spec, changed, importing ? { readOnly: true, readOnlyHelp: t('Keeps the original container name; the takeover does not change it.') } : {});
		const imageF = HB.specform.imageField(spec, changed, { defaultImage: (HB.settings && HB.settings.default_image) || 'homebridge/homebridge:latest' });
		HB.append(basic, [
			importing ? importCallout(w.from, () => go(3)) : null,
			h('h3', { class: 'wiz-page-title', text: importing ? t('Give it a name') : t('Name and look') }),
			dn,
			HB.specform.emojiField(spec, changed),
			HB.specform.colorField(spec, changed),
			h('div', { class: 'form-row' }, [nameF, imageF]),
		]);

		const domain = HB.specform.domainField(spec, changed, { ip: () => (spec.network.mode === 'static' ? spec.network.ip : (importing ? w.from.ip : null)), checkNow: false });
		const network = HB.specform.networkFields(spec, changed, {
			exclude: () => (importing ? w.from.name : undefined),
			hostnameFollows: true,
			slot: domain,
			openAdvancedIfMac: importing,
			checkNow: importing,
		});
		const netPage = h('section', { class: 'wiz-page', 'data-step': '1', hidden: true }, [
			h('h3', { class: 'wiz-page-title', text: t('Its address on the local network') }),
			h('p', { class: 'wiz-page-lead', text: t('Every Homebridge has its own address on the local network. HomeKit and the Homebridge web page find it there.') }),
			network.el,
		]);

		const folders = HB.specform.folderFields(spec, changed, { suggestParent: suggestedParent, checkNow: true, creating: !importing, existingText: importing ? t('Uses the original work folder. The settings, plugins and pairing in it are kept.') : undefined });
		const folderPage = h('section', { class: 'wiz-page', 'data-step': '2', hidden: true }, [
			h('h3', { class: 'wiz-page-title', text: t('Where its data lives') }),
			folders.el,
		]);

		const review = h('section', { class: 'wiz-page', 'data-step': '3', hidden: true });
		const pages = [basic, netPage, folderPage, review];
		const layout = h('div', { class: 'wiz-layout' }, [preview, h('div', { class: 'wiz-pages' }, pages)]);

		// ---------- step nav ----------
		const nav = h('ol', { class: 'wiz-steps', 'aria-label': t('Steps') });
		const navBtns = STEPS.map((s, i) => {
			const b = h('button', { type: 'button', class: 'wiz-step' }, [h('span', { class: 'wiz-num', text: String(i + 1) }), h('span', { text: t(s.title) })]);
			b.addEventListener('click', () => {
				if (i <= visited || importing) {
					go(i);
				}
			});
			nav.appendChild(h('li', null, b));
			return b;
		});

		const initial = JSON.stringify(spec);
		const ctl = HB.dialog({
			title: importing ? t('Take over {name}', { name: w.from.name }) : t('Add Homebridge'),
			head: nav,
			size: 'xl',
			className: 'wizard',
			body: layout,
			focusBody: true,
			dismissible: true,
			actions: [],
			beforeDismiss: () => {
				if (progressStarted || JSON.stringify(spec) === initial) {
					return true;
				}
				return HB.confirm({
					title: importing ? t('Stop the takeover?') : t('Stop adding?'),
					body: t('What you filled in will not be kept.'),
					cancel: t('Keep editing'),
					confirm: importing ? t('Stop the takeover') : t('Stop adding'),
				});
			},
		});

		function paintNav() {
			navBtns.forEach((b, i) => {
				b.classList.toggle('is-current', i === step);
				b.classList.toggle('is-done', i < step || (i <= visited && i !== step && importing));
				b.disabled = !(i <= visited || importing) || busy;
				if (i === step) {
					b.setAttribute('aria-current', 'step');
				} else {
					b.removeAttribute('aria-current');
				}
			});
		}

		function paintFoot() {
			const last = step === STEPS.length - 1;
			ctl.setActions([
				{ label: t('Cancel'), kind: 'quiet', close: false, onClick: () => ctl.dismiss() },
				'spacer',
				step > 0 ? { label: t('Back'), kind: 'quiet', close: false, onClick: () => go(step - 1) } : null,
				last
					? { label: importing ? t('Take over and restart') : t('Create Homebridge'), kind: 'primary', close: false, onClick: (c, b) => submit(b) }
					: { label: t('Next'), kind: 'primary', close: false, onClick: (c, b) => next(b) },
			].filter(Boolean));
		}

		async function go(i) {
			if (i === step) {
				return;
			}
			step = i;
			visited = Math.max(visited, i);
			pages.forEach((p, n) => {
				p.hidden = n !== i;
			});
			if (i === 3) {
				paintReview();
			}
			if (i === 1 && spec.domain) {
				domain.recheck();
			}
			paintNav();
			paintFoot();
			ctl.body.scrollTop = 0;
			const first = pages[i].querySelector('input:not([readonly]):not([type=radio]):not([type=checkbox]), select');
			if (first && i !== 3) {
				first.focus({ preventScroll: true });
			} else {
				const primary = ctl.foot.querySelector('.btn-primary');
				if (primary) {
					primary.focus();
				}
			}
		}

		function stepErrors(i) {
			let errs = {};
			if (i === 0) {
				const n = nameF.validate();
				if (n) {
					errs.name = n;
				}
				const im = imageF.validate();
				if (im) {
					errs.image = im;
				}
				if ((spec.display_name || '').length > 40) {
					errs.display_name = t('A display name has at most 40 characters');
				}
			} else if (i === 1) {
				errs = network.validate();
				const d = domain.validate();
				if (d) {
					errs.domain = d;
				}
			} else if (i === 2) {
				errs = folders.validate();
			}
			return errs;
		}

		async function next(btn) {
			HB.clearFieldErrors(pages[step]);
			if (step === 1 && spec.network.mode === 'static' && spec.network.ip) {
				await HB.busy(btn, t('Checking the address…'), () => network.ipCheck());
			}
			const errs = stepErrors(step);
			if (Object.keys(errs).length) {
				HB.showFieldErrors(pages[step], errs);
				return false;
			}
			go(step + 1);
			return false;
		}

		// ---------- review ----------
		function paintReview() {
			HB.clear(review);
			const net = spec.network;
			const ifc = ((HB.networks && HB.networks.interfaces) || []).find((i) => i.iface === net.iface);
			const where = net.ip;
			const label = spec.display_name || spec.name;
			review.appendChild(h('h3', { class: 'wiz-page-title', text: importing ? t('Check before taking over') : t('Check before creating') }));
			const lead = {
				name: label,
				image: h('span', { class: 'mono', text: spec.image }),
				ip: h('strong', { text: where || '' }),
				domain: h('span', { class: 'mono', text: spec.domain || '' }),
				folder: h('span', { class: 'mono', text: spec.workdir }),
			};
			let leadKey;
			if (net.mode === 'static') {
				leadKey = spec.domain
					? HB.N_('“{name}” will run {image} at {ip} on the local network, open its web page at {domain}, and keep its settings and pairing in {folder}.')
					: HB.N_('“{name}” will run {image} at {ip} on the local network and keep its settings and pairing in {folder}.');
			} else {
				leadKey = spec.domain
					? HB.N_('“{name}” will run {image} with an address from the router, open its web page at {domain}, and keep its settings and pairing in {folder}.')
					: HB.N_('“{name}” will run {image} with an address from the router and keep its settings and pairing in {folder}.');
			}
			review.appendChild(h('p', { class: 'summary-lead' }, HB.tn(leadKey, lead)));
			const mounts = spec.mounts || [];
			const envKeys = Object.keys(spec.env || {});
			const restartText = {
				always: t('Always start it (recommended)'),
				'unless-stopped': t('Start it unless stopped by hand'),
				no: t('Do not start it'),
			}[spec.restart || 'always'];
			const colour = HB.COLORS.find((c) => c[0] === spec.color) || HB.COLORS[0];
			const iface = ifc ? ifc.display : net.iface;
			const rows = [
				[t('Look'), [h('span', null, [h('span', { class: 'emoji-inline', text: spec.emoji + ' ' }), label]), h('span', { class: 'sub', text: t('{colour} roof, container name {name}', { colour: t(colour[1]), name: spec.name }) })], 0],
				[t('Image'), [h('span', { class: 'mono', text: spec.image }), /^oznu\/homebridge/.test(spec.image) ? h('span', { class: 'sub', text: t('No longer maintained. After creating, you can switch to the new image in one step (click the parcel at the door).') }) : null], 0],
				[t('Network'), [net.mode === 'static' ? h('span', null, HB.tn('Fixed address {ip}', { ip: h('span', { class: 'inline-plate', text: net.ip }) })) : h('span', { text: t('Automatic (DHCP)') }), h('span', { class: 'sub', text: net.mac ? t('{interface}, MAC {mac}', { interface: iface, mac: net.mac }) : t('{interface}, MAC generated automatically', { interface: iface }) })], 1],
				[t('Domain'), [h('span', { class: spec.domain ? 'mono' : 'muted', text: spec.domain || t('Not set; it opens by IP address') })], 1],
				[t('Folders'), [h('span', { class: 'mono', text: spec.workdir }), h('span', { class: 'sub', text: mounts.length ? t('Other folders: {folders}', { folders: HB.list(mounts.map((m) => (m.ro ? t('{path} (read-only)', { path: m.host }) : m.host))) }) : t('No other folders') })], 2],
				[t('Other'), [h('span', { text: restartText }), h('span', { class: 'sub', text: envKeys.length ? t('Environment variables: {names}', { names: HB.list(envKeys) }) : t('No environment variables') })], 2],
			];
			const box = h('div', { class: 'summary' });
			for (const [k, v, target] of rows) {
				const edit = HB.button(t('Change'), { kind: 'ghost', size: 'sm', className: 'summary-edit', onClick: () => go(target) });
				edit.setAttribute('aria-label', t('Change {section}', { section: k }));
				box.appendChild(h('div', { class: 'summary-row' }, [h('span', { class: 'summary-key', text: k }), h('div', { class: 'summary-val' }, v), edit]));
			}
			review.appendChild(box);
			if (importing) {
				review.appendChild(takeoverCallout(w.from));
				if (w.from.notes && w.from.notes.length) {
					review.appendChild(HB.callout('warn', t('These settings do not come over'), h('ul', null, w.from.notes.map((n) => h('li', { text: HB.msg(n) })))));
				}
			} else {
				review.appendChild(HB.callout('info', null, h('p', { text: t('Creating downloads the image (the first time can take a few minutes), creates the container and checks that it runs. Then you can add it in the Home app on your iPhone.') })));
			}
		}

		// ---------- submit ----------
		async function submit(btn) {
			if (busy) {
				return false;
			}
			for (let i = 0; i < 3; i++) {
				const errs = stepErrors(i);
				if (Object.keys(errs).length) {
					await go(i);
					HB.showFieldErrors(pages[i], errs);
					return false;
				}
			}
			const body = () => ({ spec: cleanSpec(spec), replace_existing: flags.replace, confirm_ip_in_use: flags.confirmIp });
			busy = true;
			ctl.setBusy(true);
			paintNav();
			const span = btn.querySelector('span');
			const label = span.textContent;
			span.textContent = t('Sending…');
			try {
				for (let attempt = 0; attempt < 3; attempt++) {
					try {
						const r = await HB.api.post('/instances', body());
						startProgress(r.job_id);
						return false;
					} catch (err) {
						if (err.handled) {
							return false;
						}
						const d = err.details || {};
						if (err.code === 'conflict' && d.kind === 'container_exists' && !flags.replace) {
							ctl.setBusy(false);
							const ok = await confirmReplace(spec.name, d);
							ctl.setBusy(true);
							if (!ok) {
								return false;
							}
							flags.replace = true;
							continue;
						}
						if (err.code === 'conflict' && d.kind === 'ip_in_use' && d.by === 'lan' && !flags.confirmIp) {
							ctl.setBusy(false);
							const ok = await HB.actions.confirmIpInUse(spec.network.ip, d);
							ctl.setBusy(true);
							if (!ok) {
								await go(1);
								return false;
							}
							flags.confirmIp = true;
							continue;
						}
						if (err.code === 'conflict' && d.kind === 'ip_in_use') {
							await go(1);
							HB.showFieldErrors(pages[1], { 'network.ip': HB.errorText(err) });
							return false;
						}
						if (err.fields) {
							const target = firstStepWith(err.fields);
							await go(target);
							HB.formError(ctl.body, err);
							return false;
						}
						HB.fail(err);
						return false;
					}
				}
			} finally {
				if (document.contains(span)) {
					span.textContent = label;
				}
				busy = false;
				if (!progressStarted) {
					ctl.setBusy(false);
					paintNav();
				}
			}
			return false;
		}

		function firstStepWith(fields) {
			let best = 3;
			for (const path of Object.keys(fields)) {
				const el = ctl.body.querySelector('[data-path="' + CSS.escape(path) + '"]');
				const page = el ? el.closest('.wiz-page') : null;
				if (page) {
					best = Math.min(best, Number(page.dataset.step));
				}
			}
			return best;
		}

		// ---------- progress ----------
		let progressStarted = false;
		function startProgress(jobId) {
			progressStarted = true;
			busy = false;
			ctl.setBusy(false);
			const label = spec.display_name || spec.name;
			ctl.setTitle(importing ? t('Taking over “{name}”', { name: label }) : t('Creating “{name}”', { name: label }));
			nav.remove();
			HB.clear(ctl.body);
			const panel = HB.jobs.panel(jobId, {
				instance: spec.name,
				draft: { spec },
				takeover: importing || flags.replace,
				onFinish: (job) => {
					const acts = ['spacer'];
					if (job.status === 'succeeded') {
						acts.push(HB.jobs.openButton(null));
						acts.push({ label: t('Done'), kind: 'primary', autofocus: true });
					} else {
						acts.unshift({ label: t('Close'), kind: 'quiet' });
						// a create that did not succeed leaves no Homebridge behind: go back to the form with what was filled in
						acts.push({ label: t('Go back and edit'), kind: 'primary', onClick: () => {
							setTimeout(() => open({ mode: w.mode, spec: HB.deepCopy(spec), step: 0, from: w.from }), 50);
						} });
					}
					ctl.setActions(acts);
					const p = ctl.foot.querySelector('.btn-primary');
					if (p) {
						p.focus();
					}
				},
				onReady: (url) => {
					const a = ctl.foot.querySelector('.open-when-ready');
					if (a) {
						HB.jobs.setOpenButton(a, url);
					}
				},
			});
			ctl.body.appendChild(panel.el);
			ctl.setActions(['spacer', { label: t('Continue in the background'), kind: 'quiet' }]);
			const old = ctl.el.querySelector('.dlg-close');
			if (old) {
				old.focus();
			}
			const prevClose = ctl.close;
			ctl.close = (v) => {
				panel.destroy();
				prevClose(v);
			};
			HB.refresh(true);
		}

		paintPreview();
		paintNav();
		paintFoot();
		if (step !== 0) {
			go(step);
		}
		return ctl;
	}

	/* The spec as POST /instances wants it: no blank optional values. */
	function cleanSpec(spec) {
		const s = HB.deepCopy(spec);
		if (!s.display_name) {
			delete s.display_name;
		}
		if (!s.domain) {
			s.domain = '';
		}
		if (!s.hostname) {
			delete s.hostname;
		}
		if (s.network.mode === 'dhcp') {
			s.network.ip = '';
		}
		if (!s.network.mac) {
			delete s.network.mac;
		}
		s.mounts = (s.mounts || []).filter((m) => m.host || m.container);
		return s;
	}

	function importCallout(c, toReview) {
		return h('div', { class: 'callout callout-info' }, [
			icon('home', { size: 20 }),
			h('div', { class: 'callout-body' }, [
				h('strong', { class: 'callout-title', text: t('Everything else came over from {name}', { name: c.name }) }),
				h('p', { text: c.ip
					? t('The image {image}, the address {ip}, the folders and the MAC. Give it a name you will recognise, or go straight to the review.', { image: c.image, ip: c.ip })
					: t('The image {image}, the folders and the MAC. Give it a name you will recognise, or go straight to the review.', { image: c.image }) }),
				h('div', { style: { marginTop: '8px' } }, HB.button(t('Go to the review'), { size: 'sm', onClick: toReview })),
			]),
		]);
	}

	function takeoverCallout(c) {
		return HB.callout('info', t('What happens when you take it over'), h('ul', null, [
			h('li', { text: t('Homebridge restarts once, for about a minute.') }),
			h('li', { text: t('The original container stops and is kept as the backup under the name {backup}, so you can go back at any time.', { backup: c.name + '.hbmgr-prev' }) }),
			h('li', { text: t('The name, address, MAC and folders stay the same, and HomeKit pairing is not affected.') }),
			h('li', { text: t('If the new container does not run properly, the original one is put back automatically.') }),
		]), 'shield');
	}

	function confirmReplace(name, d) {
		const running = d.status === 'running';
		const lead = d.image
			? (running ? t('It is running {image}. Should the new Homebridge take over this name?', { image: d.image }) : t('It is stopped and uses {image}. Should the new Homebridge take over this name?', { image: d.image }))
			: (running ? t('It is running. Should the new Homebridge take over this name?') : t('It is stopped. Should the new Homebridge take over this name?'));
		return HB.confirm({
			title: t('A container named {name} already exists', { name }),
			icon: 'home',
			body: [
				h('p', { text: lead }),
				h('ul', null, [
					h('li', { text: t('The old container stops and is kept as the backup under the name {backup}, so you can go back at any time.', { backup: name + '.hbmgr-prev' }) }),
					h('li', { text: t('The new container takes the same name and is then created and started.') }),
					h('li', { text: t('No folder is deleted.') }),
				]),
			],
			cancel: t('Go back and edit'),
			confirm: t('Replace it'),
		});
	}

	HB.wizard = { openCreate, openImport };
})();
