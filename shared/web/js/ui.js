/* Homebridge UI - shared widgets: toasts, dialogs, confirmations, menus,
 * tabs, form fields with server-side error mapping, switches. */
(function () {
	'use strict';

	const { h, icon } = HB;

	// ---------- Dialogs (native <dialog>: focus trap, Esc, top layer) ----------

	const stack = [];

	HB.topDialog = () => (stack.length ? stack[stack.length - 1].el : null);

	HB.closeAllDialogs = function () {
		while (stack.length) {
			stack[stack.length - 1].close(undefined);
		}
	};

	/*
	 * opts: { title, head (extra node under the title), body, actions: [{ label,
	 * kind, value, icon, onClick(ctl) -> false keeps it open }], size: 'sm'|'md'|'lg',
	 * className, dismissible (default true), onClose(value), labelClose }
	 * Returns a controller: { el, body, foot, close(value), setBusy(bool), done: Promise }
	 */
	HB.dialog = function (opts) {
		const titleId = HB.uid('dlg-title');
		const closeBtn = h('button', { type: 'button', class: 'icon-btn dlg-close', 'aria-label': opts.labelClose || HB.t('Close') }, icon('x'));
		const title = h('h2', { id: titleId, class: 'dlg-title', text: opts.title || '' });
		const head = h('header', { class: 'dlg-head' }, [h('div', { class: 'dlg-titles' }, [title, opts.head || null]), closeBtn]);
		const body = h('div', { class: 'dlg-body' });
		const foot = h('footer', { class: 'dlg-foot' });
		const el = h('dialog', { class: ['dlg', 'dlg-' + (opts.size || 'md'), opts.className], 'aria-labelledby': titleId }, [head, body, foot]);
		HB.append(body, opts.body);

		let resolve;
		const done = new Promise((r) => { resolve = r; });
		let busy = false;
		let closed = false;
		const returnFocus = document.activeElement;

		const ctl = {
			el, body, foot, head, title, done,
			close(value) {
				if (closed) {
					return;
				}
				closed = true;
				const i = stack.indexOf(ctl);
				if (i >= 0) {
					stack.splice(i, 1);
				}
				if (el.open) {
					el.close();
				}
				el.remove();
				if (opts.onClose) {
					opts.onClose(value);
				}
				resolve(value);
				if (returnFocus && document.contains(returnFocus) && returnFocus.focus) {
					returnFocus.focus({ preventScroll: true });
				}
			},
			setBusy(on) {
				busy = !!on;
				for (const b of foot.querySelectorAll('button')) {
					b.disabled = busy || b.dataset.disabled === '1';
				}
				el.classList.toggle('is-busy', busy);
			},
			setActions(actions) {
				HB.clear(foot);
				for (const a of actions || []) {
					if (a === 'spacer') {
						foot.appendChild(h('span', { class: 'grow' }));
						continue;
					}
					if (a instanceof Node) {
						foot.appendChild(a);
						continue;
					}
					const b = h(a.href ? 'a' : 'button', {
						type: a.href ? null : 'button',
						href: a.href || null,
						target: a.href ? '_blank' : null,
						rel: a.href ? 'noopener' : null,
						class: ['btn', a.kind ? 'btn-' + a.kind : 'btn-quiet', a.className],
						disabled: a.disabled || false,
						'data-disabled': a.disabled ? '1' : null,
					}, [a.icon ? icon(a.icon) : null, h('span', { text: a.label })]);
					b.addEventListener('click', async () => {
						if (a.onClick) {
							const r = await a.onClick(ctl, b);
							if (r === false) {
								return;
							}
						}
						if (!a.href && a.close !== false) {
							ctl.close(a.value);
						}
					});
					if (a.autofocus) {
						b.setAttribute('autofocus', '');
					}
					foot.appendChild(b);
				}
			},
			setTitle(text) {
				title.textContent = text;
			},
		};

		/* Close button and Esc: ask opts.beforeDismiss() first (e.g. unsaved input). */
		let asking = false;
		async function dismiss() {
			if (busy || asking || opts.dismissible === false) {
				return;
			}
			if (opts.beforeDismiss) {
				asking = true;
				const ok = await opts.beforeDismiss();
				asking = false;
				if (!ok) {
					return;
				}
			}
			ctl.close(undefined);
		}
		closeBtn.addEventListener('click', dismiss);
		el.addEventListener('cancel', (e) => {
			e.preventDefault();
			if (HB.topDialog() === el) {
				dismiss();
			}
		});
		// Chrome may close a dialog on a repeated Esc even after preventDefault.
		el.addEventListener('close', () => {
			if (closed) {
				return;
			}
			if (busy || opts.dismissible === false || opts.beforeDismiss) {
				el.showModal();
				if (!busy && opts.dismissible !== false) {
					dismiss();
				}
			} else {
				ctl.close(undefined);
			}
		});

		ctl.dismiss = dismiss;
		ctl.setActions(opts.actions);
		document.body.appendChild(el);
		stack.push(ctl);
		el.showModal();
		const auto = el.querySelector('[autofocus]') || body.querySelector('input:not([type=hidden]):not([disabled]), select, textarea');
		if (auto && opts.focusBody !== false) {
			auto.focus();
		} else {
			const primary = foot.querySelector('.btn-primary, .btn-danger');
			(primary || closeBtn).focus();
		}
		return ctl;
	};

	/*
	 * A plain yes/no question. body: string or nodes.
	 * opts: { title, body, confirm, cancel, danger, icon }
	 */
	HB.confirm = function (opts) {
		const body = typeof opts.body === 'string' ? h('p', { text: opts.body }) : opts.body;
		const ctl = HB.dialog({
			title: opts.title,
			size: 'sm',
			className: 'dlg-confirm' + (opts.danger ? ' is-danger' : ''),
			body: [opts.icon ? h('div', { class: 'confirm-mark' }, icon(opts.icon, { size: 22 })) : null, h('div', { class: 'confirm-text' }, body)],
			focusBody: false,
			actions: [
				'spacer',
				{ label: opts.cancel || HB.t('Cancel'), value: false },
				{ label: opts.confirm || HB.t('OK'), kind: opts.danger ? 'danger' : 'primary', value: true },
			],
		});
		return ctl.done.then((v) => v === true);
	};

	// ---------- Toasts ----------

	function toastHost() {
		// the top-most open modal (dialogs and the drawer), else the page
		const open = document.querySelectorAll('body > dialog[open]');
		const host = open.length ? open[open.length - 1] : document.body;
		let box = host.querySelector(':scope > .toasts');
		if (!box) {
			box = h('div', { class: 'toasts', role: 'status', 'aria-live': 'polite' });
			host.appendChild(box);
		}
		return box;
	}

	/* kind: 'ok' | 'info' | 'warn' | 'error' */
	HB.toast = function (message, kind, opts) {
		opts = opts || {};
		const box = toastHost();
		const iconName = { ok: 'check', warn: 'alert', error: 'alert', info: 'info' }[kind || 'info'];
		const el = h('div', { class: ['toast', 'toast-' + (kind || 'info')] }, [
			icon(iconName, { size: 18 }),
			h('span', { class: 'toast-text', text: message }),
			opts.action ? h('button', { type: 'button', class: 'toast-action', text: opts.action.label, onclick: () => { opts.action.onClick(); dismiss(); } }) : null,
		]);
		box.appendChild(el);
		let timer = setTimeout(dismiss, opts.timeout || (kind === 'error' ? 9000 : 4500));
		el.addEventListener('mouseenter', () => clearTimeout(timer));
		el.addEventListener('mouseleave', () => { timer = setTimeout(dismiss, 2500); });
		function dismiss() {
			clearTimeout(timer);
			el.classList.add('is-leaving');
			setTimeout(() => el.remove(), HB.reducedMotion() ? 0 : 180);
		}
		return dismiss;
	};

	/* Show an API error as a toast unless the page already took it over. */
	HB.fail = function (err) {
		if (err && err.handled) {
			return;
		}
		HB.toast(HB.errorText(err), 'error');
	};

	// ---------- Menus ----------

	let openMenu = null;

	/*
	 * Attach a menu to a button. items(): [{ label, icon, onClick, danger, disabled, hint }]
	 * built each time the menu opens, so it always reflects current data.
	 */
	HB.menuButton = function (button, items, opts) {
		opts = opts || {};
		button.setAttribute('aria-haspopup', 'menu');
		button.setAttribute('aria-expanded', 'false');
		button.addEventListener('click', (e) => {
			e.stopPropagation();
			if (openMenu && openMenu.button === button) {
				openMenu.close();
				return;
			}
			show(button, items(), opts);
		});
		button.addEventListener('keydown', (e) => {
			if (e.key === 'ArrowDown') {
				e.preventDefault();
				show(button, items(), opts);
			}
		});
	};

	function show(button, list, opts) {
		if (openMenu) {
			openMenu.close();
		}
		const menu = h('div', { class: 'menu', role: 'menu', popover: 'auto', 'aria-label': opts.label || null });
		const buttons = [];
		for (const it of list) {
			if (!it) {
				continue;
			}
			if (it === 'sep') {
				menu.appendChild(h('div', { class: 'menu-sep', role: 'separator' }));
				continue;
			}
			const b = h('button', {
				type: 'button', role: 'menuitem', class: ['menu-item', it.danger ? 'is-danger' : null], disabled: it.disabled || false, tabindex: '-1',
			}, [it.icon ? icon(it.icon) : h('span', { class: 'icon-space' }), h('span', { class: 'menu-label' }, [h('span', { text: it.label }), it.hint ? h('small', { text: it.hint }) : null])]);
			b.addEventListener('click', () => {
				close();
				it.onClick();
			});
			buttons.push(b);
			menu.appendChild(b);
		}
		const host = button.closest('dialog') || document.body;
		host.appendChild(menu);
		function close() {
			if (!openMenu || openMenu.menu !== menu) {
				return;
			}
			openMenu = null;
			button.setAttribute('aria-expanded', 'false');
			try {
				menu.hidePopover();
			} catch (e) { /* already hidden */ }
			menu.remove();
		}
		openMenu = { menu, button, close };
		menu.addEventListener('toggle', (e) => {
			if (e.newState === 'closed') {
				const refocus = menu.contains(document.activeElement) || document.activeElement === document.body;
				close();
				if (refocus) {
					button.focus();
				}
			}
		});
		menu.addEventListener('keydown', (e) => {
			const enabled = buttons.filter((b) => !b.disabled);
			const i = enabled.indexOf(document.activeElement);
			if (e.key === 'ArrowDown') {
				e.preventDefault();
				enabled[(i + 1) % enabled.length].focus();
			} else if (e.key === 'ArrowUp') {
				e.preventDefault();
				enabled[(i - 1 + enabled.length) % enabled.length].focus();
			} else if (e.key === 'Home') {
				e.preventDefault();
				enabled[0].focus();
			} else if (e.key === 'End') {
				e.preventDefault();
				enabled[enabled.length - 1].focus();
			} else if (e.key === 'Tab') {
				close();
			}
		});
		menu.showPopover();
		button.setAttribute('aria-expanded', 'true');
		const r = button.getBoundingClientRect();
		const mw = menu.offsetWidth;
		const mh = menu.offsetHeight;
		let left = opts.align === 'start' ? r.left : r.right - mw;
		left = Math.max(8, Math.min(left, window.innerWidth - mw - 8));
		let top = r.bottom + 6;
		if (top + mh > window.innerHeight - 8) {
			top = Math.max(8, r.top - mh - 6);
		}
		menu.style.left = left + 'px';
		menu.style.top = top + 'px';
		const first = buttons.find((b) => !b.disabled);
		if (first) {
			first.focus();
		}
	}

	// ---------- Tabs ----------

	/* tabs: [{ key, label, render(panel) }] -> { el, select(key), current } */
	HB.tabs = function (tabs, opts) {
		opts = opts || {};
		const list = h('div', { class: 'tabs', role: 'tablist', 'aria-label': opts.label || null });
		const panels = h('div', { class: 'tab-panels' });
		const btns = {};
		const panelEls = {};
		const ctl = { el: h('div', { class: 'tabset' }, [list, panels]), current: null, list, panels };
		for (const t of tabs) {
			const id = HB.uid('tab');
			const b = h('button', { type: 'button', role: 'tab', id: id, class: 'tab', 'aria-selected': 'false', tabindex: '-1' }, [h('span', { text: t.label }), t.badge ? t.badge : null]);
			const p = h('div', { role: 'tabpanel', class: 'tab-panel', 'aria-labelledby': id, hidden: true, tabindex: '0' });
			b.setAttribute('aria-controls', p.id = HB.uid('panel'));
			b.addEventListener('click', () => ctl.select(t.key, true));
			b.addEventListener('keydown', (e) => {
				const keys = tabs.map((x) => x.key);
				const i = keys.indexOf(t.key);
				let next = null;
				if (e.key === 'ArrowRight') {
					next = keys[(i + 1) % keys.length];
				} else if (e.key === 'ArrowLeft') {
					next = keys[(i - 1 + keys.length) % keys.length];
				} else if (e.key === 'Home') {
					next = keys[0];
				} else if (e.key === 'End') {
					next = keys[keys.length - 1];
				}
				if (next) {
					e.preventDefault();
					ctl.select(next, true);
					btns[next].focus();
				}
			});
			btns[t.key] = b;
			panelEls[t.key] = p;
			list.appendChild(b);
			panels.appendChild(p);
		}
		ctl.buttons = btns;
		ctl.panels = panelEls;
		ctl.select = function (key, user) {
			if (!btns[key]) {
				key = tabs[0].key;
			}
			const prev = ctl.current;
			ctl.current = key;
			for (const t of tabs) {
				const on = t.key === key;
				btns[t.key].setAttribute('aria-selected', on ? 'true' : 'false');
				btns[t.key].tabIndex = on ? 0 : -1;
				panelEls[t.key].hidden = !on;
			}
			const t = tabs.find((x) => x.key === key);
			if (prev !== key && t.render) {
				t.render(panelEls[key]);
			}
			if (opts.onSelect) {
				opts.onSelect(key, prev, !!user);
			}
		};
		return ctl;
	};

	// ---------- Form fields ----------

	/*
	 * A labelled control. opts: { label, help, control, path, optional, hint (node
	 * under the control, e.g. a live check), className }
	 */
	HB.field = function (opts) {
		const control = opts.control;
		let id = control.id;
		if (!id && control.matches && control.matches('input, select, textarea')) {
			id = control.id = HB.uid('f');
		}
		const helpId = opts.help ? HB.uid('help') : null;
		const errId = HB.uid('err');
		const err = h('p', { class: 'field-error', id: errId, hidden: true });
		const labelEl = opts.label ? h(id ? 'label' : 'span', { class: 'field-label', for: id || null }, [
			h('span', { text: opts.label }),
			opts.optional ? h('span', { class: 'field-optional', text: HB.t('Optional') }) : null,
		]) : null;
		const wrap = h('div', { class: ['field', opts.className], 'data-path': opts.path || null }, [
			labelEl,
			control,
			opts.hint || null,
			opts.help ? h('p', { class: 'field-help', id: helpId, text: opts.help }) : null,
			err,
		]);
		// editing a field takes its error away, so the live check under it shows again
		wrap.addEventListener('input', () => {
			if (wrap.classList.contains('has-error')) {
				wrap.classList.remove('has-error');
				err.hidden = true;
				err.textContent = '';
				const bad = wrap.querySelector('[aria-invalid]');
				if (bad) {
					bad.removeAttribute('aria-invalid');
				}
			}
		});
		const described = [helpId, errId].filter(Boolean).join(' ');
		if (control.matches && control.matches('input, select, textarea')) {
			control.setAttribute('aria-describedby', described);
		}
		return wrap;
	};

	HB.input = function (attrs) {
		return h('input', Object.assign({ class: 'input', type: 'text', spellcheck: 'false', autocomplete: 'off' }, attrs || {}));
	};

	/*
	 * Map server `fields` ({ "network.ip": "…" }) onto [data-path] wrappers inside
	 * root. Returns the messages that matched nothing, so callers can show them.
	 */
	HB.showFieldErrors = function (root, fields) {
		HB.clearFieldErrors(root);
		const unmatched = [];
		let first = null;
		for (const path of Object.keys(fields || {})) {
			const wrap = root.querySelector('[data-path="' + CSS.escape(path) + '"]');
			if (!wrap) {
				unmatched.push(HB.msg(fields[path]));
				continue;
			}
			wrap.classList.add('has-error');
			const e = wrap.querySelector(':scope > .field-error') || wrap.querySelector('.field-error');
			if (e) {
				e.textContent = HB.msg(fields[path]);
				e.hidden = false;
			}
			const ctl = wrap.querySelector('input, select, textarea');
			if (ctl) {
				ctl.setAttribute('aria-invalid', 'true');
			}
			if (!first) {
				first = wrap;
			}
		}
		if (first) {
			const ctl = first.querySelector('input, select, textarea, button');
			if (ctl) {
				ctl.focus({ preventScroll: true });
			}
			first.scrollIntoView({ block: 'center', behavior: HB.reducedMotion() ? 'auto' : 'smooth' });
		}
		return unmatched;
	};

	/* Show an API error inside a form: fields on their inputs, anything else as a toast. */
	HB.formError = function (root, err) {
		if (!err || err.handled) {
			return;
		}
		if (err.fields) {
			const rest = HB.showFieldErrors(root, err.fields);
			for (const m of rest) {
				HB.toast(m, 'error');
			}
			return;
		}
		HB.fail(err);
	};

	HB.clearFieldErrors = function (root, path) {
		const sel = path ? '[data-path="' + CSS.escape(path) + '"]' : '.has-error';
		for (const w of root.querySelectorAll(sel)) {
			w.classList.remove('has-error');
			const e = w.querySelector('.field-error');
			if (e) {
				e.hidden = true;
				e.textContent = '';
			}
			const ctl = w.querySelector('[aria-invalid]');
			if (ctl) {
				ctl.removeAttribute('aria-invalid');
			}
		}
	};

	HB.switchControl = function (opts) {
		const input = h('input', { type: 'checkbox', role: 'switch', checked: !!opts.checked, class: 'switch-input', 'aria-label': opts.ariaLabel || null });
		if (opts.onChange) {
			input.addEventListener('change', () => opts.onChange(input.checked));
		}
		const el = h('label', { class: ['switch', opts.className] }, [input, h('span', { class: 'switch-track', 'aria-hidden': 'true' }, h('span', { class: 'switch-thumb' })), opts.label ? h('span', { class: 'switch-label', text: opts.label }) : null]);
		el.input = input;
		return el;
	};

	/* Radio cards: options [{ value, title, text, icon }] */
	HB.choiceCards = function (opts) {
		const name = HB.uid('choice');
		const wrap = h('div', { class: ['choices', opts.className], role: 'radiogroup', 'aria-label': opts.label || null });
		const inputs = [];
		for (const o of opts.options) {
			const input = h('input', { type: 'radio', name: name, value: o.value, checked: o.value === opts.value, class: 'choice-input' });
			input.addEventListener('change', () => {
				if (input.checked && opts.onChange) {
					opts.onChange(o.value);
				}
			});
			inputs.push(input);
			wrap.appendChild(h('label', { class: 'choice' }, [
				input,
				h('span', { class: 'choice-body' }, [
					h('span', { class: 'choice-title' }, [o.icon ? icon(o.icon) : null, h('span', { text: o.title })]),
					o.text ? h('span', { class: 'choice-text', text: o.text }) : null,
				]),
			]));
		}
		wrap.set = (v) => {
			for (const i of inputs) {
				i.checked = i.value === v;
			}
		};
		return wrap;
	};

	/* A short inline status line used by live checks: kind ok | warn | error | busy | info */
	HB.checkLine = function () {
		const el = h('p', { class: 'check-line', 'aria-live': 'polite', hidden: true });
		el.set = function (kind, text) {
			HB.clear(el);
			if (!kind) {
				el.hidden = true;
				return;
			}
			el.hidden = false;
			el.className = 'check-line is-' + kind;
			const mark = kind === 'busy' ? h('span', { class: 'spinner', 'aria-hidden': 'true' }) : icon({ ok: 'check', warn: 'alert', error: 'alert', info: 'info' }[kind], { size: 16 });
			HB.append(el, [mark, h('span', { text: text })]);
		};
		return el;
	};

	HB.callout = function (kind, title, body, iconName) {
		return h('div', { class: ['callout', 'callout-' + kind] }, [
			icon(iconName || ({ warn: 'alert', error: 'alert', ok: 'check', info: 'info' }[kind] || 'info'), { size: 20 }),
			h('div', { class: 'callout-body' }, [title ? h('strong', { class: 'callout-title', text: title }) : null, body]),
		]);
	};

	HB.button = function (label, opts) {
		opts = opts || {};
		const b = h(opts.href ? 'a' : 'button', {
			type: opts.href ? null : 'button',
			href: opts.href || null,
			target: opts.href && opts.external !== false ? '_blank' : null,
			rel: opts.href && opts.external !== false ? 'noopener' : null,
			class: ['btn', 'btn-' + (opts.kind || 'quiet'), opts.size ? 'btn-' + opts.size : null, opts.className],
			disabled: opts.disabled || false,
			title: opts.title || null,
			'aria-label': opts.ariaLabel || null,
		}, [opts.icon ? HB.icon(opts.icon) : null, label ? h('span', { text: label }) : null]);
		if (opts.onClick) {
			b.addEventListener('click', opts.onClick);
		}
		return b;
	};

	/* Run an async action with the button disabled and a busy label. */
	HB.busy = async function (button, label, fn) {
		const span = button.querySelector('span');
		const old = span ? span.textContent : '';
		button.disabled = true;
		button.classList.add('is-busy');
		if (span && label) {
			span.textContent = label;
		}
		try {
			return await fn();
		} finally {
			button.disabled = false;
			button.classList.remove('is-busy');
			if (span && label) {
				span.textContent = old;
			}
		}
	};
})();
