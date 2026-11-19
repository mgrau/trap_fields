// "Export" menu of a plot: copy or save the figure (PNG, SVG) and its data (CSV, TSV).
import { useEffect, useRef, useState } from 'react';
import { copyPng, figureSvg, saveText, slug, svgToPng, toCsv, toTsv, type FigureMeta, type FigureTheme } from './exportFigure';
import { download } from '../results';

type Rows = (string | number | null | undefined)[][];

const THEME_KEY = 'tf.figureTheme';
function loadTheme(): FigureTheme {
  try {
    return localStorage.getItem(THEME_KEY) === 'light' ? 'light' : 'dark';
  } catch {
    return 'dark';
  }
}

export function FigureMenu(props: { svg: () => SVGSVGElement | null; meta: FigureMeta; rows: () => Rows; name: string }) {
  const [open, setOpen] = useState(false);
  const [theme, setTheme] = useState<FigureTheme>(loadTheme);
  const [note, setNote] = useState<string | null>(null);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent ? e.key === 'Escape' : !box.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener('pointerdown', close);
    window.addEventListener('keydown', close);
    return () => {
      window.removeEventListener('pointerdown', close);
      window.removeEventListener('keydown', close);
    };
  }, [open]);

  useEffect(() => {
    if (!note) return;
    const t = window.setTimeout(() => setNote(null), 1600);
    return () => window.clearTimeout(t);
  }, [note]);

  const file = slug(props.name);
  const svgText = () => {
    const el = props.svg();
    if (!el) throw new Error('nothing to export');
    return figureSvg(el, props.meta, theme);
  };
  const act = (label: string, f: () => Promise<unknown> | unknown) => async () => {
    setOpen(false);
    try {
      await f();
      setNote(label);
    } catch (e) {
      setNote(`failed: ${(e as Error).message}`);
    }
  };
  const setThemePersist = (t: FigureTheme) => {
    setTheme(t);
    try {
      localStorage.setItem(THEME_KEY, t);
    } catch {
      /* private mode */
    }
  };

  return (
    <div className="fig-menu" ref={box}>
      <button className="plot-table-toggle" onClick={() => setOpen(!open)} aria-haspopup="menu" aria-expanded={open} title="Copy or save this figure and its data">
        {note ?? 'export ▾'}
      </button>
      {open && (
        <div className="fig-menu-list" role="menu">
          <button role="menuitem" onClick={act('copied', () => copyPng(svgToPng(svgText())))}>
            Copy image
          </button>
          <button role="menuitem" onClick={act('copied', () => navigator.clipboard.writeText(toTsv(props.rows())))}>
            Copy data <span className="hint">(tab-separated)</span>
          </button>
          <div className="fig-menu-sep" />
          <button role="menuitem" onClick={act('saved', async () => download(await svgToPng(svgText()), `${file}.png`))}>
            Save PNG
          </button>
          <button role="menuitem" onClick={act('saved', () => saveText(svgText(), `${file}.svg`, 'image/svg+xml'))}>
            Save SVG
          </button>
          <button role="menuitem" onClick={act('saved', () => saveText(toCsv(props.rows()), `${file}.csv`, 'text/csv'))}>
            Save CSV
          </button>
          <div className="fig-menu-sep" />
          <label className="check fig-menu-theme" title="Theme of copied and saved images">
            <input type="checkbox" checked={theme === 'light'} onChange={(e) => setThemePersist(e.target.checked ? 'light' : 'dark')} />
            light background
          </label>
        </div>
      )}
    </div>
  );
}

/** Copy / save menu for an HTML table (the first table inside `root()`). */
export function TableMenu(props: { root: () => HTMLElement | null; name: string }) {
  const [note, setNote] = useState<string | null>(null);
  useEffect(() => {
    if (!note) return;
    const t = window.setTimeout(() => setNote(null), 1600);
    return () => window.clearTimeout(t);
  }, [note]);
  const rows = () => {
    // every table of the card (not a plot's table view), separated by an empty row
    const tables = Array.from(props.root()?.querySelectorAll('table') ?? []).filter((x) => !x.closest('.plot-table'));
    if (!tables.length) throw new Error('no table');
    return tables.flatMap((t, i) => [
      ...(i > 0 ? [[] as string[]] : []),
      ...Array.from(t.rows).map((r) => Array.from(r.cells).map((c) => (c.textContent ?? '').trim())),
    ]);
  };
  const run = (label: string, f: () => Promise<unknown> | unknown) => async () => {
    try {
      await f();
      setNote(label);
    } catch (e) {
      setNote(`failed: ${(e as Error).message}`);
    }
  };
  return (
    <span className="table-menu">
      {note ? (
        <span className="hint">{note}</span>
      ) : (
        <>
          <button className="plot-table-toggle" onClick={run('copied', () => navigator.clipboard.writeText(toTsv(rows())))} title="Copy the table (tab-separated, pastes into spreadsheets)">
            copy
          </button>
          <button className="plot-table-toggle" onClick={run('saved', () => saveText(toCsv(rows()), `${slug(props.name)}.csv`, 'text/csv'))} title="Save the table as CSV">
            csv
          </button>
        </>
      )}
    </span>
  );
}
