// moneyball prototype: index.html?scene=<id> draws one frame of the flow board. A frame is a terminal (cells
// captured from the real binary by capture.py into SCREENS, or authored for the redesign in redesign.js) or a
// browser showing the HTML report. With &inspect it speaks the board's protocol: pins, select, export.

const PARAMS = new URLSearchParams(window.name || location.search);
const SCENE = PARAMS.get('scene') || FRAMES[0].scene;
const THEME = PARAMS.get('theme') === 'dark' ? 'dark' : 'light';
const INSPECT = PARAMS.has('inspect');
const $ = id => document.getElementById(id);
const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const frameOf = scene => FRAMES.find(f => f.scene === scene);

for (const [k, v] of Object.entries(TOKENS[THEME])) document.documentElement.style.setProperty(k, v);
document.documentElement.dataset.theme = THEME;

// ── Authored screens: a line of text with [cyan]...[/] style tags, the TUI's own colours ──

const TAGS = { red: 'f1', green: 'f2', yellow: 'f3', blue: 'f4', magenta: 'f5', cyan: 'f6', gray: 'f7', dim: 'f8', white: 'f15', bold: 'B', italic: 'I', under: 'U', rev: 'R' };
const TAG_RE = new RegExp(`\\[(\\/|(?:${Object.keys(TAGS).join('|')})(?: (?:${Object.keys(TAGS).join('|')}))*)\\]`, 'g');

function parseMarkup(text) {
	return text.replace(/^\n/, '').replace(/\n\s*$/, '').split('\n').map(line => {
		const runs = [];
		let style = '';
		let pos = 0;
		const push = t => { if (!t) return; if (runs.length && runs[runs.length - 1][1] === style) runs[runs.length - 1][0] += t; else runs.push([t, style]); };
		for (const m of line.matchAll(TAG_RE)) {
			push(line.slice(pos, m.index));
			style = m[1] === '/' ? '' : m[1].split(' ').map(t => TAGS[t]).join(' ');
			pos = m.index + m[0].length;
		}
		push(line.slice(pos));
		return runs;
	});
}

function screenOf(scene) {
	if (SCREENS[scene]) return SCREENS[scene];
	const a = typeof AUTHORED !== 'undefined' && AUTHORED[scene];
	if (a) return { kind: a.kind || 'tui', cwd: a.cwd || '/Users/Kailash/DEV/moneyball-test/moneyball-data', cmd: a.cmd || 'moneyball', cols: 140, rows: 42, lines: parseMarkup(a.text), authored: true };
	return null;
}

// ── Drawing ──

const XTERM = i => {
	if (i < 16) return `var(--a${i})`;
	if (i >= 232) { const v = 8 + (i - 232) * 10; return `rgb(${v},${v},${v})`; }
	const n = i - 16, c = x => (x ? 55 + x * 40 : 0);
	return `rgb(${c(Math.floor(n / 36))},${c(Math.floor(n / 6) % 6)},${c(n % 6)})`;
};

