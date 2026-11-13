/* Homebridge UI - background jobs: one poller per job shared by every view
 * that shows it, the job panel (steps, pull progress, log, outcome) and the
 * dialog that hosts it. Closing a view never stops the job. */
(function () {
	'use strict';

	const { h, icon } = HB;
	const entries = new Map();
	const shown = new Map(); // job id -> number of panels showing it

	function entry(id) {
		let e = entries.get(id);
		if (!e) {
			e = { id, job: null, log: '', offset: 0, listeners: new Set(), timer: null, inflight: false, finished: false, error: null };
			entries.set(id, e);
		}
		return e;
	}

	/* One request at a time per job, and a chunk is appended only if it starts where the log ends. */
	async function poll(e) {
		e.timer = null;
		if (e.inflight) {
			return;
		}
		e.inflight = true;
		const from = e.offset;
		try {
			const job = await HB.api.get('/jobs/' + HB.enc(e.id), { offset: from });
			if (from === e.offset) {
				if (typeof job.log === 'string' && job.log) {
					e.log += job.log;
				}
				if (typeof job.next_offset === 'number') {
					e.offset = job.next_offset;
				}
			}
			e.job = job;
			e.error = null;
			const wasFinished = e.finished;
			e.finished = job.status !== 'running';
			for (const fn of e.listeners) {
				fn(e);
			}
			if (e.finished && !wasFinished) {
				e.inflight = false;
				document.dispatchEvent(new CustomEvent('hb:job-finished', { detail: job }));
				return;
			}
		} catch (err) {
			if (err.handled) {
				e.inflight = false;
				return;
			}
			e.error = err;
			for (const fn of e.listeners) {
				fn(e);
			}
		}
		e.inflight = false;
		if (!e.finished && e.listeners.size && !e.timer) {
			e.timer = setTimeout(() => poll(e), e.error ? 3000 : (document.hidden ? 4000 : 1000));
		}
	}

	/* Subscribe to a job; returns an unsubscribe function. */
	function watch(id, fn) {
		const e = entry(id);
		e.listeners.add(fn);
		if (e.job) {
			fn(e);
		}
		if (!e.finished && !e.timer && !e.inflight) {
			e.timer = setTimeout(() => poll(e), e.job ? 1000 : 0);
		}
		return () => {
			e.listeners.delete(fn);
			if (!e.listeners.size && e.timer) {
				clearTimeout(e.timer);
				e.timer = null;
			}
		};
	}

	function stopAll() {
		for (const e of entries.values()) {
			if (e.timer) {
				clearTimeout(e.timer);
				e.timer = null;
			}
		}
	}

	/* Whole-job percentage from the steps: finished steps plus the running one.
	 * Downloading the image is usually most of the wait, so it weighs more. */
	const STEP_WEIGHT = { pull: 4, verify: 2 };

	function overallPercent(job) {
		if (!job || !job.steps || !job.steps.length) {
			return null;
		}
		const steps = job.steps.filter((st) => st.status !== 'skipped');
		if (!steps.length) {
			return null;
		}
		let total = 0;
		let done = 0;
		for (const st of steps) {
			const w = STEP_WEIGHT[st.key] || 1;
			total += w;
			if (st.status === 'done' || st.status === 'failed') {
				done += w;
			} else if (st.status === 'running') {
				const p = job.progress && typeof job.progress.percent === 'number' ? job.progress.percent : 30;
				done += w * p / 100;
			}
		}
		return Math.max(1, Math.min(99, Math.round(done / total * 100)));
	}

	/* Are the windows lit while a job rebuilds the house? */
	function lightsDuring(job, inst) {
		const running = inst && inst.state && inst.state.status === 'running';
		if (!job) {
			return running ? 'on' : 'off';
		}
		const by = {};
		for (const st of job.steps || []) {
			by[st.key] = st.status;
		}
		const stopped = (by.stop === 'done' || by.stop === 'running' || by.restore === 'running' || by.restore === 'done');
		const started = by.start === 'done';
		if (job.kind === 'create' && by.stop !== 'done' && by.stop !== 'running') {
			return started ? 'on' : (running ? 'on' : 'off');
		}
		if (started) {
			return 'on';
		}
		if (stopped) {
			return 'off';
		}
		return running ? 'on' : 'off';
	}

	// ---------- the panel ----------

	const STEP_ICON = { done: 'check', failed: 'x', skipped: null, running: null, pending: null };

	/*
	 * HB.jobs.panel(jobId, { instance: name, draft: { spec } , onFinish(job) })
	 * -> { el, destroy(), house }
	 */
	function panel(id, opts) {
		opts = opts || {};
		const t = HB.t;
		const house = HB.house.create({ size: 'lg' });
		const steps = h('ol', { class: 'steps', 'aria-label': t('Progress') });
		const bar = h('div', { class: 'bar', role: 'progressbar', 'aria-valuemin': '0', 'aria-valuemax': '100', 'aria-label': t('Progress of the current step') }, h('span', { class: 'bar-fill' }));
		const detail = h('p', { class: 'job-detail', 'aria-live': 'polite' });
		const progress = h('div', { class: 'job-progress' }, [bar, detail]);
		const outcome = h('div', { class: 'job-outcome', hidden: true });
		const logPre = h('pre', { class: 'log', tabindex: '0', 'aria-label': t('Job log'), 'data-empty': t('Nothing logged yet.') });
		const copyBtn = HB.button(t('Copy'), { icon: 'copy', size: 'sm', onClick: async () => {
			const ok = await HB.copyText(logPre.textContent);
			HB.toast(ok ? t('Log copied') : t('Could not copy. Select the text and copy it yourself.'), ok ? 'ok' : 'warn');
		} });
		const logBox = h('details', { class: 'job-log' }, [h('summary', null, [icon('logs'), h('span', { text: t('Show log') })]), h('div', { class: 'log-tools' }, copyBtn), logPre]);
		const status = h('p', { class: 'job-status', 'aria-live': 'polite' });
		const side = h('div', { class: 'job-side' }, [house.el, h('p', { class: 'job-house-name' })]);
		const main = h('div', { class: 'job-main' }, [status, steps, progress, outcome, logBox]);
		const el = h('div', { class: 'jobpanel' }, [side, main]);
		let finishedSeen = false;
		let readyTimer = null;
		let logShown = -1;
		shown.set(id, (shown.get(id) || 0) + 1);

		function instance() {
			return opts.instance ? HB.findInstance(opts.instance) : null;
		}

		function renderHouse(job) {
			const inst = instance();
			const spec = inst ? inst.spec : (opts.draft ? opts.draft.spec : null);
			if (!spec) {
				return;
			}
			side.querySelector('.job-house-name').textContent = spec.display_name || spec.name;
			const fakeInst = inst || { spec, state: { status: 'missing', ip: null }, update: {}, pending_changes: false, job: id };
			const model = HB.house.modelFor(Object.assign({}, fakeInst, { job: job && job.status === 'running' ? id : null }), job);
			model.kind = 'managed';
			model.parcel = null;
			model.pending = false;
			model.lights = job && job.status === 'running' ? lightsDuring(job, inst) : (inst && inst.state.status === 'running' ? 'on' : (job && job.status === 'succeeded' && job.kind !== 'remove' ? 'on' : 'off'));
			if (job && job.status !== 'running' && ((job.kind === 'remove' && job.status === 'succeeded') || (job.kind === 'create' && job.status !== 'succeeded'))) {
				model.kind = 'missing';
				model.lights = 'off';
			}
			house.update(model);
		}

		const STEP_WORD = { done: HB.N_('done'), failed: HB.N_('failed'), skipped: HB.N_('skipped'), running: HB.N_('in progress'), pending: HB.N_('waiting') };

		function renderSteps(job) {
			HB.clear(steps);
			for (const st of job.steps || []) {
				const mark = st.status === 'running'
					? h('span', { class: 'step-mark' }, h('span', { class: 'spinner' }))
					: h('span', { class: 'step-mark' }, STEP_ICON[st.status] ? icon(STEP_ICON[st.status], { size: 14 }) : null);
				steps.appendChild(h('li', { class: ['step', 'is-' + st.status] }, [
					mark,
					h('span', { class: 'step-label', text: HB.msg(st.label) }),
					h('span', { class: 'visually-hidden', text: t('({state})', { state: t(STEP_WORD[st.status] || HB.N_('waiting')) }) }),
					st.status === 'skipped' ? h('span', { class: 'step-note', text: t('Not needed') }) : null,
				]));
			}
		}

		function renderProgress(job) {
			const running = job.status === 'running';
			progress.hidden = !running;
			if (!running) {
				return;
			}
			const p = job.progress || {};
			const fill = bar.firstChild;
			if (typeof p.percent === 'number') {
				bar.classList.remove('is-indeterminate');
				fill.style.width = p.percent + '%';
				bar.setAttribute('aria-valuenow', String(p.percent));
			} else {
				bar.classList.add('is-indeterminate');
				fill.style.width = '';
				bar.removeAttribute('aria-valuenow');
			}
			const cur = (job.steps || []).find((st) => st.status === 'running');
			detail.textContent = p.detail ? HB.msg(p.detail) : (cur ? t('{step}…', { step: HB.msg(cur.label) }) : '');
		}

		/* Repaint only when new text arrived, and never under a selection. */
		function renderLog(e, force) {
			if (!force && e.log.length === logShown) {
				return;
			}
			const sel = window.getSelection();
			if (!force && sel && !sel.isCollapsed && logPre.contains(sel.anchorNode)) {
				return;
			}
			const atBottom = logPre.scrollTop + logPre.clientHeight >= logPre.scrollHeight - 24;
			logShown = e.log.length;
			const started = e.job ? e.job.started_at : null;
			const frag = document.createDocumentFragment();
			for (const line of HB.logLines(e.log, started)) {
				const row = h('span', { class: line.raw ? 'l-raw' : null });
				if (line.time !== null) {
					row.appendChild(h('span', { class: 'l-time', text: HB.clockTime(line.time) + '  ' }));
				}
				row.appendChild(document.createTextNode(line.text + '\n'));
				frag.appendChild(row);
			}
			HB.clear(logPre);
			logPre.appendChild(frag);
			if (atBottom) {
				logPre.scrollTop = logPre.scrollHeight;
			}
		}

		function outcomeFor(job) {
			const inst = instance();
			const name = inst ? HB.displayName(inst) : (opts.draft ? (opts.draft.spec.display_name || opts.draft.spec.name) : '');
			const reason = job.error ? HB.msg(job.error.msg || job.error.message) : '';
			if (job.status === 'succeeded') {
				const changed = !(job.result && job.result.changed === false);
				const map = {
					create: opts.takeover
						? [t('Takeover complete'), t('“{name}” is now managed here. HomeKit pairing carries over, so there is nothing to add again; the original container is kept as the backup.', { name })]
						: [t('“{name}” is up and running', { name }), null],
					apply: [t('The new settings are applied'), t('The container was recreated with the new settings. The old one is kept as the backup.')],
					upgrade: changed
						? [t('Upgrade complete'), t('“{name}” now runs the new image. The previous one is kept as the backup; if something is wrong, restore it under Backup.', { name })]
						: [t('Already up to date'), t('There is no newer image, so the container was left as it was.')],
					rollback: [t('Restored from the backup'), t('“{name}” is back to the container and settings it had when the backup was made.', { name })],
					remove: [t('Container removed'), t('The work folder and all other folders are still on the NAS.')],
					check_updates: [t('Check complete'), null],
				};
				const o = map[job.kind] || [t('Done'), null];
				return { kind: 'ok', title: o[0], text: o[1] };
			}
			if (job.status === 'rolled_back') {
				return { kind: 'warn', title: t('It did not work, so everything was put back'), text: t('The original container is running again and Homebridge works as before.'), reason };
			}
			const titles = {
				create: HB.N_('Creating did not finish'), apply: HB.N_('Recreating did not finish'), upgrade: HB.N_('Upgrading did not finish'),
				rollback: HB.N_('Restoring did not finish'), remove: HB.N_('Removing did not finish'), check_updates: HB.N_('Checking for updates did not finish'),
			};
			return {
				kind: 'error',
				title: t(titles[job.kind] || HB.N_('The job did not finish')),
				text: job.kind === 'create' ? t('No container was created. Fix the cause below and try again.') : null,
				reason,
			};
		}

		function renderOutcome(job) {
			if (job.status === 'running') {
				outcome.hidden = true;
				status.textContent = t('You can close this window; the work goes on in the background.');
				status.hidden = false;
				return;
			}
			status.hidden = true;
			const o = outcomeFor(job);
			HB.clear(outcome);
			outcome.hidden = false;
			outcome.className = 'job-outcome is-' + o.kind;
			HB.append(outcome, [
				h('div', { class: 'outcome-head' }, [icon(o.kind === 'ok' ? 'check' : 'alert', { size: 20 }), h('h3', { text: o.title })]),
				o.text ? h('p', { text: o.text }) : null,
				o.reason ? h('p', { class: 'outcome-reason' }, HB.tn('Reason: {reason}', { reason: h('span', { text: o.reason }) })) : null,
			]);
			if (job.status !== 'succeeded') {
				logBox.open = true;
			}
			if (job.kind === 'create' && job.status === 'succeeded') {
				const ready = h('p', { class: 'outcome-ready', hidden: !!opts.takeover });
				outcome.appendChild(ready);
				waitForReady(ready);
			}
		}

		/* After a create: show the pairing code and wait until the web page has an address. */
		function waitForReady(line) {
			let tries = 0;
			const tick = async () => {
				readyTimer = null;
				let inst = instance();
				try {
					inst = await HB.api.get('/instances/' + HB.enc(opts.instance));
				} catch (e) {
					/* keep what we have */
				}
				HB.clear(line);
				const pin = inst && inst.homebridge && inst.homebridge.pin;
				if (pin) {
					HB.append(line, HB.tn('In the Home app on your iPhone, add an accessory and enter the setup code {pin}.', { pin: h('strong', { class: 'pin-inline', text: pin }) }));
				} else {
					line.textContent = t('Open the Homebridge web page to see the setup code for the Home app.');
				}
				const url = inst && inst.homebridge && inst.homebridge.ui_url;
				if (opts.onReady) {
					opts.onReady(url, inst);
				}
				if (!url && tries++ < 12 && document.contains(el)) {
					readyTimer = setTimeout(tick, 2500);
				}
			};
			tick();
		}

		const stop = watch(id, (e) => {
			if (e.error && !e.job) {
				status.textContent = t('Cannot read the progress right now ({reason}). Trying again shortly.', { reason: HB.errorText(e.error) });
				return;
			}
			const job = e.job;
			if (!job) {
				return;
			}
			renderSteps(job);
			renderProgress(job);
			renderLog(e);
			renderHouse(job);
			if (job.status !== 'running' && !finishedSeen) {
				finishedSeen = true;
				el.classList.add('is-finished');
				if (job.status === 'succeeded') {
					house.el.classList.add('is-celebrating');
				}
				renderOutcome(job);
				// the instance list refreshes on hb:job-finished; redraw the house once it has
				setTimeout(() => renderHouse(job), 900);
				if (opts.onFinish) {
					opts.onFinish(job);
				}
			} else if (job.status === 'running') {
				renderOutcome(job);
			}
		});

		return {
			el,
			house,
			destroy() {
				stop();
				clearTimeout(readyTimer);
				const n = (shown.get(id) || 1) - 1;
				if (n > 0) {
					shown.set(id, n);
				} else {
					shown.delete(id);
				}
			},
		};
	}

	/*
	 * Open a dialog that follows a job. opts: { instance, draft, title }
	 */
	function openDialog(id, opts) {
		opts = opts || {};
		const t = HB.t;
		const inst = opts.instance ? HB.findInstance(opts.instance) : null;
		let p = null;
		const ctl = HB.dialog({
			title: opts.title || t('Progress'),
			size: 'lg',
			className: 'dlg-job',
			body: [],
			onClose: () => p && p.destroy(),
			actions: ['spacer', { label: t('Continue in the background'), kind: 'quiet' }],
		});
		p = panel(id, {
			instance: opts.instance,
			draft: opts.draft,
			onFinish: (job) => {
				const acts = ['spacer'];
				if (job.kind === 'create' && job.status === 'succeeded') {
					acts.push(openButton(null));
				} else if (inst && job.status === 'succeeded' && job.kind !== 'remove' && job.kind !== 'check_updates') {
					const fresh = HB.findInstance(inst.spec.name);
					const url = fresh && fresh.homebridge && fresh.homebridge.ui_url;
					if (url) {
						acts.push({ label: t('Open Homebridge'), href: url, icon: 'open', kind: 'quiet' });
					}
				}
				acts.push({ label: t('Done'), kind: 'primary', autofocus: true });
				ctl.setActions(acts);
				const done = ctl.foot.querySelector('.btn-primary');
				if (done) {
					done.focus();
				}
			},
			onReady: (url) => {
				const a = ctl.foot.querySelector('.open-when-ready');
				if (a) {
					setOpenButton(a, url);
				}
			},
		});
		ctl.body.appendChild(p.el);
		return ctl;
	}

	/* A link that becomes usable once the Homebridge web page has an address. */
	function openButton(url) {
		const a = h('a', { class: 'btn btn-quiet open-when-ready', target: '_blank', rel: 'noopener' }, [icon('open'), h('span')]);
		setOpenButton(a, url);
		return a;
	}

	function setOpenButton(a, url) {
		if (url) {
			a.href = url;
			a.removeAttribute('aria-disabled');
			a.classList.remove('is-waiting');
			a.lastChild.textContent = HB.t('Open Homebridge');
		} else {
			a.removeAttribute('href');
			a.setAttribute('aria-disabled', 'true');
			a.classList.add('is-waiting');
			a.lastChild.textContent = HB.t('Waiting for Homebridge…');
		}
	}

	/*
	 * Start a job through an API call and show it. call(): Promise<{ job_id }>.
	 * Handles 409 busy by offering the job that is already running.
	 */
	async function run(call, opts) {
		opts = opts || {};
		try {
			const r = await call();
			if (opts.instance) {
				const inst = HB.findInstance(opts.instance);
				if (inst) {
					inst.job = r.job_id;
				}
				document.dispatchEvent(new CustomEvent('hb:instances-changed'));
			}
			if (opts.quiet) {
				return r.job_id;
			}
			openDialog(r.job_id, opts);
			return r.job_id;
		} catch (err) {
			if (err.code === 'busy' && err.details && err.details.job_id) {
				HB.toast(HB.errorText(err), 'warn', { action: { label: HB.t('Show progress'), onClick: () => openDialog(err.details.job_id, opts) } });
				return null;
			}
			HB.fail(err);
			return null;
		}
	}

	HB.jobs = { isShown: (id) => shown.has(id), watch, stopAll, overallPercent, lightsDuring, panel, openDialog, run, openButton, setOpenButton, entry: (id) => entries.get(id) || null };
})();
