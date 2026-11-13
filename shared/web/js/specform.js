/* Homebridge UI - the fields of a spec, shared by the create wizard and the
 * settings tab of the drawer. Every builder edits a spec object in place and
 * calls onChange(); every field carries data-path so the server's
 * error.fields land on the right input. */
(function () {
	'use strict';

	const { h, icon, t } = HB;

	const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,62}$/;
	const IPV4_RE = /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/;
	const LABEL_RE = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;
	const ENV_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

	function validDomain(d) {
		d = (d || '').toLowerCase().replace(/\.$/, '');
		return d.length <= 253 && d.includes('.') && d.split('.').every((l) => LABEL_RE.test(l));
	}

	function validHostname(d) {
		d = (d || '').toLowerCase();
		return d.length <= 253 && d.split('.').every((l) => LABEL_RE.test(l));
	}

	// ---------- identity ----------

	function displayNameField(spec, onChange) {
		const input = HB.input({ value: spec.display_name || '', maxlength: '40', placeholder: t('For example: First floor, Living room, Lab') });
		input.addEventListener('input', () => {
			spec.display_name = input.value;
			onChange('display_name');
		});
		return HB.field({ label: t('Display name'), path: 'display_name', control: input, help: t('The name you see here and on the street. You can change it at any time.') });
	}

	function emojiField(spec, onChange) {
		const name = HB.uid('emoji');
		const grid = h('div', { class: 'emoji-grid', role: 'radiogroup', 'aria-label': t('Sign') });
		const custom = HB.input({ 'aria-label': t('Another emoji'), maxlength: '16', placeholder: '🙂' });
		const radios = [];
		for (const e of HB.EMOJI) {
			const r = h('input', { type: 'radio', name, value: e, checked: spec.emoji === e, 'aria-label': e });
			r.addEventListener('change', () => {
				if (r.checked) {
					spec.emoji = e;
					custom.value = '';
					onChange('emoji');
				}
			});
			radios.push(r);
			grid.appendChild(h('label', { class: 'emoji-opt' }, [r, h('span', { 'aria-hidden': 'true', text: e })]));
		}
		if (spec.emoji && !HB.EMOJI.includes(spec.emoji)) {
			custom.value = spec.emoji;
		}
		custom.addEventListener('input', () => {
			const g = HB.firstGrapheme(custom.value);
			if (!g) {
				return;
			}
			if (HB.byteLength(g) > 16) {
				return;
			}
			spec.emoji = g;
			for (const r of radios) {
				r.checked = false;
			}
			onChange('emoji');
		});
		const wrap = h('div', null, [grid, h('label', { class: 'emoji-custom' }, [h('span', { text: t('Or paste another emoji') }), custom])]);
		return HB.field({ label: t('Sign'), path: 'emoji', control: wrap, help: t('The emoji on the roof, so you can tell them apart at a glance.') });
	}

	function colorField(spec, onChange) {
		const name = HB.uid('color');
		const row = h('div', { class: 'color-row', role: 'radiogroup', 'aria-label': t('Roof colour') });
		for (const [value, label] of HB.COLORS) {
			const r = h('input', { type: 'radio', name, value, checked: (spec.color || 'violet') === value, 'aria-label': label });
			r.addEventListener('change', () => {
				if (r.checked) {
					spec.color = value;
					onChange('color');
				}
			});
			row.appendChild(h('label', { class: 'color-opt', title: label, style: { '--sw': 'var(--roof-' + value + ')' } }, [r, h('span', { class: 'color-swatch', 'aria-hidden': 'true' })]));
		}
		return HB.field({ label: t('Roof colour'), path: 'color', control: row });
	}

	/* opts: { readOnly, onInput } */
	function nameField(spec, onChange, opts) {
		opts = opts || {};
		const input = HB.input({ value: spec.name || '', maxlength: '63', class: 'input mono', readOnly: !!opts.readOnly, autocapitalize: 'off' });
		const line = HB.checkLine();
		input.addEventListener('input', () => {
			spec.name = input.value.trim();
			onChange('name');
			check();
		});
		function check() {
			const v = spec.name;
			if (opts.readOnly || !v) {
				line.set(null);
				return;
			}
			if (!NAME_RE.test(v)) {
				line.set('error', t('Use only letters, digits, - _ and ., starting with a letter or digit.'));
			} else if (HB.instances.some((i) => i.spec.name.toLowerCase() === v.toLowerCase())) {
				line.set('error', t('There is already a Homebridge with this name.'));
			} else if (HB.discovered.some((c) => c.name === v)) {
				line.set('warn', t('A container with this name already exists on this NAS. Creating will ask whether to replace it (the old one is kept as the backup).'));
			} else {
				line.set(null);
			}
		}
		check();
		const f = HB.field({
			label: t('Container name'), path: 'name', control: input, hint: line,
			help: opts.readOnly ? (opts.readOnlyHelp || t('The container name cannot be changed after creating.')) : t('The container’s name in Docker. It cannot be changed later.'),
		});
		f.validate = () => {
			if (!spec.name) {
				return t('Enter a container name');
			}
			if (!opts.readOnly && !NAME_RE.test(spec.name)) {
				return t('Use only letters, digits, - _ and ., starting with a letter or digit.');
			}
			if (!opts.readOnly && HB.instances.some((i) => i.spec.name.toLowerCase() === spec.name.toLowerCase())) {
				return t('There is already a Homebridge with this name.');
			}
			return null;
		};
		return f;
	}

	function imageField(spec, onChange, opts) {
		opts = opts || {};
		const input = HB.input({ value: spec.image || '', class: 'input mono', placeholder: opts.defaultImage || 'homebridge/homebridge:latest', autocapitalize: 'off' });
		input.addEventListener('input', () => {
			spec.image = input.value.trim();
			onChange('image');
		});
		const reset = opts.defaultImage ? HB.button(t('Use default'), { kind: 'ghost', size: 'sm', onClick: () => {
			input.value = opts.defaultImage;
			spec.image = opts.defaultImage;
			onChange('image');
		} }) : null;
		const legacy = /^oznu\/homebridge(:|$)/.test(spec.image || '');
		const f = HB.field({
			label: t('Image'), path: 'image', control: h('div', { class: 'input-group' }, [input, reset]),
			help: legacy ? t('This image is no longer maintained. After creating, you can switch to the new image in one step (click the parcel at the door).') : t('Usually left as it is. The form is name:tag, for example homebridge/homebridge:latest.'),
		});
		f.validate = () => {
			if (!spec.image) {
				return t('Enter an image name');
			}
			if (spec.image.includes('@')) {
				return t('Use name:tag; an @sha256 digest is not accepted');
			}
			return null;
		};
		return f;
	}

	// ---------- network ----------

	function subnetRange(cidr) {
		const m = /^(\d+\.\d+\.\d+\.\d+)\/(\d+)$/.exec(cidr || '');
		if (!m) {
			return null;
		}
		const toInt = (ip) => ip.split('.').reduce((a, b) => (a * 256) + Number(b), 0);
		const toIp = (n) => [24, 16, 8, 0].map((s) => Math.floor(n / Math.pow(2, s)) % 256).join('.');
		const bits = Number(m[2]);
		const size = Math.pow(2, 32 - bits);
		const base = Math.floor(toInt(m[1]) / size) * size;
		if (size < 4) {
			return null;
		}
		return [toIp(base + 1), toIp(base + size - 2)];
	}

	/*
	 * Interface, mode, address, MAC and host name. opts: { exclude, onIp(ip),
	 * hostnameFollows: bool (wizard), showHostname: bool }
	 * Returns { el, ifaceEl, ipCheck: () => Promise<status>, validate() }
	 */
	function networkFields(spec, onChange, opts) {
		opts = opts || {};
		const ifaces = (HB.networks && HB.networks.interfaces) || [];
		spec.network = spec.network || { mode: 'static', iface: '', ip: '' };
		const net = spec.network;
		if (!net.iface && ifaces.length) {
			net.iface = (ifaces.find((i) => i.default) || ifaces[0]).iface;
		}
		const iface = () => ifaces.find((i) => i.iface === net.iface) || null;

		// interface
		let ifaceControl;
		if (ifaces.length > 1) {
			ifaceControl = h('select', { class: 'select' });
			for (const i of ifaces) {
				ifaceControl.appendChild(h('option', { value: i.iface, selected: i.iface === net.iface, text: HB.t('{name} ({subnet})', { name: i.display, subnet: i.subnet }) }));
			}
			if (net.iface && !ifaces.some((i) => i.iface === net.iface)) {
				ifaceControl.appendChild(h('option', { value: net.iface, selected: true, text: t('{name} (interface not found)', { name: net.iface }) }));
			}
			ifaceControl.addEventListener('change', () => {
				net.iface = ifaceControl.value;
				paintRange();
				onChange('network.iface');
				runIpCheck();
			});
		} else {
			const i = iface();
			ifaceControl = h('p', { class: 'static-value', text: i ? t('{name} ({subnet})', { name: i.display, subnet: i.subnet }) : (net.iface || t('No network interface found')) });
		}
		const ifaceField = HB.field({ label: t('Network interface'), path: 'network.iface', control: ifaceControl });

		// mode
		const modes = HB.choiceCards({
			label: t('How it gets an address'),
			value: net.mode,
			options: [
				{ value: 'static', title: t('Fixed address'), text: t('You choose the address. The most stable choice, and the best for a domain name.'), icon: 'home' },
				{ value: 'dhcp', title: t('Automatic (DHCP)'), text: t('The router assigns it. The MAC stays the same, so the address usually survives a recreate.'), icon: 'refresh' },
			],
			onChange: (v) => {
				net.mode = v;
				if (v === 'dhcp') {
					lastStaticIp = net.ip || lastStaticIp;
					net.ip = '';
				} else {
					net.ip = lastStaticIp || '';
					ipInput.value = net.ip;
				}
				paintMode();
				onChange('network.mode');
				if (v === 'static') {
					runIpCheck();
				}
			},
		});
		const modeField = HB.field({ label: t('Address on the local network'), path: 'network.mode', control: modes });

		// address
		let lastStaticIp = net.ip || '';
		const ipInput = HB.input({ value: net.ip || '', class: 'input mono', inputmode: 'decimal', placeholder: t('For example {ip}', { ip: '192.168.1.20' }) });
		const ipLine = HB.checkLine();
		const range = h('p', { class: 'range-hint' });
		const ipField = HB.field({ label: t('IP address'), path: 'network.ip', control: ipInput, hint: ipLine });
		ipField.appendChild(range);
		const dhcpNote = h('p', { class: 'field-help dhcp-note', text: t('The address the router gives it appears once it starts. To open it by domain name, reserve an address for this MAC on the router.') });

		let seq = 0;
		let pending = null;
		let status = null; // { ip, iface, kind, message }
		const debounced = HB.debounce(() => runIpCheck(), 550);
		ipInput.addEventListener('input', () => {
			net.ip = ipInput.value.trim();
			lastStaticIp = net.ip;
			HB.clearFieldErrors(ipField.closest('form, .wiz-page, .group, .dlg-body') || ipField.parentNode, 'network.ip');
			status = null;
			onChange('network.ip');
			if (!net.ip) {
				ipLine.set(null);
				debounced.cancel();
				return;
			}
			if (!IPV4_RE.test(net.ip)) {
				ipLine.set(null);
				debounced();
				return;
			}
			ipLine.set('busy', t('Checking this address…'));
			debounced();
		});
		ipInput.addEventListener('blur', () => {
			if (net.ip && !IPV4_RE.test(net.ip)) {
				ipLine.set('error', t('Enter an IPv4 address, for example {ip}', { ip: '192.168.1.20' }));
			}
		});

		function runIpCheck() {
			debounced.cancel();
			if (net.mode !== 'static' || !net.ip) {
				pending = null;
				return Promise.resolve(null);
			}
			if (!IPV4_RE.test(net.ip)) {
				status = { ip: net.ip, iface: net.iface, kind: 'error', message: t('Enter an IPv4 address, for example {ip}', { ip: '192.168.1.20' }) };
				ipLine.set('error', status.message);
				return Promise.resolve(status);
			}
			if (status && status.ip === net.ip && status.iface === net.iface) {
				return Promise.resolve(status);
			}
			const my = ++seq;
			const ip = net.ip;
			const ifc = net.iface;
			ipLine.set('busy', t('Checking this address…'));
			pending = HB.api.post('/check/ip', { iface: ifc, ip, exclude: opts.exclude ? opts.exclude() : undefined }).then((r) => {
				let s;
				if (!r.valid) {
					s = { kind: 'error', message: HB.msg(r.message) || t('This address cannot be used') };
				} else if (r.available) {
					s = { kind: 'ok', message: t('This address is free') };
				} else if (r.reason === 'lan') {
					s = { kind: 'warn', message: t('{reason}. If that is the old container being replaced, you can go on.', { reason: HB.msg(r.message) || t('A device on the local network uses this address') }) };
				} else {
					s = { kind: 'error', message: HB.msg(r.message) || t('This address is already in use') };
				}
				s.ip = ip;
				s.iface = ifc;
				return s;
			}, (err) => {
				if (err.handled) {
					return null;
				}
				return { ip, iface: ifc, kind: 'info', message: t('Cannot check right now ({reason}). It is checked again when you save.', { reason: HB.errorText(err) }) };
			}).then((s) => {
				if (my !== seq) {
					return status;
				}
				pending = null;
				status = s;
				if (s) {
					ipLine.set(s.kind, s.message);
					if (opts.onIp) {
						opts.onIp(s);
					}
				}
				return s;
			});
			return pending;
		}

		function paintRange() {
			const i = iface();
			HB.clear(range);
			if (!i) {
				return;
			}
			const r = subnetRange(i.subnet);
			const b = (text) => h('b', { text: text || '—' });
			HB.append(range, r
				? HB.tn('Usable range {from} – {to}, router {gateway}, NAS {nas}', { from: b(r[0]), to: b(r[1]), gateway: b(i.gateway), nas: b(i.host_ip) })
				: HB.tn('Router {gateway}, NAS {nas}', { gateway: b(i.gateway), nas: b(i.host_ip) }));
		}

		function paintMode() {
			ipField.hidden = net.mode !== 'static';
			dhcpNote.hidden = net.mode !== 'dhcp';
		}

		// advanced: MAC, host name
		const macInput = HB.input({ value: net.mac || '', class: 'input mono', placeholder: t('Leave empty to generate one (02:42:…)'), autocapitalize: 'off' });
		macInput.addEventListener('input', () => {
			net.mac = macInput.value.trim().toLowerCase();
			onChange('network.mac');
		});
		const macField = HB.field({ label: t('MAC address'), path: 'network.mac', control: macInput, optional: true, help: t('It stays the same after creating, so the DHCP address and HomeKit pairing survive a recreate. A takeover keeps the original one.') });

		let hostTouched = !opts.hostnameFollows;
		const hostInput = HB.input({ value: opts.hostnameFollows ? '' : (spec.hostname || ''), class: 'input mono', autocapitalize: 'off' });
		if (opts.hostnameFollows && spec.hostname && spec.hostname !== effectiveHostDefault()) {
			hostInput.value = spec.hostname;
			hostTouched = true;
		}
		function effectiveHostDefault() {
			return (spec.domain || '').trim().toLowerCase() || (spec.name || '').toLowerCase();
		}
		function paintHostPlaceholder() {
			hostInput.placeholder = effectiveHostDefault() ? t('Default: {name}', { name: effectiveHostDefault() }) : t('Defaults to the domain or the container name');
		}
		hostInput.addEventListener('input', () => {
			hostTouched = hostInput.value.trim() !== '';
			spec.hostname = hostInput.value.trim().toLowerCase();
			onChange('hostname');
		});
		const hostField = HB.field({ label: t('Host name'), path: 'hostname', control: hostInput, optional: true, help: opts.hostnameFollows ? t('The container’s own host name. Left empty, it follows the domain, or the container name when there is no domain.') : t('The container’s own host name. A change takes effect after Apply and recreate.') });

		const more = h('details', { class: 'more' }, [
			h('summary', null, [h('span', { text: t('Advanced') }), h('span', { class: 'more-hint', text: t('MAC address, host name') }), icon('chevronDown', { size: 16 })]),
			h('div', { class: 'more-body' }, [macField, opts.showHostname === false ? null : hostField]),
		]);
		if (net.mac && opts.openAdvancedIfMac) {
			more.open = true;
		}

		paintRange();
		paintMode();
		paintHostPlaceholder();
		if (net.mode === 'static' && net.ip && opts.checkNow !== false) {
			runIpCheck();
		}

		const el = h('div', { class: 'form-stack' }, [ifaceField, modeField, ipField, dhcpNote, opts.slot || null, more]);
		return {
			el,
			ifaceField,
			ipStatus: () => status,
			ipCheck: () => (pending || runIpCheck()),
			hostnameSync() {
				paintHostPlaceholder();
				if (opts.hostnameFollows && !hostTouched) {
					spec.hostname = effectiveHostDefault();
				}
			},
			validate() {
				const errs = {};
				if (!net.iface) {
					errs['network.iface'] = t('Choose a network interface');
				}
				if (net.mode === 'static') {
					if (!net.ip) {
						errs['network.ip'] = t('Enter an IP address');
					} else if (!IPV4_RE.test(net.ip)) {
						errs['network.ip'] = t('Enter an IPv4 address, for example {ip}', { ip: '192.168.1.20' });
					} else if (status && status.ip === net.ip && status.kind === 'error') {
						errs['network.ip'] = status.message;
					}
				}
				if (net.mac && !/^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/.test(net.mac)) {
					errs['network.mac'] = t('A MAC address looks like 02:42:xx:xx:xx:xx');
					more.open = true;
				}
				if (spec.hostname && !validHostname(spec.hostname)) {
					errs.hostname = t('A host name uses only letters, digits and -, in parts separated by .');
					more.open = true;
				}
				return errs;
			},
		};
	}

	/*
	 * Domain with a live DNS check. opts: { ip: () => string|null, onChange }
	 * Never blocks saving; only a malformed name does.
	 */
	function domainField(spec, onChange, opts) {
		opts = opts || {};
		const input = HB.input({ value: spec.domain || '', class: 'input mono', placeholder: t('For example {domain}', { domain: '1f.example.com' }), autocapitalize: 'off', inputmode: 'url' });
		const line = HB.checkLine();
		let seq = 0;
		const debounced = HB.debounce(() => check(), 650);
		input.addEventListener('input', () => {
			spec.domain = input.value.trim().toLowerCase();
			onChange('domain');
			if (!spec.domain) {
				line.set(null);
				debounced.cancel();
				return;
			}
			line.set('busy', t('Looking it up in DNS…'));
			debounced();
		});

		async function check() {
			debounced.cancel();
			const d = (spec.domain || '').replace(/\.$/, '');
			if (!d) {
				line.set(null);
				return;
			}
			if (!validDomain(d)) {
				line.set('error', t('This is not a valid domain name. Example: {domain} (it needs at least one dot)', { domain: '1f.example.com' }));
				return;
			}
			const my = ++seq;
			const ip = opts.ip ? opts.ip() : null;
			line.set('busy', t('Looking it up in DNS…'));
			let r;
			try {
				r = await HB.api.post('/check/domain', { domain: d, ip: ip || undefined });
			} catch (err) {
				if (my === seq && !err.handled) {
					line.set('info', t('Cannot look it up in DNS right now ({reason})', { reason: HB.errorText(err) }));
				}
				return;
			}
			if (my !== seq) {
				return;
			}
			paintResult(line, r);
		}

		const f = HB.field({
			label: t('Domain'), path: 'domain', control: input, hint: line, optional: true,
			help: t('Opens the Homebridge web page under this name. Point it at this address in your DNS yourself.'),
		});
		f.recheck = () => {
			if (spec.domain) {
				check();
			}
		};
		f.validate = () => (spec.domain && !validDomain(spec.domain) ? t('This is not a valid domain name. Example: {domain} (it needs at least one dot)', { domain: '1f.example.com' }) : null);
		if (spec.domain && opts.checkNow !== false) {
			check();
		}
		return f;
	}

	/* The three calm states of a domain check. */
	function paintResult(line, r) {
		if (!r.valid) {
			line.set('error', HB.msg(r.message) || t('This is not a valid domain name'));
		} else if (r.matches === true) {
			line.set('ok', HB.msg(r.message) || t('Points at this Homebridge'));
		} else if (r.resolves && r.resolves.length && r.matches === false) {
			line.set('warn', HB.msg(r.message) || t('Points at another address: {addresses}', { addresses: HB.list(r.resolves) }));
		} else if (!r.resolves || !r.resolves.length) {
			line.set('info', HB.msg(r.message) || t('This domain name is not found'));
		} else {
			line.set('info', HB.msg(r.message) || t('Points at {addresses}', { addresses: HB.list(r.resolves) }));
		}
	}

	// ---------- folders ----------

	/* Work directory with a picker and a "new or existing" note. opts: { lockedNote } */
	function workdirField(spec, onChange, opts) {
		opts = opts || {};
		const input = HB.input({ value: spec.workdir || '', class: 'input mono', placeholder: '/share/Container/homebridge', autocapitalize: 'off' });
		const browse = HB.button(t('Browse…'), { icon: 'folder', size: 'sm', onClick: async () => {
			const start = spec.workdir ? spec.workdir : (opts.suggestParent ? opts.suggestParent() : '/share');
			const p = await HB.folders.pick({ title: t('Choose the work folder'), start });
			if (p) {
				input.value = p;
				spec.workdir = p;
				onChange('workdir', true);
				check();
			}
		} });
		const line = HB.checkLine();
		const debounced = HB.debounce(() => check(), 600);
		input.addEventListener('input', () => {
			spec.workdir = input.value.trim().replace(/\/+$/, '');
			onChange('workdir', true);
			debounced();
		});
		let seq = 0;
		async function check() {
			debounced.cancel();
			const p = spec.workdir;
			if (!p) {
				line.set(null);
				return;
			}
			if (!p.startsWith('/share/')) {
				line.set('error', t('Only folders under /share can be used'));
				return;
			}
			const owner = usedBy(p);
			if (owner) {
				line.set('error', t('“{name}” already uses this folder. Choose another one.', { name: HB.displayName(owner) }));
				return;
			}
			const my = ++seq;
			const parent = p.replace(/\/[^/]+$/, '') || '/share';
			try {
				// list the parent rather than asking for the folder itself, so a
				// folder that does not exist yet is an answer, not an error
				const list = await HB.api.get('/fs', { path: parent });
				if (my !== seq) {
					return;
				}
				if (!list.dirs.some((d) => d.path === p)) {
					line.set('info', t('This folder does not exist yet. It is created when you create the Homebridge.'));
					return;
				}
				if (my === seq) {
					if (opts.creating) {
						line.set('warn', t('This folder already exists, and the new Homebridge will use what is in it. Good for a move; otherwise choose another one.'));
					} else {
						line.set('ok', opts.existingText || t('Uses the existing folder. The settings and plugins in it stay as they are.'));
					}
				}
			} catch (err) {
				if (my !== seq || err.handled) {
					return;
				}
				if (err.code === 'not_found') {
					line.set('error', t('The folder {path} does not exist. Create it first or choose another place.', { path: parent }));
				} else {
					line.set('error', err.fields && err.fields.path ? HB.msg(err.fields.path) : HB.errorText(err));
				}
			}
		}
		const f = HB.field({
			label: t('Work folder'), path: 'workdir',
			control: h('div', { class: 'input-group' }, [input, browse]),
			hint: line,
			help: t('Mounted at /homebridge in the container. It holds the settings, plugins and HomeKit pairing, and is never deleted, not even when the Homebridge is removed.'),
		});
		f.setValue = (v) => {
			input.value = v;
			check();
		};
		f.check = check;
		function usedBy(p) {
			return HB.instances.find((i) => i.spec.workdir === p && i.spec.name !== opts.self) || null;
		}
		f.validate = () => {
			if (!spec.workdir) {
				return t('Choose a work folder');
			}
			const owner = usedBy(spec.workdir);
			return owner ? t('“{name}” already uses this folder. Choose another one.', { name: HB.displayName(owner) }) : null;
		};
		if (spec.workdir && opts.checkNow !== false) {
			check();
		}
		return f;
	}

	/*
	 * Extra mounts: host folder, path inside the container, read-only.
	 * list is edited in place. opts: { pathPrefix: 'mounts', onChange, addLabel }
	 * -> { el, get(), validate() }
	 */
	function mountsEditor(list, opts) {
		opts = opts || {};
		const prefix = opts.pathPrefix || 'mounts';
		const rows = h('div', { class: 'rows' });
		const add = HB.button(opts.addLabel || t('Add a folder'), { icon: 'plus', size: 'sm', kind: 'ghost', className: 'add-row', onClick: () => {
			list.push({ host: '', container: '', ro: false });
			paint();
			changed();
			const inputs = rows.querySelectorAll('.row-card:last-child input');
			if (inputs[0]) {
				inputs[0].focus();
			}
		} });
		const el = h('div', { class: 'rows-wrap form-stack', style: { gap: '10px' } }, [rows, add]);

		function changed() {
			if (opts.onChange) {
				opts.onChange();
			}
		}

		function paint() {
			HB.clear(rows);
			if (!list.length) {
				rows.appendChild(h('p', { class: 'rows-empty', text: opts.emptyText || t('No other folders. Add one when Homebridge needs to read another folder on the NAS, such as certificates.') }));
				return;
			}
			list.forEach((m, i) => {
				const host = HB.input({ value: m.host || '', class: 'input mono', placeholder: '/share/…', 'aria-label': t('NAS folder of mount {n}', { n: i + 1 }), autocapitalize: 'off' });
				host.addEventListener('input', () => {
					m.host = host.value.trim().replace(/\/+$/, '');
					changed();
				});
				const browse = h('button', { type: 'button', class: 'icon-btn icon-btn-sm', 'aria-label': t('Browse NAS folders'), title: t('Browse…') }, icon('folder'));
				browse.addEventListener('click', async () => {
					const p = await HB.folders.pick({ title: t('Choose the folder to mount'), start: m.host || '/share' });
					if (p) {
						m.host = p;
						host.value = p;
						if (!m.container) {
							m.container = '/' + p.split('/').filter(Boolean).pop();
							cont.value = m.container;
						}
						changed();
					}
				});
				const cont = HB.input({ value: m.container || '', class: 'input mono', placeholder: '/data', 'aria-label': t('Path in the container of mount {n}', { n: i + 1 }), autocapitalize: 'off' });
				cont.addEventListener('input', () => {
					m.container = cont.value.trim();
					changed();
				});
				const ro = HB.switchControl({ checked: !!m.ro, label: t('Read-only'), onChange: (v) => {
					m.ro = v;
					changed();
				} });
				const remove = h('button', { type: 'button', class: 'icon-btn icon-btn-sm row-remove', 'aria-label': t('Remove this folder'), title: t('Remove') }, icon('trash'));
				remove.addEventListener('click', () => {
					list.splice(i, 1);
					paint();
					changed();
					add.focus();
				});
				rows.appendChild(h('div', { class: 'row-card' }, [
					HB.field({ label: t('Folder on the NAS'), path: prefix + '.' + i + '.host', control: h('div', { class: 'input-group' }, [host, browse]) }),
					HB.field({ label: t('Path in the container'), path: prefix + '.' + i + '.container', control: cont }),
					ro,
					remove,
				]));
			});
		}
		paint();
		return {
			el,
			get: () => list.map((m) => ({ host: m.host, container: m.container, ro: !!m.ro })),
			repaint: paint,
			validate() {
				const errs = {};
				const seen = new Set();
				list.forEach((m, i) => {
					if (!m.host) {
						errs[prefix + '.' + i + '.host'] = t('Choose a folder on the NAS');
					}
					if (!m.container) {
						errs[prefix + '.' + i + '.container'] = t('Enter the path in the container, for example {path}', { path: '/data' });
					} else if (!m.container.startsWith('/')) {
						errs[prefix + '.' + i + '.container'] = t('The path in the container starts with /');
					} else if (m.container === '/homebridge' || m.container.startsWith('/homebridge/')) {
						errs[prefix + '.' + i + '.container'] = t('/homebridge is where the work folder goes. Choose another path');
					} else if (seen.has(m.container)) {
						errs[prefix + '.' + i + '.container'] = t('Another folder already uses this path');
					}
					seen.add(m.container);
				});
				return errs;
			},
		};
	}

	/* Environment variables as key/value rows. env object is replaced on change. */
	function envEditor(spec, onChange) {
		const pairs = Object.keys(spec.env || {}).map((k) => ({ key: k, value: spec.env[k] }));
		const rows = h('div', { class: 'rows' });
		function sync() {
			const env = {};
			for (const p of pairs) {
				if (p.key) {
					env[p.key] = p.value;
				}
			}
			spec.env = env;
			onChange('env');
		}
		const add = HB.button(t('Add a variable'), { icon: 'plus', size: 'sm', kind: 'ghost', className: 'add-row', onClick: () => {
			pairs.push({ key: '', value: '' });
			paint();
			const inputs = rows.querySelectorAll('.row-card:last-child input');
			if (inputs[0]) {
				inputs[0].focus();
			}
		} });
		function paint() {
			HB.clear(rows);
			if (!pairs.length) {
				rows.appendChild(h('p', { class: 'rows-empty', text: t('No variables of your own.') }));
				return;
			}
			pairs.forEach((p, i) => {
				const k = HB.input({ value: p.key, class: 'input mono', placeholder: 'NAME', 'aria-label': t('Variable name'), autocapitalize: 'characters' });
				const v = HB.input({ value: p.value, class: 'input mono', placeholder: t('Value'), 'aria-label': t('Variable value') });
				k.addEventListener('input', () => {
					p.key = k.value.trim();
					sync();
				});
				v.addEventListener('input', () => {
					p.value = v.value;
					sync();
				});
				const remove = h('button', { type: 'button', class: 'icon-btn icon-btn-sm row-remove', 'aria-label': t('Remove this variable'), title: t('Remove') }, icon('trash'));
				remove.addEventListener('click', () => {
					pairs.splice(i, 1);
					paint();
					sync();
					add.focus();
				});
				rows.appendChild(h('div', { class: 'row-card env-row', 'data-path': 'env.' + p.key }, [
					HB.field({ label: t('Name'), control: k }),
					HB.field({ label: t('Value'), control: v }),
					remove,
					h('p', { class: 'field-error', hidden: true, style: { gridColumn: '1 / -1' } }),
				]));
			});
		}
		paint();
		const f = HB.field({ label: t('Environment variables'), path: 'env', control: h('div', { class: 'form-stack', style: { gap: '10px' } }, [rows, add]), optional: true, help: t('TZ (the time zone) defaults to the NAS’s time zone.') });
		f.validate = () => {
			const errs = {};
			const seen = new Set();
			for (const p of pairs) {
				if (!p.key && !p.value) {
					continue;
				}
				if (!ENV_RE.test(p.key)) {
					errs['env.' + p.key] = t('A variable name uses only letters, digits and _, and does not start with a digit');
				} else if (seen.has(p.key)) {
					errs['env.' + p.key] = t('This name is used twice');
				} else if (/[\n\r\0]/.test(p.value)) {
					errs['env.' + p.key] = t('A value cannot contain a line break');
				}
				seen.add(p.key);
			}
			return errs;
		};
		return f;
	}

	function restartField(spec, onChange) {
		const sel = h('select', { class: 'select' }, [
			h('option', { value: 'always', text: t('Always start it (recommended)') }),
			h('option', { value: 'unless-stopped', text: t('Start it unless stopped by hand') }),
			h('option', { value: 'no', text: t('Do not start it') }),
		]);
		sel.value = spec.restart || 'always';
		sel.addEventListener('change', () => {
			spec.restart = sel.value;
			onChange('restart');
		});
		return HB.field({ label: t('When the NAS starts or the container crashes'), path: 'restart', control: sel });
	}

	/* Folders step: work directory, mounts, and env + restart under Advanced. */
	function folderFields(spec, onChange, opts) {
		opts = opts || {};
		spec.mounts = spec.mounts || [];
		const wd = workdirField(spec, onChange, opts);
		const mounts = mountsEditor(spec.mounts, { onChange: () => onChange('mounts') });
		const mountsField = h('div', { class: 'field', 'data-path': 'mounts' }, [
			h('span', { class: 'field-label' }, [h('span', { text: t('Other folders') }), h('span', { class: 'field-optional', text: t('Optional') })]),
			h('p', { class: 'field-help', text: t('Lets Homebridge reach other folders on the NAS, such as certificates or recordings. It can write to one only when Read-only is off.') }),
			mounts.el,
			h('p', { class: 'field-error', hidden: true }),
		]);
		const env = envEditor(spec, onChange);
		const restart = restartField(spec, onChange);
		const more = h('details', { class: 'more' }, [
			h('summary', null, [h('span', { text: t('Advanced') }), h('span', { class: 'more-hint', text: t('Environment variables, automatic start') }), icon('chevronDown', { size: 16 })]),
			h('div', { class: 'more-body' }, [env, restart]),
		]);
		return {
			el: h('div', { class: 'form-stack' }, [wd, mountsField, more]),
			workdir: wd,
			mounts,
			validate() {
				const errs = Object.assign({}, mounts.validate(), env.validate());
				const w = wd.validate();
				if (w) {
					errs.workdir = w;
				}
				if (Object.keys(errs).some((k) => k.startsWith('env.'))) {
					more.open = true;
				}
				return errs;
			},
			openAdvanced() {
				more.open = true;
			},
		};
	}

	HB.specform = {
		displayNameField, emojiField, colorField, nameField, imageField,
		networkFields, domainField, paintDomainResult: paintResult,
		workdirField, mountsEditor, envEditor, restartField, folderFields,
		validDomain, subnetRange,
	};
})();
