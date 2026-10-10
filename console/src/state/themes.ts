// Terminal colours and type: iglu's own, a few well-known themes, any theme
// pasted in Ghostty's format, and the sizes a font comes in. Pure.

export interface Colors {
  background: string;
  foreground: string;
  cursor: string;
  /** What selected text is drawn on, and in. */
  selection: string | null;
  selectionText: string | null;
  /** The 16 colours programs pick by number. */
  palette: readonly string[];
}

/** xterm's 16 colours, which iglu's own colours use. */
export const XTERM: readonly string[] = [
  "#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
  "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff",
];

/** Themes as Ghostty ships them, from iTerm2-Color-Schemes. */
export const THEMES: Readonly<Record<string, Colors>> = {
  "Catppuccin Mocha": {
    background: "#1e1e2e", foreground: "#cdd6f4", cursor: "#f5e0dc",
    selection: "#f5e0dc", selectionText: "#1e1e2e",
    palette: ["#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de", "#585b70", "#f7aec2", "#c2ecbf", "#fcd682", "#aeccfc", "#f398da", "#b1eae1", "#a6adc8"],
  },
  "Catppuccin Latte": {
    background: "#eff1f5", foreground: "#4c4f69", cursor: "#dc8a78",
    selection: "#dc8a78", selectionText: "#eff1f5",
    palette: ["#bcc0cc", "#d20f39", "#40a02b", "#df8e1d", "#1e66f5", "#ea76cb", "#179299", "#5c5f77", "#acb0be", "#e7103f", "#46b02f", "#e49931", "#3878f6", "#ef95d7", "#19a1a8", "#6c6f85"],
  },
  "Dracula": {
    background: "#282a36", foreground: "#f8f8f2", cursor: "#f8f8f2",
    selection: "#44475a", selectionText: "#ffffff",
    palette: ["#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2", "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df", "#a4ffff", "#ffffff"],
  },
  "GitHub Dark": {
    background: "#0d1117", foreground: "#e6edf3", cursor: "#2f81f7",
    selection: "#e6edf3", selectionText: "#0d1117",
    palette: ["#484f58", "#ff7b72", "#3fb950", "#d29922", "#58a6ff", "#bc8cff", "#39c5cf", "#b1bac4", "#6e7681", "#ffa198", "#56d364", "#e3b341", "#79c0ff", "#d2a8ff", "#56d4dd", "#ffffff"],
  },
  "GitHub Light": {
    background: "#ffffff", foreground: "#1f2328", cursor: "#0969da",
    selection: "#1f2328", selectionText: "#ffffff",
    palette: ["#24292f", "#cf222e", "#116329", "#4d2d00", "#0969da", "#8250df", "#1b7c83", "#6e7781", "#57606a", "#a40e26", "#1a7f37", "#633c01", "#218bff", "#a475f9", "#3192aa", "#8c959f"],
  },
  "Gruvbox Dark": {
    background: "#282828", foreground: "#ebdbb2", cursor: "#ebdbb2",
    selection: "#665c54", selectionText: "#ebdbb2",
    palette: ["#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984", "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2"],
  },
  "Gruvbox Light": {
    background: "#fbf1c7", foreground: "#3c3836", cursor: "#3c3836",
    selection: "#3c3836", selectionText: "#fbf1c7",
    palette: ["#fbf1c7", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#7c6f64", "#928374", "#9d0006", "#79740e", "#b57614", "#076678", "#8f3f71", "#427b58", "#3c3836"],
  },
  "Kanagawa Wave": {
    background: "#1f1f28", foreground: "#dcd7ba", cursor: "#dcd7ba",
    selection: "#dcd7ba", selectionText: "#1f1f28",
    palette: ["#090618", "#c34043", "#76946a", "#c0a36e", "#7e9cd8", "#957fb8", "#6a9589", "#c8c093", "#727169", "#e82424", "#98bb6c", "#e6c384", "#7fb4ca", "#938aa9", "#7aa89f", "#dcd7ba"],
  },
  "Nord": {
    background: "#2e3440", foreground: "#d8dee9", cursor: "#eceff4",
    selection: "#eceff4", selectionText: "#4c566a",
    palette: ["#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0", "#596377", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4"],
  },
  "Rosé Pine": {
    background: "#191724", foreground: "#e0def4", cursor: "#e0def4",
    selection: "#403d52", selectionText: "#e0def4",
    palette: ["#26233a", "#eb6f92", "#31748f", "#f6c177", "#9ccfd8", "#c4a7e7", "#ebbcba", "#e0def4", "#6e6a86", "#eb6f92", "#31748f", "#f6c177", "#9ccfd8", "#c4a7e7", "#ebbcba", "#e0def4"],
  },
  "Rosé Pine Dawn": {
    background: "#faf4ed", foreground: "#575279", cursor: "#575279",
    selection: "#dfdad9", selectionText: "#575279",
    palette: ["#f2e9e1", "#b4637a", "#286983", "#ea9d34", "#56949f", "#907aa9", "#d7827e", "#575279", "#9893a5", "#b4637a", "#286983", "#ea9d34", "#56949f", "#907aa9", "#d7827e", "#575279"],
  },
  "Solarized Dark": {
    background: "#002b36", foreground: "#839496", cursor: "#839496",
    selection: "#073642", selectionText: "#93a1a1",
    palette: ["#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5", "#335e69", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3"],
  },
  "Solarized Light": {
    background: "#fdf6e3", foreground: "#657b83", cursor: "#657b83",
    selection: "#eee8d5", selectionText: "#586e75",
    palette: ["#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#bbb5a2", "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3"],
  },
  "Tokyo Night": {
    background: "#1a1b26", foreground: "#c0caf5", cursor: "#c0caf5",
    selection: "#33467c", selectionText: "#c0caf5",
    palette: ["#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6", "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5"],
  },
};

/** The colours of a theme: "iglu", iglu's own, which follow the look, gives
 * none, as does "pasted" when the paste isn't a theme. */
export function colorsOf(theme: string, pasted: string): Colors | null {
  if (theme === "pasted") {
    const parsed = parseTheme(pasted);
    return "colors" in parsed ? parsed.colors : null;
  }
  return Object.hasOwn(THEMES, theme) ? THEMES[theme]! : null;
}

/** `#rrggbb`, from Ghostty's hex forms: `#rrggbb`, `rrggbb`, `#rgb` or `rgb`. */
function hex(value: string): string | null {
  const match = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(value);
  if (!match) return null;
  const digits = match[1]!.toLowerCase();
  return `#${digits.length === 3 ? [...digits].map((d) => d + d).join("") : digits}`;
}

/** A value as Ghostty takes it, quoted or not. */
function unquote(value: string): string {
  return value.length >= 2 && value.startsWith('"') && value.endsWith('"') ? value.slice(1, -1).trim() : value;
}

export type Parsed = { colors: Colors } | { error: string };

/** A theme in Ghostty's format, as its theme files have it and `ghostty
 * +show-config` prints it. Keys that aren't colours are passed over, so a
 * whole config can be pasted, as are palette colours past the 16 programs
 * pick by number and keys left empty. Colours are read in hex, not by name. */
export function parseTheme(text: string): Parsed {
  const found: Record<string, string> = {};
  const palette = [...XTERM];
  const lines = text.split("\n");
  for (const [index, raw] of lines.entries()) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const equals = line.indexOf("=");
    if (equals < 0) return { error: `Line ${index + 1} isn't \`key = value\`.` };
    const key = line.slice(0, equals).trim();
    let value = unquote(line.slice(equals + 1).trim());
    if (!value) continue;
    if (key === "palette") {
      const entry = /^(\d+)\s*=\s*(\S+)$/.exec(value);
      if (!entry) return { error: `Line ${index + 1}: a palette entry is \`palette = 0=#1e1e2e\`.` };
      const number = Number(entry[1]);
      if (number > 15) continue;
      value = entry[2]!;
      const colour = hex(value);
      if (!colour) return { error: `Line ${index + 1}: ${value} isn't a colour iglu reads; use hex, like #1e1e2e.` };
      palette[number] = colour;
      continue;
    }
    if (!["background", "foreground", "cursor-color", "selection-background", "selection-foreground"].includes(key)) continue;
    const colour = hex(value);
    if (!colour) return { error: `Line ${index + 1}: ${value} isn't a colour iglu reads; use hex, like #1e1e2e.` };
    found[key] = colour;
  }
  const background = found["background"];
  const foreground = found["foreground"];
  if (!background || !foreground) return { error: "A theme needs at least a background and a foreground." };
  return {
    colors: {
      background,
      foreground,
      cursor: found["cursor-color"] ?? foreground,
      selection: found["selection-background"] ?? null,
      selectionText: found["selection-foreground"] ?? null,
      palette,
    },
  };
}

/** Colours in Ghostty's format, for starting a theme of one's own from one. */
export function formatTheme(colors: Colors): string {
  return [
    `background = ${colors.background}`,
    `foreground = ${colors.foreground}`,
    `cursor-color = ${colors.cursor}`,
    ...(colors.selection ? [`selection-background = ${colors.selection}`] : []),
    ...(colors.selectionText ? [`selection-foreground = ${colors.selectionText}`] : []),
    ...colors.palette.map((colour, index) => `palette = ${index}=${colour}`),
  ].join("\n");
}

/** Whether a theme is a light one, by its background's luminance. */
export function isLight(colors: Colors): boolean {
  const n = parseInt(colors.background.slice(1), 16);
  const [r, g, b] = [n >> 16, (n >> 8) & 0xff, n & 0xff];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b > 128;
}

/** A font's name as typed, if it's one that can be asked for safely: CSS
 * takes it quoted, so quotes and the like aren't allowed. */
export function fontName(text: string): string | null {
  const name = text.trim().replace(/\s+/g, " ");
  return /^[\p{L}\p{N} ._-]{1,64}$/u.test(name) ? name : null;
}

/** Rows are this many times the font's size: 17px at iglu's 13⅓px. */
const LINE = 1.275;

export interface CellSize {
  /** The font's size, in pixels. */
  font: number;
  /** A cell's width and height, in whole pixels. */
  width: number;
  row: number;
}

/** The size the terminal's text is set at for a size asked for. It's nudged
 * so each cell is a whole number of pixels wide and tall: glyphs, and the
 * blocks and lines drawn in them, then meet with no seams. `advance` is the
 * font's character width as a fraction of its size. */
export function cellSize(advance: number, asked: number): CellSize {
  const width = Math.max(1, Math.round(asked * advance));
  const font = width / advance;
  return { font, width, row: Math.round(font * LINE) };
}

/** The sizes a font comes in, between `least` and `most` pixels. */
export function cellSizes(advance: number, least = 9, most = 28): CellSize[] {
  const sizes: CellSize[] = [];
  for (let width = Math.max(1, Math.ceil(least * advance)); width / advance <= most; width++) sizes.push(cellSize(advance, width / advance));
  return sizes;
}
