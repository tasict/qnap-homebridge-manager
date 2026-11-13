/* Apply the remembered theme before the first paint. Loaded as a blocking
 * script in <head>; the toggle itself lives in app.js. */
(function () {
	'use strict';
	try {
		var t = window.localStorage.getItem('hb-theme');
		if (t === 'light' || t === 'dark') {
			document.documentElement.setAttribute('data-theme', t);
		}
	} catch (e) {
		/* storage blocked: follow the system */
	}
})();
