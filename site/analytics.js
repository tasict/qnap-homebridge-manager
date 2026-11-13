/* Google Analytics 4 with Consent Mode. Until the visitor allows analytics, GA stores
 * nothing on the device and gets only cookieless pings. The choice is kept in
 * localStorage and can be changed from the footer. Clicks on links marked
 * data-download are sent as a download_click event. */
(function () {
	'use strict';

	const GA_ID = '';
	const KEY = 'hbm-analytics';

	if (!GA_ID) {
		return;
	}

	function stored() {
		try {
			return localStorage.getItem(KEY);
		} catch (e) {
			return null;
		}
	}

	function store(v) {
		try {
			localStorage.setItem(KEY, v);
		} catch (e) {
			/* storage blocked: the choice lasts for this page only */
		}
	}

	window.dataLayer = window.dataLayer || [];
	function gtag() {
		window.dataLayer.push(arguments);
	}

	gtag('consent', 'default', {
		analytics_storage: stored() === 'granted' ? 'granted' : 'denied',
		ad_storage: 'denied',
		ad_user_data: 'denied',
		ad_personalization: 'denied',
	});
	gtag('js', new Date());
	gtag('config', GA_ID);

	const tag = document.createElement('script');
	tag.async = true;
	tag.src = 'https://www.googletagmanager.com/gtag/js?id=' + encodeURIComponent(GA_ID);
	document.head.appendChild(tag);

	document.addEventListener('click', (e) => {
		const a = e.target.closest && e.target.closest('a[data-download]');
		if (a) {
			gtag('event', 'download_click', { link_url: a.href, placement: a.dataset.download, transport_type: 'beacon' });
		}
	});

	let banner = null;

	function decide(v) {
		store(v);
		gtag('consent', 'update', { analytics_storage: v });
		if (banner) {
			banner.hidden = true;
		}
	}

	function button(text, cls, v) {
		const b = document.createElement('button');
		b.type = 'button';
		b.className = 'btn ' + cls;
		b.textContent = text;
		b.addEventListener('click', () => decide(v));
		return b;
	}

	function showBanner() {
		if (!banner) {
			banner = document.createElement('section');
			banner.className = 'consent';
			banner.setAttribute('aria-label', 'Analytics cookies');
			const p = document.createElement('p');
			p.textContent = 'May this site use Google Analytics cookies to count visits and download clicks? The site works the same either way.';
			const actions = document.createElement('div');
			actions.className = 'consent-actions';
			actions.append(button('Allow', 'btn-primary', 'granted'), button('No thanks', 'btn-quiet', 'denied'));
			banner.append(p, actions);
			document.body.appendChild(banner);
		}
		banner.hidden = false;
	}

	function start() {
		const note = document.querySelector('[data-consent-note]');
		const settings = document.querySelector('[data-consent-settings]');
		if (note) {
			note.hidden = false;
		}
		if (settings) {
			settings.addEventListener('click', showBanner);
		}
		if (!stored()) {
			showBanner();
		}
	}

	if (document.readyState === 'loading') {
		document.addEventListener('DOMContentLoaded', start);
	} else {
		start();
	}
})();
