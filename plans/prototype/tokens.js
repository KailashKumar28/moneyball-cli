// moneyball design tokens: the terminal the frames are drawn in, light and dark. moneyball's TUI paints only
// named ANSI colours on the terminal's default background (ratatui Color::Cyan, DarkGray, ...), so the user's
// terminal theme decides how it looks; these are GitHub's light and dark terminal palettes, a common pair.
// --a<n> is ANSI colour n; ratatui names: 1 Red, 2 Green, 3 Yellow, 6 Cyan, 7 Gray, 8 DarkGray, 15 White.
const TOKENS = {
	light: {
		'--term-bg': '#ffffff', '--term-fg': '#1f2328', '--chrome': '#ececec', '--chrome-line': '#d6d6d6', '--chrome-ink': '#4d4d4d',
		'--a0': '#24292f', '--a1': '#cf222e', '--a2': '#116329', '--a3': '#4d2d00', '--a4': '#0969da', '--a5': '#8250df', '--a6': '#1b7c83', '--a7': '#6e7781',
		'--a8': '#57606a', '--a9': '#a40e26', '--a10': '#1a7f37', '--a11': '#633c01', '--a12': '#218bff', '--a13': '#a475f9', '--a14': '#3192aa', '--a15': '#8c959f',
		'--sel': '#0969da', '--note': '#9a6700',
	},
	dark: {
		'--term-bg': '#0d1117', '--term-fg': '#e6edf3', '--chrome': '#2b2b2b', '--chrome-line': '#1e1e1e', '--chrome-ink': '#b0b0b0',
		'--a0': '#484f58', '--a1': '#ff7b72', '--a2': '#3fb950', '--a3': '#d29922', '--a4': '#58a6ff', '--a5': '#bc8cff', '--a6': '#39c5cf', '--a7': '#b1bac4',
		'--a8': '#6e7681', '--a9': '#ffa198', '--a10': '#56d364', '--a11': '#e3b341', '--a12': '#79c0ff', '--a13': '#d2a8ff', '--a14': '#56d4dd', '--a15': '#ffffff',
		'--sel': '#58a6ff', '--note': '#d29922',
	},
};

// ratatui's name for each ANSI index, for the inspector's Style code
const ANSI_NAMES = ['Black', 'Red', 'Green', 'Yellow', 'Blue', 'Magenta', 'Cyan', 'Gray', 'DarkGray', 'LightRed', 'LightGreen', 'LightYellow', 'LightBlue', 'LightMagenta', 'LightCyan', 'White'];
