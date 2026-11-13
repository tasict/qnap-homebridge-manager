/* Homebridge UI - the house. One drawing per Homebridge; its parts encode
 * state so the street can be read at a glance:
 *   roof colour = spec.color      sign in the gable = spec.emoji
 *   windows lit = running         dark = stopped     red, blinking = trouble
 *   door plate  = address         parcel on the step = an update is waiting
 *   scaffolding = a job is rebuilding it (percentage on the plate)
 *   paint mark  = saved settings not applied yet
 *   dashed      = the container is missing; grey = not managed yet
 * Drawn on a 200 x 170 grid; the plate and parcel are HTML on top so they
 * stay real text and a real button. */
(function () {
	'use strict';

	const { h, s } = HB;

	// One shared blur filter for the window glow, referenced by every house.
	function ensureDefs() {
		if (document.getElementById('hb-defs')) {
			return;
		}
		const defs = s('svg', { id: 'hb-defs', width: 0, height: 0, 'aria-hidden': 'true', focusable: 'false', class: 'svg-defs' }, [
			s('defs', null, [
				s('filter', { id: 'hb-glow', x: '-60%', y: '-60%', width: '220%', height: '220%' }, [
					s('feGaussianBlur', { stdDeviation: '7' }),
				]),
			]),
		]);
		document.body.appendChild(defs);
	}

	function drawing(opts) {
		const win = (x, n) => s('g', { class: 'h-window h-window-' + n }, [
			s('rect', { class: 'h-glow', x: x - 6, y: 92, width: 52, height: 40, rx: 10 }),
			s('rect', { class: 'h-win', x: x, y: 98, width: 40, height: 28, rx: 4 }),
			s('path', { class: 'h-mullion', d: 'M' + (x + 20) + ' 98v28M' + x + ' 112h40' }),
		]);
		return s('svg', { class: 'house-svg', viewBox: '0 0 200 170', 'aria-hidden': 'true', focusable: 'false' }, [
			// walls first, the roof overlaps their top edge
			s('rect', { class: 'h-wall', x: 28, y: 76, width: 144, height: 93, rx: 2 }),
			s('rect', { class: 'h-eave', x: 29, y: 83, width: 142, height: 6 }),
			win(44, 1),
			win(116, 2),
			s('path', { class: 'h-door', d: 'M51 169.5V139a4 4 0 0 1 4-4h22a4 4 0 0 1 4 4v30.5z' }),
			s('circle', { class: 'h-knob', cx: 75.5, cy: 154, r: 1.9 }),
			s('rect', { class: 'h-step', x: 45, y: 166.5, width: 42, height: 3.5, rx: 1.5 }),
			s('path', { class: 'h-roof', d: 'M19 79.5L93.6 21.6a10.4 10.4 0 0 1 12.8 0L181 79.5a2.6 2.6 0 0 1-1.6 4.7H20.6a2.6 2.6 0 0 1-1.6-4.7z' }),
			s('circle', { class: 'h-sign', cx: 100, cy: 57, r: opts.small ? 21 : 17 }),
			s('text', { class: 'h-emoji', x: 100, y: opts.small ? 58.5 : 58.5, 'text-anchor': 'middle', 'dominant-baseline': 'central', 'font-size': opts.small ? 27 : 20 }),
			// paint mark: settings saved but not applied
			s('g', { class: 'h-paint' }, [
				s('circle', { cx: 160, cy: 72, r: 11 }),
				s('path', { d: 'M154.5 77.5h3.2l7.6-7.6a2.2 2.2 0 0 0-3.2-3.2l-7.6 7.6z' }),
			]),
			// parcel on the doorstep: an update is waiting
			s('g', { class: 'h-parcel' }, [
				s('rect', { class: 'h-box', x: 22, y: 152, width: 25, height: 17.5, rx: 2 }),
				s('path', { class: 'h-tape', d: 'M34.5 152v17.5M22 157.5h25' }),
				s('g', { class: 'h-legacy' }, [
					s('circle', { cx: 47, cy: 152, r: 6.5 }),
					s('path', { d: 'M47 148.6v3.8M47 155.1v.2' }),
				]),
			]),
			// scaffolding: a job is working on this house
			s('g', { class: 'h-scaffold' }, [
				s('path', { class: 'h-pole', d: 'M16 169.5V40M70 169.5V24M130 169.5V24M184 169.5V40' }),
				s('path', { class: 'h-plank', d: 'M10 88h180M10 131h180M58 46h84' }),
				s('path', { class: 'h-brace', d: 'M16 131L70 88M130 131L184 88M70 169.5L130 131' }),
			]),
		]);
	}

	/*
	 * HB.house.create({ size: 'lg' | 'md' | 'sm' }) -> { el, update(model) }
	 * model: { kind: managed|unmanaged|missing|draft, color, emoji, lights: on|off|trouble,
	 *          plate: { text, dhcp, muted } | null, parcel: null|update|legacy, onParcel,
	 *          parcelLabel, pending, scaffold, percent }
	 */
	function create(opts) {
		opts = opts || {};
		ensureDefs();
		const size = opts.size || 'md';
		const svg = drawing({ small: size === 'sm' });
		const emoji = svg.querySelector('.h-emoji');
		const plateText = h('span', { class: 'plate-text' });
		const plateTag = h('span', { class: 'plate-tag', text: 'DHCP', hidden: true });
		const plate = h('span', { class: 'plate' }, [plateTag, plateText]);
		const parcelBtn = h('button', { type: 'button', class: 'parcel-hit', hidden: true });
		const el = h('div', { class: ['house', 'house-' + size] }, [svg, size === 'sm' ? null : plate, size === 'sm' ? null : parcelBtn]);
		let onParcel = null;
		parcelBtn.addEventListener('click', (e) => {
			e.stopPropagation();
			if (onParcel) {
				onParcel();
			}
		});

		function update(m) {
			el.dataset.kind = m.kind || 'managed';
			el.dataset.color = m.color || 'violet';
			el.dataset.lights = m.lights || 'off';
			el.classList.toggle('has-parcel', !!m.parcel);
			el.classList.toggle('is-legacy', m.parcel === 'legacy');
			el.classList.toggle('has-paint', !!m.pending);
			el.classList.toggle('has-scaffold', !!m.scaffold);
			const e = m.emoji || '🏠';
			if (emoji.textContent !== e) {
				emoji.textContent = e;
			}
			if (size !== 'sm') {
				if (m.scaffold) {
					plate.classList.add('is-progress');
					plate.style.setProperty('--p', (m.percent === null || m.percent === undefined ? 0 : m.percent) + '%');
					plateTag.hidden = true;
					plateText.textContent = m.percent === null || m.percent === undefined ? '…' : m.percent + '%';
					plate.hidden = false;
				} else {
					plate.classList.remove('is-progress');
					const p = m.plate;
					plate.hidden = !p;
					if (p) {
						plateText.textContent = p.text || '—';
						plateTag.hidden = !p.dhcp;
						plate.classList.toggle('is-muted', !!p.muted);
						const len = (p.text || '').length + (p.dhcp ? 5 : 0);
						plate.dataset.len = len > 15 ? 'xl' : len > 12 ? 'l' : 'm';
					}
				}
				parcelBtn.hidden = !m.parcel || !m.onParcel;
				onParcel = m.onParcel || null;
				if (m.parcelLabel) {
					parcelBtn.setAttribute('aria-label', m.parcelLabel);
					parcelBtn.title = m.parcelLabel;
				}
			}
		}

		return { el, update, plate };
	}

	/* Build the house model for an instance (and the job it is running, if any). */
	function modelFor(inst, job, onParcel) {
		const st = inst.state || {};
		let lights = 'off';
		if (st.status === 'running') {
			lights = 'on';
		} else if (st.status === 'restarting' || st.status === 'dead') {
			lights = 'trouble';
		}
		const ip = st.ip || (inst.spec.network.mode === 'static' ? inst.spec.network.ip : null);
		const up = inst.update || {};
		let parcel = null;
		if (up.legacy_image) {
			parcel = 'legacy';
		} else if (up.available) {
			parcel = 'update';
		}
		const percent = job ? HB.jobs.overallPercent(job) : null;
		return {
			kind: st.status === 'missing' ? 'missing' : 'managed',
			color: inst.spec.color,
			emoji: inst.spec.emoji,
			lights,
			plate: { text: ip || '—', dhcp: inst.spec.network.mode === 'dhcp', muted: st.status !== 'running' },
			parcel: inst.job ? null : parcel,
			parcelLabel: parcel === 'legacy' ? HB.t('This image is no longer maintained. Switch to the new image and upgrade') : HB.t('An update is available. Upgrade'),
			onParcel: onParcel,
			pending: !!inst.pending_changes && st.status !== 'missing',
			scaffold: !!inst.job,
			percent,
		};
	}

	HB.house = { create, modelFor };
})();
