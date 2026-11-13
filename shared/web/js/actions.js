/* Homebridge UI - actions on one Homebridge: start / stop / restart,
 * upgrade (with the legacy image switch), apply and recreate, roll back,
 * delete the backup, remove. Each one says what will happen before it does. */
(function () {
	'use strict';

	const { h, icon, t } = HB;

	function inst(name) {
		return HB.findInstance(name);
	}

	/* Put a fresh Instance from the API into the list and repaint. */
	function store(fresh) {
		const i = HB.instances.findIndex((x) => x.spec.name === fresh.spec.name);
		if (i >= 0) {
			HB.instances[i] = fresh;
		} else {
			HB.instances.push(fresh);
		}
		HB.street.render();
		document.dispatchEvent(new CustomEvent('hb:data'));
	}
	HB.store = store;

	function backupNote(i) {
		return i.backup ? h('p', { class: 'note', text: t('The current backup (from {date}) will be replaced by this container.', { date: HB.dateTime(i.backup.created_at) }) }) : null;
	}

	// ---------- start / stop / restart ----------

	const POWER = {
		start: [HB.N_('Starting…'), HB.N_('Started “{name}”'), HB.N_('Starting “{name}”…')],
		stop: [HB.N_('Stopping…'), HB.N_('Stopped “{name}”'), HB.N_('Stopping “{name}”…')],
		restart: [HB.N_('Restarting…'), HB.N_('Restarted “{name}”'), HB.N_('Restarting “{name}”…')],
	};

	async function power(name, what, button) {
		const i = inst(name);
		if (!i) {
			return;
		}
		const words = POWER[what];
		const run = async () => {
			try {
				const fresh = await HB.api.post('/instances/' + HB.enc(name) + '/' + what);
				store(fresh);
				HB.toast(t(words[1], { name: HB.displayName(fresh) }), 'ok');
			} catch (err) {
				if (err.code === 'busy' && err.details && err.details.job_id) {
					HB.toast(HB.errorText(err), 'warn', { action: { label: t('Show progress'), onClick: () => HB.jobs.openDialog(err.details.job_id, { instance: name }) } });
				} else {
					HB.fail(err);
				}
			}
		};
		if (button) {
			await HB.busy(button, t(words[0]), run);
		} else {
			const dismiss = HB.toast(t(words[2], { name: HB.displayName(i) }), 'info', { timeout: 20000 });
			await run();
			dismiss();
		}
	}

	// ---------- upgrade ----------

	function upgrade(name) {
		const i = inst(name);
		if (!i) {
			return;
		}
		if (i.job) {
			HB.jobs.openDialog(i.job, { instance: name });
			return;
		}
		const up = i.update || {};
		const st = i.state || {};
		const current = i.spec.image;
		const repo = current.split(':')[0];
		const version = st.image_version ? t('Version {version}', { version: st.image_version }) : t('The version it runs now');
		let switchTo = up.legacy_image && up.suggested_image ? up.suggested_image : null;

		const house = HB.house.create({ size: 'md' });
		house.update(Object.assign(HB.house.modelFor(i, null), { pending: false }));

		const from = h('div', { class: 'ver ver-from' }, [h('span', { class: 'ver-label', text: t('Now') }), h('span', { class: 'mono', text: current }), h('span', { class: 'ver-sub', text: version })]);
		const toImage = h('span', { class: 'mono' });
		const toSub = h('span', { class: 'ver-sub' });
		const to = h('div', { class: 'ver ver-to' }, [h('span', { class: 'ver-label', text: t('After the upgrade') }), toImage, toSub]);
		function paintTo() {
			toImage.textContent = switchTo || current;
			if (switchTo) {
				toSub.textContent = t('The latest version of a maintained image');
			} else if (up.reason === 'newer_local') {
				toSub.textContent = t('A newer version this NAS has already downloaded');
			} else if (up.available) {
				toSub.textContent = t('The latest version on Docker Hub');
			} else {
				toSub.textContent = t('Docker Hub is asked again; it changes only if there is a newer version');
			}
		}

		const parts = [h('div', { class: 'upgrade-top' }, [h('div', { class: 'upgrade-house' }, house.el), h('div', { class: 'versions' }, [from, h('div', { class: 'ver-arrow', 'aria-hidden': 'true' }, icon('chevronDown')), to])])];

		if (up.legacy_image && up.suggested_image) {
			const since = st.image_created ? new Date(HB.parseTime(st.image_created)).getUTCFullYear() : null;
			const why = since
				? t('This image has not been updated since {year}. Switching to {image} is recommended: the folder layout is the same, and the settings, plugins and HomeKit pairing are kept.', { year: since, image: up.suggested_image })
				: t('This image is no longer updated. Switching to {image} is recommended: the folder layout is the same, and the settings, plugins and HomeKit pairing are kept.', { image: up.suggested_image });
			parts.push(HB.callout('warn', t('{image} is no longer maintained', { image: repo }), h('p', { text: why })));
			parts.push(HB.choiceCards({
				label: t('Which image to use'),
				value: 'switch',
				options: [
					{ value: 'switch', title: t('Switch to {image}', { image: up.suggested_image }), text: t('Recommended. You will hear about every new version.') },
					{ value: 'keep', title: t('Keep {image}', { image: repo }), text: up.available ? t('Upgrades to its last version.') : t('There will be no new versions, so usually nothing changes.') },
				],
				onChange: (v) => {
					switchTo = v === 'switch' ? up.suggested_image : null;
					paintTo();
				},
			}));
		}
		parts.push(h('div', { class: 'confirm-text' }, [
			h('ul', null, [
				h('li', { text: t('The new image is downloaded first, then the container is recreated with it. Homebridge restarts, for about a minute.') }),
				h('li', { text: t('The current container is kept as the backup. If the new one does not run properly, it is put back automatically.') }),
				h('li', { text: t('The settings, plugins and HomeKit pairing live in the work folder and are not affected.') }),
			]),
			backupNote(i),
		]));
		paintTo();

		const title = switchTo ? t('Switch image and upgrade “{name}”', { name: HB.displayName(i) }) : HB.jobTitle('upgrade', HB.displayName(i));
		HB.dialog({
			title,
			size: 'md',
			className: 'dlg-upgrade',
			body: parts,
			focusBody: false,
			actions: ['spacer', { label: t('Cancel') }, {
				label: t('Start the upgrade'), kind: 'primary', icon: 'upgrade', onClick: async (c) => {
					c.setBusy(true);
					const body = switchTo ? { image: switchTo } : {};
					const id = await HB.jobs.run(() => HB.api.post('/instances/' + HB.enc(name) + '/upgrade', body), { instance: name, quiet: true });
					c.setBusy(false);
					c.close();
					if (id) {
						HB.jobs.openDialog(id, { instance: name, title: HB.jobTitle('upgrade', HB.displayName(i)) });
					}
					return false;
				},
			}],
		});
	}

	// ---------- apply and recreate ----------

	async function apply(name) {
		const i = inst(name);
		if (!i) {
			return;
		}
		const missing = i.state.status === 'missing';
		// a container with the name exists but is not ours (after going back to the original of a takeover, for one)
		const foreign = missing && !!i.state.foreign;
		const label = HB.displayName(i);
		const folder = i.spec.workdir;
		let title;
		let body;
		let confirm;
		let jobTitle;
		if (foreign) {
			title = t('Take over the container of “{name}”', { name: label });
			confirm = t('Take over container');
			jobTitle = t('Take over the container of “{name}”', { name: label });
			body = [
				h('p', { text: t('The name {container} now belongs to a container that is not managed here. Taking it over means:', { container: i.spec.name }) }),
				h('ul', null, [
					h('li', { text: t('That container stops and is kept as the backup, so you can go back at any time.') }),
					h('li', { text: t('A new container is created from the saved settings. Homebridge restarts, for about a minute.') }),
					h('li', { text: t('The data in the work folder {folder} is used as it is. If the new container does not run properly, the old one is put back automatically.', { folder }) }),
				]),
			];
		} else if (missing) {
			title = t('Create a container for “{name}”', { name: label });
			confirm = t('Create container');
			jobTitle = title;
			body = [
				h('p', { text: t('This Homebridge has no container right now. A new one is created from the saved settings and started.') }),
				h('ul', null, [h('li', { text: t('The data in the work folder {folder} is used as it is.', { folder }) })]),
			];
		} else {
			title = HB.jobTitle('apply', label);
			confirm = t('Apply and recreate');
			jobTitle = title;
			body = [
				h('p', { text: t('The container is created again from the saved settings:') }),
				h('ul', null, [
					h('li', { text: t('Homebridge restarts, for about a minute.') }),
					h('li', { text: t('The settings, plugins and HomeKit pairing live in the work folder and are not affected.') }),
					h('li', { text: t('The current container is kept as the backup. If the new one does not run properly, it is put back automatically.') }),
				]),
				backupNote(i),
			];
		}
		const ok = await HB.confirm({ title, icon: 'refresh', body, confirm });
		if (!ok) {
			return;
		}
		await HB.jobs.run(() => HB.api.post('/instances/' + HB.enc(name) + '/apply'), { instance: name, title: jobTitle });
	}

	// ---------- backup ----------

	async function rollback(name) {
		const i = inst(name);
		if (!i || !i.backup) {
			return;
		}
		const b = i.backup;
		const ok = await HB.confirm({
			title: t('Go back to the backup?'),
			icon: 'undo',
			body: [
				h('p', { text: t('“{name}” goes back to the container kept on {date} ({image}), and its settings go back to what they were then.', { name: HB.displayName(i), date: HB.dateTime(b.created_at), image: b.image }) }),
				h('ul', null, [
					h('li', { text: t('The current container stops and Homebridge restarts, for about a minute.') }),
					h('li', { text: t('The data in the work folder does not change.') }),
				]),
			],
			confirm: t('Go back to the backup'),
		});
		if (!ok) {
			return;
		}
		await HB.jobs.run(() => HB.api.post('/instances/' + HB.enc(name) + '/rollback'), { instance: name, title: HB.jobTitle('rollback', HB.displayName(i)) });
	}

	async function deleteBackup(name) {
		const i = inst(name);
		if (!i || !i.backup) {
			return;
		}
		const ok = await HB.confirm({
			title: t('Delete the backup?'),
			icon: 'trash',
			danger: true,
			body: [h('p', { text: t('After this you cannot go back to the version from {date}. The running container and all folders are not affected.', { date: HB.dateTime(i.backup.created_at) }) })],
			confirm: t('Delete backup'),
		});
		if (!ok) {
			return;
		}
		try {
			const fresh = await HB.api.del('/instances/' + HB.enc(name) + '/backup');
			store(fresh);
			HB.toast(t('Deleted the backup of “{name}”', { name: HB.displayName(fresh) }), 'ok');
		} catch (err) {
			HB.fail(err);
		}
	}

	// ---------- remove ----------

	function remove(name) {
		const i = inst(name);
		if (!i) {
			return;
		}
		let mode = 'keep';
		const label = HB.displayName(i);
		const hasContainer = i.state.status !== 'missing';
		const folders = [i.spec.workdir].concat((i.spec.mounts || []).map((m) => m.host));
		const choices = HB.choiceCards({
			label: t('How to remove it'),
			value: 'keep',
			options: [
				{ value: 'keep', title: t('Stop managing it'), text: hasContainer ? t('The container keeps running; it is just no longer managed here. You can take it over again later.') : t('Removes this Homebridge’s settings from here.') },
				{ value: 'remove', title: t('Remove the container too'), text: i.backup ? t('Stops and deletes the container and its backup. The accessories in the Home app lose their connection.') : t('Stops and deletes the container. The accessories in the Home app lose their connection.') },
			],
			onChange: (v) => {
				mode = v;
				const b = ctl.foot.querySelector('.btn-danger, .btn-primary');
				b.className = 'btn ' + (v === 'remove' ? 'btn-danger' : 'btn-primary');
				b.querySelector('span').textContent = v === 'remove' ? t('Remove container') : t('Stop managing');
			},
		});
		const ctl = HB.dialog({
			title: HB.jobTitle('remove', label),
			size: 'md',
			body: h('div', { class: 'form-stack', style: { gap: '14px' } }, [
				hasContainer ? choices : null,
				HB.callout('ok', t('Every folder stays'), h('div', null, [
					h('p', { text: hasContainer ? t('Whichever you choose, these folders are not deleted:') : t('Only the settings here are removed. These folders are not deleted:') }),
					h('ul', null, folders.map((f) => h('li', { class: 'mono', text: f }))),
				]), 'folder'),
			]),
			focusBody: false,
			actions: ['spacer', { label: t('Cancel') }, {
				label: hasContainer ? t('Stop managing') : t('Remove'), kind: 'primary', onClick: async (c) => {
					c.setBusy(true);
					if (mode === 'keep' || !hasContainer) {
						try {
							await HB.api.del('/instances/' + HB.enc(name), { container: 'keep' });
							c.setBusy(false);
							c.close();
							HB.drawer.close(true);
							HB.toast(hasContainer ? t('No longer managed here; the container keeps running') : t('Removed “{name}”', { name: label }), 'ok');
							HB.refresh(true);
						} catch (err) {
							c.setBusy(false);
							HB.fail(err);
						}
						return false;
					}
					const id = await HB.jobs.run(() => HB.api.del('/instances/' + HB.enc(name), { container: 'remove' }), { instance: name, quiet: true });
					c.setBusy(false);
					c.close();
					if (id) {
						HB.drawer.close(true);
						HB.jobs.openDialog(id, { instance: name, title: HB.jobTitle('remove', label) });
					}
					return false;
				},
			}],
		});
	}

	// ---------- ip in use ----------

	function confirmIpInUse(ip, d) {
		return HB.confirm({
			title: t('{ip} seems to be in use', { ip }),
			icon: 'alert',
			body: [
				h('p', { text: d.mac ? t('A device on the local network (MAC {mac}) answered on this address.', { mac: d.mac }) : t('A device on the local network answered on this address.') }),
				h('ul', null, [
					h('li', { text: t('If that is the old container being replaced, or you are sure the device is no longer used, you can go on.') }),
					h('li', { text: t('Otherwise two devices fight over one address, and neither connects reliably.') }),
				]),
			],
			cancel: t('Choose another address'),
			confirm: t('Use it anyway'),
		});
	}

	// ---------- the overflow menu on each house ----------

	function menuItems(i) {
		if (!i) {
			return [];
		}
		const name = i.spec.name;
		const s = i.state.status;
		const busy = !!i.job;
		const hasContainer = s !== 'missing';
		const up = i.update || {};
		const items = [
			{ label: t('Details'), icon: 'info', onClick: () => HB.drawer.open(name) },
			'sep',
		];
		if (busy) {
			items.push({ label: t('Show progress'), icon: 'refresh', onClick: () => HB.jobs.openDialog(i.job, { instance: name }) });
		}
		if (hasContainer) {
			if (s === 'running' || s === 'restarting') {
				items.push({ label: t('Stop'), icon: 'stop', disabled: busy, onClick: () => power(name, 'stop') });
				items.push({ label: t('Restart'), icon: 'restart', disabled: busy, onClick: () => power(name, 'restart') });
			} else {
				items.push({ label: t('Start'), icon: 'play', disabled: busy, onClick: () => power(name, 'start') });
			}
		}
		if (up.legacy_image) {
			items.push({ label: t('Switch image and upgrade…'), icon: 'upgrade', disabled: busy, hint: t('The current image is no longer maintained'), onClick: () => upgrade(name) });
		} else if (up.available) {
			items.push({ label: t('Upgrade…'), icon: 'upgrade', disabled: busy, hint: t('An update is available'), onClick: () => upgrade(name) });
		}
		if (i.pending_changes) {
			const label = hasContainer ? t('Apply and recreate…') : (i.state.foreign ? t('Take over container…') : t('Create container…'));
			const hint = hasContainer ? (i.state.in_container_station ? t('Still in Container Station') : t('Some settings are not applied yet')) : null;
			items.push({ label, icon: 'refresh', disabled: busy, hint, onClick: () => apply(name) });
		}
		items.push('sep');
		items.push({ label: t('Logs'), icon: 'logs', onClick: () => HB.drawer.open(name, 'logs') });
		items.push({ label: t('Settings'), icon: 'pencil', onClick: () => HB.drawer.open(name, 'settings') });
		return items;
	}

	HB.actions = { power, upgrade, apply, rollback, deleteBackup, remove, confirmIpInUse, menuItems, store };
})();
