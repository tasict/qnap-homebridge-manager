/* Homebridge UI - folder picker. Walk /share with GET /fs, make a folder with
 * POST /fs/mkdir, and choose the folder you are in (works the same with a
 * mouse, a keyboard or a finger: tap a folder to open it). */
(function () {
	'use strict';

	const { h, icon } = HB;

	/* HB.folders.pick({ title, start }) -> Promise<string | null> */
	function pick(opts) {
		opts = opts || {};
		const t = HB.t;
		let current = null;
		const crumbs = h('nav', { class: 'fs-crumbs', 'aria-label': t('Current location') });
		const list = h('div', { class: 'fs-list', role: 'list', 'aria-live': 'polite' });
		const msg = HB.checkLine();
		const nameInput = HB.input({ placeholder: t('New folder name'), 'aria-label': t('New folder name'), maxlength: '120' });
		const createBtn = HB.button(t('Create'), { kind: 'quiet', size: 'sm' });
		const newRow = h('div', { class: 'fs-new', hidden: true }, [nameInput, createBtn]);
		const newBtn = HB.button(t('New folder'), { icon: 'folderPlus', size: 'sm', kind: 'ghost' });
		const here = h('span', { class: 'fs-current' });

		const ctl = HB.dialog({
			title: opts.title || t('Choose a folder'),
			size: 'md',
			className: 'dlg-folders',
			body: [crumbs, list, h('div', { class: 'input-group', style: { marginTop: '10px' } }, [newBtn]), newRow, msg],
			focusBody: false,
			actions: [here, { label: t('Cancel'), value: null }, {
				label: t('Use this folder'), kind: 'primary', onClick: () => {
					if (!current || !current.selectable) {
						return false;
					}
					ctl.close(current.path);
					return false;
				},
			}],
		});
		const chooseBtn = ctl.foot.querySelector('.btn-primary');

		newBtn.addEventListener('click', () => {
			newRow.hidden = !newRow.hidden;
			if (!newRow.hidden) {
				nameInput.value = '';
				nameInput.focus();
			}
		});
		nameInput.addEventListener('keydown', (e) => {
			if (e.key === 'Enter') {
				e.preventDefault();
				createBtn.click();
			}
		});
		createBtn.addEventListener('click', async () => {
			const name = nameInput.value.trim();
			if (!name) {
				nameInput.focus();
				return;
			}
			if (/[/\\]/.test(name) || name === '.' || name === '..') {
				msg.set('error', t('A folder name cannot contain / or \\'));
				return;
			}
			if (!current || current.path === '/share') {
				msg.set('error', t('Open a shared folder first, then create the new folder inside it'));
				return;
			}
			await HB.busy(createBtn, t('Creating…'), async () => {
				try {
					const r = await HB.api.post('/fs/mkdir', { path: current.path + '/' + name });
					newRow.hidden = true;
					msg.set('ok', t('Created {path}', { path: r.path }));
					await load(r.path);
				} catch (err) {
					if (!err.handled) {
						msg.set('error', err.fields && err.fields.path ? HB.msg(err.fields.path) : HB.errorText(err));
					}
				}
			});
		});

		async function load(path) {
			HB.clear(list);
			list.appendChild(h('p', { class: 'fs-empty' }, [h('span', { class: 'spinner', 'aria-hidden': 'true' }), ' ', t('Loading…')]));
			let data;
			try {
				data = await HB.api.get('/fs', { path: path || '/share' });
			} catch (err) {
				if (err.handled) {
					return;
				}
				if (path && path !== '/share') {
					// the starting folder may not exist yet: fall back to its parent, then /share
					const parent = path.replace(/\/[^/]*$/, '') || '/share';
					msg.set('info', err.code === 'not_found' ? t('{path} does not exist yet, so this starts one level up.', { path }) : HB.errorText(err));
					return load(parent.length < 7 ? '/share' : parent);
				}
				HB.clear(list);
				list.appendChild(h('p', { class: 'fs-empty', text: HB.errorText(err) }));
				return;
			}
			const selectable = data.path !== '/share' && !(opts.blocked && opts.blocked(data.path));
			current = { path: data.path, parent: data.parent, selectable };
			paintCrumbs(data.path);
			HB.clear(list);
			if (data.parent) {
				list.appendChild(row('..', data.parent, null, 'chevronLeft'));
			}
			if (!data.dirs.length) {
				list.appendChild(h('p', { class: 'fs-empty', text: data.path === '/share' ? t('There are no shared folders to use.') : t('There are no folders in here. Use this folder, or create a new one.') }));
			}
			for (const d of data.dirs) {
				list.appendChild(row(d.name, d.path, d.selectable ? null : t('Browse only'), 'folder'));
			}
			here.textContent = data.path;
			here.title = data.path;
			chooseBtn.disabled = !selectable;
			chooseBtn.dataset.disabled = selectable ? '' : '1';
			if (data.path === '/share') {
				msg.set('info', t('Open a shared folder first. Homebridge can only use folders under /share.'));
			} else if (msg.classList.contains('is-info')) {
				msg.set(null);
			}
			const first = list.querySelector('.fs-item');
			if (first && ctl.el.contains(document.activeElement) === false) {
				first.focus();
			}
		}

		function row(name, path, note, iconName) {
			const b = h('button', { type: 'button', class: 'fs-item', role: 'listitem' }, [
				icon(iconName),
				h('span', { class: 'fs-name', text: name === '..' ? HB.t('Up one level') : name }),
				note && name !== '..' ? h('small', { text: note }) : null,
				name === '..' ? null : icon('chevronRight', { class: 'chevron' }),
			]);
			b.addEventListener('click', () => load(path));
			return b;
		}

		function paintCrumbs(path) {
			HB.clear(crumbs);
			const parts = path.split('/').filter(Boolean);
			let acc = '';
			parts.forEach((p, i) => {
				acc += '/' + p;
				const target = acc;
				if (i > 0) {
					crumbs.appendChild(h('span', { class: 'fs-sep', 'aria-hidden': 'true', text: '/' }));
				}
				const last = i === parts.length - 1;
				const b = h('button', { type: 'button', class: 'fs-crumb', text: i === 0 ? '/' + p : p, 'aria-current': last ? 'location' : null });
				b.addEventListener('click', () => load(target));
				crumbs.appendChild(b);
			});
		}

		load(opts.start || '/share');
		return ctl.done.then((v) => (typeof v === 'string' ? v : null));
	}

	HB.folders = { pick };
})();