// - a run's colours and modifiers; reverse video swaps the two colours like a terminal does
function runStyle(style) {
	let fg = null;
	let bg = null;
	const cls = [];
	for (const part of style.split(' ').filter(Boolean)) {
		const m = part.match(/^([fb])(#[0-9a-f]{6}|\d+)$/);
		const value = m && (m[2].startsWith('#') ? m[2] : XTERM(Number(m[2])));
		if (m?.[1] === 'f') fg = value;
		else if (m) bg = value;
		else if (part !== 'R') cls.push(part);
	}
	if (style.split(' ').includes('R')) [fg, bg] = [bg || 'var(--term-bg)', fg || 'var(--term-fg)'];
	return { css: [fg && `color:${fg}`, bg && `background:${bg}`].filter(Boolean).join(';'), cls: cls.join(' ') };
}

const width = text => [...text].length;
// - a terminal grid gives every character one cell; a glyph the font lacks falls back to a wider face, so each
//   non-ASCII character gets its own one-cell box instead of pushing its neighbours
const cells = text => [...text].map(ch => (ch.charCodeAt(0) < 128 ? esc(ch) : `<span class="g">${esc(ch)}</span>`)).join('');

function terminalHtml(s) {
	const title = s.kind === 'shell' ? `${s.cwd.split('/').pop()} - bash` : `moneyball - ${s.cwd.split('/').pop()}`;
	const lines = s.lines.map((runs, r) => {
		let col = 1;
		const spans = runs.map(([text, style]) => {
			const { css, cls } = runStyle(style);
			const w = width(text);
			const html = `<span class="run ${cls}" data-row="${r + 1}" data-col="${col}" data-style="${style}" style="width:${w}ch;${css}">${cells(text)}</span>`;
			col += w;
			return html;
		}).join('');
		return `<div class="ln" data-row="${r + 1}">${spans || ' '}</div>`;
	}).join('');
	return `<div class="win term"><div class="bar"><i></i><i></i><i></i><span>${esc(title)} - ${s.cols}x${s.rows}</span></div>
		<div class="screen" id="screen">${lines}</div></div>`;
}

function browserHtml(w) {
	return `<div class="win web"><div class="bar"><i></i><i></i><i></i><span class="url">${esc(w.url)}</span></div>
		<iframe id="page" title="${esc(w.url)}"></iframe></div>`;
}

function draw() {
	const f = frameOf(SCENE);
	const web = f?.web;
	if (web) {
		document.body.innerHTML = browserHtml(web) + '<div id="ix"></div>';
		const iframe = $('page');
		iframe.addEventListener('load', () => {
			const doc = iframe.contentDocument;
			for (const sel of web.check || []) doc.querySelectorAll(sel).forEach(box => { box.checked = true; });
			if (web.anchor) doc.getElementById(web.anchor)?.scrollIntoView({ block: 'start' });
			if (web.scroll) iframe.contentWindow.scrollBy(0, web.scroll);
			ready();
		}, { once: true });
		// a report is megabytes: load only the one this frame shows (a packed board has them inline already)
		const show = () => { iframe.srcdoc = REPORTS[web.report]; };
		if (REPORTS[web.report]) show();
		else document.head.append(Object.assign(document.createElement('script'), { src: `reports/${web.report}.js`, onload: show }));
		return;
	}
	const s = screenOf(SCENE);
	document.body.innerHTML = s ? terminalHtml(s) + '<div id="ix"></div>' : `<p class="missing">No screen for "${esc(SCENE)}": run capture.py</p>`;
	ready();
}

// ── Inspect: the board's Dev Mode for a terminal. A run is the element: its text, position and ratatui Style ──

let selected = null;
let pinItems = [];
let pinsOn = true;
const send = message => parent.postMessage({ source: 'moneyball-inspect', scene: SCENE, ...message }, '*');

function ready() {
	window.SCENE_READY = true;
	if (!INSPECT) return protoMenu();
	send({ type: 'ready' });
}

// - a pin's target: ~text (the run where that text starts), @row or @row:col, else a CSS selector
function find(sel) {
	if (!sel) return null;
	const doc = $('page')?.contentDocument || document;
	if (sel.startsWith('~')) {
		const want = sel.slice(1);
		for (const ln of document.querySelectorAll('.ln')) {
			const at = ln.textContent.indexOf(want);
			if (at < 0) continue;
			let col = 0;
			for (const run of ln.querySelectorAll('.run')) { col += width(run.textContent); if (col > at) return run; }
		}
		return null;
	}
	const m = sel.match(/^@(\d+)(?::(\d+))?$/);
	if (m) {
		const ln = document.querySelector(`.ln[data-row="${m[1]}"]`);
		if (!ln || !m[2]) return ln;
		return [...ln.querySelectorAll('.run')].find(run => Number(run.dataset.col) + width(run.textContent) > Number(m[2])) || ln;
	}
	try { return doc.querySelector(sel); } catch { return null; }
}

// - where an element sits in this page, through the report iframe when it lives there
function boxOf(el) {
	const r = el.getBoundingClientRect();
	if (el.ownerDocument === document) return r;
	const f = $('page').getBoundingClientRect();
	return { left: r.left + f.left, top: r.top + f.top, width: r.width, height: r.height };
}

function paintIx(hl) {
	const ix = $('ix');
	if (!ix) return;
	const box = (el, cls) => { const b = boxOf(el); return `<div class="${cls}" style="left:${b.left}px;top:${b.top}px;width:${b.width}px;height:${b.height}px"></div>`; };
	const pins = pinsOn ? pinItems.map(p => { const el = find(p.sel); if (!el) return ''; const b = boxOf(el); if (b.top < 0 || b.top > innerHeight) return ''; return `<button type="button" class="pin" data-kind="${p.kind}" data-n="${p.n}" style="left:${b.left}px;top:${b.top}px">${p.n}</button>`; }).join('') : '';
	ix.innerHTML = (selected ? box(selected, 'box sel') : '') + (hl?.el ? box(hl.el, `box hl k-${hl.kind}`) : '') + pins;
}

function elementData(el) {
	if (el.classList.contains('run')) {
		const style = el.dataset.style || '';
		const parts = style.split(' ').filter(Boolean);
		const color = key => { const p = parts.find(x => x.startsWith(key) && /^[fb](#|\d)/.test(x)); if (!p) return null; const v = p.slice(1); return v.startsWith('#') ? { name: v, hex: v } : { index: Number(v), name: Number(v) < 16 ? ANSI_NAMES[v] : `Indexed(${v})`, hex: Number(v) < 16 ? TOKENS[THEME][`--a${v}`] : XTERM(Number(v)) }; };
		const fg = color('f');
		const bg = color('b');
		const mods = { B: 'BOLD', D: 'DIM', I: 'ITALIC', U: 'UNDERLINED', R: 'REVERSED' };
		const on = parts.filter(p => mods[p]).map(p => mods[p]);
		const code = ['Style::default()', fg && `.fg(Color::${fg.index !== undefined && fg.index < 16 ? fg.name : `Indexed(${fg.index})`})`, bg && `.bg(Color::${bg.name})`, ...on.map(m => `.add_modifier(Modifier::${m})`)].filter(Boolean).join('');
		return { name: 'run', text: el.textContent, row: Number(el.dataset.row), col: Number(el.dataset.col), len: width(el.textContent), fg, bg, mods: on, code, path: ['screen', `line ${el.dataset.row}`, 'run'] };
	}
	if (el.classList.contains('ln')) return { name: 'line', text: el.textContent, row: Number(el.dataset.row), len: width(el.textContent), path: ['screen', `line ${el.dataset.row}`] };
	return { name: el.tagName.toLowerCase() + (el.className && typeof el.className === 'string' ? '.' + el.className.split(' ')[0] : ''), text: (el.textContent || '').trim().slice(0, 160), path: ['page', el.tagName.toLowerCase()] };
}

function pick(el) {
	selected = el;
	paintIx();
	if (el) send({ type: 'select', element: elementData(el) }); else send({ type: 'clear' });
}

function screenText() {
	return [...document.querySelectorAll('.ln')].map(ln => ln.textContent.replace(/\s+$/, '')).join('\n');
}

function exportData() {
	const f = frameOf(SCENE);
	if (f?.web) return { theme: THEME, kind: 'web', url: f.web.url, text: '', runs: [] };
	const s = screenOf(SCENE);
	const runs = [];
	s.lines.forEach((line, r) => { let col = 1; for (const [text, style] of line) { if (style && text.trim()) runs.push({ row: r + 1, col, text, style }); col += width(text); } });
	return { theme: THEME, kind: s.kind, cwd: s.cwd, cmd: s.cmd, cols: s.cols, rows: s.rows, authored: !!s.authored, text: screenText(), runs };
}

if (INSPECT) {
	addEventListener('message', e => {
		const m = e.data;
		if (!m || m.source !== 'moneyball-board') return;
		if (m.type === 'zoom') document.documentElement.style.setProperty('--iz', m.k);
		if (m.type === 'pins') { pinItems = m.items; pinsOn = m.on; paintIx(); }
		if (m.type === 'hl') paintIx({ el: find(m.sel), kind: m.kind });
		if (m.type === 'pick') pick(find(m.sel));
		if (m.type === 'clear') { selected = null; paintIx(); }
		if (m.type === 'up' && selected) { let el = selected; for (let i = 0; i < m.levels && el.parentElement && el.id !== 'screen'; i++) el = el.parentElement; pick(el); }
		if (m.type === 'export') send({ type: 'export', id: m.id, ...exportData() });
	});
	// a press that moves pans the board; one that doesn't picks the run under it
	addEventListener('pointerdown', e => {
		const pin = e.target.closest('.pin');
		if (pin) { send({ type: 'pin', n: Number(pin.dataset.n) }); return; }
		const start = { x: e.clientX, y: e.clientY };
		let last = start;
		let moved = false;
		const move = ev => { if (!moved && Math.hypot(ev.clientX - start.x, ev.clientY - start.y) < 4) return; moved = true; send({ type: 'pan', dx: ev.clientX - last.x, dy: ev.clientY - last.y }); last = { x: ev.clientX, y: ev.clientY }; };
		const up = ev => { removeEventListener('pointermove', move); removeEventListener('pointerup', up); if (!moved) pick(ev.target.closest('.run, .ln')); };
		addEventListener('pointermove', move);
		addEventListener('pointerup', up);
	});
	addEventListener('wheel', e => { e.preventDefault(); send({ type: 'wheel', dx: e.deltaX, dy: e.deltaY, zoom: e.ctrlKey || e.metaKey, shift: e.shiftKey, x: e.clientX, y: e.clientY }); }, { passive: false });
	addEventListener('keydown', e => send({ type: 'key', key: e.key, shift: e.shiftKey, code: e.code }));
}

// ── Prototype mode (no &inspect): step through the flow like a slideshow ──

function protoMenu() {
	if (PARAMS.has('still')) return;
	const i = FRAMES.findIndex(f => f.scene === SCENE);
	const go = scene => { location.search = `?scene=${scene}&theme=${THEME}`; };
	const nav = document.createElement('div');
	nav.className = 'proto';
	nav.innerHTML = `<button type="button" data-step="-1" aria-label="Previous screen">&larr;</button><span>${esc(FRAMES[i]?.code || '')} ${esc(FRAMES[i]?.title || SCENE)}</span><button type="button" data-step="1" aria-label="Next screen">&rarr;</button>
		<select aria-label="Jump to a screen">${SECTIONS.map(s => `<optgroup label="${esc(s.code)} · ${esc(s.title)}">${FRAMES.filter(f => f.s === s.code).map(f => `<option value="${f.scene}"${f.scene === SCENE ? ' selected' : ''}>${esc(f.code)} ${esc(f.title)}</option>`).join('')}</optgroup>`).join('')}</select>
		<button type="button" data-theme-flip="1">${THEME === 'dark' ? 'Light' : 'Dark'}</button>`;
	document.body.append(nav);
	nav.addEventListener('click', e => {
		const b = e.target.closest('button');
		if (b?.dataset.step) go(FRAMES[Math.max(0, Math.min(FRAMES.length - 1, i + Number(b.dataset.step)))].scene);
		if (b?.dataset.themeFlip) location.search = `?scene=${SCENE}&theme=${THEME === 'dark' ? 'light' : 'dark'}`;
	});
	nav.querySelector('select').addEventListener('change', e => go(e.target.value));
	addEventListener('keydown', e => { if (e.key === 'ArrowRight' && FRAMES[i + 1]) go(FRAMES[i + 1].scene); if (e.key === 'ArrowLeft' && i > 0) go(FRAMES[i - 1].scene); });
}

FRAMES.forEach((f, n) => { f.code ||= `${f.s}${FRAMES.filter((g, m) => g.s === f.s && m <= n).length}`; });
draw();
