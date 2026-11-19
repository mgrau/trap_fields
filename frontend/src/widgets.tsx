import { useEffect, useState, type ReactNode } from 'react';
import { num, toMeters, unitLabel, type Units } from './units';

/** Numeric input showing a length (stored in meters) in display units. */
export function LengthField(props: { value: number; units: Units; onChange: (m: number) => void; label?: string; title?: string }) {
  return (
    <NumberField
      label={props.label}
      title={props.title}
      value={Number(num(props.value, props.units, 6))}
      suffix={unitLabel(props.units)}
      onChange={(v) => props.onChange(toMeters(v, props.units))}
    />
  );
}

/** Commit-on-blur/Enter numeric input. */
export function NumberField(props: { value: number; onChange: (v: number) => void; label?: string; suffix?: string; title?: string; step?: number }) {
  const [text, setText] = useState(String(props.value));
  useEffect(() => setText(String(props.value)), [props.value]);
  const commit = () => {
    const v = Number(text);
    if (Number.isFinite(v) && text.trim() !== '') {
      if (v !== props.value) props.onChange(v);
    } else setText(String(props.value));
  };
  return (
    <label className="field" title={props.title}>
      {props.label && <span className="field-label">{props.label}</span>}
      <input
        type="text"
        inputMode="decimal"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
        }}
      />
      {props.suffix && <span className="field-suffix">{props.suffix}</span>}
    </label>
  );
}

export function Section(props: { title: string; children: ReactNode; right?: ReactNode; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(props.defaultOpen ?? true);
  return (
    <section className="section">
      <header onClick={() => setOpen(!open)}>
        <span className={`chev ${open ? 'open' : ''}`}>▸</span>
        <h3>{props.title}</h3>
        <span className="section-right" onClick={(e) => e.stopPropagation()}>
          {props.right}
        </span>
      </header>
      {open && <div className="section-body">{props.children}</div>}
    </section>
  );
}

export function KV(props: { k: ReactNode; v: ReactNode; title?: string }) {
  return (
    <div className="kv" title={props.title}>
      <span className="k">{props.k}</span>
      <span className="v">{props.v}</span>
    </div>
  );
}

export function Seg<T extends string>(props: { value: T; options: { value: T; label: ReactNode; title?: string }[]; onChange: (v: T) => void }) {
  return (
    <div className="seg">
      {props.options.map((o) => (
        <button key={o.value} className={o.value === props.value ? 'on' : ''} title={o.title} onClick={() => props.onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}
